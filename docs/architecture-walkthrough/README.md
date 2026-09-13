# Architecture Walkthrough

> 本目录是按模块深入讲解 rove 架构的系列笔记，用于面试准备和系统性复习。
> 每篇对应一个底座子系统：讲清楚「怎么设计、怎么工作、为什么这么取舍」，
> 并给出代码锚点（`文件:行号`）方便核对。内容与 `docs/runtime/` 的当前状态一致。

## 定位：这个项目到底在做什么

rove 不是"一个 Agent"，而是 **Agent 运行时/底座 + Agent 策略 + 产品壳** 三层：

```
┌─ 产品壳层：CLI / API / Web / Desktop / TUI      ← 参考实现，证明底座能跑
├─ Agent 策略层：PlanReact（Planner/Evaluator/StepRunner/Finalizer）、Review 只读模式
│                                                 ← 底座之上具体的 agent 行为（薄层）
└─ 底座层（大部分工作）：
   kernel + host 抽象 / provider 层 / ToolRegistry / Executor /
   context / compaction / memory / state / resume / MCP / 安全模型
```

- 大部分工作是底座层；agent 行为是底座之上的策略层；产品壳是证明底座能支撑真实产品的参考实现。
- 市场对照：纯框架（LangGraph / Rig / OpenAI Agents SDK）有底座无产品；纯产品（Claude Code / Cursor / Pi）有产品无框架层抽象；rove 选择两者都做（OpenHands 也是这个位置）。
- 面试定位：后端/Agent 岗位问的架构问题正是底座问题。**面试讲底座，产品缺口留在 roadmap。**

## 系列目录

| 编号 | 模块 | 状态 |
|---|---|---|
| 01 | Kernel / Host（agent loop 抽象、one kernel many hosts）| ✅ 已记录 |
| 02 | Memory（三层内存、CJK 召回、原子写入）| 待讨论 |
| 03 | Context（replay-safe 历史切分、token 预算、prompt 构建）| 待讨论 |
| 04 | Compaction（确定性截断 vs 模型压缩、安全边界）| 待讨论 |
| 05 | Planning（Planner/Evaluator/StepRunner/Finalizer 四角色）| 待讨论 |
| 06 | State / Resume（trace/state/report 三件套、reconciliation）| 待讨论 |

每篇结构：

1. 一句话总结（可背）
2. 概念与分层
3. 架构动机（为什么这么设计）
4. 核心机制走查（带代码锚点）
5. 设计决策与面试亮点
6. 市场对比（CC / Pi / 其他框架）
7. 扩展模型（封闭协议 vs 开放通道）
8. 面试速答卡（一问一答）
9. 高频追问方向

## 代码锚点约定

- 锚点形如 `core/src/kernel.rs:203`，用 `文件:行号`，方便面试前快速核对。
- 行号会随代码演进漂移，核对时以 `git blame` 或搜索函数名为准。
