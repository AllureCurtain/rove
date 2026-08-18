# 硬只读 Review 工作流计划

> Status: **Implemented in `feature/read-only-review`; pending merge and optional external gates**
>
> Date: 2026-08-16
>
> Base: `main` at `f6676d1` (PR #33, productization integration)
>
> Program: [下一轮产品化计划](2026-08-16-next-productization-round.md) 的并行任务，与
> [用户级运行数据目录迁移](2026-08-16-user-state-directory-migration.md) 从同一基线并行启动，
> 合并顺序在迁移之后。

Rove 是一个用 Rust 实现的本地 Agent 产品，CLI、TUI、HTTP/SSE API、Web、Desktop 和 benchmark 共用 Runtime、Engine、ToolRegistry、canonical events、Project Trust、Execution Environment、diff、Artifact 和证据边界。当前代码已有 Planner、Evaluator、Finalizer、Coding Tool V2、工具属性和审批基础，但还没有一个独立、可验证、真正禁止副作用的 Review 产品合同。

在一个为本任务单独创建的 worktree 中完成这项工作。开始前确认 worktree、分支、基线提交、工作目录和 `git status --short --branch`。基线必须来自已经审查并合入 main 的提交；不能读取、搬运、覆盖、清理或提交其他 worktree 的未提交内容。先以当前代码、测试、生成 OpenAPI 和 `docs/runtime/` 为事实来源，核对现有 diff、Artifact、Run Inspector、ToolRegistry、Execution Environment、Project Trust、审批和 Finalizer 行为。开始修改前在仓库根创建或追加 `IMPLEMENTATION_LOG.md`，记录基线、计划、失败和真实验证命令。

> Current implementation note (2026-08-18): the Runtime, ProductStore v14,
> API, CLI, Web, tests, and current-state documentation described below are
> implemented in this worktree. Interrupted API Reviews are conservatively
> classified as `needs_attention` rather than resumed automatically; the
> deterministic evidence and unrun external gates are recorded in
> `VERIFICATION.md`.

## 目标与边界

实现一个用户可以明确启动、查看、取消和复查的 Review 工作流。Review 针对代码或配置变更给出有证据的结构化 finding，但整个 Review 过程不能修改目标 workspace、Git index、工作树、项目配置、MCP 配置、memory、Provider 配置或任何 mutation-capable 外部资源。

Review 是共享 Runtime 上的一个受限运行 profile 和产品流程，不是第二套 Agent loop、Planner、事件生命周期、ToolRegistry、token authority 或持久队列。Review 结果是可重建的 derived projection；canonical events、task state、Artifact store、ProductStore canonical rows 和目标快照各自保持原有 authority。

首版至少支持：

- 当前 workspace 的未提交改动，明确区分 staged、unstaged、untracked、deleted、renamed 和 binary；
- 一个显式的 Git base revision 或 commit target，并记录解析后的 revision、工作树状态和 target digest；
- CLI 人类输出与 JSON/JSONL、真实 API contract，以及现有 ProductApp/Run Inspector 中的 Review 入口；TUI 只能复用同一命令和 API/Runtime contract，不得建立 TUI 私有 Review backend；
- 运行中的取消、断线重连、目标变更后的 stale/needs-attention、无改动、无 finding、部分结果、模型不可用和内部错误等真实状态。

不要把 Review 偷换成“运行一次普通 Agent 后展示最终答案”。用户必须能区分目标快照、分析过程、finding、未检查范围、证据缺失和 Review 本身未完成。

## 只读执行合同

先写一份可评审设计，明确 Review target、snapshot、run identity、权限、持久化、API、finding schema、取消/恢复和产品入口。Review 启动时生成不可变的目标描述和 digest；若分析期间目标工作树或 base 发生变化，结果标记 stale 并停止或要求重新取样，不能把旧 finding 静默归给新代码。

执行隔离必须在两个边界同时成立：

1. Review 专用 Execution Environment/工具目录只注册有明确只读语义的工具。读取文件、列目录、读取 Git diff/status、有限搜索、读取现有 Artifact 和必要的结构化元数据可以使用共享实现；写文件、删除、移动、覆盖、Git add/checkout/reset/commit、shell 任意命令、MCP 写工具、hooks、外部进程和网络副作用默认不可用。
2. 在 dispatch 前再次检查工具属性、capability snapshot、路径、参数、Project Trust 和 Review mode。模型输出、提示词、仓库 `AGENTS.md`、MCP annotation、URL、文件名和 approval 不能把只读工具变成可写工具；即使用户点击批准，Review 也不能获得写权限。

如果必须执行 shell，只有可证明的、参数化的只读命令 allowlist 可以进入 Review；不能以“模型通常只读”作为安全理由。拒绝、不可判断和 capability 缺失都要返回 typed failure，并记录到 Review 证据中。Review 可以把自身的 trace、task state、report 和 finding artifact 写入受控的 Rove state，但这些写入不能落到目标 workspace，也不能改变目标快照。

## Finding 与结果合同

定义带版本的 Review result schema。每个 finding 至少包含稳定 finding ID、review/run/target identity、严重程度、置信度、类别、相对路径、起止行/列（无法定位时明确说明）、标题、问题解释、有限证据片段、触发规则或来源、修复建议和状态。结果还要包含目标 digest、扫描文件/字节/耗时/并发上限、未检查或截断范围、模型/provider snapshot、tool/capability snapshot 和整体结论。

finding 是不可信模型输出，必须经过 schema 校验、路径和行号校验、字节限制、去重和脱敏。原始 system prompt、隐藏推理、完整 tool payload、凭据、Authorization、环境变量值和敏感文件内容不能进入 finding、snippet、日志、trace、report、导出或浏览器状态。二进制、超大文件、生成目录、忽略文件和外部路径按现有 workspace/sensitive-path policy 处理，并如实标记未检查。

整体结论至少区分通过、发现问题、部分完成、目标已变化、不可用、取消和错误；“没有 finding”不等于“所有文件都已检查”。CLI plain/json/jsonl、API、Web 和后续 TUI 都从同一结构化事实生成，不允许各自猜测严重程度或统计数字。结果可 inline 查看，也可作为有界 detached Artifact 引用，但不能新增第二份永久报告 authority。

## 运行、恢复与产品入口

复用现有 Engine、canonical events、Run/Session identity、Artifact、diff 和 Run Inspector。Review 的状态、取消、重试、resume 和最终化要能从 durable facts 重建；已完成的只读分析不能因恢复而重复写入或重复发布 finding。目标 digest 变化、Artifact 过期、历史缺失、Provider 断开和结果截断都必须显示具体状态。

CLI 提供与现有命令语法一致的 Review 入口和目标参数；API 提供版本化请求、状态、结果分页和 finding 读取；Web 在现有 Workspace/Session/Chat/Run Inspector 信息结构中提供可达入口，不制作营销页或平行工作台。主聊天保持简洁，Review 详情放进按需 Inspector/详情视图。界面覆盖 loading、empty、no findings、partial、stale、restricted、unavailable、cancelled、error 和 retry；长中文路径、窄窗口、键盘 focus、深浅主题和屏幕阅读器语义不能出现重叠、死按钮或静默跳转。

Review 首版不提供自动修复、批量应用 patch、自动 commit、自动 checkout、自动回滚或“发现问题后直接再次运行写工具”。修复是后续独立任务；Review 页面中的任何建议都只是文本和证据。

## 验证要求

先运行定向 Rust 测试和静态检查，再扩大到 workspace tests、Web test/typecheck/build、真实本地 API 和浏览器场景。Fake Provider 必须能生成确定的 finding、无 finding、格式错误和长输出；Fake 结果只证明本地合同，不代表外部 Provider 兼容。所有验证记录真实命令、工作目录、退出码、耗时和证据路径，不能用测试名称、源码正则或模型自述代替行为断言。

至少覆盖：

- staged/unstaged/untracked/deleted/renamed/binary/空 diff/base revision/commit target；
- 大 diff、中文和 Unicode 路径、长行、重复 finding、非法行号、超长证据和结果截断；
- 模型尝试写文件、删除文件、执行 `git reset`/任意 shell、调用 MCP 写工具、访问 workspace 外路径或利用提示词授予权限；
- Project Trust 受限、approval 策略变化、缺失 capability、Provider 不可用、取消、断线、重启、resume、Artifact 过期和目标 digest 变化；
- Review 前后工作树、index、项目配置、MCP 配置、memory、Provider 配置和目标文件的字节/digest 对比，证明没有目标副作用；
- 真实 CLI stdout/stderr/退出码、API/OpenAPI 字段一致、Web 刷新后状态恢复、finding 精确跳转和键盘操作；
- 多个 Review 并发时的 workspace 隔离、预算/超时/并发上限和重复请求幂等。

最终交付实现代码、当前 `docs/runtime/` 更新、`REVIEW_WORKFLOW.md`、`SUMMARY.md`、`IMPLEMENTATION_LOG.md`、`VERIFICATION.md` 和 `DIFF_SUMMARY.md`。文档必须说明目标合同、只读证明、finding 版本、脱敏、限制和未运行的外部 gate。清理临时数据库、日志、浏览器结果、Artifact、密钥和其他生成状态，不提交其他 worktree 内容。

## 本任务不做

不实现用户级运行数据目录迁移、managed worktree、后台任务中心、会话全文搜索、Context Inspector、插件市场、向量 RAG、LSP/语义代码图、多 Agent supervisor、自动修复或第二套事件/运行循环。需要的 state/path 能力通过公开接口复用；若接口不足，补最小兼容 contract，不在本 worktree 重写目录模型。

本任务与[用户级运行数据目录迁移](2026-08-16-user-state-directory-migration.md)并行时，拥有 Review target、只读 Execution Environment、finding schema、Review API/UI 和相关文档的修改权；不要复制或改写迁移 worktree 的路径解析。两条线都从同一个干净的 main 提交开始；若共享 bootstrap、API route registry 或 runtime assembly 文件必须修改，记录依赖并在合并时先接入目录解析的公开接口，再整合 Review。
