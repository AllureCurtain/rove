# Web → Desktop Master Delivery Plan

> Status: **Active coordinator plan — P0 reconciled; C0 foundation next**
>
> Coordinator: the primary conversation working from the repository `main`
> checkout. Worker conversations implement bounded branches only; they do not
> merge integration branches or `main`.
>
> Baseline audited: `main` at `b244104`.
>
> Product decisions:
> [`../design/2026-07-25-agent-desktop-web-ui-design.md`](../design/2026-07-25-agent-desktop-web-ui-design.md)
> and
> [`../design/2026-07-26-web-complete-design.md`](../design/2026-07-26-web-complete-design.md).
>
> Detailed Web milestone plan:
> [`2026-07-26-web-complete.md`](2026-07-26-web-complete.md).

This is the coordinator-level source for delivery order, worktree ownership,
parallel boundaries, PR authority, and the handoff from Web Complete to a future
Tauri Desktop milestone. It does not replace current runtime truth in
`docs/runtime/**`, and it does not claim that Web Complete or Desktop is already
implemented.

---

## 1. Outcome and fixed order

The product sequence is:

```text
P0 documentation and contract reconciliation
  → Web C0 product persistence/session foundation
  → Web C1 continuity + C2 settings/platform work
  → Web C3 UI completion and acceptance
  → Web Complete on main
  → Desktop D0 design and bootstrap contract
  → Desktop implementation and packaging
```

The Web information architecture is already sealed and must not be redesigned:

```text
Workspace → Session → Run

App:      Workspace/session rail | Chat | collapsible Inspector
Settings: full page with its own section navigation
```

The remaining Web work is continuity, durable product state, usable Settings,
deep routing, session observation, recovery states, accessibility, responsive
behavior, and a final visual/acceptance seal. Desktop starts only after this
shared UI is complete enough to host without forking the product.

---

## 2. Audited baseline

### 2.1 Implemented on `main`

| Delivery | Evidence | Result |
|---|---|---|
| Cleanup W1 | `8ffb291` | Product provider vocabulary and single assembly path |
| Cleanup W2a | `d13646a` | Tools/events/policy/assembly cleanup |
| Cleanup W2b | `9847fdd` | Runtime domain regroup |
| Cleanup W3 | `a3f2681` | First-class bounded `search_code` |
| Web M1 F0 | `46b945d` | Explicit Folder/Repo execution root and fail-closed resume |
| Web M1 F1 | `ecfabbd` | Product shell on the live API |
| Web M1 F2 | `93a724c` | Theme, Inspector states, running badges, Advanced-only benchmark |
| Web Complete seal | `b244104` | Accepted C0–C3 product scope |

The default Web route is the product shell. `/dev/workbench` is an advanced
escape hatch, not a second product line.

### 2.2 Web gaps still open

- refresh does not rebuild the active session transcript;
- product Workspace/Session catalogs and provider profiles still treat browser
  storage as authority;
- `resume: "latest"` is workspace-scoped and is ambiguous when one workspace
  contains multiple product sessions;
- switching sessions does not fully reattach/rebuild the correct running state;
- durable workspace/session/settings routes do not exist;
- several Settings sections are placeholder-only;
- loading/empty/partial/error/recovery states, keyboard/focus behavior,
  reduced-motion behavior, and responsive layout need a completion pass;
- the default product shell has mock-backed browser coverage, while the current
  three-case real-API suite targets `/dev/workbench`; the provider runner's Web
  step also still uses pre-M1 selectors;
- there is no committed high-fidelity screenshot baseline for final acceptance.

### 2.3 Desktop is not implementation-ready

There is no `apps/desktop`, Tauri dependency, `src-tauri`, packaging pipeline, or
sealed D0 plan. The accepted direction is only:

```text
same shared product UI
  + Tauri 2 host
  + native folder picker
  + embedded/local runtime bootstrap
```

`docs/runtime/desktop-workspace-spec.md` describes a future agent-controlled
Desktop automation workspace. It is not the Tauri product shell design.

---

## 3. Coordinator and PR governance

### 3.1 Authority

Only the primary conversation may:

- create or retire delivery worktrees;
- choose the common foundation SHA;
- change coordinator-owned contract files;
- merge worker PRs into an integration branch;
- merge an integration PR into `main`;
- declare a wave or product milestone complete.

Worker conversations may edit only their assigned worktree and allowed files.
They commit and push their branch, open a PR to the named integration branch,
and hand back evidence. They never merge the PR, rebase another worker, or
modify the primary checkout.

### 3.2 Branch flow

```text
main
  └─ integration branch at a coordinator-sealed foundation SHA
       ├─ worker branch A ── PR ──┐
       ├─ worker branch B ── PR ──┼─> coordinator merges to integration
       └─ worker branch C ── PR ──┘

integration branch ── coordinator PR/review ──> main
```

Every merge is represented by a GitHub PR. Direct worker merges and direct
worker pushes to `main` are forbidden. The coordinator may use a local merge
only to resolve/test integration, but the final recorded merge still goes
through the corresponding PR.

### 3.3 Safe parallelism rule

Parallel branches are allowed only when all are true:

1. they start from the same sealed foundation SHA;
2. their contracts are already committed;
3. their allowed-file sets are disjoint or explicitly coordinator-owned;
4. one branch can be reviewed without assuming uncommitted work from another;
5. the coordinator controls integration order and conflict resolution.

Parallelism is not used merely to split a list. Shared routers, public wire
types, OpenAPI registration, package manifests, shell roots, and runtime event
contracts stay with the coordinator unless a later foundation explicitly
partitions them.

---

## 4. C0 contract foundation

C0 establishes one product-control plane without creating a second execution
runtime or event lifecycle.

### 4.1 ProductStore ownership

`ProductStore` is API application-global state. It is not stored separately in
each opened execution workspace.

Initial Web host rule:

```text
API bootstrap config/state root
  └─ product.sqlite             # product catalog/settings/mappings

opened Workspace root
  └─ configured runtime state   # trace/task_state/report/state.sqlite
```

The first implementation resolves `product.sqlite` from the API bootstrap
state directory. Browser input cannot select this path. Desktop D0 may inject a
stable OS application-data directory through bootstrap configuration, without
changing the ProductStore contract.

ProductStore owns:

- known workspaces and safe display metadata;
- product sessions and their runtime bindings;
- ordered session-to-run mappings;
- provider profiles containing secret references only;
- explicitly safe product preferences;
- schema and browser-migration receipts.

Runtime trace, task state, reports, tool artifacts, and canonical events remain
in the selected workspace's runtime store. ProductStore must not duplicate
those event facts as a second source of truth.

### 4.2 Product session to runtime identity

The server owns the product session ID. A product session is bound to exactly
one workspace and tracks:

```text
product_session_id
  → workspace_id + canonical root/kind
  → runtime_session_id?         # set after first run starts
  → latest_job_id?
  → latest_run_id?
  → ordered run bindings
```

The product Web path adds an optional `product_session_id` to `POST /jobs` (or
an equivalent additive server-owned binding finalized in C0). Its rules are:

1. First turn: the product session has no runtime binding, so the server starts
   fresh and records the returned runtime session/job/run identity.
2. Later turn: the server resolves the product session's exact
   `latest_run_id` and resumes that run in the same workspace store.
3. A client-supplied resume key that conflicts with the server binding is
   rejected. The product path never guesses using workspace-global `latest`.
4. Workspace root/kind mismatch is rejected.
5. Only one active turn is permitted per product session; concurrent creation
   for the same session returns a typed conflict. Different sessions may run in
   parallel.
6. Missing/corrupt/stale runtime state is a typed fail-closed error. It never
   creates a disconnected turn and labels it continuation.

The existing lower-level `resume` field remains available to non-product API
consumers. M1 browser data may use `latest` only while being migrated and only
when the mapping is unambiguous; after binding, all product continuation is
exact-session/exact-run.

### 4.3 Transcript read projection

The transcript is rebuilt as a read projection:

```text
product session
  → ordered run bindings
  → each workspace StateStore
  → canonical indexed StreamEvent rows
  → optional task/report fallback metadata
  → ordered read response
```

The response carries run segments and canonical sequenced events rather than a
new writable chat protocol. `run_started.user_message`, model message/chunk,
tool, approval/input, plan, and completion events remain the facts the Web
already knows how to project.

The response must include:

- `complete` or `partial` status;
- typed partial reasons such as missing run mapping, missing event range,
  corrupt artifact, or cleaned history;
- stable product/runtime identities needed for routing and observation;
- deterministic ordering by session run ordinal, then event sequence.

An empty event list is not silently called a complete transcript when mapped
runs exist. Reports may provide bounded fallback summaries, but a derived
report must not replace canonical events as durable truth.

### 4.4 ProductStore schema and compatibility

The initial schema is additive and versioned. Logical tables are:

| Table | Purpose |
|---|---|
| `product_schema_migrations` | Applied schema versions |
| `product_workspaces` | Canonical root/kind, display metadata, pin/recency |
| `product_sessions` | Product identity, workspace, title/status, exact latest runtime binding |
| `product_session_runs` | Immutable ordered mapping to runtime session/job/run IDs |
| `product_provider_profiles` | Type/base/model/key-env reference; never raw key |
| `product_preferences` | Known safe preference fields only |
| `product_migration_receipts` | Idempotent client migration keys/results |

Serialized/public additions require defaults and bounded validation. Deleting a
product catalog entry does not implicitly delete workspace files or runtime
artifacts. Destructive runtime cleanup remains an explicit, separately
authorized action.

### 4.5 M1 browser migration

Migration is versioned and idempotent:

1. Client reads known M1 local-storage schema versions.
2. Client constructs a sanitized payload of workspaces, sessions, profiles, and
   safe preferences. Unknown fields and raw-key-shaped fields are not sent.
3. Server validates bounds, canonicalizes workspace roots, upserts by stable
   identity, and records an idempotency receipt.
4. Client marks migration complete only after a successful server response.
5. Failure leaves browser data intact and offers retry; it never reports
   success after a partial network failure.

Raw provider keys are rejected even if an old browser payload unexpectedly
contains them. `api_key_env` or an equally safe future secret reference is the
only accepted credential field.

### 4.6 Coordinator-owned hotspots

During C0, the primary conversation owns:

- `apps/api/src/lib.rs`;
- `apps/api/src/types.rs`;
- `apps/api/src/docs.rs`;
- API/public OpenAPI registration;
- `Cargo.toml`, crate manifests, and lockfiles;
- shared ProductStore traits/public contract modules;
- `docs/runtime/**`;
- changes to existing runtime event or serialized-state contracts.

Workers must report a needed hotspot change instead of editing it unless the
coordinator explicitly amends their allowed-file set before work begins.

---

## 5. C0 worktree topology

### 5.1 Foundation and integration

| Role | Directory | Branch |
|---|---|---|
| C0 integration | `.worktrees/web-complete-persistence` | `feature/web-complete-persistence` |
| Store worker | `.worktrees/web-c0-store` | `feature/web-c0-store` |
| Transcript worker | `.worktrees/web-c0-transcript` | `feature/web-c0-transcript` |
| Web client worker | `.worktrees/web-c0-client` | `feature/web-c0-client` |

The coordinator first updates the integration branch to the P0-merged `main`,
implements the public foundation and commits it, then creates all three worker
worktrees from that exact commit.

### 5.2 Worker ownership

The final startup prompts will contain an exact commit and file list. Intended
ownership is:

| Lane | Allowed implementation surface | Deliverable |
|---|---|---|
| Store | New internal ProductStore schema/repository files and colocated tests | SQLite migrations, validated CRUD, migration receipts |
| Transcript | New internal transcript projection files and colocated tests | Ordered canonical-event projection with typed partial reasons |
| Web client | New product API types/client/migration modules and tests | Thin typed client, idempotent migration state machine; no shell UI |

No worker edits `ProductApp`, `SettingsShell`, API router/type/OpenAPI central
files, manifests, runtime events, or current runtime docs in this wave.

### 5.3 C0 integration order

The coordinator reviews and merges in this order unless dependency evidence
requires a recorded change:

1. ProductStore worker PR;
2. transcript worker PR;
3. Web client worker PR;
4. coordinator wiring, API/OpenAPI/integration tests, security review, and
   current runtime documentation;
5. integration PR from `feature/web-complete-persistence` to `main`.

---

## 6. Work after C0

### 6.1 Parallel wave A

Once C0 is on `main`, two bounded branches may run in parallel:

| Lane | Scope |
|---|---|
| C1 continuity/routing | Deep routes, boot restore, partial/error UI, focused SSE reattach/rebuild, exact product-session turn creation |
| C2 platform/settings API | Backend APIs for safe preferences, policy controls, workspace/session management, and memory views needed by Settings |

The C1 lane owns Web shell/chat/routing. The C2 lane owns new backend modules and
thin client contracts; it does not edit `SettingsShell` during this parallel
wave.

### 6.2 Settings partition

After C1 shell integration, the coordinator first splits `SettingsShell` into
stable section modules and commits that foundation. Two UI workers may then run
in parallel:

| Lane | Sections |
|---|---|
| Settings catalog | General, Providers, Workspace/Paths, Sessions |
| Settings runtime | Tools/Approvals, Memory, Keyboard, Advanced, About |

Shared navigation, route ownership, product tokens, and common form primitives
remain coordinator-owned.

### 6.3 C3 completion seal

C3 is integrated after all functional lanes. It owns:

- migration/recovery polish;
- responsive layout and narrow viewport behavior;
- keyboard navigation, visible focus, shortcut conflict handling;
- reduced motion and motion-duration consistency;
- complete empty/loading/error/partial/success states;
- the Inspector as a restrained execution/evidence spine, without turning Chat
  into a debug console;
- screenshot baselines for representative light/dark, empty/running/partial,
  Settings, and narrow layouts;
- replacing the `/dev/workbench`-only real-API harness and stale provider-runner
  selectors with live-API product-shell coverage while retaining bounded
  advanced-surface coverage;
- the Web Complete acceptance script and final documentation truth.

The existing information architecture and single harbor/ink accent remain
sealed. C3 improves craft and identity; it does not start another redesign.

---

## 7. Desktop D0 gate

A docs-only Desktop D0 branch may research in parallel late in C3, but Desktop
implementation cannot merge before Web Complete. D0 must decide:

1. target operating systems and first release platform;
2. Tauri 2 application/package structure;
3. how shared Next UI is built for a webview without depending on the Next
   server proxy at runtime;
4. embedded `rove-api`/runtime bootstrap, port/lifecycle/readiness semantics;
5. stable OS application-data, workspace state, and log locations;
6. native folder picker contract through the existing platform adapter;
7. secret storage boundary and migration from `api_key_env`-only Web behavior;
8. background-run, close/quit, cancellation, crash, and restart behavior;
9. installer/signing/notarization/update scope;
10. security boundaries for navigation, custom protocols, local HTTP, CSP, and
    exposed Tauri commands;
11. deterministic local development and packaging verification.

Desktop reuses ProductStore, routes, UI state, canonical events, and exact
product-session bindings. It must not introduce a second agent loop, a second
chat protocol, or Desktop-only session truth.

---

## 8. Worker startup and handoff contract

Before asking the user to open a worker conversation, the coordinator provides:

- exact absolute worktree directory;
- branch and expected foundation SHA;
- PR base branch;
- goal and non-goals;
- allowed files and forbidden hotspots;
- required source documents;
- verification expectations;
- a ready-to-copy opening prompt.

Every worker handoff must return:

```text
Outcome
- implemented contract

Changed
- exact files

Verified
- exact focused checks and results

Not run
- checks and reason

PR
- commit, pushed branch, PR URL targeting the named integration branch

Notes
- compatibility, risks, requested coordinator-owned changes, git status
```

The worker stops after opening the PR. “PR opened” is not permission to merge.

---

## 9. Completion criteria

Web Complete is done only when:

- the API-backed product store is authoritative;
- multiple product sessions in one workspace resume their own exact runtime
  chain;
- refresh and route entry rebuild a complete or honestly partial transcript;
- session switching and live observation do not corrupt another session;
- every Settings section has a real, bounded capability;
- provider profiles survive browser storage clearing without exposing secrets;
- deep links, recovery states, responsive behavior, keyboard/focus, and visual
  acceptance are complete;
- product/API/runtime documentation matches the implementation;
- the integration PR is reviewed and merged by the primary conversation.

Desktop implementation begins only after a sealed D0 design and plan exist on
`main`. Desktop is complete only when the shared UI runs through Tauri with a
native workspace picker, safe embedded runtime lifecycle, stable product data,
and documented packaging/security behavior.

---

## changelog

- 2026-07-26: Completed P0 reconciliation. Distinguished mock product-shell
  browser coverage from `/dev/workbench` real-API evidence, recorded the stale
  provider-runner Web step as C3 work, and removed obsolete current-state RAG
  gate claims.
- 2026-07-25: Created after a full current-doc/code/history audit. Recorded the
  primary-conversation-only merge rule, exact product-session/runtime binding,
  canonical-event transcript projection, API-global ProductStore, bounded C0
  worker lanes, post-C0 parallel waves, C3 UI seal, and Desktop D0 gate.
