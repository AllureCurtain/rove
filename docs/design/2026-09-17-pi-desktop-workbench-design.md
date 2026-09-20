# 借鉴 open-vetta 与 PI-Desktop 的 rove 对话工作台设计

> 状态：**Partially Implemented**。左栏、中栏输入、右栏工作面板、动效规范已按本文实现并有测试证据；本地可执行 HTML 预览（§7）未实现，仅交付威胁模型；任一参考项目的实机点击验收仍未完成。
> 日期：2026-09-17；2026-09-19 按用户确认修订分栏参考来源；同日二次修订，补记 rove 既有能力证据（会话 diff、偏好合同、审批落库、状态更新通道）与授权历史归属、输入请求契约、UI 偏好口径、文案/动效约束；同日三次修订：新增 §2.4 已有能力对比判定与 §3.3 动效与微交互规范。
> 本文的调研事实与目标设计分开记录；本文描述的目标设计中未完成的部分不应被读作已实现，也不代表已完成任一参考项目的实机点击验收。逐阶段实施证据与未关闭项见[实施计划](../plans/2026-09-17-pi-desktop-workbench-implementation.md)与 [P0/P1a 证据报告](../plans/2026-09-17-pi-desktop-workbench-p0-evidence.md)。
>
> **已确认方向（2026-09-19 更新，取代 2026-09-17 的"左栏参考 PI-Desktop"决定）**：
>
> | 分栏 | 参考来源 | 说明 |
> |---|---|---|
> | 左侧项目/会话导航 | **open-vetta**（全面参考） | 取代原"左栏参考 PI-Desktop"的决定 |
> | 中间对话区 | **open-vetta 与 PI-Desktop 混合** | 输入区结构与视觉参考 open-vetta；键盘/IME/草稿/提交竞争契约沿用 PI-Desktop 调研结论 |
> | 右侧工作面板 | **PI-Desktop**（维持不变） | 待处理、文件、变更、浏览四页签 |

## 1. 结论与范围

建议借鉴 open-vetta 与 PI-Desktop 将对话、操作控制和工作结果放在同一工作区的组织方式，保留 rove 的 Runtime、ProductStore、API/SSE 和 Tauri 宿主，不移植另一套 Agent 循环、权限系统或状态框架。

本次目标是给出可讨论、可拆任务、可验收的方案，未获确认前不修改产品代码。优先级为输入体验、右侧审批、文件浏览、审批与变更证据、浏览器。完整的竞品实机调研尚未完成，不能将本草案视为全量体验验收报告。

| 用户关注点 | 建议落地方式 | 关键约束 |
|---|---|---|
| 对话框更优雅 | 收敛输入框层级，明确发送/换行、模型入口、停止与运行状态 | 先保留 textarea，不为外观直接引入 contenteditable |
| 右侧审批框 | 右侧工作面板展示当前待审批详情，消息流保留关联入口 | 同一审批记录、同一提交动作，不能生成第二套权限状态 |
| 当前工作区文件 | 文件页签按需展开目录，点击只读预览 | 工作区边界、分页、取消过时请求；文件不是自动授权的上下文 |
| 当前对话审批和修改 | 分开显示授权记录、工具执行结果、会话变更、工作区 Git 差异 | 批准不等于执行成功，Git 差异不等于本会话造成 |
| 浏览器正确打开 | 分别设计系统外链、本地 HTML 预览、可选内嵌浏览器 | Web 与 Tauri 能力不同，失败要可见，不把 iframe 当通用浏览器 |

## 2. 本次证据与限制

### 2.1 仓库基线

- rove 调研基线为 `7e54ab6`，开始调研及文档写入前工作区干净。
- 参考目录一为 `D:\Study\project\agent\third-party-agents\PI-Desktop`。
- 远端为 `https://github.com/vastsa/PI-Desktop.git`，分支为 `main`。
- 已执行 `git pull --ff-only`，由 `f61dc1dd` 更新到 `8d826433`；更新后工作区干净。
- "最新"只指该次拉取观察到的远端 main，不保证此后远端没有新提交，不等同于稳定发行版。
- 参考仓库 package.json 标示版本 `0.14.9-beta.1`。
- 参考目录二为 `D:\Study\project\agent\third-party-agents\open-vetta`（2026-09-19 新增），本地快照提交 `19b3e712`（2026-09-18 写入工作区，本次未执行 `git pull` 更新）；下文相对 open-vetta 根目录的引用固定到该快照。

### 2.2 已直接核实的事实

参考路径以下均相对于上述 PI-Desktop 根目录，固定到该提交阅读。

| 证据 | 能证明什么 | 不能证明什么 |
|---|---|---|
| PI `README.md` 52–98 行 | 产品强调项目、会话、文件、review、preview、模型和权限在一个工作区 | 不证明每项功能无缺陷 |
| PI `AGENTS.md` 94–127 行 | Renderer → Preload IPC → Electron Main → Rust Host / Node Agent Runtime 的架构边界 | 不能照搬为 rove 的 Tauri/HTTP 实现 |
| PI `apps/desktop/src/features/chat/composer/ComposerInput.tsx` 96–135 行 | IME 确认不发送；补全键盘选择；Shift 换行及可配置 Enter、Ctrl/Meta+Enter 发送分支 | 不证明跨平台输入法实测均通过 |
| PI `apps/desktop/src/features/chat/composer/hooks/useComposerSubmit.ts` 117–169 行 | 异步增强结果检查请求、会话草稿键及版本，避免覆盖过时草稿 | 不应据此引入自动提示词增强或新的付费请求 |
| PI `package.json` 23–35 行 | 有 composer paste/autocomplete、transcript、layout、supervision 等测试入口 | 本次没有执行这些测试 |
| rove [Composer](../../apps/web/chat/Composer.tsx) | 本地 message/submitting 状态；成功发送才清空；发送、停止、模型及 Review 入口 | 当前没有同等 Enter/IME 专用键盘契约；不能据此断言已有缺陷 |
| rove [Composer 测试](../../apps/web/chat/Composer.test.tsx) | busy/disabled 下停止按钮及发送按钮的静态渲染断言 | 不覆盖真实点击、IME、发送竞争或草稿切换 |
| rove [Runtime 索引](../runtime/README.md) | 已有持久会话、审批、证据、Review 和 Tauri D0；长历史与部分平台验收仍有缺口 | 不应把这些已有能力列为"全部缺失" |
| rove [SessionRecord](../../apps/web/state/product-types.ts) 与 [contracts.rs](../../apps/api/src/product/contracts.rs) 996–1100 行 | 会话服务端已支持：`title`、状态投影（idle/running/error/needs_attention）、**重命名与归档**（`UpdateProductSessionRequest { title, archived }`）、**标题子串搜索 + 游标分页**（`search`、`next_cursor`、默认页 50/上限 200）、归档排在活动会话之后；**工作区级置顶**（`ProductWorkspace.pinned`）已有；**会话级置顶字段不存在** | 会话置顶、会话标签、scheduled 图标在 rove 无对应字段，相关 UI 要么走合同扩展阶段，要么不做假按钮 |
| open-vetta `apps/desktop/src/renderer/root-layout/RootLayoutView.tsx` | SidebarDock + SidebarOverlay（窄屏悬浮）+ MainContentFrame 布局；`data-sidebar-*` 属性驱动 CSS 自适应，侧栏折叠不重渲染路由内容树 | 不证明同等性能结论适用于 rove 的 shell |
| open-vetta `domains/project/components/sidebar/`（Sidebar.tsx、DefaultSidebar.tsx、sidebar-nav-layout.ts） | 左栏为 顶部 TopBar → 导航项（置顶区/收纳区，「新会话」锁首位、最多 5 项、布局持久化并带版本迁移）→ 项目分组（每项目内会话分页展示、置顶、内联重命名、右键菜单）→ 底部 BottomBar（设置菜单、消息中心、更新提示） | 不照搬其 jotai atom、插件目录与主题区域机制；持久化键是 localStorage 而非服务端合同 |
| open-vetta `sidebar/projects/ProjectGroup.tsx`、`useProjectGroupModel.ts`、`services/sidebar-session-order.ts` | 项目内会话排序为「置顶（按置顶时间倒序）在前，其余按修改时间倒序」；默认显示 5 条但**置顶项永远全部可见**（折叠计数 = max(5, 置顶数)），「显示更多」按钮带剩余条数文案；行视图引用稳定化（reuseUnchangedSessionViews）保证 memo 生效 | rove 首批的排序与分页见其 §3.1 映射行（updatedAt 倒序、默认 5 条）；置顶字段缺失时的降级见 §3.1 |
| open-vetta `theme-ui/project/SessionRowView.tsx`、`sidebar/SessionStatusIcon.tsx` | 会话行是一个 `<button>`：leading 图标按 运行中（旋转 refresh 图标）→ 定时（时钟图标）→ 普通（会话气泡图标）优先级；pinned 在图标位前加 pin 图标；forked 有分叉图标；激活态 `bg-accent + font-semibold`，非激活 hover `bg-accent/50`；行高 `py-[6px]`、缩进 `pl-[30px]`、标题 13px truncate；renaming 时整行就地换成输入框，点击不再触发选择 | rove 的"待审批"状态在 open-vetta 行模型里没有对应图标，需要自行扩展一个状态位，不冒充其设计 |
| open-vetta `theme-ui/project/ProjectRowView.tsx` | 项目行三段：展开/收起按钮（图标与箭头互换 + 运行脉冲点）、名称按钮（未展开时先展开再导航）、尾部徽标（hover 时被新建按钮替换）；新建按钮 `stopPropagation`，不冒泡成项目导航 | 「点击名称=导航到项目详情页」在 rove 没有项目详情页，映射为「选中工作区」，差异需在 P0 确认 |
| open-vetta `useSessionContextMenuModel.ts`、`theme-ui/project/SessionContextMenuView.tsx` | 会话右键菜单固定序：置顶/取消置顶 → 重命名（可隐藏）→ 附加项（标签子菜单）→ 在文件夹中显示 → 删除（danger，可隐藏）；权限位 `access.rename/delete` 控制显示 | rove 无标签体系、无"在文件夹中显示"宿主命令的 Web 对应物；删除对应归档，菜单项按现有能力裁剪 |
| open-vetta `useSidebarModel.ts` | 侧栏宽度 180–400px 拖调、localStorage 持久化、拖拽期间直写 DOM 避免整树重渲染；`ResizeHandle` 用 8px 命中区 + 拖拽期间全屏 overlay 兜住指针事件 + rAF 合帧 | 性能数字是该仓库实测，不作为 rove 的指标承诺 |
| open-vetta `SidebarCommandMenuTrigger.tsx` 与 `useSessionSearch.ts` | 顶栏查找是命令菜单入口（主快捷键 ⌘K）；会话搜索请求走宿主、180ms 防抖、流式分批合并结果、报告 limited/skipped | rove 首批不做全局命令菜单；查找映射为「过滤当前列表 + 可选服务端 `search` 参数」，见 §3.1 查找契约 |
| open-vetta `settings-menu/useSettingsMenuModel.ts`、`SidebarBottomBar.tsx` | 底栏 = 设置菜单（账号/主题三段切换/配额/IM 状态/打开时顺带触发更新检查）+ 消息中心按钮 + 更新横幅 | rove 底栏只保留设置入口与连接状态；主题切换、账号、配额、IM、更新横幅均无对应能力，不进首批 |
| open-vetta `apps/desktop/DESIGN.md` | Token 化颜色、1px 线条、卡片 0 阴影、按钮统一走基础组件等硬性 UI 规范，可作为 rove 视觉收敛的参考纪律 | 规范参考思路，数值按 §3 的 rove 令牌；不引入其 tailwind/iconify/theme-sdk 依赖栈 |
| open-vetta `domains/conversation/components/`（ChatView、chat-view/DefaultChatView.tsx、input-bar/） | 中栏结构：`DefaultChatView` = 消息列 + 可选活动列；`ChatComposer` 固定底部；InputBar 为 `MessageInput.Root/Surface/Content/Toolbar(Leading|Trailing)` 组合件，卡片化输入区（限宽居中、@container 按输入区宽度折叠工具行文案），工具行左簇=能力按钮（技能/@/附件），右簇=模型选择器→上下文环→语音→发送；下沿 Footer 插槽（todo/语音状态）用 `grid-rows: 0fr↔1fr` CSS 过渡；SendButton 有 send/stop 双态与 pending 文案态，运行中且草稿非空时主按钮变成「排队」按钮 | 附件、@ 提及、语音、技能、执行模式、todo 环 rove 均没有对应服务端合同，参考结构不参考功能全集；「排队」语义以 rove 服务端 Send Message 返回为准 |
| rove [diff.rs](../../apps/api/src/product/diff.rs) 21–25、82–98、151–162、256–278 行（路由注册 [lib.rs](../../apps/api/src/lib.rs) 286 行） | 已有 `GET /product/sessions/{session_id}/diff`，scope 为 run/git/all：run 作用域取 report.json 规范化工具变更，git 作用域跑有界 `git status --porcelain` 与 `git diff HEAD`；上限 4096/512 条、单条 128 KiB、总量 4 MiB、git 10s 超时，二进制不返回内容，超限进 partial_reasons | 基线固定为 HEAD、无采样时间字段；§6.2 的变更页签应复用它，不新建第二套 git 命令面 |
| rove [routes.rs](../../apps/api/src/product/routes.rs) 679–718 行 与 [contracts.rs](../../apps/api/src/product/contracts.rs) 967–980 行 | 已有 `GET/PUT /product/preferences`（expected_revision 冲突返回 409），偏好为类型化字段（theme、default_approval_policy、active_workspace_id、active_session_id、provider_selection） | 无自由键值通道，不能直接承载侧栏宽度等布局偏好；布局值首批走 localStorage UI 偏好 |
| rove [rove-client.ts](../../apps/web/lib/rove-client.ts) 172–179 行 与 [use-server-product-state.ts](../../apps/web/state/use-server-product-state.ts) 392–394 行 | 活动回合走 `GET /jobs/{job_id}/events` SSE（支持 after 重放）；会话目录在存在 running/needs_attention 会话时按 2.5s 轮询 `GET /product/sessions` | 轮询是条件触发，其他窗口新启动运行的可见性受该条件约束；左栏不得另建第二套事件流 |
| rove [state/index.rs](../../runtime/src/state/index.rs) 100–111 行 与 [lib.rs](../../apps/api/src/lib.rs) 323–324、1011、1059 行 | 审批决策状态落 runtime StateStore `pending_approvals`（call_id/job_id/run_id/name/args/reason/status/created_at/updated_at，决策原地更新 status）；job 域有 approvals/inputs、product 域有 controls/steers/followups 提交面 | 表内无"谁决策"字段，产品域无历史查询端点；授权历史展示须走 §5.1 与 P4 的合同扩展流程 |
| open-vetta `renderer/shared/hooks/useRunningSessionsSync.ts` 17–31 行 与 `shared/store/running-sessions-atoms.ts` | 会话运行状态来自根级初始快照 + 宿主事件增量订阅（RUNNING_CHANGED）写入 jotai atom，非轮询；其行模型无"待关注"指示 | rove 以 SSE+条件轮询达成同类效果，机制不同，不照搬其宿主事件通道 |
| open-vetta `apps/desktop/DESIGN.md` 141–165 行 | 动效纪律：hover 只反馈不表演、进入动画 ≤0.5s、禁止装饰性持续旋转（loading spinner 除外）、禁 transition-all | 数值是该仓库自己的规范，rove 按 §3 约束与既有 prefers-reduced-motion 实践裁剪 |
| PI `stores/slices/work-panel-slice.ts` 24–48 行 与 `lib/pending-permissions.ts` 8–22 行 | WorkPanel 只持久化宽度（localStorage 键 `pi.desktop.workPanel`），页签与激活页签为内存态；审批只有实时队列，决策后出队，无历史记录 UI | rove 右栏首批同样只持久化宽度（§5.0）；授权历史分区是 rove 扩展，不冒充 PI 设计 |
| open-vetta `apps/desktop/src/renderer/styles.css` 745–752、595–599、608–615 行 与 `packages/theme-ui/src/project/session-row-transition.ts` 7–9 行 | 全局 button/a 色过渡 150ms；运行图标与脉冲点带 `contain`/`will-change` 性能隔离；会话行选中填充瞬时上色（内联剔除 background-color 过渡）；全局 `:focus-visible { outline: none }` | focus 规则不采纳——rove 要求可见焦点（§3）；其余作为 §3.3 输入 |
| PI `apps/desktop/src/styles/tokens.css` 196–202 行 与 `src/lib/work-panel-resize.ts` 16–23 行 | 全仓动效令牌：150/200/300ms 三档时长 + ease-out/ease-in/ease-standard 三个贝塞尔，transition/animation 一律引用变量；宽度预算常量（450/460/244/compact 1px）集中一处 | 数值是该设计系统自己的规范，rove 按其令牌化思路在 §3.3 定义自己的令牌，不照抄数值体系 |
| PI `apps/desktop/src/features/chat/transcript/hooks/useTranscriptScroll.ts` 168–183、640–649 行 与 `src/components/PermissionCard.tsx` 55–67 行 | 滚动贴底只被真实用户手势解除（布局噪声 re-baseline）、跳转按 reduced-motion 降级 auto/smooth；审批卡 120s 倒计时（role=timer，1s 刷新）+ 到期自动拒绝 | 自动拒绝绑定其宿主语义，rove 无对应合同，不照搬（§3.3） |
| rove [product-v2.css](../../apps/web/styles/product-v2.css) 134–145、2785–2802 行 与 [product.css](../../apps/web/styles/product.css) 1112–1121 行 | 现状动效基线：按钮 140ms、v2 抽屉 180ms cubic-bezier(0.16,1,0.3,1)、骨架屏 shimmer；全局 prefers-reduced-motion 通配 + e2e 断言；三代 token（v1/v2/v3）并存、无动效令牌 | 动效无统一令牌；新组件按 §3.3 用新令牌，不批量翻新旧样式 |

本次实际运行 `pnpm --dir apps/web test chat/Composer.test.tsx`，退出码 0，1 个测试通过。没有运行 PI-Desktop 桌面程序、逐项点击或截图，没有验证外部网站和真实 Provider。本次原有并行调研记录在续接时不可取回，不作为证据引用。

### 2.3 待补充的竞品证据

实施前完成同版本独立测试实例的六条点击旅程：左侧导航、输入对话、审批、文件、变更、浏览器。左栏与中栏输入区的对照对象是 open-vetta，右栏与键盘契约的对照对象是 PI-Desktop，两者的实机证据分别记录，互不替代。不能借用用户正在使用的会话或凭据。逐条记录入口、动作、结果、失败态、截图、提交和平台。以下尚未验证的对应内容是 rove 的建议设计，不是对参考项目已实现行为的断言；实机受阻时按实现计划 P0 的缩减范围确认规则处理。

PI-Desktop 左栏静态证据（`apps/desktop/src/components/Sidebar.tsx` 15–55 行定义每项目近期会话默认折叠数量 10，引用 `getGlobalPinnedSessions`、时间分组、会话状态、项目排序与宽度约束；86–96 行的 `ProjectEntry` 包含会话集合、展开态、激活态和可选分支）保留为对比记录；2026-09-19 起左栏主线参考已改为 open-vetta（见 §2.2 对应行），PI 左栏内容不再作为实现依据，仅用于差异对照。本文项目内置顶仍是 rove 的暂定适配，置顶等持久字段须核对服务端合同，不能声称原样复制任一参考项目。

### 2.4 已有能力对比与采用判定（2026-09-19 三次修订新增）

对 rove 已有能力逐项对照两个参考实现，给出"哪边更好"的判定；判定基于快照源码阅读，不是实机验收结论。rove 现状证据均为本次核查所得，参考侧证据见 §2.2。

| 能力 | rove 现状 | 参考实现 | 判定与采用 |
|---|---|---|---|
| 会话重命名 | 仅 Settings→Sessions 内联表单（[CatalogSettings.tsx](../../apps/web/settings/CatalogSettings.tsx) 305–319、353–383 行：空值/同名不提交），侧栏无入口 | open-vetta 行内重命名：挂载即 focus+select，Enter 提交、Escape/blur 取消 | 参考交互更好；P1a 第 3 条落地行内重命名。"失焦"语义相反（参考=取消，rove 计划=提交），rove 保留失焦=提交防丢字，记录为既定差异 |
| 会话归档 | 无任何 UI 入口：合同支持 `archived`（[product-api-types.ts](../../apps/web/product/product-api-types.ts) 634–637 行），所有消费方过滤归档会话（product-catalog.ts 53、89 行），但无组件发送该字段 | open-vetta 右键菜单 danger 删除项 | 参考入口模式更好；P1a 第 5 条补侧栏菜单归档，打通已有但闲置的合同字段 |
| 会话查找 | 仅本地过滤已加载列表 | open-vetta 180ms 防抖 + 宿主搜索 + limited/skipped 报告 | 参考更好；§3.1 查找行已采纳防抖 + 服务端 search |
| 侧栏宽度与收起 | 宽度固定；窄屏仅 v2 皮肤抽屉 180ms（product-v2.css 2785–2802 行），焦点管理完整（transitionend+兜底、Escape、inert、焦点归还） | 两侧均有拖调 + 收起 + 持久化 | 参考功能更全；P1a/P2 已采纳；rove 既有抽屉焦点管理保持不变 |
| 审批呈现 | ApprovalCard 挂载聚焦、role=alert、busy 禁用、无倒计时（Transcript.tsx 524–573 行） | PI 120s 倒计时 + role=timer + 到期自动拒绝 + resolve 后焦点归还输入区 | 倒计时绑定 PI 服务端自动拒绝语义，rove 无此合同，不照搬；采纳防重入与决策后焦点归还（§3.3） |
| 提交竞争与草稿 | submitting 禁用、成功才清空草稿 | PI 草稿快照→清空→发送→失败回滚，版本 + 请求 token 三重比对防旧回调（useComposerSubmit.ts 124–131、157–172、271–276 行） | 参考更稳；P1b 第 5 条已按此建模 |
| 消息流滚动 | 48px 阈值贴底 + Return to latest + 历史加载位置补偿（Transcript.tsx 86–131、203–207 行），基础可用 | PI 手势解除贴底 + rAF 跟随 + reduced-motion 跳转降级 | rove 基础可用，PI 手势判定更稳；P1b 增强采纳（§3.3） |
| 会话状态指示 | 文字徽章 + 静态圆点（WorkspaceTree.tsx 389–398 行） | open-vetta 图标优先级 + 1s 旋转 + 脉冲点 + contain 隔离 | 参考更好；P1a 第 3 条已采纳，数值入 §3.3 |
| 会话列表体量 | 服务端分页全量拉取（最多 64 页），侧栏树全渲染、无加载更多（server-product-state.ts 64–88 行） | open-vetta Virtuoso 虚拟化、行高常量 34 | 参考更稳；P1a 首批"默认 5 条 + 显示更多"先行，虚拟化作为超长列表后续项 |
| 文件浏览 | 前缀平铺 + 100 条/页 + cursor 加载 + 完整错误态（inspector/FilesPanel.tsx 52–55、109–127 行） | open-vetta 列表虚拟化（其文件树未在本次核查范围） | rove 错误态已达标；P3 按需展开树，长列表必要时虚拟化 |
| Diff 渲染 | unified + 双行号 + 200k 字符/2000 行截断 + fallback（DiffView.tsx 5–6、53–79 行） | 参考侧本次未核查 | 不判定；P4 维持复用 DiffView |

## 3. 信息架构与视觉规范建议

**已确认方向（2026-09-19 修订）**：三栏结构不变；左栏改为**全面参考 open-vetta**（不再参考 PI-Desktop 左栏）；中栏混合参考（输入区结构与视觉参考 open-vetta，键盘/IME/草稿/提交竞争契约沿用 PI-Desktop 调研结论）；右栏维持参考 PI-Desktop。具体视觉和操作细节仍需对照同版本参考项目后确认，整体实现尚未开始。

桌面分为左侧项目/会话导航、中间对话、右侧工作面板。左侧按 open-vetta 的 TopBar → 导航项 → 项目分组/会话行 → BottomBar 组织重做视觉层级与常用操作，但继续映射 rove 的 Workspace → Session 数据关系；中间以可阅读的消息及工具活动为主；右侧承载需要检查、审批、比较或预览的对象。

```text
左侧导航                  中间对话                    右侧工作面板
新建会话 / 查找            当前会话、模型、运行状态     待处理 | 文件 | 变更 | 浏览
项目分组 / 置顶会话        消息、工具活动、证据入口     当前对象详情
会话列表 / 运行状态        输入框、发送、常驻停止       处理结果 / 加载状态
当前项目快捷入口 / 设置
```

建议初值而非已测量竞品尺寸：左栏 200–360px 可拖调、初值 240px（见 §3.1 表格末行）；中栏阅读宽度 680–840px，且拥有 450px 硬底线（见 §5.0）；右栏初始 400px，可在 320–640px 拖动（见 §5.0）。容器小于 1180px 时右栏改抽屉，小于 760px 时左右导航不同时占宽。以现有主题变量为基础，统一中性色表面、细边框、焦点样式和间距，不复制品牌资产。

- 正文建议 14–15px、行高 1.6；工具摘要比正文低一级，警告不依赖颜色表达。
- 输入框建议圆角 16px、内边距 12–16px，自动增高至约 240px 后内部滚动。
- 模型与附加操作放底部工具行；发送和停止视觉清晰，不塞入一排无标签图标。
- 发送、停止、审批状态必须可感知，禁止无限转圈替代失败信息。
- 消息流避免所有内容都套同样的卡片；用户消息、助手正文、工具摘要、审批入口分层展示。
- 用户查看历史时不强拉到底部；出现新内容用可点击提示。长历史稳定分页另设任务，不声称本次排版解决 F.4。
- 每个交互支持键盘焦点；抽屉关闭后焦点归还触发入口；窄屏触摸目标至少 44px。
- 新增动效（运行状态图标旋转、状态条 0fr↔1fr 过渡、抽屉开合）遵循既有 prefers-reduced-motion 全局实践（[product.css](../../apps/web/styles/product.css) 1112 行等），并参考 open-vetta 的克制纪律（§2.2）：无装饰性持续动画，运行/loading 指示除外。对象级数值规范见 §3.3。
- 新增文案一律走既有 copy 体系（[CopyProvider.tsx](../../apps/web/copy/CopyProvider.tsx)，zh-CN/en-US 双字典同步补键），cool/warm 两皮肤（[ui-skin.tsx](../../apps/web/shell/ui-skin.tsx)）下分别验证新组件；不硬编码单语言字符串。

### 3.1 左侧导航同步改造（open-vetta 参考）

左栏是本次首批改造范围，2026-09-19 起参考对象从 PI-Desktop 改为 open-vetta。下面仍是拟采用的 rove 操作契约，不是对 open-vetta 当前实现的逐项事实描述；实施前对照 open-vetta 快照的左栏结构与行为，记录采用、调整和不采用的项目，避免只做换色。

open-vetta 左栏自上而下为四段，rove 的映射意向如下（"rove 现状与差距"列以 [WorkspaceTree.tsx](../../apps/web/sidebar/WorkspaceTree.tsx)、[product-types.ts](../../apps/web/state/product-types.ts)、[contracts.rs](../../apps/api/src/product/contracts.rs) 为准）：
| open-vetta 结构 | rove 映射意向 | rove 现状与差距 | 操作约定 |
|---|---|---|---|
| 顶部 TopBar（品牌区、命令菜单触发 ⌘K、收起按钮） | 新建会话、查找入口、左栏收起按钮 | 现状已有新建（带工作区下拉）与本地过滤输入框；**无左栏收起按钮**（窄屏只有整块抽屉开合），需新增收起/展开与键盘可达性 | 新建绑定明确的当前工作区；未选工作区时先选择，不创建无归属会话；收起态把新建/查找入口移交给页头 |
| 导航项：置顶区 + 收纳区（「新会话」锁首位、最多 5 项、用户可拖拽排序并持久化） | rove 首批只保留「新建会话」与「查找」两个固定入口；可排布导航目录与「更多」收纳区推迟到确有更多一级入口时再做，不做只有一两个入口的空收纳区 | 现状无对应物，首批保持无 | 若未来做可排布导航，持久化、容量上限、版本迁移参考 open-vetta `sidebar-nav-layout.ts` 的纯逻辑分层；本地布局与服务端业务数据分开 |
| 项目分组（ProjectGroup：项目行含名称/类型徽标/运行指示/展开箭头/右键菜单/新建会话） | 工作区分组：展开箭头、名称、当前工作区标识、运行指示点、行内新建会话按钮、右键菜单 | 现状已有展开/选中/置顶徽标/移除菜单；差距：项目行无 hover 新建会话快捷按钮、无运行脉冲指示、无右键菜单（仅菜单按钮）；rove 有工作区级置顶（`ProductWorkspace.pinned`），保留 | 展开/收起只改变列表；点击名称选中工作区（rove 无项目详情页，这是与 open-vetta 的既定差异）；切换工作区不隐式授权目录访问 |
| 会话行（SessionRow：active/running/scheduled/pinned/renaming 状态分离、hover 出操作、内联重命名、「显示更多」分页） | 紧凑会话行：leading 状态图标（运行中旋转指示 → 待审批警示 → 普通会话图标，优先级递减）+ 标题 + hover 操作；项目内默认显示 5 条并按需展开 | 现状会话行平铺无分页、无内联重命名、无右键菜单、状态仅文字；`SessionRecord` 已有 title/status/updatedAt，**无 pinned 字段** | 单击打开会话；选中、hover、键盘焦点与运行态互相独立；renaming 时行内输入框接管（Enter 提交/失焦提交/Escape 取消，空值或与原值相同则不提交），点击不再触发选择 |
| 会话排序（置顶区按置顶时间倒序 + 其余按修改时间倒序；折叠计数 = max(5, 置顶数)，置顶永不折叠） | 按 `updatedAt` 倒序；默认可见 5 条，「显示更多 N 条」展开，折叠状态首批按工作区在内存中记忆（不持久化，刷新重置；宽度才落 UI 偏好） | 现状无排序保证与分页 | 置顶能力分两档：先做「无置顶字段时的排序与分页」；会话置顶字段若要落地须走 P1a 的服务端合同扩展（默认值、迁移、并发、旧客户端兼容），不先用 localStorage 冒充 |
| 会话右键菜单（置顶 → 重命名 → 标签子菜单 → 在文件夹中显示 → 删除 danger） | 右键/菜单按钮统一一份菜单：重命名、归档（danger 语义、确认文案）；置顶项随置顶合同扩展补入 | 现状仅工作区级移除；会话级操作缺失。重命名/归档走既有 `UpdateProductSessionRequest { title, archived }`，无新合同 | 「在文件夹中显示」Web 端无宿主能力，Tauri 端可作为受控命令后续补；标签体系不做。归档不等于删除本地目录；运行中会话归档按现有生命周期限制处理 |
| 侧栏内查找（命令菜单 + 宿主搜索，180ms 防抖、流式合并、limited/skipped 提示） | 两层：本地即时过滤已加载项目/会话名（现状已有，保留）；输入防抖后可选调服务端 `search` 参数（≤128 字节、配合游标分页），结果明确区分"已加载范围匹配"与"服务端匹配" | 现状只有本地过滤，且不过滤未加载会话 | 未加载不能被报告为不存在；区分无项目、空项目、无匹配、部分列表和加载失败；完整历史搜索是服务端能力，不在左栏假装已实现 |
| 底部 BottomBar（设置菜单含账号/主题/配额/IM 状态 + 消息中心 + 更新横幅） | 底部固定设置入口及必要的连接/服务状态 | 现状设置入口在顶栏；连接状态缺失 | 长列表滚动不挤走设置入口；错误可查看详情；账号/配额/IM/更新横幅等 rove 无对应来源，不做假入口 |
| 侧栏宽度 180–400px 拖调并持久化（`ResizeHandle`：8px 命中区、拖拽期全屏 overlay、rAF 合帧、直写 DOM） | 左栏宽度可拖调，建议范围 200–360px、初值 240px，存 UI 偏好 | 现状宽度固定 | 拖拽期间避免整树重渲染的思路可借鉴；宽存 localStorage UI 偏好（产品偏好合同是类型化字段、不承载布局值，见 §2.2），不存业务状态；窄屏拖拽把手隐藏 |

会话默认在项目内按最近活动（`updatedAt` 倒序）组织；置顶区作为合同扩展项单独处理。时间分组可作呈现，不新建业务对象。状态轮询不应让会话行频繁跳位；排序更新时保持选中项可见和滚动稳定。

会话行状态更新沿用既有通道：聚焦 job 的 `GET /jobs/{job_id}/events` SSE 与存在 running/needs_attention 会话时的目录轮询（§2.2 rove 证据行），左栏不私建第二套事件流；多窗口以服务端投影重读为准。条件轮询对其他窗口新启动运行的可见性延迟是既有行为，实施时不为左栏放宽成全局常驻轮询，如需改进另立任务。

- 会话行建议高 36–40px，窄屏命中区域至少 44px；长标题截断但可获得完整名称，重名项目需可查看路径区别。
- hover 显示更多操作，键盘聚焦时同样可见；菜单与展开箭头不应冒泡触发会话切换。选中项使用背景和文本权重，不能只靠细微颜色差。
- 重命名、归档/删除等操作优先复用已有产品能力。删除不等于删除本地目录；运行中会话按现有生命周期限制处理，不因从列表移除而停止执行。
- 查找首期定位当前已加载项目/会话名称，并明确搜索范围；完整历史搜索需有服务端支持，未加载不能被报告为不存在。
- 会话列表按需加载，区分空项目、无匹配、加载失败和部分列表。导航分页与消息历史分页是不同任务，不能混为 F.4 已解决。
- 折叠状态、列表滚动和当前选择按工作区隔离；深链接、刷新、前进/后退仍通过现有路由恢复。窄屏打开会话后收起导航并将焦点移入会话标题。
- 左栏只负责定位，文件树、审批详情和 diff 保持右侧单一展示位置；左侧不再复制一套文件/证据状态。

### 3.2 三栏联动

切换左侧会话时，中间恢复该会话草稿和消息，右侧清除不属于该会话的审批/证据选择；同工作区文件预览可保留，但必须明确它是工作区内容。跨工作区切换时清除旧文件、预览及异步结果。后台任务继续运行，左栏仅显示其服务端状态，不把导航切换当成取消操作。

### 3.3 动效与微交互规范（2026-09-19 三次修订新增）

本节把两个参考项目的动效数值固化为 rove 的实现规范；数值来自当前快照源码（§2.2 动效证据行），实机手感核对仍属 P0。三条硬约束：

- 全部用 CSS transition/keyframes 实现，不引入 framer-motion/motion 等新依赖；open-vetta 中只有浮层/胶囊用 JS spring，rove 首批对应物用 CSS 等效或保持瞬时。
- 新增动效令牌 `--motion-duration-fast:150ms`、`--motion-duration-normal:200ms`、`--motion-duration-slow:300ms` 与 `--motion-ease-out:cubic-bezier(0.22,1,0.36,1)`、`--motion-ease-in:cubic-bezier(0.4,0,1,1)`、`--motion-ease-standard:cubic-bezier(0.2,0,0,1)`（体系取自 PI `tokens.css` 196–202 行；open-vetta 无令牌、硬编码 `cubic-bezier(0.22,0.61,0.36,1)`，两参考中速缓动数值接近但不相同，rove 取 PI 的令牌化方案并只用一套缓动）。新组件一律引用令牌，不写散落时长；既有 v1/v2/v3 样式不批量翻新。
- rove 既有全局 prefers-reduced-motion 通配（product.css 1112–1121 行，配 e2e 断言）比两个参考项目的逐组件覆盖更强，保留；新动效只用 transition/animation 属性即自动被覆盖，不用 JS 驱动动画。

| 对象 | 参考行为（快照数值） | rove 采用 |
|---|---|---|
| 会话/项目行 hover 与选中 | 全局 button 色过渡 150ms（open-vetta styles.css 745–752）；选中填充瞬时上色——会话行内联剔除 background-color 过渡（session-row-transition.ts 7–9），项目行 100ms | 150ms 色过渡 + 选中瞬时上色，避免选中"慢半拍" |
| 运行状态指示 | 会话行旋转图标 animate-spin（1s linear infinite，SessionStatusIcon.tsx 15–24），配 `contain: layout paint style` 与 `will-change: transform`（styles.css 595–599）；项目行脉冲点 ping 1s | P1a 状态图标按此实现并保留 contain/will-change；文字徽章保留为可访问文本 |
| 项目分组展开/收起 | `grid-template-rows 0fr↔1fr + opacity` 200ms ease-out，动画结束（220ms）才卸载子树（ProjectSessionsView.tsx 57–62、useDelayedUnmount 220ms）；超长列表虚拟化（Virtuoso，行高 34） | CSS 等效 + 220ms 延迟卸载；虚拟化作为超长列表后续项（§2.4） |
| 左栏收起/展开 | 占位宽度瞬时落定，面板 transform+opacity 240ms `cubic-bezier(0.22,0.61,0.36,1)`，收起态 inert+aria-hidden、子树不卸载（SidebarDock.tsx 11、70–83）；窄屏浮层 180ms（SidebarOverlay.tsx 19–24） | 240ms transform 方案，焦点管理沿用 rove 既有 transitionend+兜底模式；窄屏抽屉沿用现有 180ms 并统一到令牌 |
| 右栏显隐与拖宽 | 面板入场 200ms ease-out / 出场 150ms ease-in（opacity+宽度+translateX 8px，work-panel.css 29–72），动画结束才卸载（WorkPanel.tsx 438–443）；拖宽瞬时无过渡，拖动期全局 col-resize 光标+禁选（WorkPanel.tsx 220–229），指示条 150ms | 同规则；拖宽不加过渡，宽度记忆走 localStorage（§5.0） |
| 键盘调宽 | 方向键 ±16px、Shift ±32px、Home/End 到边界（WorkPanel.tsx 397–407） | 左右栏拖调把手补键盘支持（P2） |
| 页签切换 | 内容瞬时切换（display none），仅页签底色 150ms（work-panel.css 218–221、364–366） | 不加内容切换动画 |
| 右键/下拉菜单 | 出现与消失各 100ms fade + zoom(0.95)（dropdown-menu.tsx 14–15） | CSS keyframes 等效 |
| 行内重命名 | 无过渡，挂载即 focus+select，Enter 提交、Escape/blur 取消（SessionRenameInputView.tsx 23–26、SessionRowView.tsx 72–78） | 交互照搬（focus+select、Escape）；"失焦"语义保留 rove 的失焦=提交；焦点可见性按 rove 规则，不采纳 open-vetta 全局去焦点环（styles.css 608–615） |
| 发送/停止与 pending 胶囊 | open-vetta SendButton 为纯 CSS 形变状态机（to-stop ≈333ms / to-send 1s keyframes + 1.8s ripple，send-button.css 3–4、137–277 行）；pending 胶囊 `grid-template-columns 0fr→1fr` 180ms ease-out（291–302 行） | 首批不做图标形变与 ripple：图标切换 + 150ms 过渡；pending 胶囊照搬 180ms 0fr→1fr |
| 输入区下沿状态条 | 非对称折叠：展开 300ms ease-out / 收起 200ms ease-in（"抬高比落下慢"），内容 280ms 延迟 40ms，退场 220ms 卸载（InputBarFooter.tsx 21–30、input-bar-footer-state.ts 13） | CSS 等效；§4.0 的运行/错误状态条按此实现 |
| 消息流滚动跟随 | 贴底跟随只被真实用户手势（wheel/touch/pointer/keydown）解除，布局噪声 re-baseline；rAF 跟随；回到最新按钮 200ms scale+opacity 入场；跳转按 reduced-motion 选 auto/smooth（useTranscriptScroll.ts 168–183、320–369、640–649） | rove 已有 48px 阈值 + Return to latest（§2.4），增强手势解除判定与 reduced-motion 跳转 |
| 审批卡 | PI 倒计时 120s（role=timer，1s 刷新）+ 到期自动拒绝 + resolve 后焦点归还输入区（PermissionCard.tsx 55–67、126–152 行） | 倒计时/自动拒绝依赖 PI 服务端语义，rove 无此合同不照搬；采纳防重入与决策后焦点归还输入区 |
| 上限纪律 | open-vetta DESIGN.md 145–165 行：hover y≤2px、按钮 scale ≤1.04/≥0.94、进入 ≤0.5s、stagger 0.04–0.06、禁装饰性持续旋转（spinner 除外）、禁 transition-all | 作为新组件动效审查清单 |
| 不采用 | open-vetta 装饰套件（Aurora/EnergyWell/PixelHand/BlazeFlame/PixelTorch/Orb）、占位文案轮换、主题切换圆形揭示 | 全部不进 rove |

## 4. 对话输入和内容展示

中栏采用混合参考：**输入区结构与视觉参考 open-vetta**（会话头、消息流与底部 Composer/InputBar 分层，工具行承载模型入口与附加操作），**键盘、IME、草稿与提交竞争契约沿用 PI-Desktop 调研结论**。open-vetta 输入区还包含 @ 提及、附件、语音、todo/上下文指示等能力；这些是 rove 没有对应服务端合同的能力，本设计只参考其分层结构，功能不进入首批范围，什么时候引入取决于上下文与上传协议的独立设计。

### 4.0 中栏结构映射（open-vetta 参考）

| open-vetta 结构 | rove 映射意向 | rove 现状与差距 |
|---|---|---|
| `PageHeader` 页面级会话头（`PageHeader.tsx` 8–20 行：标题 + 左右 slot；头部无模型/运行状态指示，模型选择器在输入栏 `InputBarToolbar.tsx`，`ChatHeaderActionsView.tsx` 12–33 行放徽标与导出/置顶/面板按钮） | 中栏保留现有会话头（[ProductApp.tsx](../../apps/web/shell/ProductApp.tsx) 482–499 行：标题、工作区、Fork/Inspector 入口）；模型入口保持在输入区工具行，不在会话头重复；运行状态可在会话头或输入区状态条呈现，读取 `SessionRecord.status` | 现状会话头已有标题与入口，缺运行状态投影；与 open-vetta 一致不在头部放模型 |
| `DefaultChatView`：消息列 + 可选活动列，`ChatComposer` 固定底部 | 消息流 + 底部 Composer 的既有结构保留；右栏沿用 PI-Desktop 方案，不引入 open-vetta 的活动列 | 现状 [Composer.tsx](../../apps/web/chat/Composer.tsx) 是平铺表单：textarea + 发送/停止按钮 + 模型控件行 + Review 表单混排 |
| `MessageInput.Surface`：卡片化输入容器（居中 max-w-2xl、`@container` 按输入区宽度折叠工具行） | 输入区收敛为一张卡片：输入区、工具行在同一容器内，居中限宽，窄屏用输入区宽度而非视口宽度决定工具行折叠 | 现状输入区无卡片边界与限宽 |
| 工具行左簇（技能/命令、@ 提及、附件） | 左簇首批为空或只放 Review 入口；@/附件/技能无服务端合同不进首批 | 现状 Review 表单内联在输入区下方，需收进工具行入口 + 弹层 |
| 工具行右簇（模型选择器 → 上下文环 → 语音 → 发送） | 右簇为 模型选择（复用 QuickModelControl）→ 发送/停止 双态按钮 | 现状模型控件与发送按钮分离在不同行；停止是独立 danger 按钮，不与发送同位 |
| `SendButton`：send/stop 同位双态、pending 文案态（提交前的前置步骤展开成带文案胶囊并拒绝点击）、abort 请求序号防旧回调复活 | 发送/停止同位双态；提交中态有明确 pending 呈现；"正在停止"不等于"已停止" | 现状发送与停止是两个按钮，提交中仅 disabled，无 pending 语义区分 |
| 运行中且草稿非空时主按钮变「排队」按钮（`canQueue && isStreaming && !isEmpty`） | rove 只有一个 Send Message 动作，运行中消息的排队/进入语义以服务端返回为准，UI 展示该语义但不私建第二队列 | 现状 busy 时发送禁用，无排队表达 |
| 输入区下沿 Footer 插槽（todo/语音状态，`grid-rows: 0fr↔1fr` 纯 CSS 过渡） | 下沿插槽首批只承载运行/错误状态条；动画用 CSS 过渡，不为状态条引入 JS 动画 | 现状 busy/错误提示是 meta 行文字 |
| `InputBarFooter` 上方的 MCP 提问/用户提问浮层 | rove 的审批/输入请求走右栏与消息流入口（§5），不在输入区上浮 | — |

### 4.1 推荐操作契约（沿用 PI-Desktop 调研结论）

默认保持现有 Enter 换行行为；新增 Ctrl/Meta+Enter 发送。可在后续设置中增加 Enter 发送偏好，启用后 Shift+Enter 换行。中文 IME composing 或 keyCode 229 时不触发发送，补全展开时先接受候选，不顺带发送。

只保留一个 Send Message 产品动作，不照搬 PI 的独立 steering 语义。运行中消息如何排队或进入当前执行，以 rove 服务端返回状态为准；product API 已有运行中提交面（steers/followups，§2.2 rove 证据行），首批 UI 不新增入口，展示语义仍以服务端返回为准。停止常驻且与发送独立；"正在停止"不等于"已停止"。

| 状态 | 输入框和按钮 | 恢复策略 |
|---|---|---|
| 空草稿 | 可输入，发送禁用 | 无错误提示 |
| 可发送 | 展示快捷键提示 | 按钮与快捷键同一处理函数 |
| 提交中 | 防重复提交；停止若适用仍可用 | 绑定原会话与提交标识 |
| 发送成功 | 只清除本次已提交草稿 | 不清除切换会话后的内容 |
| 明确失败 | 保留草稿，显示原因 | 用户主动重试 |
| 回应丢失/结果不明 | 显示正在确认 | 先查服务端绑定，不能自动重复开 run |
| 运行中 | 显示消息接收语义和停止 | 不私建第二个队列 |
| 会话切换 | 按会话保存内存草稿 | 过时响应不写入新会话 |

首期不持久化草稿到 localStorage，避免敏感文本落盘与迁移扩大；持久化另行讨论。文件引用首期只作为可见文本引用，须解释引用不等于内容已读入。附件、富文本 chips、命令补全待上下文与上传协议确认后再做。

### 4.2 消息与工具结果

长工具输出默认摘要，按需展开；正文 Markdown、代码、diff 使用已有渲染器。错误、截断、部分历史不可折叠成成功摘要。文件路径、工具调用和审批记录使用结构化标识关联，不能从自然语言回答推断事实。复制正文与复制代码分别有反馈，不把内部日志作为正文。

## 5. 右侧工作面板与审批

### 5.0 右栏结构映射（PI-Desktop 参考）

PI-Desktop `components/workpanel/WorkPanel.tsx` 与 `lib/work-panel-resize.ts` 的已核实结构：

| PI-Desktop 结构 | rove 映射意向 | rove 现状与差距 |
|---|---|---|
| 多页签面板：页签条（激活页签自动滚入视野）+「+」新建页签 + 工具启动器（Review 为宿主自有，文件/浏览器为插件贡献） | 固定页签集：待处理、文件、变更、浏览（不引入插件贡献机制）；每页签可关闭与否在实施时确定，首批可只切换不关闭 | 现状 [RunInspector.tsx](../../apps/web/inspector/RunInspector.tsx) 已有 Files/Diff/Artifact/Review 面板组件，但组织在检查器而非固定右侧栏 |
| 三栏共享宽度预算：`MAIN_PANE_MIN_WIDTH=450` 是中栏硬底线，右栏最大宽度 = 容器 − 左栏 − 450；到达底线时先自动收起左栏，右栏让位 | 采用同样的预算优先级：中栏底线 > 右栏请求 > 左栏；容器过窄时按 收起左栏 → 右栏改抽屉 的顺序降级 | 现状右栏（检查器）无共享预算，窄屏降级规则见 §3 断点 |
| 右栏宽度持久化 + 拖调：常态最小 244px、默认 360px；左栏重开时右栏先让位保中栏，必要时压到 compact 最小值 | 右栏初始 400px、可在 320–640px 拖动（§3 已定），宽度存 localStorage UI 偏好（同 §3.1 口径）；左栏重开时右栏让位的联动逻辑参考 `workPanelWidthForSidebarReopen` | 现状无宽度记忆 |
| 预览最大化：右栏可吃掉中栏宽度（maximized 时中栏不渲染） | 文件/diff 预览可提供"放大"态；审批页签不放大（审批需要对照消息流） | 现状无 |
| 页签内容由 tab 引用解析；插件被禁用后页签回退为 id 文案而非空白 | 页签内容加载失败/来源失效时显示明确空态，不留白屏 | — |

右栏页签与内容的对应：待处理=当前审批与输入请求（§5.1），其下分区展示本会话最近授权记录（P4 交付后启用）；文件=工作区只读浏览（§6.1）；变更=会话证据与工作区 Git 差异（§6.2）；浏览=外链与本地预览（§7）。页签与打开对象首批为内存态：刷新后回到默认页签，PI 同样只持久化宽度、不持久化页签（§2.2）；对象级深链接作为后续路由扩展评估，首批不进路由。

### 5.1 审批契约

右侧"待处理"展示请求对象、工具名、脱敏参数、作用范围、风险说明、关联消息及操作按钮。消息流中的审批卡展示摘要并跳转到同一详情；窄屏仍能在消息流完成操作。新审批可提示并增加计数，不抢夺正在编辑的焦点。

状态建议为 waiting、submitting、approved、denied、expired、error；这些是 UI 投影名，需映射现有服务端类型，不能直接新增平行事件。

- 每次操作绑定 workspace、product session、job/run、approval ID 及服务端已有的并发校验字段。
- 双击、多个标签页、过时请求必须由服务端拒绝或幂等处理；不允许仅靠前端 disabled 保证正确性。
- 若现有记录不支持历史授权字段，先给出"历史不可用"，再在后续独立阶段扩展持久化合同。
- "仅此次""会话范围""工作区信任"等选项只有现有权限系统真实支持时才展示，不能由布局设计授予权限。
- 授权记录显示谁/何时/范围/决策及关联结果；没有该字段就显示未知，不从工具成功猜测已获批准。

特别区分：工具权限审批、计划确认、用户输入请求、只读代码 Review 是不同对象，不合并成一个含糊的"审批"。

授权记录分区：待处理页签在当前请求之下展示本会话最近授权记录（谁/何时/范围/决策/关联结果）。rove 现状是决策状态与时间戳已落 runtime StateStore `pending_approvals`，但无"谁决策"字段、无产品域历史查询端点（§2.2）；首批先显示"历史不可用"或字段级未知，查询端点与"决策者"字段走 P4 的合同扩展流程。PI 只有实时待审批队列、决策后出队、无历史 UI（§2.2），该分区是 rove 扩展，不冒充参考项目设计。

输入请求与计划确认：与工具权限审批并列展示于待处理页签，映射现有提交面（job 域 approvals/inputs、product 域 controls/steers/followups；Web 端现有 [Transcript.tsx](../../apps/web/chat/Transcript.tsx) 的 ApprovalCard 与 InputCard）。输入请求卡展示请求来源（run/step）、问题内容与可提交选项；提交绑定 workspace、product session、job 与请求 ID 及服务端并发校验字段，过时或已决请求由服务端拒绝。不新增提交协议，P2 第一步做"已有字段 → UI 需要字段"对照。

## 6. 文件与变更面板

### 6.1 文件浏览

面板顶部展示当前工作区名称和相对目录，按需展开文件夹，分页/继续加载提示可见。单击只读预览，双击行为在实施前统一确定，不混入编辑。区分空目录、无访问权限、读取失败、结果截断、文件已删除、二进制和超大文件。

所有路径解析在已有执行环境或服务端边界处理，验证根路径、符号链接、编码及 TOCTOU；前端不接受服务器任意路径当成可读取本地路径。默认尊重现有 ignore 规则，不扫描 node_modules/target 等生成树。刷新取消过时请求，工作区切换不得显示旧结果。

### 6.2 变更证据

提供两个明确分区：

1. **本会话记录的变更**：来自可关联的工具结果与 Artifact；按文件聚合，保留调用及时间，允许查看当次 diff。
2. **当前工作区 Git 差异**：注明比较基线和采样时间，可能包含用户及其他会话的修改。优先复用既有 `GET /product/sessions/{session_id}/diff`（[diff.rs](../../apps/api/src/product/diff.rs)：`scope=run` 对应分区 1、`scope=git` 对应本分区，基线当前固定为 HEAD；已带 4096/512 条、单条 128 KiB、总量 4 MiB、10s 超时约束与 partial_reasons），不新建第二套 git 命令面；需要采样时间或非 HEAD 基线时按 P4 的合同扩展流程提出。

两者都不能替代不可变执行事实。缺少前后快照时显示"缺少可比较内容"，不是零修改；rename/delete/binary/truncated 采用明确状态。默认不提供一键撤销、丢弃或应用补丁，避免覆盖用户修改。若未来增加撤销，必须另有检查点、当前内容冲突检测和审批设计。

## 7. 浏览器与预览

"正确打开"至少需要以下三个独立验收对象。

| 场景 | 首期建议 | 失败和限制 |
|---|---|---|
| 用户点击外部 http(s) 链接 | Web 新标签；Desktop 经受控宿主入口打开系统浏览器 | 明确打开失败、复制链接、重试；不将启动调用成功等同页面加载成功 |
| 工作区本地 HTML | 提供受限预览服务和独立隔离来源；由用户打开 | 不暴露任意 file://，不共享产品登录凭据，不向页面注入宿主权限 |
| 任意网站内嵌浏览 | 独立后续阶段评估原生 webview | CSP/X-Frame-Options、登录、下载和弹窗不能靠 iframe 通用解决 |

预览面板展示地址、加载状态、刷新、关闭、外部打开；不支持返回/前进时不放假按钮。网页加载失败不得白屏，错误应区分不支持、拒绝加载、服务未启动和连接失败。

本地预览会执行不可信项目内容，打开前必须告知并经项目访问策略判断；预览服务只绑定回环地址且有随机访问令牌/生命周期限制，资源请求仍受根目录约束。外部导航只允许明确协议，不接受 javascript/data/file 等作为外链。若服务端代取远程内容，需独立评估 SSRF/重定向边界；首期不做通用代理。

BrowserPreview 开发工具可帮助检查页面，但不是 rove 已有用户产品能力。浏览器自动化/Agent 控制浏览器不在首期范围内。

## 8. 实施前需要讨论的决定

已确认三栏结构；左栏改参考 open-vetta、右栏维持参考 PI-Desktop、中栏混合参考；以下细节除已注明外仍为建议，不视为产品实现授权。

| 决策 | 推荐默认 | 替代方向与代价 |
|---|---|---|
| 左栏 | 参考 open-vetta 四段结构：TopBar（新建/查找/收起）、项目分组与会话行、底部设置；会话行状态分离与内联重命名按 open-vetta 的 SessionRow 思路 | 不照搬其导航目录、jotai 状态与 localStorage 布局；置顶等持久化能力需核对服务端合同 |
| 中栏输入区结构 | 视觉与分层参考 open-vetta InputBar：工具行承载模型入口与附加操作 | @ 提及、附件、语音等无服务端合同的能力不进首批 |
| 中栏键盘契约 | textarea 渐进改造；IME/发送/草稿沿用 PI-Desktop 调研结论 | 富文本需要额外光标、粘贴、撤销、IME 测试 |
| 右栏策略 | 可折叠；审批提示不抢焦点 | 自动切换更醒目，但可能打断查看文件 |
| 变更范围 | 会话证据与 Git 分开 | 合并展示容易误认修改归属 |
| 浏览器目标 | 先系统外链和隔离本地预览 | 内嵌任意网站增加宿主、平台及安全成本 |
| 首批交付 | 左侧导航改造 + 输入框 + 右栏容器 + 当前审批 | 文件、证据持久化与浏览器分阶段验收 |
| 查找入口 | 顶栏即时过滤框 + 服务端 `search` 防抖检索（§3.1 查找行）；⌘K 全局命令菜单推迟 | 不做全局命令菜单的首批成本是跨工作区全文检索不可用 |
| 授权历史归属 | 待处理页签下方"本会话最近授权记录"分区（§5.1）；决策状态已落 runtime StateStore，产品域查询端点与"决策者"字段走 P4 合同扩展 | 并入变更页签（证据语义混杂）或独立页签（页签数增加，与"待处理"心智分离） |
| 布局偏好存储（宽度等） | localStorage 存 UI 偏好；产品偏好合同是类型化字段、不承载布局值（§2.2） | 扩展偏好合同可跨端同步，但需 schema/类型扩展、迁移与 revision 语义评审 |
| 动效实现方式 | CSS transition/keyframes + 新增 --motion-* 令牌（§3.3）；不引入 motion 库 | 引入 framer-motion 可得 spring/stagger 与 layout 动画，但属新增 npm 依赖，与仓库"不为外观加依赖"规则冲突 |

## 9. 不直接搬用及验收边界

不直接移植 Electron preload、Node runtime、PI store 或权限模型；同样不移植 open-vetta 的 jotai 状态体系、theme-sdk/theme-ui 主题区域机制、插件目录、tailwind/iconify 依赖栈或宿主 IM/更新能力。不复制任一参考项目的产品标识和素材。若未来复制代码，必须先核对对应提交 LICENSE、第三方依赖和素材条款，再记录来源及归属；本次仅参考交互思路，没有复制源代码。

实施任务、门禁和回滚方式见[实现文档](../plans/2026-09-17-pi-desktop-workbench-implementation.md)。当前 [Runtime 文档](../runtime/README.md)不因本文被改成已实现状态。最终验收需要真实点击证据；单元测试、mock 浏览器测试、live API、安装版 Desktop 分开记录，未跑不得写 PASS。
