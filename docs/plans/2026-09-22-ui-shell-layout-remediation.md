# rove 三栏外壳布局修复与产品化补齐

> 状态：**Partially Implemented**。P0（阻断级网格缺陷）已在 worktree
> `fix/ui-shell-layout` 实现并有自动化回归；P1–P4 尚未实现，仍为提议。
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
| 1024×768 修复后 | `240px 424px 360px` | (0,52) 240×716 | (240,52) 424×716 | (664,52) 360×716 |
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

未做：P1–P4；未跑 `pnpm build`；未跑 Rust 门禁（本阶段无 Rust 变更）。

## 4. 目标（以设计文档为准）

- 三栏**一行**：左 200–360（默认 240，拖调 + localStorage）｜中 ≥450 硬底线、
  阅读列 680–840｜右 默认 400、可拖 320–640。
- 宽度预算优先级：**中栏底线 > 右栏请求 > 左栏**；不足时先自动收起左栏，
  再让右栏变抽屉。
- 断点：`<1180` 右栏抽屉；`<760` 左右不同时占宽。
- 右栏固定四页签：待处理 / 文件 / 变更 / 浏览（页签内存态，只持久化宽度）。
- 拖拽 = 8px 分隔条 + 键盘（16 / Shift 32 / Home·End），保留
  `role="separator"` 与 `aria-valuenow`。

## 5. P1–P4（提议，待确认）

| 阶段 | 内容 | 退出条件 |
|---|---|---|
| P1 | 宽度来源收敛为 `--sidebar-width` + `--work-panel-width`；实现 450 硬底线与"先收左栏"降级；右栏 range 换成 8px 分隔条；JS/CSS 断点统一 1180/760 | 1024 下中栏 ≥450 且无横向溢出；`layout.spec.ts` 的 fixme 转为通过 |
| P2 | 右栏改固定四页签（待处理/文件/变更/浏览），把 Files/Diff/Artifact/预览从"活动"提升；键盘可达与空态 | 页签用例 + 浏览器用例通过；设计 §5.0 对照表逐行可核 |
| P3 | 中栏阅读列 680–840 居中、composer 同宽、按 §3.3 核对动效 | Playwright 视觉/几何用例；reduced-motion 断言 |
| P4 | 样式表层收敛（v3 只留 token/skin，v1 布局段迁入 v2 后删除）并同步 `docs/runtime/` | `pnpm test/typecheck/build/test:e2e` 全绿；文档与代码一致 |

## 6. 待确认的决策

1. 右栏页签集合：设计四页签 vs 现状三页签；"活动"时间线与用量归入哪个页签。
2. 断点：按设计 1180/760，还是保留现状 960。
3. 右栏默认宽度：设计 400（320–640）还是现状 360（280–560）。
4. 是否把 `_start-stack.ps1` / `_layout-probe.cjs` 提升为受版本控制的 `scripts/`
   工具（便于 worktree 与 CI 复用同一套"起服务 + 量布局"证据）。
