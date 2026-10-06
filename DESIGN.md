# Design rules

Token values are authoritative in `apps/web/styles/v3/tokens.css` (skins, `--cp-*` prefix, plus the skin-independent `--z-*` ladder); structural and motion tokens live in `apps/web/styles/product-v2.css` (`--v2-*`, `--motion-*` prefixes); base tokens live in `apps/web/styles/tokens.css`. This file only covers usage.

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

### Graphite skin (`data-skin="graphite"`)

The default skin is a near-monochrome console: the page is pure white (`--cp-bg: #ffffff`), the rail drops one tier (`--cp-surface-warm: #f3f3f3`), and the accent is the ink ramp itself — no hue. Selection and focus read as light-on-dark in dark mode (`--cp-accent: #ffffff`, `--cp-accent-on: #181818`). Status tones (`--cp-success/-warning/-danger/-info/-purple`) carry the only color, each with an opaque `-soft` wash and a measured `-on` fill so `accessibility.spec.ts` clears 4.5:1 in both themes. Dark mode inverts the same ramp (`--cp-bg: #181818`, surfaces `#212121`/`#282828`).

## Depth and layering

- Elevation is same-hue alpha, never a raw `rgb()`: every shadow and translucent raised wash draws from the skin's elevation channel `--cp-elev-ch` (bridged as `--v2-elev-ch`). Common steps are `--cp-elev-1` (resting ≈5%), `-2` (raised ≈10%), `-3` (floating ≈18%, higher in dark themes); a one-off alpha writes `rgb(var(--cp-elev-ch) / <alpha>)`.
- `--cp-shadow-xs`/`-sm`/`-md` consume the channel; warm uses its brown ink (`60 40 20`), graphite and both dark themes use near-black.
- Stacking uses the semantic `--z-*` ladder defined on `.product-app-frame`: `--z-inset` → `--z-edge` → `--z-pane` → `--z-dock` → `--z-cover` → `--z-progress` → `--z-popover` → `--z-menu` → `--z-float` → `--z-peek` → `--z-drawer` → `--z-scrim` → `--z-palette` → `--z-search` → `--z-chrome` → `--z-modal`. A literal `z-index` in a rule is ladder drift; pick the step whose name matches the element's role.

## Typography and spacing

- Fonts: only `--font-ui` and `--font-mono`.
- Radii: only `--radius-sm` / `--radius-md` / `--radius-lg` / `--radius-xl` / `--radius-pill`.
- Shell dimensions use tokens: `--sidebar-nav-width` (left rail, resizable 240–520px, default 275px; bounds in `shell/use-sidebar-width.ts`), `--work-panel-request`/`--work-panel-resolved` (right panel width resolved against the shared 450px conversation floor), `--topbar-height` (46px; shared by the conversation top bar and the work-panel header). The legacy `--sidebar-width`/`--inspector-width` pair in `tokens.css` is not redefined in the v2 block — no v2 rule consumes it.
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
- On desktop the rail and the work panel collapse by animating their allocated width (flex-basis) so neighbors move with them; below 960px both become fixed overlays and slide by transform instead.

## Accessibility

- Icon buttons must have `aria-label`
- Form controls must have an associated label
- All actions must be keyboard-reachable; hover cards must not be triggered by keyboard focus
- Live status is announced through a live region
- Respect `prefers-reduced-motion`: disable sliding and looping animations
