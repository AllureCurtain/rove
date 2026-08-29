# 附录 A — Rove UI V3 完整 Design Token CSS

> 归属文档：`2026-08-24-frontend-ui-v3-restyle-implementation.md`
> 落地位置：`apps/web/styles/v3/tokens.css`。全部值取自 cc-partner `web/src/styles/tokens.css`，按 rove 需要裁剪并追加存量变量桥接段。

```css
/* ============================================================
   Rove UI V3 tokens — Anthropic 暖米色系统 + terracotta 强调色
   单一颜色/字体/间距来源；v3 作用域内禁止硬编码色值/尺寸。
   品牌字体不可分发，仅声明 fallback 栈。
   ============================================================ */

/* ---------- 浅色（默认） ---------- */
.product-app-frame[data-ui-version="v3"] {
  color-scheme: light;

  /* 颜色 */
  --cp-bg: #f5f4ed;
  --cp-surface: #faf9f5;
  --cp-surface-warm: #e8e6dc;
  --cp-fg: #141413;
  --cp-fg-2: #3d3d3a;
  --cp-muted: #5e5d59;
  --cp-meta: #87867f;
  --cp-border: #f0eee6;
  --cp-border-soft: #e8e6dc;
  --cp-border-strong: #d9d5c9;

  --cp-accent: #c96442;
  --cp-accent-on: #faf9f5;
  --cp-accent-soft: color-mix(in oklab, var(--cp-accent) 14%, transparent);
  --cp-accent-hover: color-mix(in oklab, var(--cp-accent) 90%, #141413);

  --cp-success: #17a34a;
  --cp-success-soft: color-mix(in oklab, var(--cp-success) 12%, transparent);
  --cp-warning: #eab308;
  --cp-warning-soft: color-mix(in oklab, var(--cp-warning) 14%, transparent);
  --cp-danger: #b53333;
  --cp-danger-soft: color-mix(in oklab, var(--cp-danger) 12%, transparent);

  --cp-selection: rgba(201, 100, 66, 0.18);
  --cp-overlay: rgba(20, 20, 19, 0.55);
  --cp-focus-ring: var(--cp-accent);

  /* 字体 */
  --cp-font-display: Georgia, "Times New Roman", serif;
  --cp-font-body: system-ui, -apple-system, "Segoe UI",
                  "Helvetica Neue", Arial, sans-serif;
  --cp-font-mono: ui-monospace, "SFMono-Regular", Menlo,
                  Consolas, monospace;

  /* 字号十档 */
  --cp-text-xs: 11px;  --cp-text-sm: 12px;  --cp-text-base: 13px;
  --cp-text-md: 14px;  --cp-text-lg: 16px;  --cp-text-xl: 20px;
  --cp-text-2xl: 24px; --cp-text-3xl: 32px; --cp-text-4xl: 40px;
  --cp-text-5xl: 56px;

  --cp-leading-tight: 1.2;   --cp-leading-normal: 1.5;  --cp-leading-relaxed: 1.6;
  --cp-weight-regular: 400;  --cp-weight-medium: 500;
  --cp-weight-semibold: 600; --cp-weight-bold: 700;
  --cp-tracking-tight: -0.015em; --cp-tracking-normal: 0;
  --cp-tracking-wide: 0.02em;    --cp-tracking-widest: 0.14em;

  /* 间距（4px 网格） */
  --cp-space-1: 4px;  --cp-space-2: 8px;   --cp-space-3: 12px;
  --cp-space-4: 16px; --cp-space-5: 20px;  --cp-space-6: 24px;
  --cp-space-8: 32px; --cp-space-10: 40px; --cp-space-12: 48px;
  --cp-space-16: 64px; --cp-space-20: 80px;

  /* 圆角 */
  --cp-radius-xs: 4px; --cp-radius-sm: 6px; --cp-radius-md: 8px;
  --cp-radius-lg: 12px; --cp-radius-xl: 16px; --cp-radius-full: 9999px;

  /* 阴影 */
  --cp-shadow-xs: 0 1px 2px rgba(0, 0, 0, 0.04);
  --cp-shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.04), 0 2px 8px rgba(0, 0, 0, 0.06);
  --cp-shadow-md: 0 4px 12px rgba(0, 0, 0, 0.08);
  --cp-shadow-lg: 0 12px 32px rgba(0, 0, 0, 0.12);
  --cp-shadow-window: 0 18px 56px rgba(0, 0, 0, 0.18);

  /* 动效 */
  --cp-motion-fast: 150ms; --cp-motion-base: 200ms; --cp-motion-slow: 300ms;
  --cp-ease-standard: cubic-bezier(0.2, 0, 0, 1);
  --cp-ease-emphasized: cubic-bezier(0.32, 0.72, 0, 1);

  /* 布局（对齐 rove 现有尺寸） */
  --cp-sidebar-width: 280px;
  --cp-inspector-width: 320px;
  --cp-topbar-height: 52px;

  /* z-index */
  --cp-z-base: 0; --cp-z-sticky: 10; --cp-z-overlay: 100;
  --cp-z-modal: 1000; --cp-z-toast: 2000;

  /* ==========================================================
     桥接段：rove 存量变量名 → v3 值。
     未改写的存量规则自动获得暖色系；逐块替换后改引 --cp-* 本名。
     ========================================================== */
  --bg: var(--cp-bg);
  --bg-elevated: var(--cp-surface);
  --surface: var(--cp-surface);
  --surface-strong: var(--cp-surface);
  --surface-soft: var(--cp-surface-warm);
  --surface-muted: var(--cp-surface-warm);
  --text: var(--cp-fg);
  --text-secondary: var(--cp-fg-2);
  --muted: var(--cp-muted);
  --border: var(--cp-border);
  --border-strong: var(--cp-border-strong);
  --accent: var(--cp-accent);
  --accent-hover: var(--cp-accent-hover);
  --accent-soft: var(--cp-accent-soft);
  --success: var(--cp-success);
  --warning: var(--cp-warning);
  --error: var(--cp-danger);
  --focus-ring: var(--cp-focus-ring);
  --selection: var(--cp-selection);
  --overlay: var(--cp-overlay);
  --shadow-sm: var(--cp-shadow-xs);
  --shadow-md: var(--cp-shadow-lg);

  /* v1/v2 变量桥（保证三版并存期间 v3 下无残留冷色） */
  --v2-canvas: var(--cp-bg);
  --v2-bg: var(--cp-bg);
  --v2-surface: var(--cp-surface);
  --v2-surface-raised: var(--cp-surface);
  --v2-surface-sunken: var(--cp-surface-warm);
  --v2-surface-hover: var(--cp-surface-warm);
  --v2-ink: var(--cp-fg);
  --v2-ink-2: var(--cp-fg-2);
  --v2-ink-3: var(--cp-muted);
  --v2-border: var(--cp-border);
  --v2-border-strong: var(--cp-border-strong);
  --v2-signal: var(--cp-accent);
  --v2-signal-strong: var(--cp-accent-hover);
  --v2-signal-soft: var(--cp-accent-soft);
  --v2-on-signal: var(--cp-accent-on);
}

/* ---------- 深色 ---------- */
html[data-theme="dark"] .product-app-frame[data-ui-version="v3"] {
  color-scheme: dark;

  --cp-bg: #1f1d1b;
  --cp-surface: #292524;
  --cp-surface-warm: #33302d;
  --cp-fg: #faf9f5;
  --cp-fg-2: #e5e2dc;
  --cp-muted: #a8a49c;
  --cp-meta: #78756e;
  --cp-border: #33302d;
  --cp-border-soft: #2b2825;
  --cp-border-strong: #443f3a;

  --cp-accent: #d97757;
  --cp-accent-on: #1f1d1b;
  --cp-success: #4ade80;
  --cp-warning: #fbbf24;
  --cp-danger: #e07a6a;
  --cp-accent-soft: color-mix(in oklab, var(--cp-accent) 16%, transparent);
  --cp-success-soft: color-mix(in oklab, var(--cp-success) 16%, transparent);
  --cp-warning-soft: color-mix(in oklab, var(--cp-warning) 16%, transparent);
  --cp-danger-soft: color-mix(in oklab, var(--cp-danger) 16%, transparent);

  --cp-selection: rgba(217, 119, 87, 0.28);
  --cp-overlay: rgba(10, 9, 8, 0.65);
}
```

## base.css 关键片段（随 tokens 一并落地）

```css
.product-app-frame[data-ui-version="v3"] ::selection { background: var(--cp-selection); }
.product-app-frame[data-ui-version="v3"] :focus-visible {
  outline: 2px solid var(--cp-focus-ring); outline-offset: 2px; border-radius: var(--cp-radius-xs); }

.product-app-frame[data-ui-version="v3"] .cp-sr-only {
  position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }

@keyframes cp-pulse { 50% { opacity: 0.35; } }
@keyframes cp-shimmer {
  from { background-position: 200% 0; }
  to   { background-position: -200% 0; } }

@media (prefers-reduced-motion: reduce) {
  .product-app-frame[data-ui-version="v3"] *,
  .product-app-frame[data-ui-version="v3"] *::before,
  .product-app-frame[data-ui-version="v3"] *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important; } }
```

## check-css-tokens.mjs 校验规则（移植自 cc-partner，按 rove 目录适配）

扫描 `apps/web/styles/v3/*.css`：
1. 除 `tokens.css` 外禁止出现 `#[0-9a-fA-F]{3,8}` 字面量颜色；
2. 禁止出现未列入 token 白名单的裸 `px` 尺寸（允许 `0`、`1px` 边框与 keyframes 内部值）；
3. 所有 `var(--…)` 引用必须指向已定义的 token 或白名单的存量桥接变量；
4. 违规即非零退出并列出文件：行号。
