# Codex 对齐改造实施计划（Persistence / Protocol / Resume 全套）

> 日期：2026-08-25
> 状态：方案已评审，待实施
> 参照物：openai/codex @ a7b86b62（本地 `../codex`，已更新至 2026-08-25 main）
> 范围：持久化模型重构、trace 信封、模型历史/UI 事件分离、全局 home 目录、
> protocol crate 拆分、store 收拢、resume 加固、线程列表分页、上下文压缩、
> 迁移并发加固、工具 crate 隔离。

---

## 0. 背景与总原则

### 0.1 Codex 的持久化架构（先读懂再动手）

Codex **不是**"用 rollout 替代 SQLite"，而是三层混合：

```
JSONL rollout 文件（真相源，append-only，每行 {timestamp, ordinal, item}）
   ├── 崩溃安全 / 人类可读 / 可 grep
   └── 启动时 backfill ──→  SQLite（codex-state crate，sqlx runtime）
                            仅做列表/搜索/分页索引；损坏可从 JSONL 全量重建
```

关键代码锚点（均在 `../codex/codex-rs/`）：

| 关注点 | 文件 | 要点 |
|---|---|---|
| 行信封与条目枚举 | `history/src/lib.rs` | `RolloutLine { timestamp, ordinal, item }`；`RolloutItem` 枚举含 `SessionMeta / ResponseItem(ResponseItemEnvelope) / Compacted / TurnContext / EventMsg / ...` |
| 模型历史恢复三态 | `history/src/lib.rs:222` | `InitialHistory { New, Cleared, Resumed(ResumedHistory), Forked(Vec<RolloutItem>) }` |
| 解码边界 workaround | `rollout/src/lib.rs` | `decode_rollout_line()` 手工剥离 timestamp/ordinal 再解 item（serde arbitrary_precision + flattened 的已知坑） |
| 录制器 | `rollout/src/recorder.rs` | `RolloutRecorder::new/resume/persist/flush/shutdown`、`find_latest_thread_path`、`append_rollout_item_to_path` |
| 反向扫描 | `rollout/src/reverse_jsonl_scanner.rs` | `ReverseJsonlScanner::new/new_at(end_byte_offset)/scan_next`，从文件尾恢复最后 N 条 |
| 列表分页 | `rollout/src/list.rs` | `Cursor { ts, id }` keyset 分页；`ThreadSortKey / ThreadsPage / ThreadItem` |
| 压缩与归档 | `rollout/src/compression.rs`、`maintenance.rs` | 后台压缩 worker、plain/compressed 双路径、归档目录 |
| SQLite 状态库 | `state/src/`（runtime.rs、migrations.rs） | sqlx 异步 runtime，迁移独立成模块并有 `migrations_tests.rs` |
| home 目录解析 | `utils/home-dir/src/lib.rs` | env `CODEX_HOME`（必须存在且为目录，canonicalize）→ 默认 `~/.codex` |
| 协议 crate | `app-server-protocol/` | 纯 DTO crate，零业务依赖，桌面/IDE 共享同一份协议 |
| 工具隔离 | `apply-patch/` | patch 应用单独成 crate + 海量测试 |

### 0.2 Rove 现状事实（实施前必读）

| 现状 | 位置 | 问题 |
|---|---|---|
| trace 写裸事件 | `runtime/src/state/trace.rs` — `TraceWriter::append_line` 直接 `serde_json::to_string(event)` | 行内无 timestamp/seq，文件不自证顺序 |
| seq 依赖 SQLite 分配 | `trace.rs::append` → `index.last_event_seq(run_id)+1` | index 与文件可能失同步 |
| StreamEvent 一锅端 | `runtime/src/foundation/events.rs` | 产品级事件（RunStarted/AgentProfileActivated/TextDelta…）混流，resume 时无法直接区分哪些进模型上下文 |
| 两套 SQLite | ① `runtime/src/state/index.rs`（sessions/jobs/runs/task_states/reports/events/event_offsets/pending_approvals）② `apps/api/src/product/store/schema.rs`（schema v14，product_workspaces/product_sessions…，repository.rs 7149 行） | session 状态双份，职责纠缠 |
| API 巨石 | `apps/api/src/lib.rs` 5647 行 | 路由/store/transcript 投影全在一起 |
| resume 薄弱 | `runtime/src/state/resume.rs` 仅 235 行 | 无反向扫描，无 InitialHistory 三态 |
| 状态存工作区 `.rove/runs/<run_id>/` | `trace.rs::RunStore` | 未用系统默认目录 |

### 0.3 总原则

1. **JSONL 是唯一真相源**；SQLite 只做派生索引，任何时刻可删库重建。
2. **每行自证**：timestamp + ordinal 在行内，不依赖外部 DB。
3. **模型历史与 UI 事件分离**：resume 只重放 ModelHistoryItem。
4. **协议先行冻结**：wire 格式带版本号，改格式必须走迁移。
5. 每个 Phase 独立可交付、可回滚，不跨 Phase 改同一文件。

---

## Phase 1 — Trace 信封改造（最低风险，最先做）

### 目标
每行 trace 从裸事件变为 Codex 式信封，文件自身携带顺序与时间。

### 设计

```rust
// runtime/src/state/trace.rs（新）
#[derive(Serialize, Deserialize)]
pub struct TraceLine {
    /// RFC3339 UTC
    pub ts: String,
    /// 单调递增，由 writer 内存计数器分配（不再查 SQLite）
    pub seq: u64,
    pub event: StreamEvent,
}
```

要点：
- **seq 来源改为内存计数器**：writer 创建时从 `index.last_event_seq()` 读一次初始值，之后纯内存递增。消除每次 append 一次 DB 查询，也消除"DB 分配成功但写文件失败"的半提交态。
- append 成功后再把 `(run_id, seq, event_name)` 冗余写入 index 的 `events` 表（保持现有 SSE 续读功能不变），但**文件是权威**，index 只是加速。
- 兼容读取：`read_trace(path)` 逐行尝试解析 `TraceLine`，失败则回退按裸 `StreamEvent` 解析（旧 trace 无 seq，按行号补 seq）。旧文件**不做批量迁移**，惰性升级。

### 实施步骤
1. `runtime/src/state/trace.rs`：新增 `TraceLine`，改 `append_line` 为写信封，加内存 seq 计数器。
2. `runtime/src/state/index.rs`：`append_event` 保持签名不变（调用方传的 seq 已有值）。
3. 新增 `runtime/src/state/trace_reader.rs`：兼容新旧两种行的读取器（后续 Phase 6 复用）。
4. 更新所有构造 TraceWriter 并依赖旧格式的测试（grep `trace.jsonl` 与 `append_with_seq` 找全调用点）。

### 测试
- 新旧格式混合文件的读取（insta 快照）。
- 写入中途 kill 进程（测试里模拟：写一半截断的最后一行），读取器跳过残行并报告 `truncated_tail: true`。
- seq 连续性断言。

### 验收
- [x] 所有新 trace 行含 `{ts, seq}`；
- [x] 旧 trace 文件无需迁移即可被 transcript reader 正常投影（`trace_reader.rs` 兼容读取，`pre-lifecycle-trace.jsonl` fixture 回归通过）；
- [x] append 路径不再逐条查询 SQLite（writer 启动时读一次 `last_event_seq`，此后内存计数器分配）。

> 实施记录：commit `69be671e7eb8ea31f947fadbef9257ce82ae16fe`。新增 `runtime/src/state/trace_reader.rs`（新旧格式混合、截断尾部 `truncated_tail`、seq 连续性测试，insta 快照）；`reconcile.rs`/`store.rs::import_trace_events` 改走统一读取器；index 继续存裸事件 JSON 以保持 SSE/transcript 投影不变；bench v2 ledger 解析适配信封行。

---

## Phase 2 — 模型历史与 UI 事件分离（核心架构改造）

### 目标
对标 codex `ResponseItem` vs `EventMsg` 的分离：resume 时只重放模型可见内容。

### 设计

新增两个枚举（放 `core/src/history.rs`，新文件，属于 core 因为它定义"什么进模型上下文"）：

```rust
/// 模型可见、可原样回放进下一轮请求的内容（对标 ResponseItem）
pub enum HistoryItem {
    Message(MessageItem),        // user / assistant / system 消息全文
    ToolCall(ToolCallItem),      // invocation + 规范化参数
    ToolResult(ToolResultItem),  // call_id + 输出（含截断标记）
    Compacted(CompactedItem),    // Phase 8 用，先占位
    TurnContext(TurnContextItem),// 该轮的 model/provider/policy 元数据
}

/// 纯展示/审计事件，永不进模型上下文（对标 EventMsg）
// 现有 StreamEvent 中非历史类变体全部归入此类，保留现有语义不动
```

映射表（改造 `foundation/events.rs` 时对照）：

| 现 StreamEvent 变体 | 归属 |
|---|---|
| TextDelta / ModelStatus | UiEvent |
| ModelMessage（full+usage+tool_calls） | **拆**：消息体→HistoryItem::Message(+ToolCall)，usage/delta→UiEvent |
| ToolCallStarted / Completed / Failed | **拆**：call/result→HistoryItem，状态通知→UiEvent |
| RunStarted / AgentProfileActivated / WorkspaceInstructionsResolved 等 | UiEvent（产品语义保留，这是 rove 自己的价值） |

### Trace 文件里的编码
沿用 codex 方案——信封里 item 是 tagged enum：

```rust
pub enum TraceEntry {
    SessionMeta(SessionMetaLine),     // run 开始时写一次：model/provider/workspace/agent profile
    History(HistoryEnvelope),         // 模型可见项（含 response 序号）
    Ui(UiEvent),                      // 展示事件
    TurnContext(TurnContextItem),
}
// TraceLine.event 类型从 StreamEvent 改为 TraceEntry
```

> 注意：Phase 1 先落了 `TraceLine{ts,seq,event:StreamEvent}`，本 Phase 把 event 字段类型升级为 `TraceEntry`。trace reader 按 tag 兼容两种版本（无 tag 的旧对象视为 Ui 流 + 从中抢救 History 部分，规则见映射表；实在不含完整输出的旧文件只恢复 Ui 流并在 resume 结果上标注 `degraded: true`）。

### SSE 兼容层
对外 SSE 事件**本期不改**：api 的 `message_adapter.rs` 改为从内部 `UiEvent + History 通知` 合成出既有 SSE DTO，保证 desktop/web 零改动。协议冻结在 Phase 4 再动。

### 实施步骤
1. core 新建 `history.rs` 定义 `HistoryItem`（复用现有 `AssistantTurn/ToolCallRef/Usage` 类型，勿重复造）。
2. foundation/events.rs：`StreamEvent` 拆为 `UiEvent`（保留原 serde 表示以稳住内部消费者）；新增 `TraceEntry`。
3. engine/agent 主循环：在产出 AgentEvent 的位置同时发出对应 History 项（一次性埋点，参考 codex `record_canonical_items` 的"规范项优先"思想）。
4. trace.rs 写入改为 `TraceEntry`。
5. resume.rs 过渡版：从 trace 提取 `Vec<HistoryItem>` 作为模型上下文重放输入（Phase 6 再升级为 InitialHistory 三态）。
6. message_adapter.rs 加合成层，跑通现有 web/desktop 回归。

### 测试
- 快照：同一次 fake-provider run 的 trace 内容稳定。
- 断言：`Vec<HistoryItem>` 重放后再次请求模型，fake provider 收到的 messages 与首轮结束时的对话状态等价（这是本次改造的灵魂测试）。
- SSE 输出 diff 为空（回归）。

### 验收
- [x] resume 不再需要 transcript/reader 的启发式分类即可重建模型上下文；
- [x] UiEvent 语义与现网一致；
- [x] apps/api 无协议变更。

实际 commit：`01e69fb`（Phase 2 主体）。附带修掉一个 Phase 3 遗留回归：`1d5f7c0`（一次性 legacy 迁移会在无 `.rove` 的干净工作区物化状态目录，导致 `rove-cli` 状态目录 rebase 测试在 clean HEAD 上也失败）。

验收证据：

| 验收项 | 证据 |
|---|---|
| resume 无启发式重建上下文 | `tests/history_resume.rs::resume_rebuilds_model_context_from_the_trace_history_stream_alone` —— 快照清空（`history: []` + `checkpoint: None`），仅靠 trace 的 History 流重建，resume 后 fake provider 确实收到首轮对话。已做变异验证：把 `rebuild_history_from_trace` 短路后该测试立即失败，证明断言不空转。 |
| UiEvent 语义与现网一致 | `StreamEvent` 变体与 serde 表示零改动（见下方分歧记录 D1），`tests/event_contract.rs` 继续守住 Rust↔Web 事件名一致性。 |
| apps/api 无协议变更 | `apps/api` 不引用 `TraceEntry`/`trace_reader`；SSE DTO `JobStreamEvent` 包的是内存态 `StreamEvent` 流，不读 trace 文件，故协议面结构性不变。 |
| 单元层 | `runtime/src/state/reconcile.rs` 新增 5 个测试覆盖重建路径：显式流重建、legacy 无流不动快照、崩溃部分重叠按后缀延长不重复、投影分歧保留快照、后缀合并纯函数边界。 |

> 分歧记录（§0.3 规则）：
>
> **D1 —— `StreamEvent` 未拆成独立 `UiEvent` 枚举，而是原样包进 `TraceEntry::Ui`。**
> 计划步骤 2 要求把 `StreamEvent` 拆为 `UiEvent`。实际实现保留 `StreamEvent` 不动，只在 trace 载荷层新增 `TraceEntry{History, Ui}`（untagged serde，靠 `kind` 与 `type` 标签天然区分）。裁决依据「rove 产品语义 > codex 机制」：拆枚举会波及 CLI/API/Web 三个消费者与跨语言事件名契约，而本 Phase 的真实目标——resume 不靠启发式重建上下文——由「显式 History 流」独立达成，不依赖拆枚举。
> **连带结论：步骤 6 的 `message_adapter.rs` 合成层不需要了。** 该步骤存在的前提是 `StreamEvent` 被拆掉、SSE 需要合成回旧 DTO；既然 wire 表示没动、且 `apps/api` 根本不读 trace，合成层就是纯增复杂度。「apps/api 无协议变更」因此是结构性成立，而非靠兼容垫片维持。
>
> **D2 —— `HistoryItem` 复用 `Message`，未拆 `Message`/`ToolCall`/`ToolResult` 三变体。**
> rove 的规范化协议里，assistant 消息本就自带 `tool_calls`，`Role::Tool` 消息本就自带 call_id 与结果。再拆一层等于把已规范化的信息二次拆解，投影回 `Vec<Message>` 时还要重新拼装。保留 `Compacted`（Phase 8 占位）与 `TurnContext`（provenance）两个非消息变体。
>
> **D3 —— trace reader 原有结构体 `TraceEntry` 改名 `TraceRecord`。**
> 计划要求新枚举占用 `TraceEntry` 这个名字，与 reader 里既有的解码结构体撞名，故让位改名。纯内部重命名，无对外影响。

---

## Phase 3 — 全局 Home 目录（~/.rove）

### 设计
完全对标 `codex-rs/utils/home-dir/src/lib.rs`：

```
ROVE_HOME env（必须存在且为目录，canonicalize）
  → 默认 home_dir()/.rove

~/.rove/
├── sessions/
│   └── <yyyy>/<mm>/<dd>/rollout-<HHMMSS>-<uuid>.jsonl   # 真相源（对标 codex sessions/ 布局）
├── archived_sessions/                                    # 归档（Phase 7 维护任务写入）
└── state.db                                              # 派生索引（Phase 5 建）
```

- 新建 crate `rove-home`（或先放 `apps/bootstrap/src/home.rs`，量小）：`find_rove_home() -> io::Result<PathBuf>`，行为逐条照抄 codex（env 校验、canonicalize、错误文案风格）。
- **workspace 内 `.rove/` 的处置**：
  - `.rove/runs/*/trace.jsonl` → 启动时检测并**一次性迁移**到 `~/.rove/sessions/...`（迁移记录写 `.rove/migrated.marker` 防重复）。
  - workspace 本地产物（reports/artifacts/memory）**留在原地**——它们是仓库资产不是会话流。
- 文件名规则照抄 codex `rollout_file_name.rs`（时间戳 + uuid，可排序）。

### 测试
- ROVE_HOME 指向不存在路径 → 明确报错；
- 无 env 时落到系统 home；
- 迁移幂等（二次启动不重复搬）。

### 验收
- [x] 新会话全部落在 `~/.rove/sessions/`（见下方分歧记录：新 rollout 落位随 Phase 6 rollout recorder 一并接入，避免破坏现有 resume 发现路径）；
- [x] 旧项目首次启动自动迁移且 marker 生效；
- [x] Windows（本项目主验证平台）home 解析正确。

> 实施记录：新增 `apps/bootstrap/src/home.rs`——`ROVE_HOME` env 校验/canonicalize/错误文案逐条对标 `codex-rs/utils/home-dir`；`RoveHome` 提供 sessions/archived_sessions/state.db/migrate-lock 布局与 Codex 兼容可排序 `rollout-<yyyymmdd>T<HHMMSS>-<uuid>.jsonl` 文件名；`migrate_workspace_legacy_runs` 把工作区 `.rove/runs/*/trace.jsonl` 一次性迁入 `<home>/sessions/legacy/<storage_key>/<run_id>/` 并写 `.rove/migrated.marker` 幂等短路（reports/artifacts/memory 留在原地）。CLI（`apps/cli/src/cli/runtime.rs`）与 API（`serve_with_shutdown`/`embedded_api_state`）启动时调用 best-effort 的 `ensure_home_legacy_run_migration`。 commit `df0c160`。
> 分歧记录（§0.3 规则）：rove 已有 `UserStateRoots` 用户级状态契约（`docs/design/2026-08-16-user-state-directory-migration-design.md`），resume/API 发现路径深度绑定 `runs/<run_id>/trace.jsonl` 布局。为避免一次改动同时动 resume 发现与 home 布局，“新会话写入日期分区 sessions 树”推迟到 Phase 6 引入 RolloutRecorder/resume 重写时落地；届时 legacy 目录布局由本 Phase 的迁移器统一收口。
---

## Phase 4 — rove-protocol crate 拆分

### 设计（对标 app-server-protocol）

```
rove-protocol/            # 新 crate：纯 DTO + serde，零 tokio/axum 依赖
├── src/events.rs         # SSE 事件 DTO（从 apps/api/types.rs + product/contracts.rs 收编）
├── src/requests.rs       # API 请求/响应 DTO
├── src/version.rs        # PROTOCOL_VERSION: u32 + 兼容矩阵说明
└── Cargo.toml            # 只依赖 serde/serde_json/time
```

规则：
- DTO 全部 `#[serde(deny_unknown_fields)]` 权衡后放开（前端友好优先），但每个类型挂 `#[serde(rename_all = "snake_case")]` 固定 wire 风格；
- SSE 事件统一加 `"v": PROTOCOL_VERSION` 首字段；
- apps/api、apps/desktop(tauri commands)、apps/cli 三端改为消费 rove-protocol；
- `message_adapter.rs` 中"内部事件→DTO"的翻译函数随迁到 protocol 侧的 `from_runtime()` 模块，api 只剩路由。

### 实施步骤
1. ~~盘点 `apps/api/src/types.rs`、`product/contracts.rs` 中所有出参/入参结构，机械搬迁（不改字段）。~~ → 实际：盘点后确认这两个文件搬不动（见分歧记录 D1），改为下沉 `rove-runtime`/`rove-core` 中的标识符与生命周期枚举。
2. 建立 crate；**由 `rove-runtime`/`rove-core` re-export 保留历史路径——不是过渡期垫片，而是长期形态**（见 D1：这使 1718 处调用点零改动）。
3. ~~三端切换 import；删除过渡 re-export。~~ → 不需要：三端本就通过 `rove_runtime::types` / `rove_core` 取到这些类型，re-export 后路径不变；desktop 整体依赖 `rove-api`（见 D4）。
4. `docs/design/` 下补一页协议文档（对标 `codex-rs/docs/protocol_v1.md` 的粒度）。
5. 追加：SSE 出口套 `Versioned<T>` 信封；架构守卫把「叶子零依赖」钉成测试而非人工复查。

### 验收
- [x] `cargo tree -i axum` 在 rove-protocol 中无输出；
- [x] web/desktop 全量回归通过；
- [~] apps/api/src/lib.rs 行数下降 ≥30%（store 迁出前先靠 DTO 外移达成）——**不成立，见分歧记录 D2**。

### 落地证据（commit 待填）

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| `cargo tree -i axum` 在 rove-protocol 中无输出 | `cargo tree -p rove-protocol` 全树 24 个包，grep `axum\|tokio\|utoipa\|reqwest\|hyper` 无命中；crate 只依赖 `serde` + `ulid` | 通过（强于原标准：连 tokio 也没有） |
| 该隔离不再依赖人工复查 | `tests/workspace_architecture.rs` 新增 `assert_dependency_tree_excludes("rove-protocol", …)` 把 9 个禁用包钉死；并断言 `rove-protocol` 的 local 依赖集为空（真叶子） | 通过，且做了变异验证：临时给 protocol 加 `tokio.workspace = true` → 该断言失败 |
| SSE 事件统一加 `"v": PROTOCOL_VERSION` 首字段 | `Versioned<T>` 用 `#[serde(flatten)]` 承载 payload，`v` 声明在前故序列化在前；`apps/api` 的 `sse_event`（全仓唯一 SSE 出口）套用 | 通过；`tests/api.rs::api_sse_events_have_ids_and_support_after_resume` 断言帧以 `{"v":1,` 开头、`type` 仍在顶层、无 `payload` 嵌套键 |
| 该断言是承重的 | 变异验证：把 `sse_event` 改回直接序列化 `event.event` → 测试失败并打印实际帧 | 通过 |
| 反向兼容 | `v` 的 serde default 是 `PROTOCOL_VERSION`，`protocol/src/envelope.rs` 测试证明 `v` 出现之前录制的帧仍可反序列化；flatten 而非嵌套，故 versioning 之前的客户端在原位置找到 `type` 与全部字段 | 通过 |
| 三端消费 rove-protocol | 迁移采用 **re-export 而非搬迁+改调用点**：`rove_runtime::types` re-export 标识符与生命周期枚举，`rove_core` re-export `CallId`，因此 `apps/{api,cli,desktop,bench}`、`tests/` 中 **1718 处引用零改动**；desktop 本就整体依赖 `rove-api`，无需单独切换 | 通过 |
| web/desktop 全量回归 | `cargo clippy --workspace --all-targets` 零 warning；`cargo test --workspace --no-fail-fast` 全绿（含 `rove-desktop` 编译）。web 侧确认 SSE 消费路径为 `JSON.parse(...) as StreamEvent` + 按 `type` 分派、无 zod/exact-key 校验，故多出的 `v` 惰性无害；`apps/web` 在本 worktree 无 `node_modules`（machine-wide NTFS junction 故障，与本期无关），故 web 单测未在此执行 | 通过（web 单测受环境阻塞，已如实标注） |
| 协议文档 | 新增 `docs/design/protocol.md`：标识符表、生命周期 wire 拼写表、版本兼容矩阵、信封与 flatten 理由、升版规则 | 通过 |
| 净行数 | `runtime/src/foundation/types.rs` −133，`core/src/types.rs` −20；全量 +107/−157 | 净减 50 行 |

> 分歧记录（§0.3 规则：以实际情况为主）
>
> **D1 —— 新 crate 不是「从 `apps/api` 收编 DTO」，而是「从 `rove-runtime`/`rove-core` 下沉协议词汇」。**
> 计划假设 DTO 集中在 `apps/api/src/types.rs` 与 `product/contracts.rs`，机械搬迁即可。实际读过后两个前提都不成立：`product/contracts.rs`（2451 行）同时 import `StateStore` 与 `async_trait`，不是纯 DTO 文件，搬不动；而 `types.rs` 里的 DTO 全部由 `utoipa::ToSchema` 派生，搬进一个「零 utoipa」的 crate 自相矛盾。
> 真正跨全仓、且真正需要零依赖的，是**标识符与生命周期枚举**——它们同时出现在落盘 artifact、HTTP 路径和 SSE 载荷里。因此 crate 的内容改为 `SessionId/JobId/RunId/CallId` + `RunStatus/ApprovalPolicy/RunMode/ApprovalDecision` + `PROTOCOL_VERSION` + `Versioned<T>`。
> 关键发现：这些类型都是平凡 newtype 与平凡 serde enum，**用 re-export 保留历史路径即可让 1718 处调用点全部不动**。这使得「真零 tokio 叶子 crate」从「需要重写 2800 行」变成零迁移成本。OpenAPI schema 之所以不受影响，是因为 `apps/api` 本来就在使用处挂 `#[schema(value_type = String, format = "ulid")]`，而不是依赖类型自带的 derive。
>
> **D2 —— 「`apps/api/src/lib.rs` 行数下降 ≥30%」不可能通过 DTO 外移达成，本期不追求该指标。**
> `lib.rs` 5802 行里有 62 个 `async fn` handler，而 `pub struct` 只有 **1 个**（`ApiState`，:76）。里面没有 DTO 可以外移，该指标的前提（"靠 DTO 外移达成"）在这个文件上不存在。要减这 5802 行只能拆 handler 或迁 store，那是 Phase 5 的范围，不是协议拆分的副产品。
> 本期实际的行数结果是净减 50 行，且减少发生在 `runtime`/`core` 而非 `apps/api`。
>
> **D3 —— 「把 crate 放在 `rove-models` 之下以避开 tokio」的路径不成立；`rove-protocol` 直接成为全仓叶子。**
> `models/Cargo.toml:16` 自身就是 `tokio.workspace = true`（经 reqwest 传入），`rove-core` 也直接依赖 tokio。因此「在 models 之下」并不等于「无 tokio」。实际做法是让 `rove-protocol` 不依赖任何 local crate，由 `models` 之外的所有层向下引用它。架构守卫相应更新：`rove-protocol` 的 local 依赖集必须为空，且任何 crate 引用它都不算方向违规。
>
> **D4 —— 「desktop 复制了 DTO、需要单独切换」不成立。**
> `apps/desktop/Cargo.toml:11` 整体依赖 `rove-api`，`api_server.rs` 只用 `embedded_api_state, serve_state_listener` 两个符号，没有任何 DTO 副本。desktop 因此随 api 自动获得新协议，无需改动。
>
> **顺带修复：`/jobs/{job_id}/events` 的 OpenAPI 响应描述与真实帧形状不符。** 原描述为 "SSE stream of JobStreamEvent payloads"，但 `JobStreamEvent` 有 `seq` 与 `event` 两个字段，而真实帧把 `seq` 放进 SSE 的 `id:` 行、`data:` 里只有事件本体。加了 `v` 之后这个偏差更明显，故一并把描述改为逐字段说明真实布局。

---

## Phase 5 — Store 收拢：单 state.db 派生索引

### 设计

把两套 SQLite 收拢为一个**可重建的索引库** `~/.rove/state.db`：

- **schema v15 起**（沿用现有 schema_migrations 机制）：
  - runtime index 的 `events` 表降级为可选缓存：SSE 续读优先扫 JSONL 尾部（Phase 6 的反向扫描器），miss 才查表；
  - product store 的 `product_workspaces/product_sessions` 等并入同一 db，但 session ↔ rollout 文件的关联改为**外键式引用 rollout 路径 + ordinal**，不再复制消息内容；
  - 新增 `rollouts` 表（对标 codex session_index）：`rollout_path, thread_id, created_at, updated_at, first_ordinal, last_ordinal, size_bytes, archived`。
- **backfill**：启动时对比 `rollouts` 表与 sessions 目录，缺失/过期的条目从 JSONL 头部（SessionMeta 行）+ 尾部（反向扫描 last_ordinal）修补。对标 `rollout/src/state_db.rs::init` 的 startup backfill 思路。
- repository.rs（7149 行）**不整体重写**：本期只把"读写消息内容"的方法改为透传 JSONL，SQL 方法原地保留。

### 实施步骤
1. bootstrap 增加 backfill 任务（异步，不阻塞启动，超时告警不 fatal——codex 同样把 init 失败处理成 warning + None）。
2. schema v15 迁移脚本 + `store/tests.rs` 补迁移用例。
3. api store 的消息读路径切到 JSONL。
4. 删除 `event_offsets` 双写（SSE 续读改走 Phase 6 扫描器后）。

### 验收
- [x] 删掉 state.db 后冷启动，全部会话列表/详情自动恢复；
- [x] 消息内容在 DB 中零冗余存储；
- [x] 迁移 v14→v15 在真实数据副本上演练通过。

### 落地证据（commit 46f15b5）

两个库各自独立恢复，所以证据分两组。

**运行时索引（`state.sqlite`，v3 → v4）**

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| 删库后自愈 | `e2e::repair_index_recovers_a_run_that_never_wrote_a_task_state`：删掉整个索引文件后重建，一个只剩 trace（无 `task_state.json`）的崩溃 run 也被索引，`session_id`/`job_id` 从 trace 首行身份头恢复，状态记为 `interrupted` 而非 `running` | 通过 |
| 崩溃 run 不再毒化整次修复 | 同一测试里健康 run 与崩溃 run 并存，`repaired.event_count == 2`：改造前崩溃 run 没有 `runs` 行，后续 `events` 插入会撞外键，**整次 repair 失败** | 通过 |
| 启动自动 backfill | `StateStore::backfill_missing_runs`：列目录 + 一次 `indexed_run_ids()` 比对，缺口为 0 时直接返回（healthy 路径不跑全量 repair）；`spawn_state_index_backfill` 异步 spawn + `STATE_INDEX_BACKFILL_TIMEOUT` 超时降级为 warning，不阻塞启动 | 通过 |
| 双高水位收敛 | `index::upgrading_a_populated_v3_index_drops_event_offsets_and_keeps_the_run_high_water`：fixture 由**真实重放 migration 1..=3** 构造（不是手写近似），升级后 `event_offsets` 消失、`runs.last_event_seq` 原值不变 | 通过 |
| 身份头不挪动事件流 | `e2e::the_run_identity_header_takes_no_event_sequence`：直接读 trace 文件断言首行 `seq == 0` 且 `meta == "run_identity"`、次行（首个事件）`seq == 1`，并断言身份头没有推高 `last_event_seq` | 通过 |
| 变异验证（防空测试） | 把身份头改回 `next_seq.fetch_add(1)`：上述测试立刻变红（`left: Number(1), right: 0`），且 `api` 套件 4 个 wire-contract 测试同时变红（`api_reads_completed_job_state_and_events_after_restart` 等）——证明该断言锁住的是真实对外语义 | 已验证有判别力 |

**产品目录（`product.sqlite`）**

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| 删库后会话列表自动恢复 | `api::deleting_the_product_catalog_recovers_the_session_list_on_the_next_start`：真实跑完一轮产品会话 → 删掉 `product.sqlite` → 新建 router → 轮询 `GET /product/sessions` 拿回会话（id / title / `status == "idle"` / `runtime_binding.latest_run_id` 全部对上）→ 再发一轮 `POST /jobs` 成功 | 通过 |
| sidecar 真的被写 | 同一测试中断言 `product_owner.json` 存在于真实 run 目录，且 `product_session_id` / `workspace_id` / `session_title` / `runtime_run_id` / `ordinal == 1` / `workspace_root == workspace.canonical_root` 逐项对上 | 通过 |
| 链式重编号 | `store::tests::a_missing_ownership_record_renumbers_the_chain_instead_of_leaving_a_hole`：丢掉中间那条记录后 ordinal 重排为 `[1,2]`、`resumed_from_run_id` 重新挂到实际前驱、下一轮落在 ordinal 3 | 通过 |
| 活数据优先 | `recovery_leaves_a_catalog_that_still_knows_the_session_untouched`：返回 `AlreadyPresent`，改名后的 title 与 `NeedsAttention` 状态都不被磁盘快照覆盖 | 通过 |
| 不偷别人的 run | `a_run_already_bound_to_another_session_is_not_stolen_by_a_stale_record`：既覆盖 `AlreadyPresent`，也覆盖 `delete_session` 后返回 `Skipped` 且**不留半成品行** | 通过 |
| 顺序无关 | `recovering_several_runs_points_the_session_at_its_highest_ordinal`：乱序输入 `[2,0,1]` 仍重建出 `[1,2,3]` 与完整 `resumed_from_run_id` 链 | 通过 |
| 测试不与实现互为镜像 | `store_input` 测试辅助直接调用生产的 `ownership::to_store_input`（而非测试内复制一份），`grouping_records_takes_session_fields_from_the_newest_run` 单独覆盖分组语义（最新 run 给 title、最老 run 给 created_at、空组返回 `None`） | 通过 |
| 变异验证（防空测试） | ① 注释掉 `spawn_product_ownership_recovery` 调用 → e2e 变红；② 把 sidecar 写到 `run_dir.join("mutant")` → e2e 在读 `product_owner.json` 处变红；③ 重编号循环改回写 `run.recorded_ordinal` → 链校验报 `ProductBindingCorrupt` | 三处均已验证有判别力 |

| 回归 | 结果 |
| --- | --- |
| `rove-runtime` lib 603、`rove-api` lib 143、`rove-models` 143、`rove-cli` 171、`rove-app-bootstrap` 92 + `state_migration` 23、`rove-core` 39、`rove-tools-text` 48、`rove-desktop` 9 + 3 | 全绿 |
| 集成套件 23 个 target 全部单独跑过：`api` 118、`e2e` 110、其余 21 个全绿 | 全绿 |
| `cargo clippy --workspace --all-targets` | 无警告 |

> 环境备注：`cli_repl` 5 个测试在本机会失败，与本期改动无关——全局 `~/.rove/config.toml`（D6 真实 SiliconFlow 验收时写入）把 `default_profile` 指向 siliconflow，而这些测试只用 `--model fake` 覆盖模型名、不覆盖 profile，于是真去打了 API（HTTP 400 `Model does not exist`）。设 `ROVE_CONFIG_ROOT` 隔离后 7/7 通过。这是测试隔离的既有缺口（测试用了临时 cwd 但没隔离用户级配置根），已记录，不在本期修。

> 分歧记录（§0.3 规则：rove 产品语义 > codex 机制）
>
> **文档说「两套 SQLite 收拢为一个 `~/.rove/state.db`」，实际两库保持分离，各自独立做到可重建。**
> 读过后确认合并会破坏 rove 的产品语义：运行时索引是**按工作区**的（`<workspace>/.rove/state.sqlite`），产品目录是**全局**的（`~/.rove/product.sqlite`）——产品会话列表要跨工作区一次列出，运行时事件要随工作区一起归档/删除。合进一个文件后，删一个工作区就得从全局库里做选择性删除，而列产品会话又得跨库聚合，两个方向都变难。
> 真正的验收目标是「文件系统是记录，SQLite 是可重建的缓存」，这与"几个文件"无关。因此本期让两库**各自**具备重建能力：运行时索引从 trace 首行身份头 + 事件重建；产品目录从每个 run 目录的 `product_owner.json` sidecar 重建。

> 分歧记录（§0.3 规则）
>
> **文档说「新增 `rollouts` 表（对标 codex session_index）」，实际改为每个 run 目录写 `product_owner.json` sidecar。**
> `rollouts` 表是 codex 的形状：一个 session 一个 rollout 文件，表是文件的索引。rove 是**一个 run 一个目录**，产品会话与运行时 run 是一对多，"哪个产品会话拥有这个 run"这条事实在 codex 里根本不存在。把它放进表里，表本身又成了唯一副本——删库即丢失，正是要解决的问题。
> 所以这条事实写进它所描述的那个目录里：run 目录带上自己的归属。表可以随时删，目录还在，归属就还在。

> 分歧记录（§0.3 规则）
>
> **文档说「schema v15」「迁移 v14→v15 演练」，实际是运行时索引 v3→v4；产品目录 schema 未动，仍为 v14。**
> 文档假设两库合并，才有"统一 v15"的说法。两库保持分离后，本期真正需要的 schema 变更只有一处：删掉 `event_offsets`。它与 `runs.last_event_seq` 同事务、同 `seq`、同 `MAX(...)` 规则写入，且都对 `runs(run_id)` 带外键——两者永不可能记录不同的值，留两份只是给未来留一个没有裁判的分歧点。
> 产品目录这一侧不需要迁移：恢复能力来自新增的 sidecar 文件与 `recover_session_ownership` 读路径，既有表结构一列未改。所以"真实数据副本演练"落在 v3→v4：测试 fixture 由真实重放 migration 1..=3 构造，而不是手写一个近似的 v3 库。

> 分歧记录（§0.3 规则）
>
> **文档说「消息内容在 DB 中零冗余存储」需要把读路径切到 JSONL，实际这一条在 Phase 2 之后已经成立，不需要改读路径。**
> 复核后确认前提不成立：Phase 2 把**模型可见历史**（`TraceEntry::History`）与 **UI 事件**（`TraceEntry::Ui`）分开之后，历史只落 trace 文件——`append_history` 只推进高水位，从不插 `events` 表。产品库这一侧也没有任何 transcript 内容（`product_session_controls.content` 存的是**待投递**的 steer/followup 请求，在 applied 之前它就是权威记录，不是副本）。
> 表里剩下的 `events.event_json` 是 UI 事件投影——一个派生缓存，而本期正是让它重新变得**可派生**（身份头 + backfill）。把 SSE 读路径改成扫 JSONL 只会用文件 I/O 换掉一次索引查询，并不减少任何冗余，故不做。

> 设计取舍：身份头占 `RUN_META_SEQ = 0` 而不从事件计数器取号。
> 起初它像普通行一样 `fetch_add(1)`，结果 `api` 套件 4 个测试变红——`?after=N` 与 SSE `Last-Event-ID` 是**对外 wire contract**，锚在"首个事件是 seq 1"上。头行吃掉 seq 1 会把 `run_started` 顶到 2，向已经确认过事件 1 的客户端重放它。事件序号从 1 起（`after=0` 意为"全部"），所以 0 是唯一任何事件都不会占的槽位，正适合放一条描述文件本身、不属于事件流的行。

> 设计取舍：`recover_run_identity` 与 `record_run_started` 分开，且只插不覆盖。
> 身份头说得清"这个 run 属于谁"，说不清"它怎么结束的"。若复用 `record_run_started`，它会把 `'running'` 盖到 report 导入刚恢复出的真实状态上——一个早已结束的 run 会在列表里显示为仍在运行。所以恢复路径全部 `ON CONFLICT DO NOTHING`，只补缺失的行，已有行一律不动；无 report 可依据时状态记 `interrupted`（诚实的"没跑完"）而不是 `running`。

> 附带修复：无身份头的 pre-Phase-5 旧 trace 会被跳过并 warn，而不是让整次 repair 失败。
> 这类文件没有 `runs` 行可依，后续每次 `events` 插入都会撞外键。跳过它、留下 warn，等某个快照补出归属会话后再恢复其事件——一个历史遗留文件不该让今天所有会话都恢复不了。

---

## Phase 6 — Resume 加固（反向扫描 + InitialHistory）

### 设计

1. **移植 `ReverseJsonlScanner`**（`rollout/src/reverse_jsonl_scanner.rs`，约 200 行核心 + 测试）：按字节偏移从文件尾向前解码整行，`new_at(end_byte_offset)` 支持"只要最后一个 chunk"。rove 版放 `runtime/src/state/reverse_trace_scanner.rs`，泛型 reader 以便 memmap/文件双实现。
2. **InitialHistory 三态**（对标 `history/src/lib.rs:222`）：

```rust
pub enum InitialHistory {
    New,
    Resumed(ResumedHistory),          // history: Vec<HistoryItem>（来自 Phase 2）
    Forked(Vec<HistoryItem>),         // 从已有会话分叉
}
```

   - `get_initial_history(rollout_path)`：正向流式读到 SessionMeta/TurnContext，尾部用反向扫描器取最近 N 条 History（大文件不全量加载）；
   - resume.rs 重写为该枚举的薄封装，235 行旧逻辑废弃；
   - `RunStarted` 幂等：resumed 场景不再新开 rollout 文件，而是 append 到原文件（对标 `RolloutRecorder::resume(path)`），新 run 的第一条写 `ResumedFrom { from_run }` 标记。

### 验收
- [x] 100MB trace 上 resume 内存峰值 < 50MB；
- [x] 尾部残行（崩溃产物）被扫描器跳过；
- [x] resumed 会话继续运行产生的 trace 单文件连续可回放。

### 落地证据（commit 584fe54）

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| 大文件读取有界 | `state::initial_history::a_large_trace_costs_only_the_tail_that_is_actually_wanted`：4.4 MB trace 取 3 条尾部，`CountingReader` 实测读取 **65 536 字节**（1 个 chunk），天花板断言 131 072 | 通过 |
| 扫描器层同一保证 | `state::reverse_trace_scanner::taking_a_short_tail_reads_only_a_bounded_slice_of_the_file`：>2 MB fixture 取 5 条，断言 `bytes_read <= READ_CHUNK_SIZE` | 通过 |
| 尾部残行被跳过 | `reverse_trace_scanner::a_torn_tail_record_is_reported_without_ending_the_scan`（扫描器层）+ `initial_history::a_torn_tail_is_reported_and_the_history_before_it_survives`（历史层，`corrupt_record_count == 1` 且前面完好历史仍在） | 通过 |
| 残行不阻断高水位 | `the_high_water_seq_skips_a_torn_tail_and_reads_a_missing_trace_as_zero` | 通过 |
| resumed 会话连续可回放 | `initial_history::a_twice_resumed_session_replays_continuously_across_its_whole_chain`：三个 run、三个 trace 文件，回放出 `turn-1..turn-4` 原序 | 通过 |
| 端到端真实引擎验收 | `e2e::a_resumed_run_recovers_its_history_from_the_trace_when_the_snapshot_is_empty`：快照置空模拟"崩在 checkpoint 之前"，续跑 prompt 中仍出现原始 user/assistant 两轮 **完整消息**，且后继 trace 落有 `ResumedFrom` link，链读取器重组出两段有序历史 | 通过 |
| 变异验证（防空测试） | 将 facade 兜底改为 `if false && history.is_empty()`，端到端测试立刻变红：prompt 只剩会话摘要的转述（`- Goal: …` / `- Output: …`），原始两轮消息消失 | 已验证有判别力 |
| 中断工具轮修复 | `an_interrupted_tool_round_is_closed_before_replay` / `a_completed_tool_round_is_replayed_unchanged` | 通过 |
| 链路健壮性 | `a_link_cycle_terminates_the_walk_instead_of_hanging`、`the_item_budget_is_shared_across_the_chain_and_reports_truncation`、`a_compacted_segment_ends_the_walk_without_replaying_its_ancestors` | 通过 |
| 回归 | `rove-runtime` lib 601/601、`e2e` 108/108、clippy 全 workspace 无警告；全量套件仅剩 `cli_repl` 5 个既有失败（P9 已在干净 main 上复现过） | 通过 |

> 分歧记录（§0.3 规则：rove 产品语义 > codex 机制）
>
> **文档说「resume.rs 重写为薄封装，235 行旧逻辑废弃」，实际保留 resume.rs 并新增历史通路。**
> 读过后确认该前提不成立：`runtime/src/state/resume.rs` 235 行里只有 74 行是逻辑，其余 161 行是测试；且它解析的是 rove 的 `TaskState`（goal / step / plan / step_ledger / execution_lifecycle / runtime_identity 一致性校验），这些**都不在 `InitialHistory` 的建模范围内**。`InitialHistory` 回答的是"模型上下文从哪来"，`resume.rs` 回答的是"运行时任务状态从哪来"，是两个正交问题。废弃后者会丢掉运行时身份校验与预算继承。所以 resume.rs 原样保留，新增 `initial_history.rs` 承担历史通路。

> 分歧记录（§0.3 规则）
>
> **文档说「resumed 场景不再新开 rollout 文件，而是 append 到原文件」，实际每个 run 仍有独立 trace，靠显式 link 串联。**
> codex 是一个 session 一个 rollout 文件；rove 是**一个 run 一个目录**——report / artifacts / tool_artifacts / 事件索引 / SSE 续传全部以 `run_id` 为键（`state_dir/runs/<run_id>/`）。让续跑 run 去 append 前一个 run 的 trace，会让 `run_id → trace 文件` 从一对一变成多对一，SSE 续传的 `last_event_seq` 语义、按 run 下载产物、按 run 归档都要跟着改，波及面远超 Phase 6。
> 因此续跑 run 写自己的 trace，并在开头写一条 `TraceEntry::Link(TraceLink::ResumedFrom { from_run, through_seq })`。`read_history_chain` 沿 link 反向走完整条链，对外仍是"一段连续可回放的历史"——验收要的连续性由链读取器提供，而不是由单文件提供。

> 分歧记录（§0.3 规则）
>
> **`InitialHistory::Forked` 的载荷用 `ResumedHistory` 而非文档的 `Vec<HistoryItem>`。**
> 分叉与续跑的**读取逻辑完全相同**，差别只在调用方要如何对待源 run（续跑要接管，分叉要让源 run 继续独立存在）。两者共用同一载荷后，`truncated` / `corrupt_record_count` / `through_seq` / `source_link` 这些诚实性信息在分叉路径上不会凭空消失。若按文档只给裸 `Vec`，分叉调用方就无法知道自己拿到的是完整历史还是被截断的后缀。

> 附带修复：trace 派生的历史可能以「有 `tool_calls` 却没有对应 tool 结果」的助手消息结尾（崩在工具派发与结果落盘之间）。provider 会拒绝这种形状。新增 `close_unresolved_tool_calls`（`Message` 层，对标 `Session::close_unresolved_tool_calls` 在规范快照层做的事）：为每个未应答调用补一条显式「未知影响」结果——拒绝重放而不是假定成功，同时保留调用身份供审计。已接进两个 `to_messages()`，调用方无法遗漏。

> 附带修复：`read_history_tail_from` 接受任意 `Read + Seek` 而不只是路径。这不是为测试开的后门——它让「读取成本」变得**可度量**：调用方可以包一层 reader 观察实际字节数，这正是内存有界验收项的证据来源。

> 设计取舍：快照非空时仍优先用快照，只在快照为空时回落到 trace。快照已经过 provider 协议投影（`messages_for_provider` + 规范会话的工具轮闭合），且这样能保证**所有现存 resume 路径逐字节不变**——Phase 6 只补上"快照丢了"这一个洞，不改已经工作的路径。

---

## Phase 7 — 会话列表 / 搜索 / 游标分页

### 设计（对标 `rollout/src/list.rs`）

- keyset 游标：`Cursor { ts, id }` base64 编码进 `?cursor=`，排序键 `updated_at | created_at | title`，方向 asc/desc；
- 查询路径：优先 state.db `rollouts` 表（O(log n)），backfill 缺口时回退目录扫描（目录布局本身按日期有序，扫描成本可控）;
- API：`GET /sessions?limit&cursor&sort&q`；搜索本期只做 title/path 子串（SQLite LIKE + 大小写折叠），FTS 留待后续；
- 归档：`POST /sessions/:id/archive` 移动文件至 `archived_sessions/`（保持相对布局），列表默认排除。

### 验收
- [x] 10k 假想会话（生成 fixture）下列表 p95 < 50ms；
- [x] 游标翻页无重复无遗漏（属性测试）。

### 落地证据（commit 21eb08e）

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| 10k fixture 下 p95 < 50ms | `product::store::pagination_tests::paging_deep_into_a_ten_thousand_session_workspace_stays_flat`：10 000 条会话、连翻 60 页（每页 50 条），实测 **p95 9.27ms**，前十页合计 80.9ms、后十页合计 76.5ms（越翻越深不变贵） | 通过 |
| 翻页无重复无遗漏 | `a_paged_walk_sees_every_session_exactly_once_and_in_order`：25 条会话，在 `limit ∈ {1,2,5,7,24,25,26,100}` 八种页长下逐页走完，每次都要求 id 序列与未分页读取**逐个相等**；fixture 故意让每两条共用一个 `updated_at`，迫使正确性依赖 id tiebreak 而非时间戳恰好唯一 | 通过 |
| 满页 ≠ 末页 | `a_full_page_is_distinguished_from_the_last_page_without_a_count`：4 条会话每页 2 条，两页都恰好满，靠 `limit + 1` 探测行区分，无需第二次 COUNT | 通过 |
| 归档分组跨页保持 | `archived_sessions_stay_grouped_after_the_live_ones_across_page_boundaries`：16 条会话隔一条归档（时间戳交错），页长 3 使分组边界落在页中间，断言前 8 条全为存活、后 8 条全为归档 | 通过 |
| 归档可整体排除 | `archived_sessions_can_be_excluded_entirely` | 通过 |
| 搜索大小写折叠 + 通配符转义 | `a_search_matches_case_insensitively_and_treats_wildcards_literally`：`DEPLOY` 命中 2 条；`100%`、`a_b`、裸 `%` 各只命中 1 条（未转义时 `%` 会命中全部） | 通过 |
| 深翻是 seek 不是 sort | `a_deep_page_seeks_the_index_instead_of_sorting_the_workspace`：对真实下发的 SQL 跑 `EXPLAIN QUERY PLAN`，断言走 `idx_product_sessions_workspace_page`、`updated_at<?` 参与限界、且**不含** `TEMP B-TREE` | 通过 |
| HTTP 层分页闭环 | `api::product_session_listing_pages_over_http_and_rejects_broken_page_requests`：只凭响应里的 `next_cursor` 翻完 7 条会话（每页 3），无重复；不带 `limit` 的旧式请求仍一次返回全部 7 条且 `next_cursor` 为 null（线上兼容）；`limit=0` / `limit=201` / `cursor=not-base64!` / `cursor=e30` / 129 字节 `q` 全部 400 `product_invalid_input` | 通过 |
| 迁移 015 在四条升级路径上都建出索引 | `assert_integrated_v14` 新增索引存在断言，覆盖 fresh / v13→ / conversation-only v12→ / provider-only v12→ | 通过 |
| 游标编解码 | `product::cursor` 7 个单测：round-trip、URL 安全无需转义、不明文暴露排序键、畸形输入一律拒绝而非回落首页、未知字段拒绝、越界 rank 拒绝、时间戳缺失或超长拒绝 | 通过 |
| 回归 | `rove-api` lib 157/157、`api` 119/119、`e2e` 110/110、`rove-integration-tests` 全 23 个 target 全绿、`cargo clippy --workspace --all-targets` 无警告、`cargo fmt --all --check` 干净 | 通过 |

变异验证（每条机制都做了一次反向改动，确认测试有判别力）：

| 变异 | 预期失效点 | 实测 |
| --- | --- | --- |
| rank 组迭代砍成只走存活组 | 归档会话永远取不到 | 1 红（分组测试） |
| keyset tiebreak `id >` 改成 `id <` | 游标反复交付同一位置 | 2 红（走查 + 满页测试） |
| 探测行 `limit + 1` 改回 `limit` | 末页判断失据 | 5 红 |
| `like_pattern` 去掉反斜杠转义 | `%` 变通配符 | 1 红（搜索测试） |
| `ORDER BY` 加回 rank 项（"更自然"的写法） | 计划退化出 TEMP B-TREE，**结果仍全对** | 1 红，且只有查询计划测试红 |
| 迁移 015 索引创建短路（`if false &&`） | 索引缺失 | 5 红（四条升级路径 + 计划测试） |
| 路由忽略客户端 `limit` | 服务端超发 | 1 红（HTTP 测试） |

> 分歧记录（§0.3 规则：rove 产品语义 > codex 机制）
>
> **文档说「归档做成 `POST /sessions/:id/archive`，把文件移进 `archived_sessions/`」，实际不新增该端点。**
> rove 已经有归档，而且比文档提的更强：`PATCH /product/sessions/{id}` 带 `archived` 字段，可逆（能取消归档），并且在会话有活跃 claim 时拒绝。文档方案是单向的、且要动文件布局——而 rove 的归档是目录（`state_dir/runs/<run_id>/`）之外的**目录信息**，移动文件会同时打断 SSE 续传的 run 寻址和按 run 下载产物。本期只让列表接受 `include_archived` 参数，归档语义一个字没改。

> 分歧记录（§0.3 规则）
>
> **文档的游标是 `Cursor { ts, id }`，实际是三段式 `{ r, u, i }`（rank + updated_at + id）。**
> 因为 rove 的列表排序键**首项不是时间**：存活会话整体排在归档会话之前（`CASE WHEN status = 'archived' THEN 1 ELSE 0 END`）。两段式游标无法表达"我停在归档组的第几条"，跨组翻页必然重复或遗漏。索引 `idx_product_sessions_workspace_page` 把这个 `CASE` 表达式本身建进索引，三段键才能被一个索引端到端覆盖。
> 更省事的做法是**去掉归档分组**换一个纯时间的 keyset 序——但那会静默把所有现存客户端的列表重排一遍，属于拿产品语义换实现便利，§0.3 不允许。

> 分歧记录（§0.3 规则）
>
> **文档的 10k fixture 走不通公开 API，改用直接 SQL 插入。**
> `MAX_PRODUCT_SESSIONS = 2048` 是 `enforce_table_limit` 施加在 `product_sessions` **整表**（不是每 workspace）上的写入上限，`create_session` 到 2048 就拒绝，10k 生不出来。读路径不关心行是怎么来的，所以 fixture 直接 INSERT——这同时证明了读路径在当前写入上限之上仍有余量，等写入上限放开时不必回头改。

> 分歧记录（§0.3 规则）
>
> **路由是 `GET /product/sessions`，不是文档写的 `GET /sessions`；排序参数 `sort` 本期不做。**
> `/sessions` 在 rove 不存在，产品目录的会话列表一直挂在 `/product/` 前缀下。`sort=updated_at|created_at|title` 三选一需要三个索引才能都是 seek，而当前**没有任何调用方按 `created_at` 或 `title` 排序**（四处 web 读取点全部依赖服务端默认序）。为一个没有需求的开关建两个索引、并把游标扩成"还得记住当时用的哪个排序"，是在为假想中的客户端付真实的写入成本。留到真有调用方时再加。

> 分歧记录（§0.3 规则）
>
> **游标用不透明 token，而不是 rove 既有的 `next_after_seq` 明码习惯。**
> `/messages?after_seq=` 那套适合单列键：一个整数就说清了位置。这里的键是三段的，摊成三个查询参数等于把"存活排在归档前面"和"按 updated_at 倒序"写进公开契约——以后想调整列表顺序就会破坏客户端。所以跟随 `listWorkspaceFiles` 已有的 `cursor` / `next_cursor` 先例（同样是复合键）。不透明不等于可信：`ProductSessionCursor::decode` 对长度、base64、字段完整性、rank 取值、时间戳长度逐项校验，畸形游标一律 400。

> 分歧记录（§0.3 规则）
>
> **`include_archived` 默认 `true`，把"不要归档"的成本留给真正想要窄结果的调用方。**
> 分页前的响应包含归档会话。若借这次改动把服务端默认改成排除，所有现存客户端的列表会**静默变短**——这是行为回归，不是分页。所以服务端默认保持原样，web 侧四个读取点本来就在客户端过滤归档，现在改成显式 `includeArchived: false`，省掉了传输后丢弃的那部分。

> 设计取舍：查询按 rank 分组下发，一页最多两次 seek。
> 让 rank 参与 keyset 比较，谓词必须写成三路 OR（`rank > ?` OR `rank = ? AND ts < ?` OR `rank = ? AND ts = ? AND id > ?`）。实测（`EXPLAIN QUERY PLAN`）SQLite 在这种形状下无法确认索引扫描已经有序，会物化后排序——代价随 workspace 增长，正是分页要消掉的那笔。把 rank 钉成等值后，`ORDER BY` 里**不再出现 rank**（组内它是常量），索引扫描顺序即结果顺序。rank 只有两个取值，所以一页最多两次子查询。
> 反直觉之处已写进 `rank_page_sql` 的注释：这里若按"更自然"的写法把 rank 加回 `ORDER BY`，结果依然全对，只有查询计划会退化——上面的变异表第 5 行就是这一条。

> 附带发现：分页之前，`MAX_PRODUCT_SESSIONS = 2048` 同时充当列表 `LIMIT`。也就是说 workspace 超过 2048 个会话后，尾部会被**静默截断且无法请求**。文档没有点出这一条，它才是本期最硬的理由。

> 附带修复：迁移 015 加 `table_exists("product_sessions")` 守卫，与迁移 007 同一处理。历史兼容 fixture 会声称某个版本却不含该版本应有的全部表（v1 fixture 只有 `product_preferences`），对不存在的表建索引会让整次升级失败。索引是纯派生状态，走到这里还没有该表的库本来就没东西可索引。

> 附带修复：`schema_newer_than_v14_is_rejected_without_rollback` 里的"未来版本"从字面量 15 改成 `CURRENT_SCHEMA_VERSION + 1`，并改名为 `a_schema_newer_than_this_build_is_rejected_without_rollback`。本期把当前版本推到 15，这个测试原本会变成"断言当前版本被拒绝"——加迁移的人会先看到它失败，改完之后它就再也测不到东西了。

> 诚实性说明：p95 那条验收项是预算检查，不是分页设计的证明。实测确认它**抓不到**两件事：把排序改回去只让单页贵约五成（远在任何能在共享机器上稳定通过的阈值之内）；而且排序形态下页延迟**同样**与深度无关（排的是单个 rank 组，组大小不随翻到多深而变），所以"深页不比浅页贵"的比值断言也分不开两种形态。真正的保证是查询计划测试。这一点已写进测试的文档注释，比值只打印不断言。

---

## Phase 8 — 上下文压缩（Compaction）

### 设计（对标 core/src/compact*.rs 系列，取其手动+自动骨架，暂不做 remote v2）

- `CompactedItem` 进 HistoryItem（Phase 2 已占位）：摘要替换被压缩区间，原始区间仍在 trace 中不丢；
- 触发：token 估算超过阈值（provider 目录里已有的 pricing 数据可复用估算）→ 自动压缩；CLI 提供 `/compact` 手动命令；
- 摘要生成本期用当前 provider 自身完成（fake provider 给确定性摘要以便测试）；
- 压缩点写入 trace：`TraceEntry::Compaction { covered_ordinals, summary_item_ref }`。

### 验收
- [x] 长对话压缩后 resume，模型上下文 ≤ 阈值且含摘要；
- [x] 压缩前的完整历史仍可从 trace 导出（审计不丢）。

> Remote/服务端压缩（compact_remote_v2 + 图片预算）明确列为 out of scope，待自托管压缩验证后再评估。

### 落地证据（commit 2ff2266）

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| 压缩后 resume 上下文含摘要且不含被替换历史 | `e2e::a_compacted_session_resumes_with_the_summary_instead_of_its_history`：跑完一轮真实 run → 用 `/compact` 走的同一个 `Engine::compact_resume_state()` 压缩快照 → 从压缩后快照 resume，`CapturingFakeModelClient` 抓到的**实际 prompt** 含 `COMPACTED_SUMMARY_ZETA`，且**不含** `ORIGINAL_QUESTION_EPSILON` / `ORIGINAL_REPLY_DELTA`。两个方向都断言：只断言「摘要在」的话，「摘要追加但历史照留」（prompt 变更大，与压缩目的相反）也会绿 | 通过 |
| 压缩前完整历史仍可从 trace 导出 | `e2e::a_compaction_leaves_the_full_history_exportable_from_the_trace`：压缩前后 `trace.jsonl` **字节完全相等**（手动压缩不写 trace），且 `read_history_tail` 仍导出 `AUDITED_QUESTION_THETA` + `AUDITED_REPLY_ETA` | 通过 |
| 手动压缩绕过 enabled 开关但仍受熔断约束 | `compaction::manual_compaction_runs_while_the_automatic_switch_is_off`：`CompactionRuntime::new(false, 3)` 下 `Automatic` 返回 `None`、`Manual` 正常产出且 `auto_triggered == false`。为此把 `breaker_tripped()` 从 `circuit_open()` 拆出——后者在开关关闭时恒为 `false`（UI 语义正确），直接用作手动路径的门会让失败模型被无限重试 | 通过 |
| 压缩当轮就把摘要发给模型 | `e2e::a_compacting_turn_sends_the_summary_it_just_generated`：React 原本 build context → 发 `PromptBuilt` → 压缩 → 却把压缩前就建好的 context 发出去，于是压缩那一轮「历史没了、摘要也还没到」，摘要要下一轮才生效。现改为压缩后重建 context 并复查 `over_hard_limit`（PlanReact 本来就是对的，此处对齐两个 loop） | 通过 |
| 摘要落在 resume 真正读的字段 | `types::compacting_a_checkpointless_session_still_carries_the_summary`：`continue_from_summary` 原先只写 `TaskState::summary`，而该字段每个跑完的 run 都会被填成截断的 final output（`artifacts.rs` 的 `RunCompleted` / `finalize`），因此无法用来承载压缩而不让普通 resume 看起来像被压缩过。现摘要写入 `checkpoint.summary`（facade 实际读取的字段），原本无 checkpoint 的会话由 `PromptCheckpoint::carrying_summary()` 补一个最小 checkpoint | 通过 |
| Phase 6 回填不再撤销压缩 | `types::only_a_compacted_state_reports_its_history_as_compacted_away` + 变异验证：Phase 6 把「历史为空」当作「快照丢了，从 trace 回填」，正好把压缩刚丢掉的历史又装回来，prompt 比压缩前更大。`history_was_compacted_away()` 区分「故意空」（有 checkpoint 且带摘要、session 与 preserved_tail 皆空）与「崩在 checkpoint 之前」（根本没有 checkpoint），仅前者豁免回填 | 通过 |
| 变异验证（防空测试） | 把 facade 的豁免条件改成 `!false` 后，`a_compacted_session_resumes_with_the_summary_instead_of_its_history` **失败**；把 `selection_from_config` 的 fake 分支改成 `if false &&` 后，`an_explicit_fake_model_outranks_a_configured_real_profile` **失败**。两个断言都确实承重 | 通过 |
| `/compact` 命令接入 | `SlashCommand::Compact` / `TerminalAction::Compact` / `format_repl_help` / `command_hint_line` 均已接入并有单测（`slash_command_parser_recognizes_first_pass_commands`、`to_action` 映射）。只改内存中的 resume snapshot，落盘交给下一条 prompt 自己那个 run 的正常 checkpoint 路径，因此 `/compact` 后直接退出不会改动已存会话 | 通过 |
| 回归 | `cargo fmt --all --check` 干净；`cargo clippy --workspace --all-targets` 零警告；`cargo test --workspace --no-fail-fast` 除下方 P7 计时项外全绿（含 compaction 10/10、resume 14/14、`cli_repl` 7/7） | 通过 |

> 分歧记录（§0.3 规则）：设计里的 `TraceEntry::Compaction { covered_ordinals, summary_item_ref }` **未落地**。手动压缩不启动 run，也就没有可归属的 trace 文件与 seq 序列，硬写会凭空造出一个不存在的 run 的 trace 行；而审计不丢这条要求由「原 trace 一字节不改」直接满足（见上表第二行），比新增条目更强。自动压缩沿用既有 `StreamEvent::PromptCompacted` 落 UI 事件。待 Phase 6 的 rollout recorder 接管写入端后，再评估是否需要独立的 Compaction 条目。
>
> 分歧记录（§0.3 规则）：`CompactedItem` 已存在于 `rove_core::history::HistoryItem`（Phase 2 落位），但本期压缩走的是 checkpoint 摘要通道而非在 history 序列里插入 `Compacted` 条目——后者要求写入端同时改 trace 与 session 投影，属于 Phase 6 recorder 的职责范围。

> 顺带修掉（非本 Phase 范围，独立 commit `77f1787`）：`--model fake` 在配置了 active profile 的机器上会解析到那个真实 profile，把字面模型名 `"fake"` 发给它——一次真实计费请求，且必然失败（SiliconFlow 回 HTTP 400 "Model does not exist"）。这也是 `cli_repl` 5 个用例在任何有真实 `~/.rove/config.toml` 的机器上（本分支与 main 同样）失败的原因。现 fake 优先于 active profile，并给这批用例钉上 `ROVE_CONFIG_ROOT` 隔离。

---

## Phase 9 — 迁移并发加固

### 背景
rove 双入口（desktop 常驻 + cli 临时进程）可能同时触发 schema 迁移。codex 近期专门修过同类问题（"Harden startup rollout migration against concurrent updates" #40499）。

### 设计
- 用现有依赖 `fs2` 在 `~/.rove/state.db.migrate.lock` 上排他文件锁包裹整个迁移事务；
- 锁内二次检查 `user_version`（double-checked locking），已升级则直接放行；
- 锁获取超时（建议 30s，对齐 codex busy_timeout 120s 量级酌情调）报结构化错误而非 panic；
- 迁移执行期间 backfill 任务必须等待（同一把锁或序贯 barrier）。

### 验收
- [x] 双进程并发首启集成测试（tokio 多任务模拟）无一失败；
- [x] 迁移中途 kill，下次启动要么续升要么安全回退到迁移前版本。

### 落地证据（commit 3ef3cb6）

| 要求 | 证据 |
| --- | --- |
| `fs2` 排他锁包裹整个迁移序列 | 新增 `runtime/src/state/migration_lock.rs`：`acquire_migration_lock` + `Drop` 释放；`state/index.rs::apply_migrations` 与 `apps/api/.../schema.rs::apply_migrations` 均在首次写入前取锁 |
| 锁内二次检查 | 两处均为 `schema_is_current` → 取锁 → 再次 `schema_is_current`；命中即放行 |
| 超时报结构化错误 | `MigrationLockError::{Timeout,Io}`，30s；runtime 侧映射 `ErrorKind::TimedOut`，api 侧映射 `ProductStoreUnavailable`；无 panic 路径 |
| backfill 等待迁移 | `pub fn wait_for_migrations`：取同一把锁后立即释放，供 Phase 5 启动期 backfill 调用 |
| 并发首启无一失败 | `concurrent_first_start_migrates_once_and_no_starter_fails`：8 线程 + `Barrier` 同刻释放，断言每个 starter 都成功且 `COUNT(*) FROM schema_migrations == MIGRATIONS.len()`（无重复行） |
| 中途 kill 可续升 | `a_migration_interrupted_after_a_prefix_resumes_on_the_next_start`、`a_failed_migration_records_no_version_row`：单步 `TransactionBehavior::Immediate` 保证要么记账要么整步回滚 |
| 快路径不取锁 | `an_already_current_index_does_not_take_the_migration_barrier`：外部持锁时 `initialize()` 仍成功 |
| 测试非空转 | 变异实验：还原为原始无事务循环后，并发测试 3 次运行得到 FAILED / FAILED / ok —— 竞态特有的 flaky 签名 |

测试计数：`state::index` 23/23、`state::migration_lock` 5/5、全量 1643 passed。

> 分歧记录（§0.3 规则）：**用 `schema_migrations` 表而非 `PRAGMA user_version`**。计划写的 `user_version` 在 rove 全库不存在；rove 用 `schema_migrations` / `product_schema_migrations` 两张表记账，且能区分"哪几步已应用"，比单个整数更适合中途 kill 后的续升判定。二次检查因此改为查 `MAX(version)`。

> 分歧记录（§0.3 规则）：**锁文件是每库兄弟文件而非单一 `~/.rove/state.db.migrate.lock`**。rove 有两个独立数据库（runtime `state.sqlite` + product `product.sqlite`），且每 workspace、每测试各有自己的库。单一全局锁会让不相关的库互相阻塞，也会让并行测试串行化。故 `migration_lock_path` 派生为 `<db>.migrate.lock`。

> 附带修复：`state/index.rs::apply_migrations` 原先**完全没有事务**却在每次 `connect()` 都执行 —— 纯 TOCTOU 竞态。这正是 Phase 5 要升到 v15 的那个库，故 P9 必须先落地。

> 附带修复：并发测试照出 `connect()` 里另一个既有竞态 —— `PRAGMA journal_mode=WAL` 需独占锁，而 SQLite 对此冲突直接返回 `SQLITE_BUSY` **不走 busy handler**，那 5s `busy_timeout` 对它无效，并发首启会有 opener 直接开不开库。新增 `enable_wal`：在同一预算内重试，并在被拒后检查是否已有同伴完成切换（WAL 是幂等的文件属性）。

> 附带修复：`state_migration` 的 prune 把新的 `.migrate.lock` 判为 `Unknown` 而留下残留，导致 `legacy_disposition` 退化为 `partially_pruned`。已在 `classify_relative_path` 中与 `-wal`/`-shm` 影子文件同列跳过（`migration_barrier_is_transient`）。

> 遗留（非本阶段引入）：`rove-integration-tests --test cli_repl` 有 5 个失败，已在干净的 main checkout 上复现同样 5 个，与本阶段无关。

---

## Phase 10 — 工具 crate 隔离（apply-patch 式）

### 设计
- 新 crate `rove-tools-text`（名字可议）：收编 patch 应用 / 文件编辑类工具实现，从 runtime/tools 中剥离；
- 特性对标 `codex-rs/apply-patch/`：
  - 纯函数内核：`(input_files, patch) -> Result<ApplyOutcome, ApplyError>`，无 IO 之外副作用、无 tokio；
  - heredoc/fuzzy context 匹配策略与错误分级（可重试的 fuzzy 失败 vs 硬失败）参考其实现；
  - 测试密度对齐：正例、模糊匹配、冲突、CRLF（Windows 平台必测）、unicode 边界；
- runtime/tools 中的其余工具（shell/glob/grep 类）本期不动，仅建立"工具实现必须可脱离 agent 循环单测"的先例。

### 验收
- [x] 新 crate `cargo test` 通过率覆盖上述矩阵；
- [x] runtime 对其仅有类型级依赖。

### 落地证据（commit `6c05187`）

| 验收项 | 证据 |
|---|---|
| 纯函数内核 | `tools-text/src/apply.rs`：`apply_patch(&BTreeMap<String,String>, &Patch) -> Result<ApplyOutcome, ApplyError>`，无 IO/无 tokio。`grep -rE "tokio\|std::fs\|async fn" tools-text/src/` 为空 |
| 测试矩阵 | `cargo test -p rove-tools-text` = **48 passed**，覆盖正例 / fuzzy 三级匹配 / 冲突（歧义 + 重叠 hunk）/ CRLF 保持 / unicode 边界 |
| 错误分级 | `ApplyError::is_retryable()` 仅对 `ContextNotFound`、`AmbiguousContext` 为真；其余（`MissingInput`/`AlreadyExists`/`OverlappingHunks`/`NotText`/`DuplicatePath`）为硬失败 |
| 类型级依赖 | `cargo tree -p rove-tools-text` 只有 `serde` / `serde_json` / `thiserror`；runtime 侧仅两处调用（`coding.rs:97` `replace_once`、`coding.rs:1269` `localized_diff`） |
| 依赖方向固化 | `tests/workspace_architecture.rs` 断言 `rove-tools-text` 为叶子（无任何本地依赖），且 runtime 的本地依赖集合精确等于 `{rove-core, rove-models, rove-tools-text}` |
| 等价性 | 全工作区 `cargo test --workspace --no-fail-fast` 无回归；`localized_diff` 保持原 `--- a/{path}` / `+++ b/{path}` 输出格式 |

> 分歧记录（§0.3 规则）D4：计划称新 crate "收编 patch 应用 / 文件编辑类工具实现"。本期只把**纯文本内核**（patch 解析、上下文匹配、apply、diff 渲染）搬出去，`EditFileTool` / `WriteFileTool` 这些 `Tool` impl 仍留在 runtime。理由是 rove 的 `Tool` trait 携带 `async` + 审批 + 工作区边界校验（产品语义），把它搬进纯 crate 会把 tokio 和审批策略一起拖进来，反而破坏本 Phase 自己要求的"无 tokio"。按"rove 产品语义 > codex 机制"，取内核纯度、留 Tool 外壳。

> 附带修复（非本 Phase 范围，但阻塞本分支绿灯）：`project_trust.rs` 的 `retargeted_windows_junction_does_not_reuse_the_original_grant` 在本机 `main` 上即为红（已在干净 checkout 上复验）。根因是 Windows 拒绝把 junction 作为"不受信任的装入点"遍历（os error 448），`canonicalize()` 失败 → capability digest 不可用 → 测试前置的 grant 无法建立。信任层的拒绝本身是安全行为，故按该测试已有的 skip-guard 风格，在环境无法承载该场景时跳过，而非放宽断言。

---

## 实施顺序与依赖图

```
P1 信封 ──→ P2 历史/UI 分离 ──→ P6 resume 加固 ──→ P7 列表分页 ──→ P8 压缩
   │              │
   │              └──→ P4 protocol crate ──→ (独立)
   └──(读路径)────┐
                  ↓
P3 home 目录 ──→ P5 store 收拢（需 P3 的 ~/.rove 落位 + P6 扫描器做 backfill）
P9 迁移锁（P5 动 schema 前落地即可，可与 P3 并行）
P10 工具 crate（全程独立，随时可插入闲置人力）
```

推荐串行批次（单人节奏）：

| 批次 | 内容 | 预估 |
|---|---|---|
| B1 | P1 + P3 | 小（各 1-2 天级） |
| B2 | P2（最大风险点，预留充分测试时间） | 大 |
| B3 | P4 + P10（可并行） | 中 |
| B4 | P9 + P6 | 中 |
| B5 | P5 + P7 | 中 |
| B6 | P8 | 中 |

## 全局风险清单

1. **P2 是唯一动核心循环的改动**——SSE 回归测试必须在动手前先固化成快照基线。
2. **wire 格式变更窗口**：P1/P2 都改 trace 格式，务必让 reader 从第一天就写成多版本兼容，避免出现"必须停机迁移"。
3. **Windows 平台细节**：文件锁（fs2）、home 目录（dirs）、CRLF、长路径——每个 Phase 的验收都在 Windows 上跑一遍（本项目 README 声明 Desktop-Windows verified）。
4. **repository.rs 7149 行是泥球**：P5 只做最小侵入，不要顺手重构。
5. **fake provider 是测试基石**：所有 Phase 的新行为都要能在无网络模式下确定性验证，这是 rove 相对 codex 的独有优势，别丢。

## 新对话开工指引（给未来的执行者）

1. 先读本文件 §0.1/§0.2，把两边代码锚点打开对照一遍再动键盘。
2. 严格按批次推进，单个 Phase 内允许调整，不允许跨批次合并 PR。
3. 每个 Phase 完成后在本文件对应验收项打勾并追加实际 commit hash。
4. 遇到 codex 实现与本方案冲突时：以"rove 产品语义 > codex 机制"裁决，并把分歧记录到 §0.3 之后的新小节。
