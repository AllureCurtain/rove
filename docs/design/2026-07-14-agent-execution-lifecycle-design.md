# rove Agent Execution Lifecycle Design - 2026-07-14

> Status: **Partially implemented — remaining target is proposed**
>
> 本文同时保留原始目标与尚未完成的设计，不是当前实现说明。当前运行时事实仍以 [`docs/runtime/`](../runtime/README.md) 为准；不得因为某一阶段已落地，就把本文剩余的类型、配置或行为整体描述为已实现。

本文定义 rove 下一阶段的 Agent 执行生命周期：如何选择 `react` 或 `plan_react`，Planner 在什么上下文中规划，单个计划步骤如何运行 bounded ReAct，步骤结果如何形成 append-only ledger，何时继续、替换剩余计划或结束，以及最终答案如何基于证据生成。

设计参考了 OnCall 项目中的 Plan-Execute-Replan 思想，但不复制其产品目标、LangGraph 状态图或 Python/LangChain 实现。借鉴对象是机制，不是框架。

## Implementation checkpoint

The following slices are implemented and documented in
[`docs/runtime/react-loop.md`](../runtime/react-loop.md):

- typed `ExecutionPolicy` with `react` / `plan_react` compatibility resolution;
- bounded multi-turn ReAct inside each planned step;
- append-only terminal `StepRecord` facts and conservative resume behavior;
- immutable parent-linked `PlanRevision` values;
- deterministic rule-first `PlanDecision` values and canonical
  `step_result` / `plan_decision` / `plan_revised` events;
- task-state/checkpoint/report/SQLite projections of that lifecycle.

Still proposed: capability/procedure-aware planning, model-on-ambiguity
evaluation, an independent evidence-grounded Finalizer, public
multidimensional budget configuration and global enforcement, structured
budget/finalization events, and trace-tail reconciliation newer than the latest
task-state snapshot.

Section 2 below is the design-time snapshot that motivated the work; its file
paths and “current behavior” statements are historical. Use current code and
runtime docs for present behavior.

## Suggested /goal Objective

后续进入实现阶段时，可以基于本文建立独立 `/goal`：

> Based on `docs/design/2026-07-14-agent-execution-lifecycle-design.md`, evolve rove's agent execution lifecycle from a boolean planned loop into explicit `react` and `plan_react` strategies; add capability-aware planning, bounded ReAct step execution, an append-only step ledger and plan revision chain, rule-first plan evaluation, evidence-grounded finalization, multidimensional budgets, and resumable lifecycle state while preserving rove's provider, tool-safety, event, state, and runtime-identity boundaries.

## 1. Scope and Source-of-Truth Boundary

### 1.1 本文解决什么

本文只解决 Agent 的“执行控制面”问题：

- 一个请求采用哪种 execution strategy；
- Planner 能看到哪些目标、约束、工具能力、程序性知识和预算；
- 一个计划步骤何时算真正完成；
- 工具结果如何被模型再次理解，而不是直接冒充步骤结论；
- 已完成结果如何稳定保留，剩余工作如何被修订；
- 何时继续、重规划或提前结束；
- 最终回答如何从执行证据中生成；
- 上述状态如何进入事件、artifact、checkpoint 和 resume 语义。

### 1.2 事实来源的优先级

在本设计尚未实现时，解释 rove 行为应遵循以下优先级：

1. `src/` 中的当前代码和测试；
2. [`docs/runtime/`](../runtime/README.md) 中的当前实现文档；
3. 本文及其他 `docs/design/` 中的未来设计。

如果本文与当前 runtime 文档看起来冲突，应理解为“未来目标尚未落地”，而不是当前实现已经改变。

### 1.3 参考项目边界

本文分析所使用的 OnCall snapshot 位于 rove 仓库同级目录：

```text
../OnCall/super_biz_agent_py-release-2026-03-16
```

它与 rove 的产品目标不同：OnCall 偏向运维问答、知识检索和 AIOps 工作流，rove 是 local-first、stateful、可恢复的通用 Agent runtime。因此本文不会用功能数量或业务场景做横向胜负判断，只比较可迁移的 Agent 机制。

### 1.4 Decision Summary

| Area | Decision |
|---|---|
| Framework | 借鉴 Plan-Execute-Replan 状态机思想，不引入 LangGraph |
| Strategy | 用显式 `react` / `plan_react` 替代单一 `plan_enabled` 语义 |
| Planning | Planner 使用 capability-aware、procedure-aware、budget-aware context，但不直接执行工具 |
| Step execution | 每个 plan step 运行 bounded ReAct；tool success 不等于 step success |
| Historical facts | `StepRecord` append-only，tool/evidence/mutation 不被 replan 覆盖 |
| Plan change | `PlanRevision` 只替换 remaining work，并保留 parent chain |
| Decision | `PlanEvaluator` rule-first，输出 `continue` / `replace_remaining` / `finish` |
| User answer | `Finalizer` 基于 ledger 和 evidence 生成；失败时 deterministic fallback |
| Limits | plan steps、step attempts、model turns、tool calls、revisions、time、tokens/cost 分开预算 |
| Persistence | 复用 trace + task state + checkpoint + report + SQLite index，不建立第二套状态系统 |
| Safety | 计划不是授权；workspace、approval、destructive ordering 始终由现有 runtime 执行 |
| Documentation | 已实现 slices 写入 `docs/runtime/`；本文其余目标保持 Proposed |

## 2. Current State: rove 现在真实做了什么

### 2.1 已有基础是可靠的

rove 当前并不是缺少 Agent loop，而是 planned execution 的控制语义还不够细。以下边界已经形成，应继续保留：

- `ModelClient` 和归一化 `ModelEvent` 隔离 provider 差异；
- `run_model_turn`、`run_tool_turn` 和 history writeback 被拆成共享模块；
- tool registry、workspace boundary、approval、destructive classification 和 batch ordering 已有明确边界；
- `trace.jsonl`、`task_state.json`、`report.json` 和 SQLite index 提供事件、快照和索引；
- `PromptCheckpoint`、compaction 和 runtime identity 已进入恢复链路；
- CLI、API 和 Web 消费相同的 `StreamEvent`，接口层不是另一套执行内核；
- cancellation、token hard limit、restart interruption 和 resume compatibility 已有工程基础。

这些能力比 OnCall snapshot 中的内存 checkpointer 和框架默认状态管理更适合 rove 的长期目标。新设计应在这些边界上演进，不引入第二套 runtime。

### 2.2 当前 planned loop 的关键事实

| Area | 当前行为 | 证据位置 | 设计上的含义 |
|---|---|---|---|
| Strategy | core 使用 `plan_enabled: bool` 在 planned/unplanned loop 之间选择；共享接口装配固定传入 `true` | `src/core/engine.rs`, `src/interfaces/runtime.rs` | core 有两条路径，但 CLI/API 用户还没有明确的 strategy 语义 |
| Planner input | Planner 接收 goal 与 history，调用模型时 tools 参数为 `[]` | `src/core/planner.rs` | Planner 不知道本次运行实际可用的工具及其约束 |
| Plan model | `TaskPlan` 只有 goal、steps、current_step；`PlanStep` 只有 id、title、done | `src/core/types.rs` | 没有 revision、依赖、完成条件、证据要求或步骤结果 |
| Step execution | 每个计划步骤只进行一次 model turn；若产生 tool call/batch，则执行一次 tool turn | `src/core/plan_loop.rs` | 当前“ReAct inside”是一个 conceptual turn，不是步骤内部的 bounded loop |
| Step completion | 工具成功后立即 `mark_current_done()` | `src/core/plan_loop.rs` | 模型没有机会读取工具结果、判断是否还需工具或形成步骤结论 |
| Failure handling | tool/malformed failure 才触发 Planner，再用新 `TaskPlan` 整体覆盖 active plan | `src/core/plan_loop.rs` | 已完成事实主要靠 history 和旧事件间接保留，没有显式 revision chain |
| Replan decision | 没有独立 Evaluator；成功时必然进入下一步，失败时必然尝试重新 draft | `src/core/plan_loop.rs` | 不能在信息已足够时提前结束，也不能在成功结果推翻后续假设时主动改计划 |
| Final output | run 结束时使用最后一次 step text 或最后一个 tool history output | `src/core/plan_loop.rs` | 没有独立 Finalizer 汇总全部步骤、失败和证据 |
| Events | 有 `PlanCreated`、step started/completed/failed；重新规划仍发 `PlanCreated` | `src/core/events.rs` | 初始计划和修订无法从事件类型上区分 |
| Persistence | checkpoint 保存当前 plan，runtime identity 保存 `plan_enabled`、planner prompt hash 和 tool signature | `src/core/types.rs`, `src/core/runtime_identity.rs`, `src/state/artifacts.rs` | 已有可扩展基础，但没有 strategy、revision chain、step ledger 和多维预算状态 |
| Limits | `max_steps` 在 unplanned loop 中近似 model-turn limit，在 planned loop 中近似 plan-step limit | `src/core/run_loop.rs`, `src/core/plan_loop.rs` | 同一配置在不同 strategy 下含义不同，未来 bounded step loop 会进一步放大歧义 |

### 2.3 对当前 “Plan outside, ReAct inside” 文档的精确定义

[`docs/runtime/react-loop.md`](../runtime/react-loop.md) 将现状描述为 “Plan outside, ReAct inside”。这一描述对共享的 `ContextBuild -> ModelTurn -> Action -> ToolTurn -> HistoryAppend` 单元是成立的，但当前 planned loop 在一次成功 tool turn 后就完成步骤，并不会回到同一步再进行下一次 model turn。

本文使用更严格的术语：

- **ReactTurn**：一次模型决策，以及可选的一次 tool turn 和 history append；
- **bounded ReAct step**：在同一 plan step 内执行一个或多个 ReactTurn，直到模型基于工具结果给出 step conclusion，或该步骤的预算/失败策略终止；
- **run-level ReAct**：当前 `run_unplanned_loop` 的循环，直到直接得到用户答案。

因此，本文不是否定现有文档，而是把 planned loop 中的 “inside” 从单个 ReactTurn 收紧为真正有边界的 step loop。

## 3. OnCall 中值得借鉴与不应照搬的部分

### 3.1 值得借鉴的机制

OnCall snapshot 提供了五个有价值的机制样本：

1. **能力感知的规划**

   `app/agent/aiops/planner.py` 在规划前获取本地与 MCP 工具描述，使计划与真实能力对齐。

2. **规划前的程序性知识检索**

   Planner 先通过 `retrieve_knowledge` 获取运维经验，再将经验放入规划上下文。这说明 RAG 不只用于回答事实，也可以用于注入“怎么做”的 procedural knowledge。

3. **工具结果回到模型进行步骤综合**

   `app/agent/aiops/executor.py` 在工具执行后再次调用模型，再把综合结果写入 `past_steps`。这比把原始工具输出直接当作步骤完成更合理。

4. **append-only 的执行历史**

   `PlanExecuteState.past_steps` 使用追加语义保存 `(step, result)`，剩余计划的变化不会抹去已经完成的结果。

5. **继续、替换剩余计划、结束的三态决策**

   `app/agent/aiops/replanner.py` 区分 `continue`、`replan`、`respond`，并用独立 response generation 汇总执行历史。这比“成功必继续、失败必重 draft”更完整。

### 3.2 不应照搬的实现选择

| OnCall 做法 | 为什么不直接采用 | rove 的调整方向 |
|---|---|---|
| 用 LangGraph 固定 Planner/Executor/Replanner 节点 | rove 已有事件流、持久化、resume、approval 和 provider 边界，引入另一套图状态会形成双重控制面 | 保持 Rust engine orchestration，吸收状态机思想 |
| `MemorySaver` 作为 checkpointer | 不能满足 rove 的跨进程恢复、artifact 审计和 SQLite replay | 继续以 trace + task snapshot + checkpoint + index 为基础 |
| Planner 异常时返回“收集/分析/报告”默认计划 | 计划看似存在但与能力和目标无关，会制造伪确定性 | 显式降级到 `react` 或明确失败，并发出 degradation event |
| Executor 异常后仍移除当前步骤 | 失败步骤被消费，后续无法区分完成与跳过 | 写入 failed `StepRecord`，由 Evaluator 决定重试、替换或结束 |
| 工具列表和最大步骤数硬编码 | 与 runtime config、capability snapshot 和 provider budget 脱节 | 从统一 config 和本次运行 budget 构造 |
| Replanner prompt 中用固定 “3/5/8 步”启发式 | 业务经验被固化成不可解释的全局规则 | 使用类型化预算和 rule-first policy，模型只处理歧义 |
| 将执行结果截断到固定字符数后交给 Replanner | 可能丢失关键证据，且没有 artifact reference | 使用 bounded summary + evidence/artifact references |
| Planner/Executor 各自重新获取工具 | 同一次运行的 capability 可能漂移，且重复连接 MCP | 在运行开始生成带 signature 的 capability snapshot |

### 3.3 rove 已经做得更好的部分

与 OnCall snapshot 相比，rove 应明确保留以下优势：

- provider-neutral 的 streaming/model boundary；
- native tool-use 与兼容 JSON action 的统一转换；
- destructive tool approval、workspace safety 和稳定的 batch writeback；
- append-only trace、可读 artifacts、SQLite index 和 restart semantics；
- token-aware context、compaction metadata 与 checkpoint-first resume；
- runtime identity 对 model、prompt、workspace、approval 和 tool signature 的兼容性检查；
- CLI/API/Web 共用 core events，而不是由服务层重新解释一套 Agent 状态；
- safe `model_status`，不把隐藏 reasoning 当作观测数据输出。

设计结论不是把 rove 改造成 OnCall，而是把 OnCall 在 execution control 上更清楚的思想，接入 rove 已经更强的 runtime substrate。

## 4. Design Goals

本文的目标是：

1. 用明确的 `ExecutionStrategy` 代替含义有限的 `plan_enabled` boolean。
2. 让 Planner 基于真实 capability snapshot、约束、程序性知识、memory 和剩余预算规划。
3. 让每个 plan step 运行 bounded ReAct，而不是在首次工具成功后直接完成。
4. 用 append-only `StepRecord` 保存已执行事实，用 `PlanRevision` 表达剩余工作的变化。
5. 把“计划评估/修订”和“最终答案生成”拆成 `PlanEvaluator` 与 `Finalizer`。
6. 采用 rule-first、model-on-ambiguity 的决策方式，降低额外成本和不可预测性。
7. 把 plan steps、model turns、tool calls、plan revisions、wall time、tokens 和 cost 分开计量。
8. 为策略、步骤账本、计划版本和预算提供事件、artifact、checkpoint 与 resume 语义。
9. 保持 tool safety、provider、context、state、event 和 interface 边界不倒退。
10. 所有降级、部分完成和预算耗尽都必须可见，不得伪装成完整成功。

## 5. Non-Goals

本文不做以下事情：

- 不引入 LangGraph 或其他 workflow framework；
- 不设计多 Agent delegation、swarm 或角色协作；
- 不允许 Planner 绕过 Executor 直接执行工具；
- 不把 plan 当作安全授权，候选工具和参数提示仍需运行时校验与 approval；
- 不在本轮定义 Agent package、`AGENTS.md`、skill/procedure 的完整装载协议；
- 不在本轮定义 MCP Streamable HTTP、artifact transport 或资源引用协议；
- 不在本轮制定完整 OnCall-vs-rove benchmark/evaluation plan；
- 不自动恢复进程崩溃时未知状态的 destructive tool call；
- 不承诺完全自主、无限步骤或无限重规划；
- 不在本文中修改任何 Rust、Web、配置或 runtime 文档的“已实现”状态。

上述主题中与本文相关的接口会预留，但分别由后续专门 spec 收口。

## 6. Design Principles

### 6.1 Strategy is explicit

运行策略必须是可配置、可记录、可恢复的状态。不能再只靠 interface assembly 中一个不可见的 boolean 决定。

### 6.2 Plan is mutable; facts are append-only

计划是对未来工作的假设，可以被修订；已经执行的工具、证据、变更和步骤结论是历史事实，只能追加或补充关联，不能被新计划覆盖。

### 6.3 A tool success is not a step conclusion

工具返回成功只表示一次调用完成。步骤是否满足目标，需要模型读取工具结果并形成结论，或由明确的 deterministic completion rule 判断。

### 6.4 Rule-first, model when ambiguous

能由状态和预算确定的决策不调用模型。模型只处理“证据是否改变计划”“剩余步骤是否仍必要”等语义歧义。

### 6.5 Evidence over prose

Step summary 和 final answer 应引用真实 tool call、artifact、mutation 或结构化 observation。大段原始输出保留在 trace/artifact 中，ledger 只保存 bounded summary 和引用。

### 6.6 Safety boundaries survive planning

Planner 建议某个工具，不代表工具可用、参数安全或用户已批准。Tool registry、workspace boundary、approval policy 和 destructive ordering 始终具有最终决定权。

### 6.7 Stable boundaries before clever recovery

优先保证在 plan revision、step result 和 finalization 边界可以稳定恢复。未知中间状态不通过猜测或盲目重放来“自动修复”。

### 6.8 Events describe decisions, not hidden reasoning

事件可以包含 decision、reason code、证据引用和安全摘要，但不能暴露 chain-of-thought 或 provider thinking delta。

## 7. Target Architecture

```text
CLI / API / Web
    -> Runtime Facade
        -> ExecutionStrategySelector
            -> react
            |    -> existing Run-level ReAct Runner
            |
            -> plan_react
                 -> PlannerContextBuilder
                 -> Planner
                 -> Plan-React Coordinator
                      -> StepRunner (bounded ReAct)
                      -> StepRecord Ledger
                      -> PlanEvaluator / Replanner
                           -> continue
                           -> replace_remaining
                           -> finish
                 -> Finalizer

Shared runtime substrate
    -> ModelClient / ModelEvent normalization
    -> Context + Compaction
    -> Tool Registry / Approval / Batch execution
    -> StreamEvent / Trace
    -> TaskState / PromptCheckpoint / RuntimeIdentity
    -> Report / SQLite index
```

这里的 `Plan-React Coordinator` 是 engine 内部的 orchestration boundary，不是新的外部 workflow service，也不要求图框架。

## 8. ExecutionStrategy

### 8.1 Target type

概念上的类型只有两个运行策略：

```text
ExecutionStrategy
  - react
  - plan_react
```

- `react`：沿用 run-level ReAct，适合直接问答、探索性任务或不值得先规划的短任务；
- `plan_react`：先形成结构化计划，每个步骤内部运行 bounded ReAct，再评估、修订并最终汇总。

第一阶段不增加由模型决定的第三个 `auto` strategy。自动选择可以在未来成为 policy，但实际运行时仍必须落到上述两个明确值之一。

### 8.2 Selection precedence

推荐优先级：

```text
request override
-> interface/session override
-> project config default
-> compatibility default
```

兼容迁移阶段默认使用 `plan_react`，以避免 CLI/API 当前固定 planning 的行为突然变化；同时允许用户显式选择 `react`。待后续 evaluation 有足够证据后，再讨论是否改变默认值。

### 8.3 Selection record

每次 run 必须记录：

- selected strategy；
- selection source，例如 `request`、`session`、`config`、`compatibility_default`；
- 是否由其他 strategy 降级而来；
- 对应的 execution policy/version。

这些信息进入首个 lifecycle event、task state、report 和 runtime identity。

### 8.4 Planner failure degradation

当 `plan_react` 的 Planner 无法生成有效计划时，推荐默认策略是：

1. 记录 Planner failure；
2. 发出 `execution_degraded`，明确 `plan_react -> react`；
3. 在剩余全局预算内从原始 goal 启动 run-level ReAct；
4. final report 保留降级原因。

如果请求或配置明确要求“规划失败即失败”，则直接终止。禁止生成与目标无关的通用默认计划来掩盖失败。

## 9. PlannerContext and Capability Snapshot

### 9.1 PlannerContext

Planner 不应只接收 goal/history。概念上的输入至少包含：

```text
PlannerContext
  goal
  user_constraints
  runtime_constraints
  workspace_context
  capability_snapshot
  procedural_context
  relevant_memory
  prior_step_records        # replan 时存在
  active_plan_revision      # replan 时存在
  remaining_budget
  runtime_identity_summary
```

其中：

- `user_constraints`：用户明确的范围、禁止项、输出要求和成功标准；
- `runtime_constraints`：workspace、approval、安全策略、可用时间和预算；
- `procedural_context`：与“如何完成任务”相关的 runbook、skill 或经验检索结果；
- `relevant_memory`：与用户/项目相关、经过 token budget 筛选的 session/durable memory；
- `prior_step_records`：只提供 bounded summaries 和 evidence references，不复制全部 trace。

### 9.2 CapabilitySnapshot

capability snapshot 在 run 开始或显式 capability refresh 时由 runtime 构建，Planner、StepRunner 和 Evaluator 共享同一版本。至少包含：

```text
CapabilitySnapshot
  snapshot_id
  tool_signature
  captured_at
  tools[]
    name
    description
    input_schema
    source                 # builtin / mcp / future extension
    availability
    mutation_class
    approval_requirement
    transport_metadata     # sanitized
```

规则：

- snapshot 来自实际 `ToolRegistry`，不能由 prompt 手写另一份列表；
- 不向模型暴露 secret、credential、内部 command line 或不必要 transport detail；
- Planner 只能参考 capability，不能调用工具；
- plan 中的 `candidate_tools` 只是提示，不是 binding 或预授权；
- StepRunner 每次真正调用前仍由 registry 验证名称、schema、workspace 和 approval；
- capability 发生变化时生成新 snapshot，并触发 runtime identity/plan viability 检查，而不是静默漂移。

### 9.3 Procedural knowledge before planning

本文借鉴 OnCall 的“先检索经验、再制定计划”，但检索由 `PlannerContextBuilder` 在 Planner 调用前完成，而不是让 Planner 自由调用 retrieval tool。

procedural context 应满足：

- 有明确 source/reference；
- 受 token budget 和 relevance 限制；
- 标记时效、适用范围和置信度；
- 被视为建议，不覆盖用户约束和安全策略；
- 检索失败时返回空 context 加 warning，不伪造经验，也不必然阻断执行。

完整的 Agent definition、procedure selection 和知识格式由 [`2026-07-14-agent-definition-and-procedural-knowledge-design.md`](2026-07-14-agent-definition-and-procedural-knowledge-design.md) 定义。

### 9.4 Planner output

Planner 输出必须是结构化 `PlanDraft`，而不是从任意 prose 中尽量截取 JSON。概念字段建议为：

```text
PlanDraft
  goal
  assumptions[]
  steps[]
    step_id
    objective
    completion_hint
    expected_evidence[]
    candidate_tools[]
    depends_on[]
```

校验规则：

- steps 非空且不超过 `max_plan_steps`；
- step IDs 在该 revision 内唯一且稳定；
- 依赖不存在环和 dangling reference；
- candidate tools 必须能在 snapshot 中解析，否则降为 warning 或要求修复；
- plan 不得声称某个写操作已获批准；
- completion hint 只帮助 StepRunner，不作为未经验证的成功证明；
- invalid output 可以在预算内做一次结构化 repair；仍失败则按 Planner failure policy 处理。

## 10. StepRunner: 每个计划步骤内部运行 bounded ReAct

### 10.1 Step input

StepRunner 的上下文应聚焦当前步骤，但不能像完全孤立任务一样丢失原始目标。输入至少包括：

```text
StepContext
  original_goal
  current_plan_revision
  current_step
  completed_step_summaries
  relevant_evidence_refs
  capability_snapshot
  user_and_runtime_constraints
  step_budget
  compact_summary / relevant memory
```

原始用户 history 不应无限复制到每个步骤。ContextBuilder 根据当前步骤选择必要 history、ledger summaries、memory 和 evidence references。

### 10.2 Bounded ReAct semantics

每个步骤的目标循环为：

```text
build step context
-> model turn
-> action
   -> tool call/batch
      -> existing tool safety + approval + execution
      -> append assistant call and tool result to step history
      -> return to model turn within the same step
   -> step conclusion
      -> validate and create StepRecord
   -> malformed/recoverable error
      -> bounded repair or retry within the same step
```

关键不变量：

1. 一次成功 tool call 不得直接完成步骤。
2. 工具结果必须回到模型，除非该步骤存在显式 deterministic completion rule。
3. 模型可以根据结果调用第二个工具、补充参数、选择只读替代方案或形成结论。
4. `Action::Final` 在 StepRunner 中表示 **step conclusion**，不是整个 run 的用户最终答案。
5. StepRunner 只能在自己的 step budget 和全局剩余预算内循环。
6. 所有工具仍通过现有 `run_tool_turn`、approval、batch ordering 和 history writeback。
7. destructive call 被拒绝后，不允许通过改名、shell 包装或等价工具绕过同一安全决定。

### 10.3 Step terminal outcomes

StepRunner 只产生以下终态之一：

```text
succeeded
failed
skipped
cancelled
budget_exhausted
interrupted
```

- `succeeded`：已形成步骤结论，且满足 completion rule 或给出足够 evidence；
- `failed`：明确错误、证据不足或重试耗尽；
- `skipped`：Evaluator 判定步骤不再需要，不由 StepRunner 自行假定；
- `cancelled`：用户或系统取消；
- `budget_exhausted`：步骤或全局某一预算耗尽；
- `interrupted`：进程/worker 中断，无法确认中间动作状态。

这些状态不能通过一个 `done: bool` 完整表达。

### 10.4 Scoped history and evidence

StepRunner 可以维护 step-local scratch history，但必须保持以下分层：

- raw model/tool events：append-only trace；
- tool output 和 mutation metadata：现有 tool event/artifact；
- step conclusion：bounded `StepRecord.summary`；
- large output：artifact/reference，不复制到每次 Planner/Evaluator prompt；
- global prompt：只注入完成步骤的必要 summary 与 evidence references。

这样既能让步骤内部充分 ReAct，也避免计划越长、全局上下文越快膨胀。

## 11. StepRecord: append-only execution ledger

### 11.1 Why a ledger

`TaskPlan.steps[*].done` 只表达计划游标，不能回答：

- 步骤使用了哪些工具和模型轮次；
- 失败后是否重试；
- 哪些文件或外部资源发生变化；
- 结论依据是什么；
- 它属于哪个 plan revision；
- 新计划替换剩余步骤后，旧结果为什么仍可信。

因此需要独立于 plan 的 append-only step ledger。

### 11.2 Conceptual StepRecord

```text
StepRecord
  record_id
  plan_id
  plan_revision_id
  step_id
  attempt
  status
  started_at
  finished_at
  summary
  completion_basis
  evidence_refs[]
  tool_call_ids[]
  artifact_refs[]
  mutations[]
  model_turns_used
  tool_calls_used
  token_usage
  error_code
  safe_error_summary
  supersedes_record_id?       # only for explicit retry relationship
```

规则：

- 每个 terminal step attempt 追加一个 record；
- record 一旦写入，不因 replan 被删除或覆盖；
- retry 创建新 record，并通过 attempt/supersedes 关联旧失败；
- summary 不得伪造 tool output 中不存在的数据；
- evidence ref 必须能解析到 trace event、tool call、artifact 或受支持的外部资源；
- mutation 使用 rove 现有 `ToolMutation`/write-set 语义；
- error 只保存安全摘要，不泄漏 secret 或隐藏 reasoning。

### 11.3 Canonical storage

第一阶段不需要为了 ledger 新增另一套独立数据库事实源：

- `step_result` event 写入 `trace.jsonl`，作为 append-only canonical transition；
- `task_state.json` 保存 ledger 的 materialized projection，便于 resume；
- `PromptCheckpoint` 保存恢复所需的 bounded ledger state 或 pointer；
- `report.json` 汇总最终 step records、预算和 evidence coverage；
- SQLite 继续作为可重建 index，而不是覆盖 file artifacts 的事实来源。

如果未来 ledger 体积证明需要独立 `steps.jsonl`，应通过单独存储设计决定，不能在实现中临时产生第四个互相冲突的事实源。

## 12. PlanRevision: 只替换未来，不改写过去

### 12.1 Revision model

概念上的 plan revision 至少包含：

```text
PlanRevision
  plan_id
  revision_id
  parent_revision_id?
  created_at
  trigger_step_record_id?
  decision_id
  safe_reason_codes[]
  retained_step_ids[]
  superseded_remaining_step_ids[]
  remaining_steps[]
  capability_snapshot_id
  budget_snapshot
```

初始计划是 revision 0。每次 `replace_remaining` 创建新 revision，并指向 parent。

### 12.2 Revision invariants

1. completed/failed step records 永远保留在 ledger 中；
2. revision 只能替换尚未执行的 remaining work；
3. 已产生 mutation 的步骤不能通过 replan 变成“从未发生”；
4. 新步骤使用新 ID，或显式声明保留旧 step ID；
5. 被替换的未执行步骤标记为 superseded，而不是 completed；
6. revision chain 必须可从当前 plan 追溯到初始计划；
7. 每次 revision 消耗 `max_plan_revisions` 与对应 model/token budget；
8. revision 后的计划仍需通过 capability、依赖和 step-count validation。

这比当前用新的 `TaskPlan` 整体覆盖 active plan 更适合审计、resume 和用户解释。

## 13. PlanEvaluator and Replanner

### 13.1 Separation of concerns

`PlanEvaluator` 决定执行生命周期下一步是什么；`Replanner` 只在 decision 是 `replace_remaining` 时生成新的剩余步骤。两者不应与 Finalizer 合并成一个含糊的 prompt。

Evaluator 输出三态：

```text
PlanDecision
  - continue
  - replace_remaining
  - finish
```

其中 `finish` 表示停止执行并进入 Finalizer，不等于“所有步骤均成功”。它可以携带 `completed`、`partial`、`blocked`、`budget_exhausted` 或 `failed` 等 finish reason。

### 13.2 Rule-first decisions

以下情况由 deterministic rules 直接决定：

| Condition | Decision |
|---|---|
| 用户取消 | `finish(cancelled)`，不调用模型 |
| fatal safety/runtime error | `finish(failed_or_blocked)` |
| 所有必要步骤已有 succeeded/skipped record | `finish(completed)` |
| 全局预算不足以安全开始下一步 | `finish(budget_exhausted)` |
| 成功步骤未改变假设，下一步 capability 可用且依赖满足 | `continue` |
| plan revision budget 已耗尽 | 禁止 `replace_remaining`，选择可执行的 `continue` 或 `finish(partial)` |
| permission denial 且没有用户授权或安全替代方案 | `finish(blocked_or_partial)` |
| runtime identity/capability mismatch 使 plan 不再可信 | `replace_remaining` 或 `finish`，不得静默继续 |

以下语义问题才允许调用 model evaluator：

- 新证据是否已经使剩余步骤不再必要；
- 成功结果是否推翻原计划假设；
- recoverable failure 是否有符合安全约束的替代路径；
- 用户目标是否已被部分结果充分满足；
- 多个剩余步骤的依赖或顺序是否需要调整。

### 13.3 Model evaluator input/output

模型只接收：原始 goal、约束、当前 revision、bounded step ledger、capability snapshot summary 和剩余预算。它不接收隐藏 reasoning，也不调用工具。

输出必须结构化，并至少包含：

```text
decision
safe_reason_codes[]
safe_summary
remaining_work_requirements[]   # replace_remaining 时
finish_reason?                  # finish 时
```

`safe_summary` 是可展示的决策摘要，不是 chain-of-thought。

### 13.4 Anti-thrashing rules

- 每个 terminal step record 最多触发一次 plan decision；
- 相同 evidence/capability/remaining plan 不重复调用 model evaluator；
- revision 必须改变 remaining work，空变化不计为有效 replan；
- 连续 revision 受 `max_plan_revisions` 限制；
- Replanner 不能通过每次替换都新增大量步骤绕过 plan-step budget；
- evaluator/replanner failure 时优先保持可验证的当前剩余计划，只有它仍安全且预算足够时才能继续。

## 14. Finalizer: 从 ledger 与 evidence 生成用户答案

### 14.1 Why separate finalization

计划步骤的 conclusion 面向后续执行；用户最终回答面向原始目标。二者的粒度和责任不同。

Finalizer 的输入至少包括：

```text
FinalizationContext
  original_goal
  user_output_requirements
  execution_strategy
  finish_reason
  plan_revision_chain_summary
  step_records
  evidence_refs
  mutations / write_set
  failures / blocked_items
  budget_usage
```

### 14.2 Finalizer rules

- `plan_react` 在正常、部分完成、预算耗尽和可报告失败路径上都进入 Finalizer；
- Finalizer 第一阶段不调用工具，只基于已记录事实；
- final answer 必须区分事实、推断、未完成项和失败项；
- mutation/write-set 应在任务需要时被明确汇报；
- 不得把 failed/skipped step 描述为成功；
- 不得因为 plan 最终为空就声称目标已完成；
- evidence 不足时给出 partial/blocked answer，而不是补全想象数据；
- finalizer model turn 也受全局 model/token/time/cost budget 约束，但应预留 reserved finalization budget。

### 14.3 Deterministic fallback

如果模型 Finalizer 失败，runtime 仍应从 ledger 生成确定性后备回答，至少包含：

- 原始目标；
- finish reason；
- 已成功步骤及其 summary；
- 失败/被阻塞/未执行步骤；
- 关键 evidence/artifact references；
- 已发生的 workspace mutations；
- 为什么无法生成更完整的综合回答。

fallback 可以朴素，但不得丢失已获得的事实，也不得伪造完整成功。

### 14.4 React strategy and finalization

`react` strategy 中，run-level ReAct 的 `Action::Final` 本身就是用户答案，不强制再增加一次 Finalizer model call。它仍应进入统一的 completion/report boundary。

如果 `react` 因预算、取消或错误结束且已有可报告证据，可以复用 deterministic fallback formatter；不为了“看起来统一”无条件增加模型成本。

## 15. Multidimensional Execution Budgets

### 15.1 Why `max_steps` is insufficient

当前 `max_steps` 在两条 loop 中不是同一个单位：unplanned loop 每轮递增，planned loop 每个 plan step 递增。引入 bounded step loop 后，单个 plan step 还会包含多个 model turns 和 tool calls，因此继续使用一个数字会导致配置、事件和报告无法解释。

### 15.2 Target budget dimensions

| Budget | Meaning | Accounting rule |
|---|---|---|
| `max_plan_steps` | 任一有效 plan revision 可包含的最大步骤数 | 初始计划和每次 revision 校验 |
| `max_step_attempts` | 整个 run 最多执行的 step attempts | succeeded/failed/budget-exhausted 等 terminal attempt 均计数 |
| `max_model_turns` | 整个 run 的模型调用总数 | Planner、step turns、model evaluator、Replanner repair、Finalizer 全部计数 |
| `max_model_turns_per_step` | 单个 step attempt 的模型轮次 | 防止步骤内部无限 ReAct |
| `max_tool_calls` | 整个 run 实际发起的 tool calls | batch 按 call 数量而不是 batch 数量计数 |
| `max_tool_calls_per_step` | 单个 step attempt 的 tool calls | 防止一个步骤吞掉全部预算 |
| `max_plan_revisions` | 初始 revision 之后允许的新 revision 数 | invalid/empty revision 不得通过重试无限绕过 |
| `max_wall_time` | run 的 active wall-clock budget | 默认暂停显式等待用户 input/approval 的时间，并记录 waiting duration |
| `max_total_tokens` | provider 报告或 runtime 估算的总 token budget | 所有模型角色统一累计，给 finalization 预留 reserve |
| `max_cost` | 有价格元数据时的可选 cost ceiling | 无可靠价格时只做 telemetry，不能宣称已强制成本上限 |

### 15.3 Budget invariants

- 检查全局预算后才能开始 Planner、StepRunner turn、tool call、Replanner 或 Finalizer；
- per-step budget 不能超过全局 remaining budget；
- model retry、JSON repair 和 fallback model call 都必须计数；
- tool batch 在执行前按所有 calls 预占预算，不能执行一半后才发现超限；
- approval/input 等待默认暂停 active wall time，但不暂停用户取消和显式 absolute deadline；
- 为 Finalizer 预留预算；如果用户明确要求“不做额外总结”，可由 policy 调整；
- provider 没有返回 usage 时使用保守估算并标记 `estimated`；
- 任一预算耗尽都产生结构化 dimension，而不是全部折叠成模糊 `StepLimit`。

### 15.4 Compatibility migration

迁移阶段可以接受旧 `runtime.max_steps`，但必须：

- 明确它映射到哪个新 budget；
- 在 config dump 中显示 resolved multidimensional budgets；
- 对歧义映射给出 deprecation warning；
- checkpoint/runtime identity 保存 resolved budgets，而不只保存旧字段；
- CLI/API 使用同一解析逻辑。

不建议让 `max_steps` 同时映射 plan steps、model turns 和 tool calls。

## 16. Events and Observability

### 16.1 Target lifecycle events

在保留现有 model/tool/prompt events 的基础上，目标事件至少包括：

| Event | Purpose |
|---|---|
| `execution_strategy_selected` | 记录 strategy、selection source 和 policy version |
| `execution_degraded` | 记录显式 strategy/finalizer/replanner 降级，不静默 fallback |
| `plan_created` | 只表示初始 revision，不再兼任 replan event |
| `plan_step_started` | 带 plan/revision/step/attempt ID 和开始时预算 |
| `step_result` | append-only terminal StepRecord |
| `plan_decision` | `continue` / `replace_remaining` / `finish`，带 safe reason codes |
| `plan_revised` | 新 revision、parent 和 superseded remaining work |
| `finalization_started` | 记录 finish reason 与 finalizer mode |
| `finalization_completed` | 记录 model/deterministic mode、evidence coverage 与使用预算 |
| `run_completed` | 保持整个 run 唯一的终止事件 |

现有 `plan_step_completed` / `plan_step_failed` 可以在兼容窗口内作为 `step_result` 的 derived events，但新的 canonical lifecycle 不应要求消费者把两个事件拼成完整 StepRecord。

### 16.2 Event ordering invariants

`plan_react` 的正常顺序为：

```text
run_started
-> execution_strategy_selected
-> plan_created
-> plan_step_started
-> prompt/model/tool events (one or more turns)
-> step_result
-> plan_decision
   -> continue -> next plan_step_started
   -> replace_remaining -> plan_revised -> next plan_step_started
   -> finish -> finalization_started -> finalization_completed
-> run_completed
```

要求：

- `step_result` 必须在对应 `plan_decision` 之前持久化；
- `plan_revised` 必须引用产生它的 decision 和 parent revision；
- 每个 started step attempt 最终有且只有一个 terminal `step_result`，进程崩溃时由恢复流程补成 `interrupted`；
- 每个 run 最终有且只有一个 `run_completed`；
- UI 可丢弃增量渲染，但 trace 不得丢弃 canonical lifecycle events。

### 16.3 Safe observability

事件与 report 可以公开：

- strategy、step/revision IDs；
- structured status 和 reason codes；
- tool names、call IDs、artifact refs、mutation metadata；
- model/tool/token/time/cost usage；
- safe plan/decision summaries。

不得公开：

- provider hidden thinking；
- chain-of-thought；
- secret-bearing raw args/output；
- Planner/Evaluator 的内部长推理文本。

### 16.4 Metrics

建议从事件投影以下指标：

- strategy 分布与显式 override 比例；
- Planner 成功率、repair rate、fallback-to-react rate；
- 每步 model turns/tool calls 分布；
- tool-result 后再次调用模型的覆盖率；
- plan revision rate、原因与 revision depth；
- step success/failure/blocked/budget-exhausted 比例；
- finalizer model/fallback 比例；
- 各 budget dimension 的耗尽率；
- resume 后重复工具调用与 destructive replay 数量，目标必须为零。

## 17. Persistence, Checkpoint, and Resume

### 17.1 TaskState target additions

概念上，`TaskState` 需要能够恢复：

```text
execution_strategy
strategy_selection_source
capability_snapshot_id / tool_signature
active_plan_id
active_plan_revision_id
plan_revisions or revision pointers
step_records or ledger pointer
active_step_attempt?
budget_limits
budget_consumed
finalization_state
degradation_records
```

实际字段拆分应服从 schema size 和 artifact design，但上述语义不能只存在内存。

### 17.2 PromptCheckpoint target additions

checkpoint 至少需要：

- selected strategy；
- active plan revision/cursor；
- bounded ledger summaries 或 ledger high-water mark；
- active step/attempt identity；
- remaining multidimensional budgets；
- capability snapshot/tool signature；
- last canonical lifecycle event sequence；
- finalization state（如果已经开始）；
- 现有 summary、preserved tail、memory pointers 和 compaction state。

checkpoint 不需要内嵌全部 raw tool outputs；它应通过 trace/artifact reference 恢复。

### 17.3 RuntimeIdentity target additions

当前 runtime identity 已保存 model、provider target、approval、`max_steps`、`plan_enabled`、prompt hashes、workspace fingerprint 和 tool signature。目标应演进为：

- execution strategy；
- execution policy version；
- resolved budget policy/hash；
- Planner/Evaluator/Replanner/Finalizer prompt or policy hashes；
- capability snapshot/tool signature；
- 保留现有 model/provider/workspace/approval/system prompt identity。

不同 mismatch 的处理应分级：

- safety/workspace/tool capability mismatch：不得静默继续旧计划；
- model/prompt/policy mismatch：允许用户确认后 re-evaluate 或 restart，不假装完全等价；
- telemetry-only metadata mismatch：可继续但记录 warning。

### 17.4 Stable resume boundaries

本文不改变现有 restart semantics：进程或 worker 丢失后，active run 先标记为 `interrupted`，不得在服务启动时隐式继续执行。以下恢复流程只在用户或上层 runtime 明确发起 resume 后运行。

首选稳定恢复点：

1. 初始 plan 已持久化、尚未开始步骤；
2. terminal `step_result` 已持久化、尚未产生 decision；
3. `plan_decision` / `plan_revised` 已持久化、尚未开始下一步；
4. finalization 尚未开始或已完成。

恢复顺序：

1. 读取 checkpoint 与 ledger high-water mark；
2. 对齐 trace 中 checkpoint 之后的 canonical events；
3. 校验 runtime identity、capability signature 和 resolved budgets；
4. 如果处于稳定边界，从未完成的 deterministic transition 继续；
5. 如果处于 step 中间，重建已知 model/tool events并判断终态；
6. 对状态未知的 in-flight destructive tool，禁止自动重放；
7. 生成 `interrupted` StepRecord，由 Evaluator/用户决定替代路径或结束；
8. 继续时使用剩余预算，不重置为初始预算。

### 17.5 Backward compatibility

旧 checkpoint 只有 `plan` 和 `plan_enabled` 时：

- 根据兼容规则解析 strategy；
- 将现有 plan 包装为 revision 0；
- 已完成 `done` steps 可以生成 migration-derived ledger summaries，但必须标记 `evidence_incomplete`；
- 不伪造旧版本不存在的 tool/evidence relationships；
- schema migration 后仍保留原始 artifacts 可读性。

## 18. Failure and Degradation Semantics

| Failure | Target behavior |
|---|---|
| Procedural retrieval failure | 用空 procedural context 继续，记录 warning；不伪造 runbook |
| Capability snapshot failure | 若 registry 本身不可用则 fail；若个别可选 MCP source 不可用则生成 degraded snapshot 并校验 plan |
| Planner model error | 按 policy 显式 `fallback_to_react` 或 fail |
| Invalid/empty plan | 预算内一次 structured repair；仍失败则执行 Planner failure policy |
| Step model error | 在 per-step retry policy 内重试；耗尽后写 failed StepRecord |
| Malformed action | 计入 model turn，bounded repair；不得无限重试 |
| Tool failure | 工具错误写入 step history，允许模型在同一步选择安全替代；最终状态由 StepRunner/Evaluator 决定 |
| Permission denied / approval rejected | 不记作成功，不绕过；写 blocked/failed record并评估 partial finish 或等待新授权 |
| Tool output too large | 保存 artifact/reference，向模型提供 bounded representation，不静默截断关键结构 |
| Evaluator model failure | 如果 deterministic next step 仍安全则 continue，否则 finish partial；记录 degradation |
| Replanner invalid output | 保留当前 revision；只有仍安全且预算足够才继续，否则 finish partial |
| Revision limit | 禁止继续 replan；继续可验证的剩余步骤或 finish partial |
| Step/global budget exhaustion | 写 structured dimension，保留 ledger，进入 Finalizer 或对应终止路径 |
| Finalizer model failure | 生成 deterministic ledger-based fallback |
| Cancellation | 尽快停止新 model/tool work，写稳定 checkpoint 和 cancelled outcome |
| Process interruption mid-step | 不盲目重放未知 destructive call；补 interrupted record |
| Runtime identity mismatch on resume | 按 mismatch class re-evaluate、要求确认或拒绝 resume；不静默沿用旧计划 |

任何 fallback 都必须满足：

- 事件可见；
- report 可见；
- 不增加安全权限；
- 不清空已获得 evidence；
- 不把 partial/blocked/error 改写成 complete；
- 不重置已消费预算。

## 19. Configuration and Interface Surface

### 19.1 Conceptual config shape

字段命名可在实现计划中进一步校准，但语义应接近：

```toml
[runtime.execution]
strategy = "plan_react"
planner_failure_policy = "fallback_to_react"
evaluator_mode = "rule_first"

[runtime.execution.budget]
max_plan_steps = 8
max_step_attempts = 12
max_model_turns = 32
max_model_turns_per_step = 4
max_tool_calls = 32
max_tool_calls_per_step = 6
max_plan_revisions = 2
max_wall_time_ms = 600000
max_total_tokens = 0        # 0 only if documented as unlimited/disabled
max_cost = 0                # enforcement requires price metadata
```

本文不决定最终默认数值。默认值必须通过 fake-model scenarios、真实 provider smoke 和 evaluation plan 得出，不能照搬 OnCall prompt 中的 3/5/8 heuristics。

### 19.2 CLI/API

目标接口应允许：

- CLI request 显式选择 `react` / `plan_react`；
- API create-job request 使用同一枚举；
- session/project config 设置默认 strategy；
- job state 返回 resolved strategy、active revision、step ledger summary 和 remaining budget；
- config dump 展示 resolved execution policy，继续执行 secret redaction；
- 未知 strategy 返回 validation error，不默默回退。

### 19.3 Web and terminal rendering

消费者应基于 events 展示：

- 当前 strategy；
- plan revision，而不是把每次 replan 当成全新无关计划；
- step attempt 和 terminal result；
- partial/blocked/budget exhausted；
- finalization phase；
- degradation notice。

UI 不负责推导 canonical state。它只渲染 core events/state projection。

## 20. Testing Strategy

### 20.1 Unit tests

- strategy precedence 与旧 `plan_enabled` compatibility mapping；
- PlannerContext redaction、capability signature 和 token bounding；
- structured plan validation、repair limit、dependency cycle；
- StepRecord append-only、attempt/supersedes relationship；
- revision 只替换 remaining steps；
- rule-first evaluator decision table；
- revision anti-thrashing；
- 每个 budget dimension 的 accounting；
- deterministic finalizer fallback；
- runtime identity mismatch classification；
- old checkpoint migration 不伪造 evidence。

### 20.2 Deterministic fake-model scenarios

至少覆盖：

1. 一个步骤需要 `tool -> model -> second tool -> model conclusion`；
2. tool success 后模型判断证据不足，不得提前完成步骤；
3. 多步骤全部成功后 Finalizer 汇总所有 records；
4. 成功结果使剩余计划不再需要，Evaluator 提前 `finish`；
5. 成功结果推翻假设，Evaluator `replace_remaining`；
6. tool failure 后在同一步选择安全替代工具；
7. approval rejection 不被绕过，并生成 partial/blocked final answer；
8. Planner error 显式降级到 `react`；
9. Planner invalid output repair 失败后不生成伪默认计划；
10. replan 保留 completed records 与 mutations；
11. revision limit 阻止 replan loop；
12. model/tool/token/time budget 分别耗尽；
13. Finalizer model error 生成 deterministic fallback；
14. stable-boundary resume 不重复已完成步骤；
15. mid-step resume 不重复未知 destructive tool call；
16. capability signature 变化触发 re-evaluation；
17. cancellation 在 Planner、tool wait、Evaluator、Finalizer 各阶段都只产生一个 `run_completed`。

### 20.3 Contract and persistence tests

- Rust `StreamEvent` 与 API SSE/Web/terminal exhaustive mapping；
- lifecycle event ordering invariants；
- trace -> task-state projection 重建；
- checkpoint round-trip 与 schema migration；
- SQLite repair 从 artifacts 重建 revision/step index；
- report 中 strategy、budget、records、evidence 和 degradation 完整；
- compaction 后 ledger high-water mark 和 evidence references 仍可解析。

### 20.4 Evaluation dimensions

完整 OnCall reference evaluation 由后续 spec 定义，但本设计实现后至少要度量：

- task completion correctness；
- tool-result grounding；
- unnecessary tool calls；
- unnecessary plan revisions；
- planner/finalizer latency 与 token overhead；
- partial failure honesty；
- resume duplication；
- safety boundary violations；
- short task 在 `react` 与 `plan_react` 下的成本差；
- long task 相比当前 one-turn-per-step loop 的成功率变化。

## 21. Risks and Trade-offs

### 21.1 Cost and latency

步骤内回到模型、Evaluator 和 Finalizer 都可能增加模型调用。通过 bounded step turns、rule-first evaluator、reserved finalization budget 和显式 `react` strategy 控制。

### 21.2 Context growth

append-only ledger 若直接复制 raw outputs 会快速膨胀。通过 bounded summaries、artifact references、high-water marks 和现有 compaction 管理。

### 21.3 Plan rigidity

过度规划会降低探索效率。短任务允许 `react`，plan steps 表达 objective 而不是过细命令，Evaluator 可以提前结束或替换 remaining work。

### 21.4 Replan thrashing

模型可能反复改计划。通过 rule-first、revision cap、no-op rejection、相同输入去重和 total budget 限制。

### 21.5 Resume complexity

bounded step loop 引入更细的中间状态。通过先支持稳定边界、canonical events 和禁止未知 destructive replay 控制，而不是一开始承诺任意指令级恢复。

### 21.6 Capability staleness

MCP 工具可能在运行中变化。通过 snapshot ID、tool signature、显式 refresh 和 viability check，避免 Planner、Executor 各自看到不同世界。

### 21.7 Event/schema churn

新增 canonical events 和 checkpoint fields 会影响 API/Web/state migration。应采用 additive event rollout、schema versioning 和兼容 derived events，不能一次删除旧消费者依赖。

### 21.8 Procedural knowledge quality

错误 runbook 可能把计划带偏。procedural context 必须带来源、时效和置信度，并始终低于用户约束、安全 policy 和实时 evidence。

## 22. Acceptance Criteria

当且仅当以下条件都有代码、测试和 runtime 文档证据时，本文目标才可标记为 implemented：

1. runtime 使用明确的 `react` / `plan_react` strategy，并记录 selection source。
2. CLI 与 API 不再只能隐式固定 planning，且共享同一 strategy resolution。
3. Planner 收到本次运行真实、脱敏、带 signature 的 capability snapshot。
4. PlannerContext 有受限的 procedural knowledge、memory、constraints 和 remaining budget slots。
5. Planner 失败会显式降级或失败，不返回无关的通用默认计划。
6. 一个 plan step 可以执行多个 model/tool turns，并有 per-step 上限。
7. tool success 不会在模型读取结果前自动完成步骤。
8. 每个 terminal step attempt 产生 append-only `StepRecord`。
9. completed records、evidence 和 mutations 不会被 replan 覆盖。
10. plan revision 只替换 remaining work，并保留 parent chain。
11. Evaluator 支持 `continue`、`replace_remaining`、`finish`，且 deterministic condition 不调用模型。
12. `plan_react` 使用独立 Finalizer 汇总 ledger；Finalizer 失败有 deterministic fallback。
13. plan steps、step attempts、model turns、tool calls、revisions、time、tokens/cost 能分别计量。
14. strategy、step result、plan decision、plan revision 和 finalization 有 canonical events。
15. events 不暴露 hidden reasoning 或 secret-bearing payload。
16. task state/checkpoint 保存 strategy、active revision、ledger high-water mark 和 remaining budgets。
17. runtime identity 能识别 execution policy/capability mismatch。
18. stable-boundary resume 不重复已完成步骤或已知 destructive mutations。
19. unknown in-flight destructive tool 不被自动重放。
20. partial、blocked、budget exhausted、cancelled 和 interrupted 不被报告为 completed。
21. CLI/API/Web 对新事件与状态有一致 contract tests。
22. [`docs/runtime/`](../runtime/README.md) 在实现完成后同步更新，并继续作为当前事实来源。

## 23. Implementation Dependency Order

本文不是实现 checklist，但后续计划应遵守以下依赖顺序：

1. **Semantic foundation**

   定义 strategy、budget、StepRecord、PlanRevision、PlanDecision 和兼容迁移语义。

2. **StepRunner**

   从现有 shared model/tool turns 组合 bounded ReAct step，先不引入 model-based replan。

3. **Ledger and canonical events**

   让 step result 先成为可持久化事实，再让 Evaluator 消费它。

4. **Rule-first Evaluator and revision chain**

   先实现 deterministic rules，再加入 model-on-ambiguity 与 Replanner。

5. **Finalizer and deterministic fallback**

   确保所有 terminal paths 都能诚实交付结果。

6. **Checkpoint, resume, runtime identity, report and repair**

   从稳定边界开始，之后再扩展 mid-step reconstruction。

7. **CLI/API/Web strategy and lifecycle surfaces**

   core contract 稳定后再暴露 request/config/UI 控件。

8. **Procedural knowledge integration and evaluation**

   由独立 Agent-definition/procedural-knowledge spec 和 OnCall evaluation plan 提供输入。

每一阶段都必须同步 deterministic tests、event contract 和必要的 runtime current-state 文档；不能先把未来行为写进 `docs/runtime/`。

## 24. Relationship to Existing and Future Documents

### 24.1 Existing docs

- [`2026-05-24-rove-runtime-hardening-design.md`](../Archive/design/2026-05-24-rove-runtime-hardening-design.md) 是已归档的 local-first hardening 背景；本文细化其中当时尚未展开的 Agent execution lifecycle。
- [`docs/runtime/react-loop.md`](../runtime/react-loop.md) 解释当前已实现 loop；本文描述它的目标演进，不替代当前事实。
- [`docs/runtime/implementation-status.md`](../runtime/implementation-status.md) 继续记录真实实现状态；在代码落地前不应因本文而改成 Met。
- `trace.jsonl` / `task_state.json` / `report.json` 的事实与索引关系沿用现有 runtime hardening decision。

### 24.2 Follow-up specs

以下主题应分别沉淀，不塞进本文：

- [`2026-07-14-agent-definition-and-procedural-knowledge-design.md`](2026-07-14-agent-definition-and-procedural-knowledge-design.md)
- [`2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`](2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)
- [`2026-07-15-oncall-reference-agent-evaluation-plan.md`](2026-07-15-oncall-reference-agent-evaluation-plan.md)
- [根级 `AGENTS.md`](../../AGENTS.md) 与 [维护者 onboarding](../ONBOARDING.md) 已作为当前维护入口落地；runtime 对 workspace `AGENTS.md` 的 typed discovery 仍属于未来实现

## 25. Design Decision

本设计的核心决定是：

> rove 不需要复制 OnCall 的 LangGraph 形态，但应吸收其更成熟的 Plan-Execute-Replan 分工；同时利用 rove 已有的 provider、tool safety、event、artifact、checkpoint 和 runtime identity，把它演进成 explicit strategy、bounded step ReAct、append-only evidence ledger、versioned remaining plan、rule-first evaluation 和 evidence-grounded finalization。

换句话说：

- `react` 保留快速、直接、探索性的执行路径；
- `plan_react` 不再是“每步一次模型或工具调用”的薄包装；
- plan 可以改变，已经发生的事实不能被改写；
- 工具成功不是任务成功，最终回答也不是最后一条工具输出；
- 所有智能决策都必须被预算、安全、持久化和恢复语义包围。
