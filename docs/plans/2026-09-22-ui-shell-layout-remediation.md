# rove 三栏外壳布局修复与产品化补齐

> 状态：**Implemented（P0–P4）**。P0（阻断级网格缺陷）、P1a（设计 §5.0 共享宽度预算）、
> P1b（PI-Desktop 式分隔条、宽度持久化、左栏先让位/重开让位、预算上限取代固定上限）、
> P2（右栏动态页签）、P3（中栏统一阅读列）、P4（样式分层契约）均已在 worktree
> `fix/ui-shell-layout` 实现并有自动化回归；逐阶段实测退出码见 §3b–§3f。
> 两处与初稿不同的落地决定记录在 §3f 与 §6（保留 960 断点、保留 v1 布局层）。
> **2026-09-22 追加一轮独立复核**（§7）：用与本用例集无关的几何探针在 11 个视口量测，发现并修掉
> 两个自杀用例集漏掉的缺陷（左栏拖拽原点取到会随宽度移动的把手上；清空页签后残留非法 ARIA），
> 并补齐 `pnpm build` 门禁。§5 的阶段表原为"提议，待确认"，现已被 §3b–§3f 的实现取代。
> 日期：2026-09-22。
> 基线：`f853a97`（`origin/main`）。
> 参照：[工作台设计](../design/2026-09-17-pi-desktop-workbench-design.md) §3 / §4.0 / §5.0、
> [实施计划](2026-09-17-pi-desktop-workbench-implementation.md)。

## 1. 现象与实测证据

进入 `http://localhost:3000` 后三栏位置全错：工作面板出现在**左下角**、中栏右侧
留出大片空白、三栏高度都被砍半。用 `outputs/_layout-probe.cjs`（多视口量真实 DOM
几何）得到的实测值：

| 视口 | `.product-body` 计算列 | 左栏 | 中栏 | 右栏 |
|---|---|---|---|---|
| 1440×900 修复前 | `240px 298px 902px` | (0,52) 240×**424** | (538,52) 902×424 | (**0**,**476**) 360×424 |
| 1440×900 修复后 | `240px 840px 360px` | (0,52) 240×**848** | (240,52) 840×848 | (1080,52) 360×848 |
| 1024×768 修复前 | `240px 0px 784px` | (0,52) 240×358 | (240,52) 784×358 | (**0**,**410**) 360×358 |
| 1024×768 P0 后 | `240px 424px 360px` | (0,52) 240×716 | (240,52) 424×716 | (664,52) 360×716 |
| 1024×768 P1a 后 | `240px 450px 334px` | (0,52) 240×716 | (240,52) **450**×716 | (690,52) 334×716 |
| 768×1024 / 375×812 | 单列 | 抽屉 | 正常 | 抽屉（0×0） |

期望契约：`rail | conversation | panel` 同行，三者高度 = 视口 − 顶栏 52px。
窄屏（≤960）抽屉行为本来就正确，不受影响。

## 2. 根因（按影响排序）

1. **拖拽把手占用网格轨道**（阻断级）：`apps/web/styles/product-v2.css` 把
   `.sidebar-resize-handle` 设为 `grid-column: 2; grid-row: 1`，而
   `.product-body` 只有三列 `var(--sidebar-nav-width) minmax(0,1fr) auto`。
   把手吃掉第 2 列后，`<main>` 自动排到第 3 列（`auto` → 按内容 902px），
   `<aside.product-inspector>` 被排到第 2 行第 1 列 → 左下角面板 + 高度减半。
   把手本身 8px、透明、`margin-left: -4px`，视觉上还看不见。
2. **宽度有五个来源互相矛盾**：`--sidebar-width:248px`、内联
   `--sidebar-nav-width:240px`、`--inspector-width:304px`、React 内联
   `min(panel.width=360px,45vw)`、设计值（右栏默认 400、可拖 320–640）。
   `@media (max-width:1180px)` 的 `224px/268px` 已被后面的 `var()` 规则覆盖成死代码。
3. **右栏信息架构偏离设计**：实现只有 3 个页签 `run | review | approval`
   （活动/审查/待处理），Files/Artifact/Diff 全部叠在"活动"页签内部；
   设计 §5.0 要求固定四页签 **待处理 / 文件 / 变更 / 浏览**。
4. **宽度调节是可见的 `<input type="range">` 横条**，不是设计的 8px 分隔条 + 键盘。
5. **断点不一致**：JS `matchMedia("(max-width: 960px)")` 与 v2 CSS 用 960，
   设计 §3/§5.0 要求 1180 起右栏抽屉、760 起左右不同时占宽。
6. **三套样式表叠加**：`app/layout.tsx` 同时引入 `product.css`（v1，布局规则无作用域）、
   `product-v2.css`（v2 作用域）、`styles/v3/{tokens,base}.css`（warm skin，仅覆盖
   token/组件）。v1 的 `@media (max-width:960px) ... .product-main { display: none }`
   与 v2 抽屉语义冲突，任何 v1 规则缺 v2 覆盖都会漏出来。
7. **中栏阅读宽度未对齐**：设计为 680–840 且中栏 450 硬底线、宽度预算
   "中栏 > 右栏 > 左栏"；实现里中栏无底线，1024 下只剩 424。

## 3. P0（已实现）

- `apps/web/styles/product-v2.css`：`.product-body:not([data-settings="true"])` 加
  `position: relative`；`.sidebar-resize-handle` 改为绝对定位覆盖在左栏右缘
  （`left: var(--sidebar-nav-width)`、`top/bottom: 0`、8px、`margin-left: -4px`），
  不再占用网格轨道。
- 新增 `apps/web/tests/e2e/layout.spec.ts`：在 1440/1280/1024 断言
  "三栏同行、高度填满、中栏紧贴左栏、右栏贴视口右缘、无横向溢出"；
  1024 下的 450px 中栏底线用 `test.fixme` 显式记录为已知缺口（设计 §5.0 未实现）。

验证（worktree `fix/ui-shell-layout`，真实退出码）：

- 修复前跑该用例：**3 failed**（三个视口全部失败，证明用例可失败）；
- 修复后：`3 passed, 1 skipped`，退出码 0；
- `pnpm typecheck` 退出码 0；
- `pnpm exec vitest run`：46 文件 / 354 用例通过，退出码 0；
- 多视口几何见 §1 表；修复后的界面截图见 worktree `outputs/_shots-after/`。

未做：P1b（分隔条替换 range 滑块、宽度持久化、断点统一 1180/760）与 P2–P4；
未跑 `pnpm build`；未跑 Rust 门禁（本阶段无 Rust 变更）。

## 3b. P1a：共享宽度预算（已实现）

设计 §5.0 要求"中栏 450px 硬底线优先，右栏让位"。实现方式：由 `.product-body` 的
网格轨道承担，面板自身不再设宽（内联 width 会让面板无法收窄）。

- `apps/web/styles/product-v2.css`：`.product-body:not([data-settings="true"])` 定义
  `--pane-floor: 450px` 与 `grid-template-columns: var(--sidebar-nav-width) minmax(var(--pane-floor), 1fr) var(--work-panel-track, auto)`；
  `.product-inspector` 的 `width` 由 `304px` 改为 `100%`（填满所属轨道）；
  删除 `@media (max-width:1180px)` 里已被覆盖的 `224px` 轨道与 `width: 268px`
  —— 后者此前被内联宽度掩盖，一旦面板改为填满轨道就会静默破坏预算。
- `apps/web/shell/ProductApp.tsx`：在 `.product-body` 内联
  `--work-panel-track: min(<请求宽度>px, calc(100% − var(--sidebar-nav-width) − var(--pane-floor)))`；
  左栏收起时用 `minmax(0, var(--work-panel-collapsed-width, 40px))`。
- `apps/web/inspector/RunInspector.tsx`：仅 v1 皮肤保留内联宽度（v1 没有该轨道），
  通过新增的 `uiVersion` 属性区分。
- `apps/web/tests/e2e/layout.spec.ts`：把原先的 `test.fixme` 转为正式断言——1024 下
  中栏 ≥450、右栏 ≤ `1024 − 左栏 − 450`、三栏宽度和等于视口。

实测（探针 `outputs/_shots/layout-probe-budget.json`）：

| 视口 | 列 | 左栏+中栏+右栏 | 中栏 ≥450 | 横向溢出 |
|---|---|---|---|---|
| 1440×900 | `240px 840px 360px` | 1440 | 是 | 无 |
| 1280×800 | `240px 680px 360px` | 1280 | 是 | 无 |
| 1024×768 | `240px 450px 334px` | 1024 | 是 | 无 |

验证：`pnpm exec playwright test layout.spec.ts workbench-panel.spec.ts shell.spec.ts workbench-navigation-motion.spec.ts polish.spec.ts` → 27 passed，退出码 0；
`pnpm typecheck` 退出码 0；`pnpm exec vitest run` 46 文件 / 354 用例通过，退出码 0。

## 3c. P1b：PI-Desktop 式分隔条与让位策略（已实现）

对照 PI-Desktop `components/workpanel/WorkPanel.tsx` 与 `lib/work-panel-resize.ts` 逐项移植：

- 新增 `apps/web/inspector/work-panel-layout.ts`（纯模块，10 个单测）：`WORK_PANEL_MIN_WIDTH=244`、
  `WORK_PANEL_DEFAULT_WIDTH=360`、`WORK_PANEL_COMPACT_MIN_WIDTH=1`、`MAIN_PANE_MIN_WIDTH=450`、
  `MAIN_PANE_REOPEN_TARGET_WIDTH=460`；`workPanelLayout`（**上限 = 实时预算**，不再有 560 固定上限）、
  `workPanelWidthForSidebarReopen`、`workPanelKeyboardWidth`（左/右键 ±16、Shift ±32、Home=min、
  **End=实时上限**）、`parseStoredWorkPanelWidth`。比 PI 多一处硬化：非有限测量值按"无空间"处理，
  不再传播 `NaN`。
- 新增 `apps/web/inspector/use-panel-resize.ts`：分隔条指针拖拽（pointer capture + 单帧 rAF 合并 +
  松开提交 + Escape 取消 + 卸载清理 + `html[data-work-panel-resizing]` 全局光标），对齐 PI 的分隔条语义。
- `apps/web/inspector/RunInspector.tsx`：删除可见的 `<input type="range">` 滑块，改为面板左缘
  `role="separator"` 的 8px 分隔条（aria-valuemin/max/now、tabIndex、指针与键盘）。
- `apps/web/inspector/use-work-panel.ts`：宽度改为 localStorage UI 偏好（`rove.ui-work-panel-width`），
  仅约束下界，渲染宽度由预算决定；新增 `resizeByKeyboard`。
- `apps/web/shell/ProductApp.tsx`：ResizeObserver 量测 shell 宽度（回调 ref，避免 boot 未就绪时
  错过观察），计算 `workPanelLayout`；**左栏先让位**（`shouldCollapseSidebar` 触发自动收起），
  手动重开左栏时先用 `workPanelWidthForSidebarReopen` 收窄右栏而不是挤压中栏。
- 预算的唯一真源是 JS：CSS 轨道只消费 `panelLayout.panelWidth`，并保留
  `calc(100% - var(--pane-floor))` 作为测量竞态兜底。此前 CSS 用**展开态**的左栏宽度重复推导上限，
  一旦左栏收起就会与 JS 分歧（实测：`aria-valuemax` 830 而渲染 590）。

实测（`pnpm exec playwright test layout.spec.ts workbench-panel.spec.ts`）：

| 场景 | 结果 |
|---|---|
| 1440×900 / 1280×800 | 左栏 240 保留，中栏 840 / 680，右栏 360 |
| 1024×768 | 中栏命中 450 底线 → **左栏自动收起**，右栏保持 360，中栏 ≥450，无横向溢出 |
| 1024 手动重开左栏 | 右栏让位到 `1024 − 240 − 460 = 324`，中栏 460 |
| 键盘 | Home=244、ArrowLeft +16、Shift+ArrowLeft +32、End=实时上限；到下限后 ArrowRight 不越界 |

验证：27 条 e2e 通过（layout/workbench-panel/shell/polish/navigation-motion），退出码 0；
`pnpm typecheck` 退出码 0；`pnpm exec vitest run` 47 文件 / 364 用例通过，退出码 0。

P1b 遗留复核（同日完成）：

- 断点：不改为 1180/760。核实后 960 在 JS 与 CSS 两侧**本来就一致**（v2 的两处
  `@media (max-width: 960px)` 与 `ProductApp`/`use-work-panel` 的 `matchMedia` 都是 960），
  1180 处只有中栏行宽微调、与面板无关。真正的问题是"同一个数字散落四处"，因此改为
  单一来源：新增 `lib/viewport-breakpoints.ts`（`DRAWER_MAX_WIDTH = 960`、
  `DRAWER_MEDIA_QUERY`、`matchesDrawerLayout()`），`ProductApp` 与 `use-work-panel`
  改为消费它，两份 CSS 在该断点处加注释指向该模块。改值只影响抽屉/对话框的开启宽度，
  与设计中的"三栏 ≥1024 一行"不冲突（1024 仍为三栏，已由 `layout.spec.ts` 钉住）。
- `--inspector-width` / `--sidebar-width` **不是死变量**：`ROVE_PRODUCT_UI_VERSION=v1`
  仍是一条受支持的开关（`app/(product)/layout.tsx`），`product.css` 的 v1 布局确实消费
  它们（`product-body` 的 `grid-template-columns`、`product-inspector` 的 `width`）。
  删除会破坏 v1 路径，因此**不删**，改为在 §3f 明确三层的归属契约。

## 3d. P2：右栏动态页签（已实现）

对照 PI-Desktop `components/workpanel/WorkPanel.tsx` + `lib/work-panel-tabs.ts` 移植，去掉 rove
没有的插件视图，保留全部**动态**语义：页签可开、可关、可原地替换、按会话各自持有。

- 新增 `apps/web/inspector/work-panel-tabs.ts`（纯模块，13 个单测）：kind 集合
  `new | pending | status | files | changes | review`；`defaultWorkPanelTabs`（新会话只开"运行"）、
  `openWorkPanelTab`（已开则只激活，不重复）、`closeWorkPanelTab`（激活后继/前驱，允许清空）、
  `activateWorkPanelTab`、`replaceWorkPanelTab`（`+` 启动页被选中的 kind 原地替换）、
  `sanitizeWorkPanelTabs`（丢弃未知 kind/重复并修复 activeKind）、`workPanelTabStep`（左右环绕、Home/End）。
- `apps/web/inspector/use-work-panel.ts`：`PanelSelection.tab` → `tabs: WorkPanelTabsState`，
  新增 `openTab/activateTab/closeTab/replaceTab`；`open(kind, target, trigger)` 默认沿用当前激活 kind。
  页签状态随会话（`RunInspector` 以 `workspace:session` 为 key），切会话/刷新不串。
- `apps/web/inspector/RunInspector.tsx`：页签条改为 `role="tablist"` 动态渲染，每个页签带独立关闭按钮，
  尾部 `+` 启动按钮放在 tablist **之外**（只有 tab 属于 tablist）；roving tabIndex（选中 0、其余 -1）、
  左右/Home/End 移动选择并转移焦点、Delete/Backspace 关闭、中键关闭、选中页签 `scrollIntoView`、
  `role="tabpanel"` + `aria-labelledby`/`aria-controls`。正文按激活 kind 真正条件渲染（不再把 8 个分区
  堆在一个页签里）：
  - 待处理：审批卡详情 + 待批工具列表 + P4 会话级授权；
  - 运行：导出、空/加载/错误态、度量、时间线、计划、工具列表；
  - 文件：文件树（含审查发现定位）；
  - 变更：改动文件、工件、diff；
  - 审查：`ReviewPanel`（有进行中的审查时自动开页签）；
  - 启动页：五个 kind 的入口；全部关闭后正文提示"没有打开的页签，用 + 打开一个"。
- 审批触发 `panel.open("pending", …)`、审查发现跳转 `panel.open("files", …)`：不再依赖"三个固定页签
  永远存在"，关闭过的页签必须能被重新打开。
- 文案：`inspector.tabRun`/`inspector.tabApproval` 退役，新增 `tabsLabel`/`tabNew`/`tabPending`/`tabStatus`/
  `tabFiles`/`tabChanges`/`openTab`/`closeTab`/`launcherTitle`/`noTabs`，zh-CN 与 en-US 同步。
- CSS：页签条由 grid 固定列改为 flex 滚动条带（页签数量运行时变化），关闭按钮在 hover/focus/选中时显现。

修复过程中发现并修掉一个真实缺陷：`activeKind` 用 `??` 回退到内置 fallback，导致"关掉最后一个页签"后
tablist 已空、正文却仍渲染"运行"内容。状态源改为按 panel 是否存在选择，不再用 nullish 回退。

实测：`layout.spec.ts` 新增 "work panel tab strip opens, closes and reopens tabs"，覆盖默认单页签、
`+` 启动页 → 原地替换、roving tabIndex、方向键、Delete、中键、关闭按钮、清空后的回退入口；
`workbench-panel.spec.ts` 的文件分页用例改为经 `+` 打开"文件"页签（关闭过的页签必须能被重新打开）。
`pnpm exec playwright test layout workbench-panel shell polish navigation-motion`：31 passed，退出码 0。

## 3e. P3：中栏统一阅读列（已实现）

设计 §3.3 要求中栏只有一个阅读测量宽度，composer 与正文同宽：

- `apps/web/styles/product-v2.css` 新增 `--reading-column-max: 840px`（仅 v2 层）。
- `.chat-transcript` 左右内边距固定 16px；`.chat-transcript__content` 为
  `width: 100%; max-width: var(--reading-column-max); margin-inline: auto`。
- `.chat-composer` 改为 `width: calc(100% - 32px); max-width: var(--reading-column-max);
  align-self: center`，窄屏移动端按 10px 内边距对应 `calc(100% - 20px)`。
  两者解析出的盒子完全相同（列宽 − 32px，上限 840px），因此在任何宽度下左右边缘都对齐。
- 第一阶段实现用 `max(16px, calc((100% - var(--reading-column-max)) / 2))` 与
  `min(100% - 32px, var(--reading-column-max))`，实测**两条声明都失效**（计算值回落到初始值：
  `.chat-transcript` padding 变成 0，composer 宽度变成内容宽度 403px）。已在 §3e 定稿为不含
  裸算术的写法（`max-width` + `calc(100% - 32px)`），并由几何用例钉住。

实测（`layout.spec.ts` 新增 3 条）：1440×900 中栏 840 → 阅读列 808；1280×800 中栏 680 → 648；
375×812 移动端 355。三种视口下 `|正文宽 − composer 宽| ≤ 1`、`|正文 x − composer x| ≤ 1`、
左右留白差 ≤ 2、无横向溢出，退出码 0。该改动不新增任何动效，`polish.spec.ts` 的 reduced-motion
断言继续覆盖动效一致性。

## 3f. P4：样式分层契约（已实现，结论与初稿不同）

层级事实（`app/layout.tsx` 的导入顺序即层叠顺序）：

| 层 | 文件 | 归属 |
|---|---|---|
| 1 | `styles/product.css` + `styles/tokens.css` | 基础重置、共享组件类、**v1 布局**（`ROVE_PRODUCT_UI_VERSION=v1`） |
| 2 | `styles/product-v2.css` | 默认 v2 表现与三栏外壳布局，全部规则限定在 `[data-ui-version="v2"]` |
| 3 | `styles/v3/index.css`（`tokens.css` + `base.css`） | 设计 token 与可选 `data-skin="warm"` 皮肤，后加载故可覆盖 1–2 层 |

结论：初稿"v1 布局段迁入 v2 后删除"**不执行**。v1 是受支持的开关路径（`ProductUiVersion` 含
`"v1"`，由环境变量选择），删除会破坏它，且 `--sidebar-width`/`--inspector-width` 只有 v1 消费。
因此 P4 的落地方式是把分层契约写进代码，让后续改动不会越层：

- 三个样式表头部各加一段层级说明（谁是第几层、v2 规则必须限定 scope、第 3 层只能 token/skin）。
- `product-v2.css` 头部额外声明：宽度预算的唯一真源是 `shell/ProductApp.tsx` +
  `inspector/work-panel-layout.ts`，该表只消费 `panelLayout.panelWidth`，不得再从别的列反推预算；
  960 断点镜像 `lib/viewport-breakpoints.ts`。
- `docs/runtime/` 没有描述过这两个样式表（已核对全部 `docs/runtime/*.md`），因此本阶段不产生
  运行文档改动；分层事实记录在本文件与样式表头。

验证（P1b + P3 + P4 合并后一次性跑完）：

| 门 | 结果 | 退出码 |
|---|---|---|
| `pnpm exec playwright test layout workbench-panel shell polish navigation-motion` | 31 passed | 0 |
| `pnpm typecheck` | 无错误 | 0 |
| `pnpm exec vitest run` | 48 文件 / 377 用例通过 | 0 |

（含 `polish.spec.ts` 对 warm skin 与 reduced-motion 的回归。）

## 4. 目标（以设计文档为准）

- 三栏**一行**：左 200–360（默认 240，拖调 + localStorage）｜中 ≥450 硬底线、
  阅读列 680–840｜右 默认 400、可拖 320–640。
- 宽度预算优先级：**中栏底线 > 右栏请求 > 左栏**；不足时先自动收起左栏，
  再让右栏变抽屉。
- 断点：`<1180` 右栏抽屉；`<760` 左右不同时占宽。
- 右栏固定四页签：待处理 / 文件 / 变更 / 浏览（页签内存态，只持久化宽度）。
- 拖拽 = 8px 分隔条 + 键盘（16 / Shift 32 / Home·End），保留
  `role="separator"` 与 `aria-valuenow`。

## 5. P1b–P4（原提议表，已由 §3b–§3f 的实现取代）

> 本表是复核阶段的初稿提议。实际落地见 §3b–§3f 与 §7；两处偏离（保留 960 断点、保留 v1 布局层）
> 见 §3c/§3e/§6。P2 的"形态待用户确认"已按 §6 第 1 条落为动态页签，因此本表仅作历史记录。

| 阶段 | 内容 | 退出条件 |
|---|---|---|
| P1b | 宽度来源收敛为 `--sidebar-width` + 单一面板宽度变量；右栏 range 换成 8px 分隔条（保留键盘/aria）；面板宽度持久化到 localStorage；JS/CSS 断点统一 1180/760；预算不足时"先自动收起左栏" | 分隔条鼠标+键盘用例；刷新后宽度恢复；1180/760 断点行为与设计一致 |
| P2 | 右栏信息架构（**形态待用户确认**，见 §6 第 1 条）：待处理 / 运行状态 / 文件与变更（审查仅在有审查项时出现）或按设计四页签 | 页签用例 + 浏览器用例通过；设计 §5.0 对照表逐行可核 |
| P3 | 中栏阅读列 680–840 居中、composer 同宽、按 §3.3 核对动效 | Playwright 视觉/几何用例；reduced-motion 断言 |
| P4 | 样式表层收敛（v3 只留 token/skin，v1 布局段迁入 v2 后删除）并同步 `docs/runtime/` | `pnpm test/typecheck/build/test:e2e` 全绿；文档与代码一致 |

## 6. 实施期的决策结论

1. **右栏信息架构（P2 的前置）**：已按参考实现落地为**动态页签**，而不是"四页签固定映射"。
   五个 host kind（待处理 / 运行 / 文件 / 变更 / 审查）共享 PI-Desktop 的开、关、替换语义，
   新会话默认只开"运行"；审查页签在出现进行中的审查时自动打开。用户的意向"右栏 = 当前
   的一些状态"由默认页签 + 计数徽标（待处理 / 审查）满足，同时不再把 8 个分区堆在一个页签里。
2. **断点**：保留 960，不改为 1180/760（理由见 §3c 复核）。改的是"单一来源"而不是数值。
3. **右栏默认宽度**：保留 360（P1a/P1b 的预算与 e2e 均以它为准；改成设计里的 400 需要同时
   重跑预算用例与 1024 让位用例，收益只是观感，留作后续可选项）。
4. **`_start-stack.ps1` / `_layout-probe.cjs` 提升到 `scripts/`**：**未做**（可选）。它们现在只在
   worktree 的 `outputs/` 下作为本地证据工具使用；若要给 CI 复用，需要先脱敏并确认仓库的
   `scripts/` 约定，属于独立变更。

## 7. 独立复核与由此发现并修掉的缺陷（2026-09-22）

复核动机：`layout.spec.ts` 等断言是本轮修复自带的，通过它们只能证明"代码与自己的假设一致"，
不能证明"界面是对的"。因此另写了一个与用例集无关的几何/交互探针
`outputs/_verify-sweep.cjs`（未纳入版本控制）：在 11 个视口量真实 DOM，再验证分隔条键盘/拖拽/
Escape、页签生命周期、左栏让位与重开、刷新持久化，逐项打印原始数字而非只打印通过/失败。

### 7.1 实测宽度扫描（真实退出码 0，140 项全通过）

| 视口 | `.product-body` 计算列 | 左栏 | 中栏 | 右栏 | 横向溢出 | 左栏收起 |
|---|---|---|---|---|---|---|
| 1440×900 | `240px 840px 360px` | 240×848 | 840×848 | 360×848 | 0 | false |
| 1280×800 | `240px 680px 360px` | 240×848 | 680×848 | 360×848 | 0 | false |
| 1180×900 | `240px 580px 360px` | 240×848 | 580×848 | 360×848 | 0 | false |
| 1100×900 | `240px 500px 360px` | 240×848 | 500×848 | 360×848 | 0 | false |
| 1024×768 | `0px 664px 360px` | 收起 | **664**×848 | 360×848 | 0 | true |
| 1000×900 | `0px 640px 360px` | 收起 | 640×848 | 360×848 | 0 | true |
| **961×900** | `0px 601px 360px` | 收起 | 601×848 | 360×848 | 0 | true |
| **960×900** | `960px`（单列） | 抽屉 | 960×848 | 抽屉（0×0） | 0 | false |
| 900 / 768 / 375 | 单列 | 抽屉 | 占满 | 抽屉 | 0 | false |

要点：961/960 断点两侧都正确（961 仍在三栏内，由左栏让位保住 450 底线；960 起转抽屉）；
每个视口三栏 y 相同、高 = 视口 − 52、相邻不重叠、宽度之和 = 外壳宽度、分隔条恒为 8px。
1024 下中栏从修复前的 424 变为 664（设计 §5.0 的 450 底线不再被击穿）。

### 7.2 缺陷 A（**本轮 P0 之前就存在**，已修）：左栏拖拽把宽度算到了把手上

`use-sidebar-width.settleWidth` 原本用**把手自己的 rect.left** 作原点：`width = clientX − rect.left`。
但把手钉在左栏右缘、随宽度一起移动（P0 之后 `left: var(--sidebar-nav-width)`），于是抓取动作本身
就把宽度算成 `240 − 236 = 4px`，被钳到 200 下限；之后每一帧又从移动后的把手重新量，永远回不来。
实测：1440 下把把手向右拖 40px，左栏从 240 掉到 200。

A/B 判定责任归属：把 origin/main 的把手几何（网格项、`grid-column: 2`）注入当前页面后重跑同一拖拽：

```
[A] 当前 CSS（P0 绝对定位覆盖）：handleLeft=236，+40px 拖拽后 rail=200
[B] origin/main 几何（网格项，注入）：handleLeft=236，+40px 拖拽后 rail=200
```

两种几何完全相同 ⇒ **P0 没有改变该行为**，是既有缺陷，不是本轮引入。

修复：新增纯函数 `sidebarWidthFromPointer(clientX, containerLeft)`，`settleWidth` 改从**外壳**
（把手父元素，不随左栏移动）取原点，使"指针位置 = 请求的左栏宽度"。修复后同一探针：+40px ⇒ 276–284。

### 7.3 缺陷 B（**P2 引入**，已修）：清空页签后残留非法 ARIA

动态页签允许关到 0 个页签（P2 有意设计）。此时 `role="tablist"` 仍渲染但**不拥有任何 tab**
（ARIA `aria-required-children` 违规），`role="tabpanel"` 也仍在且 `aria-labelledby` 为 undefined
（无名的 tabpanel）。两者都可达：点最后一个页签的 ✕ 即可。

修复：`role`/`aria-label`/`aria-labelledby` 改为按状态输出——`tabs.length > 0` 才有 tablist，
`activeKind !== null` 才有 tabpanel；空态继续显示 `inspector.noTabs` 引导文案。

### 7.4 复核中发现的其他事实（不改代码，记录备查）

- **`--work-panel-collapsed-width` 全仓未被定义**（`git grep` 仅 `ProductApp.tsx` 一处引用），
  因此内联轨道 `minmax(0, var(--work-panel-collapsed-width, 40px))` 实际恒取 40px 兜底。
  该值与 origin/main 既有的 `.product-inspector[data-collapsed="true"] { width: 40px }` 一致，
  所以行为正确（探针实测宽屏收起后右栏 = 40px、无溢出），但这是个"引用未定义变量"的味道问题，
  后续可把它改成显式常量或补上变量定义。
- 宽屏收起时，8px 分隔条仍在 40px 残条内（探针 INFO 记录）。不影响布局与可操作性，属观感项。
- 本轮探针自身有过 3 处断言写法错误（`locator().locator()` 只查后代；把"重新推导后的
  aria-valuemax"与按下瞬间的 now 相比；左栏拖拽方向断言），已修正后重跑；修正过程与原始
  失败输出保留在 `outputs/_sweep.*.log`，未美化。
- PI-Desktop 参考实现路径 `D:\Study\project\agent\third-party-agents` **可用**（本文件早前的
  摘要曾误记为不可达），`work-panel-resize.ts` 的 `244/360/450/460/1` 常量与
  `workPanelLayout`/`workPanelWidthForSidebarReopen` 语义已逐行比对一致；差异仅为 rove 侧新增的
  键盘步进与 `finiteOr` 防御（PI 的 `shouldCollapseSidebar` 用原始请求值，rove 用钳后值，
  仅影响非整数输入且 ≤1px）。

### 7.5 追加复核后的门禁（均为真实退出码）

| 门 | 结果 | 退出码 |
|---|---|---|
| `pnpm exec playwright test`（全量浏览器用例，非仅本轮涉及文件） | 91 passed / 5 skipped / 0 failed；`.last-run.json`=`passed` | 0 |
| `pnpm exec vitest run` | 48 文件 / 380 用例通过 | 0 |
| `pnpm typecheck` | 无错误 | 0 |
| `pnpm build` | 编译成功（此前从未跑过，本轮补齐） | 0 |
| 独立几何探针 `outputs/_verify-sweep.cjs` | 140 passed / 0 failed | 0 |

新增回归：`use-sidebar-width.test.ts` 3 条（含"抓取不得把左栏压到下限"的缺陷 A 回归）、
`layout.spec.ts` 2 条（左栏拖拽后三栏仍成立；左栏键盘 16/32/Home/End），并在既有页签用例里
补 `tablist`/`tabpanel` 空态断言（缺陷 B 回归）。

## 8. 参照 PI-Desktop / open-vetta 的对齐（第三轮，2026-09-22）

复核之后按"照参考项目的好做法改"逐项比对，读的是：

- PI-Desktop：`apps/desktop/src/components/ConversationWidthHandles.tsx`（D439）、
  `packages/shared/src/chat-content-width.ts` + 其单测、`apps/desktop/src/styles/chat-shell.css`
  （阅读栏与手柄样式）、`styles/responsive.css`、`hooks/use-armed-delete.ts`、
  `features/app/AppShell.tsx`、`styles/tokens.css`。
- open-vetta：`apps/desktop/src/renderer/root-layout/RootLayoutView.tsx`、`useRootLayoutModel.ts`、
  `.agents/skills/web-design-guidelines/SKILL.md`、`.agents/skills/frontend-design/SKILL.md`。

### 8.1 本轮落地（两处按参考实现改写）

**A. 会话阅读栏变成双向可调（移植 PI D439）。** 此前 §3e 只做到"中栏统一阅读列"，宽度是固定
840px、用户不能改。参考实现是**两侧各一个手柄**、共同驱动一个**偏好最大宽度**（实际渲染
`min(可用, 偏好)`），因此列被压缩时不会改写用户偏好。

- 新增 `chat/reading-width.ts`（纯逻辑，9 条单测）＋ `chat/use-reading-width.ts`（偏好持久化）
  ＋ `chat/ReadingWidthHandles.tsx`（双柄、指针捕获＋rAF 预览、Escape 取消、双击复位、
  侧别相关的方向键、`aria-valuetext`），CSS 见 `product-v2.css` 的手柄段。
- 参考实现的四条语义**逐条照搬**：1px 指针 = **2px** 宽度（两侧镜像）、方向键对"哪一侧"敏感、
  Home 回默认、End 吃满整列；拖动期间 `html[data-reading-resizing]` 让全窗口切 `col-resize`
  且关闭过渡。
- 与参考的三处**有意偏离**：

  | 项 | 参考 | 本仓库 | 理由 |
  |---|---|---|---|
  | 上下界 | 560 – 窗格宽，默认 760 | 560 – **840**，默认 **840** | 上界取设计 §3.3 的 840 阅读尺度（窗格再宽也不该把行拉长）；下界取参考的 560，否则在 1024 这一档控制完全动不了 |
  | 存储 | 应用设置（服务端） | `localStorage: rove.ui-reading-width` | 设计 §2.2：产品偏好契约不承载布局值（与既有 `rove.ui-sidebar-width` 一致） |
  | 边距 | 硬编码 24px/侧 | 从 `.chat-transcript` 计算样式读取内边距 | 避免把 CSS 常量复制到 JS——本外壳已经因这类分歧修过一次 |

- 顺带把三个分隔条的 `aria-valuetext` 补上（参考有），并把左栏手柄里硬编码的 `200/360` 改为
  引用 `SIDEBAR_MIN_WIDTH/SIDEBAR_MAX_WIDTH`（消除一处常量重复）。

**B. 危险操作改为二次确认（移植 PI `use-armed-delete`）。** 侧栏"从列表移除工作区"**一次点击**就
执行，而它会一并从目录里删掉该工作区的会话（`workspace.pathRulesBody` 自己写明）；同一操作的
确认文案此前只存在于设置页。现在：

- 新增 `shell/use-armed-delete.ts`（与参考同形，`ARMED_DELETE_MS = 3200`，到期自动解除）。
- 菜单项第一次点击只**进入待确认**并改标签（新增 `workspace.removeWorkspaceArmed`），菜单保持
  打开；第二次点击才移除；菜单被关闭/Escape/失焦/重新打开都会清除待确认状态。
- 覆盖：`tests/e2e/sidebar-armed-delete.spec.ts` 2 条（首击不移除＋次击移除；到期自动解除＋
  关闭后重开不残留），并同步改掉 `continuity.spec.ts` 里原本"一击即移除"的既有用例。

**C. 外壳几何不再穿过 React 状态（先量后改，不是照搬 open-vetta 的结论）。**

open-vetta 在 `RootLayoutView` 里把路由内容 `memo` 掉，理由是"侧栏折叠状态不隔离则每次 toggle
重渲染整棵内容树（~28ms）"。本仓库结构上同源（`ProductApp` 内联渲染 `Transcript`，几何状态也在
其中），但结论不能照搬，所以先测：`outputs/_profile-shell.cjs` 用 CDP `Profiler` + `Performance`
指标，对**鼠标动作完全相同**（24 步 × 4px）的三种拖拽取 CPU 自耗时，并按 Next dev chunk 名归因。

| 拖拽 | 修前 ScriptDuration | 修后 | 说明 |
|---|---|---|---|
| **左栏把手**（每 pointermove 提交 state → 整棵 shell 重渲染） | **604.6 ms**（react/vendor 163.0、**chat 18.3**） | **89.0 ms**（react/vendor 33.4、**chat 3.0**） | 本轮修的就是它 |
| 右栏分隔条（预览态在 RunInspector 内部） | 147.0 ms | 未改动 | 成本是面板自身逐帧预览，不是内容树 |
| 阅读栏手柄（叶子预览 + 命令式 CSS 变量） | 40.7 ms | 未改动 | 本轮新代码本来就是这个形态 |

**中途纠正过一次自己的结论**：最初拿"右栏 vs 阅读栏"做 A/B，把它当作"内容树重渲染"的证据——
那是错的，右栏拖拽并不会重渲染内容树（ProductApp 只在提交时渲染一次），147ms 主要是
`RunInspector` 自身的逐帧预览。真正做到"内容树逐帧重渲染"的是**左栏**路径（`chat` chunk 在左栏
拖拽时执行 18.3ms，其它两种只有 0.6–2.3ms），修在左栏才对。

改动：

- `use-sidebar-width`：`settleWidth`（每次 pointermove 都 `setState` + 写 localStorage）拆成
  `previewWidth`（直接写 `.product-body` 的 `--sidebar-nav-width` 与手柄自身的 `aria-valuenow`，
  不经过 React）/ `commitWidth`（释放时提交一次 state + 持久化）/ `cancelPreview`（Escape 或
  pointercancel 时把 CSS 变量与 aria 还原成已提交值）。这正是 PI 分隔条注释里写的策略，
  而本仓库原实现与自己的注释相反。
- `SidebarResizeHandle`：pointerup 提交、pointercancel/lostpointercapture/window blur 取消、
  Escape 取消（与右栏、阅读栏两个分隔条一致）。
- `ProductApp`：ResizeObserver 回调先按最新输入算一次预算，**结果没变就不 setState**（窗口缩放
  不再逐像素重渲染内容树）。视觉仍然精确：轨道是 `min(panelWidth, calc(100% - var(--pane-floor)))`，
  `100%` 由 CSS 实时求值，所以 JS 状态落后一帧不影响几何。`expandRail()` 改读实时 ref，避免用到
  过期的宽度。
- 结果：左栏拖拽 604.6 → **89.0 ms** 脚本时间，`chat` chunk 18.3 → 3.0 ms，而真正必要的
  reflow（LayoutDuration/LayoutCount）不变（37.2/48 → 31.9/48）。

**D. 窄屏左栏改为悬停唤出（移植 open-vetta 的 hover 浮层）。**

参考：`useRootLayoutModel.ts` 的 `openOverlay` / `scheduleOverlayClose`（**120ms** 关延迟、重新进入即
取消关闭、离开窄屏强制关闭）与 `RootLayoutView` 里 `SidebarOverlay` 的 `onMouseEnter`/`onMouseLeave`。
此前 rove 窄屏必须先点顶栏按钮才出现侧栏。

- 新增 `shell/sidebar-overlay.ts`：`SIDEBAR_OVERLAY_CLOSE_DELAY_MS = 120`、
  `shouldShowSidebarHoverZone`、`shouldFocusSessionHeadingAfterSelection`；`ProductApp` 增加 peek 状态
  与 120ms 定时器（进入左侧热区/侧栏即取消关闭，移出即延时关闭，离开窄屏或卸载时清掉）。
- CSS：`.sidebar-hover-zone` 14px 贴左边缘，**只在 `@media (max-width: 960px) and (hover: hover) and
  (pointer: fine)`** 下存在，无任何视觉。
- 与参考的**两处有意偏离**（写在模块头注释里）：
  1. 悬停唤出的是**非模态偷看**：没有 `role="dialog"`/`aria-modal`、没有焦点陷阱、没有遮罩、**不移动
     焦点**。参考只有一个侧栏形态；rove 的窄屏侧栏同时是"点击打开的模态抽屉"（带焦点陷阱），
     若让模态对话框在指针下方弹出并抢焦点，会直接打断用户正在写的对话。
  2. 热区只对精细指针存在，触屏设备不会长出一条会吞掉点击的隐形条。
- 覆盖：`sidebar-overlay.test.ts` 3 条（热区条件、120ms 常量、只有"刻意打开"才回焦）
  ＋ `sidebar-hover-overlay.spec.ts` 3 条（唤出且不抢焦点/非 dialog/离开再在宽限期内返回保持打开；
  偷看态选中会话不移动焦点而按钮路径会移动到标题；宽屏没有热区）。

**F. 命令面板（移植 open-vetta `CommandMenu`）。**

读的是 `types.ts`、`lib/match.ts`、`lib/build-groups.ts`、`components/CommandMenu.tsx`、
`hooks/useCommandMenuModel.ts`。照搬其中**三条载荷级决策**（不是外观）：

1. **动作用数据描述，不用闭包**：适配层因此是纯函数、可直接断言；真正的导航副作用只发生在宿主一处。
2. **组序是常量，永不按相关性重排**：五个源的延迟差两个数量级，异步结果到达时若参与同一全局排序，会把
   光标下的行顶走——用户回车打开的将是从未看见的东西。
3. **子串匹配而非模糊子序列**，且多 token 全命中（AND）：中文没有词边界，子序列会让「设置」命中
   「设计与配置」，"搜不到"就不再可信；副标题命中仍算，但分数明显更低。

落地：

- `shell/command-palette.ts`（纯模型）：组序 `actions|workspaces|sessions|settings`、每组上限 6、
  规范化/tokenize、区间合并（避免嵌套高亮）、前缀 3 > 词首 2 > 任意 1 > 副标题 0.5 的打分、按 **id**
  锚定的选择移动（跳过禁用项并环绕）、高亮切分。
- `shell/CommandPalette.tsx`（视图）：`combobox` + `listbox` + `aria-activedescendant`，打开即聚焦输入框、
  Escape 关闭并**还原焦点**、Tab 不离开输入框（列表归方向键管）、命中但当前不可用的命令**显示为禁用**并
  给出原因，而不是静默过滤。
- `ProductApp`：把快捷键分发从原来的 `switch` 抽成 `runShortcut`，键盘监听与面板**共用同一实现**；
  条目覆盖既有 4 个快捷键、切换主题、工作区、会话、设置分区；`handleCommandAction` 是唯一的副作用分发点。
- 复用而非另起一套：新增 `open-command-palette`（Ctrl/Cmd+K，`allowInEditable: true`——**在输入框里也
  能唤起**，否则没人会用）；设置分区标签抽到 `sections.ts` 的 `SETTINGS_SECTION_COPY_KEYS` 与
  `VISIBLE_SETTINGS_SECTIONS`（`SettingsShell` 与面板共用，去掉一处重复映射，并让「advanced 只是路由
  兼容别名」这条事实只有一个来源）。
- 覆盖：`command-palette.test.ts` 14 条（子串非子序列的 CJK 用例、固定组序、每组上限、禁用项跳过、
  按 id 锚定、区间合并与高亮不丢字）＋ `command-palette.spec.ts` 5 条（从输入框唤起并执行、CJK 语义、
  显示为禁用、方向键/Escape/焦点还原、从面板跳转工作区与会话）。
- **顺带修掉我自己的一处断言缺陷**：`layout.spec.ts` 的左栏键盘用例用 `expect(width).toBe(288)` 去读
  **正在动画**的宽度，整轮负载下读到 `287.390625` 而失败（同一用例两次通过过）。改为
  `expect.poll(...).toBeCloseTo(288, 0)`——断言收敛，比原来更强。

**E. 空闲预取路由（移植 open-vetta `useIdleRoutePrefetch`）。**

参考：`useIdleRoutePrefetch.ts` 用 `requestIdleCallback(run, { timeout: 8000 })`，无该 API 时退化为
`setTimeout(3000)`，提前卸载则取消，预取失败静默（真正导航会重试）。

- 先核对：**挂起视图与判据我们已经有了，且已符合参考的规则**——`routePending` 只在"URL 尚不能由目录
  满足"时为真（启动/恢复期），分阶段打开会先乐观更新目录，所以不会盖掉乐观内容；`RouteLoadingView`
  是 `role="status"` 的限额文案而不是白屏。这两项**未改**。
- 真正缺的是预取：全仓此前没有任何 `router.prefetch`。新增 `shell/route-prefetch.ts`（纯策略：
  `ROUTE_PREFETCH_IDLE_TIMEOUT_MS = 8000`、`ROUTE_PREFETCH_FALLBACK_DELAY_MS = 3000`、
  `routePrefetchTargets` 只取"一键可达"的两项：顶栏齿轮直达的 `/settings/{general,providers}` 与
  当前工作区 `/w/<id>`，上限 3 条，不预取整个目录以免和启动路径抢资源）＋ `shell/use-idle-route-prefetch.ts`
  （同样的调度与取消语义）。
- 覆盖：`route-prefetch.test.ts` 4 条（目标有界/去重/含工作区/常量与参考一致）＋
  `tests/e2e/route-prefetch.spec.ts` 2 条。
- **实测发现（值得记下）**：Next **只在生产构建里预取**，`next dev` 下不发请求——dev 上写断言只会
  得到假阴性。因此该 e2e 默认 `test.skip`，并给出跑法（`pnpm build` + `next start` +
  `ROVE_E2E_PROD=1 PLAYWRIGHT_BASE_URL=... pnpm exec playwright test route-prefetch`）。本轮已按该方式
  实跑：**2 passed**，并在生产服务器上观察到
  `/settings/providers?_rsc=…`、`/settings/general?_rsc=…`、`/w/<id>?_rsc=…` 三类预取请求
  （`outputs/_probe-prefetch.cjs`）。
- **同时量出一个真实副作用并据此收紧了实现**：把预取无条件打开后，全量 e2e 连续两轮各失败**一个不同
  的**时序敏感用例（第一次 `shell.spec.ts` 的设置跳转、第二次 `workbench-panel.spec.ts` 的审批刷新），
  两者单独跑都通过——典型的"整轮负载下变慢"型抖动；dev 下预取请求本身并不会发出去，但它会让服务端去做
  按需编译，等于给每个用例加压。于是新增 `shouldPrefetchRoutes(process.env.NODE_ENV)`，**只在生产构建
  里预取**（与 Next 自身行为一致，并且在 dev 里彻底不产生噪声）；随后连续两轮全量 e2e 均 0 failed。
  这条抖动**不是**我的断言放宽——`shell.spec.ts` 与 `workbench-panel.spec.ts` 里的断言一字未改。

**G. 跟随滚动与滚动条显隐（移植 PI `lib/transcript-scroll.ts` / `lib/scrollbar-reveal.ts`）。**

此前外壳只有一个 `atLatest` 布尔量，每次 scroll 事件用 48px 阈值重算。它无法区分"用户手势"与
"布局钳制"：分数 DPR 下浏览器报告的位置与请求值差一个亚像素，严格比较就读成"用户向上滚"，跟随被
误解除。参考实现把**显式跟随状态**与**近底视觉阈值**分开，只有真实上移才解除。

- `chat/transcript-scroll.ts`（纯 reducer＋常量 48/1/200、`bottomScrollTop`、`isRecentScrollGesture`）
  ＋ `chat/use-follow-scroll.ts`（状态机、手势窗口、帧内重钉、`pinToLatest`/`followNow`/
  `recordScrollPosition`/`releaseFollow`/`jumpToMarker`）。
- `lib/scrollbar-reveal.ts`（捕获阶段监听 scroll → 给滚动元素打 `data-scrolling` → 静默 300ms 清除）。
- 覆盖：`chat/transcript-scroll.test.ts` 9 条、`lib/scrollbar-reveal.test.ts` 5 条、
  `tests/e2e/transcript-scroll.spec.ts` 4 条（打开即钉在新回合；滚轮上移解除跟随并揭示滚动条；
  "回到最新"出现、点击后归零并消失；程序化修正不被误判为手势）。

**量出并修掉一个真实缺陷：恢复的会话打开后停在距底 22px。** 这里归因错过两次，值得记下来：

1. 先怀疑平滑动画——`.chat-transcript` 设了 `scroll-behavior: smooth`，于是 `behavior:"auto"` 会被
   **动画化**，而动画目标会被随后的内容增长重新定向，最终停在目标之前。改成显式 `instant`
   （`FOLLOW_SCROLL_BEHAVIOR`）后**指针稳定停在 5170 而非 5192**，说明还有第二个原因。
2. 用临时探针（`tests/e2e/_probe-scroll.spec.ts`，测完即删）逐帧记录几何与 follow 决策才看到真相：
   `ResizeObserver` 当时只观察**内容元素**，而**滚动容器自身**在数据到达后从 543px 变矮到 521px
   （输入区变高），内容盒子不变 → 回调不触发 → 永远差那 22px。
3. 修法：`observer.observe(transcript)` 同时观察容器本身（注释写明原因与实测值）。两处修完 4/4 通过；
   探针另行证明跟随链路本身正常（外部插入 40px 造出增长后自动回钉到 gap 0）。

**H. 会话小地图（移植 PI `lib/conversation-minimap.ts` + `ConversationMinimap.tsx`）。**

- `chat/conversation-minimap.ts`（纯逻辑）：每个用户消息一个标记；同一回合的多条助手消息**合并**到
  一个标记（流式分片、中间说明、最终答案属于同一回合），预览按 280 字符截断、空内容的流式气泡不占
  标记；`shouldRenderConversationMinimap` 定义显示门槛（溢出或"上面还有未加载的历史"）。
- `chat/ConversationMinimap.tsx`：`nav` + 每标记一个按钮，方向键/Home/End 在轨道内移动焦点，
  标签为"第 n 个标记，共 m 个：角色"，`title` 是该回合预览。
- `Transcript.tsx`：消息气泡加 `data-message-id` 作为锚点；`jumpToMarker` 跳到**已渲染**的回合并
  **保持解除跟随**（跳到旧回合是阅读位置，不该被自动拉回）。
- 与参考的**两处有意偏离**：
  1. 标记只指向已渲染的回合；`hiddenRunCount > 0` 时轨道仍显示，兼作"上面还有历史"的提示。
  2. 可见性用**容器查询**（`@container transcript-frame (min-width: 940px)`）而不是视口查询。理由是
     实测的几何：中栏宽度同时取决于左栏与右面板，而且**在 1440 视口下中栏恰好等于 840 阅读上限**
     （frame 840 / 走廊 0），此时轨道无立足之地，所以它只在窗格真正宽于上限时出现（1600 视口下
     frame 1000 / 走廊 80px）。门槛的算式写在 CSS 注释里（26px 偏移＋20px 轨道）。
- 覆盖：`chat/conversation-minimap.test.ts` 6 条 ＋ `tests/e2e/conversation-minimap.spec.ts` 3 条
  （六个标记的次序与角色；点标记跳到该回合且跟随保持解除；方向键/Home/End 在轨道内移动焦点）。

**I. `frame-batcher` / `latest-wins` 两个原语（移植 PI `lib/frame-batcher.ts` / `lib/latest-wins.ts`）。**

- `lib/frame-batcher.ts`：按 key 只留最新值的帧内合并，保留不同 key 的插入顺序，提供 `merge` 折叠与
  `flushNow()`（终态不必等下一帧）；无 rAF 环境退化为 16ms 定时器。覆盖 6 条单测（含手工驱动的
  帧时钟）。
- `lib/latest-wins.ts`：`LatestWinsGate`（`begin`/`isCurrent`/`invalidate`），覆盖 3 条单测。
- **接在真实缺陷上，而不是只放进仓库**：`settings/SettingsShell.tsx` 的"测试连接"与"获取模型列表"
  都是按钮发起、没有 effect cleanup 的请求，此前**先点 A 再点 B，A 的慢响应会渲染成 B 的结果**
  （`probeResult`/`modelResult` 不按 profileId 校验），并且 A 的 `finally` 还会把 B 正在进行的
  busy 态清掉。现在两类请求各一个 gate，只有最新请求能提交状态、也只能由它清 busy。
  `queued-prompts` 的排队语义经核对本仓库已有实现（统一消息生命周期的 `requested_delivery`
  `current_run`/`next_turn` 与 `queued`→`claimed_successor` 状态机），因此不重复实现。

**J. 按 `web-design-guidelines` 做一次针对性审计（open-vetta 自带该 skill，规则来自 vercel-labs）。**

审计范围是本次改动涉及的外壳面（面板、转录、命令面板、代码块）。查到的**真实缺陷**：

- `--v2-ink-1` 这个自定义属性**从未被定义**（主题只有 `--v2-ink`/`-2`/`-3`），却被 3 处引用
  （命令面板输入与选项文字、`.ghost.icon-button` 悬停）——浅色下是"未定义变量"，深色下文字色直接
  失效。已改为 `var(--v2-ink)`。
- 嵌套滚动容器缺 `overscroll-behavior: contain`：滚轮/触控板到代码块或面板列表尽头会**继续滚到后面
  的窗格**，横向还会触发浏览器返回手势。已补齐命令面板列表＋代码块/图表/diff 预览/队列列表/模态卡片
  （共 10 处），并写明理由。
- 命令面板选项是自绘的 `div[role=option]` 点击目标，缺 `touch-action: manipulation`（触屏上会等
  双击缩放），已补。

同一轮核对**已合规**、无需改动的项：reduced motion（全窗口 `animation-duration/.scroll-behavior`
归零＋面板与阅读栏各自的退出规则）、可见焦点环、`alt` 文本（两处 `<img>` 均有）、图标按钮的
`aria-label`、表单 `autoComplete`、图片预览的 `noopener,noreferrer`、以及"没有 `transition: all`"与
"文案里没有 `...`"两项（本轮复查为空）。


### 8.2 参考有、但**不适用**于本仓库的一处


PI 的 reduced-motion 用 `animation-duration: 0.01ms` 而不是 `none`，理由是"退出动画的卸载依赖
`animationend`"。核查：本仓库全 `apps/web` **没有 `animationend`/`animationEnd` 监听**
（`git grep` 为空），所以 `animation: none` 不可能让某个卸载悬空——这不是缺陷，按事实记录而不照抄。

### 8.3 参考的好做法里，本仓库仍是缺口的（已列清单，未实现）

按价值/成本排序，均**未**在本轮改动：

1. ~~**内容子树与外壳布局状态解耦**（open-vetta）~~：**已完成**，见 §8.1C（左栏拖拽 604.6 →
   89.0 ms 脚本时间，`chat` chunk 18.3 → 3.0 ms）。同源问题里还剩一项未做：右栏 `RunInspector`
   自身逐帧预览约 147ms/24 步（预览态在组件内部，属面板自己的重渲染；要再降低需要把预览降到
   CSS 变量+叶子订阅，收益与风险都需另测）。
2. ~~**窄屏侧栏改为悬停浮层**（open-vetta `SidebarOverlay` + `scheduleOverlayClose`）：我们现在必须
   点击才出现。~~ **已完成**，见 §8.1D。
3. **命令面板**（open-vetta `CommandMenu`）：**已完成**，见 §8.1F。同项的**空闲预取路由**亦已完成
   （§8.1E），**路由挂起内容视图**经核对本仓库已有且符合参考规则（同样见 §8.1E），无需改动。
   到此前瞻清单里"按参考对齐"的条目已全部落地。
4. ~~**会话小地图**（PI `ConversationMinimap`）、**跟随滚动细节**（PI `use-follow-scroll`）、
   `scrollbar-reveal`、`frame-batcher` / `latest-wins`~~：**已完成**，见 §8.1G–§8.1I。
   `queued-prompts` 的排队语义经核对**本仓库已有实现**（统一消息生命周期 F.1–F.3：`requested_delivery`
   的 `current_run`/`next_turn`、`queued`→`claimed_successor` 状态机与提升/撤销按钮），故不重复实现。
   到此前瞻清单里"按参考对齐"的条目已全部落地。
5. **设计审查流程**：open-vetta 自带 `web-design-guidelines`（拉取 vercel-labs 规则做 UI 审查）与
   `frontend-design`（质量底线：响应式、可见焦点、尊重 reduced motion）。本外壳这三条目前都满足；
   本轮已按该规则对改动面做过一次针对性审计并修掉查到的缺陷（§8.1J），但把"规则化审查"做成常设
   步骤仍属独立工作，且该 skill 的规则要从网络拉取（本轮该 URL 取不到，规则按既有记忆核对）。
6. 两个参考项目的大量产品面（插件市场、Agent 团队、同步、移动端、文档站）不在"外壳修复"范围内。

### 8.4 本轮门禁（真实退出码）

§8.1A–§8.1F 落地时：

| 门 | 结果 | 退出码 |
|---|---|---|
| `pnpm exec playwright test`（全量 110 项，dev） | 103 passed / 7 skipped / 0 failed | 0 |
| `pnpm exec playwright test route-prefetch`（生产构建，`ROVE_E2E_PROD=1`） | 2 passed | 0 |
| `pnpm exec vitest run` | 52 文件 / 411 用例通过 | 0 |
| `pnpm typecheck` | 无错误 | 0 |
| `pnpm build` | 编译成功 | 0 |
| 独立几何探针（含阅读栏 7 项） | 147 passed / 0 failed | 0 |

§8.1G–§8.1J 落地后（本轮新增小地图与两个原语，另修 22px 距底缺陷与指南审计项）**全部重跑**：

| 门 | 结果 | 退出码 |
|---|---|---|
| `pnpm exec playwright test`（全量 117 项，dev，在提交后的树上复跑） | 110 passed / 7 skipped / 0 failed | 0 |
| 其中本轮新增：`transcript-scroll.spec.ts` | 4 passed | 0 |
| 其中本轮新增：`conversation-minimap.spec.ts` | 3 passed | 0 |
| `pnpm exec vitest run` | 57 文件 / 440 用例通过（新增 6＋3＋6 条） | 0 |
| `pnpm typecheck`（`tsc --noEmit`） | 无输出＝无错误 | 0 |
| `pnpm build`（生产构建） | Compiled successfully ＋ TypeScript 13.0s ＋ 静态页 6/6 | 0 |
| 独立几何探针 `outputs/_verify-sweep.cjs` | **本轮未取得结论**：脚本在宽度扫描之前停在等待上（`getByRole("textbox")` 等待超时前未继续），对生产与 dev 两种服务器都一样；**因此不声称 147 项**。几何回归由全量 e2e 覆盖（`layout.spec.ts` 1440/1280/1024、`reading-width.spec.ts`、`product-ui-v2.spec.ts` 含移动端抽屉与证据面板） | — |

§8.1C 的性能对照仍有效（左栏拖拽脚本时间 604.6 → 89.0 ms，`chat` chunk 18.3 → 3.0 ms，reflow 不变）。
7 项 skip = 5 项既有 skip ＋ 2 项生产构建专属的预取用例（跑法见 §8.1E）。本轮另修掉一处我自己引入的
回归：`next build` 会把受版本控制的 `apps/web/next-env.d.ts` 在 `./.next/dev/...` 与 `./.next/...` 之间
翻转，已还原（生成物不入库）。
