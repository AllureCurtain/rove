# Rove 前端 UI V3 重构实现文档（cc-partner / Anthropic 暖米色风格）

> 状态：待实施　|　原分支 `feature/ui-restyle`（基于 main@8a4e141）与其 worktree 已于 2026-08-29 删除——里面从未有过代码改动，只有本文档与附录 A。开工时请基于当时的 main 新建 worktree，并注意本文档的方案是针对 8a4e141 写的，那之后 main 已合入 codex 对齐十期改造，动手前需复核前端现状。
> 视觉基准：cc-partner（https://github.com/mmletgo/cc-partner）
> 配套文件：
> - 附录 A（完整 Design Token CSS，可直接落地）：`docs/design/2026-08-24-ui-v3-tokens-appendix.md`
> - 项目外分析报告：`D:/Study/project/agent/cc-partner-ui-analysis.md`
>
> ## ⚠️ 工作流铁律
> 1. **任何改动只留在工作区，禁止 `git commit`**。用户需先本地看效果，确认后才允许提交。
> 2. 启动 dev server / 运行测试前必须先征得用户同意。
> 3. 每完成一个 Phase 向用户汇报，确认后再进入下一阶段。

---

## 目录

1. 目标与范围
2. 基线盘点
3. 总体架构决策
4. Design Token 规格（摘要 → 附录 A）
5. 新旧 Token 映射表
6. 原语组件（Primitives）规格
7. 外壳与视图换肤规格（逐组件）
8. 深色主题规格
9. 可访问性要求
10. 文件变更清单
11. 分阶段执行计划与验收标准
12. 测试与回归
13. 收尾与清理
14. 附录：cc-partner 参考文件索引

---

## 1. 目标与范围

### 1.1 目标
把 `apps/web` 的整体视觉从「冷灰绿 + 海港蓝」切换为 cc-partner 的 **Anthropic 暖米色设计系统**：

- 暖米色背景层级（`#f5f4ed` / `#faf9f5` / `#e8e6dc`）
- Terracotta 强调色（浅 `#c96442` / 深 `#d97757`）
- 衬线标题字体栈（Georgia fallback）+ 无衬线正文 + mono 等宽
- spacing/radius/shadow/motion 全量 token 化
- 统一原语组件视觉（Button/Card/Pill/StatusDot/StatusMessage/Input/Dialog 等）
- cc-partner 式细节语言：eyebrow 小标签、衬线大标题、分段 tab 控件、四态模板（skeleton/empty/notice/error）

### 1.2 范围内
- `apps/web/styles/*` 样式层全部
- `shell / sidebar / chat / inspector / settings / product-v2` 的呈现层
- 新增 `/dev/design-system` 样式对照页（dev-only）

### 1.3 范围外（明确不做）
- 业务逻辑：state hooks、API client（`product/`）、i18n 文案、路由结构
- Tauri 桌面壳（`apps/desktop`）原生窗口外观
- CLI/TUI/API surfaces
- 响应式断点策略变化（保持现有行为）

---

## 2. 基线盘点

### 2.1 样式文件现状
| 文件 | 行数 | 角色 |
|---|---|---|
| `apps/web/app/globals.css` | 1660 | 旧全局样式（青绿 accent），部分仍被引用 |
| `apps/web/styles/tokens.css` | 93 | 基础 token（绿灰 + 海港蓝 `#3a5f7a`），light/dark |
| `apps/web/styles/product.css` | 2400 | v1 呈现层（默认皮肤基础规则） |
| `apps/web/styles/product-v2.css` | 3139 | v2 呈现层：`.product-app-frame[data-ui-version="v2"]` 后代选择器整树覆盖 v1，自带 `--v2-*` 命名空间 |

### 2.2 版本切换机制（本方案核心依托）
```
app/(product)/layout.tsx
  productUiVersion() = process.env.ROVE_PRODUCT_UI_VERSION === "v1" ? "v1" : "v2"
  <ProductApp uiVersion={...} />

shell/ProductApp.tsx
  export type ProductUiVersion = "v1" | "v2";
  <div className="product-app-frame" data-ui-version={uiVersion}>
    <div className="product-root" data-presentation={uiVersion}>
```
**DOM 与类名三版共用，皮肤靠 `data-ui-version` 属性作用域下的 CSS 决定。v3 沿用同一机制。**

### 2.3 换肤对象类名清单
```
.product-app-frame            ← 最外层作用域锚点
.product-root                 ← TopBar + body 纵向 flex
.product-topbar               ← 品牌/__mark/__meta/__connection、.status-dot[data-tone]
.product-body                 ← 主网格；data-settings / data-workspace-open / data-inspector-open
.product-sidebar              ← .workspace-search、.workspace-group__row/__button
.product-main                 ← .shell-alert、.route-state、.boot-state
.chat-pane(__header)          ← .chat-transcript-frame/__content、.chat-bubble、.message-byline、
                                 .message-evidence、.approval-card*、.load-older-turns
.chat-composer(__row/__controls/__meta/__review)、.chat-error
.empty-state(__card/__actions/__recents)
.inspector-header/tabs/sections/facts/kv/skeleton/state/body
.field / .field-grid / .field-actions / .input-card / .mcp-*   ← Settings
.modal-backdrop / .modal-card(__lede) / .modal-actions
按钮：.secondary / .ghost / .danger / .icon-button / .mobile-only
```

---

## 3. 总体架构决策

### 3.1 采用「v3 呈现层」，不重写 DOM
- 扩展 `ProductUiVersion = "v1" | "v2" | "v3"`；`ROVE_PRODUCT_UI_VERSION=v3` 时生效（最终把默认值切到 v3）。
- 沿用 v2 已验证的作用域覆盖模式（所有 v3 规则写在 `.product-app-frame[data-ui-version="v3"] ...` 下）。理由：
  - 与仓库现行惯例一致，TSX 改动最小；
  - 可与 v1/v2 并存，随时切换对比验收；
  - 清理阶段一次性收敛为一套全局样式（§13）。
- 新增原语组件是全新 JSX + 全新 `cp-*` 类名，不受存量类名影响。

### 3.2 文件分层
```
apps/web/styles/v3/
  index.css         ← 汇总入口（按序 @import 下列文件）
  tokens.css        ← 唯一颜色/字体/间距来源（含 light/dark + 存量变量桥接段）
  base.css          ← 作用域内补充 reset、.cp-eyebrow/.cp-sr-only 工具类、reduced-motion
  primitives.css    ← cp-* 原语组件样式
  shell.css         ← topbar / sidebar / body 布局换肤
  chat.css          ← transcript / bubble / composer
  inspector.css     ← run inspector 各 panel
  settings.css      ← settings 表单与 tab
```
在根 layout 或 `app/(product)/layout.tsx` 追加 `import "../styles/v3/index.css";`（置于现有四个 CSS 之后）。

### 3.3 硬约束
- v3 作用域内禁止硬编码颜色/间距——全部引用 token；新增 `scripts/check-css-tokens.mjs` CI 校验。
- 不引入新 npm 运行时依赖；图标继续用 `@radix-ui/react-icons`（线性风格与目标一致）。

---

## 4. Design Token 规格

完整可落地 CSS 见 **附录 A**（`2026-08-24-ui-v3-tokens-appendix.md`），要点：

- 作用域锚点：`.product-app-frame[data-ui-version="v3"]`（light 默认）+ `html[data-theme="dark"]` 覆盖块
- 命名空间 `--cp-*`，含颜色/字体/字号十档/4px 间距网格/圆角六档/阴影五档/动效三档两曲线/z-index 五层
- **桥接段**：把 rove 存量变量名（`--bg/--surface/--text/--muted/--accent/--border/--success…` 及全部 `--v2-*`）指向对应 `--cp-*`，让未改写的存量规则自动获得暖色系，后续逐块替换时再改引本名
- 字体：`--cp-font-display: Georgia, "Times New Roman", serif`（品牌字体不可分发，仅 fallback）

---

## 5. 新旧 Token 映射表

逐块改写存量选择器时按下表替换：

| 旧变量 | 旧值 | → 新变量 | 新值 |
|---|---|---|---|
| `--v2-canvas` / `--v2-bg` | #e7edf1 / #eff3f5 | `--cp-bg` | #f5f4ed |
| `--v2-surface` / `-raised` | #f8fafb / #fcfdfd | `--cp-surface` | #faf9f5 |
| `--v2-surface-sunken` | #e6ecef | `--cp-surface-warm` | #e8e6dc |
| `--v2-surface-hover` | #dde5ea | `--cp-accent-soft` 或 `--cp-surface-warm` | — |
| `--v2-ink` / `-2` / `-3` | #111a20/#34444e/#62737e | `--cp-fg` / `-fg-2` / `-muted` | #141413/#3d3d3a/#5e5d59 |
| `--v2-border` / `-strong` | rgba ink 10%/18% | `--cp-border` / `-strong` | #f0eee6 / #d9d5c9 |
| `--v2-signal` / `-strong` / `-soft` / `-on-signal` | #0d789f 系 | `--cp-accent` / `-hover` / `-soft` / `-on` | terracotta 系 |
| `--v2-rail*` 系列 | 蓝灰侧栏 | `--cp-surface` + `--cp-border-soft` | — |
| `--v2-success/-warning/-danger` | 冷语义色 | `--cp-success/-warning/-danger` | #17a34a/#eab308/#b53333 |
| `--v2-radius-xs/sm/md/lg` | 3/5/8/?px | `--cp-radius-xs/sm/md/lg` | 4/6/8/12px |
| `--v2-shadow-sm/md` | 蓝灰阴影 | `--cp-shadow-xs/md` | 中性阴影 |
| tokens.css `--accent` | #3a5f7a | 桥接至 `--cp-accent` | 自动 |
| globals.css `--accent` | #0f766e | 桥接至 `--cp-accent` | 自动 |

---

## 6. 原语组件（Primitives）规格

React 组件放 `apps/web/components/ui/*`（无业务依赖、forwardRef、纯展示）；样式放 `styles/v3/primitives.css`，类名统一 `cp-` 前缀。移植源以括号内 cc-partner 文件为准。

### 6.1 Button（参考 `web/src/components/primitives/Button/*`）
```tsx
type Variant = "primary" | "secondary" | "ghost" | "danger" | "icon";
type Size = "sm" | "md" | "lg";                    // 高 26 / 32 / 40
props: variant(默认 secondary) / size(默认 md) / loading / icon / iconRight
```
- `data-variant` / `data-size` 属性驱动样式；loading 时 spinner 原地叠加不改宽高、禁点击
- primary = terracotta 实心；secondary = surface + border 描边；ghost = 透明 hover surface-warm；danger = 红字透明底 hover danger-soft；icon = 26px 圆形描边钮
- focus-visible：`box-shadow: 0 0 0 2px var(--cp-accent-soft)`；disabled：opacity .5

### 6.2 Card（参考 `primitives/Card`）
- 插槽 header/body/footer；header/footer 用 `--cp-border-soft` 分隔线；圆角 `--cp-radius-lg`、边框 `--cp-border`、阴影 `--cp-shadow-xs`

### 6.3 Pill / Tag（参考 `primitives/Pill`、`primitives/Tag`）
- Pill：22px 高胶囊 + 可选 6px 彩点（`__dot`），tone = neutral/success/warn/danger，语义 tone 用对应 soft 底 + 深字色（warn 底黄配 `--cp-fg` 字，勿配白字）
- Tag：小号直角标签（radius-xs），同 tone 集合

### 6.4 StatusDot / StatusMessage（参考同名 primitives）
- StatusDot：7px 圆点，status = idle(muted)/running(success+pulse)/attention(warning)/error(danger)
- StatusMessage：四 tone 信息条（info=surface-warm / success / warn / danger 各 soft 底），可选右侧 action 槽放 secondary sm 按钮；`role=status`（非致命）/ `role=alert`（danger）
- 替换点：`.shell-alert` → danger StatusMessage；`.chat-error` → 同款

### 6.5 Input / Textarea（参考 `primitives/Input`）
- surface 底 + `--cp-border-strong` 边框 + radius-md；focus-visible 双环（accent-soft box-shadow + accent border）；placeholder 用 `--cp-meta`

### 6.6 Dialog / Modal（改造现有 `.modal-backdrop / .modal-card`）
- backdrop：`background: var(--cp-overlay)`（始终深色半透明，不随主题反转）
- 卡片：surface 底、radius-lg、`--cp-shadow-window`；Escape/backdrop 关闭合同保持现有实现不动，仅换肤
- busy 时禁关闭的行为保留

### 6.7 ProgressBar（参考 `primitives/ProgressBar`）
- 高 6px 圆角条，track = surface-warm，fill 按 tone 取 accent/success/danger

### 6.8 分段 Tab 控件（从 cc-partner Workbench `.inspectorTabs` 移植）
```css
.cp-segmented { display:grid; grid-auto-flow:column; gap:var(--cp-space-1);
  padding:var(--cp-space-1); border:1px solid var(--cp-border-soft);
  border-radius:var(--cp-radius-md);  background:var(--cp-surface-warm); }
.cp-segmented__item { height:26px; border-radius:var(--cp-radius-sm); font-size:var(--cp-text-sm);
  color:var(--cp-muted); background:transparent; border:none; cursor:pointer;
  transition:all var(--cp-motion-fast) var(--cp-ease-standard); }
.cp-segmented__item[data-active="true"] { background:var(--cp-surface); color:var(--cp-fg);
  box-shadow:var(--cp-shadow-xs); }
```
- 用途：RunInspector `.inspector-tabs`、Settings 的 tab 切换
- a11y：`role="tablist"` + 方向键移动焦点（roving tabindex），与现有键盘合同一致

### 6.9 工具类（放 `styles/v3/base.css`）
```css
.product-app-frame[data-ui-version="v3"] .cp-eyebrow {
  font-family:var(--cp-font-body); font-size:var(--cp-text-xs);
  letter-spacing:var(--cp-tracking-widest); text-transform:uppercase;
  color:var(--cp-muted); font-weight:var(--cp-weight-medium); }
/* .cp-sr-only 与 cc-partner globals.css 同款裁剪式隐藏 */
```

### 6.10 页面四态模板（从 cc-partner 各页面 header/empty/loading 模式归纳）
每个主视图统一提供四种状态，视觉规格：
1. **loading**：skeleton 块（surface-warm 底、radius-md、`cp-shimmer` 微光动画；reduced-motion 下退化为静态块）
2. **empty**：居中 Card——eyebrow 小标签 → 衬线大标题 → muted lede → actions 行
3. **notice**：StatusMessage(info)
4. **error**：StatusMessage(danger) + 重试按钮

---

## 7. 外壳与视图换肤规格（逐组件）

> 以下全部写在 `data-ui-version="v3"` 作用域内；「替换」指改写存量选择器声明，「新增」指追加 cp-* 规则。

### 7.1 全局画布（shell.css）
```css
.product-app-frame[data-ui-version="v3"] { background:var(--cp-bg); color:var(--cp-fg);
  font-family:var(--cp-font-body); font-size:var(--cp-text-base); line-height:var(--cp-leading-normal);
  ::selection { background:var(--cp-selection); } }
```

### 7.2 TopBar（`.product-topbar*`）
- 背景 `--cp-surface`，底部 1px `--cp-border`；高保持 52px
- 品牌 mark：terracotta 圆角方块或衬线首字母；品牌名用 `--cp-font-display` semibold、`--cp-text-md`
- 右侧控件组：icon-button 化（26px 圆钮），间距 `--cp-space-2`
- `.status-dot[data-tone]` 映射到 cp StatusDot 色板：ok→success / warn→warning / error→danger

### 7.3 Sidebar / WorkspaceTree（`.product-sidebar`, `.workspace-*`）
- 背景 `--cp-surface`，右缘 1px `--cp-border`
- 搜索框 → cp Input 样式（高 28px，radius-md）
- 分组标题 → **eyebrow 样式**（11px 大写宽字距 muted）——这是 cc-partner 侧栏的核心识别点
- 行项：高 30px、radius-sm hover surface-warm；active 行 = accent-soft 底 + 左侧 2px terracotta 指示条 + fg 字色
- 图标 14px、`color: var(--cp-muted)`，active 时 accent

### 7.4 Chat Transcript（chat.css：`.chat-bubble`, `.message-byline`, …）
- 用户消息 bubble：`--cp-surface` 卡片风（border+shadow-xs+radius-lg）；assistant 消息：透明底直接排版，靠 byline 区分——cc-partner 的对话区不是双栏气泡而是文档流
- byline：角色名 semibold sm + 时间戳 meta 色 mono xs
- 代码块/patch：`--cp-font-mono`、surface-warm 底、radius-md
- `.approval-card*`：cp Card 结构化重排——header 放标题+tone Pill，body 放说明，footer 放按钮组（primary=批准 / secondary=拒绝）
- `.load-older-turns`：ghost 按钮 + 居中

### 7.5 Composer（`.chat-composer*`）
- 外框：surface 卡片、border、focus-within 时 border 变 accent + accent-soft 光环
- textarea 无边框透明底，行高 relaxed
- controls 行：左侧模式/模型选择器（secondary sm），右侧发送钮（primary md，禁用时降饱和）
- `.chat-error`：composer 上方贴一条 danger StatusMessage

### 7.6 Inspector（inspector.css：`.inspector-*`）
- 面板背景 `--cp-bg`（比主内容略深一档的层级感），左缘 1px border
- `.inspector-header`：eyebrow「RUN」+ 衬线标题 + 状态 Pill
- `.inspector-tabs` → cp-segmented
- section 标题 → eyebrow 样式；kv 表：key 用 muted sm，value 用 mono
- skeleton → cp 四态模板之 loading

### 7.7 Settings（settings.css：`.field*`, `.input-card`, `.mcp-*`）
- 页头模板：eyebrow → 衬线大标题（text-xl）→ muted lede（对齐 cc-partner Settings 页）
- `.field`：label sm medium + 说明 muted xs；`.field-grid` 双列 gap space-4
- `.input-card`：cp Card 包裹每组设置，footer 放保存动作
- 危险区：danger StatusMessage + danger 按钮

### 7.8 Modal / EmptyState / BootState
- `.modal-card`：按 §6.6 换肤；`.modal-card__lede` 用 muted
- `.empty-state__card`：§6.10 empty 模板；`.empty-state__recents` 列表行 hover surface-warm
- `.boot-state / .route-state`：居中 spinner（accent 色）+ eyebrow 文案

---

## 8. 深色主题规格

rove 已有 `html[data-theme]` 机制（TopBar 可切换、服务端 bootstrap 注入）。v3 深色块同样写在 tokens.css：

| Token | Dark 值 |
|---|---|
| `--cp-bg` | #1f1d1b |
| `--cp-surface` | #292524 |
| `--cp-surface-warm` | #33302d |
| `--cp-fg` | #faf9f5 |
| `--cp-fg-2` | #e5e2dc |
| `--cp-muted` | #a8a49c |
| `--cp-meta` | #78756e |
| `--cp-border` | #33302d |
| `--cp-border-soft` | #2b2825 |
| `--cp-border-strong` | #443f3a |
| `--cp-accent` | #d97757 |
| `--cp-accent-on` | #1f1d1b |
| `--cp-danger` | #e07a6a |
| `--cp-success` | #4ade80 |
| `--cp-warning` | #fbbf24 |
| soft 色阶 | color-mix 16%（暗底需更高占比才可读） |
| shadow | 不变但整体减淡（暗色阴影感知弱） |

验收要点：dark 下所有文字对比 ≥4.5:1；soft 底上的字必须取对应实色而非白/黑。

---

## 9. 可访问性要求

从 cc-partner 移植并强制执行的清单：

1. focus-visible 全局 2px 可见环（accent 系），永不 `outline:none`
2. `prefers-reduced-motion: reduce` → 关闭 shimmer/pulse/spinner 动画，transition ≤0.01ms
3. 对比度：正文 ≥4.5:1；大字号/图形 ≥3:1；meta 色仅限装饰性文本
4. 键盘：segmented tabs roving tabindex；modal 焦点陷阱维持现状；侧栏树方向键导航维持现状
5. aria：StatusMessage 按 tone 设 role=status/alert；图标按钮必须有 aria-label（沿用 i18n 文案）
6. 语言切换后 aria-label 同步（复用现有 lib/i18n）

---

## 10. 文件变更清单

### 新建
| 文件 | 内容 |
|---|---|
| `apps/web/styles/v3/index.css` | @import 入口 |
| `apps/web/styles/v3/tokens.css` | 完整 token + dark + 桥接段（附录 A） |
| `apps/web/styles/v3/base.css` | reset 补充、工具类、reduced-motion、shimmer keyframes |
| `apps/web/styles/v3/primitives.css` | cp-* 原语样式 |
| `apps/web/styles/v3/shell.css` | topbar/sidebar/body |
| `apps/web/styles/v3/chat.css` | transcript/composer |
| `apps/web/styles/v3/inspector.css` | inspector 各 panel |
| `apps/web/styles/v3/settings.css` | settings |
| `apps/web/components/ui/CpButton.tsx` 等 | §6 各原语组件 |
| `apps/web/app/dev/design-system/page.tsx` | 样式对照页（dev-only，仿 cc-partner DesignSystem 页） |
| `apps/web/scripts/check-css-tokens.mjs` | v3 CSS token 合规校验脚本 |

### 修改
| 文件 | 改动 |
|---|---|
| `apps/web/shell/ProductApp.tsx` | `ProductUiVersion` 增加 `"v3"` |
| `apps/web/app/(product)/layout.tsx` | env 支持 v3 并设为默认；import v3/index.css |
| `package.json`（web） | 新增 `"check:css-tokens": "node scripts/check-css-tokens.mjs"` |

### 不动
业务 hooks、`product/` API client、i18n 文案值、e2e 测试逻辑（仅必要时把类名选择器换成 data-testid）、Tauri 工程。

---

## 11. 分阶段执行计划与验收标准

### Phase 0 — 准备（0.5 天）
- [ ] 本 worktree 已建（✅ `feature/ui-restyle` @8a4e141）
- [ ] 记录基线截图（征得同意后起 dev server，light/dark × 主要视图）

### Phase 1 — Token 层（1 天）
- 建 `styles/v3/tokens.css` + `base.css` + `index.css`，layout 引入
- 写 `check-css-tokens.mjs` 并接入 package.json scripts
- ✅ 验收：`ROVE_PRODUCT_UI_VERSION=v3` 下应用可用（桥接段生效，整体已偏暖米色，细节未精修）；校验脚本通过；v1/v2 切换不受影响

### Phase 2 — 原语组件（2 天）
- cp-* 组件 + primitives.css + `/dev/design-system` 对照页
- ✅ 验收：对照页展示全部原语 light/dark 两态；typecheck 通过

### Phase 3 — 外壳换肤（2 天）
- shell.css：TopBar/Sidebar/body + boot/route state
- ✅ 验收：三区布局无回归，移动端抽屉行为不变，对比截图达标

### Phase 4 — 视图逐个换肤（4–5 天）
顺序：chat transcript → composer → RunInspector → Settings → modal/empty/gates
每完成一个视图即截图对比。
- ✅ 验收：全部主要视图 light/dark 达标；`check:css-tokens` 通过；无控制台样式告警

### Phase 5 — 打磨与收尾（2 天）
- a11y 清单过一遍、reduced-motion 验证、默认版本切到 v3、更新 e2e 选择器
- ✅ 验收：全量检查绿；等待用户确认效果后才允许 commit

合计约 **11–12 个工作日**（单人）。

---

## 12. 测试与回归

- `pnpm --filter web typecheck / test / build`（执行前征得同意）
- 手工矩阵：{light, dark} × {web, desktop 静态导出} × {workspace/chat/inspector/settings/modal} × zh/en
- e2e：跑现有套件；因类名未变的规则理论上零影响，凡确需改 DOM 的地方先加 data-testid 再动类名
- 视觉回归：以 Phase 0 截图为基准逐 Phase 对比

---

## 13. 收尾与清理（用户确认后另行任务）

1. 删除 `styles/product.css`、`styles/product-v2.css` 及 `globals.css` 中被取代的部分（预计 -5000 行级）
2. 把 v3 作用域选择器降为普通全局规则（去掉 `[data-ui-version="v3"]` 前缀），移除 ProductUiVersion 类型与 env 开关
3. 存量类名逐步改名收敛到 cp-* 语义（可选二期）
4. 更新 `docs/design/` 相关设计文档与 README 截图

---

## 14. 附录：cc-partner 参考文件索引

| 目标 | cc-partner 源路径（仓库根 `cc-partner/web/src`） |
|---|---|
| Token | `styles/tokens.css`、`app/globals.css` |
| Reset/工具类 | `styles/reset.css`、globals 中 `.sr-only/.eyebrow` |
| Button/Card/Pill/Tag/StatusDot/StatusMessage/Input/ProgressBar/Dialog | `components/primitives/<Name>/` |
| 分段 tab | Workbench 页 `.inspectorTabs` 相关样式 |
| 页面头模板（eyebrow/title/lede） | 任一页面（如 `pages/Home/Home.tsx`）头部结构 |
| 四态模板 | 各页面 loading/empty/notice/error 分支 |
| token 校验脚本 | `scripts/check-css-tokens.mjs`（移植时适配 rove 目录结构） |
| 样式对照页 | dev-only DesignSystem 页面路由 |
