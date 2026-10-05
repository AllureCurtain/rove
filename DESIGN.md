# Design rules

Token values are authoritative in `apps/web/styles/v3/tokens.css` (current skin, `--cp-*` prefix); structural and motion tokens live in `apps/web/styles/product-v2.css` (`--v2-*`, `--motion-*` prefixes); base tokens live in `apps/web/styles/tokens.css`. This file only covers usage.

Styles are layered in three tiers, imported in order by `app/layout.tsx`: `product.css` (base reset and shared component rules) → `product-v2.css` (shell layout and structure, scoped by `data-ui-version="v2"`) → `v3/index.css` (tokens and skins only).

- Layout geometry may only be written in the layer that renders it.
- Later layers must not re-derive an earlier layer's width budgets.
- The v3 layer contains no layout, widths, or breakpoints.

This file covers only visual and component rules; information architecture, default visibility, and interaction specs live in [`docs/web-console-design.md`](docs/web-console-design.md).

## Color

| token | usage |
| --- | --- |
| `--cp-bg` / `--cp-surface` / `--cp-surface-raised` | page background, panels, overlays |
| `--cp-fg` / `--cp-fg-2` | body text, secondary text |
| `--cp-muted` / `--cp-meta` | helper text, metadata; never for body text |
| `--cp-border` / `--cp-border-soft` / `--cp-border-strong` | dividers, subtle dividers, emphasis borders |
| `--cp-accent` / `-hover` / `-on` / `-soft` | primary actions, links, selection; `-on` is text on filled backgrounds; `-soft` is tag background |
| status colors (success, warning, error) `*` / `-on` / `-soft` | status dots, badges, error messages |

- Every text/background combination must reach at least 4.5:1 contrast.
- `-soft` uses opaque values so a tag keeps the same contrast on any background.
- `tests/e2e/accessibility.spec.ts` verifies these pairings under both themes and both skins.
- Theme switches via `html[data-theme="dark"]`; skin switches via `.product-app-frame[data-skin]`. Never write raw color values.

## Typography and spacing

- Fonts: only `--font-ui` and `--font-mono`.
- Radii: only `--radius-sm` / `--radius-md` / `--radius-lg` / `--radius-pill`.
- Shell dimensions use tokens: `--sidebar-width` (fixed 240px left rail), `--inspector-width`, `--topbar-height`.
- Motion:
  - Durations only via `var(--motion-duration-*)`; easings only via `var(--motion-ease-*)`, `linear`, or `steps(...)`.
  - `pnpm lint:style-tokens` enforces this.
  - For a genuine exception, add a `style-token: allow` comment next to the declaration and explain why.

## Component states

Every interactive component needs: default, hover, focus (keyboard-visible `--focus-ring`), disabled, and loading. An operation waiting on a server result must be disabled and show its state — do not fake success.

## Page states

- Every data page handles four states: loading, empty, error, and normal.
- Transcripts also handle "partial recovery": state clearly what is missing and why.
- A problem already visible in the page (migration, partial recovery, the session being viewed) must not be re-announced via toast.

## Responsive

- Breakpoints: 480 / 760 / 960 / 1180px.
- Below 960px the Inspector becomes a drawer, per `DRAWER_MAX_WIDTH` and `DRAWER_MEDIA_QUERY` in `lib/viewport-breakpoints.ts`.
- The left rail collapses with a transform slide; do not animate the grid track, which would force a full-page relayout.

## Accessibility

- Icon buttons must have `aria-label`
- Form controls must have an associated label
- All actions must be keyboard-reachable; hover cards must not be triggered by keyboard focus
- Live status is announced through a live region
- Respect `prefers-reduced-motion`: disable sliding and looping animations
