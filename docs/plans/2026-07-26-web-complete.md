# Web Complete Delivery Plan

> Status: **Completed — C0–C3 integrated and verified on main**
>
> Decisions:
> [`../design/2026-07-26-web-complete-design.md`](../design/2026-07-26-web-complete-design.md)
>
> Prior M1 plan (completed on main):
> [`./2026-07-25-web-management-m1.md`](./2026-07-25-web-management-m1.md)
>
> Coordinator plan and PR/worktree authority:
> [`./2026-07-25-web-desktop-master-delivery.md`](./2026-07-25-web-desktop-master-delivery.md)
>
> Rules:
>
> - **Docs commit to `main` first**, then create implementation worktrees from that tip.
> - **All implementation in `.worktrees/<name>`**, not the primary checkout mid-flight.
> - Milestone is one product outcome (**Web Complete**); delivery uses
>   coordinator-owned foundations followed by bounded, same-SHA workers.
> - Only the primary conversation creates worktrees, merges worker PRs into an
>   integration branch, and merges an integration PR into `main`.
> - Do not invent a second chat protocol. Extend jobs/SSE/resume + additive read/CRUD APIs.
> - Soft session continuity remains **forbidden**.
> - Desktop / Gateway are **out of this plan’s implementation waves**.

---

## 1. Parallelism verdict

| Pair | Parallel? | Why |
|------|-----------|-----|
| **C0 contract foundation ∥ anything** | **No** | Product session/runtime identity, ProductStore, transcript projection, and public API shapes must be committed first |
| **C0 store ∥ transcript ∥ Web client workers** | **Yes after foundation** | Workers start from one SHA and own new disjoint internal files; coordinator owns routers/public types/OpenAPI |
| **C1 continuity ∥ C2 settings/platform API** | **Yes after C0** | C1 owns Web shell/routes; C2 initially owns new backend modules and thin clients, not `SettingsShell` |
| **Settings catalog ∥ Settings runtime sections** | **Yes after a shell split** | Coordinator first commits stable section modules and shared navigation/form boundaries |
| **Web Complete ∥ Desktop** | **No for implementation** | Desktop explicitly deferred |
| **Any wave ∥ uncommitted docs** | **No** | Breaks shared baseline |

### Recommended mode

```text
P0 docs PR → main
  → C0 coordinator contract foundation
      ├─ store worker
      ├─ transcript worker
      └─ Web client worker
  → C0 integration PR → main
      ├─ C1 continuity/routing
      └─ C2 settings/platform APIs
  → coordinator Settings shell split
      ├─ settings catalog sections
      └─ settings runtime sections
  → C3 polish + acceptance
  → coordinator review/integration → main
```

Parallel workers never merge their own PRs. The primary conversation reviews
and merges them into the named integration branch, then owns the PR to `main`.

---

## 2. Worktree map

| Wave | Directory | Branch | Goal |
|------|-----------|--------|------|
| P0 docs | `.worktrees/product-roadmap-reconcile` | `planning/product-roadmap-reconcile` | Reconcile current truth, contract, topology, and handoff |
| **C0 integration** | `.worktrees/web-complete-persistence` | `feature/web-complete-persistence` | Coordinator foundation, worker integration, API/OpenAPI wiring |
| C0 store worker | `.worktrees/web-c0-store` | `feature/web-c0-store` | Internal ProductStore schema/repository |
| C0 transcript worker | `.worktrees/web-c0-transcript` | `feature/web-c0-transcript` | Canonical-event transcript read projection |
| C0 Web client worker | `.worktrees/web-c0-client` | `feature/web-c0-client` | Thin typed product client and browser migration state machine |
| **C1** | `.worktrees/web-complete-continuity` | `feature/web-complete-continuity` | Restore transcript, session switch/SSE reattach, deep links |
| **C2 integration** | `.worktrees/web-complete-settings` | `feature/web-complete-settings` | Settings/platform API foundation and later UI integration |
| Settings catalog worker | `.worktrees/web-settings-catalog` | `feature/web-settings-catalog` | General, Providers, Workspace/Paths, Sessions |
| Settings runtime worker | `.worktrees/web-settings-runtime` | `feature/web-settings-runtime` | Tools/Approvals, Memory, Keyboard, Advanced, About |
| **C3** | `.worktrees/web-complete-polish` | `feature/web-complete-polish` | Edge states, migration polish, full acceptance script green |

The C0 workers were created from the sealed foundation and their store,
transcript, and Web client implementations were integrated with
coordinator-owned route/job/migration/stream wiring and commit-boundary
hardening. C0 passed aggregate Rust/Web CI and entered `main` only through the
integration PR. C1 then wired the default shell to those contracts. Its Web
gates pass: `pnpm test` (14 files, 121 tests), `pnpm typecheck`, `pnpm build`,
and the mock-backed continuity Playwright suite (17/17). C2 and C3 were then
implemented in dependency order as a stacked branch chain and integrated into
`main` through PRs #25 and #26 after C1 landed through PR #24.

### After each merge

```powershell
cd D:\Study\project\agent\rove
git checkout main
git pull
git worktree remove .worktrees/web-complete-<wave>
git branch -d feature/web-complete-<wave>
```

---

## 3. Wave details

### C0 — Persistence / API foundation

#### C0 goal

Make durable product state authoritative on the API/runtime side.

#### C0 in scope

1. **Provider profile store** (CRUD) via API; no raw keys in browser.
2. **API-global ProductStore** for a workspace/session catalog, ordered runtime
   run bindings, safe preferences, and schema/migration receipts. Runtime event
   facts stay in each execution workspace store.
3. **Exact product-session continuity**: a server-owned product session binds
   one workspace and its exact latest runtime session/job/run. Product turns do
   not use workspace-global `latest` after binding.
4. **Transcript rebuild read model**: product session → ordered run bindings →
   canonical sequenced events, with typed `partial` reasons. Do not create a
   second writable chat protocol or copy event truth into ProductStore.
5. Versioned, idempotent migration from M1 `localStorage`; the browser marks
   success only after the server commits, and never uploads raw keys.
6. OpenAPI + `apps/web` client types for new endpoints.
7. Integration tests for store CRUD, exact-session resume, migration, and
   rebuild payload shape.

#### C0 out of scope

- Full Settings UI polish (C2)
- Deep-link router work except temporary client hooks if required for tests
- Desktop host

#### C0 exit checklist

- [x] Provider profiles survive browser storage clear while API process/state remains
- [x] Session list/detail needed by UI can be loaded from API
- [x] Transcript rebuild endpoint/payload returns ordered canonical events or an explicit partial status/reason
- [x] Two product sessions in one workspace resume their own exact runtime runs
- [x] Hard resume remains fail-closed for missing/mismatched runtime state
- [x] ProductStore does not duplicate canonical trace/task/report event truth
- [x] Browser migration is versioned/idempotent and rejects raw-key fields
- [x] Focused store, route, transcript, exact-resume, migration, stream, and Web client coverage exists on the implementation branches
- [x] Final aggregate contract/security review, current-state docs PR, integration CI, and integration PR to `main`

The final review closed the cancellation and commit-boundary questions:

- the 30-second migration deadline covers preparation only;
- accepted apply work runs in an API-owned supervisor and survives handler
  disconnect;
- ProductStore persists and reuses the first preflight preference baseline,
  applies preferences with revision CAS, and removes the preparation atomically
  with a success receipt;
- a source-mapped active session returns typed `product_session_active`, while
  the Web retains the exact pending key/body for retry;
- runtime SQLite paths are canonicalized, sorted, reserved, bounded to the
  workspace when external paths are disabled, and opened with no-follow guards;
- `POST /jobs` preparation runs in an owned task tracker, and shutdown drains
  starts before supervisors and handles.

#### C0 likely touch surfaces

- `apps/api/**`
- `runtime/**` state/read helpers as needed
- `apps/web/lib/rove-types.ts`, thin client only if needed for tests
- `docs/runtime/**` only if user-visible semantics change

The detailed schema, hotspot ownership, worker allowed-file policy, and merge
order are authoritative in the master delivery plan §4–§5.

---

### C1 — Continuity UI

#### C1 goal

Refreshing and switching sessions feels like a real product conversation surface.

#### C1 depends on

C0 on `main` (rebuild + catalog APIs).

#### C1 in scope

1. Load active session transcript on boot/route enter via C0 rebuild.
2. Partial/error restore UI.
3. Session switch: save/restore focus, reattach or rebuild run observation.
4. Background session status badges remain correct.
5. App routes:
   - `/w/:workspaceId/s/:sessionId`
   - `/settings/:section`
   - sensible redirects from `/`
6. Keep hard resume behavior; after restore, the next turn uses the C0
   server-owned product-session binding to resume its exact latest run.

#### C1 out of scope

- Implementing every Settings section (C2)
- New provider store backend (already C0)

#### C1 exit checklist

- [x] Refresh on a session URL restores bubbles (or explicit partial/error)
- [x] Second turn after restore uses the exact product-session binding in
      mock-backed Playwright, backed by C0 exact-resume API integration tests
- [x] Switching sessions does not drop or flash the wrong transcript
- [x] Parallel running/attention badges remain accurate without background SSE
- [x] Playwright coverage exists for restore, routes, focused reattachment,
      ambiguous job-start reconciliation, and provider persistence

C1 browser evidence is intentionally mock-backed. The live `rove-api` product
shell acceptance gate was reserved for C3; completing C1 did not satisfy §5 by
itself. C3 later completed that gate on the stacked branch.

#### C1 likely touch surfaces

- `apps/web/shell/**`, `state/**`, `chat/**`, `app/**` routes
- `api/run-controller.ts` reattach behavior
- e2e under `apps/web/tests/e2e`

---

### C2 — Settings completeness

#### C2 goal

Every Settings section is usable and persists through C0 stores where required.

#### C2 depends on

C0 on `main`; C1 continuity must be integrated before Settings UI workers start.
The settings/platform API lane may have run in parallel with C1, but its branch
must update to post-C1 `main` before the coordinator commits a stable Settings
shell/module split.

#### C2 in scope

Delivery is split into two dependency-ordered parts:

1. C2 integration builds bounded backend APIs and thin clients for preferences,
   policies, workspace/session management, and memory views. It does not edit
   `SettingsShell` while C1 is active.
2. After C1 lands, the coordinator updates the C2 integration branch to that
   post-C1 `main`, then splits Settings into stable section modules and shared
   form/navigation boundaries. The catalog and runtime Settings workers start
   from that same foundation SHA and submit PRs to the C2 integration branch.

| Section | Work |
|---------|------|
| General | Complete usable preference controls; C1 already persists theme and safe selection/focus preferences through the API |
| Providers | Complete edit/update and section polish; C1 already uses C0 list/create/delete plus API-backed active selection |
| Tools & Approvals | Real controls for approval defaults / tool policy the backend honors |
| Workspace / Paths | Manage known workspaces, path guidance, remove/pin durable |
| Memory | Browse/read/manage using existing memory APIs; pragmatic depth |
| Sessions | Rename/delete/export/cleanup against durable catalog |
| Keyboard | Documented shortcuts + wire high-value ones |
| Advanced | Keep Benchmark; no primary-nav leak |
| About | Richer runtime/connection/resume health |

#### C2 exit checklist

- [x] No settings section is a dead placeholder-only page
- [x] Provider CRUD uses API store
- [x] Tools/Approvals changes actually affect subsequent jobs
- [x] Sessions management operations work and are tested where critical
- [x] Memory section can show real data or honest empty from API

C2 is implemented by the revision-safe settings/platform API and Web client,
the nine-section Settings shell, focused unit tests, and mock-backed
`apps/web/tests/e2e/settings.spec.ts`. Live-API default-shell acceptance was the
separate C3 gate and was completed later on the stacked branch.

#### C2 likely touch surfaces

- new backend settings/platform modules and thin client contracts
- `apps/web/settings/**`
- coordinator-owned API router/public types/OpenAPI wiring
- docs in `apps/web/README.md`

---

### C3 — Polish + acceptance

#### C3 goal

Close dual-entry debt leftovers, migration edges, and pass the full Web Complete
script on a clean tree.

#### C3 depends on

C0 plus the ordered C1–C2 stacked integration chain.

#### C3 in scope

1. localStorage → API migration UX (one-time, safe).
2. Empty/loading/error consistency across restore + settings.
3. Final README / onboarding / plan checklist updates.
4. Full acceptance script automation where practical (Playwright + API tests).
5. Replace the current `/dev/workbench`-only real-API suite and stale
   provider-runner browser selectors with product-shell live-API coverage;
   retain bounded advanced-surface coverage.
6. Remove leftover M1-only assumptions in copy.

Implementation status: complete and verified on `main`. The default product
shell runs the fail-closed
migration gate before catalog boot, reuses the exact idempotency key and body on
retry, preserves mapped deep routes, and includes the responsive,
keyboard/focus, live-status, reduced-motion, theme, and visual polish required
by the C3 seal.

#### C3 exit checklist

- [x] Acceptance script (§5) passes through the local fake-provider real-API
      gate plus focused deterministic Web suites
- [x] M1 leftovers list either fixed or explicitly reclassified with reason
- [x] Live-API Playwright evidence exercises `/`, not only `/dev/workbench`
- [x] `pnpm test`, `typecheck`, `build`, and focused e2e are green
- [x] No Desktop/Gateway implementation scope was introduced
- [x] External-provider validation is explicitly optional; it was not run and
      no external-provider evidence is claimed
- [x] Coordinator review and ordered integration of the stacked PR chain into
      `main`

---

## 4. Implemented C0 API surface

C0 uses these additive REST shapes:

```text
GET    /product/workspaces
POST   /product/workspaces
DELETE /product/workspaces/{workspace_id}

GET    /product/sessions?workspace_id=
POST   /product/sessions
PATCH  /product/sessions/{session_id}
DELETE /product/sessions/{session_id}

GET    /product/sessions/{session_id}/transcript   # complete/partial + typed reasons

GET    /product/provider-profiles
POST   /product/provider-profiles
PUT    /product/provider-profiles/{profile_id}
DELETE /product/provider-profiles/{profile_id}

GET/PUT /product/preferences               # theme + safe UI prefs if durable
POST   /product/migrations/m1-browser      # versioned/idempotent sanitized import
```

Jobs/SSE/approvals remain the execution path:

```text
POST /jobs   (+ workspace root + product_session_id; server resolves exact run)
GET  /jobs/{id}/events
...
```

Transcript responses expose ordered run segments plus the canonical sequenced
events already persisted for those runs, with `complete | partial` and typed
partial reasons. Existing reports/task state may supply bounded fallback
metadata, but C0 does not add an independent append-only chat log merely for UI
restore.

---

## 5. Web Complete acceptance script

Run on a clean `main` tree with `rove-api` + Web. The post-merge tree passed
this local acceptance path with the fake provider. `local-full` reported all
three real-API Playwright scenarios passing: live M1 migration, default-shell
exact continuity/refresh/tools/cancellation/Settings, and the bounded advanced
`/dev/workbench` smoke. The external-provider gate was not run.

1. Cold start Web; empty or last-route landing works.
2. Open Folder workspace by absolute path; create session.
3. Send turn 1 (fake or configured provider); see stream + inspector.
4. Handle approval if triggered.
5. Send turn 2; confirm the server resumes the active product session's exact
   prior run, not another session's workspace-global latest run.
6. **Refresh browser** on session URL; transcript restores (or explicit partial).
7. Send turn 3 after restore; hard resume still works.
8. Open second session; run in parallel; badges show running.
9. Switch between sessions; correct transcript/state each time.
10. Settings → Providers: create profile, test, list models; restart browser
    with cleared site data; profiles still present via API store.
11. Settings → Tools & Approvals: change a real policy; new job honors it.
12. Settings → Workspace/Paths & Sessions: manage entries without console hacks.
13. Settings → Memory: see real empty or real items from API.
14. Settings → Keyboard: page usable; critical shortcuts documented.
15. Deep link open `/settings/providers` and a session URL works.
16. Theme toggle persists appropriately.
17. `/dev/workbench` remains non-primary; Benchmark only under Advanced.
18. `pnpm test && pnpm typecheck && pnpm build` and targeted e2e green.

---

## 6. Documentation obligations

When a wave merges:

1. Update `apps/web/README.md` capabilities honestly.
2. Update OpenAPI/client types with API changes.
3. Touch `docs/runtime/**` only if runtime semantics change.
4. Check off wave exit items in this plan via changelog notes (do not silently
   rewrite history).
5. Keep Desktop-specific work in the dedicated D0 design and plan; D0 is now
   implemented on `program/full-delivery` and is not part of the historical C0-C3
   merge sequence.

---

## 7. Completion handoff

C0–C3 implementation, ordered coordinator integration, and post-merge local
verification are complete. Do not reintroduce browser authority or
workspace-global `latest` inside `ProductApp`. The optional external-provider
gate remains unrun. At this historical C0-C3 handoff, Desktop D0 was still
deferred and had not started; it was implemented later on
`program/full-delivery`.

---

## 8. Explicit backlog after Web Complete (not this plan)

- D0 Desktop Tauri host
- Remote Gateway
- IDE-grade file tree / diff studio
- MCP hub marketplace
- Multi-user hosted product

---

## changelog

- 2026-07-27: Integrated the stacked delivery chain in dependency order. PR #24
  merged C1 as `db8f970`, PR #25 merged C2 as `abbd7d6`, and PR #26 merged C3
  as `e3c2403`. The post-merge Rust/Web gates, mock Playwright suite (44 passed,
  3 opt-in skipped), and `local-full` real-API suite (3/3) passed. Desktop D0
  was not started, external-provider validation was not run, and historical
  worktrees were retained.
- 2026-07-27: Completed C3 implementation and local acceptance in the stacked
  polish worktree. Product boot now runs fail-closed M1 migration before catalog
  reads; mapped deep routes, responsive layouts, focus/keyboard and live-status
  behavior, reduced motion, theme restore, and screenshot artifacts are covered.
  `local-full` passed all three real-API Playwright scenarios, including the
  bounded `/dev/workbench` smoke. External-provider validation was not run and
  coordinator integration into `main` remains pending.
- 2026-07-27: Completed C1 continuity integration. The default shell now uses
  API-authoritative catalog/preferences/profiles, durable workspace/session and
  Settings routes, canonical transcript restore with explicit partial/error
  states, exact `product_session_id` turns, focused SSE reattachment, background
  status polling, and bounded no-duplicate reconciliation after ambiguous job
  starts. `pnpm test` (14 files, 121 tests), `pnpm typecheck`, `pnpm build`, and
  the mock-backed continuity Playwright suite (17/17) pass. At that point C2
  Settings completeness and C3 live-API acceptance remained open; both were
  completed later on their stacked branches.
- 2026-07-26: Completed C0 after aggregate contract/security review. Migration
  preparation/apply ownership, durable baselines and preference CAS,
  active-session retry, canonical runtime reservations/path guards, and owned
  job-start shutdown order are covered by focused tests. Current docs and
  aggregate Rust/Web CI were integrated through the C0 PR path; no C1–C3 or
  Desktop work was started.
- 2026-07-26: Sealed the coordinator-owned C0 public foundation on the
  integration branch: server-owned product IDs and store traits, product
  route/OpenAPI shapes, typed transcript/partial and strict migration DTOs,
  bounded runtime event snapshots, and fail-closed `product_session_id` entry.
  Worker implementations and integration remain open.
- 2026-07-26: Completed P0 reconciliation. Corrected route-scoped browser
  evidence, made the existing C0 worktree update command idempotent, and aligned
  C2 with the coordinator foundation plus post-C1 Settings worker split.
- 2026-07-25: Reconciled after M1/code audit. Replaced the original purely
  serial delivery recommendation with a coordinator-owned C0 foundation and
  bounded same-SHA worker lanes, followed by partitioned C1/C2 and Settings
  work. Sealed ProductStore ownership, exact product-session/runtime mapping,
  canonical-event transcript projection, idempotent migration, and
  primary-conversation-only PR merge authority. No C0 implementation claimed.
- 2026-07-26: Initial sealed plan for Web Complete serial waves C0–C3.
  Implementation not started by this document alone.
