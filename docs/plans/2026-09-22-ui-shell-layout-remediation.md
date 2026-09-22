# rove 三栏外壳布局修复与产品化补齐

> 状态：**Partially Implemented**。P0（阻断级网格缺陷）、P1a（设计 §5.0 共享宽度
> 预算）与 P1b（PI-Desktop 式分隔条、宽度持久化、左栏先让位/重开让位、预算上限
> 取代固定上限）已在 worktree `fix/ui-shell-layout` 实现并有自动化回归；
> 仍未实现：断点统一 1180/760、死变量清理（`--inspector-width`/`--sidebar-width`）、
> P2（右栏信息架构）、P3（中栏阅读区）、P4（样式表层收敛）。
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

未做：断点仍为 960（设计为 1180/760）；`--inspector-width`、`--sidebar-width` 等死变量未清理。

## 4. 目标（以设计文档为准）

- 三栏**一行**：左 200–360（默认 240，拖调 + localStorage）｜中 ≥450 硬底线、
  阅读列 680–840｜右 默认 400、可拖 320–640。
- 宽度预算优先级：**中栏底线 > 右栏请求 > 左栏**；不足时先自动收起左栏，
  再让右栏变抽屉。
- 断点：`<1180` 右栏抽屉；`<760` 左右不同时占宽。
- 右栏固定四页签：待处理 / 文件 / 变更 / 浏览（页签内存态，只持久化宽度）。
- 拖拽 = 8px 分隔条 + 键盘（16 / Shift 32 / Home·End），保留
  `role="separator"` 与 `aria-valuenow`。

## 5. P1b–P4（提议，待确认）

| 阶段 | 内容 | 退出条件 |
|---|---|---|
| P1b | 宽度来源收敛为 `--sidebar-width` + 单一面板宽度变量；右栏 range 换成 8px 分隔条（保留键盘/aria）；面板宽度持久化到 localStorage；JS/CSS 断点统一 1180/760；预算不足时"先自动收起左栏" | 分隔条鼠标+键盘用例；刷新后宽度恢复；1180/760 断点行为与设计一致 |
| P2 | 右栏信息架构（**形态待用户确认**，见 §6 第 1 条）：待处理 / 运行状态 / 文件与变更（审查仅在有审查项时出现）或按设计四页签 | 页签用例 + 浏览器用例通过；设计 §5.0 对照表逐行可核 |
| P3 | 中栏阅读列 680–840 居中、composer 同宽、按 §3.3 核对动效 | Playwright 视觉/几何用例；reduced-motion 断言 |
| P4 | 样式表层收敛（v3 只留 token/skin，v1 布局段迁入 v2 后删除）并同步 `docs/runtime/` | `pnpm test/typecheck/build/test:e2e` 全绿；文档与代码一致 |

## 6. 待确认的决策

1. **右栏信息架构（P2 的前置）**。"四页签"来自设计 §5.0 对 PI-Desktop `WorkPanel`
   的映射（P2 第 1 条同样如此），它属于参考实现的分类建议，**与"三栏布局"决策
   无关**；而且现状实现并未照做（现在是活动/审查/待处理三页签，且默认"活动"页签
   内堆了 8 个分区）。用户的意向是"右栏 = 当前的一些状态"。建议改为
   `待处理 / 运行状态 / 文件与变更`，审查仅在有审查项时作为第 4 个出现。
2. 断点：按设计 1180/760，还是保留现状 960。
3. 右栏默认宽度：设计 400（320–640）还是现状 360（280–560）。
4. 是否把 `_start-stack.ps1` / `_layout-probe.cjs` 提升为受版本控制的 `scripts/`
   工具（便于 worktree 与 CI 复用同一套"起服务 + 量布局"证据）。
