# Agent 思想基础

> 这一份不讲技术栈,不讲代码,只讲**怎么理解 agent**。后面所有的架构决策都建立在这上面。

---

## 1. Agent 到底是什么 (纠正最常见的心理模型)

### 错的心理模型

> Agent = 一个能调工具的聊天机器人

这是表象。**这个心理模型会让你做出非常差的 agent**,因为它把 LLM 放在了中心。

### 正确的心理模型

> **Agent = 一个把 LLM 嵌入到[感知 → 决策 → 行动 → 反馈]控制循环里的系统。**

注意主语:**系统**,不是 LLM。LLM 只是这个系统里"决策"那个位置的一个组件。

定义 agent 的真正部分是 LLM **外面**那一圈:

| 部件 | 作用 |
|---|---|
| 循环本身 | 主控制流 |
| 感知层 | 怎么把世界变成 LLM 能看懂的 prompt |
| 行动层 | 怎么把 LLM 的输出变成对世界的操作 |
| 反馈层 | 怎么从世界拿到结果再回到下一轮 |
| 状态层 | 循环之间怎么传承 |
| 边界层 | 危险动作怎么管住 |
| 终止条件 | 什么时候算完 |

**LLM 在这套系统里是可替换的。** 换 GPT / Claude / 本地模型,效果会变,但 agent 还是同一个 agent。但如果循环 / 状态 / 边界设计差,换什么模型都救不回来。

> 这就是 pico README 说"它更像一个能在仓库里持续工作的命令行助手,不是纯聊天窗口"的原因 —— agent 的本质是**持续工作**,不是**一次性回答**。

---

## 2. Agent 的本质抽象

把花哨概念剥光,一个 agent 就是这个递推式:

```
State_t,  Observation_t  →  [LLM]  →  Action_t  →  [Env]  →  State_{t+1},  Observation_{t+1}
```

读法:**在某个状态下,看到某些观察,LLM 决定一个动作,环境执行后产生新状态和新观察。**

这就是强化学习里的 **MDP (Markov Decision Process)**,只不过 policy 是 LLM 而不是训练好的神经网络。

### 四个核心抽象

| 抽象 | 含义 | pico 对应物 |
|---|---|---|
| **State** | agent 此刻关于世界 + 任务的所有内部表征 | `task_state` + `memory` + `session` |
| **Observation** | 从环境拿到的新信息 | 工具返回值 + workspace 快照 |
| **Action** | agent 能对世界做的事 | 工具调用 + `<final>` 输出 |
| **Reward / Termination** | 什么时候算成功、什么时候必须停 | `<final>` 触发 / step limit / retry limit |

**这套抽象的普适性**:LangGraph、AutoGen、CrewAI、eino,不管 API 长什么样,**都能映射到这四个抽象**。各家区别只是"State 怎么组织 / Action 空间怎么定义 / Observation 怎么 normalize"。

---

## 3. 好 agent 和差 agent 的分界线

这是最值钱的部分。直接对照:

| 维度 | 差 agent | 好 agent |
|---|---|---|
| **状态** | 跑完就忘,每次都是新的 | 显式 state,可序列化、可恢复、可审计 |
| **Prompt** | 一坨字符串,什么都往里塞,长度爆了就崩 | 结构化 section (prefix / memory / history / current),有 budget,有裁剪策略 |
| **工具调用** | LLM 输出直接 eval/exec,出错崩 | 统一边界:schema 校验 → 路径/参数检查 → 审批 → 沙箱 → 失败可恢复 |
| **错误** | 一次失败就退出 | retry + reflection + fallback 三层 |
| **可观测性** | 黑盒,跑完只看结果 | 每一步 trace,可 replay,可 diff |
| **终止** | 没有上限,容易死循环或烧 token | step / token / cost / time 多维 budget |
| **评测** | 靠人肉跑感觉 | 固定 benchmark + fake model + 可比指标 |
| **记忆** | 全塞 prompt | 分层:working / episodic / semantic / durable |
| **失败模式** | 不知道为什么失败 | 知道是 model 错、parse 错、tool 错、env 错 |

**pico 已经做对了:状态、prompt 结构、工具边界、可观测性、终止、评测、记忆分层。**

**这就是 pico 的差异化所在**,大部分开源 agent 项目在这张表上都是左列。rove 继续放大这个优势,不要丢。

---

## 4. 设计 agent 时你在回答的五个根本问题

**任何 agent 设计决策,最终都在回答这五个问题之一。** 想清楚它们,你就知道下一步该做什么。

### Q1: 怎么让 LLM 选对动作?
- prompt 工程 (system prompt / few-shot / tool description)
- tool schema 的颗粒度 (粗的工具好懂但弱,细的工具强但乱)
- 失败时:retry?reflect?换策略?

这是最像"调味"的部分,最不可证伪,最吃经验。

### Q2: 怎么管 context?
- 历史装不下:截断?摘要?retrieval?
- 哪些信息必须保留 (关键决策、关键事实)
- 关键事实怎么从历史里提取出来

pico 的 `ContextManager` 就是在解这个。Claude Code 还有 compact 机制 (会话自动压缩)。

### Q3: 怎么让动作安全?
- 工具边界、schema 校验、审批模式
- 路径逃逸、命令注入、SSRF
- 失败回滚、幂等性

pico 的 `run_tool()` 在解这个。Claude Code 把这块做成了显式 pipeline。

### Q4: 怎么让任务真的能完成?
- Reactive (一步看一步) vs Planning (先规划再执行)
- 子任务分解的颗粒度
- 何时承认失败、何时换策略
- 长任务里"忘了原始目标"怎么办

### Q5: 怎么让系统可观测、可比较?
- trace 粒度
- replay 机制
- benchmark 设计 (任务 / verifier / fixture)
- 怎么知道版本变好了不是变差了

### 这五个问题之间是有耦合的

- Q4 (planning) 做得好,Q2 (context) 的压力就小 (不需要在一个 ctx 里塞全部历史)
- Q3 (边界) 做得好,Q1 (prompt) 就可以放心点 (模型胡来你也接得住)
- Q5 (评测) 做得好,Q1 (prompt 调味) 才有客观依据,不然全是体感

**设计时不要五个问题同时往前推,轮流推进。** 这一周专心解 Q2,下一周专心解 Q4,效率比平推高很多。

---

## 5. 当前 agent 设计的几个思想流派

| 流派 | 核心思想 | 代表 | 适合 |
|---|---|---|---|
| **ReAct** | Thought → Action → Observation 反复循环 | pico、aider、smolagents、Claude Code 基底 | 短中长任务都行,工程上最简单。**rove 起点** |
| **Plan-and-Execute** | 先生成完整 plan,再逐步执行 | BabyAGI、LangChain plan-execute | 长任务,但 plan 错了整条线崩 |
| **Reflection / Self-Critique** | 自己批评自己的输出,再修正 | Reflexion、Self-RAG | 提高单步质量,代价 token 翻倍 |
| **Tree Search** | 把动作当搜索空间,MCTS over actions | LATS、Tree of Thoughts | 强但极贵,研究中用,工程价值低 |
| **Multi-Agent** | 多角色协作,分工 | AutoGen、CrewAI、MetaGPT | 复杂任务,但协调成本高,容易"开会开死" |
| **Hybrid** | 上面几种混搭 | Devin、Manus、Claude Code 完整版 | 工业界主流,工程复杂度高 |

### 给新项目的选型路径

- **M1-M2**: ReAct 为底
- **M2 加 Planner**: 演化到 Plan-and-Execute 的轻量版 (plan 不死板,中途可改)
- **M3-M4 不动循环结构**:工具增加,主循环骨架不变
- **可选开关 Reflection**: 单步质量不够时启用,默认关 (因为贵)
- **Tree Search 不要做**:工程价值低
- **M7+ Multi-Agent**:在稳固单 agent 之后再考虑

---

## 6. 从 Claude Code 解析得到的五个工业级启示

[来源:`claude-code-analysis/analysis/`]

### 启示 1:统一执行内核 (`query()` AsyncGenerator)

Claude Code 所有运行形态 (REPL / headless / SDK / subagent / background / remote) **共用一个 `query()` 函数**,它 yield 事件流,UI/SDK/API 都是消费方。

[来源:`05-differentiators-and-comparison.md` § 2]

**这是"core 是 library"原则的工业级证据。新项目第一天就该这样做。**

### 启示 2:Tool 不是函数,是 pipeline

Claude Code 的 tool 执行链路:

```
schema 校验 → validateInput → pre-hook → permission → call() → tool_result
```

[来源:`04b-tool-call-implementation.md`]

pico 有 boundary 但没有显式 pipeline 阶段。**rove 要把这条流水线显式化,每一步可插拔。**

### 启示 3:Memory 是文件系统,不是数据库

Markdown + 目录树。

```
.claude/memdir/
├── MEMORY.md
└── topics/*.md
.claude/sessions/*.md
.claude/agents/*/
.claude/team-memory/
```

[来源:`04-agent-memory.md`]

pico 已经这样了 (`.pico/sessions/*.json` + `.pico/memory/`)。**继续保持。** 文件化带来:可审计 / 可手改 / 可 diff / 可 git。

### 启示 4:Streaming 是一等公民,不是优化

Claude Code 从 day 1 用 `AsyncGenerator<StreamEvent>`,事件流贯穿全栈。**pico 没完全做到,rove 从开始就用,否则 Web 阶段会推倒重写。**

> Rust 等价物:`impl Stream<Item = StreamEvent>`,用 `async_stream` crate 写起来语法和 Python `async def ... yield` 几乎一样。详见 [05](./05-下一步-统一执行内核.md)。

### 启示 5:Hooks 系统

`pre-tool` / `post-sampling` hooks 把扩展点和核心分离。Claude Code 能支持 skills / 插件 / 第三方扩展,根基就在 hooks。**pico 没有这个抽象,rove 应该加。**

---

## 7. 一句话总结

> **Agent 的核心竞争力不在 LLM 有多强,而在 LLM 外面那套系统设计得有多好。**
>
> **LLM 是肌肉,系统是骨架神经。骨架烂,肌肉再强也是抽搐;骨架好,肌肉一般也能干活。**

pico 已经证明你能搭出好骨架。rove 的任务是**把这套骨架再纯粹一点、再可扩展一点**,而不是被 LLM 的能力进化牵着走。

---

## changelog

- 2026-05-17 (v1):初版
- 2026-05-17 (v2):把"新项目"统一替换为 rove,在 AsyncGenerator 处加 Rust 等价说明 (`impl Stream` + `async_stream`),其余内容语言无关,不变
