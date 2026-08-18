# 用户级运行数据目录与旧 `.rove/` 迁移计划

> Status: **Implemented**
>
> Date: 2026-08-16
>
> Worktree baseline: `5fe9d70` (based on main `f6676d1`, PR #33)
>
> Program: [下一轮产品化计划](2026-08-16-next-productization-round.md) 的地基任务，与
> [硬只读 Review 工作流](2026-08-16-read-only-review-workflow.md) 从同一基线并行启动。

截至 2026-08-18，本 worktree 已实现本计划的用户 data-root 解析、workspace
identity/marker、默认 state/memory/MCP 接线、API-global ProductStore 路径、
幂等 dry-run/apply/prune 迁移、SQLite 索引路径重定位、冲突备份、并发锁、
prepared journal/receipt、MCP 首次写入的 legacy promotion、CLI 命令、关键
入口测试和全部交付文档。F.4/F.5
产品历史窗口、外部 Provider/真实第三方 MCP、Desktop 安装及 macOS/Linux
发布证据不属于本次实现，仍保持未验证或未来工作。下面的目标和场景清单
保留为验收合同；当前事实以 `docs/runtime/`、
`STATE_LAYOUT_AND_MIGRATION.md` 和 `VERIFICATION.md` 为准。

Rove 是一个用 Rust 实现的本地优先 Agent 产品，CLI、TUI、HTTP/SSE API、Web、Desktop 和 benchmark 共用 Runtime、Engine、ToolRegistry、canonical events、状态存储、memory、Artifact 和安全边界。

在一个为本任务单独创建的 worktree 中完成这项工作。开始前确认 worktree、分支、基线提交、工作目录和 `git status --short --branch`。基线必须来自已经审查并合入 main 的提交；如果基线分支或本任务 worktree 有未提交改动，先记录依赖和原因，不要把脏工作目录当成基线，也不要读取、搬运或清理其他 worktree 的未提交内容。

先以当前代码、生成契约和 `docs/runtime/` 为事实来源，核对现有 `UserConfigPaths`、`AppConfig`、`Workspace`、Runtime state、memory、ProductStore、Project Trust、CLI/API/作用域绑定和 Desktop 路径。设计文档中写成 Proposed 的内容不能当作已实现。开始修改前在仓库根创建或追加 `IMPLEMENTATION_LOG.md`，记录基线提交、环境、计划和真实命令；上下文整理后先读这份日志再继续。

## 目标

把项目目录中的 `.rove/` 从“所有配置、运行状态和生成产物的默认容器”改成清晰、可解释的边界。默认情况下，用户运行数据应进入跨平台、可发现、可测试的 Rove 用户数据目录，并按规范化 workspace identity 隔离；项目目录只保留确实需要随项目共享、且受 Project Trust 约束的最小项目配置。

这不是简单地把 `state_dir` 的默认字符串换成另一个路径。我要得到一套统一的目录解析、数据归属、旧数据迁移、冲突处理、恢复和 repair 合同，所有入口都使用同一个解析结果，不产生第二份 state、memory、trust 或 ProductStore authority。

## 数据归属与目录合同

先写一份短的设计记录，列出当前每一种路径的 owner、authority、生命周期、敏感级别、迁移策略和消费者。至少逐项覆盖：

- 用户配置、Provider catalog、profile 和 credential reference；
- ProductStore 及其 workspace/session/preferences 映射；
- workspace identity、state SQLite、runs、trace、task state、report、checkpoint、tool artifacts 和 evidence；
- session memory、durable memory、memory index；
- Project Trust authority、临时 trust、approval 与 input 的进程内状态；
- `.rove/config.toml`、`.rove/mcp_servers.json`、`.env`、AgentDefinition、`AGENTS.md` 和其他项目来源；
- cache、锁、临时文件、日志、导出和一次性 benchmark 产物。

不能盲目搬走整个 `.rove/`。项目配置和 MCP 启动仍须服从现有 Project Trust 规则；信任迁移不能自动授予 capability；进程内 approval/input 不能伪装成可恢复的 durable authority。对每个“继续留在项目内”“迁移到用户目录”“只在本次运行存在”的决定给出代码和测试依据。

实现一个共享的跨平台路径解析合同，供 CLI、API、Web job、TUI、Desktop、benchmark 和 embedding 使用。它至少应具备：

- 明确区分 config、data、state、cache、workspace-owned 和 run-owned 根；
- Windows、macOS、Linux 使用现有平台约定，不能写死用户名或只支持 Windows；
- 为测试和嵌入提供显式、绝对路径的环境变量或配置 override，并拒绝相对路径和含糊的 home 解析；
- 以规范化 workspace root、可靠的大小写/符号链接或 reparse-point 规则和稳定 digest 生成隔离身份；同一 workspace 的不同入口必须得到同一目录，不同 workspace 不能碰撞；
- 所有路径在读写和迁移边界再次验证，不能由 HTTP、Provider、MCP 或模型提供的字符串逃逸 workspace 或用户根；
- 目录创建、锁、权限和临时文件行为有界且可解释，敏感文件采用现有安全权限策略；
- trace、report、API、Web、日志和错误中不出现原始 key、Authorization、环境变量值、短期凭据或未脱敏的用户绝对路径。

## 迁移与恢复

为已有项目提供幂等迁移。迁移必须先能 dry-run，显示版本、来源、目标、文件分类、冲突、预计字节数和风险，不写入文件、不打开写事务、不启动 Provider/MCP、不改变 Trust，也不调用模型。正式迁移要有明确的版本和 journal/marker，使用原子替换或可恢复的阶段记录；进程在任意阶段退出后再次运行，不能重复事件、重复 memory、重复 Artifact 或重复 ProductStore 记录。

至少处理这些情况：

- 新项目首次启动；
- 只有旧 `.rove/`、只有新用户目录、两边都有数据；
- 目标已存在且内容相同、内容不同、目标被锁定或权限不足；
- 部分文件已经搬运、临时文件残留、journal 损坏、SQLite schema 较旧或高于当前版本；
- runs/trace/task state 中存在可恢复和已完成的运行；
- memory、Artifact、MCP 配置、Trust 记录和 ProductStore 映射分别发生冲突；
- workspace 被移动、重命名、通过 symlink/reparse point 访问或无法 canonicalize；
- 两个进程同时执行迁移、迁移中断后 repair、用户明确拒绝迁移或选择保留旧数据。

冲突不能静默覆盖。提供保留源数据、备份/隔离冲突、重试和人工决定的路径；失败关闭时旧数据仍可读，不能留下“报告成功但 resume 找不到事实”的半状态。迁移完成后旧目录的处理必须显式记录，不能无确认删除用户数据。`cleanup`、`repair`、`rollback` 与迁移是不同动作，不能用一个含糊的“fix”按钮代替。

提供与现有 `state`/diagnostics 语法一致的 CLI 入口，至少能查看解析后的路径和 workspace identity、执行 dry-run、执行迁移、查看 journal/冲突并请求 repair。需要 HTTP/API 或 Web 展示时复用共享结构化 contract；不要为 Web 另造路径判断。JSON stdout 必须干净，日志走 stderr，错误和退出码稳定。没有合适的现有命令时，先写命令兼容设计和迁移说明，再实现最小、可测试的新增命令。

## 运行时兼容

把解析结果接入所有真正的生产入口，而不是只改 CLI 默认值：

- CLI 的 exec、REPL、TUI、session、state、trust、memory、MCP 和 benchmark；
- API 启动、ProductStore、job/workspace binding、SSE、resume、repair 和导出；
- Web 默认 ProductApp 通过 API 使用服务端解析结果，浏览器不能决定本地路径；
- Desktop 的 API host、平台目录和 clean-install 路径；
- Runtime embedding 和 deterministic fake-provider 测试。

旧配置字段、序列化类型和 SQLite schema 要有默认值、版本、向后读取、迁移和负向测试。已有项目配置仍能选择受支持的 profile/model 和受信任的项目来源；迁移不能改变 Provider snapshot、canonical event identity、ToolRegistry snapshot、approval policy 或精确 resume 语义。运行中已完成的副作用永远不能因目录切换而重放；未知外部副作用必须显示为未知并保守处理。

## 验证与交付

先做定向测试，再按风险扩大到 Rust fmt、clippy、workspace tests、Web test/typecheck/build、受影响的 API/Web E2E 和 Desktop/CLI smoke。验证必须真实启动入口并记录退出码、工作目录、环境 override、耗时和关键输出；不能用源码字符串、文件存在或模型自述代替行为断言。

至少加入隔离目录测试和 Windows 可复跑脚本，覆盖：fresh layout、legacy discovery、dry-run 无副作用、冲突、权限失败、损坏 SQLite、部分迁移重试、并发锁、取消/中断恢复、workspace identity 隔离、symlink/reparse 边界、旧 run 精确 resume、secret-free trace/report/API/Web、cleanup/repair 和重启后的重复运行。测试结束清理临时数据库、用户目录、日志、锁和生成状态。

最终交付实现代码、当前 `docs/runtime/` 更新、`STATE_LAYOUT_AND_MIGRATION.md`、`SUMMARY.md`、`IMPLEMENTATION_LOG.md`、`VERIFICATION.md` 和 `DIFF_SUMMARY.md`。文档只写真实行为；未运行的外部 Provider、MCP、Desktop 安装或平台 gate 明确写为 Not Run/Unverified。不得提交临时状态、密钥、真实用户路径、其他 worktree 内容或大体积无引用日志。

## 本任务不做

不实现会话全文搜索、Context Inspector、Review、managed worktree、后台任务中心、插件市场、向量 RAG、第二套 Agent loop、第二份事件/队列/state authority 或“迁移后自动授权”的快捷路径。若发现这些能力的依赖，记录接口边界和后续建议，不在本 worktree 顺手扩张。

本任务与[硬只读 Review 工作流](2026-08-16-read-only-review-workflow.md)并行时，拥有路径解析、迁移、state/memory/ProductStore 目录合同和对应当前文档的修改权；Review worktree 通过公开解析接口使用它，不复制路径逻辑。两条线均从同一个干净的 main 提交开始；若共享 API 注册或 runtime assembly 文件必须修改，保持增量、记录冲突，并在合并时先整合本任务的目录合同再整合 Review。
