# rove 对 Grok Build 的借鉴与 TUI 方向 - 2026-07-16

> Status: **Partially Implemented / First vertical slice landed 2026-07-18**
>
> 本文记录对 Grok Build 的参考分析，并为可选的全屏 `rove tui`
> 定义目标方向。首个纵向切片已经实现；本文仍不是完整当前运行时说明，
> 未实现的 approval/input modal、session picker 和 PTY hardening 仍属于后续工作。
> 当前事实仍以 [`docs/runtime/`](../runtime/README.md) 为准。

## 1. 决策摘要

| 领域 | 决策 |
|---|---|
| 产品入口 | 保留当前 rich REPL 和 `rove exec`，新增可选的 `rove tui` 命令。 |
| 运行时 | 复用 `CliRuntime`、`Engine`、`StreamEvent`、状态 artifacts、approval 与 input providers，不创建 TUI 私有 agent loop。 |
| UI 架构 | 使用 reducer 风格的 `State -> Action -> Effect` 边界；运行时事件继续通过现有 `RunViewState` 投影。 |
| 终端技术栈 | 第一版使用上游 `ratatui` 与 `crossterm`、alternate screen 和统一的终端清理 guard。 |
| 第一版目标 | 键盘优先、单会话，目标包含 transcript、plan/tool 活动、composer、状态、approval/input、cancel 和 resume；当前已落地基础 transcript、plan/tool、composer、状态、cancel 和启动时 `--resume`。 |
| 兼容性 | 默认 REPL、`rove exec`、API、Web、事件名、report、trace 和 resume 行为保持兼容。 |
| 参考边界 | 学习 Grok Build 的模式，不整体移植其 pager、shell、permission 或 MCP 子系统。 |

## 2. 当前基础

rove 已有最重要的 renderer-neutral 基础：

- 当前交互面仍保留 rich、line-oriented REPL；全屏 TUI 的当前行为见
  [implementation guide](../runtime/implementation-guide.md#full-screen-tui)；
- [`RunViewState`](../../src/interfaces/terminal/view.rs) 将 canonical
  `StreamEvent` 投影为终端状态；
- [`TerminalAction`](../../src/interfaces/terminal/action.rs) 已包含 prompt、
  cancel、approval、input、resume、status、sessions 和 exit 动作；
- CLI、API 与 Web 复用相同的 engine 和 durable state model；
- Web workbench 继续作为更丰富的浏览器界面。

首个切片已补齐键盘事件、焦点、布局、渲染、terminal lifecycle、共享异步
run driver 和 artifact finalization。当前仍缺少 modal 交互、session picker
和 PTY 级验证。本文是现有
[TUI-ready terminal plan](../plans/2026-06-09-tui-ready-terminal-architecture.md#follow-up-plan-after-this-one)
所预留的后续方向。

## 3. Grok Build 参考边界

本次参考快照为
[`xai-org/grok-build@c68e39f`](https://github.com/xai-org/grok-build/tree/c68e39f60462f28d9be5e683d9cbe2c57b1a5027)。
它是从更大 monorepo 周期同步出来的单提交公开快照。其 Rust 代码量比
rove 高一个数量级以上，仅 pager 就显著大于完整的 rove runtime。

值得借鉴的模式包括：

- 围绕 Action/Effect dispatch 的轻量终端事件循环；
- renderer-independent 的 session/application state；
- TUI、headless 与 ACP 复用同一 agent service；
- 基于 channel 的 approval 和 input 交互；
- 带虚拟屏幕、resize 和帧耗时的 PTY 测试；
- structured headless output、session rewind、task monitoring、agent
  profiles、skills、现代 MCP transport 和细粒度 permission UX。

需要保留的警惕包括：

- 其源码树对 Windows 构建只提供 best-effort 支持；
- Plan Mode 文档明确存在 shell 与 subagent 写入门控缺口；
- hooks 和不可用的 sandbox enforcement 可能 fail-open；
- 部分直接 path permission 弱于 rove 已有的 canonical workspace boundary；
- MCP 结果仍有文本扁平化，而 rove 的 Tool Output Envelope 与 Artifact
  设计应保留 typed content；
- 复制代码需要审查 Apache-2.0 和第三方 notices；Grok 中来自 Codex 或
  OpenCode 的 port 应优先回看原始上游。

## 4. TUI 之后值得继续学习的能力

| 能力 | rove 方向 |
|---|---|
| Structured headless output | 在 `rove exec` 下增加 `plain`、`json`、`jsonl`，保持 stdout/stderr 适合脚本。 |
| Coding tools | 增加有界 read range、目录列表、grep 和 atomic patch/edit，同时保留 canonical path 与 mutation reporting。 |
| Agent definitions | 延续 versioned `AgentDefinition -> AgentRuntimeProfile` 设计，Grok 主要作为 authoring UX 参考。 |
| Workspace instructions 与 skills | 实现 scoped `AGENTS.md` 和 progressive skill loading，但不把 prose 当成 permission。 |
| Execution lifecycle | 在暴露更丰富的 plan 控件前，继续 bounded StepRunner、append-only StepRecord、PlanRevision、Evaluator 和 Finalizer。 |
| MCP | 增加 Streamable HTTP、session negotiation、pagination、refresh 和 OAuth，同时保留 rove 的保守安全与 rich artifact model。 |
| Permissions 与 sandbox | 在 operator hard caps 下增加 deny/ask/allow；承诺 strict sandbox 前先定义 fail-closed 与真实 Windows 方案。 |
| Background work 与 subagents | 在 scheduler 或并行子会话前，先实现 typed task registry、lifecycle events、output bounds、cancellation 和 workspace isolation。 |
| Memory 与 code intelligence | 基础 coding tools 稳定后，再考虑跨 clone repo identity、hybrid search、LSP 和增量 code graph。 |
| Product hardening | 选择性借鉴 PTY 测试、crash-safe terminal restore、secret sanitization 和跨平台 packaging。 |

这些是独立后续方向。TUI 不得在 interface 层提前、隐式实现其未来语义。

## 5. TUI MVP

### 5.1 产品契约

现有命令含义保持不变：

```text
rove                 rich scrollback REPL
rove "prompt"        带初始 prompt 的 rich REPL
rove exec "prompt"   非交互执行
rove tui             可选的全屏终端 UI
```

第一版只支持一个 active session 和一个 active run。多 session tabs、
并发 prompt queue、background tasks 和 subagent dashboard 属于后续功能。

### 5.2 布局

使用简单纵向布局，保证窄终端仍可用：

```text
+----------------------------------------------+
| transcript: user, assistant, tool, plan       |
|                                               |
+----------------------------------------------+
| activity: current step / tool / model status  |
+----------------------------------------------+
| composer                                      |
+----------------------------------------------+
| workspace | model | run status | key hints    |
+----------------------------------------------+
```

Approval、user input、session selection 和展开后的 tool detail 使用 modal
overlay。第一版不要求 mouse interaction 和复杂 pane resize。

### 5.3 数据流

```text
crossterm input
  -> TerminalAction / TuiAction
  -> reducer
  -> TuiEffect
  -> shared CliRuntime / Engine

Engine StreamEvent
  -> RunViewState::apply_event
  -> TuiState
  -> ratatui render

ToolApprovalProvider / UserInputProvider
  -> bounded channel + oneshot response
  -> approval/input modal
```

主循环可以通过 `tokio::select!` 同时消费 terminal input、engine events、
effect completion、cancellation 和低频 animation tick。异步 Effect 不得在
`await` 期间持有可变 UI state。

### 5.4 目标交互（部分实现）

以下列表描述完整目标，不代表首个纵向切片已经全部支持。当前已实现
prompt、cancel、focus、transcript scroll 和确认退出；approval/input modal
与 session modal 仍未实现。

- `Enter`：idle 时提交 prompt；
- `Ctrl+C`：取消 active run，idle 时清空 draft；
- `Tab`：切换 transcript/composer focus；
- 方向键和 PageUp/PageDown：浏览 transcript；
- approval modal：approve once 或 reject once；
- input modal：提交回答；
- session modal：列出并 resume 已有 task states；
- `Ctrl+Q`：存在 active work 时确认后退出。

精确键位可以在实现期调整，但每个动作必须映射为 typed action，不能在
key handler 内直接执行 runtime work。

## 6. 不变量与非目标

TUI 必须保持：

- `StreamEvent` 仍是 canonical lifecycle contract；
- 工具执行仍经过 `ToolRegistry`、`Executor`、approval 和 workspace safety；
- terminal presentation 不能授予权限，也不能把 failure 解释为 success；
- trace、task state、report、SQLite index 和 resume 继续共享；
- cancellation 使用现有 token，并产生诚实的 terminal state；
- 不把 hidden model reasoning 暴露为 TUI 内容；
- panic、error 和 normal exit 都恢复 raw mode、cursor 与 alternate screen；
- logs、debug panel 和 snapshots 不泄漏 secrets。

第一版不包括：

- 自定义 Ratatui fork 或 inline/native-scrollback viewport；
- mouse、image paste、voice、Mermaid render 或 theme marketplace；
- multi-agent dashboard、scheduler、prompt queue 或 background task manager；
- 新的 AgentDefinition、MCP、memory、planning 或 permission 语义；
- 与 `StreamEvent` 平行的 TUI 私有 runtime lifecycle。

## 7. 实现切片

1. **Terminal shell**：增加 `rove tui`、Ratatui/Crossterm 初始化、cleanup
   guard、基础 state/reducer 和 TestBackend render。
2. **Runtime stream**：提交 fake-provider prompt、消费 `StreamEvent`、渲染
   transcript/activity、取消 run，并保持 report finalization。
3. **Human interaction**：增加 channel-backed approval/input providers 和
   对应 modal。
4. **State navigation**：增加 session list/resume、tool detail、scroll、
   resize 和 narrow-terminal fallback。
5. **Hardening**：增加 PTY smoke、terminal restore、Windows/Unix 验证、help；
   行为真实落地后再更新 current runtime docs。

每个切片都必须保持默认 REPL 和 `rove exec` 测试通过。

## 8. 完整目标验收条件（部分未实现）

以下条件用于判断完整 TUI MVP，而不是首个纵向切片的当前验收声明。
当前已满足 shared engine/artifacts、基础 cancel/exit、resize/narrow render
和默认测试门禁；approval/input、session picker、PTY 及 Windows/Unix
交互验证仍待后续切片完成。

- `rove tui --model fake` 不依赖 provider credential 即可启动；
- prompt 进入 shared engine，流式显示并写出原有 trace/task/report artifacts；
- destructive tool approval 与 `request_input` 可在 TUI 内完成；
- cancel、error、completion 和 exit 都能正确恢复 terminal；
- 可列出并 resume 已有 task states；
- resize 和窄终端不会 panic 或丢失 composer；
- 不引入 TUI-only runtime lifecycle 或 persistence format；
- `cargo fmt --all --check`、`cargo clippy --all-targets -- -D warnings`、
  focused TUI/CLI tests 和 `cargo test` 通过。

## 9. 与其他设计的关系

- 当前 REPL contract 仍由
  [Rich Terminal REPL Design](2026-06-09-rich-terminal-repl-design.md) 定义；
- Agent packaging 与 workspace instructions 仍属于 proposed
  [Agent Definition and Procedural Knowledge](2026-07-14-agent-definition-and-procedural-knowledge-design.md)；
- planned execution semantics 仍属于 proposed
  [Agent Execution Lifecycle](2026-07-14-agent-execution-lifecycle-design.md)；
- modern MCP 与 rich tool result 仍属于 proposed
  [MCP Streamable HTTP and Tool Artifacts](2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)。

只有对应代码与测试落地后，才能同步更新 `docs/runtime/`。
