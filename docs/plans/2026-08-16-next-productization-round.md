# 下一轮产品化计划：入口、控制面与证据

> Status: **Proposed / Not Implemented**
>
> Date: 2026-08-16
>
> Base: `main` at `f6676d1` (PR #33, productization integration)
>
> 前置计划: [2026-08-10 post-full-delivery productization](2026-08-10-post-full-delivery-productization.md)
> ——其 Workstream A-E 与 F.1-F.3 已实现并合入 main，F.4/F.5 各余一小簇收尾项（见本文“收尾项”），
> G 的外部 gate 保持 opt-in。
>
> 并行计划: [用户级运行数据目录迁移](2026-08-16-user-state-directory-migration.md)（第一轮先行的地基任务）、
> [硬只读 Review 工作流](2026-08-16-read-only-review-workflow.md)（第一轮并行任务）。

实施将在为本计划单独创建的 worktree 和独立对话中进行。开始前先阅读仓库根 `AGENTS.md`、`docs/ONBOARDING.md`、`docs/runtime/README.md`、相关 current-state 文档、当前 main 分支代码与测试，再查看状态明确的设计或实施计划。必须以代码、测试、生成契约和 `docs/runtime/` 为当前事实来源，不能把 Proposed 文档当成已经实现的能力。

## 工作边界

- 开始前确认本任务使用的分支、worktree、基线提交和工作目录，执行 `git status --short --branch` 与 `git worktree list --porcelain`。
- 第一轮并行的 `user-state-migration` 与 `read-only-review` worktree 属于各自计划。不得修改、覆盖、清理、提交、合并或评审这些 worktree 中的未提交内容，也不得把它们的脏工作目录当成本任务基线。
- 只从经过审查并已合入 main 的提交获取其他任务成果（当前基线 `f6676d1`）。若前置成果尚未合入，应记录依赖并避免重复实现，不得擅自跨 worktree 搬运代码。
- 本文件是已提交的正式计划。实现代码、运行时文档和测试只能落在对应任务自己的 worktree 和分支中；对共享入口（bootstrap、API route registry、CLI 入口）的修改按“实施顺序与 Worktree 划分”的合并顺序进行。
- 不引入第二套 Agent loop、事件生命周期、持久化队列、工具注册表或 Web 专用运行时。CLI、TUI、API、Web、Desktop 和 benchmark 必须继续复用共享 Rust Runtime、Engine、ToolRegistry 与 canonical events。
- 不照搬 DeepSeek Harness 的 `Everything is a plugin`。可以借鉴其产品化、可检查配置、日志可重建约束和快照测试，但不能削弱 Rove 已有的安全、权限、持久化和恢复边界。

## 任务目标

把 Rove 从“能力已经很多但入口、配置和运行证据较分散”的状态，推进为一个默认可对话、命令简洁、配置归属清楚、Web 可完整观察和控制运行过程的产品。详细运行信息应随时可查，但不能继续挤占主对话区域；任何展示必须来自权威运行事实，而不是由 UI 从自然语言结果中猜测。

DeepSeek Harness 仍处于 developer preview（官方声明存在破坏性变更），其公开安装说明、Web/CLI 结构、Cordis 组合方式和 session-log 约束可作为参考，但本地 `npx` 启动此前没有验证成功，未实际验证的行为不能写成事实。Rove 自己的 Desktop D0 host 已通过 PR #30 合入 main，Windows MSI/NSIS 打包证据已记录，但 macOS/Linux 打包、签名与完整 installed-Desktop 旅程仍未验证。本轮仍优先完善 Rove Web 控制端，不开展 Desktop 专项改造。

## 必须覆盖的设计与实现

### 默认进入 TUI，并提供真正的一命令体验

- 用户在项目目录执行 `rove` 时，默认进入全屏 TUI，并能像 Claude Code、Codex 一样直接进行连续对话，不再默认进入行式 REPL。
- 保留明确的非交互入口，例如 `rove exec "task"`。旧 REPL 应经过兼容性审查后放到显式命令下，或正式退役。
- Rust 不限制一命令启动体验。应围绕已安装二进制、portable binary 或受控 launcher 设计首跑路径，不需要模仿 npm 分发机制。
- 为 Web 控制端定义一个简短、可记忆、能正确管理 API/Web 子进程生命周期的一命令入口；具体命令必须服从统一 CLI 语法设计。
- 同步更新 clap 解析、帮助文本、shell completion、README、onboarding、runtime 文档和入口行为测试。

### 重新设计 CLI 命令语法

- 先盘点现有顶层命令、参数、别名和重复能力，再定义小而稳定的顶层语法。
- 常用路径必须短且一致：`rove` 用于 TUI 对话，`rove exec` 用于一次性任务；session、config、trust、state、provider、MCP 和 diagnostics 使用一致的名词与动词规则。
- 不为了兼容而永久保留混乱语法。需要保留旧命令时，应设计明确、可测试的弃用或迁移路径。
- TUI slash commands、CLI 子命令和 Web 操作应共享同一产品概念，不能为每个界面发明不同术语。

### 解释并产品化 Project Trust

- 先用产品语言说明 Project Trust 的含义和后果：它控制项目配置、workspace instructions、MCP/进程定义、hooks/extensions、provider/credential selectors 和 external paths 等 workspace-owned 能力是否可激活。
- Project Trust 必须继续按规范化根目录和具体 capability 授权，默认受限、失败关闭，并在相关源内容变化后使对应授权失效。
- Project Trust 与工具审批是两套权限。信任项目不能自动批准破坏性工具调用，也不能让仓库文本扩大自身权限。
- CLI/TUI/Web 应展示相同的 trust 状态、capability、来源摘要、失效原因和实际影响，避免只暴露含义模糊的 `trust project` 文案。
- Web Settings 和统一诊断视图必须能解释“当前为什么受限、授予某项能力会启用什么、哪些能力仍未授权”。不得暴露密钥、原始敏感配置或不必要的绝对路径。

### 将用户配置与运行状态迁出项目目录

本节已拆分为独立计划
[用户级运行数据目录与旧 `.rove/` 迁移](2026-08-16-user-state-directory-migration.md)，
作为第一轮地基任务先行实施；本计划不重复其范围，只依赖其公开目录解析合同。

### 以 Web 为主要运行控制与观察界面

- 基于现有 `ProductApp`、ProductStore、API/SSE、`useServerProductState`、`useSessionContinuity`、Transcript、Composer、Settings 和 `RunInspector` 演进，不另建一套 Web runtime。
- 主聊天只保留用户、Agent 和必要交互信息。详细工具过程、指标、原始 ID、配置来源和证据进入可展开的 Inspector/Diagnostics，不默认淹没对话。
- composer、消息生命周期、队列与 streaming 跟随（F.1-F.3）已合入 main，本轮直接在其上构建；仍缺的 F.4 服务端 transcript 分页按“收尾项”一节归入本 workstream，不得另起实现。
- Web 必须清晰呈现运行中的模型步骤、工具调用、审批、用户输入、文件变更、计划、重试、降级、取消、恢复和最终化过程。这里的“清晰可见”是基于现有 canonical facts 的可查询投影，不是无限制记录内部推理，也不是泄露敏感数据。
- 如果现有 API 没有提供某项权威事实，应沿共享 contract 补齐 producer、persistence、API/OpenAPI、Web consumer 和 contract tests，而不是在前端推测。

### 建立比“工具三层分离”更广的权威分层

为整个执行过程定义并落实以下不同职责，明确每层的 source of truth、序列化边界、保留期限、脱敏规则和消费者：

- canonical event facts：实际发生的生命周期事实；
- durable resumable state：恢复所需的 task state、checkpoint、ledger、binding 和 immutable run snapshots；
- model-input projection：真正进入模型请求的 messages、headers、tool schemas、route 和动态上下文；
- UI projection：面向聊天、时间线、工具卡片、文件和差异的可读展示；
- diagnostics projection：解释当前 workspace/session/run 的有效配置、能力、健康和身份；
- finalizer/evidence/export projection：面向总结、审计和可分享证据的有界脱敏结果。

这些层可以引用同一权威事实，但不能互相冒充。`report.json` 不能取代 trace，UI 不能反解析模型文本来构造事实，diagnostics 不能变成第二份配置 authority，恢复状态也不能被展示层反向覆盖。工具现有的 canonical value、model/UI/finalizer/audit projections 应保留并纳入这个更广的分层模型。

### 增加统一、可解释的运行诊断视图

在 Web 中提供一个面向当前 workspace、session 和 run 的统一诊断入口，至少覆盖：

- 当前 provider profile、provider type、model、reasoning、route、fallback 和不可用原因；
- missing/deleted/dangling profile 的明确状态和恢复入口；
- approval policy 与 Project Trust 状态及逐 capability 授权；
- MCP server health、transport、catalog revision、降级状态和本次运行固定的 tool bindings；
- 内置工具、MCP 工具、AgentDefinition、procedures、instructions、hooks 和其他有效能力的组合清单；
- system/dynamic context、history、memory、compaction、prompt hash、tool signature 和 capability snapshot 等 prompt 组装事实；
- execution environment、runtime identity、workspace identity、session/run/job ID、预算、token/context/cost、步骤与事件计数；
- 计划步骤、模型调用、工具调用、变更、重试、错误、取消、恢复、降级和 finalization 的时间线与证据引用。

该视图应提供类似 DeepSeek Harness `dump-config`/Cordis 插件树的“当前有效组合为什么是这样”的解释能力，但 Rove 展示的是共享 Runtime 的配置、注册表、快照和来源，不以插件数量代替真实能力，也不把 Agent loop 改造成动态插件。

### 保持并说明 Provider 的正确边界

- 不要把当前 Provider 实现描述为错误，也不要因为参考 DeepSeek Harness 就重写现有 provider boundary。Rove 已有 provider-neutral protocol、profile、route、server-owned session selection 和 fail-closed 校验，应在这些基础上改进产品体验。
- 删除 profile 后解绑 session `profile_id`、保留既有 model snapshot，与“浏览器或 session 仍引用缺失 profile 时拒绝提交”是不同场景，诊断和文案必须准确区分。
- 重点补强 provider onboarding、模型发现、能力声明、连接测试、错误分类、缺失配置恢复和统一诊断，不允许静默回落到一个用户不知情的 provider/model。
- 原始 key 继续只存在于受控的用户/服务端 authority 中，不能进入浏览器状态、请求、日志、trace、report、截图或测试证据。

### 把模型输入可重建性升级为硬门槛

- 不得声称 Rove 当前完全不能重建，也不得声称已有 hash 就等于能够逐字重建全部 provider request。当前 canonical events、task state、profile snapshots、prompt metadata 和稳定 hash 是基础，需要补上可验证的完整链路。
- 定义“model-visible”范围，至少包括 system/lifecycle prompt、workspace instructions、AgentDefinition/procedure 内容、memory、history、compaction summary、用户输入、steer/follow-up、tool results、tool schemas、provider route/model/options 和动态 context。
- 每次模型调用必须能由 durable facts 或显式记录的不可变脱敏快照重建其语义等价输入，并能用稳定 hash 证明一致；无法安全持久化的内容必须有明确的不可重建标记和失败策略，不能假装完整。
- 设计 secret、超大内容、二进制 artifact、外部资源、短期凭据和 provider-specific payload 的存储边界。Provider 私有 wire payload 仍留在 provider boundary，不因可重建要求泄露敏感数据。
- 增加 assembled-application 级快照测试，覆盖 prompt sections、history/compaction、selected provider/model、tool schemas、capability snapshot、Project Trust、approval policy、request header、关键 UI diagnostics 和恢复后的再次组装。
- 继续以 fake provider、确定性 benchmark、API/Web contract tests 为默认无密钥证据；外部 provider、真实 MCP 和真实浏览器仍是明确 opt-in gate，未运行不能宣称通过。

### 只讨论并保护 Rove 已经具备的优势

在正式设计中增加一段基于当前代码和测试的比较，说明 Rove 相对 Claude Code、Codex、OpenClaw、Hermes、Pi 和 DeepSeek Harness 已经具备的差异化优势。只写已实现且有证据的内容，例如共享 Rust Runtime、跨 CLI/API/Web/TUI/Desktop 的统一生命周期、canonical durable events、精确 resume、安全审批与 Project Trust、统一 ToolRegistry、local deterministic execution、可审计 artifacts/evidence，以及独立 Finalizer 和执行预算。

这部分只用于确定架构约束和产品定位，不为“对比”本身新增功能，不虚构其他产品能力，也不把尚未实现的计划包装成现有优势。Pi 等项目中已经吸收的 Provider/Agent 设计应准确说明继承关系，不得因为新的参考对象而推翻经过验证的边界。

## 收尾项：2026-08-10 计划遗留的 F.4/F.5 缺口

以下缺口已在基线核验中确认，随本轮对应 workstream 一并完成，不单独立项：

- F.4 服务端 transcript 分页（归 Web 控制与观察 workstream）：`ProductTranscriptResponse` 增加有界 cursor/上一页语义与按需 prepend，前端恢复从单次 `getTranscript` 改为可续拉；补充加载旧历史与滚动锚定的行为测试。
- F.5 TUI 进程重启恢复（归入口与命令 workstream）：TUI 启动时扫描会话队列，排空可领取的 FIFO successor，并对已 `claimed_successor` 但没有对应 run 的消息与 runtime run index 对账。

## 实施顺序与 Worktree 划分

本计划不要求单一巨型分支。拆为两轮 worktree，任何时刻并行不超过三个，控制 bootstrap、API route registry 和 CLI 入口等共享文件的合并冲突。

### 第一轮：地基（两个并行 worktree）

| Worktree | 分支 | 计划 | 范围 |
|---|---|---|---|
| `.worktrees/user-state-migration` | `feature/user-state-migration` | [用户级运行数据目录迁移](2026-08-16-user-state-directory-migration.md) | 目录解析合同、幂等迁移、repair/cleanup、所有生产入口的默认路径切换 |
| `.worktrees/read-only-review` | `feature/read-only-review` | [硬只读 Review 工作流](2026-08-16-read-only-review-workflow.md) | 只读 Execution Environment、finding schema、Review API/UI |

两条线从同一个干净的 main 提交开始（当前 `f6676d1`）。合并顺序固定：先合迁移，Review 在迁移合入后 rebase 到新 main 再合并。这与两份任务计划中的并行边界约定一致：Review 通过公开接口使用目录解析，不复制路径逻辑。

### 第二轮：产品化主线（迁移合入后，最多三个并行 worktree）

| Worktree | 分支 | 覆盖范围 |
|---|---|---|
| `.worktrees/product-entry` | `feature/product-entry` | 默认 TUI 与一命令体验、CLI 命令语法重设计、Project Trust 产品化入口，以及 F.5 的 TUI 进程重启恢复收尾 |
| `.worktrees/product-web` | `feature/product-web-observability` | Web 主控与观察界面、权威分层落地、统一诊断视图，以及 F.4 的服务端 transcript 分页收尾 |
| `.worktrees/model-input-rebuild` | `feature/model-input-rebuild` | 模型输入可重建性硬门槛：model-visible 范围定义、快照测试与 hash 一致性 |

依赖关系：

- `product-entry` 依赖迁移后的目录合同（状态目录、completion、安装路径）；
- `product-web` 依赖迁移后的 ProductStore 位置与已合入的消息生命周期（F.1-F.3）；
- `model-input-rebuild` 主要落在 runtime 与测试，可与前两者并行；若触及相同的 provider/context 文件，先合 `product-entry` 与 `product-web`。

### 不进入本轮的事项

- Agent Loop 可替换与第三方 Loop 插件：明确排除。`ExecutionStrategy` 维持封闭枚举，run 内确定性优先。
- 面向第三方的扩展产品层（Pi packages 式自动发现/分发/启停体验）：本轮不做，作为本轮完成后的独立评估项；AgentDefinition 能力包与 MCP 继续作为既有扩展边界，插件请求能力、不能自我授权的约束不变。
- Desktop 专项改造与 macOS/Linux 打包、签名、installed-Desktop 旅程：保持 2026-08-10 计划 G 类 opt-in gate，不阻塞本轮。

## 兼容性与实施要求

- 先给出一份可评审的中文设计，明确当前行为、目标行为、数据归属、迁移、API/事件影响、安全风险和分阶段实施顺序，再开始大范围代码改动。
- 任务跨度较大，应拆成依赖清晰、可独立验证的阶段或 PR；但所有阶段必须服从同一最终目录模型、命令语法和诊断 contract，不能留下两套长期 authority。
- 修改 serialized types、API 字段、events 或持久化格式时，必须给出 defaults、版本、迁移、旧数据读取和负向测试。
- 涉及路径、配置、工具、provider、MCP、state 或 Web 时，逐项检查输入大小、路径边界、超时、并发、重试副作用、取消、恢复重放、secret redaction 和失败可见性。
- 不新增依赖，除非现有 Rust/TypeScript 栈无法合理完成目标，并在设计中说明原因。
- 同一实现变更同步更新 `docs/runtime/` 当前合同；未来目标留在 `docs/design/`，实施步骤放在 `docs/plans/`，不得提前把 acceptance/status 标成已满足。

## 验证与交付

按风险从小到大运行定向测试、格式检查、lint、Rust workspace tests、Web test/typecheck/build，以及受影响的 API/Web E2E。状态目录迁移必须有旧数据、冲突、损坏、权限失败、幂等重试和精确 resume 测试；默认 TUI 与命令语法必须有 clap/入口测试；统一诊断必须有 API contract、脱敏和浏览器可见性测试；模型输入可重建性必须有组装快照和 hash 一致性测试。

最终交付应包含实现代码、当前运行时文档、正式设计与实施记录、迁移说明、测试证据和明确的已知风险。不得提交临时产物、生成状态、真实密钥、截图中的敏感信息或其他 worktree 的内容。
