# 01 · Kernel / Host：agent loop 抽象与 one kernel, many hosts

## 一句话总结（可背）

> Kernel 把 agent loop 抽象成 trait 状态机放在 core（只依赖 models）；Host 注入策略；
> 三个生产 host（embedded / unplanned / step）共用一个 kernel，零漂移靠类型系统保证而不是靠纪律。
> 加一种执行形态 = 加一个 host，整条机制复用。

## 概念与分层（必背，别讲混）

- **Kernel = Agent Loop**：`run_agent_kernel`（`core/src/kernel.rs`）那个 `while` 循环 + hook 协议。
  不是"循环外面包了一层"，**循环本身就是 kernel**——只是循环体调 trait 而非写死逻辑。
- **Host**：实现 `AgentKernelHost` trait 的具体策略载体。项目里有**三个**实现：
  - `EmbeddedKernelHost`（`core/src/agent.rs:225`）——内存态、极简；
  - `UnplannedKernelHost`（`runtime/src/engine/run_loop.rs:557`）——React（非计划）全功能；
  - `StepKernelHost`（`runtime/src/engine/step_runner.rs:258`）——计划模式每步执行。
- **Harness ⊃ Kernel**：Kernel + Host + ToolRegistry + Executor + model_turn 解析器 + tool_result envelope
  的整体 = 围绕模型的所有工程机制。
- **依赖方向**：`models ← core ← runtime ← apps/bootstrap ← {cli, api, desktop, bench}`。
  core 只依赖 models，零本地依赖。

比喻：Kernel = 发动机，Host = 发动机调校，Harness = 整辆车。

## 架构动机（为什么 kernel 放 core、策略放 host）

两个使用场景逼出抽象：

1. `core::Agent` —— 嵌入式，进程内跑一个不持久化的 agent（库用户、测试、benchmark 用）；
2. `UnplannedKernelHost` —— 全功能持久化执行（CLI/API/Web/Desktop 共用）。

如果两套 loop 各写各的，prompt 构建、工具历史格式、终止条件迟早漂移。rove 的选择：
**把循环骨架抽成 trait 协议放 core，策略由 host 注入**——零漂移靠类型系统保证。
core 不能依赖 runtime 类型是硬约束（否则成环依赖、embedded 场景背上用不到的依赖、不可单测）。

## 核心机制走查

### kernel 循环（`run_agent_kernel`，每次循环 7 步）

```
loop {
 ① 检查 cancel / max_model_turns                        → 超限直接终止
 ② host.before_model_turn() → Ready(messages)           ← 策略注入点：prompt/compaction/memory/token/steer
 ③ host.model_turn(messages) → ModelTurn                ← 流式调模型 + 解析
 ④ host.after_model_turn()                              ← 持久化、Evaluator
 ⑤ kernel 分派 turn.action：
      Final     → host.after_final → Complete | Continue
      Malformed → 追加 full_response + 重试消息，受 max_repairs 限制
      ToolCall / ToolBatch → 检 max_tool_calls → host.tool_turn
 ⑥ state.history.extend(host.tool_history(...))         ← host 决定怎么把响应+结果写进历史
 ⑦ host.after_tool_turn()
}
```

**kernel 完全不知道 prompt 长什么样**——`before_model_turn` 返回 `Ready(messages)` 后 kernel 才调
`model_turn`，prompt 是 host 的事。同样，tool 结果怎么进历史（`tool_history`）也是 host 的事。

### AgentKernelHost：6 个 hook（面试官可能让你默写）

| Hook | 注入什么策略 |
|---|---|
| `before_model_turn` | token 预算、steer、prompt 构建、compaction、memory flush |
| `model_turn` | 调模型（core 默认路径 `run_model_turn`，runtime 包层 steer 生命周期事件）|
| `after_model_turn` | 持久化、Evaluator 决策 |
| `tool_turn` | 路径作用域检查、工具执行管道 |
| `tool_history` | 响应 + 结果 → history 消息格式 |
| `after_final` | 决定 Complete 还是 Continue（embedded 用它实现 follow_up）|

每个 hook 都能 `Stop`——这是 runtime 注入策略**提前终止 run 的通道**
（终止为 `KernelTermination::Extension { reason, output }`）。

### 三个 Host 对照

| | `EmbeddedKernelHost` | `UnplannedKernelHost` | `StepKernelHost` |
|---|---|---|---|
| 位置 | `core/src/agent.rs:225` | `runtime/src/engine/run_loop.rs:557` | `runtime/src/engine/step_runner.rs:258` |
| `before_model_turn` | 3 行：drain steering → Ready | ~60 行：token 预算 → steer → 构建 prompt → 硬限检查 → compaction + memory flush | 构建 step-local 聚焦 prompt + 全局历史 |
| `after_final` | drain follow_up 队列，非空则 Continue | 恒 Complete | — |
| `tool_turn` | parallel-safe 并发 / 串行 | 路径作用域检查 + 完整 Executor 管道 | 复用 |
| 场景 | 库用户 / 测试 / benchmark | React 策略：CLI/API/Web/Desktop | PlanReact 策略：StepRunner |

记忆点：**embedded 证明 kernel 能裸跑，unplanned 证明 kernel 能扛全功能，
step 证明 kernel 能在计划模式下逐 step 复用——同一个 `run_agent_kernel`，三种复杂度。**

### 终止模型（`KernelTermination`，10 种）

```
Final / ModelTurnLimit / ToolCallLimit / RepairLimit / Cancelled
ModelFailed / IncompleteBeforeModelTurn / IncompleteModelTurn / IncompleteToolTurn
Extension { reason, output }
```

- `Incomplete*` 三态：stream 意外结束（返回 `None`）是**类型化错误**，不是静默当成 Final；
- `Extension`：任意 hook `Stop` 的出口，`reason` 是 host 自己的类型
  （embedded 是 `AgentError`，runtime 是 `RuntimeKernelStop::TokenLimit`）。

## 设计决策与面试亮点

1. **整批预留**（kernel 循环第 ⑤ 步内）：`state.tool_calls += 整批数量` 在**调度前**整批加，
   limit 检查对整批做——防半批执行（悬空引用 + 副作用不一致）。
   测试 `embedded_batch_reserves_tool_budget_before_any_dispatch` 验证 dispatch = 0。
   一句话：**模型请求的 tool batch 必须全有或全无。**
2. **biased select + cancel**：每个 await 点 `tokio::select! { biased; _ = cancel_token.cancelled() => ... }`，
   取消即时生效（compaction 中、流式返回中、工具执行中都能在下一个 await 点中断）。
3. **模型输出解析契约**（`core/src/model_turn.rs`）：
   - `capabilities.validate_tools` **先于模型 dispatch**——非法 schema 连请求都不发
     （测试 `invalid_tool_schema_fails_before_model_dispatch` 断言 dispatch == 0）；
   - **terminal event 契约**：`requires_terminal_event()` 为 true 的 provider 必须收到 `Done`，
     流提前结束报 `ModelError::StreamInterrupted`（测试 `truncated_stream_never_finishes_a_turn`）；
   - **legacy EOF 兼容**：不需要 terminal event 的 provider 用 EOF 当终止
     （测试 `legacy_eof_clients_remain_compatible_until_they_opt_in`）；
   - **native tool-use 优先**：保留 provider 的 `tool_use_id`，JSON 文本解析只是兼容路径
     （`tool_calls.is_empty()` 时才 `parse_action`）。
4. **工具结果重试安全**（`core/src/tool_result.rs`）：
   - `ToolResultOutcome` 7 态：`Success / Partial / Error / Rejected / Cancelled / TimedOutKnownNotSent / Indeterminate`；
   - `is_safely_retryable` **显式排除 `Indeterminate`**——只有 transport 能证明请求从未离开客户端
     （`TimedOutKnownNotSent`）才可重试，否则外部副作用未知，不可当没发生重试。
   - 测试 `outcome_retry_safety_never_includes_indeterminate` 逐一断言。
5. **工具结果四投影**（`ToolOutputEnvelope`）：`model_projection`（二进制永不出现，indeterminate
   会明说"可能未生效，不要假设已执行"）/ `ui_projection` / `finalizer_projection`（只给证据引用，
   不给原文）/ `audit_projection`（hash + lineage，无 secret）。`enforce_bounds` 构造时执行一次，
   超限截断、丢 block 时 `Success` 降级 `Partial` 并记 diagnostic。
6. **纵深防御**（如只读模式三层）：注册表组合（不注册写工具）→ descriptor `destructive` 标志硬拒
   → `ToolPolicy` 最后闸。模型只"请求"，权限由工具层决定。

## ToolRegistry（`core/src/tools.rs`）

- `BTreeMap` 词法排序 → schema 列表确定性输出，prompt 稳定；
- 注册时**钉住** descriptor + schema，重复名/非法 schema 直接失败不覆盖；
- `snapshot()` 冻结：run 开始时精确快照，MCP 热刷新**不会**替换正在跑的计划底下的 schema；
- `ToolRegistryPublisher` 用 Weak ref 防引用环；
- `ToolContext` 只带 `call_id` + `cancel_token`，runtime 服务通过 `with_extension` 类型化注入
  （core 不认识 runtime 类型）。

## 市场对比

### Claude Code（核过 `claude-code-analysis/`）

CC 的 `query()` 是单个 `AsyncGenerator`，循环体**写死**策略：
`claudeApi.stream()` / `extractToolUseBlocks()` / `runTools()` / `executePostSamplingHooks()` /
`shouldAutoCompact()`。所有运行形态共用一个函数**靠纪律**。
rove 抽成 trait，**靠类型系统**：策略可替换不碰循环（unplanned → planned 换 host，
`run_planned_loop` 就是证据）、每个 hook 可单测、embedded + durable 共用 kernel 零漂移。

### Pi（核过 `_pideck_analysis/pideck-main.js`）

| Pi | Rove | 职责 |
|---|---|---|
| `pideck-main.js`（Electron 壳）| `apps/desktop` | 界面 + 进程管理，**不跑 loop** |
| `pi` CLI 子进程（`spawn` 于 `:2697`）| `runtime` Engine + Harness | 执行体 |
| stdin/stdout JSON-line 协议（`handleLine` `:1981`）| API/SSE | 通信 |
| `--approve`/`--no-approve`（`:2650`）+ `trust.json`（`:5859`）| `ToolPolicy` + 审批策略 | 权限注入 |

- **Pi 没有 Host 层**：loop 写死在子进程内部，无 trait 抽象；
- 但有一个**类似的想法**：壳不碰 loop，策略通过进程外注入（启动参数 + 配置文件）；
- **本质区别**：rove 是进程内、类型化、可组合（trait impl）；Pi 是进程外、黑盒、不可组合
  （flag + JSON，改策略要重启进程，无类型约束）；
- **为什么 Pi 不需要 Host**：它只有一个运行形态，没有"同一 loop 跑多种形态"的需求。
  **Host 是"多形态复用"需求的产物**——没有需求就没有抽象。

### 其他市场参照

- **OpenHands**：`AgentController` 驱动事件循环，`Agent` 抽象类实现 `step(state)`——loop 与策略分离，
  最像的近亲；
- **OpenAI Agents SDK**：`Runner`（loop）+ `Agent` 配置 + 生命周期 hooks——同思路；
- **LangGraph**：控制流显式建模成图状态机——不同范式，同一想法；
- **Rig**（Rust agent 框架）：`Agent` trait。

### 三方对比总表

| 维度 | Claude Code | Pi | Rove |
|---|---|---|---|
| loop 形态 | 单 `AsyncGenerator`（写死策略）| 子进程内部（不可见）| trait 状态机 + host 注入 |
| 策略注入 | 循环体内写死 | 启动参数 + 配置文件（进程外）| `AgentKernelHost` 6 hook（进程内类型化）|
| 多形态复用 | 单 query() 靠纪律 | 只有一个形态，无需求 | 同一 kernel，三个 host + Review |
| 可测性 | 集成级 | 黑盒 | 每个 hook 可单测 |

一句话：**CC 靠纪律、Pi 靠外部配置、rove 靠类型系统。**

## 扩展模型（封闭协议 vs 开放通道）

分两层，别讲混：

1. **Kernel 协议层：6 个 hook 是封闭的。** 不能加新的生命周期点，加 hook 必须改 core
   （对所有 impl 是 breaking change）。**但** `AgentKernelHost` 是 `pub` trait，任何 crate 都能
   为自己的类型实现它——runtime 就是"core 外部"实现 core trait 的活证据。
   **"加一个新 host"是开放的，"改协议"是封闭的。** 封闭是特性：kernel 简单、契约稳定。
2. **Runtime 行为层：有一套可扩展的 hook 注册机制**（`runtime/src/tools/hooks/`）——
   `PreToolHook` / `PostToolHook` / `PostRunHook`，注册在 hook 注册表。**session memory 就是一个
   PostRunHook**（run 结束才写）。这才是真正的外部扩展通道。
3. **另外两条扩展通道**：`ToolContext::with_extension`（类型化服务注入）；
   `ToolRegistryPublisher`（外部工具源如 MCP 发布工具进注册表）。

## 面试速答卡

| 问题 | 一句话答案 |
|---|---|
| Kernel 是什么？| agent loop 本身，trait 状态机，放 core，只依赖 models |
| Host 是什么？| kernel 骨架的具体策略实现；三个：embedded / unplanned / step |
| 为什么拆？| 复用同 loop 跑多形态，零漂移；core 保持 runtime-neutral |
| 加执行形态贵吗？| 加 host 就行，`run_planned_loop` 和 Review 模式是现成证据 |
| 取消怎么实现？| cancel_token + 每个 await 点 biased select |
| 半批执行怎么防？| 整批预留，全有或全无 |
| 工具失败能重试吗？| 只重试"证明没发出"或"本地拒绝"的；Indeterminate 不可重试 |
| Pi 有 Host 吗？| 没有；它是"进程外配置注入"，rove 是"进程内 trait 注入" |
| kernel 是自创的吗？| 模式不是（OpenHands/OpenAI SDK 同思路）；runtime-neutral core + 三 host 落地是我们做的 |
| hook 能外部扩展吗？| 协议层封闭（6 hook 稳定契约）；行为层开放（runtime hook 注册 + with_extension + publisher）|

## 高频追问方向

1. "为什么不用普通的 `while` + 回调？" → trait stream 可取消、可 yield 事件、可 Stop
2. "kernel 怎么知道 prompt 长什么样？" → 不知道，`Ready(messages)` 是 host 给的
3. "取消发生在哪里？" → 每个 await 点 biased select
4. "半批执行怎么防？" → 整批预留
5. "`Indeterminate` 为什么不重试？" → 外部副作用未知，重试 = 重复副作用
6. "加个新的执行形态要改 kernel 吗？" → 不用，换 host（`run_planned_loop` / Review 是证据）
7. "embedded 和 runtime host 的区别？" → 策略复杂度不同，循环相同
8. "为什么 stream 结束不能当 Final？" → `Incomplete*` 类型化错误，半截响应不能算完成
9. "这是你发明的架构吗？" → 模式不新（Template Method/Strategy；OpenHands/OpenAI SDK 同思路）；
   我们的特点是 runtime-neutral core + 三个生产形态共用 + 类型系统强制 + 测试钉住

## 代码锚点索引

| 锚点 | 内容 |
|---|---|
| `core/src/kernel.rs` `run_agent_kernel` | kernel 循环 |
| `core/src/kernel.rs` `AgentKernelHost` | 6 hook 协议 |
| `core/src/kernel.rs` `KernelTermination` | 10 种终止 |
| `core/src/agent.rs:225` `EmbeddedKernelHost` | 嵌入式 host |
| `runtime/src/engine/run_loop.rs:557` `UnplannedKernelHost` | React 全功能 host |
| `runtime/src/engine/step_runner.rs:258` `StepKernelHost` | 计划模式 step host |
| `core/src/model_turn.rs` `run_model_turn` | 模型输出解析契约 |
| `core/src/tool_result.rs` `ToolOutputEnvelope` / `ToolResultOutcome` | 工具结果契约 |
| `core/src/tools.rs` `ToolRegistry` / `snapshot` | 工具注册与快照 |
| `runtime/src/tools/hooks/mod.rs` | 可扩展 hook 注册机制 |
