# Web Complete Delivery Plan

> Status: **Active — P0 reconciled; C0 coordinator foundation next**
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
  → C3 polish + acceptance → main
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

The coordinator creates C0 worker worktrees only after the C0 foundation commit,
all from the same SHA. Later worktrees are created only after their respective
foundation is on the integration branch or `main`.

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

- [ ] Provider profiles survive browser storage clear while API process/state remains
- [ ] Session list/detail needed by UI can be loaded from API
- [ ] Transcript rebuild endpoint/payload returns ordered visible items or explicit partial flag
- [ ] Two product sessions in one workspace resume their own exact runtime runs
- [ ] Hard resume remains fail-closed for missing/mismatched runtime state
- [ ] ProductStore does not duplicate canonical trace/task/report event truth
- [ ] Browser migration is versioned/idempotent and rejects raw-key fields
- [ ] Tests green; docs for new API surfaces updated

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

- [ ] Refresh on a session URL restores bubbles (or explicit partial)
- [ ] Second turn after restore hard-resumes successfully in e2e/integration
- [ ] Switching sessions does not drop wrong transcript
- [ ] Parallel running badges still accurate
- [ ] Playwright coverage for restore + route landings

#### C1 likely touch surfaces

- `apps/web/shell/**`, `state/**`, `chat/**`, `app/**` routes
- `api/run-controller.ts` reattach behavior
- e2e under `apps/web/tests/e2e`

---

### C2 — Settings completeness

#### C2 goal

Every Settings section is usable and persists through C0 stores where required.

#### C2 depends on

C0 on `main`. The settings/platform API lane may run in parallel with C1 because
it does not edit `SettingsShell`. Settings UI workers start only after C1 is
integrated and the coordinator commits a stable Settings shell/module split.

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
| General | Persist theme/preferences via durable or approved prefs API |
| Providers | Bind fully to C0 profile APIs; remove sole-authority localStorage |
| Tools & Approvals | Real controls for approval defaults / tool policy the backend honors |
| Workspace / Paths | Manage known workspaces, path guidance, remove/pin durable |
| Memory | Browse/read/manage using existing memory APIs; pragmatic depth |
| Sessions | Rename/delete/export/cleanup against durable catalog |
| Keyboard | Documented shortcuts + wire high-value ones |
| Advanced | Keep Benchmark; no primary-nav leak |
| About | Richer runtime/connection/resume health |

#### C2 exit checklist

- [ ] No settings section is a dead placeholder-only page
- [ ] Provider CRUD uses API store
- [ ] Tools/Approvals changes actually affect subsequent jobs
- [ ] Sessions management operations work and are tested where critical
- [ ] Memory section can show real data or honest empty from API

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

C0–C2 on `main`.

#### C3 in scope

1. localStorage → API migration UX (one-time, safe).
2. Empty/loading/error consistency across restore + settings.
3. Final README / onboarding / plan checklist updates.
4. Full acceptance script automation where practical (Playwright + API tests).
5. Replace the current `/dev/workbench`-only real-API suite and stale
   provider-runner browser selectors with product-shell live-API coverage;
   retain bounded advanced-surface coverage.
6. Remove leftover M1-only assumptions in copy.

#### C3 exit checklist

- [ ] Acceptance script (§5) passes
- [ ] M1 leftovers list either fixed or explicitly reclassified with reason
- [ ] Live-API Playwright evidence exercises `/`, not only `/dev/workbench`
- [ ] `pnpm test`, `typecheck`, `build`, focused e2e green
- [ ] No Desktop/Gateway scope creep merged

---

## 4. API sketch (contract direction, refine in C0)

C0 should prefer additive REST shapes (names indicative, not frozen until PR):

```text
GET    /product/workspaces
POST   /product/workspaces
DELETE /product/workspaces/{id}

GET    /product/sessions?workspace_id=
POST   /product/sessions
PATCH  /product/sessions/{id}
DELETE /product/sessions/{id}

GET    /product/sessions/{id}/transcript   # rebuild payload + partial flag

GET    /product/provider-profiles
PUT    /product/provider-profiles/{id}
DELETE /product/provider-profiles/{id}

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

Run on clean main-derived tree with `rove-api` + Web:

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
5. Keep Desktop marked deferred until a future D0 plan is sealed.

---

## 7. Ready-to-start commands

### After the P0 PR is on `main`

```powershell
cd D:\Study\project\agent\rove
git status --short
git pull --ff-only
cd .worktrees/web-complete-persistence
git status --short
git merge --ff-only main
```

### Next-session opener (C0)

```text
继续 rove Web Complete 的 C0 persistence/API foundation。
工作目录必须是 D:\Study\project\agent\rove\.worktrees\web-complete-persistence
分支 feature/web-complete-persistence。
先完成 coordinator-owned C0 契约 foundation：API-global ProductStore、server-owned product session → exact runtime run mapping、canonical-event transcript projection、versioned localStorage migration contracts；不要先开 worker，不要做完整 Settings UI，不要做 Desktop。
权威文档：
- docs/design/2026-07-26-web-complete-design.md
- docs/plans/2026-07-26-web-complete.md §C0
- docs/plans/2026-07-25-web-desktop-master-delivery.md §4–§5
保持 hard resume fail-closed；禁止 workspace-global latest 串错 Session，禁止软拼接续聊。
```

---

## 8. Explicit backlog after Web Complete (not this plan)

- D0 Desktop Tauri host
- Remote Gateway
- IDE-grade file tree / diff studio
- MCP hub marketplace
- Multi-user hosted product

---

## changelog

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
