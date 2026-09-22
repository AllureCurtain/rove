# open-vetta 与 PI-Desktop 体验借鉴的 rove 实现计划

> 状态：**Partially Implemented**。P1a、P1b、P2、P3 与 P4 请求侧已实现并有测试证据；P5a/P5b、P4 决策侧合同扩展与 P6 综合验收未完成。实施进展、每阶段证据与未关闭项见 [P0/P1a 证据报告](2026-09-17-pi-desktop-workbench-p0-evidence.md)。
> 日期：2026-09-17；2026-09-19 按用户确认修订分栏参考来源。
> 输入：[设计文档](../design/2026-09-17-pi-desktop-workbench-design.md)。
> 本文是未来实施文档，不是完成报告。本次仅新增文档，不修改运行时或产品 UI。
> 已确认方向（2026-09-19 更新）：三栏布局；**左栏全面参考 open-vetta**（取代 2026-09-17 的 PI-Desktop 决定）；**中栏混合**：输入区结构与视觉参考 open-vetta、键盘/IME/草稿/提交竞争契约沿用 PI-Desktop 调研；**右栏维持 PI-Desktop**。本次仅更新文档，不启动产品实现。
> 2026-09-19 二次修订（文档评审）：补 P6 综合验收章节与 P5 修改落点；P0/P4 补记既有会话 diff（`GET /product/sessions/{id}/diff`）、偏好合同形状与审批落库事实；明确授权历史归属待处理页签、布局偏好走 localStorage、会话状态沿用 SSE+条件轮询、新增文案走 copy 体系、动效尊重 prefers-reduced-motion；P2 重编号并补输入请求契约项。
> 2026-09-19 三次修订（能力对比与动效）：设计文档新增 §2.4 已有能力对比判定与 §3.3 动效规范（--motion-* 令牌、对象级数值表、纯 CSS 不引入 motion 库）；本计划 P0 补动效实机核对，P1a/P1b/P2 补动效与键盘调宽条目，验收矩阵补 M1 行。

## 1. 交付原则与当前基线

在 rove `7e54ab6` 上渐进改造。左栏与中栏视觉结构参考 open-vetta `19b3e712`（2026-09-18 本地快照，未执行 `git pull` 更新）；中栏键盘契约与右栏参考 PI-Desktop `8d826433`。每阶段作为独立可审查变更；先收敛现有组件和 API，再补不足的持久化数据。不得为右侧面板新建 Agent loop、权限引擎或私有事件生命周期，也不得移植 open-vetta 的 jotai/theme-sdk 状态与主题框架。

当前已知代码落点：

| 路径 | 本计划中的职责 |
|---|---|
| [Composer.tsx](../../apps/web/chat/Composer.tsx) | 输入区、发送/停止、模型与 Review 入口 |
| [Composer.test.tsx](../../apps/web/chat/Composer.test.tsx) | 保留已有停止按钮渲染回归并补交互测试 |
| [Transcript.tsx](../../apps/web/chat/Transcript.tsx) | 消息、活动与关联详情入口 |
| [DiffView.tsx](../../apps/web/product-v2/DiffView.tsx) | 优先复用已有差异渲染，不另造 diff 解析器 |
| [diff.rs](../../apps/api/src/product/diff.rs)（`GET /product/sessions/{id}/diff`） | 既有会话 diff 读取（run/git 双作用域、有界、partial_reasons）；P4 变更页签优先复用，不新建 git 命令面 |
| [RichText.tsx](../../apps/web/product-v2/RichText.tsx) | 正文与链接呈现，保留已有内容安全措施 |
| [copy/](../../apps/web/copy/CopyProvider.tsx) | 既有文案/语言体系（zh-CN/en-US 双字典 + CopyProvider）；新增 UI 文案必须走该体系，不硬编码字符串 |
| `apps/web/product/`、`apps/web/state/` | 现有产品 API 客户端、会话身份和状态接入；实施前定位实际拥有者 |
| `apps/api/`、`runtime/` | 审批、工具结果、Artifact、工作区路径、持久化和安全的既有权威 |
| `apps/desktop/` | Tauri 原生打开及可选预览宿主；不得替代 API/ProductStore |

未在本次确认的具体 API 字段、路由和组件名不能当作已有合同。各阶段第一项任务必须写出"已有字段 → UI 需要字段"对照，复用优先；不足时才提出具体兼容扩展并审查。

## 2. 阶段依赖与退出条件

```text
P0 证据与方案确认
  → P1a 左侧项目与会话导航
  → P1b 输入与消息体验
  → P2 工作面板与当前审批
  → P3 文件只读浏览
  → P4 授权历史与变更证据
  → P5 外链与隔离预览
  → P6 综合验收
```

P3 与 P4 在 P2 合同稳定后可由不同负责者并行，但不能并发编辑共同 shell 或状态文件。P5 的平台可行性调研可提前，产品实现不绕过 P0 的安全讨论。

### P0：冻结证据、关键决策和实施范围

**产出**：竞品点击证据表（open-vetta 左栏/中栏与 PI-Desktop 中栏/右栏分别记录）、rove 当前 API 对照、批准的首批范围。

- 在隔离数据目录和合成工作区中启动同提交 open-vetta 测试实例，核对左栏四段结构（TopBar、项目分组、会话行状态与分页、BottomBar）、宽度拖调、会话菜单与内联重命名、输入区工具行分层；启动同提交 PI 测试实例，核对输入键盘契约、审批、文件、diff、浏览器。左栏重点核对分组、置顶范围、选中/运行/待处理状态、菜单、折叠、宽度调整和设置入口，区分直接借鉴与 rove 适配。
- 先看启动与 E2E 脚本，禁止调用用户当前凭据或向生产 Provider 发送测试。
- 每项记录提交、平台、测试数据、步骤、观察结果、截图位置和失败态。不支持运行则记录 blocker，不写"交互已验证"。
- 动效核对：按设计 §3.3 数值表在双侧实机实例上核对侧栏滑动/折叠、菜单、发送按钮、滚动跟随的手感与数值，逐项记录偏差；数值表来自源码阅读，不代表已实测。
- 核实 LICENSE、第三方依赖及素材边界；优先独立实现交互思路。
- 三栏方向已确认；分栏参考已修订为左栏 open-vetta、中栏混合、右栏 PI-Desktop；继续确认左栏具体操作、输入键位、右栏自动打开策略、变更归属标签和浏览器一期范围。
- 明确已有文件/审批/证据视图及协议：含 `GET /product/sessions/{id}/diff` 的 run/git 作用域、偏好合同的类型化字段、runtime StateStore `pending_approvals`（无"决策者"字段、无产品域查询端点）与 approvals/inputs/controls/steers/followups 提交面；未找到证据的能力标记 unknown，不直接删除或替换。

**退出条件**：完成 open-vetta（左栏、输入区结构）与 PI-Desktop（键盘契约、审批、文件、变更、浏览器）两侧的实机证据并确认首批具体范围；所有"已实现"的结论可指向源代码、测试或实机证据。若实机受阻，记录 blocker，仅经用户确认的缩减范围可进入实施，不能把布局方向确认当成免除验证或完整实现授权。

### P1a：左侧项目与会话导航（open-vetta 参考）

**修改落点**：`apps/web/sidebar/WorkspaceTree.tsx`（按 TopBar/项目分组/会话行/底栏拆分子组件）、现有样式和 copy 体系、路由与产品客户端；不并行建立第二套会话 store。逐段实施范围以设计文档 §3.1 的「open-vetta 结构 → rove 映射意向 → 现状差距」对照表为准。开工第一项与 P1b 确认共享 shell 边界（布局容器、草稿 store、会话切换清理的归属）并记录在阶段变更说明中。

1. 对照 open-vetta 左栏完成四段结构：顶部 TopBar（新建/查找/收起）、中部项目分组与会话、底部设置区域，统一选中、hover、focus、状态标识和菜单层级，不只换色。open-vetta 的可排布导航目录（置顶区/收纳区）首批不做，rove 只保留固定的新建会话与查找入口，待确有更多一级入口时再按其 `sidebar-nav-layout.ts` 的纯逻辑分层设计。
2. 项目行对照 open-vetta `ProjectRowView`：展开/收起与选中工作区分离；hover 新建会话按钮 `stopPropagation` 不冒泡；运行指示点由项目内会话状态聚合。rove 无项目详情页，点击名称=选中工作区。保留深链接、刷新及前进/后退恢复。
3. 会话行对照 open-vetta `SessionRowView`：leading 状态图标按 运行中（旋转指示）→ 待审批（警示）→ 普通会话图标 优先级；renaming 行内输入框接管整行（Enter/失焦提交、Escape 取消、空值或与原值相同不提交），renaming 中点击不触发选择；运行/待审批/错误读取现有服务端投影（`SessionRecord.status`）。
4. 项目内会话默认显示 5 条 + 「显示更多 N 条」展开，折叠状态按工作区在内存中隔离（不持久化，刷新重置）；排序按 `updatedAt` 倒序；切换会话不停止后台任务，轮询不扰乱列表位置或滚动。
5. 会话右键/菜单按钮统一一份菜单：重命名、归档（danger 语义、确认文案）。走既有 `UpdateProductSessionRequest { title, archived }`，无新合同。「在文件夹中显示」首批不做（Web 无宿主能力）；标签体系不做。
6. 会话级置顶：`SessionRecord` 当前无 `pinned` 字段。分两档处理——首批只交付无置顶字段的排序与分页；若要置顶，先提出兼容扩展（默认值、迁移、并发冲突、旧客户端兼容）并审查，不得用假按钮或 localStorage 冒充持久能力。失败恢复原排序并提示。
7. 查找分两层：本地即时过滤已加载项目/会话名（保留现状）+ 输入防抖后调服务端 `search` 参数（≤128 字节、配合 `next_cursor` 分页），区分"已加载范围匹配"与"服务端匹配"；未加载不能被报告为不存在；区分无项目、空项目、无匹配、部分列表和加载失败。
8. 底部设置区固定在视口底部不被长列表挤走；连接/服务状态只读展示；账号/配额/IM/更新横幅等 rove 无对应来源的能力不做假入口。
9. 宽度拖调（建议 200–360px，初值 240px）存 localStorage UI 偏好（产品偏好合同为类型化字段、不承载布局值，见设计文档 §2.2；如需服务端同步另走合同扩展）；收起态新建/查找入口移交页头；窄屏选中会话后收起导航并正确归还焦点。
10. 左栏文件/证据入口复用右栏选择动作，随 P3/P4 能力交付再启用；跨工作区清理旧内容，同工作区切换会话清理旧审批和会话证据。
11. 会话行状态更新沿用既有通道：聚焦 job SSE 与存在 running/needs_attention 会话时的目录轮询（设计文档 §2.2），不私建第二套事件流；多窗口以服务端投影重读为准，不为左栏放宽成全局常驻轮询。
12. 新增文案全部走 `apps/web/copy/` 体系（zh-CN/en-US 双字典同步补键、CopyProvider），新增动效尊重既有 prefers-reduced-motion 全局规则；cool/warm 两皮肤下分别过一遍新组件。
13. 左栏动效按设计 §3.3 落地：150ms 色过渡 + 选中瞬时上色、运行图标 1s 旋转 + contain/will-change、项目组 200ms 0fr↔1fr + 220ms 延迟卸载、收起 240ms transform（子树不卸载 + inert/aria-hidden）、菜单 100ms fade+zoom；全部引用新增 --motion-* 令牌，不改 v1/v2 既有样式，不引入 motion 库。

**退出条件**：切换、新建、菜单不冒泡、重名长标题、部分加载、深链接恢复、置顶失败、多窗口更新、后台状态提示、键盘和窄屏均有用例；三栏身份一致，不串草稿或证据。左栏纳入首批验收，不后置成可选美化。

**回滚**：保持原路由和产品合同，呈现层可退回旧导航；若扩展置顶字段则保持前向兼容，不删除已有会话。P1a/P1b 先确定共享 shell 与草稿边界，再接 P2 三栏联动。

### P1b：输入框与消息阅读体验（open-vetta 结构 + PI-Desktop 键盘契约）

**修改落点**：`apps/web/chat/Composer.tsx`、现有样式与 [copy](../../apps/web/copy/CopyProvider.tsx) 体系（zh-CN/en-US 双字典同步补键）、`Transcript.tsx`；必要时拆出纯输入组件及会话草稿 hook，命名按仓库约定。

1. 输入区结构与视觉对照 open-vetta `domains/conversation/components/input-bar/` 与设计文档 §4.0 对照表：消息流与底部 Composer 分层；输入区收敛为居中限宽卡片，工具行承载模型入口与 Review 入口；发送/停止同位双态（参考 `SendButton` 的 send/stop/pending 态与 abort 请求序号防旧回调复活）；只参考分层，不引入其 @ 提及、附件、语音、技能等无服务端合同的能力。
2. 先补 Playwright 行为用例：Enter 换行、Ctrl/Meta+Enter 发送、IME 不误发（`isComposing`/keyCode 229）、空白不发、失败不清空、busy 停止可用。
3. 按设计统一输入框、模型入口、发送与停止的视觉层级，不改变 Send Message 协议。
4. 使用同一 submit 入口处理按钮和快捷键，防 repeat/双击重复提交；服务端保持原幂等和单活动回合合同。
5. 草稿按 `workspace_id + product_session_id` 隔离；待发送快照持有原会话与草稿版本。异步成功只能清除匹配版本，不能清除新文本。
6. 明确异常处理，保留草稿并显示错误；结果不明继续走已有绑定确认，不自动重发。
7. 运行中且草稿非空时按服务端 Send Message 语义展示接收/排队状态，UI 不私建第二队列。
8. 消息阅读保留原滚动位置和部分历史提示；长结果摘要展开不引起强制跳到底部。
9. 输入区与滚动动效按设计 §3.3：pending 胶囊 180ms 0fr→1fr、下沿状态条非对称 300ms/200ms + 220ms 退场卸载、发送/停止图标简化双态（150ms）；滚动增强——贴底解除只认真实手势（wheel/touch/pointer/keydown）、跳转按 reduced-motion 选 auto/smooth；审批/输入卡采纳防重入与决策后焦点归还输入区。

**退出条件**：鼠标与键盘行为一致；中文输入、切换会话、超时和停止均有真实浏览器用例。现有渲染测试继续通过。富文本和附件不进入本阶段。

**回滚**：恢复旧呈现和输入组件即可；本阶段不引入服务端 schema 变化，不需要数据回退。

### P2：工作面板容器与当前审批（PI-Desktop 参考）

**修改落点**：既有产品 shell、消息详情入口、审批 controller；优先收敛 `apps/web/inspector/`（FilesPanel/DiffPanel/ArtifactPanel/ReviewPanel）为右栏页签内容，而非另建第二套面板；若新建容器组件，可采用 `WorkPanel` / `ApprovalDetail` 等名称，但这些是建议名，并非当前文件。

1. 右栏容器按设计文档 §5.0 对照 PI-Desktop `WorkPanel`：固定页签集（待处理/文件/变更/浏览）、三栏共享宽度预算（中栏 450px 硬底线优先，右栏让位、必要时自动收起左栏）、右栏宽度拖调与持久化、激活页签滚入视野、内容加载失败显示明确空态不留白屏。不引入 PI 的插件贡献页签机制。

2. 提取单一面板状态：页签、打开对象、宽度、焦点返回位置。只存 UI 选择，不存"是否已授权"的权威副本。
3. 消息卡与右侧详情复用同一个审批数据适配器与动作 controller。
4. 审批按钮发送精确请求标识和现有并发验证数据；处理已被其他窗口决定、run 结束和连接断开。
5. 输入请求与计划确认卡复用现有提交面（job 域 approvals/inputs、product 域 controls/steers/followups）与 `Transcript.tsx` 的 ApprovalCard/InputCard 语义；提交绑定请求 ID 与服务端并发校验，过时或已决请求由服务端拒绝；不新增提交协议。
6. `submitting` 只代表请求中；成功状态须由服务端确认或权威重读获得。
7. 切换会话取消或隔离旧读请求；若提交已发出，按原会话处理结果，不能更新新会话卡片。
8. 小屏改抽屉/消息流入口，计数、焦点与关闭行为可键盘操作。
9. 左右栏拖调把手补键盘调宽（方向键 ±16px、Shift ±32px、Home/End 到边界，PI WorkPanel.tsx 397–407 同规则）；拖动期间全局 col-resize 光标与禁选；宽度变化本身不加过渡，面板显隐用 200ms/150ms 进出场（§3.3）。

**退出条件**：拒绝、允许、过时、重复点击、跨会话、多标签页、刷新后恢复全部有测试。仅移动呈现不扩大审批权限。

**回滚**：保留消息流原审批入口；工作面板不可用时仍能完成同一审批流程。

### P3：工作区文件与只读预览

**修改落点**：现有产品文件 API/客户端、工作区安全解析、面板文件视图；先复用已有有界文件能力。

1. 明确读取合同包含工作区身份、相对路径、目录项类型、分页/继续游标和 partial 状态。
2. 首期建议 UI 请求一页不超过 200 条；以服务端已有更严格上限为准。不得为前端需要提高安全上限。
3. 文本读取建议单次最多 256 KiB、范围读取、显示截断；图片和二进制按现有 MIME/大小上限处理，不把二进制送文本渲染器。
4. 所有访问沿已有项目信任与工作区边界；被拒绝时显示原因和现有授权入口，目录点击不能隐式提升权限。
5. 目录展开按需加载；刷新可取消、过时结果不发布；大目录不一次扫描全库。
6. 只读预览展示文件名、相对路径、类型、加载/错误/已删除/过大状态；首期无编辑保存。

**负向用例**：父目录路径、绝对路径、链接越界、读取中切换工作区、读取中删除、非法编码、超大目录、隐藏/忽略目录、无信任项目。只做本地合成 fixture 防御性验证，不测试第三方目标。

**退出条件**：文件可查可看但不能越界；大目录和大文件降级明确；无本地敏感路径泄露。

### P4：授权历史与变更证据

**修改落点**：当前 canonical event/Artifact 投影、产品会话关联、API 客户端和 DiffView；Git 差异优先复用既有 [diff.rs](../../apps/api/src/product/diff.rs)（`GET /product/sessions/{id}/diff`，scope=run/git，基线 HEAD、有界、partial_reasons），不新建 git 命令面。只有确认缺少必要持久字段后才修改 Runtime/Event/ProductStore。

先完成下列字段可用性表，禁止先建一张全新的"真相表"。

| UI 需要的信息 | 首选来源 | 缺失时行为 |
|---|---|---|
| 请求和决策标识、范围、时间 | runtime StateStore `pending_approvals`（决策状态与时间戳已落库，无"决策者"字段）；产品域查询端点缺失，需走合同扩展 | 历史不可用或"谁"显示未知，不猜测 |
| session/run/tool call 关联 | 服务端拥有的产品绑定与 canonical 事件 | 标为未关联，不归到当前会话 |
| 当次文件变更内容 | 已有工具 Artifact、当次结果快照 | 无快照，不显示空 diff 冒充未修改 |
| 当前 Git 差异 | 既有 `GET /product/sessions/{id}/diff` 的 scope=git（有界、基线 HEAD） | 明确非 Git、失败、过时和采样时间；采样时间属新增需求时走合同扩展 |
| 修改成功/失败/未知 | 执行结果与保守恢复状态 | 未知不算成功 |

授权历史 UI 归属右栏待处理页签的"本会话最近授权记录"分区（设计文档 §5.1、§8）；在查询端点落地前该分区显示"历史不可用"。

若需要合同扩展：可选新字段有默认值；老 trace 可重放且显示部分信息；旧 ProductStore migration 不丢记录；新增索引是派生视图，可重建且不能替代 trace/task_state；同步 API/OpenAPI、Web 类型、持久化和 contract tests。

**退出条件**：重启后已知证据可恢复；重复事件去重；用户手工修改不被标为 Agent 修改；会话与工作区 diff 标签明确；无不可逆"撤销全部"入口。

**回滚**：隐藏新增视图即可；有 schema 迁移时遵循前向兼容，不通过降级删除已记录的授权事实。

### P5：外链打开与本地 HTML 预览

**修改落点**：P5a 落在 `apps/desktop/`（Tauri 受控命令/已有插件）与 Web 前端链接处理；P5b 的资源服务落在服务端（apps/api 或独立受控进程，实施前按威胁模型选定并记录，Web 端不新增原生能力）；P5c 落在 `apps/desktop/`。

**P5a 系统外链**：建立窄宿主适配层。建议返回可区分的 opened/blocked/unsupported/failed UI 结果，具体序列化类型在实施时批准。Web 在用户点击时同步打开受控新标签并隔离 opener；Tauri 经受控命令/已有插件打开。不得给 Renderer 任意 shell 执行能力。

**P5b 本地预览**：先写威胁模型，再实现资源服务和面板。绑定回环、随机令牌、独立 origin、无产品 cookie/IPC、明确会话与根目录归属、关闭后撤销访问；资源数量、大小、超时和并发均受限。HTML 执行前有明确用户动作和项目访问判断；不能自动运行 package scripts。令牌不进日志和证据截图。

**P5c 可选内嵌浏览器**：先验证 Tauri 平台支持、导航隔离、下载/弹窗/登录策略，再决定是否实现。无能力则外部打开和复制地址，不能以 iframe 成功加载一个站点宣称完整浏览器可用。

**退出条件**：外部链接、无效协议、无默认浏览器/启动失败、本地 CSS/JS/图片、中文路径、链接资源越界、预览关闭后的失效、未信任项目、加载超时有明确行为。Windows 安装版单独验收；未跑 macOS/Linux 不宣称跨平台通过。

### P6：综合验收与收尾

**产出**：全量验收证据与收尾合并；本阶段不新增产品行为。

1. 收口验收矩阵（§4）全部行：自动化证据由各阶段提供，P6 汇总、补缺口并逐行记录证据位置；未执行的行明确标注 skipped 及原因，不写 PASS。
2. 运行 `scripts/product-acceptance.ps1`（或 `.sh`）生成 `PRODUCT_ACCEPTANCE_REPORT.json`：真实退出码，不手工编辑报告；live API、mock 浏览器与安装版证据分开记录。
3. 全量回归：`pnpm --dir apps/web test`、`typecheck`、`build`、`test:e2e`；涉合同变更的阶段另跑 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（按 §5 裁剪规则）。
4. Windows 安装版 Desktop 完整旅程（三栏、输入、审批、文件、变更、外链）真实点击记录；macOS/Linux 未跑则记录未跑，不宣称跨平台。
5. 核对改动合同的阶段是否已同步 `docs/runtime/`，确认 `implementation-status.md`、`acceptance-matrix.md` 未被提前标 Met；旧能力（Review、Fork、Settings、会话恢复、Send Message）与既有 local-full 冒烟（含 `/dev/workbench`）回归通过。
6. 汇总每阶段已知缺口与后续任务，更新本计划状态与交付记录。

**退出条件**：验收矩阵无未解释缺口；报告与测试退出码齐全；遗留缺口逐条列出并转入后续计划，不自动关闭旧计划中的 F.4/F.5/G。

**回滚**：本阶段不含产品变更；验收不通过则该批合并退回，已独立合并的阶段沿用各自回滚方式。

## 3. 状态所有权与缓存规则

| 状态 | 所有者 | UI 可以做什么 |
|---|---|---|
| 会话、run、消息队列、审批结果 | 既有服务端/Runtime/ProductStore | 读投影，提交用户动作，按绑定重读 |
| 工作区访问与工具许可 | Runtime 既有安全边界 | 显示解释和现有授权入口 |
| 工具执行与 Artifact | canonical 事件和持久证据 | 分组、过滤、按需读取 |
| 文件系统、Git 现状 | 有界服务端读取 | 展示采样结果及过时状态 |
| 会话草稿 | UI 按 workspace + session 隔离，内存持有 | 首期不落盘（设计 §4.1）；不存权限或凭据 |
| 面板页签、展开/折叠状态 | UI 按会话/工作区隔离 | 内存缓存，刷新重置；不存权限或凭据 |
| 导航/面板宽度等布局偏好 | UI 全局或按工作区 | localStorage UI 偏好；产品偏好合同为类型化字段、不承载布局值（设计 §2.2） |
| 原生浏览器与预览生命周期 | 受控宿主/服务端 | 显示能力和状态，不持有原生特权 |

所有异步读取携带工作区、会话和本地请求版本；结果不匹配当前选择就不渲染。已发出的副作用请求不能通过取消 fetch 假定已撤销，按原身份查询结果。

## 4. 验收矩阵

| 编号 | 用户旅程 | 自动化层次 | 人工证据 |
|---|---|---|---|
| N1 | 展开项目、新建/切换会话、菜单操作 | 组件 + 浏览器 + 产品 API | open-vetta 对照截图、选中/hover/focus 区分、菜单不误导航 |
| N2 | 置顶（若合同扩展落地）、失败重试、多窗口刷新 | 产品持久化 contract + 浏览器 | 置顶恢复、失败不假成功、轮询不扰乱列表；置顶未落地时本行只覆盖失败重试与多窗口刷新 |
| N3 | 左栏跳转文件/证据及跨项目切换 | mock 浏览器 + live fake API | 三栏身份一致、旧内容不串会话、后台任务不被停止 |
| N4 | 长列表、深链接、窄屏导航 | Playwright | 部分加载可见、滚动/焦点恢复、底部设置可达 |
| U1 | 输入中文、换行、发送、失败重试 | 组件 + Playwright | Windows 中文 IME；macOS 单独记录 |
| U2 | 运行中发送、停止、会话切换 | mock 浏览器 + live fake API | 不丢草稿、不误清新会话 |
| A1 | 从消息跳到右侧审批，批准/拒绝 | API contract + 浏览器 | 焦点、窄屏、详情可读 |
| A2 | 两窗口同时决策、刷新恢复 | live fake API | 无双重执行、过时状态可见 |
| F1 | 展开目录、预览文本和图片 | API + 浏览器 | 大目录、中文路径、空状态 |
| F2 | 边界外文件和链接被拒绝 | Runtime/API 负向测试 | UI 无敏感内容泄露 |
| E1 | 执行文件修改后查看会话 diff | Runtime/Artifact + live API | 调用、审批、结果可对应 |
| E2 | 用户修改与会话变更并存 | fixture + 浏览器 | 标签不误归属、重启后不造假 |
| B1 | 系统浏览器外链打开 | Web + 宿主测试 | 安装版真实点击 |
| B2 | 本地 HTML/CSS/JS 预览 | 服务端边界 + 浏览器 | 加载、失败、刷新、关闭 |
| R1 | 320/375/414/768/1280/1440px | Playwright | 焦点、触摸、横向溢出 |
| R2 | 长消息、长工具输出、部分历史 | 浏览器状态测试 | 阅读时不强制滚动 |
| C1 | 新增文案与控件在 zh-CN/en-US 双语言、cool/warm 两皮肤下渲染 | 组件 + copy/ui-skin 单测 | 截断、溢出与焦点态截图 |
| M1 | 侧栏/面板/菜单/输入区动效在 reduced-motion 开与关下的表现 | Playwright reduced-motion 模拟 + 组件 | 既有全局断言继续通过；动效不替代状态文本与焦点可见性 |

性能指标先做同机器基线，再设阈值。建议目标为输入 10 KiB 文本无明显卡顿、目录/正文按需加载、切换页签不重建整个 transcript；这些不是本次已测结果。自动化 IME 事件模拟不能替代真实输入法验收。

## 5. 验证命令与证据规则

按变更范围选择，不把未执行的命令列为通过。

```powershell
# Web 阶段，从仓库根运行
pnpm --dir apps/web test
pnpm --dir apps/web typecheck
pnpm --dir apps/web build
pnpm --dir apps/web test:e2e

# 涉及 Rust/API/事件/路径边界时先跑受影响测试，再扩展
cargo fmt --all --check
cargo test -p rove-integration-tests --test api
cargo test -p rove-integration-tests --test tool_safety
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

live API、产品验收和安装版测试遵循[集成测试说明](../runtime/integration-testing.md)，按脚本帮助选择适合的参数。不得默认使用真实 Provider 或生产服务。机器报告必须来自真实退出码，不手工写 PASS。截图与日志使用合成数据、脱敏路径，不包含 keys、cookie 或预览令牌。

## 6. 每阶段合并条件

- 在脏工作区中保留用户改动，只提交该阶段授权范围。
- 相关用例通过，非平凡变更做独立只读审查。
- 改动合同的阶段同步更新 `docs/runtime/`；纯视觉阶段不虚构运行时新能力。
- 不提高状态文档的完成级别来代替测试；旧计划中的 F.4/F.5/G 不被本计划自动关闭。
- 已有 Review、Fork、Settings、会话恢复和 Send Message 行为继续可用。
- 依赖变化必须解释必要性；锁文件由工具更新，禁止为纯外观提前引入编辑器或浏览器框架。
- 每阶段附回滚方式、测试退出码、未运行平台及已知缺口。

## 7. 本次文档交付记录

- 已完成：参考仓库快进更新；输入组件及架构/当前状态证据核对；两份待讨论文档。
- 已运行：`pnpm --dir apps/web test chat/Composer.test.tsx`，1 test passed，退出码 0，仅静态渲染验证。
- 补充验证：`pnpm --dir apps/web test chat product-v2`，6 个测试文件、16 个测试通过，退出码 0；只证明现有模块测试通过，不证明新方案已实现或实机交互通过。
- 文档检查：相对链接、代码围栏、重复标题、尾随空白检查通过；`git diff --check` 通过。新增文件另做全文检查，避免未跟踪文件不被 Git diff 覆盖。
- 本次不改：产品源码、API/schema、锁文件、Runtime 当前完成状态、open-vetta 源码和 PI-Desktop 源码。
- 已确认：三栏布局；分栏参考于 2026-09-19 修订为左栏全面参考 open-vetta（本地快照 `19b3e712`）、中栏混合（输入区结构与视觉参考 open-vetta，键盘/IME/草稿/提交竞争契约沿用 PI-Desktop 调研）、右栏维持 PI-Desktop `8d826433`；具体交互和产品实现仍待确认。
- 2026-09-19 细化：设计文档新增 §3.1 左栏「open-vetta 结构 → rove 映射意向 → 现状差距」九行对照表、§4.0 中栏输入区结构对照表、§5.0 右栏 PI-Desktop WorkPanel 结构对照表；证据表补充 rove 产品合同事实（会话重命名/归档/搜索/分页已有，会话级置顶字段缺失）与 open-vetta 组件级事实（排序、菜单、发送按钮、宽度预算）。本计划 P1a/P1b/P2 同步到对应粒度。尚未完成：open-vetta 与 PI-Desktop 的实机点击/截图、安装版浏览器验收。
- 2026-09-19 二次修订（文档评审）：计划补 P6 综合验收章节、P5 修改落点、P2 重编号与输入请求契约项、P4 既有 diff/审批落库引用与授权历史归属、§3 所有权表持久化口径（草稿/页签内存、宽度 localStorage）、P0 既有协议清单、P1a 状态更新通道与 copy/动效条目及共享 shell 边界确认、验收矩阵 C1 双语言行。设计文档同步补 §2.2 七行证据（rove 会话 diff、偏好合同、SSE+条件轮询、审批落库；open-vetta 状态通道与动效纪律；PI 宽度持久化与无审批历史）、§3 文案/动效约束、§3.1 状态通道段、§4.0 会话头行、§5.0 授权历史归属与页签不持久化、§5.1 授权记录分区与输入请求契约、§6.2 既有 diff 复用、§8 两行决策。新增事实均在本会话按两个参考项目与 rove 当前源码核对（rove 侧另经行号抽查）；本次仅改两份文档，未跑代码测试。已核实 `app/dev/workbench` 与 `app/dev/product-ui-v2` 未导入本次改造的共享组件（前者仅用 components/rove-workbench，后者为自含 mock），无需为其单独回归。
- 2026-09-19 三次修订（能力对比与动效）：设计文档新增 §2.4 十一行"已有能力对比与采用判定"（rove 现状新证据：归档无 UI 入口、重命名仅在 Settings、侧栏全量渲染、Transcript 48px 贴底阈值 + Return to latest、DiffView 截断上限）与 §3.3 动效规范（--motion-* 令牌体系取自 PI、15 行对象级数值表、纯 CSS 实现不引入 motion 库、不采纳 open-vetta 去焦点环）；§2.2 补四行动效/滚动/审批倒计时证据；§8 补动效实现方式决策行。计划同步：P0 补动效实机核对，P1a 补第 13 条、P1b 补第 9 条、P2 补第 9 条（键盘调宽 ±16/±32px），验收矩阵补 M1 行。动效数值与交互细节来自本会话对 open-vetta 快照与 PI-Desktop `8d826433` 的源码核查；rove 现状证据同上。本次仅改两份文档，未跑代码测试。
- 尚未完成：open-vetta 与 PI-Desktop 的实机点击/截图（含左栏四段结构、会话行状态与分页、输入区工具行）、完整源码映射及安装版浏览器验收。
- 下一步建议：补齐 P0 双侧证据，确认具体交互后实施 P1a 左栏、P1b 输入与 P2 右栏/审批；本次不修改产品代码。

## 8. 实施交付记录（2026-09-17 追加，非原文档交付）

分支 `feature/pi-desktop-workbench-p1a`，基线 `7e54ab6`。全部为可审查的独立提交，均带真实退出码：

| 阶段 | 提交 | 状态 | 证据 |
|---|---|---|---|
| — | `954e33a` | `ArtifactPanel.tsx` 存在编译阻断（重复 `if (!stale()) {` 且缺 `requestRef` 声明），`tsc` 与 Web 构建均失败；已修复 | `pnpm typecheck` 0 |
| P2 | `1ca5c58` | 关闭评审遗留的三个源码风险：终态工具被过期审批快照改写回 `running`、media-query 监听器闭包捕获首帧 panel、移动端审批焦点回退缺失 | vitest 340 用例；新回归测试在移除守卫后实测失败 |
| P3 | `f684dcb` | 工作区文件读取接入项目信任边界：revoked 根在 listing/content/download/preview 四面返回 409 `project_trust_required`；unknown/restricted 保持可读；未配置信任权威时维持既有行为 | `cargo test -p rove-integration-tests --test api` 120 通过；移除守卫后负向用例实测失败 |
| P4 | `ce7395e` | 新增“本会话最近授权请求”分区：读取产品 transcript 的 `tool_call_approval_needed` 事件，去重、倒序、上限 50；决策/决策者/决策时间标注为未知并说明未持久化 | vitest 345 用例；`pnpm test:e2e` 77 通过、5 跳过 |
| P5a | `35e3be8` | 外链接入受控 Desktop 宿主：`SafeLink` 仅在 Desktop 传输存在时转交 `open_external`（http/https + host 复校），普通浏览器保持 `target=_blank` + `noreferrer noopener`；失败经 copy 体系提示 | vitest 350 用例（新增 5 条）；`pnpm build` 与 `pnpm test:e2e` 77 通过、5 跳过 |
| 动效/宽度 | `a59548a` | §3.3 动效令牌体系（150ms 色过渡、1s 运行旋转、200ms 0fr↔1fr + 220ms 延迟卸载、240ms 收起、100ms 菜单、180ms 胶囊、300/200 状态条、200/150 面板）、左栏 200–360px 拖调与键盘调宽 + localStorage 持久化、收起态入口移交页头、窄屏选中会话后焦点移入标题 | vitest 354 用例；`pnpm test:e2e` 80 通过、5 跳过 |
| P5b | — | 仅交付[威胁模型](../design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md)：origin 隔离为不可让步约束、落点选 `apps/api`、13 项攻击面与 6 条实施门槛；**未实现，模型不授权实现** | 文档交付，无代码 |
合同同步：`docs/runtime/subsystems.md` 记录文件读取的信任边界与授权历史仅请求侧的事实。未改 Runtime 事件流、未加 schema 迁移、未新增依赖、未改锁文件。

仍未关闭，不因上述推进而改写：

- P5a：外链已接入 Desktop 宿主，但 UI 层未做真实系统浏览器点击验收（安装版 Windows 旅程未跑）；`opened` 不等于页面加载成功。
- P5b：威胁模型已写，实现未做。门槛 1（证明产品令牌在预览 origin 不可用）需真实浏览器用例，未满足前保持未实现。
- P4 决策侧（决策者、决策时间、结果归因）与产品域历史查询端点属独立合同扩展。
- ProjectTrust 能力集在设计上不含通用文件读；若要纳入需 schema 默认值、迁移与旧客户端兼容审查。
- PI-Desktop 与 open-vetta 实机点击、安装版 Windows 旅程、真实输入法、macOS/Linux、外部 Provider、真实第三方 MCP 门禁均未运行。
