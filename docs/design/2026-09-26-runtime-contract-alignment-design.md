# 运行时与产品合同对齐（下一轮）设计

- 日期：2026-09-26
- 状态：**Partially Implemented**。R1、R3、R4 已落地（§0.1 状态列、§1.7、§3.4、§4.4）；
  R2c 在同批 PR 中待独立评审（§2.4）；其余条目仍未实现。
  每个条目落地时必须在同一变更中更新 `docs/runtime/` 对应现状文档与
  `docs/runtime/implementation-status.md`、`docs/runtime/acceptance-matrix.md`，
  并按 `CONTRIBUTING.md` 走 feature worktree + PR。
- 基线：`main @ 915cbb9`（2026-09-26）。
- 范围：`runtime/`、`core/`、`models/`、`apps/api/`、`apps/bootstrap/`、`tests/`、
  ProductStore schema 迁移，以及为消费新合同所需的 `apps/web` 最小接线
  （接线的前端细节在[前端文档](2026-09-26-frontend-experience-alignment-design.md)，
  下称"前端文档"）。
- 参考副本（只读，仅参考机制与合同设计，不复制源码）：
  - `D:\Study\project\agent\third-party-agents\PI-Desktop`（LGPL-3.0；**禁止**复制代码）
  - `D:\Study\project\agent\third-party-agents\open-vetta`（Apache-2.0）
  - 机制对照另见 `docs/plans/2026-08-25-codex-alignment-implementation-plan.md`
    （其"FTS 留待后续"由本文档 R7 承接）。
- 前置阅读：`AGENTS.md`（尤其第 4 节架构不变量与第 11 节安全清单）、
  `docs/runtime/react-loop.md`、`docs/runtime/subsystems.md`、
  `docs/runtime/implementation-guide.md`。

---

## 0. 架构不变量对照（本档所有条目的硬约束）

| 不变量（AGENTS §4） | 触碰它的条目 | 保持方式 |
|---|---|---|
| CLI/API/Web 复用共享 runtime，不养平行 agent loop | R2 全部 | 恢复/重试/保留全部做在 `runtime`/`core`/`models`，API 只做投影与转发 |
| Provider 载荷留在 model/provider 边界内 | R2c | 重试预算在 run loop 层消费 `ModelError`，`ProviderRetry` 事件只携带安全摘要（错误种类/attempt/delay），禁止携带 headers/响应体 |
| 规范事件是唯一生命周期合同 | R2b、R2c、R5 | 新事件进 `StreamEvent` 枚举，同步 producer/trace 持久化/API SSE/OpenAPI/Web/合同测试；禁止私有并行通道 |
| `trace.jsonl` 记事实、`task_state.json` 记可恢复状态、`report.json` 是派生摘要 | R2b、R9 | salvage 的部分文本进 task_state 历史 + trace 事实；修剪类操作必须留 trace 事实 |
| 已完成 mutation 与已完成 plan 工作不得在 resume 时重放 | R2、R4 | salvage 只持久化文本不执行工具；恢复回合走正常审批；队列恢复沿用现有 CAS claim |
| 未知 in-flight 副作用保守处理 | R2b | 取消时工具路径行为不变；1.5s 等待只影响文本收尾，不等待工具 |
| 本地确定性执行无 key/无网络可用 | 全部 | fake provider 同步扩展脚本化行为，所有 R2 用例可在无网环境跑 |
| 秘密不出现在配置/日志/trace/report/API 响应 | R2c、R5、R7 | 事件 reason 字段只允许白名单枚举+安全摘要；搜索片段复用 redaction 规则 |
| Workspace 路径以解析后的 workspace 为界 | R8 | 附件存储在 product 会话目录，复用 `join_safe`/`is_secret_filename` 基线 |

## 0.1 条目总表

| 编号 | 条目 | 优先级 | 主要触碰面 | 状态 |
|---|---|---|---|---|
| R1 | transcript 游标分页（F.4 闭环） | P0 | `apps/api` + `apps/web` 接线 | Implemented（见 §1.7） |
| R2 | 回合失败恢复家族（a 静默回合 / b 中止保留 / c 重试预算+事件） | P0 | `core`/`runtime`/`models`/`apps/api`/事件合同 | Proposed |
| R3 | 会话 `last_outcome` 字段 | P1 | ProductStore 迁移 016 + contracts + web | Implemented（见 §3.4） |
| R4 | 队列协议扩展（原子重排 / send-now 边界语义 / 重启存活验证） | P1 | `apps/api` + 迁移 017 | Implemented（见 §4.4；Web 接线交接前端文档） |
| R5 | 产品级目录 SSE `/product/events` | P1 | `apps/api` + 迁移 018 + web | Proposed |
| R6 | 消息级编辑重发 = fork-at-message | P2 | `apps/api` fork 合同扩展 | Proposed |
| R7 | 会话内容搜索（FTS5，先单会话） | P2 | ProductStore + 端点 | Proposed |
| R8 | 附件/图片上传协议（骨架） | P2 | 存储/端点/消息合同/provider 层 | Proposed（骨架先行；实施计划与威胁模型已出，**尚未动工**，见 §8.5） |
| R9 | 压缩两层审计 + `/compact` HTTP 端点 | P2 | `runtime` + `apps/api` | Proposed（先审计后实施） |
| R10 | 登记不在本轮的方向 | — | — | 不做 |

优先级：P0 = 下一轮第一批；P1 = 第二批；P2 = 独立立项、允许排后。

---

## 1. R1 transcript 游标分页（F.4 闭环）

### 1.1 现状（已核实）

- `GET /product/sessions/{session_id}/transcript` 的 handler **不接受任何 query 参数**
  （`apps/api/src/product/routes.rs:360-382`），reader 每次都从 seq 0 投影每个 run
  （`apps/api/src/product/transcript/reader.rs:107-111`）。
- 有界常量：`MAX_TRANSCRIPT_RUNS = 256`、`MAX_EVENTS_PER_RUN = 2_000`、
  `MAX_TOTAL_EVENTS = 10_000`、单事件 1MiB、总量 16MiB、report 256KiB/2MiB
  （`reader.rs:29-36`）。
- 响应 `ProductTranscriptResponse`：`status: complete|partial` + `partial_reasons`
  （含 `response_limit_reached`）+ `segments[]`；每段带 `binding.ordinal`、
  `observed_through_seq`、`last_event_seq`、`events: Vec<JobStreamEvent>`
  （`apps/api/src/product/contracts.rs:1444-1516`）。
- 消息账本的分页**已经存在**：`ListProductMessagesQuery { after_seq, before_seq, limit }`
  （默认 64 / 上限 128，`routes.rs:49-58`、`repository.rs:2693-2777`），响应带
  `next_after_seq/next_before_seq`，每条消息有 `seq`（`contracts.rs:307-348`）——
  但 Web 从未传参（`apps/web/state/use-session-continuity.ts` 调 `listMessages`
  无页参）。
- Web 端客户端半边已就绪休眠：`chat/transcript-window.ts` 的
  `shouldRequestOlderPage` 带单测；`Transcript.tsx:112-114` 注释明确等待游标。

### 1.2 参考机制

- PI-Desktop：两步持久化（fsync JSONL 先行）+ 渲染端"有界持久页 ⊕ 活快照"拼接，
  重载永不回滚流式尾部。
- open-vetta：追加式 JSONL + 虚拟化按需窗口。

rove 的 trace/events 已是 append-only 且有 `seq`，缺的只是把"最新一页"变成"可向旧翻页"。

### 1.3 目标合同

`GET /product/sessions/{session_id}/transcript` 新增**可选** query 参数：

- `before_ordinal: Option<u64>`：只返回 `binding.ordinal < before_ordinal` 的 run 段，
  按新→旧取；不传 = 现状（最新页），行为**逐字节兼容**。
- `limit_runs: Option<usize>`：本页最多包含的 run 数；默认与现状的全局
  `MAX_TRANSCRIPT_RUNS` 行为一致，上限 64；0 → 400。

响应新增 **additive** 字段：

- `next_before_ordinal: Option<u64>`：还有更旧的 run 时 = 本页最旧 run 的
  `ordinal`；没有更旧的 = `null`。
- `has_more: bool`：`next_before_ordinal.is_some()` 的显式形式，便于客户端。

语义细则：

- run 是翻页原子：单个 run 不跨页截断；单 run 超 `MAX_EVENTS_PER_RUN` 仍按现有
  partial 语义标注该段（`response_limit_reached` 等），不影响翻页。
- 每页仍受 `MAX_TOTAL_EVENTS` 与字节预算约束：页内超预算时截断并在该页
  `partial_reasons` 标注，同时 `has_more` 仍可继续翻页。
- `before_ordinal = 0` → 400；`before_ordinal` 大于最大 ordinal → 空页 +
  `has_more: false`（合法，便于客户端游标推进）。
- 授权、redaction、绑定校验全部复用现有 reader 路径，无新权限面。

### 1.4 兼容与迁移

- 纯 additive：无 schema 迁移（数据源是既有 events/index 表）；旧客户端不传参行为不变。
- OpenAPI 同步；`tests/api.rs` 新增合同用例（见 1.6）。
- `docs/runtime/implementation-guide.md`（路由表与游标语义）同变更更新；
  原文写作时指向的 `docs/runtime/browser-workspace-spec.md` 没有 transcript 投影章节，
  属文档更正，见 §1.7 第 5 条。

### 1.5 Web 接线（最小集，细节归前端文档）

- `transcript-window.ts` 的 `shouldRequestOlderPage` 启用：`load older` 点击 →
  `before_ordinal = 当前最小 ordinal` → 响应 segments **prepend** 进 timeline
  （prepend 锚定复用 `chat/transcript-scroll` 既有机制与 e2e 场景）。
- `has_more=false` 时按钮消失；翻页期间的加载态如实显示。
- 消息账本分页（`before_seq`）不在本条接线——transcript run 粒度已覆盖"看历史"需求，
  消息账本分页留给 R7 搜索与未来需要时。

### 1.6 验证门

- `cargo test -p rove-integration-tests --test api` 新用例：
  1. 无参响应与现状完全一致（对既有用例做快照对照）；
  2. 构造 >64 runs 的会话：默认页 → 翻页序列 → 拼接 = 全量、无重复无遗漏、
     `ordinal` 单调；
  3. `before_ordinal=0` → 400；越界 → 空页 + `has_more:false`；
  4. 页内超 `MAX_TOTAL_EVENTS`：partial 标注 + 可继续翻页。
- Web e2e（mock 300 runs）：翻页拼接、prepend 滚动锚定、`has_more` 终止。
- 声明闭环：`implementation-status.md` 的 F.4 行在两条接线（服务端游标 + Web 消费）
  都落地后才允许改写；只落服务端不得宣称 F.4 完成。

### 1.7 实施记录

服务端与 Web 两条接线同批落地（branch `feature/runtime-align-r1`）。

- 服务端：`apps/api/src/product/contracts.rs`（`ProductTranscriptQuery`、
  `MAX_TRANSCRIPT_PAGE_RUNS = 64`、两个 additive 字段）、
  `apps/api/src/product/transcript/reader.rs`（`PageWalk` 纯状态机 + 游标页投影）、
  `apps/api/src/product/routes.rs`（query DTO + 400 校验）、
  `apps/api/src/product/export.rs`（沿用无参导出）。
- Web：`product/product-api-types.ts`、`product/product-client.ts`、
  `state/transcript-projection.ts`、`state/use-session-continuity.ts`、
  `chat/transcript-window.ts`、`chat/Transcript.tsx`、`shell/ProductApp.tsx`。

落地时确认并解决的设计歧义（以本文档为准的记录，不静默改写历史）：

1. **无参兼容 vs "最新页"**：§1.3 写"不传 = 现状（最新页）"，而现状实际是
   自旧到新、预算内最多 `MAX_TRANSCRIPT_RUNS` 个 run。两种读法冲突时以 §1.3 同段的
   "逐字节兼容"为准：**无参请求完全走既有路径**（旧到新、`has_more`/
   `next_before_ordinal` 两个字段整体省略），带任一页参数才返回游标页
   （新→旧取、升序发出、两字段都出现且 `has_more === next_before_ordinal.is_some()`）。
   因此 §1.3 写的 `has_more: bool` 在实现中是 additive `Option<bool>`：
   字段恒在会破坏无参响应的字节兼容。
2. **游标边界**：`before_ordinal` 大于最大 ordinal → 空页 + `has_more:false`（§1.3 已写）；
   实现同样把"等于最旧 run"的游标收成空终页，否则客户端会在最后一页之后无限重取。
3. **`limit_runs` 上界**：0 与 >64 都返回 `product_invalid_input` 400（§1.3 只写了 0）；
   只给 `before_ordinal` 时默认页大小沿用 `MAX_TRANSCRIPT_RUNS`。
4. **验证门第 4 条（页内超预算）未做端到端重放**：product 消息内容上限 32 KiB
   （`runtime/src/conversation.rs` 的 `MAX_MESSAGE_BYTES`，超限回合会转
   `needs_attention`），fake provider 也不能从 API 配置脚本化，因此让单页超过 16 MiB
   事件预算需要约 170 个满额回合（约 3 分钟）。改为 `PageWalk` 单元测试驱动同一状态机
   的"预算已耗尽"与"单 run 被预算截断"两条分支（`page_walk_continues_older_after_the_budget_stops_a_page`、
   `page_walk_keeps_paging_after_a_run_is_truncated_by_the_budget`），端到端只覆盖
   翻页拼接与边界矩阵。**这条门禁是降级覆盖，不是通过。**
5. **文档更正**：§1.4 指的 `docs/runtime/browser-workspace-spec.md` 是未来的
   browser 自动化 workspace 说明，没有 transcript 投影章节。当前合同已写入
   `docs/runtime/implementation-guide.md`（路由表 + 游标语义 + Web 消费），
   `docs/runtime/implementation-status.md` 与 `docs/runtime/acceptance-matrix.md`
   同步更新。

非目标（本轮明确不做）：分页仍受既有校验窗口约束（`min(bindings, MAX_TRANSCRIPT_RUNS + 1)`
= 257 个 binding），超过该窗口的会话无法继续向更旧翻页；消息账本 `before_seq` 分页仍不接线（§1.5）。

---

## 2. R2 回合失败恢复家族

### 2.0 共同现状（已核实）

- run loop 由 `runtime/src/engine/facade.rs`（`EngineFacade::run_with_cancel`）驱动
  `runtime/src/engine/run_loop.rs` 的 `run_unplanned_loop`，内核为
  `core/src/kernel.rs` 的 `run_agent_kernel`。
- 模型错误直接终止：`KernelTermination::ModelFailed` → `TerminationReason::Error`，
  output = `"Model error: {error}"`（`runtime/src/engine/run_loop.rs:451-455`）。
  **run loop 没有任何重试/恢复预算。**
- 现有重试只在 `models/src/routing.rs:13-133` 的 `RoutingModelClient`：
  `RetryPolicy { max_attempts 默认 1, backoff_base 250ms, backoff_max 5s }`，
  仅在"commit 前"（60s 内无首个内容事件）重试；**未配置 fallback 时 factory 直接返回
  裸主 client，没有任何重试包装**（`apps/bootstrap/src/factory.rs:82-85`）。
- 取消路径：cancel token 在 model turn 流中以 `biased` 选中 → `ModelTurnItem::Cancelled`
  → **已累积的助手文本被丢弃**：不发 `LlmMessage`、不进 task_state 历史
  （`core/src/kernel.rs:277-310, 345-356`；`run_loop.rs:446-450`）；已流出的
  `LlmChunk` 因 `yield_traced!` 落在 trace/events（`facade.rs:754-760`），但那只是事件事实，
  不是可恢复消息。run 终态 `cancelled`，产品会话回到 `Idle`
  （`apps/api/src/lib.rs:3542-3575`）。
- fake provider 已支持脚本化回合回放（`models/src/fake.rs:50` 附近），需扩展
  脚本化**错误/空回复/延迟**。
- 事件合同：`StreamEvent`（`runtime/src/foundation/events.rs:23-313`）已有
  `LlmChunk { delta }`、`ModelStatus { status, message }`、
  `LlmMessage { full, usage, tool_calls, assistant_turn? }`——`assistant_turn` 的
  `#[serde(default)]` additive 先例（枚举注释明示）是本档 additive 字段的范本。
  **没有任何 retry 相关事件。**

### 2.1 R2a 静默回合恢复（silent-turn recovery）

参考：PI-Desktop 对"回合以空回复结束"的自动恢复——检测后带一句系统 nudge 重跑一次，
用户永远不需要手动"继续"（其实测命中率 15/255 会话）。

**检测定义（必须精确实现，防止误伤）**：一个 run 以 `TerminationReason::Done` 终止，
且满足全部条件：
1. 本 run **没有任何 `LlmMessage` 事件**，或最终 `LlmMessage.full.trim().is_empty()`；
2. 本 run 没有任何 `ToolCallStarted`（无工具活动——跑过工具的 run 不算"静默"）；
3. 本 run 的触发输入是真实用户输入（`RunStarted.user_message` 非空），而非恢复回合自身。

**恢复动作**：满足时，在同一 job/run 上下文自动追加**一次**恢复回合：向 kernel 注入
一条系统 nudge（固定安全文案，如"上一回合没有产生可见回复；请继续完成或总结当前进展"，
不含用户数据），随后模型回合照常执行（工具、审批、预算与普通回合一致）。
恢复回合再次静默 → 按现状终止，**不循环**（预算硬上限 1 次）。

**观测**：
- 发出 `ModelStatus { status: "recovering_silent_turn", message: <安全文案> }`
  （沿用 `ModelStatus` 自由串模式，并在 `docs/runtime/react-loop.md` 登记该 status 值）；
- trace 事实：优先复用 `ExecutionDegraded { record }`（若 `ExecutionDegradation`
  支持种类字段；否则在实现时新增一个 `StreamEvent` 变体并在同一变更里走完
  producer/持久化/API/OpenAPI/Web/合同测试全链）。二选一在实现评审时定，
  验收标准是"事件可见、进 trace、Web 等待行能显示"。

**配置**：`apps/bootstrap/src/config.rs` 增 `recovery.silent_turn_max_attempts: usize = 1`
（0 = 关闭）；属 runtime/recovery 配置组，文档进 `docs/runtime/`。

**非目标**：不重试工具失败本身（工具失败有独立错误路径）；不与用户取消竞争
（恢复回合等待期间用户取消 → 正常取消）；不在 API 层实现（必须做在 engine 层，
保持单一事件生命周期）。

**测试**：
- fake provider 脚本化"首次空 final、二次正常"：断言恰好一次 nudge、恢复产物进
  history（resume 后仍在）、trace 有事实、事件序列可见；
- 脚本化"两次都空"：终止且不循环；
- 配置 = 0：行为与现状一致；
- 恢复回合内触发审批：审批照常（安全路径不因恢复而绕过）。

### 2.2 R2b 中止保留（abort salvage）

参考：open-vetta 在 abort 时与 provider 结果竞争 ≤1.5s，拿不到就把最后已流出的
partial 落库——"已显示给用户的内容绝不丢"；PI 在消息上标注 "(aborted)"。

**目标行为**（取消路径，`core/src/kernel.rs` 的 `Cancelled` 分支 + facade 收尾处）：

1. 取消信号到达时，若该模型回合**已累积非空文本**：
   - 先进入"收尾窗口"：最多等待 **1500ms** 让 in-flight 请求自然收尾；
   - 窗口内收到完整 final → 按完整 `LlmMessage` 持久化（`aborted=false`；
     run 终态仍是 `Cancelled`，消息完整是另一回事）；
   - 窗口超时 → 将已累积文本作为 assistant 消息持久化并标注中止。
2. 持久化 = 写入 task_state 历史（resume 可见、不重放）+ 发出
   `LlmMessage { full, usage, tool_calls: [], assistant_turn, aborted: true }`——
   **additive 字段 `aborted: bool`，`#[serde(default)]`，旧 trace 重建为 false**
   （沿用 `assistant_turn` 先例；事件合同变更同步五件套）。
3. 累积文本为空 → 维持现状（不落任何消息，Web 智能停止路径不受影响）。

**边界**：
- 收尾窗口只等待文本流，**不等待工具**：取消时工具的既有行为（拒绝未决审批、
  中断执行）完全不变——"未知 in-flight 副作用保守处理"不变量不受影响。
- salvage 不执行任何工具、不发起新模型调用（与 R2a 的边界相反：R2a 是继续干活，
  R2b 是收尸记账）。
- 多轮取消竞争（连续 cancel）：幂等——已落过 partial 就不再落第二次。

**前端联动**（细节在前端文档 F6）：`Transcript` 对 `aborted=true` 的消息渲染
"(已中止)"后缀标记；smart-stop 在存在 aborted partial 时改为提示"已保留部分回复"，
不再回填用户消息文案。

**测试**：
- 流式中途取消 → history 含 aborted assistant 消息、resume 后可见、不重放；
- 1500ms 竞争两分支（fake provider 可控延迟：窗口内完成 / 窗口超时）；
- 空文本取消 → 无消息（现状对照）；
- 连续取消幂等；
- 合同测试：`aborted` 字段出现在 API SSE 帧与 trace；旧 trace（无字段）重建为 false。

### 2.3 R2c provider 重试预算与可见性

参考：PI-Desktop 的分账重试预算——429 与瞬时错误分开计数、`Retry-After` 优先、
指数退避 + jitter、状态行如实显示 "Retrying · attempt 9/10"。

**目标合同——新 canonical 事件**：

```rust
/// A model call is being retried under the recovery budget. reason is a safe
/// summary (error kind + optional retry-after), never provider payloads.
ProviderRetry {
    attempt: u32,
    max_attempts: u32,
    delay_ms: u64,
    reason: String,   // 白名单：rate_limited / transient:<kind>
    phase: String,    // "model_call"
},
```

新增变体属事件合同变更：同步 `events.rs` 的 event_name 映射、trace 持久化、
API SSE（既有通道自动携带）、OpenAPI、Web 消费、合同测试。

**机制**：重试从"仅 RoutingModelClient 的 commit 前"扩展为 **run loop 层的模型回合
预算**，在 `run_unplanned_loop` 的模型调用边界（`run_loop.rs:805-850` 的
`run_kernel_model_turn` 外围）实现：

- 捕获 `ModelError::is_retryable()`（`models/src/error.rs:37-44`：
  RequestFailed/StreamInterrupted/RateLimited 可重试）；
- **分账**：`RateLimited` 与其它瞬时错误各自独立预算（默认：
  `recovery.retry.rate_limit_max_attempts = 6`、`recovery.retry.transient_max_attempts = 4`，
  base 2s → 上限 30s 指数 + jitter；`RateLimited` 携带 `retry_after_ms` 时优先照用，
  上限 clamp 30s）；
- 每次重试前发出 `ProviderRetry` 事件并 sleep（sleep 必须受 cancel token 保护：
  退避期间取消 → 立即终止，不留悬挂等待）；
- 预算耗尽 → 按现状终止（`TerminationReason::Error`）。

**关键边界——流中断不自动重试**：只在"本回合尚无任何 `LlmChunk` 落账"时自动重试
（连接失败/429/超时都发生在这里）。流已经开始后中断（`StreamInterrupted`）**不**
自动重试——重试会从头生成，可能与已流出的部分文本重复。该场景交给 R2a（若最终为空）
与用户手动 retry。此决策的理由必须写进 `docs/runtime/react-loop.md`。

**为什么这解决了旧争议**：W 文档拒绝"重试倒计时"的理由是 `model_status` 没有时间
字段——正确解法是让运行时先成为事实源（预算、attempt、delay 都是运行时事实），
UI 只做投影。本条与前端文档 F12 的"重试倒计时"解禁联动。

**配置**：`apps/bootstrap/src/config.rs` 增 `recovery.retry.*` 组；直连主 provider
（无 routing 包装）与路由路径行为一致——预算在 run loop 层，天然覆盖直连。
`docs/runtime/provider-smoke.md` 补充重试行为的验证说明。

**测试**（全部用扩展后的 fake provider 脚本化，无网可跑）：
- 429（带 retry_after）×N 后成功：事件序列 attempt/delay 正确、最终成功；
- 瞬时 500 ×N：独立预算、耗尽后终止；
- 首事件前连接失败 → 重试；`LlmChunk` 已流出后 `StreamInterrupted` → 不重试、终止；
- 退避 sleep 期间 cancel → 立即终止、无悬挂；
- `reason` 字段断言不含 secrets/payload；
- 配置关闭（max_attempts=1）→ 行为与现状一致。

---

## 3. R3 会话 `last_outcome` 字段

### 3.1 现状（已核实）

- `ProductSession`（`apps/api/src/product/contracts.rs:490-511`）没有结果字段；
  终态映射：`Done → NeedsAttention`、`Error|Interrupted → Error`、`Cancelled → Idle`
  （`apps/api/src/lib.rs:3542-3575`）。"刚完成的 idle"与"从未运行的 idle"不可区分
  ——这正是 W 文档 §21 W4 侧栏成功点做不出来的运行时原因。
- Web 侧 `SessionStatus = "idle" | "running" | "error" | "needs_attention"`
  （`apps/web/state/product-types.ts:14`），同样没有 success 概念。
- ProductStore 当前 schema `CURRENT_SCHEMA_VERSION = 15`
  （`apps/api/src/product/store/schema.rs:9`，迁移 001–015 同文件）。

### 3.2 目标合同

- **迁移 016**：`product_sessions` 增列
  `last_outcome TEXT NULL CHECK (last_outcome IN ('success','failed','cancelled'))`
  与 `last_outcome_at TEXT NULL`。NULL = 从未运行（向后兼容：旧行不回填）。
- 写入点：`lib.rs:3542-3575` 的终态迁移同一处——`Done → success`、
  `Error|Interrupted → failed`、`Cancelled → cancelled`。
- `ProductSession` 增 optional 字段 `last_outcome` / `last_outcome_at`
  （additive，serde skip_serializing_if none）。
- Web：`product-api-types.ts` / `product-types.ts` 同步；侧栏会话行显示"最近一次
  结果"小标（success/failed/cancelled 三态点；与现有失败红点语义合并时以
  last_outcome 为准重述文案）；命令面板 session 条目 subtitle 可带结果。
- OpenAPI、`tests/api.rs`（终态后字段正确；NULL 会话序列化缺省）、store 迁移单测
  （v15 → v16、CHECK 约束、旧库升级幂等）。

### 3.3 非目标

- 不改 `ProductSessionStatus` 枚举本身（`NeedsAttention` 语义保留）；
- 不做 per-run 历史结果表（那是 R6/未来评估器的事）。

### 3.4 实施记录

服务端、迁移与 Web 最小接线同批落地（branch `feature/runtime-align-r3`）。

- 迁移 016：`apps/api/src/product/store/schema.rs`（`CURRENT_SCHEMA_VERSION = 16`、
  `MIGRATION_016_COLUMNS` 两列声明、`apply_migration_016` 幂等 + 逐列 `table_has_column`
  守卫）。旧库升级**不回填**：既有行 `last_outcome`/`last_outcome_at` 保持 NULL。
- 合同：`apps/api/src/product/contracts.rs`（`ProductSessionOutcome`、
  `ProductSession` 两个 additive optional 字段、trait 方法签名带
  `Option<ProductSessionOutcome>`）；`store/repository.rs`（读写投影、
  `release_turn_claim_with_status` 用 `COALESCE(?3, last_outcome)` 写入）。
- 写入点：与终态迁移同一事务（`apps/api/src/lib.rs` 的
  `finish_final_product_turn` / `finish_nonfinal_product_turn` /
  `finish_product_turn_needs_attention` / `finish_failed_product_start`）。
- Web：`product/product-api-types.ts`（`PRODUCT_SESSION_OUTCOMES` + 严格解析 + 半写失败关闭）、
  `state/product-types.ts`（`SessionOutcome` + 映射）、`sidebar/session-labels.ts`
  （三态结果点与副标题）、`sidebar/WorkspaceTree.tsx`、`shell/ProductApp.tsx`、
  `copy/{zh-CN,en-US}.ts`、`styles/product-v2.css`。

落地时确认并解决的设计歧义（以本文档为准的记录，不静默改写历史）：

1. **§3.2 写的 "`Done → success`" 与实现的差距**：`Done` 是把回合交给
   `finish_nonfinal_product_turn` 处理的终态之一，语义是"run 结束但没有最终答复"，
   会话状态落到 `NeedsAttention`。因此实现里 `success` **只由带最终答复的路径写入**
   （`finish_session_turn_and_claim_followup` 分支 (a) 与 pending-follow-up 分支 (b)）；
   无答复的 `Done`/`NeedsAttention` 收尾记 `failed`，与 §3.2 的
   "`Error|Interrupted → failed`" 一致。保留 §3.2 原文不改写，差距以本条为准。
2. **未成为回合的启动尝试不写结果**：workspace 提示不匹配（且不是 provider 恢复失败）
   与引擎组装失败两条路径会把状态**还原**为上一步的状态，因此它们传
   `None`，由 `COALESCE` 保留上一次真实结果；只有被分类为失败的路径
   （runtime resume/binding/start、provider resume、需关注的收尾）写 `failed`。
   否则一次失败的启动会覆盖上一回合的真实结果（实现中由
   `product_cancel_releases_the_single_turn_claim_before_continuation` 暴露）。
3. **Web 侧点语义以 `last_outcome` 为准**：旧的 W4.4 规则只看
   `status === "error"`，成功与"从未运行"都画不出东西。实现改为
   `success`/`failed`/`cancelled` 三态点（绿/红/灰），并在载荷完全没有
   `lastOutcome`（早于 v16 的 API）时回退到旧规则；打开该会话仍清除点。
   命令面板 session 条目副标题追加结果词，使副标题可被结果词搜到。
4. **半写载荷失败关闭**：`last_outcome` 与 `last_outcome_at` 由同一条 UPDATE 写入，
   因此 Web 解析器拒绝只有其一的载荷（与 fork provenance 三字段的既有约定一致），
   而不是猜哪一半可信。

非目标（本轮明确不做）：不改 `ProductSessionStatus`；不新增 per-run 结果表；
不回填历史行；TUI 不加结果点（本轮 Web 最小接线，TUI 归前端/后续条目）。

---

## 4. R4 队列协议扩展

### 4.1 现状（已核实）

- 控件状态机：`queued → intervention_requested → applied_current_run`（steer 路径）/
  `claimed_successor`（终态接续）/ `needs_attention` / `revoked`
  （`contracts.rs:289-305`；`message_adapter.rs:183-214`）。
- `promote` 仅在回合 live 时允许：`queued → intervention_requested` 后立即
  `try_send_steer` 注入当前回合；通道拒收 → `abandoned`
  （`apps/api/src/product/routes.rs:992-1046`）。**没有"等边界"的派发语义**。
- `requested_delivery: successor | current_run` 已在消息合同中
  （`contracts.rs:300-305`）——语义字段在，端点未消费完整。
- 重启存活机制已存在：`recover_stale_turn_claims`（`repository.rs:123-179`）+
  `schedule_pending_followup_recovery` → `drain_followup_for_session`
  （`apps/api/src/lib.rs:3373-3441`），idle 会话的 pending followup 会在服务启动后
  CAS-claim 并开新 run。缺的是**端到端验证**与**重排**。
- 无重排端点；Web 队列只有相邻交换（W 文档 §17 因"运行时无重排接口"而降级）。

### 4.2 目标合同

**a) 原子重排**

- `POST /product/sessions/{session_id}/messages/reorder`
  body：`{ ordered_ids: Vec<ProductControlId>, expected_revision: Option<...> }`。
- 语义：仅对 `status = queued` 且属于本会话的控件生效；单事务内按列表重写顺序；
  列表必须恰好覆盖当前全部 queued 控件（多/少/重复/跨会话/非 queued → 400/409
  明确错误码）；响应返回重排后的完整 queued 序列。
- 存储：**迁移 017**：`product_session_controls` 增 `queue_order INTEGER NULL`
  （NULL = 旧序 `created_at` 兼容）；列表排序键：`COALESCE(queue_order, ...)`，
  重排即重写 queue_order。CAS/并发：沿用会话 turn claim 与现有 revision 模式，
  与 promote/revoke 并发时的冲突以 409 返回（前端已有 CAS 恢复范式）。

**b) send-now 的边界语义**

- `promote` 请求体增可选 `delivery: "current_run" | "successor"`（消费既有的
  `requested_delivery` 语义）：
  - `current_run`（默认，兼容现状）：立即 steer 进当前回合；
  - `successor`：提升为队首，**当前回合走到终态边界后**作为下一回合输入派发
    （复用 `claimed_successor` 与 idle-drain 既有路径，等待边界而非中断——
    PI 的 "graceful boundary stop then dispatch" 语义）。
- Web"立即发送"默认采用 `successor` 边界语义并明示文案；"插话（打断）"作为显式
  另一个动作保留 `current_run`。前端联动在前端文档登记。

**c) 重启存活验证（验证门，非新功能）**

- 集成测试：会话带 N 条 queued（含指定顺序）→ 停 API 进程 → 起回 → 断言顺序保持、
  idle drain 按新序派发、`queue_order` 迁移幂等。

### 4.3 验证门

- `tests/api.rs`：重排合法性矩阵（正常/缺项/多项/跨会话/非 queued/并发 409）、
  `delivery=successor` 的边界派发时序（当前回合终态后才派发，期间 revoke 仍有效）。
- Web e2e：重排后顺序、立即发送不打断当前回合、终态后自动派发。
- `docs/runtime/subsystems.md` 队列章节同变更更新。

### 4.4 实现记录（2026-09-26）

- **迁移 017**（`apps/api/src/product/store/schema.rs`）：`product_session_controls` 增
  `queue_order INTEGER NULL`。迁移是纯增列（`MIGRATION_017_COLUMNS` + `apply_migration_017`），
  历史行保持 NULL；`assert_integrated_v14` 校验该列与 schema 记录；重复执行幂等
  （列已存在时只补记 schema 行）。
- **排序键**：队列读取与 claim 的四处查询改为
  `ORDER BY COALESCE(queue_order, seq) ASC, seq ASC`（`repository.rs` 的
  `list_pending_followups`、`claim_next_pending_followup`、`claim_next_followup_turn`、
  `pending_followup_for_session`）。`seq` 是台账顺序，任何重排都不重写它
  （分页、transcript 投影仍按 `seq`）；`queue_order` 只表达投递顺序。
- **契约**（`contracts.rs`）：`ProductMessage` 增可选 `queue_order`（additive、缺省省略，
  旧载荷照常解析，客户端按 `queue_order ?? seq` 排序）；`PromoteProductMessageRequest`
  增可选 `delivery`（缺省 `current_run`，与旧请求体兼容）；新增
  `ReorderProductMessagesRequest`（`ordered_ids`）与 `ProductQueueResponse`（权威队列）。
- **新端点**：`POST /product/sessions/{session_id}/messages/reorder`（单事务内重写
  `queue_order`，返回重排后的完整队列）。
- **promote 的边界语义**：`delivery = "successor"` 把消息移到队首并保持
  `status = queued`（`requested_delivery = successor`，`queue_order` = 当前队首 − 1），
  **不**注入当前回合、不打断 live run；终态边界沿用既有
  `finish_session_turn_and_claim_followup` → `claimed_successor` 路径派发。
  `current_run` 保持原 steer 语义，且重复调用仍是幂等 replay。

实现时的三处决策与偏差（按 AGENTS §10 记录替换关系，而非静默改写设计）：

1. **重排集合 = 后继队列，而非“全部 queued 控件”。** 只接受
   `kind = 'followup' AND status = 'pending'` 的行，与前端合同一致（W 文档 §5.2：只动
   `status === "queued"` 且 `requested_delivery === "successor"` 的消息，`current_run`
   的插话不进该交互）。设计 §4.2a 写的“`status = queued` 的控件”在此等价，因为
   `create_message` 只写入 `requested_delivery = successor` 的 pending 行；
   `intervention_requested`/`claimed_successor`/`applied_current_run`/`needs_attention`/
   `revoked` 一律拒绝。
2. **`expected_revision` 不实现，改为“列表必须精确覆盖当前队列”。** 设计 §4.2a 建议沿用
   revision CAS，但 ProductStore 的会话没有 revision 字段，新增一个存储计数会与控件状态漂移
   （R3 的 `last_outcome` 已经暴露过同类“双事实源”问题）。现在的并发控制是：
   `ordered_ids` 必须与当前 pending 队列逐元素相等——少项/多项/未知 id/跨会话/已非 pending
   → 409 `product_control_conflict`；重复项或超过队列上限 → 400 `product_invalid_input`。
   于是 revoke/promote/新消息造成的任何队列变化都会让旧列表变成 409，等价于乐观并发失败；
   两个纯重排之间的竞争是 last-writer-wins，响应返回权威顺序。错误码仍符合设计 §4.2a 的
   “400/409 明确错误码”。
3. **`successor` 提升仍要求回合 live。** `promote_message` 保留既有的“会话 running + 存在活跃
   turn claim”前置条件：`successor` 的语义是“相对当前回合的终态边界”，离开 live 回合就没有
   可解释的队首，此时返回 409 `product_control_rejected`。idle 会话的 pending 队列由启动恢复的
   drain 负责；重排端点本身不要求 live 回合（它是纯队列操作）。

**重启存活（§4.2c）的验证切分**（诚实记录覆盖面，避免把 skip/近似当成真门）：

- `tests/api.rs::product_queue_order_survives_a_reopen_and_keeps_one_claim_per_session`：
  live 回合 + 3 条重排后的 queued → 用同一 state 目录再开一个 API state → 断言
  `queue_order` 0/1/2 与 id 的映射、`seq` 未变，且恢复把被中断的后继如实标记为
  `needs_attention`（`reason = "API process stopped during follow-up delivery"`）而不重复派发。
  测试进程内两个 state 同时存活，因此这条覆盖“顺序持久 + 中断恢复如实上报”，不是真正的冷启动。
- store 层 `an_idle_session_queue_is_listed_for_recovery_and_drained_in_its_reordered_order`：
  idle 会话 + 重排队列 → `list_idle_sessions_with_pending_followups` 报告该会话（重启后
  `recover_pending_followup_drains` 枚举的正是这个查询）→ `claim_next_followup_turn`
  （drain 的 claim）按新序返回队首，后续边界继续按新序。API 的
  `recover_pending_followup_drains` 是 `pub(crate)`、集成测试无法直接调用，所以“按新序 drain”
  在 store 边界验证；“迁移幂等”由 schema 单测覆盖（`cargo test -p rove-api --lib
  product::store::schema`）。
- `delivery = successor` 的边界时序与期间 revoke 有效性：
  `tests/api.rs::product_successor_promotion_never_steers_the_live_run_and_survives_revoke`。
- 重排合法性矩阵（正常/缺项/多项/跨会话/重复/未知 id/旧列表 409）：
  `tests/api.rs::product_queue_reorder_is_validated_atomic_and_drives_the_next_turns`
  与 store 单测 `a_reorder_list_that_does_not_cover_the_queue_is_rejected_without_moving_anything`。

**未做（显式交接）**：Web 队列仍是“撤销 + 重建”的相邻交换
（`apps/web/shell/ProductApp.tsx` 的 `handleMoveQueuedMessage`）。改用本端点、
以及“立即发送（`successor`，不打断当前回合）/ 插话（`current_run`）”两个动作的 UX
归前端工作流，已在 W 文档 §16 降级表与前端文档登记；§4.3 的 Web e2e 门因此不在本条 PR 内执行。

R4 未触碰：`ProductMessageStatus` 状态机、`seq` 写入路径、transcript 投影、CLI/TUI。

---

## 5. R5 产品级目录 SSE `/product/events`

### 5.1 现状（已核实）

- 唯一 SSE 是 `GET /jobs/{job_id}/events`（`apps/api/src/lib.rs:897-967`；
  `Last-Event-ID` 解析 `lib.rs:4795-4805`）；产品控制事件是**搭 live job 的广播通道**
  （`queue_or_publish_product_control_event`，`lib.rs:3888`）——没有 live job 的会话
  没有任何流。
- Web 只能靠 2.5s 间隔轮询目录（`use-server-product-state.ts:353-396`），
  侧栏徽标、palette、连接状态都可能滞后。

### 5.2 目标合同

- 新端点 `GET /product/events`（`text/event-stream`），帧 `id:` = 产品事件单调
  `seq`，断线用 `Last-Event-ID` 续传（实现复用 job SSE 的解析与回放范式）。
- **迁移 018**：新表
  `product_events (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
  session_id TEXT NULL, workspace_id TEXT NULL, summary TEXT NULL, created_at TEXT NOT NULL)`
  + 滚动清理（保留最近 10_000 行，写入时顺带修剪）。回放即按 `id > after` 查表。
- 事件种类（首批，`kind` 枚举在 contracts 中定义）：
  `session.status_changed`（payload 摘要：session_id、新 status、`last_outcome`
  若有）、`session.created/deleted/updated`、`workspace.*`、`preferences.changed`、
  `control.queued/promoted/revoked`。
- **发布点**：会话终态迁移（`lib.rs:3542-3575` 同点，与 R3 同写事务）、
  目录/偏好 CRUD 路由、控件生命周期路由。摘要字段是**安全摘要**：session 标题
  （鉴权后本可见的用户数据）可以；**消息正文、工具参数、错误细节、秘密一律不进**。
- 鉴权与现有 product 路由一致（token/CORS/rate-limit 既有规则覆盖新端点）。

### 5.3 Web 消费（细节归前端文档）

- `AuthorizedEventSource` 接 `/product/events`；`refreshSessionStatuses` 轮询降级为
  SSE 不可用时的回退 + 30s 低频兜底；
- 前端文档 F5 的后台失败 toast 从"轮询发现"升级为推送驱动（同一去重规则）。

### 5.4 验证门

- 集成测试：订阅 → 触发状态迁移/目录变更 → 收到事件（kind/summary 正确、
  无敏感字段）；断线带 `Last-Event-ID` 重连 → 不丢不重；表滚动清理生效。
- Web e2e（mock SSE）：后台会话失败实时上屏、断线回退轮询。

---

## 6. R6 消息级编辑重发 = fork-at-message

### 6.1 设计决策：不做原地截断

PI 的 `truncateFromMessageId` 是原地截断。rove 的 trace/events 是 append-only，
task_state 历史参与 resume 重放安全（AGENTS §4"已完成工作不重放"）；原地改写历史
违反两条不变量。因此采用 **fork 机制承载编辑重发**：编辑产物是一个新会话，
父会话历史不动。

### 6.2 目标合同

`CreateProductForkRequest`（`contracts.rs:671-683`，现有校验
`apps/api/src/lib.rs:2239-2340`：父会话 Idle、run `done`、`fork_at_event_seq =
run.last_event_seq` 服务端派生）扩展：

- 新可选字段 `truncate_after_message_seq: Option<i64>`：
  - 指定后，子会话的初始历史 = 复制至 fork 点后，**剔除该 user 消息及其之后**
    的全部条目（子会话私有状态的构造期修剪，不触碰父会话、不触碰父 trace）；
  - 约束（违反 → 400/409 明确错误）：目标消息必须是 user 角色、必须位于
    `fork_at_run_id` 对应的终态 run 内、父会话 Idle、消息 seq 必须已在账本中；
  - 编辑后的新文本由客户端在子会话中作为首条消息发送（fork 端点不收文本，
    保持单一职责）。
- 幂等：与现有 fork 幂等键机制一致。
- Web 联动（前端文档登记）：消息操作"编辑并重发"在满足约束时提供
  **"编辑并分支到新会话"**，带确认文案说明产物是新会话——这是与 PI 原地语义的
  记录在案的偏差；现有"复制会话"按钮文案保持。

### 6.3 验证门

- `tests/api.rs`：合法分支（历史截断正确、父不变、幂等重放同子会话）；
  越界矩阵（非 user 消息、消息不在目标 run、父运行中、seq 不存在）。
- Web e2e：编辑 → 分支子会话打开、父会话历史与 UI 不变。

---

## 7. R7 会话内容搜索（FTS5，先单会话）

### 7.1 现状（已核实）

- 无任何 FTS 使用（repo grep 干净）；rusqlite `bundled`
  （workspace `Cargo.toml:63`）；`q` 仅对 `product_sessions.title` 做 LIKE 子串
  （`repository.rs:4688-4773`）；codex 对齐计划明确"FTS 留待后续"。
- 消息正文存于 `product_session_controls`/消息账本与 trace（每 run 有界）。

### 7.2 目标（分两步；本轮只做第一步）

**第一步——单会话消息搜索**：

- 前置核查（实施第一个动作）：对 bundled 构建探测 FTS5 与 trigram tokenizer 可用性
  （`SELECT fts5('a')`；建 trigram 虚表试跑）。trigram 对中英文子串搜索都可用；
  **若 bundled 构建不含 FTS5/trigram**：先解决构建特性（libsqlite3-sys 特性或
  独立编译选项），仍不可行则降级为"LIKE + 有界扫描"（每会话消息量有界，
  可接受），两条路径都在本文档预期内，实施记录必须写明走了哪条。
- 虚表：`product_messages_fts`（trigram tokenizer，外部内容表指向消息正文存储），
  随迁移 019 建；索引维护走同一写事务（消息入账时同步入索引）。
- 端点：`GET /product/sessions/{session_id}/search?q=&cursor=&limit=`：
  返回 `{message_seq, snippet, created_at}` 分页（keyset，复用现有 cursor 范式）；
  `q` 复用 128B 上限；snippet 生成的截取长度有界（如 160 码点）。
- **安全**：结果授权沿用会话归属检查；snippet 是用户自己的消息内容
  （鉴权后本可见），但生成 snippet 时必须过现有 secret redaction 规则
  （`is_secret_filename` 之外的文本级秘密模式——若无现成文本级规则，则 snippet
  只允许命中词前后有界窗口并在实施评审时确认残余风险，记录进安全清单）。
- Web 接入（前端文档登记小节）：会话内命令面板扩展"搜索本会话消息"动作 +
  结果列表跳转（跳转复用 transcript 定位）。

**第二步（登记不做）**：全局跨会话搜索、trace 事件内容搜索——等单会话验证价值后另立。

### 7.3 验证门

- store 单测：中/英样例、大小写、分页 cursor 正确性、索引与消息同事务一致性
  （写入后立即可搜、删除后不再命中）。
- 集成测试：端点授权（他workspace 会话 404/403）、128B 限制、空结果。
- Web e2e：搜索→命中→跳转定位。

---

## 8. R8 附件/图片上传协议（骨架）

### 8.1 定位

rove 当前最大的功能缺口（composer 只有粘贴文本 chip，`chat/composer-paste.ts:10-12`）；
`ProductMessage` 无附件字段（`contracts.rs:307-328`）。本节只冻结**合同骨架与安全
基线**；实施前必须先出独立实施计划（含 provider 层注入设计），未评审不得动工。

### 8.2 合同骨架

- **存储**：product 会话级附件目录（API 侧受控根，非 workspace 内），命名
  服务端生成（attachment_id），**永不信任客户端文件名/路径**；复用预览资源服务
  的安全基线（`join_safe`、`is_secret_filename`、字节上限——参见
  `docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md` 的威胁建模方法，
  附件需要自己的威胁模型表）。
- **端点**：
  - `POST /product/sessions/{id}/attachments`：单附件上传；大小上限 20MiB；
    扩展名 + 实际字节嗅探（magic bytes）双校验；MIME 白名单首批
    （png/jpeg/webp/gif/pdf/txt/md）；返回 `{attachment_id, size, content_type,
    sha256}`；并发/配额上限（每会话未引用附件数、总量）。
  - `GET /product/sessions/{id}/attachments/{aid}`：下载/内联；
    `Content-Disposition` 服务端固定；`Content-Type` 以入库值为准，不回显用户头。
  - 清理：未被消息引用的附件按 TTL 清理（后台任务，周期有界）。
- **消息合同**：`ProductMessage` 增 `attachments: Vec<ProductMessageAttachmentRef>`
  （additive，迁移 019/020 视 R7 是否同批；引用 `{attachment_id, content_type,
  size, name}`，name 仅展示用）。发送消息携带引用（非重传内容）。
- **运行时注入（设计开放点，实施评审必须先答）**：消息→kernel 时附件如何呈现？
  图片走 provider 的 image content part（涉及 `models/` 各协议适配与能力协商——
  provider 不支持时的降级路径必须 typed）；文本类默认以路径引用交给工具读取
  （符合"workspace 检索走工具"的现有立场）。两个方向都要求：
  内容不得直接进 prompt 除非有界且用户可见。

### 8.3 安全清单（实施前逐项回答）

路径逃逸 / MIME 欺骗 / 大小与并发上限 / 解压类格式（首批不做压缩包，直接拒绝）/
秘密扫描（上传时的 `is_secret_filename` 级检查 + 用户警示而非静默拒绝）/ 清理竞态
（引用与 TTL）/ API 响应不回显绝对路径 / Desktop（Tauri）拖拽路径来自宿主的
信任边界。

### 8.4 验证门

以届时实施计划为准；本节验收线 = 威胁模型文档 + 合同测试 + 上传/下载/引用/
清理四条集成用例存在。

### 8.5 实施记录（2026-09-26）

§8.1 的门槛前半已满足，**后半未开始**：本次只出文档，没有任何产品代码。

1. **产出**：独立实施计划
   [`2026-09-26-attachments-implementation-plan.md`](../plans/2026-09-26-attachments-implementation-plan.md)
   与独立威胁模型
   [`2026-09-26-attachments-threat-model.md`](2026-09-26-attachments-threat-model.md)
   （同分支、docs-only）。§8.2/§8.3 原文未被改写，仍是合同上限。
2. **未动工（逐项为否）**：无端点、无表/列、无迁移、`ProductMessage` 无附件字段、
   `models/` 无 image content part 与能力位、`apps/web` 无上传接线、无 Desktop 拖拽处理。
   §8.2 的骨架不代表运行时已支持任何附件行为。
3. **迁移号定案：R8 取 020**。现状 `CURRENT_SCHEMA_VERSION = 17`
   （`apps/api/src/product/store/schema.rs:9`），016 为 R3、017 为 R4（均已落地），
   018 归 R5、019 归 R7（§0.1、§7.2），故 §8.2 "019/020 视 R7 是否同批"确定为 020。
4. **实施计划对本节的两处有意偏离**（评审应先看这两条，证据与理由见计划 §7.1、§13）：
   - 文本类附件不做"以路径引用交给工具读取"：运行时所有读取都以解析后的 workspace 为界
     （`runtime/src/environment.rs:64`），而附件根按 §8.2 在 workspace 之外，
     故改为按 attachment_id 的有界读端口（路径永不进入 provider/prompt）；
   - 大小上限按类型拆分（栅格 16 MiB、其余 20 MiB）：现有栅格上限是 16 MiB
     （`apps/api/src/product/files.rs:29`），统一按 20 MiB 会让 16–20 MiB 的图片被接受却
     无法渲染。计划 §12 Q1 将其列为待决策项。
5. **另发现的文档缺口**：§12 验收矩阵没有 R8 行（本节冻结，本次不回填；
   PR-1 落地时补）。
6. **下一步**：按 `CONTRIBUTING.md` §6，本条属安全敏感/跨包边界变更，
   两个文档需**独立盲审**通过后才可动工；实施按计划 §10 的 PR-1…PR-5 顺序。

---

## 9. R9 压缩两层审计 + `/compact` HTTP 端点

### 9.1 现状（已核实）

- 自动压缩：每个模型回合前评估（`run_loop.rs:614-687`），先丢 history 窗口再对
  丢弃切片做结构化 LLM 摘要（`runtime/src/context/compaction.rs:16-39`，prompt v3；
  熔断 3 次失败，`facade.rs:307`）；硬限直接 TokenLimit 终止。
- 工具结果引用占位已存在：`project_history_results`
  （`runtime/src/context/manager.rs:398+`）把**携带 tool_artifact RichReference**
  的旧工具结果替换为一行引用占位。
- `/compact` 只有 CLI slash 命令（`apps/cli/src/cli/repl.rs:326-366`），
  **无 HTTP 端点**——Web 等待行的 compacting 状态是降级显示，产品壳无法手动压缩。

### 9.2 目标

1. **审计任务（先行，产出文档结论）**：对照 open-vetta 的两层压缩
   （microcompact：每次调用前纯函数修剪旧 bash/工具输出，keepRecent≈8；
   LLM 摘要层在 ~80% 阈值），逐类目核对 rove：
   - 无 artifact 引用的工具结果（尤其 bash 输出）是否全量驻留到压缩触发？
   - 引用占位的覆盖面 vs vetta 的按类修剪；
   - 阈值结构（soft/hard/reserved）与 80% 的对应关系。
   产出：审计记录（放 `docs/runtime/` 或审计附录），结论二选一：
   (a) 覆盖已足够，记录证据关项；
   (b) 存在全量驻留类目 → 设计有界修剪策略：保留最近 N 条全文、更旧替换为
   占位，**修剪必须以 trace 事实记录**（"trace 记录事实"不变量），resume 后
   不重放被修剪内容之外的任何东西。
2. **`POST /product/sessions/{id}/compact`**：等价 CLI 的 `CompactionTrigger::Manual`
   路径（`facade.rs:403-444`）；仅 idle 会话允许；响应含压缩事实（是否触发、
   熔断状态）；Web 设置/等待行接入（前端文档登记）。
   `docs/runtime/react-loop.md` 同变更更新。

### 9.3 验证门

- 审计记录存在且结论明确（不做无结论审计）。
- 若做修剪：单测（修剪边界、resume 一致性、trace 事实存在）。
- compact 端点：集成测试（idle 限制、熔断时响应、幂等）。

---

## 10. R10 登记不在本轮（每条一句为什么）

| 方向 | 为什么现在不做 |
|---|---|
| Plan/Goal 模式 + 不可变 plan 工件 | 大型运行时特性，需先完成 R1/R2 把地基补齐；另立设计 |
| 子代理 `parent_tool_call_id` 事件字段 | 事件合同变更大，且 Web 卡片只是消费端；先登记需求 |
| thinking/推理通道 | 涉及 provider 协议与安全呈现策略（不得把隐藏推理当指令），独立设计 |
| 审批 120s 倒计时/到期自动拒绝 | 绑定 PI 服务端语义，rove 审批无过期合同；此前已明确不移植 |
| 全局跨会话搜索 | 等 R7 第一步验证价值 |
| 服务端摘要自动标题 | 依赖模型调用预算与失败路径，前端版（前端文档 F10）先行 |
| IM 桥/更新通道/插件市场/移动端 | 参考项目的产品面，不在对齐范围 |
| Windows ConPTY、macOS/Linux 打包等 G 门禁 | 与本档无关，属 workstream G 既有缺口 |

---

## 11. 里程碑与 PR 拆分建议

| PR | 内容 | 说明 |
|---|---|---|
| PR-1 | R1（API 游标 + OpenAPI + 合同测试 + Web 接线） | 一个 PR 闭环 F.4（服务端+消费端），避免"半接线"状态过夜 |
| PR-2 | R2c（ProviderRetry 事件 + run loop 预算 + fake 扩展 + Web 等待行） | 事件合同变更横切，独立成 PR |
| PR-3 | R2a + R2b（静默恢复 + 中止保留，共享 fake 脚本化扩展与恢复配置组） | |
| PR-4 | R3（迁移 016 + contracts + Web 成功点） | |
| PR-5 | R4（迁移 017 + 重排/边界派发 + 存活验证） | |
| PR-6 | R5（迁移 018 + 端点 + Web 接入与轮询降级） | 与 F5 toast 联动 |
| PR-7+ | R6 / R7 / R9 端点 / R8（各自独立立项） | R8 前置威胁模型 |

依赖关系：R5 的 Web 消费依赖 R3 的字段（状态变更摘要带 last_outcome，可后补）；
R2a/R2b 共享 fake provider 脚本化扩展（PR-2 先建）；R6 依赖 R1 的 run 边界语义
（弱依赖）。前端文档 PR-B/C/D 与上表 PR-1~6 有软依赖处已各自标注。

## 12. 验收矩阵（实施时逐行填证据）

| 条目 | Rust 门 | 合同/集成用例 | Web 门 | 文档更新 |
|---|---|---|---|---|
| R1 | fmt/clippy/test ✅ | api 3 组（无参兼容、65-run 翻页拼接、边界矩阵+OpenAPI 参数）✅；页内超预算改为 `PageWalk` 单测 ✅（端到端重放未做，见 §1.7 第 4 条） | mock 300-run 翻页拼接 + 锚定 `transcript-pagination.spec.ts` ✅ | implementation-guide / implementation-status / acceptance-matrix ✅（§1.7 第 5 条的更正） |
| R2a | 同上 + fake 脚本化 | 恢复/不循环/关配置/审批照常 | 等待行显示 | react-loop |
| R2b | 同上 | salvage 两分支/幂等/空文本 | "(已中止)"标记 | react-loop |
| R2c | 同上 | 分账/首事件前/流中断不重试/退避取消 | 重试状态行 | provider-smoke |
| R3 | 迁移单测 | 终态字段/序列化 | 成功点 e2e | subsystems |
| R4 | 迁移单测 | 重排矩阵/边界派发/重启存活 | 队列 e2e | subsystems |
| R5 | 迁移单测 | 订阅/续传/清理 | mock SSE e2e | subsystems |
| R6 | — | fork-at-message 矩阵 | 分支 e2e | subsystems |
| R7 | FTS 可用性核查记录 | 中英搜索/授权/分页 | 搜索跳转 e2e | subsystems |
| R9 | 修剪单测（若做） | compact 端点 | — | react-loop |

## 13. 风险登记

- R2b 的 1500ms 收尾窗口与取消 UX 的交互（用户感知"取消变慢"）：窗口只在
  有非空累积文本时存在；UI 取消反馈即时（终态迁移不变）。
- R2c 退避期间 run 占用 job：预算与上限必须可配且默认保守；provider-smoke 补
  长退避不阻塞其他会话的说明（每 run 独立 token）。
- R5 新表的写入放大（每次状态迁移一条）：滚动清理 + 摘要行极小；集成测试覆盖
  清理不丢续传游标。
- R7 中文检索质量（trigram 的召回/噪声）：实施记录必须带真实样例评测，
  不达标退回 LIKE 路径。
- R8 是本档唯一"骨架"条目：未出实施计划与威胁模型前禁止动工。
- 所有事件合同变更（R2b/R2c/R5）的同步面宽：漏一处（OpenAPI/合同测试/Web）
  就是违约——每个 PR 的检查单必须包含"事件五件套"核对。
