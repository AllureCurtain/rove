# Agent Desktop + Web Shared UI

> Status: **Partially implemented — Web M1/C0–C3 are on main; Desktop D0 is implemented on `program/full-delivery`; cross-platform evidence pending**
>
> Date: 2026-07-25
>
> Baseline commit expectation: current `main` at sealing time (cleanup W2a landed).
>
> Execution plan:
> [`../plans/2026-07-25-web-management-m1.md`](../plans/2026-07-25-web-management-m1.md)

This document freezes the product, information architecture, visual baseline, and
engineering constraints for rove's **shared product UI**, hosted first as a Web
management app and a Tauri desktop shell. The Web M1 shell and Web
Complete C0–C3 persistence, continuity, Settings, migration, polish, and local
acceptance are implemented and verified on `main`. The Tauri Desktop host
is implemented on the full-delivery branch with an embedded API, authenticated
static WebView transport, and Windows packaging/process evidence; macOS/Linux
packaging and full interactive WebView evidence remain unverified.

It does not define a second runtime or workspace kind. Current runtime truth
remains `docs/runtime/**` and the code.

---

## 1. Product decision

| Item | Decision |
|------|----------|
| Product | **Agent Desktop + Web management UI** |
| UI strategy | **One shared product UI, two hosts** |
| Delivery order | **(1) seal UI/spec → (2) Web M1 → (3) Web Complete → (4) Tauri Desktop** |
| Desktop shell | **Tauri 2** (after Web Complete; see `2026-07-26-web-complete-design.md`) |
| Remote Gateway / control plane | **Out of scope for now** |
| Old Web workbench | **Not the product line**; M1 primary entry becomes the new shell |

### Why

- Users want a normal installable agent product, not only a developer workbench.
- Web and Desktop must not fork into two UIs.
- Remote browser control of another machine (LiveAgent/OpenClaw gateway style) is a
  later class of work and is not required for “install and use locally”.

### Anti-goals

- Continuing the current workbench as the long-term product identity.
- Building a remote Gateway before a local product shell exists.
- Copying LiveAgent/Hermes full surface area in v1 (MCP hub marketplace, SSH,
  tunnels, cron suite, etc.).
- Auto-scanning the whole disk for projects.
- Defaulting agent execution onto the user home directory or entire disk.
- Making a debug timeline the default main surface.
- Implementing product waves on the primary `main` checkout instead of
  `.worktrees/*`.

---

## 2. Core concepts

```text
Workspace  = local directory (Folder | Repo | Task)
  └── Session = continuous conversation thread in that workspace
        └── Run   = one execution attempt (job/run in runtime terms)
```

| Concept | Meaning |
|---------|---------|
| **Workspace** | Local root path the agent may read/write/run in. Matches runtime `Workspace` (`Folder` / `Repo` / `Task`) plus `.rove/` state under that root. “Project” is only colloquial speech, not a third entity. |
| **Session** | Product conversation thread. Multi-turn continuity must be real runtime resume, not a cosmetic chat log. |
| **Run** | One concrete execution bound to a session (`job_id` / `run_id` today). |
| **Inspector** | Optional right rail for plan/tools/approvals/usage of the active/latest run. |

Hierarchy is only **Workspace → Session → Run**.

---

## 3. Information architecture

### Default home

**Chat**, not an ops dashboard.

### App shell

```text
┌──────────────────────┬──────────────────────────┬─────────────────┐
│ Workspaces           │ Chat                     │ Run Inspector   │
│  📁 <dir>            │ transcript + composer    │ collapsible     │
│     ├─ session       │ tool cards + approvals   │ plan/tools/     │
│     └─ session       │                          │ approvals/usage │
│  📁 <dir>            │                          │                 │
│  ⚙ Settings          │                          │                 │
└──────────────────────┴──────────────────────────┴─────────────────┘
```

### Settings shell

- Settings is a **full page**.
- Entering Settings **hides the workspace tree**.
- Settings has its own section nav + back-to-chat affordance.

### Workspace intake

- **Open Workspace…** (path on Web; native picker later on Desktop)
- Recents
- Pin/unpin
- No full-disk scan
- No mandatory pre-registration form before open

### Empty state

When no workspace is open:

- Open Workspace
- Recents (if any)
- Configure Provider (secondary)

### Chat interaction rules

- Main surface is a **chat transcript**, not an event log.
- Tool calls appear as **collapsible in-stream cards**.
- Approvals appear as **inline cards**, mirrored in Inspector.
- Multi-session **parallel runs** are allowed; sidebar must show running state.
- Chat may keep a few conversational shortcuts (for example current model).
  Full configuration lives in Settings.

### Settings sections (v1 product map)

1. General
2. Providers & Models
3. Tools & Approvals
4. Workspace / Paths
5. Memory
6. Sessions (advanced: cleanup/export)
7. Keyboard shortcuts
8. Advanced / Developer
9. About / Runtime

Web M1 depth rule:

- Full nav skeleton for all nine sections.
- Deep implementation first: **Providers & Models** and **About / Runtime**.
- Other sections may be explicit placeholders.

Benchmark UI leaves primary IA and lives under Advanced only.

---

## 4. Host parity

| Capability | Web M1 | Desktop (later) |
|------------|--------|-----------------|
| Chat + Sessions + Settings shell | Required | Same UI |
| Open workspace | Path input + recents/pin | Native directory picker + same model |
| Start/connect API | External process / dev scripts | Embedded/bootstrap runtime |
| Secrets | `api_key_env` only in browser path | May add local secret store behind same abstraction |
| Notifications | In-page | System notifications optional |
| Tray / updater | No | Optional later |

Shared UI must route host-only behavior through a thin `platform` adapter so Desktop
does not fork business pages.

---

## 5. Visual baseline

Design register: **product** (not brand/marketing).

| Dial | Value |
|------|-------|
| Soul | 6–7 refined tool with brand |
| Spectacle | 2–3 feedback-only motion |
| Density | 6–7 sidebar + chat + inspector |
| Theme | **Light-first**, dark supported |
| Accent | **Single desaturated ink/harbor blue** |

Rules:

- Neutral slate/stone surfaces; no pure `#000` / `#fff` as design tokens.
- One brand accent locked across the product.
- Green / amber / red are **semantic status only**, not brand accents.
- Banned defaults: AI purple neon, cinematic cyan+magenta, phosphor neon-green brand,
  dual Trust-SaaS blue+orange accents, emoji in UI chrome.
- Implementation craft uses product-ui discipline (tokens, states, density) rather
  than landing-page spectacle.

Exact hex/OKLCH values are locked at token implementation time; the family above is
already sealed.

---

## 6. Current system truth (constraints)

These facts shape M1 engineering:

| Fact | Implication |
|------|-------------|
| API already has jobs, SSE, approvals, inputs, cancel, runs, provider test/models | Chat main path can integrate against live API immediately |
| `CreateJobRequest.workspace` now supports explicit Folder/Repo/Task binding | Opened M1 paths are real execution roots, not cosmetic catalog entries |
| Lower-level hard resume remains workspace-store scoped; M1 originally used `resume: "latest"` | C0 added exact server-owned product-session/run binding, and C1 switched the default shell to it |
| Runtime `Session` remains thin (`SessionId` centered) | C0 keeps product catalog/mapping in API-global ProductStore and derives transcripts from canonical workspace runtime events |
| `RunSummary` already carries `session_id` / `job_id` / `run_id` | Useful mapping anchors exist |
| Current `apps/web` defaults to the sealed product shell and gates catalog boot on M1 migration | Keep the IA and fail-closed recovery path; `/dev/workbench` remains a bounded advanced escape hatch only |

---

## 7. Sealed engineering decisions

| ID | Decision |
|----|----------|
| **1B** | M1 includes backend support so an opened workspace path is the **real execution root** (`workspace_root` or equivalent Folder/Repo binding). Multi-workspace UI is not cosmetic-only. |
| **2A** | Same-session multi-turn is **hard resume**, comparable to Claude Code conversation resume. Soft fallback of “new job + frontend transcript stitch only” is **not acceptable** as the product path. If resume is insufficient, fix runtime/API. |
| **3A** | M1 includes light default + dark toggle with design tokens. |
| **4B** | New shell is the only primary Web entry in M1. Do not keep a long-lived dual main UI. |
| **5B** | Integrate against live `rove-api` from day one. Incremental UI is fine; fake-data-first product demos are not the strategy. |

### Non-negotiables for M1

1. Opened workspace path is the real execution root.
2. Session continue is durable resume continuity.
3. Providers + About are deep; other settings may scaffold.
4. Shared UI structure is hostable later by Tauri.
5. Benchmark is not primary IA.

---

## 8. Logical product model

```text
Workspace {
  id, rootPath, kind(folder|repo|task), displayName, pinned, lastOpenedAt
}

Session {
  id, workspaceId, title, createdAt, updatedAt, status,
  runtimeSessionId?, activeJobId?, activeRunId?
}

Run {
  jobId, runId, status, startedAt, finishedAt?, resumedFromRunId?
}
```

Rules:

- Many sessions per workspace.
- Many runs per session over time.
- UI focuses one active workspace + one active session.
- Other sessions may continue running in parallel.

---

## 9. Suggested routes (Web host)

```text
/                                 → last session or empty state
/workspaces                       → empty/recents when nothing active
/w/:workspaceId                   → workspace default session or empty chat
/w/:workspaceId/s/:sessionId      → chat + inspector
/settings                         → settings shell
/settings/:section                → settings section
```

Desktop later reuses the same logical screens inside the Tauri webview.

---

## 10. Implementation boundary with later Desktop

Implemented in the Web host:

- Shared shells, chat, bounded inspector, complete Settings/deep routes, and
  fail-closed M1 migration/recovery with exact idempotent retry
- Platform adapter seam
- API/runtime work for workspace root + hard resume
- Responsive, keyboard/focus, live-status, reduced-motion, theme, and visual
  acceptance polish

Do later (Desktop):

- Tauri packaging
- Native folder picker
- Embedded API/runtime bootstrap
- OS notifications / tray / updater as needed

Do not do now:

- Remote Gateway
- Multi-user hosted identity
- Full LiveAgent feature parity

---

## 11. References

- Runtime MVP boundary: [`../runtime/mvp-definition.md`](../runtime/mvp-definition.md)
- Current architecture: [`../runtime/architecture.md`](../runtime/architecture.md)
- Existing web host: `apps/web/`
- Existing API contracts: `apps/api/src/types.rs`, `apps/api/src/lib.rs`
- Sibling research context: PI / Hermes / OpenClaw / LiveAgent (local checkouts under
  `D:/Study/project/agent/`) for IA inspiration only

---

## changelog

- 2026-07-27: Integrated Web Complete C1–C3 through PRs #24, #25, and #26.
  Merge commits `db8f970`, `abbd7d6`, and `e3c2403` are on `main`; the
  post-merge Rust, Web, mock Playwright, and three-scenario `local-full` gates
  passed. Desktop D0 was not started and historical worktrees were retained.
- 2026-07-27: Marked Web Complete C0–C3 implemented and locally verified on the
  stacked branch. The default shell now runs fail-closed migration before
  catalog boot, retains durable deep routes, completes visual/accessibility
  polish, and passes the three-scenario local live-API gate. External-provider
  evidence was not run; coordinator integration and Desktop remain pending.
- 2026-07-27: Marked Web Complete C1 implemented. The default shell now uses
  API-authoritative product state, deep routes, transcript restore, exact
  product-session turns, focused observation, and persistent provider profiles;
  C2 Settings completeness and C3 migration/polish/live-API acceptance were
  completed later on the stacked branch.
- 2026-07-26: Marked Web Complete C0 implemented. API-global product state,
  exact product-session/runtime binding, transcript projection, strict browser
  migration, and typed Web client modules were present; default-shell wiring
  was still C1/C2 work at that point and was completed later on the stacked
  branch.
- 2026-07-25: Reconciled status after M1 landed. Marked the shared design
  partially implemented, updated the Folder/Repo and product-shell facts, and
  recorded exact product-session binding as Web Complete C0 work.
- 2026-07-25: Accepted and sealed after product grilling. Restored onto post-W2a
  `main` baseline for commit-before-worktree delivery. No implementation claimed.
- 2026-07-26: Delivery order updated — **Web Complete** inserted before Tauri
  Desktop. See `2026-07-26-web-complete-design.md`.
