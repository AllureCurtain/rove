# Web Complete — Local Agent Web as Daily Driver

> Status: **Implemented and locally verified — C0–C3 stacked; coordinator integration pending**
>
> Date: 2026-07-26
>
> Updated: 2026-07-27
>
> Baseline: `main` after Web M1 (F0–F2) @ `93a724c` family.
>
> Execution plan:
> [`../plans/2026-07-26-web-complete.md`](../plans/2026-07-26-web-complete.md)
>
> Prior sealed UI baseline:
> [`./2026-07-25-agent-desktop-web-ui-design.md`](./2026-07-25-agent-desktop-web-ui-design.md)

This document freezes the **Web Complete** milestone: finish the local Web product
so it is a **daily-driver** agent surface, not only an M1 shell.

The C0 persistence/API foundation, C1 continuity UI, C2 Settings, and C3
migration/polish/live-API acceptance are implemented and locally verified on
the stacked Web Complete branch. Current implementation truth remains code plus
`docs/runtime/**`. The stacked PR chain has not landed on `main`; ordered
coordinator integration remains the final Web Complete delivery action.

---

## 1. Product decision

| Item | Decision |
|------|----------|
| Milestone name | **Web Complete** |
| Goal | Web is good enough for daily primary use |
| Relation to M1 | **Evolve** the sealed product shell; do not rebuild IA |
| Desktop (Tauri) | **Deferred** until Web Complete lands |
| Remote Gateway | Still out of scope |
| Delivery style | One sealed scope; **coordinator-owned foundations + bounded parallel worker worktrees** |

### Why Web Complete before Desktop

- M1 already delivered the shell, hard resume, and workspace-root execution.
- Completing Settings, migration/recovery, responsive/accessibility polish, and
  live product-shell acceptance before Desktop produced a stronger shared UI.
- Completing Web first means Desktop later hosts a stronger shared UI.

### Anti-goals

- Tauri / installers / tray / auto-update in this milestone.
- Remote Gateway / multi-user identity.
- Full IDE file tree, diff studio, MCP marketplace.
- Soft session continuity (“new job + frontend-only transcript stitch”).
- Replacing the sealed M1 shell with a new information architecture.

---

## 2. Starting point (M1 truth)

Already on `main` after F0–F2:

- Product shell default at `/` (Workspace → Session → Chat + collapsible Inspector).
- Settings full-page shell; deep **Providers** / **About** / **General** theme;
  **Advanced** hosts Benchmark; other sections may still be placeholders.
- Open workspace by absolute Folder/Repo path; recents/pin in browser storage.
- Create-job binds real workspace root; subsequent turns use **hard**
  `resume: "latest"` under that root.
- Old workbench only at `/dev/workbench` (non-primary).
- Light-first + dark tokens; parallel session running badges.

Known M1 leftovers (must be closed or explicitly carried with product-quality
fixes in Web Complete):

- Transcript not restored after refresh (catalog persists; messages in-memory).
- Switching sessions does not fully follow parallel SSE streams.
- Most Settings sections remain placeholders.
- Provider profiles live in `localStorage` only.
- URL surface is effectively single-page (`/`), weak deep-link/refresh position.

### C0 implementation progress

The implementation includes the API-global ProductStore, product
workspace/session/profile/preferences CRUD, exact server-owned
product-session/runtime bindings, single-active-turn supervision,
canonical-event transcript reads with typed partial reasons, strict/idempotent
M1 migration, and typed Web client/migration modules. Migration preparation is
deadline-bounded while the apply transaction is API-supervised and survives
HTTP disconnect; durable preflight baselines, preference revision CAS,
canonical runtime-store reservations, workspace containment, and no-follow
  SQLite opens protect the commit boundary.

### C1 implementation progress

The default product shell now consumes the C0 workspace/session/preferences,
provider-profile, and transcript clients. It implements durable workspace,
session, and Settings routes; canonical transcript restore with explicit
partial/error/retry states; exact `product_session_id` turns without client
`resume`; focused live-job SSE reattachment; and durable background status
polling. Network-ambiguous job starts use bounded binding reconciliation and do
not automatically repeat the mutation. Provider profiles and their active
selection are API-authoritative.

This closes the C1 continuity and authority-switch portion of the M1 leftovers.
C2 subsequently completed every Settings section, provider edit/update,
revision-safe approval defaults, catalog and Memory management, runtime health,
and critical shortcuts. C3 subsequently completed product-shell invocation of
the M1 migration module, final polish, and live local-API product-shell
evidence on the stacked branch. Coordinator integration into `main` remains
pending.

---

## 3. Web Complete outcomes

When this milestone is done, a local user can:

1. Open the Web app and land in a stable product shell.
2. Open/manage workspaces and sessions without feeling like a debug console.
3. Chat with streaming, tools, approvals, inspector — as in M1.
4. **Refresh the browser** and recover the **visible transcript** for the active
   session (or get an explicit partial/failure state — never a silent empty lie).
5. Continue the same session with **hard resume** still enforced.
6. Switch sessions with predictable run/follow behavior.
7. Configure **all Settings sections** at least to a usable depth (not empty
   placeholders).
8. Keep provider profiles and continuity-critical state in a **durable store**
   reached through the API (not browser storage alone).
9. Use URL deep links for workspace/session/settings so refresh keeps place.
10. Pass a single Web Complete acceptance script.

---

## 4. Information architecture

**No IA rewrite.** Keep the sealed M1 shell:

```text
App shell:
  TopBar | Workspace tree | Chat | collapsible Inspector

Settings shell:
  Full page, hides workspace tree, section nav + content
```

### Hierarchy (unchanged)

```text
Workspace (local directory: folder | repo | task)
  └── Session (conversation thread)
        └── Run (job/run execution)
```

### Routing (now required)

M1 allowed a mostly single-page shell. Web Complete **requires** durable routes:

```text
/                                 → last session, empty state, or redirect
/w/:workspaceId                   → workspace default session
/w/:workspaceId/s/:sessionId      → chat + inspector for that session
/settings                         → settings (default section)
/settings/:section                → settings section
/dev/workbench                    → advanced escape hatch only
```

Client state and server IDs must round-trip through these routes.

---

## 5. Continuity model

### 5.1 Hard resume (non-negotiable, inherited)

- First durable turn in a product session: create-job **without** resume.
- M1 sent workspace-scoped `resume: "latest"`. C0 implements the replacement
  server-owned product-session binding, and C1 wires the shell to it:
  later product turns resolve the session's **exact latest runtime run** under
  the same workspace root. The shell sends `product_session_id` and omits the
  lower-level client `resume` field.
- Client-supplied resume state cannot override or conflict with that server
  binding. Different product sessions in the same workspace keep distinct
  runtime chains.
- Fail closed. No product path that “continues” by opening a disconnected one-shot
  job and stitching bubbles only in the frontend.

### 5.2 Transcript restore after refresh

**Decision:** restore the **visible conversation** for the active session.

| Rule | Detail |
|------|--------|
| Target | Active session’s user-visible transcript (messages, tool cards, approvals as displayable history) |
| Source of truth | Durable runtime/API artifacts (runs, reports, job state, and any new read APIs added for this milestone) |
| Quality bar | Prefer **complete** restore; if only partial history can be reconstructed, show **explicit partial** UI — do not pretend full history |
| Failure | Clear error + recovery (“retry restore”, “start viewing from latest run”, “new session”) |
| Out of scope | Perfect multi-device collaborative editing of transcripts |

**Rejected alternative:** “refresh only preserves resume ability, not chat bubbles.”
That keeps developer continuity but fails daily-driver UX.

### 5.3 Session switch / parallel observation

| Rule | Detail |
|------|--------|
| Multiple sessions may run | Already allowed in M1 |
| Switching away | Must not corrupt hard-resume bookkeeping |
| Switching back | User must see correct transcript + correct active/latest run state |
| Live follow | Best-effort live SSE while a session is focused; background sessions show durable status badges; returning to a session reattaches or rebuilds from durable state |
| Forbidden | Losing a running session’s durability flags because the UI unmounted |

---

## 6. Settings completeness

C1 makes General theme/safe preferences and Provider list/create/delete plus
active selection API-backed. C2 completes the full Web Complete Settings bar:
all nine sections below are implemented and none remains placeholder-only.

| Section | Web Complete bar |
|---------|------------------|
| **General** | Theme + basic preferences; persisted |
| **Providers & Models** | Full profile CRUD, test, list models, default selection; durable store |
| **Tools & Approvals** | Read/write approval policy and tool-facing preferences that the product can honor |
| **Workspace / Paths** | Workspace list management hooks, path rules, sensible defaults documentation in UI |
| **Memory** | Usable view/manage of session/durable memory surfaces already exposed by API/runtime (depth pragmatic, not a full organizer product) |
| **Sessions** | Rename/delete (or archive), export if feasible, cleanup entry points |
| **Keyboard shortcuts** | Documented map; wire critical ones that already fit the shell |
| **Advanced / Developer** | Benchmark + escape hatches only |
| **About / Runtime** | Connection, versions, workspace/resume health hints |

Depth may vary by section, but **no section may remain a dead “Coming later” card
without any real capability.**

---

## 7. Persistence authority

### 7.1 Decision

Browser `localStorage` is **not** the long-term authority for:

- provider profiles
- session catalog needed for restore
- continuity markers required for hard resume + transcript rebuild

Those move to **API-backed durable storage** on the local rove-api/runtime side.
C0 implements the backend as API-global `<state_dir>/product.sqlite`; C1 makes
that store authoritative for the default shell's catalog, safe preferences, and
provider profiles.

### 7.2 Still allowed in the browser

- Ephemeral UI prefs that are safe to lose (e.g. inspector collapsed).
- Cache of server state for snappy load, always revalidatable.

### 7.3 Secrets

Unchanged: browser never holds or sends raw provider keys; only `api_key_env`
(or future secret refs of equal safety).

### 7.4 Migration

On first Web Complete load, migrate any M1 `localStorage` profiles/catalog into
durable storage when present; do not drop user config silently. C0 implements
the strict API and replay-safe browser migration state machine. C3 invokes that
state machine before mounting API-authoritative product catalog reads. Pending,
rejected, blocked, and superseded outcomes fail closed; retries preserve the
exact idempotency key and body; only a newly completed migration shows a success
summary; and validated workspace/session mappings rewrite legacy deep routes
without dropping query or fragment state.

---

## 8. API / platform expectations

Web Complete C0 extends `apps/api` and adds thin Web read/client models under
the following constraints:

- Extend jobs/SSE/resume contracts; do not invent a second chat protocol.
- Prefer additive endpoints for:
  - listing sessions/workspaces known to the product store
  - fetching transcript/rebuild payloads for a session
  - CRUD provider profiles
  - settings subsets that must be durable
- Fail closed on restore/resume errors.
- Keep OpenAPI and web client types in sync.

The C0 contract is sealed further in the
[Web → Desktop master delivery plan](../plans/2026-07-25-web-desktop-master-delivery.md):

- `ProductStore` is API application-global state, separate from each execution
  workspace's trace/task/report store;
- a server-owned product session binds one workspace to an ordered set of exact
  runtime session/job/run identities;
- transcript restore is a read projection over those ordered run bindings and
  canonical events, with typed `partial` reasons when facts are unavailable;
- M1 browser migration is versioned and idempotent, and never uploads raw keys.

Platform adapter remains Web-first:

- path entry may stay typed paths on Web
- Desktop picker stays future host work

---

## 9. Visual / UX baseline

Inherit M1 visual seals:

- product register, light-first + dark
- neutral surfaces + single ink/harbor blue accent
- semantic greens/ambers/reds only for status
- no AI-purple neon, no emoji chrome

Web Complete adds:

- restore/partial/error empty-states that meet the same craft bar
- settings forms with full interaction states (loading/empty/error/success)
- deep-link landings that do not flash the wrong shell
- 390px/320px product-shell reflow with a bounded mobile Inspector and composer
- visible high-contrast focus, contained/restored dialog focus, dynamic status
  semantics, and editable-safe shortcuts
- reduced-motion behavior, server-confirmed theme bootstrap, active Settings-tab
  visibility, and representative light/dark/narrow screenshot artifacts

---

## 10. Non-goals (explicit)

| Non-goal | Why |
|----------|-----|
| Tauri Desktop host | Separate D0 milestone after Web Complete |
| Remote Gateway | Different security and product class |
| MCP hub marketplace / SSH / tunnels | LiveAgent-scale surface |
| Full IDE project explorer | Not required for daily chat+manage driver |
| Multi-user accounts / billing | Out of local-first scope |
| Soft resume stitch | Violates sealed continuity |

---

## 11. Acceptance mindset

The Web Complete implementation acceptance script passes on the clean
main-derived C3 worktree against a live local `rove-api`, including:

- cold open → work → refresh → transcript present
- second turn hard resume after restore
- settings sections usable
- provider profiles survive browser storage clear **if API store remains**
- deep links restore place

The `local-full` fake-provider run passed all three real-API Playwright
scenarios: M1 migration, default-shell continuity/refresh/tools/cancellation/
Settings, and the bounded advanced `/dev/workbench` smoke. The external-provider
gate was not run, so this is local runtime/API evidence rather than external
provider interoperability evidence. Web Complete is not recorded as landed on
`main` until the coordinator reviews and integrates the stacked PR chain.

---

## 12. Relationship to later Desktop

Desktop remains:

```text
same product UI + Tauri host + native folder picker + embedded API bootstrap
```

Web Complete should make that cheaper, not harder:

- durable settings/profiles via API
- routes that a webview can open directly
- platform seams unchanged in spirit

---

## changelog

- 2026-07-27: Completed and locally verified C3 on the stacked Web Complete
  branch. The default shell now gates catalog boot on fail-closed M1 migration,
  preserves mapped deep routes, completes responsive/accessibility/theme and
  visual polish, and passes the three-scenario `local-full` live-API suite. The
  external-provider gate was not run; coordinator integration into `main`
  remains pending, and Desktop remains deferred.
- 2026-07-27: Marked C2 implemented: all nine Settings sections are usable;
  provider CRUD, approval/step preferences, workspace/session management,
  Memory, runtime health, shortcuts, and mobile bounds have focused and
  mock-backed browser evidence. C3 was completed later on its stacked branch.
- 2026-07-27: Marked C1 implemented: API-authoritative product state, canonical
  transcript restore with explicit partial/error handling, durable deep routes,
  exact product-session turns, focused reattachment/background status polling,
  provider persistence, and bounded ambiguous-start reconciliation. Evidence is
  mock-backed at the browser boundary; C2 and C3, including C3 live local-API
  acceptance, were completed later on their stacked branches.
- 2026-07-26: Marked C0 implemented: API-global ProductStore, exact
  product-session continuation, canonical-event transcript projection,
  strict/idempotent supervised migration, runtime commit guards, and typed Web
  client modules. Default-shell adoption was completed by C1; C2 was completed
  later, followed by C3 on its stacked branch.
- 2026-07-25: Delivery coordination amended after implementation audit. The
  original serial-wave recommendation is replaced by coordinator-owned contract
  foundations plus bounded disjoint workers. Sealed exact product-session/run
  binding, canonical-event transcript projection, and API-global ProductStore;
  no implementation was claimed at that time.
- 2026-07-26: Accepted. Web Complete sealed as next milestone; Desktop deferred.
  Scope: continuity restore, full settings usability, API-backed persistence,
  deep links; multi-worktree serial delivery defined in the plan.
