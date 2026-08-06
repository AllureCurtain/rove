# rove — Agent 项目设计文档

`rove` 的设计文档集合。

**当前阶段：Web Complete C0-C3 与 CDH G1-G7 均已合入 `main`；下一阶段按 Post-CDH Agent Kernel and Coding Capability Plan 推进。当前实现事实以代码、测试和 `docs/runtime/` 为准；项目起步期材料统一归档在 `docs/Archive/`。**

---

## 项目基本信息

- **名字**: rove /roʊv/ (漫游、探索)
- **语言**: Rust
- **形态演进**: CLI → API → Web 管理端 → Desktop（Tauri，后置）
- **位置**: `D:/Study/project/agent/rove/`

---

## 当前状态

| 进度 | 内容 |
|---|---|
| ✅ | 项目命名 (rove) |
| ✅ | 语言选型 (Rust) |
| ✅ | 模块化 Cargo Workspace (`models / core / runtime / apps`) |
| ✅ | 本地优先 MVP：CLI / API / Web / state / resume / tools / memory |
| ✅ | Provider Layer redesign：开放协议注册、named profiles、`/providers/models` |
| ✅ | `docs/runtime/` 与当前实现对齐 |
| ✅ | Web M1、Web Complete C0-C3 与 CDH G1-G7 已合入 `main` |
| 🧭 | **Post-CDH**：统一 Agent kernel、Project Trust、Execution Environment 与 Coding Tool V2；Desktop 后置 |
| 📦 | 起步期设计、handoff、对照材料归档到 `docs/Archive/` |

---

## 文档导航

| # | 文件 | 一句话说明 |
|---|---|---|
| 00 | [README](./00-README.md) | 本文件,总目录 |
| onboarding | [维护者 Onboarding](./ONBOARDING.md) | 当前仓库入口、代码地图、运行方式、验证矩阵和文档事实边界 |
| runtime | [当前 runtime 文档](./runtime/README.md) | 当前实现的权威架构、子系统边界、实现状态对照 |
| archive | [历史文档归档](./Archive/README.md) | 起步期决策、设计讨论、handoff 和实现对照，仅保留历史脉络 |
| design | [模块化 Workspace 架构](./design/2026-07-22-modular-workspace-architecture.md) | Implemented: `models / core / runtime / apps` 四层结构 |
| design | [Provider Layer 重构](./design/2026-07-23-provider-layer-redesign-design.md) | Accepted/implemented provider protocol registry and profiles |
| design | [Cleanup & naming decisions](./design/2026-07-24-cleanup-and-naming-decisions.md) | Implemented: delete legacy, provider vocabulary, tools, W1–W3 |
| design | [Agent Desktop + Web Shared UI](./design/2026-07-25-agent-desktop-web-ui-design.md) | Partially implemented: Web product work landed; Tauri Desktop remains future scope |
| design | [Web Complete](./design/2026-07-26-web-complete-design.md) | Implemented on `main`: C0-C3 persistence, continuity, Settings, migration, polish, and acceptance |
| plan | [Cleanup W1/W2/W3 delivery](./plans/2026-07-24-cleanup-w1-w2-w3.md) | Completed implementation ledger |
| plan | [Web Management M1 delivery](./plans/2026-07-25-web-management-m1.md) | Completed serial waves F0→F1→F2 on main |
| plan | [Web → Desktop master delivery](./plans/2026-07-25-web-desktop-master-delivery.md) | Historical coordinator plan; Web delivery landed and Desktop is deferred |
| plan | [Web Complete delivery](./plans/2026-07-26-web-complete.md) | Completed C0-C3 delivery ledger |
| plan | [CDH G1-G7 delivery](./plans/2026-08-03-cdh-alder-merge.md) | Completed through PR #29; G8 Desktop was out of scope |
| plan | [Post-CDH Agent Kernel and Coding Capability](./plans/2026-08-05-post-cdh-agent-kernel-and-coding-capability.md) | Active M0-M10 plan; two isolation worktrees operated serially by the main Agent thread, with Subagents prohibited |
| future | [Agent Execution Lifecycle](./design/2026-07-14-agent-execution-lifecycle-design.md) | Partially implemented: StepRunner/ledger/revision/decisions landed; Finalizer/budgets remain |
| future | [Agent Definition 与程序性知识](./design/2026-07-14-agent-definition-and-procedural-knowledge-design.md) | Proposed: versioned Agent profile、procedure 与 capability binding |
| future | [MCP Streamable HTTP 与 Tool Artifacts](./design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md) | Proposed: transport/session/result/artifact 演进 |
| future | [OnCall Reference Agent Evaluation](./design/2026-07-15-oncall-reference-agent-evaluation-plan.md) | Proposed: 合成 reference Agent 与 deterministic evaluation |
| design | [Grok Build 借鉴与 TUI 方向](./design/2026-07-16-grok-build-reference-and-tui-design.md) | Bounded TUI MVP implemented; platform verification gap remains |

---

## 建议阅读顺序

**第一次读**: onboarding → runtime → 当前任务相关的 design

**回看时**: 先看 runtime（当前实现），需要了解决策沿革时再进入 Archive

**历史背景**: 早期愿景、产品定位和 agent 思想材料统一位于 [Archive](./Archive/README.md)

---

## 文档约定

- 每个关键决策都附 **理由** 和 **反例** (为什么不选 X)
- 标注 **[WIP]** = 进行中,**[决策]** = 已锁定,**[待定]** = 未决
- 引用代码时给出文件路径
- 中文为主,代码/术语保留英文
- 决策修改请在文末加 changelog,不要静默改

---

## 参考资料

- **pico** (`D:/Study/project/agent/pico/`):前一个 Python coding agent 项目,**继承思想,不复用代码**(语言已切换到 Rust)
- **Claude Code 解析** (`D:/Study/project/claude-code-analysis/analysis/`):工业级 agent 的逆向工程文档,13k+ 行,是本项目的主要参考来源
- **ragent** (`D:/Study/project/agent/ragent/`):SSE 生命周期、模型流取消、首包探测和路由降级的参考实现。历史分析见 [ragent 流式与模型设计借鉴](./Archive/RAGENT-STREAM-MODEL-NOTES-2026-05-24.md)

---

## changelog

- 2026-05-17:初版,完成 M0 之前的纸上设计 (Python 路径)
- 2026-05-17:**重大决策切换 —— 语言从 Python 改为 Rust**,项目命名定为 rove。
- 2026-05-18:新增产品定位与 Workspace 文档（现已归档）。
- 2026-05-22:更新当前状态,代码已推进到 API / Web workbench 阶段。
- 2026-05-24:新增 ragent 流式与模型设计借鉴、`docs/runtime/` 入口。
- 2026-05-25:将 `docs/runtime/` 标为当前权威入口。
- 2026-07-15:新增维护者 onboarding、根级 `AGENTS.md` 与未来设计。
- 2026-07-22:模块化 Workspace 架构落地。
- 2026-07-24:Provider Layer redesign 合入 main；起步期材料迁入 `docs/Archive/`，总目录改为 onboarding / runtime / Archive / 活跃 design。
- 2026-07-25:登记 Agent Desktop + Web 共享 UI 封板设计与 Web M1 worktree 交付计划（串行 F0→F1→F2）。
- 2026-07-25:P0 状态校准：登记 Web → Desktop 主交付计划，将 Web Complete
  调整为契约 foundation + 有界并行 worker，并修正 lifecycle/cleanup/TUI 等状态漂移。
- 2026-07-26:Web M1 已合入 main；新增 Web Complete 封板设计与 C0–C3 串行交付计划，Desktop 继续后置。
- 2026-08-06:Web Complete C0–C3 与 CDH G1–G7 已合入 main；登记 Post-CDH
  Agent Kernel and Coding Capability 主计划，明确仅主线程串行执行、禁止子 Agent。
