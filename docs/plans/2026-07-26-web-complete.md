# Web Complete Delivery Plan

> Status: **Ready to implement from a clean `main` worktree after this plan is committed**
>
> Decisions:
> [`../design/2026-07-26-web-complete-design.md`](../design/2026-07-26-web-complete-design.md)
>
> Prior M1 plan (completed on main):
> [`./2026-07-25-web-management-m1.md`](./2026-07-25-web-management-m1.md)
>
> Rules:
>
> - **Docs commit to `main` first**, then create implementation worktrees from that tip.
> - **All implementation in `.worktrees/<name>`**, not the primary checkout mid-flight.
> - Milestone is one product outcome (**Web Complete**); delivery uses **serial waves**.
> - Do not invent a second chat protocol. Extend jobs/SSE/resume + additive read/CRUD APIs.
> - Soft session continuity remains **forbidden**.
> - Desktop / Gateway are **out of this plan’s implementation waves**.

---

## 1. Parallelism verdict

| Pair | Parallel? | Why |
|------|-----------|-----|
| **C0 ∥ C1** | **No as merge trains** | Continuity UI depends on durable session/transcript read models from C0 |
| **C0 ∥ C2** | **Partial only** | Settings forms can be sketched against mocks, but durable save paths need C0 store contracts; default **serial** |
| **C1 ∥ C2** | **Risky** | Both touch `apps/web` shell/state; only allow if strictly partitioned and rebased carefully |
| **Web Complete ∥ Desktop** | **No for now** | Desktop explicitly deferred |
| **Any wave ∥ uncommitted docs** | **No** | Breaks shared baseline |

### Recommended mode

```text
commit Web Complete docs on main
  → worktree C0 persistence/API foundation   (merge)
  → worktree C1 continuity UI                (merge)
  → worktree C2 settings completeness        (merge)
  → worktree C3 polish + acceptance          (merge)
```

One active Web Complete implementation worktree at a time by default.

---

## 2. Worktree map

| Wave | Directory | Branch | Goal |
|------|-----------|--------|------|
| Docs | primary `main` | `main` | This design + plan (+ index) |
| **C0** | `.worktrees/web-complete-persistence` | `feature/web-complete-persistence` | API-backed durable stores + transcript/session read models |
| **C1** | `.worktrees/web-complete-continuity` | `feature/web-complete-continuity` | Restore transcript, session switch/SSE reattach, deep links |
| **C2** | `.worktrees/web-complete-settings` | `feature/web-complete-settings` | All Settings sections usable + wired to durable store |
| **C3** | `.worktrees/web-complete-polish` | `feature/web-complete-polish` | Edge states, migration polish, full acceptance script green |

Create **only the next wave** when that wave starts.

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

#### Goal

Make durable product state authoritative on the API/runtime side.

#### In scope

1. **Provider profile store** (CRUD) via API; no raw keys in browser.
2. **Session/workspace catalog** durable enough for restore and navigation.
3. **Transcript rebuild read model** for a session (compose from runs/reports/state
   and/or new append-only transcript records — pick the smallest reliable design).
4. Migration path from M1 `localStorage` payloads when present.
5. OpenAPI + `apps/web` client types for new endpoints.
6. Integration tests for store CRUD + rebuild payload shape.

#### Out of scope

- Full Settings UI polish (C2)
- Deep-link router work except temporary client hooks if required for tests
- Desktop host

#### Exit checklist

- [ ] Provider profiles survive browser storage clear while API process/state remains
- [ ] Session list/detail needed by UI can be loaded from API
- [ ] Transcript rebuild endpoint/payload returns ordered visible items or explicit partial flag
- [ ] Hard resume contract unchanged (still fail-closed)
- [ ] Tests green; docs for new API surfaces updated

#### Likely touch surfaces

- `apps/api/**`
- `runtime/**` state/read helpers as needed
- `apps/web/lib/rove-types.ts`, thin client only if needed for tests
- `docs/runtime/**` only if user-visible semantics change

---

### C1 — Continuity UI

#### Goal

Refreshing and switching sessions feels like a real product conversation surface.

#### Depends on

C0 on `main` (rebuild + catalog APIs).

#### In scope

1. Load active session transcript on boot/route enter via C0 rebuild.
2. Partial/error restore UI.
3. Session switch: save/restore focus, reattach or rebuild run observation.
4. Background session status badges remain correct.
5. App routes:
   - `/w/:workspaceId/s/:sessionId`
   - `/settings/:section`
   - sensible redirects from `/`
6. Keep hard resume turn builder behavior; after restore, next turn still
   `resume: "latest"` when durable.

#### Out of scope

- Implementing every Settings section (C2)
- New provider store backend (already C0)

#### Exit checklist

- [ ] Refresh on a session URL restores bubbles (or explicit partial)
- [ ] Second turn after restore hard-resumes successfully in e2e/integration
- [ ] Switching sessions does not drop wrong transcript
- [ ] Parallel running badges still accurate
- [ ] Playwright coverage for restore + route landings

#### Likely touch surfaces

- `apps/web/shell/**`, `state/**`, `chat/**`, `app/**` routes
- `api/run-controller.ts` reattach behavior
- e2e under `apps/web/tests/e2e`

---

### C2 — Settings completeness

#### Goal

Every Settings section is usable and persists through C0 stores where required.

#### Depends on

C0 on `main`. Prefer after C1 to reduce shell conflicts; if started earlier, rebase
onto post-C1 main before merge.

#### In scope

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

#### Exit checklist

- [ ] No settings section is a dead placeholder-only page
- [ ] Provider CRUD uses API store
- [ ] Tools/Approvals changes actually affect subsequent jobs
- [ ] Sessions management operations work and are tested where critical
- [ ] Memory section can show real data or honest empty from API

#### Likely touch surfaces

- `apps/web/settings/**`
- possibly thin API handlers for settings subsets
- docs in `apps/web/README.md`

---

### C3 — Polish + acceptance

#### Goal

Close dual-entry debt leftovers, migration edges, and pass the full Web Complete
script on a clean tree.

#### Depends on

C0–C2 on `main`.

#### In scope

1. localStorage → API migration UX (one-time, safe).
2. Empty/loading/error consistency across restore + settings.
3. Final README / onboarding / plan checklist updates.
4. Full acceptance script automation where practical (Playwright + API tests).
5. Remove leftover M1-only assumptions in copy.

#### Exit checklist

- [ ] Acceptance script (§5) passes
- [ ] M1 leftovers list either fixed or explicitly reclassified with reason
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
```

Jobs/SSE/approvals remain the execution path:

```text
POST /jobs   (+ workspace root + resume)
GET  /jobs/{id}/events
...
```

If existing run report APIs can rebuild transcripts without new storage, C0 may
implement rebuild facades first and only add append-only transcript logs if
rebuild quality is insufficient.

---

## 5. Web Complete acceptance script

Run on clean main-derived tree with `rove-api` + Web:

1. Cold start Web; empty or last-route landing works.
2. Open Folder workspace by absolute path; create session.
3. Send turn 1 (fake or configured provider); see stream + inspector.
4. Handle approval if triggered.
5. Send turn 2; confirm hard resume (`resume: latest`) continuity.
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

### After this plan is on `main`

```powershell
cd D:\Study\project\agent\rove
git status --short
git pull
git worktree add .worktrees/web-complete-persistence -b feature/web-complete-persistence main
cd .worktrees/web-complete-persistence
```

### Next-session opener (C0)

```text
继续 rove Web Complete 的 C0 persistence/API foundation。
工作目录必须是 D:\Study\project\agent\rove\.worktrees\web-complete-persistence
分支 feature/web-complete-persistence。
只做 API 持久化底座：provider profiles、session/workspace catalog、transcript rebuild 读模型、localStorage 迁移入口；不要做完整 Settings UI，不要做 Desktop。
权威文档：
- docs/design/2026-07-26-web-complete-design.md
- docs/plans/2026-07-26-web-complete.md §C0
保持硬 resume fail-closed；禁止软拼接续聊。
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

- 2026-07-26: Initial sealed plan for Web Complete serial waves C0–C3.
  Implementation not started by this document alone.
