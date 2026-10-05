# Web console design

- Date: 2026-10-03
- Status: **draft (pending review)**
- Scope: the `apps/web` product shell — layout, information architecture, default visibility, state presentation, copy. Does not cover the TUI, the Desktop host, or runtime contracts (the sole contract dependency is in §6.5).
- Companion: `DESIGN.md` (design rules and token usage) remains in effect; this document only adds information architecture and interaction specs.

---

## 1. Design goals and acceptance criteria

Rove's Web console is a **delegation surface**: the user hands a task to an agent that acts on its own, then waits for it to finish. The failure mode of this kind of UI is not "ugly", "slow", or "confusing" — it is **untrustworthy**.

So the acceptance criteria are four questions the interface must continuously answer without being asked:

| Must answer | Answered by | Cost when it can't |
| --- | --- | --- |
| **What is it doing right now?** | The current step in the run stream, stating what it is waiting on | The user stares at a spinner and kills a run 4 seconds from finishing |
| **Why is it doing that?** | Expandable raw tool inputs/outputs | One wrong output and every later output loses trust |
| **How much did it cost?** | A receipt line at the end of each run: steps · duration · tokens | The month-end bill arrives and the tool gets turned off |
| **How do I stop it?** | A persistent stop control, ≥44px, never inside a menu | The user closes the tab — which stops nothing |

Plus one structural criterion: **all three tenses must be reachable on the same screen, never behind navigation**.

```
future   queue / pending / plan      "what happens next, and can I intercept it"
present  the step running right now  "what it is doing at this moment, and whether it is stuck"
past     history / artifacts / cost  "what happened, why, and what it produced"
```

The moment "what is queued" requires a click to see, the user stops trusting the queue — and an untrusted queue quietly burns budget.

**Current verdict**: the page skeleton (left rail + main column + right panel, right panel becoming a drawer below `960px`) is right, but **the semantic mapping of the three columns is inverted** — "what it is doing now" sits in the expanded-by-default right panel while the main column holds only chat bubbles. It reads as a dashboard wrapped around a chat window.

---

## 2. Current defects (mechanism-level)

Each was verified against source on current `main`. ★ marks items this document requires fixing.

### ★2.1 Right panel is open by default, and "collapsed" is not remembered

- `apps/web/inspector/use-work-panel.ts:70-76`: initial state `collapsed: matchesDrawerLayout()`, which is `false` on desktop viewports — i.e., open by default.
- Same file: `STORAGE_KEY = "rove.ui-work-panel-width"` (`:42`) **persists only the width** (`writeStoredPanelWidth` `:59-68`, called only by `setWidth` `:137`); `collapsed` lives only in the in-memory store (`:85`) and is lost on unload.
- Consequence: every time the user closes it, a refresh pushes it open again — and it occupies the most expensive horizontal space on screen.
- The same problem exists on the left rail: `apps/web/shell/ProductApp.tsx:196-198, 209`.

### ★2.2 The default right-panel tab is the least informative one

`defaultWorkPanelTabs()` returns `status` (`inspector/work-panel-tabs.ts:47-49`), and the `status` tab unconditionally renders the export panel (`inspector/RunInspector.tsx:456-458`) plus this run's usage, cost, session totals, timeline, plan, and tools (`:494-666`).

Measured content during an ordinary Q&A run: export conversation (sanitized) + format picker + download, `in progress…`, `0 tokens`, cost `unavailable`, `session total 0 tokens`, `timeline`, `plan ✓ answer the request`, `tools waiting for tool calls...` — near-zero useful information in the default-visible spot.

### ★2.3 First launch is a form — and there are two different ones

- Main area `apps/web/sidebar/EmptyState.tsx:84-168`: native picker button (`:91-99`) + absolute-path input (`:101-116`) + folder/repo `<select>` (`:117-130`) + open workspace (`:136-137`) + configure model service (`:138-143`) + recently opened (`:146-166`).
- The sidebar `+` has a second similar form: `apps/web/sidebar/WorkspaceTree.tsx:995-1133`, with a different feature set.
- `folder | repo` is an internal storage distinction the user has no basis for choosing between, yet it sits on the first screen they see.

### ★2.4 The same fact is broadcast in five places

| Location | Content | Source |
| --- | --- | --- |
| Top bar | "API connected" (busy only shifts the tone; copy unchanged) | `shell/ProductApp.tsx:1364-1366` + `:416-421` |
| Sidebar session row | "running" badge + spinner | `sidebar/WorkspaceTree.tsx:854-872` |
| Transcript message header | "replying…" | `chat/Transcript.tsx:1286` |
| Composer small text | "replying…" | `chat/Composer.tsx:725` |
| Right panel | "in progress…" / "running" | `inspector/RunInspector.tsx:474, 770` |

Mechanically these come from three unowned sources: the event stream's `runState.busy` (`lib/rove-state.ts:144`), the catalog polling/SSE `session.status` (`state/product-types.ts:22,36`, cadence in `state/product-event-stream.ts:31-48`), and `connection` inferred from request results. With no single owner, divergence is only a matter of time.

### ★2.5 "Send" and "Stop" are both clickable while running

`chat/Composer.tsx`'s `canSubmit` (`:279-287`) checks draft, submitting, disabled, paused, and unfinished attachments — **but not `busy`**. So while running, send (`:922-925`) and stop (`:927`) are both clickable as long as the input has text; pressing send silently queues (`state/use-session-continuity.ts:1099-1103, 1150-1152`), and the user cannot tell from the UI whether the press meant "queue" or "interrupt".

### ★2.6 The send key runs opposite to mainstream convention, and the hint lives only in the placeholder

`chat/Composer.tsx:664-675`: Enter inserts a newline; `Ctrl/Cmd+Enter` sends. The only hint is placeholder copy (`copy/zh-CN.ts:160`, used at `:854`), which disappears once typing starts; the same string is also used as the `aria-label` for both form and textarea (`:684`, `:839`).

### ★2.7 Copy contradicts itself

The sidebar renders "No workspace registered. Add an absolute path from the sidebar." at `sidebar/WorkspaceTree.tsx:391-392` (`copy/zh-CN.ts:797`), while **at the same moment** the main area renders the picker form (`shell/ProductApp.tsx:1575-1581`). Two instructions, two interfaces — and the sidebar has no such button.

### ★2.8 The provider settings page crams three intents on one screen, and the only credential field hides behind a fold

`settings/SettingsShell.tsx` browser branch: ① current selection (`:763-829`) ② create/edit form (`:831-914`), whose "advanced options" contains only "key env var name" (`:872-885`) ③ saved profile list (`:916-966`, each row use/edit/delete + inline confirm). Three different intents spread flat across one screen. `tests/e2e/settings.spec.ts:67-71` even asserts the credential field is hidden by default.

### 2.9 Low-frequency export permanently occupies the right panel

`inspector/RunInspector.tsx:458` unconditionally renders the export panel (`inspector/ExportPanel.tsx:30-52`) in the default tab. `tests/e2e/settings.spec.ts:412` pins this in place.

### 2.10 Session titles truncate with no plain-text fallback

`sidebar/WorkspaceTree.tsx:846-847` puts the title in a `overflow:hidden; text-overflow:ellipsis; white-space:nowrap` container (`styles/product-v2.css:586-591`); the row's `title` (`:830`) and `aria-label` (`:840`) carry lineage + status (`:965-979`), and hovering swaps in a rich card, not the full title.

### 2.11 Unwired switches and hardcoded strings in the UI

- The skin selector offers "cool", but `styles/v3/tokens.css` only defines `.product-app-frame[data-skin="warm"]` (`:11` light, `:139` dark); there is no `data-skin="cool"` token block anywhere (`shell/ui-skin.tsx`, `settings/SettingsShell.tsx:290`, `shell/ProductApp.tsx:158`). Switching to it changes nothing — "looks clickable but isn't wired" damages trust more than not having the switch.
- Hardcoded strings bypassing the copy files: `shell/TopBar.tsx:58`, `sidebar/EmptyState.tsx:111`, `settings/SettingsShell.tsx:560`.
- Inline styles bypassing tokens: `sidebar/EmptyState.tsx:89-90`, `settings/CatalogSettings.tsx:177,336,487`, `settings/MemorySettings.tsx:726-1040`, `sidebar/SessionHoverCard.tsx:85`.
- `DESIGN.md:31` says "left rail fixed at 240px" while the v2 token is 248px (`styles/product-v2.css:117`).

### 2.12 Other

- Finished jobs are repeatedly polled via `GET /api/jobs/<id>/state` and return 404 (observed in request logs) — a state-convergence problem.
- First-launch empty state competes with an empty sidebar block: the main area is the entry point, yet the sidebar still renders an empty "known workspaces" section.

---

## 3. Target information architecture

**Master rule: three columns = three tenses**, all reachable on screen.

```
┌──────────┬────────────────────────────────┬──────────────┐
│ left 240 │  main  ≥450px                  │  right 360   │
│          │                                │              │
│ queue    │  run stream (what's happening) │ evidence     │
│ sessions │  ├ turns                       │ ├ changes    │
│          │  ├ tool rows (with durations)  │ ├ files      │
│ settings │  ├ approval cards (inline)     │ ├ review     │
│          │  └ receipt: steps·time·tokens  │ └ pending    │
│          │                                │              │
│          │  [input]  [send/stop, one slot]│ (closed by   │
└──────────┴────────────────────────────────┴  default)    ┘
```

### 3.1 Left rail (future + past)

- Keep the current structure (workspace groups + session rows + search + settings at bottom), plus two additions:
  1. **Queue visibility**: the server-side queue is already persisted per session (`apps/product-store/src/contracts.rs:489` `ProductQueueResponse`, `:375-381` `queue_order`, `:501` `MAX_PENDING_MESSAGES_PER_SESSION = 64`), so a "queued N" slot on the session row suffices — no new contract needed.
  2. **Status slot must not rely on color alone**: in-progress / done / failed / needs-attention differ by shape, prioritized in-progress → selected → terminal. The existing `needs_attention` count (`sidebar/WorkspaceTree.tsx:401-409`) folds into this slot.
- Delete the contradictory copy from §2.7.

### 3.2 Main column (present) — the run stream

Keep the existing RunSections, activity groups, approval cards, queue rows, minimap, reading-width handle, 450px floor and 840px reading band (`styles/product-v2.css:120-121`); rework tool rows per §4 and **move the receipt line to the end of the run stream** (§5.4).

### 3.3 Right panel (the evidence surface of past) — closed by default

- **Closed by default**; open/closed state persists together with width.
- **Only the user can open it**: the "details" toggle in the main-column header, a new keyboard shortcut, and clicking an artifact/review card in the transcript opening the matching tab. **No agent action may auto-expand the panel** — things that need attention (approvals, failures) belong in the stream, not in a pushed-open panel.
- The tab↔tense mapping stays; only the default tab moves off `status`:
  - `pending` → future (approvals, questions for the human)
  - `status` → present **details** (the main column's run stream is the primary surface)
  - `files` / `changes` / `review` → past (artifacts and evidence, `inspector/work-panel-tabs.ts:13-21`)
- **No placeholder when empty**: with no content, the panel body is not rendered (the 40px collapsed rail stays).
- Export conversation moves from permanent residency to the session's overflow menu (§2.9).

### 3.4 Width budget

Keep the current policy: 450px hard floor on the middle column (`inspector/work-panel-layout.ts:27`, `styles/product-v2.css:4022`), and the sidebar yields first when the budget is short (`work-panel-layout.ts:77-121`). Optional addition: drags anchor to the pointer-down position, and `Esc` restores the pre-drag width.

---

## 4. Run stream spec

### 4.1 Turn and row structure

One run = one turn block (existing `chat/Transcript.tsx:770-789`). Inside a turn:

- **One "full process" disclosure**: thinking, tools, retrieval, and intermediate progress all live inside it; the final answer sits outside. The in-progress disclosure is expanded and history is collapsed by default; **a disclosure the user expanded must never auto-collapse on streaming updates or completion** — that is a trust issue, not a preference.
- **Tool rows** must read well while collapsed: `[action icon] specific label  [duration] [chevron]`
  - Labels must **name the object**: `Read orders.csv (2,481 rows)`, never category labels like `tool call ×7`.
  - **Every step shows its duration** — the only information that distinguishes "slow" from "stuck".
  - A 1px vertical line connects the icon column between rows, so the stream reads as a process rather than a list of notifications.
  - Icons distinguish types by **shape** (thinking / tool / write / network / waiting-for-you / output), not color alone.
- **Expand = evidence**: result first, inputs after; monospace with `tabular-nums`; each block independently copyable; 220–260px in-block scrolling. Do not render tool payloads as prose — evidence is valuable because it is verbatim.
- **Failures expand by default**: a failure that takes a click to see is a hidden failure.
- **Approval cards inline in the stream**, never modal: `what it does` / `what it affects` (including an irreversibility note) / `why` (links back to the step that produced it) / three actions (approve / modify / reject). The existing `ApprovalCard` (`chat/Transcript.tsx:1798`) is the base; missing pieces are the "why" back-link and "modify".
- **Timeout policy printed on the card**: what happens if nobody answers. Auto-approve is a "planned bug"; auto-cancel is acceptable but must be stated.

### 4.2 Streaming text

- Appends chunk by chunk only inside a step's output block, 1 chunk/frame, with a block cursor.
- **Must be skippable**: click or `Esc` jumps straight to the final text.
- Streaming text must not cause composer relayout.

### 4.3 Only one persistent animation per page

It must stop when no run is active. This directly constrains §5.1.

---

## 5. State, feedback, and cost

### 5.1 Single ownership

| Fact | Single source of truth | Where it may render |
| --- | --- | --- |
| Whether this turn is running | event stream `runState.busy` | the transcript tail's **only** run line + the composer's slot state |
| Session list status | catalog `session.status` | only the left-rail session row status slot (shape + motion, no duplicated text) |
| Connection health | `connection` | only in the top bar **when abnormal**; silent when healthy (today it permanently says "API connected") |

- The run line must **say what it is waiting for**. `chat/activity-phase.ts` and the composer phase line (`chat/Composer.tsx:719-723`) already exist — promote it into that single run line and extend the phase vocabulary: waiting for model / retry n (with delay) / preparing / compacting context (with reason) / recovering / waiting for subtask.
- **No percentages, no second progress card.**

### 5.2 Send and stop share one slot

- `Send` when idle, `Stop` while running (≥44px, never in a menu, repaints within a frame).
- Submission while running stays possible, but the semantics must be explicit: **queue** (default; sent after this turn) vs **interject** (interrupts this turn). The server supports both already (`chat/Transcript.tsx:1008-1021`, `copy/zh-CN.ts:247-250`); only the entry points are buried in transcript queue rows. Recommended: promote to the composer — `Enter` = queue, `Alt+Enter` = interject.
- The queue rows themselves stay above the composer (server-persisted FIFO, survives restarts).

### 5.3 Send key

- Default: **Enter sends, `Shift+Enter` newlines**; provide a preference; when Enter-send is off, `Ctrl/Cmd+Enter` takes over.
- The hint must not live only in the placeholder: keep a persistent shortcut hint or `?` entry inside the composer.

### 5.4 Cost

- The **receipt line** sits at the end of the run stream: `9 steps · 42.6s · 18.2k tokens`, monospace, `tabular-nums`, quiet. Not a badge, not a chart, not a permanent panel.
- Per-step tokens appear only inside **expanded rows**; collapsed rows show duration only.
- Context pressure is real state, not hidden truncation: when earlier context is actually dropped, it is written into the stream as a step. The existing `chat/CompactionPanel.tsx` just connects to the stream.
- Model identity is run metadata: shown once in the turn header, per step only when a turn mixes models. The model selector should not occupy a permanent prime spot.
- A budget bar is built only when a real budget concept exists — there is no such contract today, so this document does not do it.

### 5.5 Where errors go

- Bound to a message/tool → **inline** on that row (with "retry from this step").
- Result of an explicit action → toast.
- Persistent conditions (backend unavailable, migration) → inline banner.
- A problem already shown in the page is never re-announced by toast (already a rule in `DESIGN.md:45`).

---

## 6. First launch and model setup

### 6.1 Empty state: from form to guidance

```
        Welcome to rove
  Pick a folder to get started

    [ Open folder ]        ← primary CTA (native picker, existing endpoint)

  Recently opened
  · D:\projects\my-app
```

- **Remove**: the absolute-path input (demoted to a collapsed "enter manually"), the folder/repo selector (remove it, or keep it collapsed defaulting to "folder" with a one-line explanation).
- **Remove** the contradictory sidebar copy; the sidebar empty state either does not render or says something that does not point elsewhere.
- Keep "recently opened" (`sidebar/EmptyState.tsx:146-166`).
- The sidebar `+` and the main-area empty state **share one selection flow**; two differently shaped forms are no longer maintained.

### 6.2 No model configured: don't block, but block sending

- The composer accepts input; `Send` is disabled with a "configure a model first" hint.
- An actual submit returns an explicit inline message + a direct action to configuration.
- Record an open question: whether starting without a workspace should be possible (today a workspace must be registered before a session can start).

### 6.3 Inline first-run checklist (optional)

A dismissible checklist in the empty state where each row is a **deep-link button**, not descriptive text: `add a model service` → `fill in the key` → `open a folder` → `send the first message`. Completed rows get a strikethrough check; the whole list is closable. **No blocking wizard.**

### 6.4 Model setup: three blocks on one screen → three layers

| Layer | Content | Form |
| --- | --- | --- |
| Current | mode (default / specific service) + current service + current model | one card, mostly read-only |
| Added services | per row: name, type, base URL, **credential status** (keyring / env var / file / none), test, set default, edit, delete (two-step confirm); validation status replaces the description line | list card |
| Add / edit service | overlay. Step 1 asks only the service preset (OpenAI / OpenAI Responses / Anthropic / Ollama / custom); step 2 fills name, base URL, **API key**, and wire format, with the rest folded under "advanced"; then model selection | overlay |

### 6.5 Entering keys in the browser (the sole contract dependency)

Today: the create/update requests in `apps/product-store/src/contracts.rs:1693-1719` carry only `label, provider_type, api_base, api_key_env: Option<String>, default_model, expected_revision` and are `#[serde(deny_unknown_fields)]` — **there is no raw-key field and none can be smuggled in**. A raw key can currently only enter via the Desktop host's `provider_credential_prompt` (`apps/web/platform/desktop-commands.ts:222-275`), and `apps/web/AGENTS.md:30` makes "keys never enter browser state/localStorage/request bodies" a hard invariant.

For "enter a key in the browser → write to the system credential store" to work:

1. **A new product contract** (e.g. `PUT /product/provider-profiles/{id}/credential`) whose body accepts only the key and whose response returns only `credential_source`. It must state: **loopback only**; **never enters logs, traces, reports, API responses, screenshots, or fixtures**; **never echoed back**; failures must not leak the provider's raw error.
2. Frontend: the key field exists only inside the overlay and is cleared from DOM and state immediately on submit — never in `localStorage`, never in browser state.
3. Update `docs/decisions.md`, `docs/api.md`, and the OpenAPI snapshot together.

---

## 7. Implementation mapping

### 7.1 Category A: structure + component behavior

| # | Change | Main touch points |
| --- | --- | --- |
| A1 | Right panel closed by default + persisted (same for left rail) | `inspector/use-work-panel.ts`, `shell/ProductApp.tsx:196-209` |
| A2 | Default tab no longer `status`; no placeholder when empty | `inspector/work-panel-tabs.ts:47-49`, `inspector/RunInspector.tsx` |
| A3 | State converges to single ownership (§5.1) | `shell/ProductApp.tsx`, `shell/TopBar.tsx`, `sidebar/WorkspaceTree.tsx`, `chat/Composer.tsx`, `inspector/RunInspector.tsx` |
| A4 | Send/stop shared slot + queue/interject semantics (§5.2) | `chat/Composer.tsx:279-287, 922-927` |
| A5 | Send key defaults to Enter + preference + persistent hint (§5.3) | `chat/Composer.tsx:664-675, 854`, `copy/zh-CN.ts:160` |
| A6 | De-form the empty state; unify the two selection flows (§6.1) | `sidebar/EmptyState.tsx`, `sidebar/WorkspaceTree.tsx:995-1133` |
| A7 | Contradictory sidebar copy (§2.7) | `sidebar/WorkspaceTree.tsx:391-392`, `copy/zh-CN.ts:797` |
| A8 | Model setup split into three layers (§6.4) | `settings/SettingsShell.tsx:763-966` |
| A9 | In-browser credential entry (§6.5) | new product contract + frontend |
| A10 | Receipt line to end of run stream; export into session menu | `inspector/RunInspector.tsx:456-558`, `inspector/ExportPanel.tsx` |
| A11 | Tool rows/disclosures/failures expanded by default (§4.1) | `chat/Transcript.tsx` activity groups and tool rows |
| A12 | Queue visible on left-rail session rows (§3.1) | `sidebar/WorkspaceTree.tsx` |

### 7.2 Category B: tokens / copy / consistency

| # | Change | Touch points |
| --- | --- | --- |
| B1 | Decide the "cool" skin: implement it or remove it from the selector (§2.11) | `styles/v3/tokens.css`, `shell/ui-skin.tsx`, `settings/SettingsShell.tsx:290` |
| B2 | Hardcoded UI strings move into copy files | `shell/TopBar.tsx:58`, `sidebar/EmptyState.tsx:111`, `settings/SettingsShell.tsx:560` |
| B3 | Inline styles return to tokens | `sidebar/EmptyState.tsx:89-90`, `settings/CatalogSettings.tsx`, `settings/MemorySettings.tsx`, `sidebar/SessionHoverCard.tsx` |
| B4 | Fix the 240 vs 248 inconsistency in `DESIGN.md:31` | `DESIGN.md` |
| B5 | Semantic z-index ladder + elevation via alpha gradients of the same hue instead of separate shadow colors (new tokens; write into `DESIGN.md` first) | `styles/v3/tokens.css`, `DESIGN.md` |

### 7.3 Tests that will be affected (must be updated in sync; never delete tests)

| Test | Current assertion | Impact |
| --- | --- | --- |
| `tests/e2e/shell.spec.ts:80-87` | right panel **expanded** on load | must change for A1 |
| `tests/e2e/shell.spec.ts:13,170-171` | first-launch copy "open a workspace to start", "absolute path", "open workspace" | changes for A6 |
| `tests/e2e/settings.spec.ts:412` | "export conversation (sanitized)" visible on the session row | changes for A10 |
| `tests/e2e/settings.spec.ts:67-71` | key field hidden behind "advanced options" | changes for A8/A9 |
| `tests/e2e/layout.spec.ts:312-391` | right panel is a dynamic closable tab strip | **keep**; A2 only changes the default |
| `tests/e2e/layout.spec.ts:14,19` | top bar 52px, middle column floor 450px | **keep** |
| `tests/e2e/reading-width.spec.ts` | 840/808 reading band, no handle at 960/375 | **keep** |
| `tests/e2e/accessibility.spec.ts:199-208` | panel collapses via `complementary[name="details"]` | **keep** (path must still work after A1) |
| `tests/e2e/polish.spec.ts:40-94` | 375px panel `data-collapsed`, drawer/focus trap/inert | **keep** |
| `apps/web/product/production-ui-boundary.test.ts:20-87` | production code may not import design mocks; pins several copy keys | **keep**, sync when copy changes |

---

## 8. Phases and acceptance

- **P0**: A1 panel closed by default + persisted · A3 state convergence · A4 send/stop shared slot · A7 sidebar copy · A5 send key default.
  Acceptance: `pnpm test && pnpm typecheck`; updated e2e passes; manual walkthrough "load → send one → running → finished → refresh" × (light/dark) × (existing skins); confirm running state speaks in exactly one place.
- **P1**: A6 de-formed empty state · A2 default tab and no empty placeholder · A10 receipt and export relocation · A12 queue visibility.
- **P2**: A11 run stream spec · A8/A9 model setup layers and in-browser keys · §6.3 first-run checklist.
- **P3**: B1–B5.

If a round only does category B: B1, B2, and B4 immediately reduce "looks clickable but unwired" and "mixed-language" noise, but none of the four questions in §1 gets answered.
