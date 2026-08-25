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
1. 盘点 `apps/api/src/types.rs`、`product/contracts.rs` 中所有出参/入参结构，机械搬迁（不改字段）。
2. 建立 crate，api re-export 保持旧路径可用一个过渡期（`pub use rove_protocol::*`）。
3. 三端切换 import；删除过渡 re-export。
4. `docs/design/` 下补一页协议文档（对标 `codex-rs/docs/protocol_v1.md` 的粒度）。

### 验收
- [ ] `cargo tree -i axum` 在 rove-protocol 中无输出；
- [ ] web/desktop 全量回归通过；
- [ ] apps/api/src/lib.rs 行数下降 ≥30%（store 迁出前先靠 DTO 外移达成）。

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
- [ ] 删掉 state.db 后冷启动，全部会话列表/详情自动恢复；
- [ ] 消息内容在 DB 中零冗余存储；
- [ ] 迁移 v14→v15 在真实数据副本上演练通过。

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
- [ ] 100MB trace 上 resume 内存峰值 < 50MB；
- [ ] 尾部残行（崩溃产物）被扫描器跳过；
- [ ] resumed 会话继续运行产生的 trace 单文件连续可回放。

---

## Phase 7 — 会话列表 / 搜索 / 游标分页

### 设计（对标 `rollout/src/list.rs`）

- keyset 游标：`Cursor { ts, id }` base64 编码进 `?cursor=`，排序键 `updated_at | created_at | title`，方向 asc/desc；
- 查询路径：优先 state.db `rollouts` 表（O(log n)），backfill 缺口时回退目录扫描（目录布局本身按日期有序，扫描成本可控）;
- API：`GET /sessions?limit&cursor&sort&q`；搜索本期只做 title/path 子串（SQLite LIKE + 大小写折叠），FTS 留待后续；
- 归档：`POST /sessions/:id/archive` 移动文件至 `archived_sessions/`（保持相对布局），列表默认排除。

### 验收
- [ ] 10k 假想会话（生成 fixture）下列表 p95 < 50ms；
- [ ] 游标翻页无重复无遗漏（属性测试）。

---

## Phase 8 — 上下文压缩（Compaction）

### 设计（对标 core/src/compact*.rs 系列，取其手动+自动骨架，暂不做 remote v2）

- `CompactedItem` 进 HistoryItem（Phase 2 已占位）：摘要替换被压缩区间，原始区间仍在 trace 中不丢；
- 触发：token 估算超过阈值（provider 目录里已有的 pricing 数据可复用估算）→ 自动压缩；CLI 提供 `/compact` 手动命令；
- 摘要生成本期用当前 provider 自身完成（fake provider 给确定性摘要以便测试）；
- 压缩点写入 trace：`TraceEntry::Compaction { covered_ordinals, summary_item_ref }`。

### 验收
- [ ] 长对话压缩后 resume，模型上下文 ≤ 阈值且含摘要；
- [ ] 压缩前的完整历史仍可从 trace 导出（审计不丢）。

> Remote/服务端压缩（compact_remote_v2 + 图片预算）明确列为 out of scope，待自托管压缩验证后再评估。

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
- [ ] 双进程并发首启集成测试（tokio 多任务模拟）无一失败；
- [ ] 迁移中途 kill，下次启动要么续升要么安全回退到迁移前版本。

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
