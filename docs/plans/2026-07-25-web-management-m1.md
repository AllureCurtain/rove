# Web Management M1 Delivery Plan

> Status: **Ready to implement from a clean `main` worktree**
>
> Decisions:
> [`../design/2026-07-25-agent-desktop-web-ui-design.md`](../design/2026-07-25-agent-desktop-web-ui-design.md)
>
> Rules:
>
> - **All implementation happens in `.worktrees/<name>` worktrees**, not on the
>   primary `main` checkout.
> - Decision + plan docs must be **committed on `main` first**, then create the
>   worktree from that shared baseline.
> - Order is fixed: **backend probes/fixes for workspace root + hard resume →
>   Web product shell on live API → polish → later Tauri Desktop**.
> - Do not invent a second chat protocol. Extend existing jobs/SSE/resume contracts.
> - Soft session continuity (“new job + frontend transcript stitch only”) is
>   **forbidden** as the product path.
> - Existing cleanup worktrees (for example `cleanup/w2b`) are a **different track**.
>   Do not mix cleanup commits into Web M1 waves unless intentionally rebased after
>   merge coordination.

---

## 1. Parallelism verdict

### Can F0 / F1 / F2 run in parallel?

| Pair | Parallel? | Why |
|------|-----------|-----|
| **F0 ∥ F1** | **No (not as mergeable product work)** | F1 depends on F0 contracts: real `workspace_root` (or equivalent) and hard resume. Building the full shell against today’s API invites throwaway adapters and false “session continue”. |
| **F0 ∥ F2** | **No** | F2 polishes F1. |
| **F1 ∥ F2** | **No as separate merge trains** | F2 is the tail of the same UI surface; stacking is fine, independent parallel branches fight. |
| **F0 ∥ pure UI spike** | **Optional, non-blocking only** | A disposable exploration branch may sketch shells against mocks, but it is **not** an M1 delivery branch and must not merge ahead of F0. Default recommendation: **skip** to avoid dual sources of truth. |
| **Web M1 ∥ cleanup/w2b** | **Only if paths don’t collide** | Keep separate branches/worktrees. Merge one track at a time to `main`; rebase the other. Do not dual-write the same files. |

### Recommended execution mode

**Serial waves, one active implementation worktree at a time for Web M1:**

```text
commit docs on main
  → worktree F0 foundation   (merge)
  → worktree F1 shell        (merge)
  → worktree F2 polish       (merge)
  → later Desktop worktree
```

This is faster end-to-end than false parallelism that rebases broken UI onto a
moving resume contract.

---

## 2. Worktree workflow (mandatory)

### Why

- Protects primary `main` from half-finished product work.
- Easy rollback: remove worktree / abandon branch.
- Matches repo convention already used by cleanup waves (`.worktrees/` is
  gitignored).

### Before any implementation worktree

1. Primary tree on `main` must be **clean**:

   ```powershell
   cd D:\Study\project\agent\rove
   git status --short
   git branch --show-current
   ```

2. This design + plan must already be **committed** on `main`.

3. Create implementation worktrees **inside the repo**:

   ```powershell
   git worktree add .worktrees/web-m1-foundation -b feature/web-m1-foundation main
   git worktree list
   ```

   Work only under:

   `D:\Study\project\agent\rove\.worktrees\<name>`

4. Do **not** create sibling checkouts like `D:\Study\project\agent\rove-web-*`
   unless an explicit exception is recorded.

### Suggested worktree / branch map

| Wave | Directory | Branch | Goal |
|------|-----------|--------|------|
| Docs | primary `main` | `main` | Commit sealed design + plan only |
| F0 | `.worktrees/web-m1-foundation` | `feature/web-m1-foundation` | Workspace root execution + hard resume |
| F1 | `.worktrees/web-m1-shell` | `feature/web-m1-shell` | Shared product shell + chat/settings on live API |
| F2 | `.worktrees/web-m1-polish` | `feature/web-m1-polish` | Theme tokens, inspector completeness, old-entry cleanup |
| D0 (later) | `.worktrees/desktop-tauri-host` | `feature/desktop-tauri-host` | Tauri host over the same UI |

Create **only the next wave** worktree when that wave starts. Do not pre-create F1/F2
worktrees before F0 merges unless doing disposable spikes.

### After merge

```powershell
git checkout main
git pull
git worktree remove .worktrees/web-m1-foundation
git branch -d feature/web-m1-foundation
```

### Cargo / frontend

- Do not share `CARGO_TARGET_DIR` across Rust worktrees.
- Run focused tests in the active worktree; full relevant suite before PR.
- Web: `pnpm test`, `pnpm typecheck`, `pnpm build`, and targeted Playwright.

---

## 3. Wave map

| Wave | Goal | Depends on | Primary surfaces |
|------|------|------------|------------------|
| **Docs** | Shared baseline on `main` | sealed discussion | `docs/design/**`, `docs/plans/**`, `docs/00-README.md` |
| **F0 Foundation** | Opened path executes; same session hard-resumes | Docs on `main` | `apps/api`, `runtime`, bootstrap/config as needed, tests |
| **F1 Shell** | New Web product shell is primary entry and talks to live API | F0 on `main` | `apps/web`, thin API client/types updates |
| **F2 Polish** | Theme, inspector depth, placeholders, remove dual entry debt | F1 on `main` | `apps/web` styles/components, docs touch-ups |
| **D0 Desktop** | Tauri host | F1/F2 stable enough | new desktop crate/app + platform adapter |

---

## 4. F0 — Foundation (workspace root + hard resume)

### Goal

Make the two non-negotiables true **before** the product shell depends on them:

1. **1B** An opened workspace path is the real execution root.
2. **2A** Same-session multi-turn hard resume works like Claude Code continuity.

### Current gaps to close

- `CreateJobRequest.workspace` is task-oriented today; arbitrary Folder/Repo path
  binding is incomplete for the product model.
- Runtime session object is thin; product continuity must be proven through the
  existing resume/job/run path or explicitly extended.
- API process `cwd` must not silently become the only real workspace.

### Required outcomes

- Create/run job can target an explicit absolute workspace root.
- Runtime binds Folder vs Repo (and keeps Task) under that root.
- State/artifacts for that run live under the target workspace’s rebased state
  dir (default relative path resolves under the opened root; typically the
  configured `state.state_dir`, not necessarily a literal `.rove/` unless that
  is the active config).
- Continuing a session resumes durable runtime state for that conversation.
  Failure mode is explicit error, not silent one-shot degradation.
- Integration tests cover:
  - job in explicit folder root
  - job in explicit repo root
  - second turn resume in same session/workspace
  - reject/continue approval still works under explicit root

### Suggested task breakdown

1. Inventory current create-job workspace handling, resume resolution, and
   `session_id` propagation (`apps/api/src/lib.rs`, runtime state/resume).
2. Design the smallest API extension:
   - preferred shape: explicit `workspace_root` / Folder|Repo binding on create job
   - keep Task workspace support
   - fail closed on invalid/missing paths
3. Implement path binding through bootstrap/runtime workspace construction.
4. Prove hard resume:
   - identify the supported resume key(s) (`latest`, run id, session linkage)
   - ensure same product session can continue with runtime memory/state continuity
   - add tests that would fail under “frontend stitch only”
5. Document the resulting contract in API/OpenAPI and a short runtime note if
   behavior changes user-visible semantics.
6. PR from F0 worktree only after focused + relevant full checks pass.

### F0 out of scope

- New product chrome
- Theme system
- Provider profile persistence service
- Desktop packaging

### F0 exit checklist

- [x] Explicit workspace root executes tools against that root
- [x] Hard resume second turn passes automated test
- [x] No product path documents “stitch transcript as continuity”
- [x] OpenAPI/types updated if request shape changed
- [ ] Merged to `main` before F1 worktree creation

---

## 5. F1 — Web product shell on live API

### Goal

Replace the developer workbench primary entry with the sealed product shell, backed
by live `rove-api` (decision **5B**), after F0 is on `main`.

### Required screens

1. Empty workspace state
2. App shell: workspace tree + chat + collapsible inspector
3. Settings shell: full section nav; deep **Providers** + **About**
4. Open workspace via path + recents/pin (local persistence allowed for the list)
5. New session / switch session
6. Send message → SSE stream → tool cards → inline approval/input
7. Stop/cancel active run
8. Connection/API status affordance

### Architecture rules

```text
apps/web/
  shell/          AppShell, SettingsShell, TopBar
  sidebar/        WorkspaceTree, SessionItem, OpenWorkspace
  chat/           Transcript, Composer, ToolCard, ApprovalCard
  inspector/      RunInspector sections
  settings/       section pages (Providers/About deep)
  state/          workspace + session + run stores
  api/            evolved rove-client
  platform/       web adapter (desktop later)
  styles/         tokens + light/dark
```

- Do not grow `rove-workbench.tsx` into the final product.
- Absorb useful stream/approval state logic; discard workbench IA.
- Old workbench is **not** a second primary entry (decision **4B**). During F1 it
  may exist only as temporary migration scaffolding if needed, but the default
  route must be the new shell by F1 exit.
- Benchmark is not primary nav; if still reachable, only via Advanced/dev path.

### Live API mapping

| UI action | API |
|-----------|-----|
| Start turn | `POST /jobs` with workspace root + model/provider/resume fields |
| Stream | `GET /jobs/{id}/events` |
| Sync | `GET /jobs/{id}/state` |
| Cancel | `POST /jobs/{id}/cancel` |
| Approval | `POST /jobs/{id}/approvals/{call_id}` |
| Input | `POST /jobs/{id}/inputs/{input_id}` |
| History anchors | `GET /runs`, `GET /runs/{id}/report` |
| Provider test/models | `POST /providers/test`, `POST /providers/models` |

Provider profiles in M1 may persist in frontend storage and inject into create-job
requests. Browser never sends raw provider keys.

### Session continuity rule

- Product session continue **must** use F0 hard resume semantics.
- If resume cannot proceed, surface a hard error and recovery action.
- Do not silently open a disconnected one-shot job and pretend it is the same chat.

### F1 exit checklist

- [ ] Default Web entry is the new shell
- [ ] Open path workspace + create session + run live job works
- [ ] Second turn hard resume works in UI against live API
- [ ] Inline approval works
- [ ] Inspector shows plan/tools/approvals for active run
- [ ] Settings Providers deep path can test + list models
- [ ] Settings About shows connection/runtime basics
- [ ] Other settings sections visible as placeholders
- [ ] Playwright smoke for empty → open → run → approve (mock or real harness)
- [ ] Unit tests for client/state critical paths

---

## 6. F2 — Polish and close dual-entry debt

### Goal

Finish M1 product feel and remove temporary debt.

### Includes

- Tokenized light/dark theme complete enough for daily use (decision **3A**)
- Inspector completeness and empty/loading/error states
- Sidebar running badges for parallel sessions
- Remove residual workbench primary-entry debt
- Advanced-only home for benchmark if still needed
- Docs/runtime touch-ups for any user-visible API behavior landed in F0/F1

### F2 exit checklist

- [ ] Theme toggle works and tokens cover shell/chat/settings
- [ ] No second primary workbench homepage
- [ ] M1 demo script passes on clean main-derived worktree
- [ ] Known M2 backlog listed (session export, memory editor depth, server-side
      provider profile store, etc.)

---

## 7. M1 acceptance demo script

Run from a worktree after F0+F1 (F2 preferred):

1. Start `rove-api` and Web.
2. Open empty state.
3. Open a workspace by absolute path.
4. Create a session; send a task with fake or configured provider.
5. Observe streaming assistant output and tool cards.
6. Handle an approval inline.
7. Open Inspector and confirm plan/tools/approvals.
8. Send a **second turn in the same session**; confirm hard resume continuity.
9. Open a second session; confirm parallel/status presentation.
10. Settings → Providers: configure/test/list models without raw keys in browser.
11. Settings → About: connection/runtime visible.
12. Toggle theme.
13. Refresh: recents/pinned workspaces still available.

---

## 8. Explicit M2+ backlog (not M1)

- Full Settings section implementations beyond Providers/About
- Server-side provider profile persistence
- Rich session export/cleanup
- Memory management UI depth
- Native Desktop host (Tauri)
- Remote Gateway / device pairing
- File-tree IDE features, diff studio, MCP hub marketplace

---

## 9. Documentation obligations when implementing

When a wave lands user-visible behavior:

1. Update current docs that would otherwise lie (`README` product entry points,
   `apps/web/README`, API notes if contract changed).
2. Keep `docs/runtime/**` as implementation truth; add a short note only if
   workspace/resume semantics change.
3. Do not rewrite sealed design history silently; append changelog entries.

Archive is historical only; do not put the new product truth only in Archive.

---

## 10. Ready-to-start command sequence (human / next agent)

### Step 0 — commit docs on primary `main` (this checkout)

```powershell
cd D:\Study\project\agent\rove
git status --short
# expect the design/plan/README docs as the commit contents
```

### Step 1 — open only F0 after docs are on main

```powershell
cd D:\Study\project\agent\rove
git status --short
git branch --show-current
# expect clean main with design+plan present

git worktree add .worktrees/web-m1-foundation -b feature/web-m1-foundation main
cd .worktrees/web-m1-foundation
# implement F0 only
```

### Step 2 — after F0 merges, open F1

```powershell
cd D:\Study\project\agent\rove
git checkout main
git pull
git worktree add .worktrees/web-m1-shell -b feature/web-m1-shell main
cd .worktrees/web-m1-shell
# implement F1 only
```

Do **not** pre-create F1/F2 worktrees in parallel with F0 for merge-intended work.

---

## changelog

- 2026-07-25: Initial sealed delivery plan with explicit serial-wave / limited-parallel
  verdict. Restored onto post-W2a `main` baseline. Implementation not started in the
  primary checkout by this document alone.
