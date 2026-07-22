# Rove 模块化 Workspace 目标架构 - 2026-07-22

> Status: **Implemented** (modular workspace migration complete; optional full RAG feature suite remains owner-deferred)
>
> 本文记录 Rove 从单 crate 项目演进为可嵌入、可扩展 Agent 平台的目标目录与依赖边界。模块化 Workspace 主体已落地。当前运行时事实仍以代码、测试和 [`docs/runtime/`](../runtime/README.md) 为准。

## 1. 决策摘要

Rove 保留 Rust 自研 Runtime，不直接替换为 `pi-agent-core`，但借鉴 Pi 的分层方式：模型协议、最小 Agent Core、持久化 Runtime 和具体产品壳彼此分离。

目标代码结构采用四个一级目录：

```text
models/
core/
runtime/
apps/
```

目录不重复添加 `rove-` 前缀；Cargo package 名保留 `rove-` 前缀，以便发布、依赖声明和诊断信息保持明确。

| 目录 | Cargo package 示例 | 定位 |
|---|---|---|
| `models/` | `rove-models` | 统一模型协议、Provider 适配与模型流 |
| `core/` | `rove-core` | 最小、可嵌入、尽量无状态的 Agent 循环 |
| `runtime/` | `rove-runtime` | Rove 的持久化任务执行与产品级运行语义 |
| `apps/` | `rove-cli`、`rove-api`、`rove-desktop` 等 | 面向用户和外部系统的第一方应用 |

这四个目录只表示主要代码分层，不意味着仓库根目录只能存在四个文件夹。`docs/`、`scripts/`、`tests/`、`benchmarks/` 和 `prompts/` 等工程目录继续保留。

## 2. 命名决策

### 2.1 目录名不加 `rove-` 前缀

在同一个仓库中，父目录已经提供 Rove 上下文，`rove-models/`、`rove-core/` 会增加视觉噪音。短目录名更适合本地导航：

```text
models/
core/
runtime/
apps/
```

但 Cargo package 位于全局命名空间，仍使用：

```toml
name = "rove-models"
name = "rove-core"
name = "rove-runtime"
name = "rove-cli"
name = "rove-api"
name = "rove-desktop"
```

Rust 代码中的 crate 标识符相应为 `rove_models`、`rove_core` 和 `rove_runtime`。目录名与 package 名不必相同。

### 2.2 使用 `core`，不使用 `agent-core` 或 `cores`

- 不使用 `agent-core`：仓库上下文已经说明它是 Agent 项目，`agent-` 没有增加有效信息。
- 不使用 `cores`：这里定义的是一个稳定核心，而不是多个互不相关的核心实现；Rust crate 和架构层名称通常使用单数。
- 使用 `core`：简短，并准确表达唯一的最小执行内核。

`models` 和 `apps` 使用复数，是因为它们天然容纳多个 Provider 和多个应用；`core` 与 `runtime` 表示单一架构层，因此使用单数。

## 3. 目标仓库结构

```text
rove/
├── Cargo.toml                  # Cargo workspace 清单
├── Cargo.lock
├── README.md
│
├── models/                     # crate: rove-models
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── message.rs
│       ├── events.rs
│       ├── client.rs
│       └── providers/
│           ├── openai.rs
│           ├── openai_responses.rs
│           ├── anthropic.rs
│           ├── ollama.rs
│           └── fake.rs
│
├── core/                       # crate: rove-core
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── agent.rs
│       ├── messages.rs
│       ├── events.rs
│       ├── loop.rs
│       ├── tools.rs
│       └── policy.rs
│
├── runtime/                    # crate: rove-runtime
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── coordinator.rs
│       ├── execution/
│       ├── planning/
│       ├── approval/
│       ├── workspace/
│       ├── state/
│       ├── memory/
│       ├── tools/
│       └── hooks/
│
├── apps/
│   ├── cli/                    # crate: rove-cli；含 REPL/TUI/exec
│   ├── api/                    # crate: rove-api；HTTP/SSE
│   ├── desktop/                # crate/app: rove-desktop；Tauri
│   ├── web/                    # Next.js Web workbench
│   └── bench/                  # 可选：确定性 benchmark runner
│
├── docs/
├── scripts/
├── prompts/
├── benchmarks/
└── tests/
```

目录中的具体文件只是边界示例，不在本文中锁定。迁移时应根据现有模块职责逐步提取，而不是为了匹配目录树一次性重写。

## 4. 四层职责

### 4.1 `models/`

`models` 负责“如何与模型通信”，不负责“Agent 应该怎样完成任务”。

包含：

- Provider-neutral 的 LLM message、tool schema、usage 和 streaming event；
- `ModelClient` 抽象；
- OpenAI、OpenAI Responses、Anthropic、Ollama、Fake Provider 适配；
- Provider 错误归一化、流式帧解析、能力描述和路由所需身份信息。

不包含：

- Agent 主循环；
- Workspace、审批和工具执行；
- Plan、TaskState、Trace、Report；
- CLI、HTTP、Web 或 Tauri 代码。

### 4.2 `core/`

`core` 是可嵌入的最小 Agent Harness。它接收模型、消息和工具，运行 turn/tool loop，并产生类型化事件。

包含：

- Agent state 与 app message 到 LLM message 的转换边界；
- 单轮和多轮 ReAct/tool-call loop；
- `Tool` contract、registry 和 before/after tool call policy hook；
- streaming events、cancellation、steering/follow-up 等运行控制；
- 与持久化无关的预算和终止条件。

不包含：

- SQLite、文件 artifact 和跨进程恢复；
- 默认 Workspace 探测；
- 面向用户的默认审批决定；
- Plan-and-Execute 产品策略；
- 内置文件、Shell、Memory、MCP 工具实现；
- 任何 App 依赖。

`core` 应可以在内存测试中只使用 Fake Model 和一个自定义 Tool 完成执行，不要求文件系统状态目录、HTTP Server 或 UI。

### 4.3 `runtime/`

`runtime` 是 Rove 相对最小 Agent Core 的主要增量，也是 Rove 自己的产品级执行语义。

包含：

- Run、Job、Session 和 Workspace 生命周期；
- `react` / `plan_react` 等 execution strategy；
- Planner、Step Runner、Evaluator、Finalizer 与执行预算；
- approval、request input、workspace boundary 和 destructive ordering；
- Trace、TaskState、Report、checkpoint、SQLite index 和 repair；
- resume、runtime identity 与兼容性检查；
- Memory、RAG、MCP 和官方内置工具；
- runtime hooks、compaction 和模型路由策略。

`runtime` 组合 `models` 与 `core`，但不依赖 CLI、Axum handler、React 或 Tauri。

当前 `Engine` 的职责未来应被拆分为：

- `core::Agent`：最小消息与工具循环；
- `runtime::RunCoordinator`：持久化任务编排、计划、审批、恢复和报告。

### 4.4 `apps/`

`apps` 是第一方产品和协议适配层。所有 App 消费同一个 `rove-runtime`，不得各自实现另一套 Agent loop 或状态语义。

建议包含：

- `apps/cli`：一次性 exec、REPL、TUI 和本地会话入口；
- `apps/api`：Job API、SSE、OpenAPI、鉴权和进程装配；
- `apps/web`：浏览器工作台；
- `apps/desktop`：Tauri 桌面产品；
- `apps/bench`：可选的 deterministic benchmark runner。

Tauri 是 Rove 的第一方产品壳，不承担核心 Agent 编排。桌面端关闭、Web 断开或 CLI 退出时，Runtime 的状态定义不能由界面决定。

## 5. 依赖方向

允许的主依赖方向为：

```text
models  <-  core  <-  runtime  <-  apps
```

箭头表示右侧依赖左侧：

- `core` 可以使用 `models` 中的统一模型协议；
- `runtime` 组合 `core` 并提供持久化执行；
- `apps` 装配并消费 `runtime`；
- 任何底层模块都不能反向依赖 App。

禁止形成以下依赖：

```text
models  -X-> core/runtime/apps
core    -X-> runtime/apps
runtime -X-> apps
```

具体 Provider 由上层装配，但 Provider 实现本身仍位于 `models`。如果未来出现可选的重型 Provider 或工具，可再拆子 crate，不提前增加第五个顶层架构层。

## 6. 与 Pi 和 LiveAgent 的关系

Rove 借鉴 Pi 的是分层和扩展哲学，不是 TypeScript 实现、终端产品形态或具体 API：

```text
Pi:
pi-ai -> pi-agent-core -> pi-coding-agent / SDK consumers

Rove:
models -> core -> runtime -> CLI / API / Web / Desktop
```

LiveAgent 以 `pi-agent-core` 和 `pi-ai` 为 Agent 基础，在其上开发桌面产品、工具、Memory、Skills 和 Gateway。Rove 不改用同一基础，而是保留独立 Rust 技术栈，并让自己的 Tauri Desktop 成为 `rove-runtime` 的第一方消费者。

Rove 不照搬 Pi 的“所有高级能力都不内置”立场。Plan、审批、恢复、MCP 和持久化继续作为官方 Runtime 能力，但必须位于最小 `core` 之外，避免所有嵌入方被迫接受同一种工作流。

## 7. 扩展边界

第一阶段不设计 Rust 动态库插件 ABI。Rust 动态插件会引入 ABI 稳定性、崩溃隔离和供应链问题。

扩展能力按以下层级提供：

1. Rust SDK：编译期实现 `ModelClient`、`Tool` 或 runtime policy；
2. Skills：提示词和程序性知识扩展；
3. MCP：标准外部工具扩展；
4. JSON-RPC/stdio：未来用于进程外 hooks、tools 和更广泛插件；
5. WASM：只有出现明确隔离和跨语言需求后再评估。

目标不是复制 Pi 的 TypeScript extension loader，而是提供符合 Rust 边界的可扩展方式。

## 8. 从当前单 crate 迁移

本次决策只新增文档，不立即移动代码。当前工作树正在进行 Agent execution lifecycle 重构，在该重构收口前进行全仓目录迁移会放大冲突和回归范围。

建议分四步实施：

### Phase 1：先稳定逻辑边界

- 在现有单 crate 内明确 model/core/runtime/interface 的依赖；
- 收紧公共 API，减少 `Engine` 隐式读取 cwd、默认审批和直接持有持久化细节；
- 增加一个外部风格示例：Fake Model + 自定义 Tool + 事件消费；
- 不改变现有 CLI/API/Web 行为。

### Phase 2：提取 `models` 与 `core`

- 根 `Cargo.toml` 改为 workspace；
- 先提取统一模型协议和 Provider；
- 再提取最小 Agent loop、Tool contract 与核心事件；
- 用依赖检查阻止 `core` 引入 App 或持久化依赖。

### Phase 3：形成 `runtime`

- 将 Workspace、Plan、approval、state、memory、resume 和官方工具迁入 `runtime`；
- 把当前大 `Engine` 收敛为 runtime facade/coordinator；
- 保持现有 trace、task state、report 和 SQLite schema 的兼容边界。

### Phase 4：迁移 `apps`

- CLI/TUI、API、Web 只做装配和展示；
- 现有 Web workbench 迁入 `apps/web`；
- 在同一 Runtime API 稳定后新增 `apps/desktop` Tauri 应用；
- 桌面端不复制 Agent loop、Provider 或状态存储。

## 9. 迁移验收条件

完成目标结构至少需要满足：

1. `core` 不依赖 `rusqlite`、`axum`、`clap`、`ratatui`、Next.js 或 Tauri；
2. `runtime` 不依赖任何 App；
3. CLI、API、Web、Desktop 消费相同的 runtime event 与持久化状态；
4. 外部 Rust 程序可以嵌入 `rove-core`，注册自定义模型和工具；
5. 外部 Rust 程序可以选择使用 `rove-runtime` 获得持久化、审批和恢复；
6. 现有行为测试在迁移过程中持续通过，不以目录重构为理由删除能力；
7. 每个 crate 有清晰 README、公共 API 示例和最小依赖说明；
8. 发布包和安装入口仍能提供一个用户可直接使用的 Rove，而不是只剩框架组件。

## 10. Non-Goals

本文不决定：

- 立即重写当前实现；
- 直接依赖或 fork Pi；
- 将 Rust 核心改写为 TypeScript；
- 删除 Plan、MCP、Memory、approval 或 resume；
- 立即实现插件市场；
- Tauri 的具体视觉设计；
- crates.io 发布节奏和稳定性承诺；
- 为目录整洁破坏现有 artifact、API 或 session 兼容性。

## 11. 后续未决项

- `core` 的最小 message model 是否支持应用自定义消息；
- core event 与 durable runtime event 的组合方式；
- JSON-RPC 插件协议是否复用现有 API event contract；
- `apps/web` 与 `apps/desktop` 如何共享前端组件；
- 哪些 RAG/MCP 能力应作为默认 feature，哪些应成为可选 crate；
- workspace 拆分后的版本策略是 lockstep 还是独立版本。

这些问题应分别通过小型设计文档确认，不阻塞本次四层目录决策。

## Changelog

- 2026-07-22：确认 `models / core / runtime / apps` 四层目标结构；目录去掉 `rove-` 前缀，Cargo package 保留前缀；记录从单 crate 渐进迁移的边界与验收条件。
