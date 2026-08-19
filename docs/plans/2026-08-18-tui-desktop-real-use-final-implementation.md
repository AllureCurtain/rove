# Rove TUI 与 Desktop 真实可用最终实现规范

> Status: **Partially Implemented / F4 and T7 complete on `feature/tui-real-use-final`; Desktop D6 and final A Gate pending**
>
> Date: 2026-08-18
>
> Scope: 完整交付一条使用真实 Provider、真实仓库工具和共享 Harness 的
> TUI 与 Windows Desktop 产品路径。本文是该范围的最终实施合同，不是分析报告、
> 探索计划或“下一步建议”。除明确列入非目标的事项外，所有要求都是必交付项。
>
> Current-state authority: 当前代码、测试、生成契约与
> [`../runtime/`](../runtime/README.md) 仍是已实现行为的来源。本文描述目标实现，
> 不能用于声称真实 Provider、TUI 或 Desktop 已经通过验收。

## 0. 最终交付合同

本实现不再扩展 Rove 的架构宽度。现有测试不因本实现删除，但新增测试必须直接
保护真实用户黄金路径，不能继续为未使用的产品表面扩张组合矩阵。

所有“真实可用”“可以演示”“外部 Provider 通过”和最终 `Implemented` 结论都必须
来自真实模型请求。Fake Provider 仅允许用于单元测试、解析器测试、reducer 测试和
其他确定性合同测试；不得用于真实可用性 Gate、TUI/Desktop 演示、验收截图、成本
统计或产品完成声明。

本实现必须完整交付以下结果：

```text
安装 rove
  -> 进入一个真实仓库
  -> 启动 TUI
  -> 选择一个真实模型
  -> 用户提出仓库问题
  -> 模型发起原生 Tool Call
  -> Harness 在当前 Workspace 内执行工具
  -> 工具结果返回模型
  -> 用户看到计划、工具活动、结果依据和最终回答
  -> 用户继续追问，或批准一次有边界的修改
```

TUI 与 Desktop 使用相同的 Provider Catalog、API、Runtime、
Tool Registry、canonical events 和持久化状态完成同一任务。Desktop 不得创建
第二套 Agent loop、工具权限或会话语义。

交付包不可裁剪：

| 交付包 | 必须产生的最终结果 |
|---|---|
| F. 共享真实 Provider 基础 | 使用指定硅基流动模型安全配置、探测并完成原生工具往返 |
| T. TUI 产品路径 | 安装后执行 `rove` 即可在当前仓库连续对话、调用工具、审批和恢复 |
| D. Desktop 产品路径 | 安装后从开始菜单选择仓库并完成与 TUI 等价的真实 Agent 任务 |
| A. 最终验收与证据 | TUI、Desktop、真实 Provider、Harness、修改、测试、重启证据齐全 |

实现阶段可以由两个 worktree 并行推进，但最终验收不可并行越级：F 必须先通过，
随后 TUI 通过，最后 Desktop 才能取得完成状态。任何交付包失败，整份实现均保持
`Not Implemented` 或 `Partially Implemented`，不得用其他包的成功替代。

## 1. 当前事实与缺口

以下事实来自当前代码和 runtime 文档，不是本规范的完成声明。

| 能力 | 当前事实 | 本规范必须补齐的证据或行为 |
|---|---|---|
| TUI 入口 | 无参数 `rove` 已默认进入全屏 TUI | 安装后的二进制在外部仓库启动，且无需 Cargo |
| Workspace | CLI 从当前工作目录执行 `Workspace::detect` | 界面明确显示实际根目录，工具调用全部绑定该根目录 |
| 会话 | TUI/REPL 已有 Session、resume 和 canonical event 路径 | 用真实模型完成连续两轮对话，不丢失工具依据 |
| Provider | OpenAI Chat、OpenAI Responses、Anthropic、Ollama 等适配器存在 | 指定硅基流动 Provider/model 完成真实原生 Tool Call |
| Fake Provider | Fake 是确定性回声和脚本播放器 | 仅用于单元/确定性合同测试，不得进入真实可用或演示证据 |
| 工具循环 | Core Agent、Runtime Engine、Tool Registry 和本地工具已实现 | 证明模型主动调用 list/search/read，并根据真实结果作答 |
| Harness 可见性 | canonical events 能表达计划、工具、审批、状态和结果 | TUI/Desktop 把关键事件转成用户可理解的进度与证据 |
| Provider onboarding | Catalog、引用式密钥、探测接口和 TUI `/model` 已存在 | 未配置时不能只报错；必须给出可恢复的配置路径 |
| Desktop | Tauri 壳、嵌入式 API、WebView、目录选择和 Windows 包证据存在 | 完整 installed Desktop 真实 Provider 旅程仍未验证 |
| Desktop 凭据 | Desktop 进程可解析 Provider Catalog 和进程环境 | 从开始菜单启动时不能依赖临时 PowerShell 环境变量 |
| 测试 | 大量确定性、集成和 Web 测试存在 | 不删除；新增验证集中到黄金路径，日常门禁按影响范围运行 |

当前明确不能声称：

- Fake Provider 的成功等于 Agent 能理解仓库；
- Provider 协议单元测试等于真实外部服务互操作；
- Desktop 能构建或保持进程存活等于完整产品旅程可用；
- 模型给出文字回答等于 Harness 已经调用工具；
- mocked browser 测试等于真实 Provider 的 Desktop 演示。

### 1.1 Fake Provider 的唯一允许边界

Fake Provider 可以继续保护以下确定性行为：

- Provider-neutral message、tool schema 和 parser 单元测试；
- Runtime 状态机、预算、resume、审批和 canonical event 合同；
- TUI/Web reducer、渲染和错误状态；
- 无网络 CI 中的安全负向测试。

Fake Provider 不得作为以下任何一项的输入：

- F4 的 Provider 完成证据；
- T7 的真实对话和 Harness 证据；
- D6 的 Desktop 安装态证据；
- A2、A3、A4 的最终验收；
- README 或发布材料中的产品演示；
- 真实 Token、成本、延迟、工具选择质量和任务成功率评估。

## 2. 产品目标与非目标

### 2.1 TUI 目标旅程

用户在已安装 Rove 和已完成 Provider 配置的机器上执行：

```powershell
cd D:\path\to\a-real-repository
rove
```

随后必须满足：

1. TUI 在正常终端中稳定打开并显示当前 Workspace、当前模型和会话状态。
2. 用户输入“当前目录有哪些内容？请先检查再回答”。
3. 模型收到当前 Workspace 信息和权威工具 Schema。
4. 模型发起原生结构化 Tool Call，而不是在文本中伪造 JSON 工具调用。
5. Harness 显示工具名称、受限参数、执行状态和有界结果摘要。
6. 工具结果以正确的 provider history 结构返回模型。
7. 最终回答引用真实文件或目录，不允许在未调用工具时伪装检查过仓库。
8. 用户继续问“入口文件在哪里，依据是什么？”，Agent 延续同一会话并再次按需搜索或读取。
9. 用户可以取消运行；发生错误时能看到可恢复的错误类型，而不是 TUI 直接退出。
10. 对修改性任务，审批、实际变更、测试结果和 diff 都可见且保持 Workspace 边界。

### 2.2 Desktop 目标旅程

在 TUI Gate 通过后，用户从已安装的 Windows Desktop 执行：

1. 启动 Rove，嵌入式 API 和 WebView 正常就绪。
2. 通过原生目录选择器选择一个真实 Git 仓库。
3. 选择已经配置并可解析凭据的真实 Provider/模型。
4. 创建 Session，提出与 TUI 相同的问题。
5. 在 Chat/Inspector 中看到相同的计划、工具、审批、结果、文件和 diff 事实。
6. 切换页面或短暂断连后，SSE 能重连并恢复正确 Session/Run。
7. 关闭并重新启动 Desktop 后，Workspace、Session、对话和终态仍可恢复。

### 2.3 Harness 应展示什么

TUI 和 Desktop 都应展示可观察事实：

- 当前 Provider、模型、Workspace 和 Session；
- Planner 产生的用户可理解步骤摘要；
- 工具名称、开始/完成/失败状态；
- 有界且经过脱敏的工具参数；
- 审批请求和用户决定；
- 文件读取、搜索命中、Shell 命令、变更文件和 diff；
- Token/费用可用性、运行终态和最终回答依据；
- 取消、超时、预算耗尽、Provider 错误和需要用户介入的状态。

不得展示或伪造：

- Provider 的隐藏 chain-of-thought；
- 原始密钥、Authorization header 或未脱敏请求；
- 将 prompt 文本冒充工具权限；
- 未执行工具时的虚假“已检查”状态；
- 把 `partial`、`blocked`、`cancelled` 或 `failed` 显示为完成。

### 2.4 非目标

本规范范围不包括：

- 新增 Provider 类型或重新设计 Provider 抽象；
- Web SaaS、账号、同步、计费或远程执行；
- Desktop 自动更新、签名和 macOS/Linux 发布；
- Browser/Desktop 自动化 Workspace；
- 新 MCP 协议、向量 RAG、Subagent 或多 Agent 编排；
- TUI 多标签后台任务管理；
- 为所有 Provider、平台和失败组合增加完整测试矩阵；
- 为演示放宽 Workspace、审批、密钥或日志安全边界。

## 3. 共享架构约束

实现必须沿用现有共享链路：

```text
TUI -----------------------------------------------------+
                                                          |
Desktop -> WebView -> Product API/SSE -------------------+-->
  App Bootstrap -> Runtime Engine -> Core Agent Kernel
    -> Provider Client -> native model/tool protocol
    -> Tool Registry -> approval/safety -> Execution Environment
    -> canonical events -> state/trace/artifacts/product projection
```

约束如下：

1. TUI、Desktop、API 和 Web 不得各自实现模型/工具循环。
2. Provider-specific payload 必须留在 `rove-models` 边界。
3. 工具执行必须经过现有 Tool Registry、Executor 和审批路径。
4. TUI 与 Desktop 只能投影 canonical events，不创建私有生命周期。
5. `trace.jsonl`、`task_state.json` 和 Artifact 仍是 Runtime 事实；UI 不是事实来源。
6. Fake Provider 继续作为测试专用的确定性 fixture，不允许进入产品运行配置或
   静默回退为正常产品模型。
7. 所有路径以解析后的 Workspace 为边界，Provider 输出不能成为本地路径授权。
8. Provider Catalog 是配置权威，ProductStore 只保存安全投影和选择关系。

主要实现入口：

- TUI：[`../../apps/cli/src/tui/`](../../apps/cli/src/tui/)
- CLI Runtime assembly：[`../../apps/cli/src/cli/runtime.rs`](../../apps/cli/src/cli/runtime.rs)
- Provider factory/catalog：[`../../apps/bootstrap/src/`](../../apps/bootstrap/src/)
- Core Agent loop：[`../../core/src/`](../../core/src/)
- Runtime Engine/tools：[`../../runtime/src/`](../../runtime/src/)
- Desktop host：[`../../apps/desktop/`](../../apps/desktop/)
- Web product shell：[`../../apps/web/`](../../apps/web/)

### 3.1 强制代码变更面

实现者必须优先修改以下现有所有权点，不得新建平行子系统：

| 模块 | 强制实现 |
|---|---|
| `apps/bootstrap/src/provider_catalog.rs` | 增加共享 `ProviderOnboardingService`，统一 metadata 校验、keyring 写入、Catalog CAS、失败补偿、probe 和默认选择 |
| `apps/cli/src/cli/args.rs` | 增加 `provider add/test/use/list` 命令合同和脱敏参数 |
| `apps/cli/src/cli/provider.rs` | 实现无回显 secret 输入、onboarding service 调用、安全输出和退出码 |
| `apps/cli/src/cli/runtime.rs` | 保持 current-directory assembly，并把缺失/失效 Provider 作为可恢复 UI 状态 |
| `apps/cli/src/tui/{state,action,effect,reducer,render,app,providers}.rs` | 实现完整 onboarding 状态机、模型选择、错误恢复和 Harness 事件投影 |
| `models/`、`core/`、`runtime/` | 只修复 F Gate 证明存在的 native tool/history/Harness 缺陷，不增加第二循环 |
| `apps/desktop/src/commands.rs` | 实现 Windows 原生 `provider_credential_prompt` 和仅返回安全 receipt 的宿主命令 |
| `apps/desktop/src/lib.rs` | 注册宿主命令并保持 API/token/WebView 生命周期 |
| `apps/web/platform/desktop-commands.ts` | 提供类型化 Desktop host wrapper，拒绝非 Desktop 环境误用 |
| `apps/web/settings/CatalogSettings.tsx` 与 provider settings model | 在 Desktop 环境调用原生 onboarding，在 Web 环境保持引用式 Provider CRUD |
| `apps/api/src/product/provider_catalog.rs` | 继续作为 Web/API Catalog 投影；只在共享服务需要安全新字段时做兼容扩展 |
| `scripts/` | 增加最终 real-use acceptance runner，记录真实退出码和仓库外 evidence 路径 |
| `docs/runtime/` 与 `README.md` | 实现完成时同步真实当前行为、安装步骤、证据和未验证边界 |

`ProviderOnboardingService` 的操作顺序必须固定：

1. 校验 profile id、provider type、base URL、model 和 secret source；
2. 按 provider type 做本地 endpoint/protocol/capability 预检；
3. 将 secret 写入本次操作唯一的 keyring entry，获得不透明 reference；
4. 用内存中的候选 profile 执行 credentialed model inventory/connection test；
5. 测试通过后，使用 expected catalog revision 在一次原子替换中写入引用式 profile
   和默认 selection；
6. 重新读取 Catalog，验证 revision、profile、selection 和 keyring reference；
7. 发布前任一步失败或 CAS 冲突时补偿删除本次 keyring entry；发布后的不确定状态
   必须返回 typed reconciliation error，不得无条件覆盖并发 Catalog 更新；
8. 返回只包含 profile、model、revision、capability 和 health 的安全 receipt。

不得修改 ProductStore schema 来保存 raw secret。若新增公共序列化字段，必须提供
默认值、向后兼容读取、OpenAPI/Web 消费者和迁移测试。

## 4. Provider 策略

### 4.1 锁定一个必过的真实 Provider

最终实现的唯一主验收目标固定为此前已配置的硅基流动低额度模型：

| 字段 | 强制值 |
|---|---|
| Provider type | `openai`（OpenAI-compatible Chat Completions） |
| Base URL | `https://api.siliconflow.cn/v1` |
| Model | `deepseek-ai/DeepSeek-V3.2` |
| Credential reference | `SILICONFLOW_API_KEY` 或指向同一凭据的 OS keyring reference |
| Tool protocol | Provider 返回的原生 structured tool calls |

该模型必须实际消耗硅基流动额度完成 inventory、streaming、原生工具调用、工具结果
回传、连续对话、修改和测试任务。若该模型不能稳定支持原生工具调用，本规范保持
未完成，并记录真实失败；实现者不能自动换成 Fake、mock、官方 OpenAI 或其他高
成本模型。更换真实模型必须由用户明确批准并同步修订本文。

Anthropic、OpenAI Responses、Ollama 和其他兼容 gateway 必须保持现有兼容性，
但它们不是本规范的替代验收证据。

选定目标后，证据必须记录以下非敏感身份：

- provider type；
- wire protocol；
- endpoint 的安全标识；
- model id；
- catalog revision；
- 是否支持 streaming/native tool calls/usage；
- 执行时间、终态和退出码。

### 4.2 最终凭据与 onboarding 合同

最终交付必须同时支持三条入口，它们写入或选择同一个 Provider Catalog：

1. CLI `rove provider add/test/use/list`，用于安装后配置、诊断和自动化；
2. TUI 首次启动 onboarding 和 `/model`，用于纯终端用户；
3. Desktop Settings 加宿主原生安全凭据提示，用于开始菜单启动的 GUI 用户。

`rove provider add` 的默认交互流程必须使用无回显 secret 输入并写入 OS keyring。
高级用户可以显式选择 environment 或 file reference。`test` 必须验证 endpoint、
credential 和 model inventory；`use` 必须通过 Catalog CAS 设置默认 profile/model；
`list` 只能显示安全元数据。配置中只保存引用，例如：

```toml
schema_version = 1

[model]
default_profile = "siliconflow-deepseek-v3-2"
default_model = "deepseek-ai/DeepSeek-V3.2"
reasoning = "default"

[provider.profiles.siliconflow-deepseek-v3-2]
label = "SiliconFlow DeepSeek V3.2"
provider_type = "openai"
base_url = "https://api.siliconflow.cn/v1"
model = "deepseek-ai/DeepSeek-V3.2"
auth = { style = "bearer", secret = { env = "SILICONFLOW_API_KEY" } }
```

未配置 Provider 时，TUI 必须进入可恢复 onboarding 状态，而不是在首次发送时
退出。该流程必须复用 Provider Catalog 的 schema、CAS、文件锁、原子写入、
symlink 拒绝和 secret-reference 规则。

Desktop 必须提供 `provider_credential_prompt` 等价的 Tauri 宿主命令：Web 只提交
profile 的非敏感元数据，Rust 宿主调用 Windows 原生安全凭据对话框，直接把 secret
写入 OS keyring，并仅向 Web 返回不透明引用和成功/失败。raw key 不得进入 React
state、localStorage、ProductStore、普通 HTTP 请求、日志、trace、截图或
`desktop.json`。共享 CLI 配置可以作为恢复入口，但不能是 Desktop 唯一可用入口。

临时 PowerShell 环境变量仅允许用于开发诊断，不能作为 TUI 或 Desktop 最终完成
证据。

### 4.3 Provider 验收顺序

1. Model inventory/probe 成功。
2. 普通 streaming 文本成功。
3. 原生单工具调用成功。
4. 工具结果返回后模型完成回答。
5. 多轮 history 中 tool call id 和 tool result 关联正确。
6. Usage/stop reason 正确或明确标记 unavailable。
7. 无效工具参数形成可恢复错误并返回模型修正。
8. 超时、401/403、429 和 5xx 在 TUI/Desktop 中得到可理解且不泄密的错误。

只有第 3-5 项通过，才证明 Agent Harness 的核心效果。

### 4.4 硅基流动额度控制

真实模型 Gate 必须显式 opt-in，不进入每次 push 的默认 CI。所有真实 TUI/Desktop
任务固定使用 `deepseek-ai/DeepSeek-V3.2`，并采用以下额度边界：

- 每个验收任务最多 6 个 plan steps；
- 每个任务最多 12 次 tool calls；
- Provider/网络失败最多自动重试 1 次；
- 不运行 stress、soak、并发压测或无关模型 inventory 全扫描；
- 只执行第 9 节规定的最小演示任务和一次必要的错误恢复；
- 每次运行记录 prompt/completion/total token、可用成本和终态；
- 达到预算、429 或余额不足时立即停止并保存失败证据，不切换 Fake 继续。

确定性单元/合同测试继续使用 Fake，不消耗硅基流动额度；任何声称真实可用性的
测试都必须明确设置真实 Gate 开关并使用上述硅基流动 profile。

## 5. F：共享真实 Provider 基础实现

### F1. 目标

交付一个已修复、已验证、可被 TUI 与 Desktop 共用的硅基流动 credentialed
Provider 闭环。初始探测只是定位手段；无论现有实现是否通过，该交付包都必须以
指定 `deepseek-ai/DeepSeek-V3.2` 真实调用仓库工具并完成回答结束。

### F2. 隔离验收环境

从最新 `main` 创建独立 worktree 和分支，例如：

```text
worktree: ../rove-tui-final
branch: feature/tui-real-use-final
```

准备一个仓库外的固定 fixture，至少包含：

```text
demo-repo/
  README.md
  src/main.rs
  src/config.rs
  tests/smoke.rs
```

内容要包含可确定验证的事实，例如 `src/main.rs` 调用 `config::load()`。状态和配置
必须使用独立 `ROVE_DATA_ROOT`/`ROVE_CONFIG_ROOT`，不得读取或修改操作者真实状态。

### F3. 实现与验证顺序

1. 通过最终 CLI/keyring 路径配置 `siliconflow-deepseek-v3-2` profile 和 secret reference。
2. 先运行现有 provider inventory/test，确认 endpoint 和模型存在。
3. 使用 `rove exec` 在 fixture 上执行一次只读任务，隔离 TUI 渲染变量。
4. 要求模型“必须检查当前目录后回答”，确认 trace 中出现原生工具调用。
5. 若请求、streaming、stop reason、tool call、tool result history 或 usage 有问题，
   在 `models`/`bootstrap`/`core`/`runtime` 的既有所有权边界内修复。
6. 实现 `rove provider add/test/use/list` 与共享 keyring 引用。
7. 再运行无参数 `rove`，在 TUI 中执行相同任务。
8. 保存 secret-free trace、report、终端输出和 Provider identity。

### F4. 完成定义

- [x] `provider add/test/use/list` 使用统一 Catalog 且不泄漏 secret；
- [x] keyring、environment 和 file reference 保持既有安全语义；
- [x] inventory/probe 通过锁定的真实 endpoint/model；
- [x] streaming 文本、stop reason 和 usage 正确或明确 unavailable；
- [x] 真实模型产生 native tool call，而不是 JSON 文本兼容调用；
- [x] tool call id、工具结果和下一轮 provider history 正确关联；
- [x] 模型根据工具结果产生 grounded final answer；
- [x] 401/403、429、5xx、超时和无效参数返回类型化且脱敏的错误；
- [x] Fake、mock 和 credentialed evidence 明确分开。

F4 evidence: credentialed SiliconFlow `tui-gate-10` completed with exit code
`0`; source fixes are split into `50e5274` and `b86aeaa`.

任何只返回流式文字、没有真实工具事件的结果都不能算 F 完成。

## 6. T：TUI 最终实现

### T1. 安装与零参数入口

目标：安装一次后，在任意目录输入 `rove` 即进入正确 Workspace 的 TUI。

工作项：

- 验证 `cargo install --locked --path apps/cli` 产生独立可运行的 `rove`；
- 将发布/演示构建与日常 `cargo run` 区分，演示不依赖仓库 `target/`；
- 启动时显示规范化 Workspace 根、Workspace kind、Provider/model 和 session；
- 当前目录不可访问时给出明确错误，不静默回退到其他目录；
- 未提供消息时始终进入 TUI，`rove tui` 保留为显式别名；
- 启动失败必须恢复终端状态。

验收：

- 在三个不同目录运行 `rove`，显示并绑定三个正确根目录；
- 从 Git 子目录启动时，Workspace 规则与 `Workspace::detect` 的当前契约一致；
- 启动后没有任何模型请求，直到用户提交消息；
- 已安装二进制启动不触发 Rust 编译。

### T2. Provider onboarding 与模型选择

目标：用户缺少 Provider 配置时仍能留在 TUI 内完成恢复。

工作项：

- 把 `provider_onboarding_required` 投影为 TUI onboarding 状态，不直接终止应用；
- 提供“查看配置位置、重新加载、测试连接、选择已配置模型”的最小流程；
- 保留 `/model` picker，并显示 profile、provider type、model 和健康状态；
- Provider probe 失败时保留用户输入和 session，不创建虚假 run；
- profile/model 变更只作用于下一次 run，并遵守 resume identity 校验；
- 不允许缺少配置时静默选择 Fake；
- TUI onboarding 必须直接调用 F 中实现的 Catalog/keyring 服务；
- 首次配置必须以掩码、无历史记录的输入接收 secret，写入 keyring 后立即清除
  UI buffer，且不能进入 REPL/TUI history、trace 或日志；
- 首次配置必须能创建 profile、输入密钥、测试连接、选择模型并设为默认；
- 配置成功后无需退出 TUI，原始 composer 内容保持并可立即提交。

### T3. 真实 Agent 工具循环

目标：仓库问题必须由模型、Harness 和工具共同完成。

工作项：

- 核对 system prompt 中的 Workspace、可用工具、边界和完成规则；
- 确认 model-facing schema 来自权威 Tool Descriptor 投影；
- 使用 native tool calls，兼容文本解析只能用于显式标记的旧 Provider；
- 将工具结果按 Provider 原生 history 结构返回，保留 tool call id；
- 对 list/search/read 的成功、截断、缺失、二进制和权限错误提供类型化结果；
- 工具错误应允许模型在预算内修正，不能立即伪装成成功回答；
- Finalizer 只能根据真实事件和证据报告完成度；
- 普通只读仓库检查不要求破坏性审批；写文件和 Shell 仍按现有策略审批。

最低任务：

1. “当前目录有哪些内容？请先检查再回答。”
2. “这个程序的入口在哪里？给出文件依据。”
3. “README 描述与代码入口是否一致？”
4. “把一个明确的小文本改动应用到指定文件，并展示 diff。”
5. “运行最小相关测试并解释结果。”

前 3 个任务必须只读；第 4、5 个任务必须清楚展示审批和 Shell/Mutation 事实。

### T4. Harness 可观察性

目标：用户能看懂 Agent 正在做什么，而不是只看到 spinner 和最终文字。

TUI 至少展示：

| 事件 | 默认投影 | 详情投影 |
|---|---|---|
| Run start | Provider/model、Workspace、预算摘要 | run/session id 和 runtime identity |
| Plan | 当前步骤和总步骤 | plan revision、attempt 和状态 |
| Tool start | 工具名和用户可理解动作 | 脱敏后的有界参数 |
| Approval | 风险、目标和允许/拒绝动作 | capability、mutation class |
| Tool result | 成功/失败、命中数、文件或命令摘要 | bounded result/artifact 引用 |
| Mutation | 修改的路径和数量 | diff/checkpoint/observation |
| Usage | Token/费用或 unavailable | Provider 返回的原始计量事实投影 |
| Terminal | success/partial/blocked/cancelled/failed | termination reason 和恢复动作 |

要求：

- 动态文本不能改变固定工具栏和输入区尺寸；
- 结果过长时使用详情视图或 Artifact，不把终端冲垮；
- 路径、命令和模型输出必须经过现有 sanitization；
- 用户可以从最终回答回看工具依据；
- 不显示隐藏推理内容。

### T5. 连续对话、取消和恢复

目标：演示不是一次性命令，而是最小可信会话。

工作项：

- 同一 TUI session 的第二轮携带正确对话和工具历史；
- 完成 run 后的下一条消息创建正确 successor，而不是重复旧 mutation；
- 运行中消息沿用统一 queue/promote/revoke 生命周期；
- Ctrl+C/取消产生 canonical cancelled 状态并恢复输入；
- Provider 短暂失败后保留 session 和消息，允许明确重试；
- 重启 TUI 后至少能恢复最近一个已完成 session；
- F.4 长历史 prepend 和完整 F.5 重启队列后台 successor drain 不属于本规范的
  产品范围；本规范仍必须完成最近已完成 Session 的 TUI/Desktop 重启恢复，且
  这两个未交付能力必须在界面和文档中诚实说明。

### T6. Windows 终端体验

目标：在目标演示机器上不依赖隐蔽按键知识。

工作项：

- 普通 composer 使用符合预期的 Enter 提交和 Shift+Enter/等价方式换行；
- Windows Terminal、PowerShell 和当前 crossterm key-event mode 做一次真实手工验收；
- 当前审批/输入 modal 保留 Windows F8 fail-closed 语义，界面必须清楚显示按键；
- 粘贴、按键 repeat/release 和旧事件不能误触发审批；
- resize、切换窗口、退出和 panic 后终端恢复正常。

不为了“像 Claude Code”而弱化审批确认的防误触边界。

### T7. TUI 完成定义

所有条件必须同时满足：

- [x] 已安装 `rove` 可从仓库外启动；
- [x] 无参数进入 TUI 并绑定当前 Workspace；
- [x] 未配置 Provider 时留在可恢复 onboarding 状态；
- [x] 指定硅基流动 `deepseek-ai/DeepSeek-V3.2` 通过 inventory/probe；
- [x] 真实模型产生原生 list/search/read Tool Call；
- [x] 工具结果返回模型并支撑最终回答；
- [x] 连续两轮对话保持正确 Session 和上下文；
- [x] 用户能看到计划、工具、结果、终态和依据；
- [x] 一次写入任务保持审批、Workspace 边界、diff 和测试证据；
- [x] Provider、取消和工具失败不会让 TUI 无解释退出；
- [x] secret-free 证据包可复查；
- [x] Fake 测试证据与硅基流动真实 Provider 证据明确分开。

T7 evidence: installed release CLI SHA256
`401fdb59756a3fef16328d2aa9c7e205bd927b5f471a55183cd0c95e6c449a11`;
credentialed evidence `<evidence-root>/tui-gate-10`; Windows ConPTY/manual
terminal and Desktop evidence remain explicitly unverified.

## 7. D：Windows Desktop 最终实现

### 7.1 并行开发与最终验收条件

Desktop worktree 可以与 TUI worktree 从同一基线并行创建：

```text
worktree: ../rove-desktop-final
branch: feature/desktop-real-use-final
base: 与 TUI worktree 相同的文档基线提交
```

Desktop worktree 可以并行完成构建、安装、原生目录选择、Web/API 投影、SSE、
重连、持久化和宿主原生凭据 UI，但不能复制或重写 Provider/Agent loop。F 中产生的
共享 Catalog/keyring/Provider 合同由 TUI worktree 先形成小而稳定的提交，Desktop
worktree 合入这些提交后完成 credentialed 集成。D6 的最终验收必须等待 F4 和 T7
通过。

若 Desktop 与 TUI 对相同 Runtime 产生不同结果，应定位 Product API/SSE 或投影
缺陷，不能在 Desktop 增加私有补丁绕过共享 Runtime。

### D1. 构建、安装与启动

工作项：

- 核对 [`../../apps/desktop/tauri.conf.json`](../../apps/desktop/tauri.conf.json)
  中 beforeDev/beforeBuild 命令的实际工作目录和 Web 路径；
- 验证 `apps/web/desktop-dist` 包含所需静态入口和 Next assets；
- 从干净构建产出 Windows installer，不依赖旧 `.next`/`target` 偶然状态；
- 安装后从开始菜单启动，而不是只运行 `cargo run`；
- 嵌入式 API readiness 失败时显示可操作错误和日志位置；
- 保持随机 loopback port、bearer token 和 origin-bound injection 现有边界。

### D2. 安装态 Provider 凭据

工作项：

- Desktop 与 TUI 读取同一个用户 Provider Catalog 语义；
- 共享 CLI/TUI keyring 配置完成后，开始菜单启动的 Desktop 可以直接解析；
- Settings 显示 profile/model、测试连接和模型列表，不显示 secret value；
- raw key 不进入 localStorage、ProductStore、普通 API 请求、日志、trace 或截图；
- 凭据失效时保留 Workspace/Session，并提供重新配置动作；
- 实现宿主原生 `provider_credential_prompt` 命令，调用 Windows 安全凭据对话框；
- Rust 宿主直接写入 keyring，Web 只接收成功/失败和不透明引用；
- Desktop 首次启动可以完整创建、测试、启用 profile，无需编辑 TOML 或打开终端。

仅通过临时 PowerShell 环境变量启动 Desktop 可以作为调试手段，不能作为安装态
完成证据。

### D3. Workspace 与 Session 黄金路径

工作项：

- 原生 folder picker 返回的路径经过 API/Runtime 的规范化和边界检查；
- 创建或复用 Product Workspace 时保存正确 kind/root；
- 创建 Session 后固化 profile/model/approval/step-limit snapshot；
- Chat 提交精确 `product_session_id`，不通过“latest”猜测运行；
- 单 active turn、完成后的 successor 和 Fork 保持现有权威语义；
- Desktop 重启后加载同一 Workspace 和 Session，而不是创建重复记录。

### D4. Chat 与 Harness 投影

Desktop 必须展示与 TUI 同源的事实：

- streaming assistant text；
- plan/step 状态；
- tool start/result/error；
- inline approval/input/cancel；
- 文件、Artifact、图片、usage 和 diff；
- reconnect/background/needs-attention；
- final termination status。

不得新增 Desktop-only 事件或根据 UI 本地状态推测工具已完成。Inspector 中的详情
必须来自 exact run/session 绑定。

### D5. SSE、重连和持久化

工作项：

- bearer-authenticated SSE 在 WebView 内正常工作；
- WebView 导航、最小化或短暂断连后只重连当前 focused job；
- 不因模糊的 job-start 响应自动重复提交；
- API shutdown 前完成 ProductStore/Runtime drain；
- 重启后 transcript 从 canonical events/ProductStore projection 恢复；
- partial、stale、migration failure 和 needs-attention 可见。

### D6. Desktop 完成定义

- [ ] 干净构建产生可安装的 Windows 包；
- [ ] 安装和卸载成功，启动不要求开发工具；
- [ ] 开始菜单启动后 Provider 凭据可解析；
- [ ] 原生目录选择创建正确 Workspace；
- [ ] 真实模型完成与 TUI 相同的两轮只读任务；
- [ ] Chat/Inspector 展示真实 Tool Call 和结果依据；
- [ ] 一次修改任务展示审批、变更、测试和 diff；
- [ ] SSE 断连/重连不重复提交任务；
- [ ] 重启 Desktop 后恢复 Workspace、Session 和终态；
- [ ] 日志、截图、trace、report 和 ProductStore 不含 raw secret；
- [ ] 证据明确标注 Windows-only，未验证平台不做发布声明。

## 8. 测试与验证合同

### 8.1 原则

1. 本规范不删除现有测试。
2. 新测试只保护新增用户旅程、已发现回归或安全边界。
3. 不为相同事实在 Core、Runtime、API、Web、TUI、Desktop 每层复制完整矩阵。
4. 内循环运行最小相关检查；全 Workspace 只在合并候选和发布 Gate 运行。
5. 真实 Provider 测试保持 opt-in，不进入每次 push 的默认确定性 CI。
6. skipped 只证明 skip 路径，不是通过。

### 8.2 必须新增的最小验证

| 层级 | 最小新增证据 | 目的 |
|---|---|---|
| Models/Core | 一个锁定 native tool call/history 往返的回归测试 | 保护真实 Gate 的协议合同 |
| Bootstrap/Catalog | `add/test/use/list`、CAS、keyring 引用和脱敏负向测试 | 保护共享配置权威 |
| Runtime | 一个 tool-result-to-model-to-final 的确定性黄金测试 | 保护共享 Harness |
| TUI | onboarding 成功、失败恢复、事件投影和连续两轮的聚焦测试 | 保护终端产品路径 |
| Real Provider | 一个 opt-in fixture 仓库两轮只读加一次修改 gate | 证明外部互操作和工具使用 |
| Desktop host | 原生 secret command、API readiness 和安全返回测试 | 保护安装态宿主 |
| Desktop/Web | 一个 live API 确定性回归，加一次 installed journey | 保护真实产品展示 |
| Security | secret、路径、审批和日志的负向测试 | 防止为演示弱化边界 |

表中测试是最小必交付集合。只有真实 Gate 暴露新的协议回归，或代码变更触及新的
安全边界时，才能增加额外测试；每个额外测试必须在 PR 中指向具体缺陷或本规范的
验收条目。

不能因为“覆盖率看起来更高”而增加以下测试：

- 每种 Provider 的相同 UI 状态排列；
- 与现有 canonical event 合同完全重复的 Desktop 单元测试；
- 只断言实现细节、私有字段或固定文案的脆弱测试；
- 大量 Fake Provider 场景冒充真实模型质量；
- 为未纳入本规范的 MCP、发布平台或高级 Settings 扩展矩阵。

### 8.3 分层命令

实现内循环按影响范围选择：

```powershell
cargo fmt --all --check
cargo test -p rove-models --lib
cargo test -p rove-core --lib
cargo test -p rove-runtime --lib
cargo test -p rove-cli --lib
cargo test -p rove-desktop --all-targets
```

Web/Desktop 表面变化：

```powershell
cd apps/web
pnpm test
pnpm typecheck
pnpm build:desktop
```

合并候选才运行仓库标准全 Gate：

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

真实 Provider Gate 使用
[`../runtime/provider-smoke.md`](../runtime/provider-smoke.md) 和现有 runner，保存真实
退出码。不得手工把报告状态改成 PASS。

## 9. 演示脚本

### 9.1 TUI 演示

演示前准备：

- 安装 release CLI；
- 使用 keyring 或 `SILICONFLOW_API_KEY` 配置 `siliconflow-deepseek-v3-2` profile；
- 使用固定、无敏感信息的 demo repository；
- 清理 demo repository 的工作树；
- 不清理用户真实状态，不使用生产服务数据。

演示步骤：

```powershell
cd D:\demo\rove-demo-repo
rove
```

依次输入：

1. `当前目录有哪些主要内容？请先实际检查，再给出简短说明。`
2. `这个程序的入口在哪里？请读取相关文件并说明依据。`
3. `把 README 中的演示标记改为 ready，修改前先说明计划，修改后展示 diff。`
4. `运行最小相关测试，并告诉我结果是否支持这次修改。`

观众必须看到：

- 硅基流动 Provider 和 `deepseek-ai/DeepSeek-V3.2`；
- Workspace 根；
- plan/step；
- list/search/read；
- approval；
- mutation/diff；
- Shell/test；
- grounded final answer。

### 9.2 Desktop 演示

1. 从开始菜单启动已安装 Rove。
2. 选择同一个 `rove-demo-repo`。
3. 确认 Provider/profile 健康。
4. 创建新 Session。
5. 执行 TUI 的前两个只读问题。
6. 展开 Inspector，展示 Tool、Files、Artifacts、Usage 和证据。
7. 执行小修改并批准，展示 diff 和测试结果。
8. 关闭应用，重新打开并恢复该 Session。

Desktop 演示不使用 DevTools、不手工调用 API、不在后台运行开发服务器，也不依赖
未提交的本地补丁。

## 10. 证据包

每个 Gate 生成一个 secret-free 证据目录，至少包含：

```text
evidence/
  manifest.json
  environment.md
  provider-safe-identity.json
  transcript.md
  tool-events.jsonl
  report.json
  git-status.txt
  checks.json
  screenshots/
```

`manifest.json` 记录 commit、平台、时间、命令、退出码、任务和文件哈希。任何原始
Provider 请求/响应在保存前必须检查 header、URL query、prompt 内容和密钥泄漏。

通过标准：

- transcript 中的目录/入口事实能对应 fixture 文件；
- tool events 至少包含一次真实 native tool call；
- 修改任务的 diff 与用户请求一致；
- checks 具有真实退出码；
- `git status` 只包含预期 demo 变更或完全干净；
- TUI 和 Desktop 的证据明确区分，不相互替代；
- Fake、mock、live local 和 credentialed external 标签准确。

### 10.1 A：最终集成验收 Gate

A Gate 必须在两个实现分支都合入后的最终 `main`、干净工作树和仓库外 evidence
目录中执行。它由五个连续且不可互相替代的 Gate 组成。

#### A1. 确定性代码 Gate

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/web
pnpm test
pnpm typecheck
pnpm build:desktop
pnpm test:e2e
cd ../..
```

所有命令必须记录真实退出码。Desktop/Tauri 构建依赖缺失、Playwright 浏览器缺失
或测试 skip 都必须明确分类，不能手工填写 PASS。

#### A2. Credentialed Provider Gate

使用硅基流动 `https://api.siliconflow.cn/v1` 和
`deepseek-ai/DeepSeek-V3.2` 执行 provider smoke 与受额度边界约束的
integration runner：

```powershell
$env:SILICONFLOW_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.siliconflow.cn/v1" `
  -ApiKeyEnv SILICONFLOW_API_KEY `
  -Model "deepseek-ai/DeepSeek-V3.2" `
  -SkipWebSmoke
```

不得为该 Gate 添加 `-RunStress` 或长时间 soak。必须保存 inventory、streaming、
native tool call、tool result history、usage/stop reason 和错误脱敏证据。只通过普通
文本请求不能通过 A2；切换 Fake 后通过也不能通过 A2。

#### A3. 安装态 TUI Gate

从最终 `main` 安装 release CLI，离开 Rove 源码目录，进入 fixture repository，
执行第 9.1 节全部四个问题。必须证明无参数 `rove`、keyring onboarding、两轮
只读工具调用、一次审批修改、diff、测试、取消/恢复和终端恢复。

#### A4. 安装态 Desktop Gate

从干净构建产出 Windows installer，安装后从开始菜单启动，不设置临时终端环境
变量、不运行开发服务器、不打开 DevTools。执行第 9.2 节全部步骤，并额外验证
原生凭据提示、SSE 短暂断连、关闭重启和 Session 恢复。

#### A5. 安全与证据 Gate

扫描 config、ProductStore、日志、trace、report、HTML/Markdown/JSON 导出和截图，
确认不存在 raw secret、Authorization header 或未脱敏 Provider 请求。检查 Workspace
边界、审批事实、最终 Git diff 和所有命令退出码。最终源码工作树必须干净；生成的
evidence、用户状态、installer、`target/` 和 `.next/` 必须位于忽略路径或仓库外。

A1-A5 全部通过后，A Gate 才是 PASS。任一项失败、跳过或没有可复查证据，本文
状态不得改为 `Implemented`。

## 11. 两个 Worktree 的实现所有权与集成顺序

两个 worktree 可以并行工作，但共享合同只有一个所有者：

先把本文档变更提交到 `main`，记录该提交为 `BASE_SHA`，再从同一基线创建：

```powershell
$baseSha = git rev-parse main
git worktree add ..\rove-tui-final -b feature/tui-real-use-final $baseSha
git worktree add ..\rove-desktop-final -b feature/desktop-real-use-final $baseSha

git -C ..\rove-tui-final merge-base --is-ancestor $baseSha HEAD
git -C ..\rove-desktop-final merge-base --is-ancestor $baseSha HEAD
```

两个 `merge-base` 命令都必须返回退出码 `0`。创建前如果同名 branch/worktree 已存在，
应先只读检查其归属和状态，不能自动删除或复用未知工作。

| Worktree | 独占或主要所有权 | 不得独立修改 |
|---|---|---|
| `feature/tui-real-use-final` | `models/`、`core/`、`runtime/`、`apps/bootstrap/`、`apps/cli/`、Provider runner 和 F/T 聚焦测试 | Desktop/Web 私有替代协议 |
| `feature/desktop-real-use-final` | `apps/desktop/`、`apps/web/`、Desktop Product API 投影、D 聚焦测试和 Windows package | Core Agent loop、Provider wire protocol、Tool Registry 权限 |

以下共享面在两个 worktree 建立前冻结：

- Provider Catalog schema 和 secret-reference 表达；
- ModelClient、native tool call 和 tool result history 合同；
- canonical StreamEvent 生命周期；
- Product Session/Run 精确绑定；
- Tool Registry、审批和 Workspace 边界。

如果 F/T 实现必须改变共享序列化、事件或 API：

1. TUI worktree 先形成一个只包含共享合同、默认值、迁移和合同测试的小提交；
2. Desktop worktree 立即合入该提交并更新消费者；
3. 两边不得各自发明兼容字段；
4. current runtime 文档在共享提交中同步更新。

集成顺序是强制的：

1. 先提交并合并本文档，两个 worktree 从同一提交创建；
2. 两边并行完成不依赖对方的实现；
3. TUI worktree 完成 F4 与 T7，收集 TUI 证据并先合并到 `main`；
4. Desktop worktree 合入最新 `main`，解决冲突并完成 credentialed D6 验收；
5. Desktop worktree 合并到 `main`；
6. 在最终 `main` 上重新执行 A Gate，并把本文状态一次性更新为 Implemented；
7. CI/测试门禁性能优化另开变更，不与本实现混入同一交付。

建议提交边界：

```text
test: record the credentialed native tool-call contract
feat(provider): add secure catalog and keyring onboarding
fix(models|bootstrap): complete the verified provider path
feat(tui): make provider onboarding recoverable
feat(tui): expose grounded harness activity
test: add the complete TUI golden-path evidence
feat(desktop): add native secure provider onboarding
fix(desktop): make installed provider and workspace startup reliable
feat(web): project exact harness evidence in desktop
test: record the installed desktop golden path
docs: publish demo runbook and evidence boundary
```

每个提交都应保留现有序列化、迁移和安全兼容性；不要用一次大提交同时重写 TUI、
Provider 和 Desktop。

## 12. 风险与停止条件

| 风险 | 处理 |
|---|---|
| 真实 Provider 不支持或不稳定地产生工具调用 | 更换已验证模型或修复 schema/prompt；不伪造工具事件 |
| Provider adapter history 不兼容 | 在 models 层修复并增加一个真实缺陷回归测试 |
| Planner/Finalizer 消耗过多调用 | 先记录实际调用和成本，再做最小策略调整 |
| TUI 隐藏错误或终端损坏 | 优先修复错误投影和 terminal restoration |
| Desktop 从开始菜单读取不到 secret | 使用共享 keyring/setup；不把密钥写进浏览器或配置明文 |
| Desktop 静态构建或 WebView 空白 | 修复 build path/asset/API readiness，不回退为开发服务器演示 |
| 测试再次膨胀 | 要求每个新增测试指向本规范中的验收条目或真实回归 |
| 演示成功但任务质量不稳定 | 不宣称产品完成，继续用固定真实任务测成功率 |

出现以下情况时，必须阻止本规范标记为 Implemented：

- 两个经过验证的主流模型都无法稳定完成最小只读工具任务；
- 为完成演示必须绕过 Tool Registry、Workspace 边界或审批；
- 只能通过硬编码 fixture 回答或 Fake 脚本通过；
- Desktop 必须持久化明文密钥才能工作；
- 黄金路径需要大规模新架构，而不是修复现有共享链路；
- 一周内仍无法获得第一条 credentialed native tool-call 证据。

## 13. 最终完成与文档收口标准

本规范完成后，项目可以准确声明：

> 在已验证的 Windows 环境中，Rove 使用硅基流动
> `deepseek-ai/DeepSeek-V3.2` 真实请求模型。用户可以安装 Rove，在任意
> 真实仓库中通过无参数 `rove` 进入 TUI，或通过已安装 Desktop 选择仓库；随后
> 完成连续对话、原生模型工具调用、Harness 执行、审批、结果回传、grounded final
> answer、diff、测试和状态恢复。所有证据均与 Fake/mock 测试分开。

完成时必须在同一变更中：

1. 将本文状态改为 `Implemented`，记录最终 merge commit 和证据目录；
2. 更新 `README.md` 的安装、TUI、Desktop 和真实 Provider 快速开始；
3. 更新 `docs/runtime/provider-smoke.md`、`implementation-guide.md`、
   `implementation-status.md`、`acceptance-matrix.md` 和 `release-readiness.md`；
4. 保留未验证平台与 Provider 的明确边界；
5. 记录所有检查的真实退出码和最终 `git status`；
6. 不把临时 evidence、secret、`target/`、`.next/` 或用户状态提交进仓库。

在 F4、T7、D6 和最终 A Gate 全部通过以前，不得使用“真实可用”“Desktop 完整
可用”或“已完成外部 Provider 互操作”等表述。
