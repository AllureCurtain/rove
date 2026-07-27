# Rove Product UI V2

> Status: **Proposed / Not Implemented**
>
> Date: 2026-07-27
>
> Product register: local-first coding-agent control center
>
> Design Read: calm, forensic, reliable and decisive; `SOUL=7`,
> `SPECTACLE=2`, `DENSITY=8`; no hero engine; feedback-only motion
>
> Visual prototype: `apps/web/app/dev/product-ui-v2/`

This document proposes the next shared product UI for the Web and future Tauri
Desktop hosts. It does not describe behavior already available from the default
product shell. The isolated `/dev/product-ui-v2` route is a reviewable mock. It
is intentionally packaged as a local developer preview, following the existing
`/dev/workbench` pattern, and carries `noindex` metadata. It is not the production
Product UI: it is inert, does not call the API or persist state, and every
approval, tool, provider, Memory, session, and Desktop interaction is simulated.

Current runtime truth remains under [`../runtime/`](../runtime/README.md). The
implemented shared-shell baseline remains
[`2026-07-25-agent-desktop-web-ui-design.md`](2026-07-25-agent-desktop-web-ui-design.md).

## 1. Decision

Rove will use one product UI in two hosts:

```text
                        Rove Product UI
                 routes, views, state, components
                              |
                  host-neutral product transport
                   /                         \
          Next.js Web host              Tauri Desktop host
          Web platform adapter          Desktop platform adapter
                   \                         /
                  shared rove-api and runtime
```

The UI is an operational tool. It is not a landing page, generic chat client,
dashboard gallery, or private event-log viewer. Its primary job is to let a
developer send work, observe progress, intervene at a safe boundary, and verify
what the Agent actually did.

The lazy default for this category is a dark slate shell with purple glow,
oversized rounded chat bubbles, and a terminal-like activity feed. Rove rejects
that default. It also rejects the second-order Linear clone and phosphor-terminal
response. The proposed identity is **Ice Steel Instrument**: ice-grey surfaces, a
controlled steel-cyan execution signal, compact typography, exact alignment,
and a visible evidence spine.

The steel-cyan signal is not semantic status. It means selected, focused, or
currently executing. Success, warning, and failure retain separate semantic
colors and always include an icon or label.

## 2. Audience And Jobs

### 2.1 Primary audience

- Developers running Rove against one or more local repositories.
- Maintainers who need long-lived sessions, repeatable resume, and evidence of
  tool effects.
- Operators who frequently switch between sessions while other runs continue.
- Future Desktop users who expect native workspace selection without learning a
  second interface.

### 2.2 Core jobs

1. Choose an exact workspace root and resume the correct product session.
2. Give the Agent a task and understand whether it is idle, running, blocked,
   canceled, failed, or complete.
3. Read assistant prose, tool activity, input requests, and approvals in the
   order they occurred.
4. Steer an active run at a safe boundary or queue the next instruction.
5. Inspect the continuity chain and canonical evidence without turning the main
   transcript into a debug console.
6. Manage providers, approval defaults, workspace memory, sessions, and host
   capabilities without exposing secrets to browser state.

### 2.3 Non-goals

- A marketing surface or onboarding tour.
- A metrics dashboard as the product home.
- A separate Desktop business UI.
- A private Web-only event lifecycle.
- Hosted multi-user identity or a remote control plane.
- Unbounded tool permissions or trust inferred from generated text.
- Claiming Markdown, queueing, export, file diff, or Desktop behavior before
  implementation and tests exist.

## 3. Current As-Built Design Model

This section records the current code rather than the desired result. It is a
design audit, not a replacement for runtime documentation.

### 3.1 Existing model

| Layer | Current source evidence | As-built value |
|---|---|---|
| Register | [`apps/web/styles/product.css`](../../apps/web/styles/product.css) | Product shell with fixed top bar, workspace rail, chat, and Inspector |
| Fonts | [`apps/web/app/layout.tsx`](../../apps/web/app/layout.tsx) | Geist Sans and Geist Mono, with system fallbacks |
| Light substrate | [`apps/web/styles/tokens.css`](../../apps/web/styles/tokens.css) | Sage-grey `#f4f5f3` background and near-white raised surfaces |
| Dark substrate | same token file | Green-tinted graphite `#141816` with lighter forward surfaces |
| Accent | same token file | Desaturated harbor blue `#3a5f7a`, lifted to `#7ea3bd` in dark mode |
| Semantics | same token file | Green success, ochre warning, red error |
| Radius | same token file | `8 / 12 / 16 / 999px` |
| Shell | same stylesheet | `280px / fluid / 320px`, `52px` top bar |
| Motion | same stylesheet | Short state transitions and reduced-motion handling |
| Product state | [`apps/web/shell/ProductApp.tsx`](../../apps/web/shell/ProductApp.tsx) | API-authoritative catalog, exact session turns, restore and reattachment |

The current substrate is materially stronger than a generic admin template. It
already has tinted light and dark neutrals, low-alpha borders, fixed product
type, a compact shell, semantic status colors, focus visibility, and explicit
reduced-motion treatment. Product UI V2 should preserve those strengths.

### 3.2 As-built drift and gaps

| Finding | Evidence | V2 response |
|---|---|---|
| The harbor-blue accent is competent but category-generic | `tokens.css` | Replace only after visual approval with a non-semantic steel-cyan execution signal |
| `999px` is the global default for buttons | `product.css` button rule | Use `3 / 5 / 8px`; reserve pills for status and compact segmentation |
| Transcript chronology is lost | [`Transcript.tsx`](../../apps/web/chat/Transcript.tsx) renders all messages, then waiting tools, inputs, and terminal tools | Project one ordered timeline before presentation |
| Messages default to rounded bubbles | `chat-bubble` rules | Use document-like assistant turns and a restrained user prompt band |
| Tool calls and results are isolated cards | `ToolCard` | Group invocation, result, scope, and mutation evidence on one timeline node |
| Composer is disabled for an active session | `ProductApp.tsx` active-session guard | Add explicit Steer and Follow-up modes after runtime queue contracts exist |
| Inspector is a list of current run fields | [`RunInspector.tsx`](../../apps/web/inspector/RunInspector.tsx) | Make continuity and canonical evidence the first two sections |
| The advanced workbench has a separate legacy visual layer | `app/dev/workbench/layout.tsx` imports `app/globals.css` | Keep it bounded and do not use it as Product UI V2 design truth |

Changing the accent, radius system, or information architecture is intentionally
proposed here rather than silently editing the implemented C0-C3 baseline.

## 4. Pi Web Relationship

Capability reference:
[`agegr/pi-web`](https://github.com/agegr/pi-web/tree/313727a3906a8553b0702dc79517a187d332a9f8).
Pi Web is a behavioral benchmark, not a visual template or architecture source
of truth. The referenced repository is MIT-licensed. This prototype copies no
source code or visual assets. Any future source-level reuse must retain the
required license notice and be reviewed separately.

### 4.1 Adopt

| Capability | Why it belongs in Rove |
|---|---|
| One chronologically ordered transcript | A user must see a tool call where it happened, not after every message |
| Tool invocation and result pairing | Collapsing one unit is faster to scan than correlating separate cards |
| Steer and follow-up queue | Active work should not make the composer unusable |
| Markdown, code, diff, and image-aware rendering | Coding work needs inspectable output, not plain text only |
| Fast model and reasoning controls | Common changes belong near the composer, while full setup stays in Settings |
| Complete session export | A useful export contains conversation and evidence, not catalog metadata only |
| Fork and session tree concepts | Branching a useful conversation is a normal coding-agent workflow |
| Context, usage, and cost visibility | These explain why an Agent compacts, slows down, or stops |

### 4.2 Adapt

| Pi Web behavior | Rove adaptation |
|---|---|
| In-process active session registry | Durable server-owned product-session and runtime bindings remain authoritative |
| SSE plus client reconciliation | Reconcile against exact job/run identity and reject stale events |
| Rich tool presentation | Render only canonical tool facts and preserve Rove approval state |
| Run-time message queue | Apply steer only at a typed safe boundary; persist follow-up intent before acknowledging it |
| Model controls | Refer to server profiles and aliases; never expose raw keys to browser state |
| File and diff UI | Resolve all paths against the exact workspace; remote names never become local paths |
| HTML export | Include provenance, partial-history reasons, and explicit redaction metadata |
| Session fork | Fork product and runtime identity together; never create a transcript-only cosmetic branch |

### 4.3 Reject

- Visual imitation of Pi Web. Rove needs its own information hierarchy and
  evidence language.
- Process memory as durable truth.
- Local-machine trust as permission. Every tool continues through the shared
  registry and approval path.
- An unbounded file browser, shell, or workspace path.
- Optimistic cancellation that detaches observation before cancellation is
  confirmed or typed as uncertain.
- Exporting raw provider keys, secrets, or unredacted tool material.
- Rebuilding Pi Web components line-for-line when Rove's canonical event and
  persistence contracts require a different data model.

## 5. Visual System

### 5.1 Identity

The product should feel like a well-made ice-steel instrument used for repeated
technical work:

- **Calm:** low-chroma surfaces, quiet separators, no ornamental gradients.
- **Forensic:** event names, scope, mutation, and continuity are aligned and
  inspectable.
- **Reliable:** active and uncertain states never look complete.
- **Decisive:** one execution signal, one primary action per local context.

The signature is the **Rove Trace Rail**, not a logo animation or decorative
hero. It is useful at rest and during active execution.

### 5.2 Target color roles

Values below are the proposed prototype values. They are not production tokens
until visual review and contrast verification are accepted.

| Role | Light | Dark | Purpose |
|---|---|---|---|
| Canvas | `#e7edf1` | `#080d10` | Outer host field |
| Background | `#eff3f5` | `#0f1519` | Inspector and secondary regions |
| Surface | `#f8fafb` | `#151d22` | Main transcript and sections |
| Raised surface | `#fcfdfd` | `#1d272d` | Toolbar, composer, framed tool/approval units |
| Sunken surface | `#e6ecef` | `#0c1216` | User prompt band, code, segmented controls |
| Hover surface | `#dde5ea` | `#263139` | Hover and selected neutral context |
| Primary ink | `#111a20` | `#edf3f5` | Headings and key values |
| Secondary ink | `#34444e` | `#bdc9ce` | Body text |
| Tertiary ink | `#62737e` | `#8f9ea6` | Metadata that still requires AA contrast |
| Border | `rgba(17,26,32,.10)` | `rgba(237,243,245,.10)` | Hairline separation |
| Execution signal | `#0d789f` | `#3fc5e8` | Active, selected, current trace node |
| Strong signal | `#09698e` | `#69d8f2` | Emphasized active and focus treatment |
| Signal soft | `rgba(13,120,159,.12)` | `rgba(63,197,232,.14)` | Selection and focus context |
| On signal | `#f8fafb` | `#071216` | Text and icons on signal fills |
| Success | `#2e7752` | `#63c795` | Completed state only |
| Warning | `#97620f` | `#e0a34a` | Approval and uncertain state |
| Failure | `#a7443c` | `#ed8179` | Failed, destructive, rejected |
| Rail | `#131b20` | `#090e11` | Workspace and Settings navigation |
| Rail muted ink | `#9cabb4` | `#8f9da5` | Navigation metadata |
| Rail signal | `#3fc5e8` | `#3fc5e8` | Active item in the dark rail |

Rules:

- No pure white or black.
- Raw colors live only in the theme token definitions.
- The execution signal is never reused for success.
- Semantic state always has text or an icon in addition to color.
- Light and dark are independently authored themes, not inversion.
- No gradient, glow, glass, decorative orb, or one-note hue wash.

### 5.3 Typography

- UI: Geist Sans because it is already installed, highly legible at compact
  product sizes, and avoids a new dependency.
- Code and identity: Geist Mono, restricted to paths, event names, commands,
  timestamps, and tabular identifiers.
- Fixed scale: `10 / 11 / 12 / 13 / 14 / 15 / 18 / 22px`.
- Letter spacing is `0` throughout.
- Headings use `text-wrap: balance`; multi-line prose uses `text-wrap: pretty`.
- Product UI does not scale type with viewport width.
- CJK localization uses the existing system fallback until a tested CJK font
  strategy is approved.

### 5.4 Geometry and elevation

| Token | Value | Use |
|---|---|---|
| Radius XS | `3px` | status tags, code labels |
| Radius SM | `5px` | controls and icon buttons |
| Radius MD | `8px` | tool groups, approvals, composer, repeated rows |
| Space base | `4px` | all spacing resolves to the 4px grid |
| Whisper shadow | `0 1px 3px` with tinted alpha | raised local units only |
| Panel shadow | tight negative-spread shadow | drawers, bottom sheets, composer |

No page section is styled as a floating card. Cards are reserved for repeated
records and genuinely framed interaction units. A tool result may contain a
code inset, but it must not contain another decorative card.

### 5.5 Motion

- State transitions: `140ms` for color, border, and opacity.
- Drawer and sheet movement: `180ms cubic-bezier(.16,1,.3,1)`.
- Active-run pulse: `1.8s`, opacity and transform only.
- No entrance choreography, bounce, elastic easing, parallax, marquee, or hero
  engine.
- `prefers-reduced-motion: reduce` removes all pulses and drawer interpolation,
  leaving the final readable state.

## 6. Application Shell

### 6.1 Desktop

```text
52px Product Bar
┌───────────────248px──────────────┬────────fluid────────┬────304px────┐
│ Workspace and Session Rail       │ Conversation        │ Evidence    │
│ exact roots, parallel status     │ ordered timeline    │ continuity  │
│ bounded workspace assurance      │ active composer     │ plan/events │
└──────────────────────────────────┴─────────────────────┴─────────────┘
```

- The Product Bar owns host-level navigation, preview mode, runtime connection,
  and theme.
- The workspace rail owns Workspace and Session only. Run details do not create
  a fourth navigation level.
- The central transcript remains the primary surface.
- The Inspector is evidence-oriented and deliberately narrower than the chat.

### 6.2 Settings

Settings is a full product view. It replaces the workspace rail with a compact
section navigation and uses an unframed, width-capped management column. The
same Product Bar remains visible so Web and Desktop do not diverge.

The target sections remain:

1. General and appearance.
2. Providers and models.
3. Tools and approvals.
4. Workspace and paths.
5. Memory.
6. Sessions and export.
7. Keyboard shortcuts.
8. Advanced and developer.
9. About and runtime.
10. Desktop host capabilities, rendered only when the host advertises them.

The prototype demonstrates provider, approval, Memory, session, and Desktop
management states. It is representative, not complete.

### 6.3 Responsive behavior

| Width | Behavior |
|---|---|
| `>1180px` | `248px / fluid / 304px` triptych |
| `961-1180px` | Rail and Inspector narrow; timeline metadata compresses |
| `761-960px` | Workspace rail plus chat; Inspector becomes a right drawer |
| `<=760px` | Chat only; workspace is a left drawer; Inspector is a bottom sheet |

On small screens:

- Chat and Settings remain reachable from the Product Bar.
- Workspace navigation never stacks above the entire chat.
- Settings navigation is a labeled horizontal scroller with stable 44px targets.
- Timeline metadata moves above content while its trace spine stays visible.
- Composer becomes exactly two control rows: context/model, then
  steer/follow-up/stop/send.
- All touch targets are at least 44px.
- No horizontal document overflow is permitted at `390px` or `375px`.

## 7. Rove Trace Rail

The Trace Rail is a product expression of the canonical lifecycle, not a second
event system.

### 7.1 Anatomy

```text
09:42  workspace.resolved   ●  Workspace boundary resolved
                              │
09:42  tool.completed       ●  Read current Web contracts
                              │  invocation + result + scope + mutation
09:43  approval.requested   ●  Approval required
                              │  consequence + exact boundary + actions
now    assistant.delta      ●  Assistant prose continues
```

- Metadata column: timestamp plus canonical event name for operational nodes.
- Spine: one continuous low-contrast line for the current run group.
- Node: neutral message, green completed, ochre approval/uncertain, steel-cyan
  active, red failed.
- Content: assistant prose, tool group, approval, input request, diff summary, or
  state transition.

### 7.2 Projection contract

The renderer consumes an ordered projection with stable identity:

```text
TimelineItem {
  item_id
  product_session_id
  job_id
  run_id
  run_ordinal
  event_ordinal
  occurred_at
  kind
  status
  correlation_id?
  canonical_event_ref?
  content
}
```

This shape is a target design, not an implemented API type. The implementation
must derive it from canonical events and persisted product bindings. It must not
create a browser-private lifecycle or infer missing facts as success.

Rules:

- Tool invocation and result correlate by canonical tool-call identity.
- Approval and input requests stay in their true chronological position.
- Assistant deltas coalesce into one visible turn without changing event order.
- Run boundaries remain visible when one Session contains several Runs.
- Historical partial reasons are presented before the affected range.
- Inspector and transcript select the same item by stable identity.
- Virtualization may remove offscreen DOM, but never reorder items or lose the
  selected anchor.

## 8. Interaction Model

### 8.1 Session focus and restore

```text
select product session
  -> read exact server binding
  -> fetch canonical transcript projection
  -> render complete, partial, or typed error
  -> if an exact live job exists, attach observation
  -> reconcile status without submitting another turn
```

An unknown session, partial projection, ambiguous start response, or failed
attachment never becomes an empty successful conversation.

### 8.2 Active composer

The composer remains usable during a run through two explicit modes:

- **Steer:** request delivery at the next runtime-declared safe boundary.
- **Follow-up:** persist an instruction for the next turn after the current run
  reaches a terminal state.

The UI must not acknowledge either mode until the server has durably accepted
the intent. An error leaves the text in the composer and explains whether any
instruction may have been accepted.

The product must define queue order, cancellation interaction, duplicate
submission protection, and resume behavior before production UI is wired.

### 8.3 Approval

An inline approval shows:

- tool or intended operation;
- consequence in plain language;
- exact workspace or external boundary;
- mutation and side-effect classification;
- Approve once and Reject actions;
- pending, accepted, rejected, expired, and uncertain states.

The Inspector mirrors the request but does not create a second decision object.
If approval submission fails, the request stays pending and visible.

### 8.4 Cancellation

```text
Stop pressed
  -> send bounded cancel request
  -> keep observing the current stream
  -> confirmed canceled: close observation after terminal evidence
  -> request failed: show typed error and remain attached
  -> effect uncertain: show uncertain state and reconcile durable status
```

The client must never close SSE merely because the cancel request returned an
error. Unknown external side effects remain unknown until evidence resolves
them.

### 8.5 Tool groups

- Summary line: icon, action, tool name, state, and disclosure.
- Detail: bounded command/input, output or result summary, duration if reported,
  scope, and mutation facts.
- Diff: file list first, then line diff on demand.
- Long output is truncated with an explicit byte/line reason and a durable
  artifact link when available.
- Errors state the cause and a recovery action.

## 9. Component Inventory

| Area | Components |
|---|---|
| Host | ProductBar, HostCapability, ConnectionState, ThemeSwitch |
| Navigation | WorkspaceRail, WorkspaceGroup, SessionRow, SessionStatus, SettingsNav |
| Conversation | OrderedTimeline, TraceNode, UserPrompt, AssistantTurn, RunBoundary |
| Tools | ToolGroup, ToolSummary, ToolResult, DiffSummary, ArtifactLink |
| Intervention | ApprovalBlock, InputRequest, CancelState, QueueReceipt |
| Composer | Composer, ContextAttach, ModelSelect, ReasoningSelect, SteerFollowUpControl |
| Evidence | EvidenceInspector, ContinuityChain, PlanList, CanonicalEventList, UsageFacts |
| Settings | ProviderRow, ApprovalPolicyChoice, WorkspaceScope, MemoryLayer, SessionExport |
| Feedback | Skeleton, EmptyState, PartialNotice, ErrorRecovery, Toast, Confirmation |
| Overlay | WorkspaceDrawer, EvidenceDrawer, EvidenceBottomSheet, AlertDialog |

Use the installed Radix icon library for familiar symbols. Icon-only controls
require an accessible name and tooltip. Text buttons are reserved for commands
whose consequence needs words.

## 10. State Matrix

| Domain | Required states |
|---|---|
| Product boot | loading, ready, typed error, retrying |
| Workspace | none, selected, unavailable, removed, path rejected |
| Session | idle, running, attention, error, canceled, complete |
| Transcript | empty, loading, complete, partial with reasons, failed with retry |
| Stream | connecting, attached, reconnecting, stale-rejected, terminal, uncertain |
| Assistant turn | queued, streaming, complete, interrupted, failed |
| Tool | proposed, approval pending, queued, running, complete, failed, canceled, uncertain |
| Approval | pending, submitting, approved, rejected, expired, submission failed |
| Input | pending, submitting, accepted, expired, submission failed |
| Composer | idle, starting, steer, follow-up, accepted, ambiguous, failed |
| Settings | loading, loaded, dirty, saving, conflict, saved, failed |
| Export | preparing, ready, partially redacted, failed |

Disabled controls must have a visible reason. Loading preserves final geometry.
An empty state states what is absent and the next valid action.

## 11. Accessibility

- Main landmarks, workspace navigation, Settings navigation, transcript log,
  Inspector, drawers, and bottom sheet receive explicit names.
- A skip link reaches the active main surface.
- Keyboard order follows Product Bar, rail, transcript, composer, Inspector.
- New streaming text uses polite live updates. Approval and errors use assertive
  announcement only when action is required.
- Focus moves to a new approval/input request and returns to the prior control
  after resolution.
- Status never relies on color alone.
- Body and control text meet WCAG AA in both themes. Focus rings remain visible
  against every surface.
- Icon controls have 32px desktop geometry and at least 44px touch geometry.
- Motion has a complete reduced-motion terminal state.
- Timeline virtualization must preserve screen-reader ordering and the focused
  item.

## 12. Web And Desktop Parity

| Capability | Shared UI | Web host | Desktop host target |
|---|---|---|---|
| Conversation, trace, approvals | Yes | API proxy plus SSE | Same product transport |
| Workspace selection | Same product model | Validated path input | Native folder picker |
| Runtime lifecycle | Same visible states | External supervised API | Host-managed local API |
| Secrets | Never in product state | Environment references | Secure store behind adapter |
| File open/reveal | Same command intent | Browser-safe fallback | Native reveal/open |
| Notifications | Same notification event | In-product | Optional OS notification |
| Theme | Same tokens | persisted Web preference | persisted host preference |
| Update/tray/window | Host capability only | unavailable | optional Desktop adapter |

Host-only capability is feature-detected. It must not fork routes, state models,
or business components. A missing Desktop capability renders unavailable with a
reason, not a dead control.

## 13. Protected Rove Contracts

Product UI V2 must preserve:

1. Exact `Workspace -> Session -> Run` identity.
2. Canonical stream events as the shared lifecycle contract.
3. `trace.jsonl` as event facts, `task_state.json` as resumable state, and
   `report.json` as derived summary.
4. Fail-closed hard resume and no replay of completed mutations.
5. Workspace-bounded path resolution.
6. Shared `ToolRegistry` safety and approval authority.
7. Distinct authority for memory, retrieved content, tool output, runtime
   policy, and workspace instructions.
8. No secrets in browser state, events, screenshots, exports, logs, or fixtures.
9. Local deterministic execution without a provider key or network.
10. Typed and visible failure rather than optimistic success.

## 14. Performance And Long Sessions

- The shell has fixed tracks so streaming text, status chips, and icons cannot
  resize the application frame.
- Transcript rendering uses a bounded visible window with anchor-preserving
  backfill once long-session fixtures show that full DOM rendering is no longer
  responsive.
- Assistant deltas batch to animation-frame or bounded time slices rather than a
  React render per token.
- Auto-scroll occurs only while the user remains near the bottom. Reading older
  content shows a clear "return to latest" control instead of stealing scroll.
- Tool output and diffs are lazy and bounded.
- Theme switching cannot cause layout shift.
- Browser acceptance must detect blank rendering, document overflow, overlap,
  console errors, and stale stream updates.

## 15. Acceptance And Screenshot Matrix

The preview is design evidence only. Production acceptance later repeats the
same matrix against live API state.

| Surface | Viewport | Theme | State | Required evidence |
|---|---:|---|---|---|
| Chat | `1440x900` | Light | active run, tool result, approval, composer | Three tracks stable; approval actions visible; no overlap |
| Chat | `1440x900` | Dark | same | Independent dark contrast and depth |
| Chat | `390x844` | Light | active run | No horizontal overflow; stable two-row composer |
| Chat | `390x844` | Dark | workspace drawer or Evidence sheet | 44px targets; underlying content inert when open |
| Settings | `1440x900` | Light | provider management | Navigation and management column scan cleanly |
| Settings | `1440x900` | Dark | approval or Memory | No light-theme hardcoded colors |
| Settings | `390x844` | Light and dark | horizontal section navigation | Labels scroll rather than clip |
| Chat | desktop and mobile | either | reduced motion | Active state remains visible with no pulse |
| Chat | desktop | either | keyboard-only | Skip link, timeline actions, approval, composer, Inspector reachable |

Automated visual checks must assert:

- document `scrollWidth == clientWidth` at every target viewport;
- no page or console error;
- Product Bar, rail/main/Inspector tracks share exact non-overlapping bounds;
- drawers and sheets remain within the viewport;
- long workspace, session, model, and event labels truncate without shifting
  controls;
- focus outlines are visible and not clipped;
- every theme renders nonblank pixels and retains readable controls.

### 15.1 One-time manual prototype audit

The isolated mock was built and served with the production Next.js server for a
one-time manual design audit on 2026-07-27. The screenshots and measurements in
this subsection were collected with local browser tooling, are not reproduced
by a committed automated test, and may become stale as the preview changes. The
local image evidence is intentionally ignored by Git and is not product
acceptance evidence:

| Surface and state | Result | Repository-relative local evidence |
|---|---|---|
| Desktop Chat, light | Pass | `apps/web/test-results/product-ui-v2/final/desktop-light-chat.png` |
| Desktop Chat, dark | Pass | `apps/web/test-results/product-ui-v2/final/desktop-dark-chat.png` |
| Desktop Settings, light provider view | Pass | `apps/web/test-results/product-ui-v2/final/desktop-light-settings.png` |
| Desktop Settings, dark Memory view | Pass | `apps/web/test-results/product-ui-v2/final/desktop-dark-settings-memory.png` |
| Mobile Chat, light | Pass | `apps/web/test-results/product-ui-v2/final/mobile-light-chat.png` |
| Mobile Chat, dark | Pass | `apps/web/test-results/product-ui-v2/final/mobile-dark-chat.png` |
| Mobile Settings, light | Pass | `apps/web/test-results/product-ui-v2/final/mobile-light-settings.png` |
| Mobile Settings, dark | Pass | `apps/web/test-results/product-ui-v2/final/mobile-dark-settings.png` |
| Mobile Evidence sheet, dark | Pass | `apps/web/test-results/product-ui-v2/final/mobile-dark-evidence.png` |
| Mobile workspace drawer, dark | Pass | `apps/web/test-results/product-ui-v2/final/mobile-dark-workspaces.png` |
| Desktop reduced motion | Pass | `apps/web/test-results/product-ui-v2/final/desktop-light-reduced-motion.png` |
| Desktop Memory session selection | Pass | `apps/web/test-results/product-ui-v2/final/desktop-light-session-memory.png` |
| Desktop provider-attention session selection | Pass | `apps/web/test-results/product-ui-v2/final/desktop-light-session-provider.png` |
| Mobile Memory session selection | Pass | `apps/web/test-results/product-ui-v2/final/mobile-light-session-memory.png` |

That one-time manual browser audit recorded:

- no console or page errors across the screenshot matrix;
- exact desktop track bounds of `0..248`, `248..1136`, and `1136..1440`;
- document and body `scrollWidth` equal to the viewport at `1440x900`,
  `390x844`, and `375x812`;
- mobile Settings tabs sized to their content and contained by a horizontal
  scrolling strip;
- the long model label constrained to `144px`, with `258px` intrinsic width,
  hidden overflow, and an ellipsis;
- the first Tab stop on the skip link with a visible `2px solid` outline;
- closed mobile panels excluded from interaction, open panels exposed as modal
  dialogs, underlying Product Bar and main content marked `inert`, and Tab focus
  contained within the active panel;
- Escape closing each mobile panel and restoring focus to its exact trigger;
- the Settings workspace command returning to Chat and opening the real
  workspace panel rather than displaying an empty scrim;
- three stable mock session identities switching independent header, timeline,
  composer, and Inspector projections with exactly one `aria-current` item;
- click and Enter activation on desktop, plus mobile drawer closure and focus
  transfer to the selected session heading, without an API request;
- reduced-motion media matching, with the cursor and run indicator retained at
  `0.01ms` rather than removed;
- light contrast ratios of `4.69:1` for tertiary text on a surface, `4.77:1`
  for signal on a surface, and `4.77:1` for text on the signal;
- dark contrast ratios of `6.18:1` for tertiary text on a surface, `8.42:1`
  for signal on a surface, and `9.36:1` for text on the signal;
- rail-muted contrast ratios of `7.38:1` in light mode and `6.96:1` in dark
  mode;
- visible `2px solid` focus outlines in both themes, with focus-on-canvas
  contrast ratios of `4.23:1` in light mode and `11.81:1` in dark mode.

These manual Pass results apply only to the representative mock state. API, SSE,
runtime, persistence, and Desktop-host acceptance remain Not Run and Not
Implemented for Product UI V2.

### 15.2 Repeatable repository coverage

The committed Playwright spec
[`product-ui-v2.spec.ts`](../../apps/web/tests/e2e/product-ui-v2.spec.ts)
reproduces only the following claims:

- the route emits `noindex, nofollow` metadata and displays its inert mock
  boundary;
- switching among three independent mock sessions performs no `/api/` request;
- desktop Tab order leaves the persistent workspace rail for the main surface;
- the mobile workspace drawer and Evidence sheet expose modal semantics, wrap
  forward and backward Tab focus, close with Escape, and restore their triggers;
- mobile session selection closes the drawer and moves focus to the selected
  session heading.

The colocated Vitest source test locks the two approved palettes, stable mock
session identities, portable mock workspace root, and inert-boundary copy. It
does not reproduce the screenshot matrix, pixel bounds, overflow measurements,
contrast calculations, reduced-motion rendering, or console audit above.

### 15.3 Finesse preflight interpretation

The one-time manual Finesse preflight found that the applicable product-UI
checks pass: the anti-default and Design Read are explicit, one steel-cyan
accent is locked across both themes, neutrals and shadows are hue-tinted,
borders are translucent, visible copy contains no em dash, mobile controls have
`44px` targets, focus is visible and contained, selected text combinations meet
WCAG AA, and the `375px` layout has no document overflow. The same manual source
scan found no gradient, glow, glass, blur,
`transition: all`, negative letter spacing, viewport-scaled type, raw color
outside the two theme token blocks, or visible type below `10px`.

Brand and landing-page preflight items are not applicable to this product
register: hero composition, grain, display-type tension, negative tracking,
long-page layout-family counts, marketing imagery, logo walls, footer legal
links, and a spectacle engine. Rove deliberately uses `SPECTACLE=2`, no hero
engine, fixed product typography, and feedback-only motion. Adding those brand
devices would reduce density and operational clarity. The interactive route is
also the product prototype itself, visibly labeled as representative mock
state; it is not a div-based screenshot used to market an absent product.

## 16. Prototype Scope And Limitations

`/dev/product-ui-v2` is an intentionally packaged local developer preview, like
the existing `/dev/workbench` escape hatch. Its `noindex` metadata and visible
boundary label are defense in depth; deployment policy still decides whether a
development route is externally reachable. The route is an inert mock, not a
production UI or a runtime acceptance surface.

The isolated preview currently demonstrates:

- the shared shell and target Ice Steel Instrument palette;
- interactive switching among three stable, independent mock sessions;
- chronologically ordered mock timeline items;
- grouped mock tool invocation/result;
- interactive mock approval state;
- Trace Rail and evidence Inspector;
- active-run Steer/Follow-up affordance;
- provider, approval, Memory, session, and Desktop management mock states;
- responsive workspace drawer and Inspector sheet;
- light/dark switching and reduced-motion CSS.

It intentionally does not implement:

- a new transcript API or ordered runtime projection;
- Markdown, syntax highlighting, Mermaid, image, or diff engines;
- durable steer/follow-up queue semantics;
- real approval, cancellation, export, deletion, provider, or Memory calls;
- Desktop bootstrap, secure storage, native picker, packaging, or updater;
- production token replacement.

No production ProductApp, chat, Settings, state, API, shared stylesheet, package
manifest, or lockfile is changed by the prototype.

## 17. Implementation Implications

Implementation should be sequenced rather than copying the preview markup into
the production shell:

1. Repair workspace-scoped Memory and cancellation observation correctness.
2. Define and test the ordered timeline projection and stable identities.
3. Add host-neutral product transport and capability contracts.
4. Implement conversation rendering, auto-scroll, Markdown/code, and tool
   grouping against the ordered projection.
5. Define durable steer/follow-up queue behavior, then enable the active
   composer.
6. Implement full evidence export, cleanup, usage/context, file/diff, and
   management surfaces.
7. Re-run complete Web/API/local-full gates before Desktop host integration.

The implementation plan belongs under `docs/plans/` and must name worktree
ownership, shared-file hotspots, compatibility, migration, tests, and merge
order. This design deliberately does not mark any future acceptance row as Met.
