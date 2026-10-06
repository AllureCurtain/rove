# Web console design

- Date: 2026-10-05 (v2; supersedes the 2026-10-03 draft)
- Status: **active** — implementation contract for the console rebuild. Phases land as one PR each.
- Scope: the `apps/web` product shell — layout, information architecture, default visibility, state presentation, copy. Does not cover the TUI, the Desktop host, or runtime contracts (the sole contract dependency is in §9.5).
- Companion: `DESIGN.md` (design rules and token usage) remains in effect; this document adds information architecture, target structure, and interaction specs.

---

## 1. Design goals and acceptance criteria

Rove's Web console is a **delegation surface**: the user hands a task to an agent that acts on its own, then waits for it to finish. The failure mode of this kind of UI is not "ugly", "slow", or "confusing" — it is **untrustworthy**.

The interface must continuously answer four questions without being asked:

| Must answer | Answered by | Cost when it can't |
| --- | --- | --- |
| **What is it doing right now?** | The current step in the run stream, stating what it is waiting on | The user stares at a spinner and kills a run 4 seconds from finishing |
| **Why is it doing that?** | Expandable raw tool inputs/outputs | One wrong output and every later output loses trust |
| **How much did it cost?** | A receipt line at the end of each run: steps · duration · tokens | The month-end bill arrives and the tool gets turned off |
| **How do I stop it?** | A persistent stop control, never inside a menu | The user closes the tab — which stops nothing |

Plus one structural criterion: **all three tenses must be reachable on the same screen, never behind navigation**.

```
future   queue / pending / plan      "what happens next, and can I intercept it"
present  the step running right now  "what it is doing at this moment, and whether it is stuck"
past     history / artifacts / cost  "what happened, why, and what it produced"
```

The moment "what is queued" requires a click to see, the user stops trusting the queue — and an untrusted queue quietly burns budget.

---

## 2. Current defects (mechanism-level)

Each was verified against source on `main`. ★ marks items this rebuild must fix.

### ★2.1 Right panel is open by default, and "collapsed" is not remembered

- `apps/web/inspector/use-work-panel.ts:70-76`: `collapsed: matchesDrawerLayout()` → open by default on desktop; only width persists (`:42-68`); `collapsed` is in-memory only. Left rail same problem (`shell/ProductApp.tsx:196-209`).
- Every refresh pushes a closed panel open again, occupying the most expensive horizontal space on screen.

### ★2.2 The default right-panel tab is the least informative one

`defaultWorkPanelTabs()` returns `status` (`inspector/work-panel-tabs.ts:47-49`), which unconditionally renders the export panel plus near-empty usage/timeline rows during an ordinary run.

### ★2.3 First launch is a form — and there are two different ones

Main area (`sidebar/EmptyState.tsx:84-168`) and the sidebar `+` (`sidebar/WorkspaceTree.tsx:995-1133`) are two differently-shaped forms; `folder | repo` is an internal storage distinction exposed on the first screen.

### ★2.4 The same fact is broadcast in five places

Top bar "API connected", sidebar row badge, transcript "replying…", composer "replying…", right panel "in progress…" — three unowned sources (`runState.busy`, catalog polling, request-inferred `connection`); divergence is only a matter of time.

### ★2.5 "Send" and "Stop" are both clickable while running

`canSubmit` (`chat/Composer.tsx:279-287`) does not check `busy`; a running send silently queues and the user cannot tell queue from interrupt.

### ★2.6 The send key runs opposite to mainstream convention

`chat/Composer.tsx:664-675`: Enter newlines, `Ctrl/Cmd+Enter` sends; the only hint is placeholder copy that disappears when typing starts.

### ★2.7 Copy contradicts itself

Sidebar says "add an absolute path from the sidebar" (`WorkspaceTree.tsx:391-392`, `copy/zh-CN.ts:797`) while the main area renders a different form — two instructions, neither matching the other.

### ★2.8 Model setup crams three intents on one screen

`settings/SettingsShell.tsx:763-966`: current selection + create/edit + saved list on one flat screen; the only credential field hides behind a fold.

### 2.9 Low-frequency export permanently occupies the right panel

`RunInspector.tsx:458` renders the export panel in the default tab unconditionally.

### 2.10 Session titles truncate with no plain-text fallback

Ellipsis-only; the full title is not reachable as text (`WorkspaceTree.tsx:830-847`).

### 2.11 Unwired switches and hardcoded strings

- "cool" skin selectable but has no token block (`styles/v3/tokens.css`, `shell/ui-skin.tsx`).
- Hardcoded strings bypassing copy files (`shell/TopBar.tsx:58`, `sidebar/EmptyState.tsx:111`, `settings/SettingsShell.tsx:560`).
- Inline styles bypassing tokens (`EmptyState.tsx:89-90`, `CatalogSettings.tsx`, `MemorySettings.tsx`, `SessionHoverCard.tsx:85`).
- `DESIGN.md:31` says 240px while the token is 248px.

### 2.12 Other

- Finished jobs are repeatedly polled and return 404 — a state-convergence problem.
- First-launch empty state competes with an empty sidebar block.

---

## 3. Target information architecture

Three columns = three tenses, all reachable on screen. The middle column is the primary column and is never squeezed below its floor; the right column borrows width from the sidebar first, and collapses the sidebar rather than compressing the middle.

```
┌────────────┬─────────────────────────────────┬──────────────┐
│ sidebar    │  main  ≥450px                   │  work panel  │
│ ~240–520px │                                 │  resizable   │
│            │                                 │              │
│ pinned     │  conversation topbar (46px)     │  tab strip   │
│ sessions   │  run stream                     │  ├ files     │
│  (time     │   ├ turns (user bubble /        │  ├ review    │
│   groups)  │   │  assistant prose)           │  ├ subagent  │
│ projects   │   ├ tool rows (grouped,         │  └ +new tab  │
│            │   │  timed, foldable)           │              │
│            │   ├ approval cards inline       │              │
│            │   └ receipt: steps·time·tokens  │              │
│ footer     │                                 │              │
│ icons      │  [composer dock: queue · asks · │              │
│            │   plan bar · todo · input ·     │              │
│            │   send/stop one slot]           │ (closed by   │
└────────────┴─────────────────────────────────┴  default)    ┘
```

### 3.1 Left column (future + past): the session rail

Zones top→bottom: header (brand + collapse) → Pinned sessions → Sessions (standalone sessions grouped by time) → Projects (collapsible workspace groups) → footer icon row. Details in §7.

### 3.2 Middle column (present): the conversation surface

Conversation topbar (title + new task + search) above a retained session pane: transcript scroll + composer dock. Details in §5 (run stream) and §6 (composer).

### 3.3 Right column (the evidence surface of past): the work panel

- **Closed by default**; open state and width both persist across reloads.
- **Only the user opens it**: the floating toggle at the top-right edge, a keyboard shortcut, or clicking an artifact/review card in the transcript. No agent action may auto-expand the panel.
- Tab strip supports close, drag-reorder, keyboard reorder, and "+" new tab. Default tab is the evidence view (review/files), not a status dump. Details in §8.

### 3.4 Width budget

Middle column floor 450px; sidebar yields first when budget is short; the work panel's drag auto-collapses the sidebar before touching the middle floor. Keep the existing policy in `inspector/work-panel-layout.ts` — the target differs only in that the sidebar collapse is animated (§4.4).

---

## 4. Shell and visual system

### 4.1 Skin: a new dark skin, set as default

Add a second skin — a near-monochrome dark theme ("graphite") — alongside warm ivory, and make it the default. Warm stays selectable. The cool-skin selector entry is removed (or implemented) per §2.11 — nothing unwired may ship.

Palette (dark; light is an inverse ramp):

```
base surfaces   #181818 (page) · #212121 (secondary/dock) · #282828 (tile) · #0d0d0d (inset)
sidebar         #000000-adjacent darkest tier, separate token so a lighter variant can move it
text            #fff (primary) · 70% alpha (secondary) · 52% (muted) · 38% (faint)
accent          the same gray ramp — no blue accent; "selected"/"focus" read as light-on-dark
borders         white at 8% (default) / 5% (subtle) / 14% (strong)
status          success #40c977 · warning #ff8549 · error #ff6764 · info gray-300 · subagent #c27aff
send-disabled   18% white bg on dark, disabled ink derived from page bg
shadows         elevation by shadow, not strokes: dialog 0 16px 48px rgba(0,0,0,.55);
                composer 0 3px 7.5px rgba(0,0,0,.04) + 0 0 20px rgba(0,0,0,.05)
```

**Surface discipline**: in-flow surfaces are borderless — hierarchy comes from three tones (page / tile / raised+shadow). Border tokens appear only on floating layers (menus, dialogs, tooltips) and the todo dock card.

All values land as design tokens (`styles/v3/tokens.css` skin block + the `--ds-*`-style semantic layer mapped onto the existing `--cp-*`/`--v2-*` bridge). `pnpm lint:style-tokens` must keep passing — no raw literals in components.

### 4.2 Type ramp and geometry

| Token family | Target |
| --- | --- |
| Text sizes | 10.5 / 11 / 11.5 / 12 / 12.5 / 13 / 13.5 / 14 / 15 / 16 / 18 / 20 / 28px ramp (xs…2xl); thread rows 13px, tool rows 12.5px, summaries 11.5px |
| Line heights | row 18px · body 1.45 · chat 1.55 · prose 1.6 |
| Radius | 4 / 6 / 8 / 10 / 12 / 14 / 16 / 18 / 20 / 24px + pill; message bubble 18px with an 8px corner on the user's own side; composer shell 20px |
| Icon button | 28px square hit area, 10–12px radius, transparent → hover fill |
| Toolbar height | 46px, shared by conversation topbar and work-panel header |
| Content band | transcript and composer share one centered max width (~760px); reading-width handle kept |
| Row pitch | 28px minimum for every sidebar row and group header; 1px gaps between rows, not border hairlines |

### 4.3 Motion

Durations: 150ms fast / 200ms normal / 300ms slow; easing `cubic-bezier(.22,1,.36,1)` out. Column open/close animates allocated width (`flex-basis`/`width`/`opacity`/8px translate together), never transform-only — content keeps a pinned min-width while `overflow:hidden` wipes it away so text does not reflow mid-animation. Disclosure folds use `grid-template-rows: 1fr → 0fr` with a clip layer. Every pulse/spinner/translate must have a `prefers-reduced-motion` off.

### 4.4 One persistent animation per page

While no run is active, nothing moves. The single allowed ambient animation is the running-state indicator in the transcript tail lane (§5.3).

---

## 5. Run stream spec (middle column)

### 5.1 Retained session panes

The conversation surface keeps the last N visited session panes mounted (`Map<sessionId, pane>`). Switching flips `data-visible`: hidden panes keep DOM and scroll position but take `inert` + `aria-hidden` + `visibility:hidden` — never `display:none`. A cold switch shows a 2px top progress bar and a transcript skeleton (§5.5); the composer stays usable.

### 5.2 Transcript structure

```
.thread-wrap
 ├─ conversation minimap            (existing; keep, restyle to skin)
 ├─ .thread-scroll (role=log, scrollbar-gutter:stable, overflow-anchor:none,
 │     bottom mask fades rows into the composer reserve instead of a solid band)
 │   └─ .thread-content (centered content band)
 │        ├─ history-load sentinel ("loading earlier…")
 │        ├─ entries: message | compaction divider | assistant-turn
 │        ├─ turn outcome card      (end of turn: duration · steps · tokens — the receipt)
 │        ├─ permission/input cards (inline, never inside a fold)
 │        └─ runtime status lane    (§5.3)
 ├─ settle veil (skeleton rows stacked from the bottom during session switch)
 └─ jump-to-latest button (32px round, raised surface, appears when follow is off)
```

- **User message**: right-aligned bubble, `max-content` width capped at 82%/600px, 18px radius with a small corner on the user's side, subtle tinted fill, no byline.
- **Assistant message**: bare markdown prose at the shared content width — no bubble, no card, no per-message role label. Hover reveals quiet icon actions in a row *below* the message (copy / retry on the last turn; edit / copy-session on the user's own), never inside the body — the body stays exactly the model's or sender's text.
- **Compaction**: a centered hairline-flanked label row.
- **Offscreen economy**: rows carry `content-visibility:auto` with a ~140px intrinsic size — required for long transcripts.

### 5.3 Runtime status lane

A reserved min-height lane at the transcript tail exists for the whole running turn so indicators mount and clear without shifting rows the user is reading. The lane renders exactly one indicator at a time:

- **working** — three staggered dots + phase label + elapsed (tabular-nums)
- **planning** — same row, info tint
- **run activity** — same row tinted by phase: waiting-for-model/starting/preparing/compacting (info), retrying/recovering (warning, hover reveals the provider error popover), waiting-for-subagents (accent-purple)

This lane is the **single owner** of "what is it doing right now" (§6.4 state convergence); the composer does not repeat it.

### 5.4 Tool rows and activity groups

Every tool call is one compact row; consecutive calls within a turn fold into one disclosure group ("N actions · elapsed").

```
.tool-activity-group
 ├─ header: icon · label · count · live-preview line (last activity, single line) · caret
 └─ body (grid-rows fold): .tool-row × N
      ├─ head: icon · name(medium) · summary(mono, single-line: command/path) · state(dot+text/spinner/denied) · caret
      │        run rows also carry a 20px copy button revealed on hover/focus
      └─ body: inputs/outputs verbatim, mono, independently copyable
```

- Row pitch: header min-height 24–26px, secondary ink → primary on hover, state dot 6px (done=green, error=red, running=pulsing secondary), spinner is an 11px ring.
- Running group rows auto-expand the active item; **a fold the user opened never auto-collapses** on stream updates or completion — that is a trust issue, not a preference.
- **Failures expand by default**; approvals and pending inputs never enter the fold (current behavior, kept).
- Labels name the object (`Read orders.csv (2,481 rows)`), never the category; every row shows its duration — the only thing distinguishing "slow" from "stuck".
- Expanded evidence renders result first, inputs after; no tool payload renders as prose.
- Subagent fan-out lifts the whole group onto a tile card (deeper surface, 42px header).

### 5.5 Session-switch skeleton

On a cold switch, an opaque veil covers the transcript (not the composer) with skeleton rows stacked from the bottom — user rows right-aligned — fading once geometry settles. Streaming must never relayout the composer.

### 5.6 Receipt

End of run: `9 steps · 42.6s · 18.2k tokens` — quiet mono, `tabular-nums`, part of the stream. Per-step tokens appear only inside expanded rows. Context compaction is written into the stream as a step (the existing `chat/CompactionPanel.tsx` plugs in). Model identity is turn metadata in the header, not a per-row label.

---

## 6. Composer spec

```
.composer-dock (absolute bottom, transparent — the transcript mask handles the fade)
 └─ .composer-stack (shared content band)
     ├─ TodoDock           session todo fold: header (symbol · "n/m done · current item") + row list
     ├─ PlanApprovalBar    pending plan (approve / edit / reject) — when the runtime exposes one
     ├─ AskToolCard        pending question card on the composer plate surface
     ├─ queued prompts     FIFO rows (text + send-now + actions; next-up row gets an accent edge)
     └─ .composer-shell    20px-radius elevated shell, shadow-only (no stroke), focus deepens shadow
         ├─ input          3–7 lines auto-grow, transparent, inline attachment chips
         ├─ autocomplete   @file / command popup anchored to the shell
         └─ toolbar        left: + attach · mode chip · permission picker
                           right: context usage ring · model picker · Send/Stop (§6.1)
```

### 6.1 Send and stop share one slot

`runActive && !hasDraft` → **Stop** (28px round, tinted fill); otherwise **Send** (accent fill). A draft typed during a run keeps Send visible — its tooltip reads "send while running (Alt+Enter)" — so queue/interject is explicit: **Enter** queues, **Alt+Enter** interjects. The server already supports both (`chat/Transcript.tsx:1008-1021`); only the entry points move to the composer.

### 6.2 Send key

Default **Enter sends, Shift+Enter newlines**; a preference restores Ctrl/Cmd+Enter. The hint must not live only in the placeholder — keep a persistent hint or `?` entry in the composer.

### 6.3 Queued prompts

Server-persisted FIFO stays above the shell; each row: text (ellipsis) + send-now + remove; the promoted next-up row reads as the queue head (elevated surface + accent edge) and locks its own actions until delivered.

### 6.4 State convergence (single ownership)

| Fact | Single source | Where it may render |
| --- | --- | --- |
| Turn running | `runState.busy` | transcript tail lane + composer slot state |
| Session status | catalog `session.status` | left-rail row status slot only (shape+motion, no text repeat) |
| Connection health | `connection` | top bar **only when abnormal**; silent when healthy |

No percentages, no second progress card, no "replying…" duplicated in five places (§2.4).

---

## 7. Sidebar spec (left column)

### 7.1 Structure

```
aside.sidebar (dark separate tier, resizable 240–520px, animated collapse)
 ├─ header: brand + collapse icon button (28px)
 ├─ body (scroll column, pinned min-width during collapse animation)
 │   ├─ Pinned      section (renders only when non-empty; capped inner scroll ~30vh)
 │   ├─ Sessions    toolbar (label + sort + "+") → time groups
 │   │              (today / yesterday / this week / older / archived; capped inner
 │   │               scroll; per-group "load more" pagination row)
 │   ├─ Projects    toolbar (label + "+")
 │   └─ groups      collapsible project sections (chevron + name + count; body folds
 │                  via grid-rows animation; rows indented ~22px)
 └─ footer: settings / plugins-scheduled icons + notification slot + version
 └─ resize handle (8px grab strip; 2×32px capsule appears on hover/focus;
     arrows ±16px / Shift ±32px / Home·End / double-click reset; aria-separator)
```

### 7.2 Session row

28px pitch, 10px radius, secondary ink: `status dot · [pin icon] · title (13px ellipsis) · project name (12px muted) · hover ⋯ button`. States: hover/active fills; multi-select tint; running pulse; archived dimmed; `data-window-blur` suppresses hover. Status slot differentiates by shape+motion, not color alone (§3.1 keeps the queue-count slot).

### 7.3 Interactions

- **Row menu** (⋯ and right-click): Rename → Pin/Unpin → Archive/Restore → Branch (disabled while running) → Copy link → **Delete with armed confirm** (first click turns the item into "Delete?", second executes; blur disarms).
- **Multi-select**: Ctrl/Shift selection → menu degrades to batch Archive/Delete.
- **Drag**: session → project group reassigns; project rows reorder with before/after indicator lines.
- **Hover card**: delayed rich card (project path, message count, run state, open action) with a hover bridge.
- **Project header menu**: open folder / rename / pin / archive / delete (armed, dialog lists session count).
- **Collapse**: the rail animates to zero width and the topbar grows a lead expand button (fixed 36px lane, animated with the rail).

---

## 8. Work panel spec (right column)

```
aside.work-panel (flex-basis = persisted width; dock surface one tier above page)
 ├─ resize handle (10px grab strip on the left edge; same capsule + keyboard rules)
 ├─ header (46px, raised dock surface)
 │   ├─ tab strip (32px high, horizontal scroll, hidden scrollbar)
 │   │    └─ tabs: 92–180px, icon + label + close× (close revealed on hover/active/focus);
 │   │        drag-reorder with 2px edge indicators; Alt+←/→ keyboard reorder;
 │   │        Delete/Backspace closes, focus falls to the right neighbor
 │   └─ actions: "+" new tab · maximize
 └─ body: active tab pane (hidden panes keep state)
```

- **Tabs**: files (per opened file), review (aggregated diff), subagent transcripts, plus "+"-spawned views. Closing is always allowed; the strip never traps the user.
- **Maximized** mode takes the middle column's width (the middle hides, header slides clear of the chrome lane); the resize handle is disabled while maximized.
- **Toggle**: floating 28px button fixed near the top-right edge; pressed state swaps the icon; disabled when no session exists.
- **Persistence**: width, open/closed, and tab order persist; refresh restores exactly.
- **Files tab**: breadcrumb directory browsing + syntax-highlighted viewer (line cap ~5000, markdown renders as Markdown) + open-externally.
- **Review tab**: header "N changes +a −d" + one collapsible diff card per file.
- **Empty tab**: one shared empty-state component (icon + title + one line), never a blank pane.

---

## 9. First launch and model setup

### 9.1 Empty state: from form to guidance

Hero (product mark + one-line prompt) + the **home variant of the same composer** + a workspace switcher inline in the hero copy (dotted-underline project name opens the picker menu) + optional onboarding checklist + recent sessions. The absolute-path input and the `folder|repo` selector are demoted behind a collapsed "enter manually"; the sidebar `+` and the empty state share **one** selection flow.

### 9.2 No model configured

Composer accepts input; Send disabled with a "configure a model first" hint; an actual submit returns an inline message with a direct action.

### 9.3 First-run checklist

Dismissible checklist where each row is a deep-link button; completed rows get a strikethrough check. No blocking wizard.

### 9.4 Model setup: three layers

Current selection (read-mostly card) · saved profiles list (name/type/base URL/credential status/test/default/edit/armed delete) · add/edit as an overlay (preset → name/base URL/**API key**/wire format, advanced folded).

### 9.5 Entering keys in the browser (the sole contract dependency)

Unchanged from the v1 draft: a `PUT /product/provider-profiles/{id}/credential`-style loopback-only contract whose body carries only the key and never echoes it back; the field exists only inside the overlay and clears on submit; update `docs/decisions.md`, `docs/api.md`, and the OpenAPI snapshot together.

---

## 10. Implementation mapping

### 10.1 Category A: structure + component behavior

| # | Change | Main touch points |
| --- | --- | --- |
| A1 | Graphite skin tokens + default switch; remove/ship "cool" | `styles/v3/tokens.css`, `shell/ui-skin.tsx`, `settings/SettingsShell.tsx:290` |
| A2 | Retained session panes + settle veil + switch progress | `chat/Transcript.tsx` → `chat/` split (`ChatSurface`, `SessionPane`) |
| A3 | Transcript entry model (message / compaction / assistant-turn+parts) + ToolRow + ActivityGroup per §5.4 | `chat/Transcript.tsx` restructure |
| A4 | Composer stack per §6 (send/stop slot, Enter default, queue rows, TodoDock slot, approval/ask plates) | `chat/Composer.tsx` |
| A5 | Runtime status lane + single ownership (§5.3, §6.4) | `shell/ProductApp.tsx`, `shell/TopBar.tsx`, `chat/Composer.tsx`, `inspector/RunInspector.tsx` |
| A6 | Sidebar per §7 (zones, time groups, menus, multi-select, drag, hover card, footer, resize) | `sidebar/WorkspaceTree.tsx` → `sidebar/SessionRail.tsx` |
| A7 | Conversation topbar per §7.1 layout (title + new task + search; connection only when abnormal) | `shell/TopBar.tsx` |
| A8 | Work panel per §8 (tab management, default tab, persistence, maximize, files breadcrumbs, shared empty) | `inspector/` (`use-work-panel.ts`, `work-panel-tabs.ts`, `RunInspector.tsx`) |
| A9 | Empty state + unified workspace flow (§9.1) | `sidebar/EmptyState.tsx`, `sidebar/WorkspaceTree.tsx` |
| A10 | Model setup three layers + in-browser key (§9.4-9.5) | `settings/SettingsShell.tsx`, new product contract |
| A11 | Receipt in stream + export into session menu | `inspector/RunInspector.tsx`, `inspector/ExportPanel.tsx` |
| A12 | Session pin/archive persistence (product-store fields if missing) | `apps/product-store`, contract migration |

### 10.2 Category B: tokens / copy / consistency

| # | Change | Touch points |
| --- | --- | --- |
| B1 | Hardcoded strings → copy files | `TopBar.tsx:58`, `EmptyState.tsx:111`, `SettingsShell.tsx:560` |
| B2 | Inline styles → tokens | `EmptyState.tsx:89-90`, `CatalogSettings.tsx`, `MemorySettings.tsx`, `SessionHoverCard.tsx:85` |
| B3 | Fix 240 vs 248 in `DESIGN.md:31`; document the graphite token set | `DESIGN.md` |
| B4 | Semantic z-index ladder + same-hue alpha elevation tokens | `styles/v3/tokens.css`, `DESIGN.md` |

### 10.3 Tests that will be affected (update in sync; never delete tests)

| Test | Current assertion | Impact |
| --- | --- | --- |
| `tests/e2e/shell.spec.ts:80-87` | right panel expanded on load | changes for §3.3 default-closed |
| `tests/e2e/shell.spec.ts:13,170-171` | first-launch copy "absolute path / open workspace" | changes for §9.1 |
| `tests/e2e/settings.spec.ts:412` | export visible on session row | changes for A11 |
| `tests/e2e/settings.spec.ts:67-71` | key field behind "advanced options" | changes for A10 |
| `tests/e2e/layout.spec.ts:312-391` | dynamic closable tab strip | **keep**; only default changes |
| `tests/e2e/layout.spec.ts:14,19` | top bar 52px → 46px; 450px middle floor | update 46px; keep floor |
| `tests/e2e/reading-width.spec.ts` | reading band + handles | **keep** |
| `tests/e2e/accessibility.spec.ts` | panel collapse path, focus, inert | **keep**, re-run both skins × both themes |
| `tests/e2e/polish.spec.ts:40-94` | 375px drawer/focus-trap/inert | **keep** |
| `apps/web/product/production-ui-boundary.test.ts` | no design-mock imports; pinned copy keys | **keep**, sync when copy changes |

---

## 11. Phases and acceptance

One PR per phase, worktree per phase, rebase merge.

- **P0 — Skin and shell**: graphite token set + default; shell chrome (46px topbar, floating work-panel toggle, animated sidebar collapse with topbar lead button, column width animations). *Accept*: `pnpm test && pnpm typecheck && pnpm lint:style-tokens`; three-column geometry and collapse/expand animation match spec in both themes.
- **P1 — Conversation surface**: retained panes + settle veil, entry model + ToolRow/ActivityGroup, user bubble / assistant prose, runtime status lane, jump-to-latest, empty-state hero + home composer, composer stack (queue rows, approval/ask plates, send/stop slot, Enter default). *Accept*: a real run reads per §5-6 end-to-end; §2.4-§2.6 defects closed.
- **P2 — Sidebar**: zones, time groups + pagination, full menus with armed delete, multi-select, drag, hover card, footer, resize interactions; session pin/archive backend fields if missing (A12). *Accept*: §7 interactions all present; e2e covers armed delete and pin persistence.
- **P3 — Work panel**: tab model (close/reorder/new), default evidence tab, files breadcrumbs + highlight, maximize, full persistence, shared empty. *Accept*: §8 behaviors verified; §2.1/§2.2/§2.9 closed.
- **P4 — Polish**: global search entry, minimap alignment, TodoDock (if backend), keyboard map, responsive, a11y regression in both skins × both themes, category B cleanup.

Every phase also answers: the four questions in §1 unambiguously, and its slice of the §2 defect list is closed out.
