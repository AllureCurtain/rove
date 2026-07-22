# Rove Modular Workspace Migration Plan - 2026-07-22

> Status: **In Progress / Phase 6 Bootstrap + Bench Extracted**
>
> Design source:
> [`../design/2026-07-22-modular-workspace-architecture.md`](../design/2026-07-22-modular-workspace-architecture.md)
>
> Current runtime source of truth:
> [`../runtime/README.md`](../runtime/README.md)

## 1. Objective

Migrate the current single-package Rove repository into a modular Cargo
Workspace without replacing the runtime, dropping behavior, or creating a
second product implementation.

The final canonical repository remains named `rove` and has this shape:

```text
rove/
├── Cargo.toml
├── Cargo.lock
├── models/
├── core/
├── runtime/
├── apps/
│   ├── bootstrap/
│   ├── cli/
│   ├── api/
│   ├── bench/
│   └── web/
├── tests/
├── docs/
├── scripts/
├── prompts/
└── benchmarks/
```

The root `Cargo.toml` is a virtual Workspace manifest at completion. The old
root `src/` package exists only during the compatibility window and is removed
after all binaries and tests consume the extracted crates.

This plan migrates existing products. It does not create an empty
`apps/desktop/` directory or implement a Tauri application.

## 2. Mandatory Invariants

The migration is complete only if all of these remain true throughout the
work:

1. CLI, API, Web, and benchmark execution continue to share one runtime.
2. Provider-specific payloads remain behind the model boundary.
3. Tool calls continue through the registry, safety, approval, input, and
   destructive-ordering boundaries.
4. Workspace paths remain bounded by the resolved workspace.
5. Runtime `StreamEvent` remains the canonical persistence, API, Web, terminal,
   and report lifecycle contract.
6. `trace.jsonl`, `task_state.json`, `report.json`, and SQLite retain their
   current fact/projection/index relationship.
7. Resume never replays completed work or an unknown in-flight side effect.
8. Existing serialized artifacts remain readable unless a separately tested
   migration explicitly changes their schema.
9. The fake-provider path and deterministic benchmarks remain network-free.
10. No secret enters manifests, fixtures, trace, report, API output, Web output,
    or migration evidence.

Directory movement is not evidence that a boundary has been implemented. Each
crate must satisfy its dependency and embedding tests before the old module is
removed.

## 3. Migration Operating Model

Use one Git repository with two physical worktrees while the migration is
active:

```text
rove/             # migration branch and final canonical directory
rove-baseline/    # temporary read-only baseline worktree
```

The baseline worktree must point at a committed, fully verified lifecycle
baseline. It is used for source comparison and deterministic behavior checks;
it is not a second maintained implementation.

Rules:

- finish and verify the current in-flight execution-lifecycle slice before
  creating the migration baseline;
- keep the migration in reviewable commits with a green gate after each
  extraction;
- move behavior-preserving modules before rewriting them;
- rewrite only the boundaries that must change to remove a dependency cycle;
- do not copy a module into two long-lived implementations;
- freeze unrelated architecture work during the physical extraction window;
- apply urgent fixes to the baseline and migration branch deliberately rather
  than allowing silent divergence;
- remove the temporary baseline worktree only after final acceptance and
  explicit approval.

## 4. Final Packages And Dependencies

All packages initially use the Workspace version and edition in lockstep.
Independent crate versioning is deferred until the public APIs stabilize.

| Directory | Package | Responsibility | Allowed local dependencies |
|---|---|---|---|
| `models/` | `rove-models` | Model protocol, clients, provider normalization | none |
| `core/` | `rove-core` | In-memory Agent and tool loop | `rove-models` |
| `runtime/` | `rove-runtime` | Persistent Rove execution semantics | `rove-models`, `rove-core` |
| `apps/bootstrap/` | `rove-app-bootstrap` | First-party config loading and product assembly | `rove-models`, `rove-core`, `rove-runtime` |
| `apps/bench/` | `rove-bench` | Benchmark library and binary | lower layers, bootstrap when needed |
| `apps/api/` | `rove-api` | HTTP/SSE/OpenAPI application | lower layers, bootstrap, bench |
| `apps/cli/` | `rove-cli` | `rove` and `rove-index` binaries, REPL, terminal, TUI | lower layers, bootstrap |
| `tests/` | `rove-integration-tests` | Cross-package contracts only | packages under test |

`apps/web/` remains a pnpm/Next.js project and is not a Cargo member.

The enforced project dependency direction is:

```text
rove-models <- rove-core <- rove-runtime <- first-party apps
```

An app may directly use a lower-layer public type for composition or testing,
but no lower layer may depend on an app. The API-to-benchmark dependency is an
explicit same-layer dependency because the API exposes benchmark endpoints;
`rove-bench` must not depend on `rove-api`.

## 5. Ownership Decisions

### 5.1 Model protocol

`rove-models` owns:

- provider-neutral `Message`, `Role`, `ToolCallRef`, and `Usage`;
- the model-visible tool input schema;
- `ModelClient`, `ModelClientId`, `ModelEvent`, and `ModelError`;
- OpenAI-compatible, OpenAI Responses, Anthropic, Ollama, and Fake clients;
- provider stream parsing and provider error normalization;
- generic routing and health primitives that require no runtime configuration.

Provider constructors accept model-layer option structs. They do not read
`AppConfig`, workspace files, CLI arguments, or API request types.

### 5.2 Minimal Agent core

`rove-core` owns:

- the in-memory `Agent` loop and model/tool turn mechanics;
- `Action`, tool-call correlation, and an Agent-level stop outcome;
- `Tool`, `ToolOutput`, `ToolRegistry`, and operational tool descriptors;
- runtime-neutral before/after tool policy hooks;
- in-memory model-turn, tool-turn, cancellation, steering, and follow-up
  control;
- `AgentEvent` and core budget accounting.

The core tool execution context contains only invocation-scoped data such as
call identity and cancellation. Workspace, memory paths, approval providers,
and input providers are injected into runtime tools or runtime policy adapters;
they are not fields on the minimal core context.

`rove-core` must compile and run an in-memory test using only Fake Model plus a
custom Tool. That test must not create `.rove/`, SQLite, an HTTP server, or a UI.

### 5.3 Persistent runtime

`rove-runtime` owns:

- `RunCoordinator` and the stable first-party runtime facade;
- `RunId`, `JobId`, `SessionId`, `RunRequest`, and resumable task state;
- workspace detection and path-boundary enforcement;
- execution strategy, Planner, StepRunner, Evaluator, Finalizer, plan revision,
  ledger, and runtime budgets as they become implemented;
- approval, request-input, mutation ordering, and runtime policy adapters;
- runtime context assembly, memory injection, compaction, and prompt metadata;
- runtime identity, state artifacts, SQLite index, repair, cleanup, and resume;
- session/durable memory, MCP, optional RAG, and official built-in tools;
- canonical durable `StreamEvent` and projections into trace/state/report.

The current `Engine` becomes an internal compatibility facade and then
converges on `RunCoordinator`. Apps must not construct their own Agent loop.

### 5.4 First-party app bootstrap

`rove-app-bootstrap` owns product composition shared by CLI and API:

- `.rove/config.toml`, environment, and explicit override loading;
- the complete first-party config document and redacted config view;
- provider selection and construction from model-layer clients;
- conversion from product config into runtime builder options;
- shared first-party defaults without terminal, Axum, or Web rendering.

This package prevents `rove-runtime` from owning `ApiConfig` or `WebConfig` and
prevents `rove-models` from reading the current monolithic `AppConfig`.

### 5.5 Apps

- `rove-cli` owns CLI arguments, config rendering, one-shot execution, REPL,
  sessions/state commands, terminal actions, full-screen TUI, and the RAG index
  binary entry.
- `rove-api` owns Axum handlers, process-local HTTP job handles, SSE transport,
  OpenAPI, bearer auth, CORS, and rate limiting.
- `rove-bench` owns benchmark schemas, checks, evidence rendering, built-in
  suites, deterministic runner, and the `rove-bench` binary.
- `apps/web` owns browser state, API proxying, React components, and browser
  tests. It never embeds a Rust Agent loop.

### 5.6 Errors

Errors are split by authority:

```text
rove_models::ModelError
rove_core::ToolError / AgentError
rove_runtime::RuntimeError
rove_api::ApiError
CLI boundary -> anyhow with typed sources preserved
```

A temporary root `rove::errors` module may re-export these during migration.

### 5.7 Event composition

The event pipeline is fixed as:

```text
provider payload
  -> rove_models::ModelEvent
  -> rove_core::AgentEvent
  -> rove_runtime::StreamEvent
```

`AgentEvent` is an in-memory embedding contract. `StreamEvent` is the only
durable and externally consumed lifecycle. Runtime translates core events
synchronously so ordering is preserved. Apps never persist `AgentEvent`
directly and do not invent a parallel lifecycle.

## 6. Compatibility Contract

### 6.1 Non-negotiable compatibility

The migration must preserve:

- CLI binary name `rove`, commands, flags, exit behavior, approval/input
  safety, and fake-model flow;
- API routes, validation, authentication, CORS, rate limits, OpenAPI, and SSE
  JSON shapes;
- Web event handling and visible job lifecycle;
- trace event names and ordering;
- persisted task-state/report defaults and old-artifact deserialization;
- SQLite migration, repair, cleanup, and historical replay;
- workspace selection and path safety;
- provider payload normalization and tool-call ID correlation;
- default and RAG feature behavior.

### 6.2 Rust source compatibility

The repository is pre-1.0 and currently has no declared publication contract.
Use the root package as a temporary source-compatibility facade while crates are
extracted. Internal tests and binaries migrate to the new crate names before
the facade is removed.

If evidence appears that an external Rust project depends on the package
`rove`, stop facade removal and record a separate compatibility decision. In
that case a permanent re-export package may be required.

### 6.3 Binary and developer commands

Final binary/package mapping:

```text
cargo run                         # default member: rove-cli, binary: rove
cargo run -p rove-api             # binary: rove-api
cargo run -p rove-bench -- ...    # binary: rove-bench
cargo run -p rove-cli --features rag --bin rove-index -- ...
cargo install --path apps/cli     # installs the user-facing Rove CLI
```

Root-wide development gates use `--workspace`; they must not accidentally test
only the CLI default member.

### 6.4 Feature forwarding

- `rove-runtime` owns `rag` and its optional heavy dependencies.
- `rove-app-bootstrap`, `rove-cli`, `rove-api`, and integration tests expose a
  forwarding `rag` feature only where needed.
- default builds retain the disabled-RAG tool schema and typed error behavior.
- `rove-index` keeps `required-features = ["rag"]` in the CLI package.

## 7. Test Ownership After Migration

Move tests to the package that owns the contract. Keep only cross-package
contracts in the root `tests/` package.

| Current test area | Final owner |
|---|---|
| Provider payloads, model routing primitives, Fake Model | `models/tests/` |
| Pure Agent/tool loop and custom Tool embedding | `core/tests/` |
| Planning, state, resume, memory, tools, MCP, RAG, safety | `runtime/tests/` |
| API routes, SSE, OpenAPI, auth/CORS/rate-limit | `apps/api/tests/` |
| CLI parsing, REPL, sessions, TUI, `CARGO_BIN_EXE_rove` | `apps/cli/tests/` |
| Benchmark schemas, runner, evidence | `apps/bench/tests/` |
| Cross-surface event/artifact/compatibility contracts | `tests/tests/` |

Split the current large `tests/e2e.rs` by ownership instead of moving the whole
file into one package. Preserve test names when practical so historical failure
evidence remains searchable.

Before physical extraction, add or retain golden compatibility coverage for:

- old `trace.jsonl` event lines;
- old `task_state.json` with missing additive fields;
- old `report.json` with missing additive fields;
- stable API event JSON;
- completed-step and unknown-side-effect resume behavior;
- CLI fake-model smoke and binary exit codes.

## 8. Implementation Phases

### Phase 0: Establish The Lifecycle Baseline

Goal: create a committed, repeatable starting point before paths change.

Tasks:

1. Finish the current PlanEvaluator/PlanRevision lifecycle slice without
   extending it into unrelated future lifecycle work.
2. Confirm canonical event ordering, task-state/report projection, API/Web
   consumers, and backward defaults.
3. Update current runtime documentation for exactly the implemented slice.
4. Run focused lifecycle, API, Web, default Rust, RAG compile, and diff gates.
5. Record the verified commit as the migration baseline.
6. Create the temporary read-only baseline worktree and migration branch.

Do not start crate extraction from an uncommitted or failing baseline.

### Phase 1: Lock Contracts Before Moving Files

Goal: make regression detection stronger than directory churn.

Tasks:

1. Add the minimal external-style Fake Model + custom Tool contract test.
2. Add old-artifact compatibility fixtures where existing tests construct only
   current values.
3. Add an architecture test that will later inspect `cargo metadata` local
   dependencies.
4. Define the public runtime facade used by CLI, API, and benchmark paths.
5. Inventory serde/OpenAPI names and reject accidental schema churn.
6. Record current CLI/API/benchmark deterministic smoke outputs.

### Phase 2: Add A Transitional Cargo Workspace

Goal: introduce Workspace mechanics without changing runtime ownership.

Tasks:

1. Keep the existing root `[package]` and add `[workspace]` with resolver `3`.
2. Add `[workspace.package]`, `[workspace.dependencies]`, and shared lint/profile
   configuration without changing dependency versions.
3. Keep the root package as the default member during extraction.
4. Verify `cargo run`, every existing binary, every test target, default build,
   and RAG build behave as before.

### Phase 3: Extract `rove-models`

Goal: make the lowest project layer independent.

Tasks:

1. Move provider-neutral message, usage, model-visible tool schema, and model
   errors from the current core/root modules.
2. Move `ModelClient`, `ModelEvent`, provider clients, Fake Model, provider
   health, and generic routing primitives.
3. Replace provider dependencies on `AppConfig` with model-layer constructor
   options.
4. Move product provider selection/factory logic to app bootstrap.
5. Re-export new types through old root paths during the compatibility window.
6. Add a metadata assertion that `rove-models` has no local project dependency.

Exit gate:

```text
rove-models -X-> rove-core / rove-runtime / apps
```

### Phase 4: Extract `rove-core`

Goal: produce a genuinely embeddable in-memory Agent harness.

Tasks:

1. Move Tool trait/output/registry into core and leave official Tool
   implementations in runtime.
2. Split model-visible tool schema from operational descriptor and policy
   metadata without changing provider request payloads.
3. Reduce Tool invocation context to call-scoped, runtime-neutral data.
4. Replace direct core dependencies on workspace, memory, approval, input,
   state, and post-run hooks with injected policy/context adapters.
5. Move model-turn, parser, action, tool-turn, and run-level ReAct mechanics.
6. Introduce `AgentEvent`; keep durable event translation in runtime.
7. Run the external-style embedding test without filesystem state.

Exit gate:

```text
rove-core local dependencies == { rove-models }
rove-core dependency tree excludes rusqlite, axum, clap, ratatui, lancedb
```

### Phase 5: Extract `rove-runtime`

Goal: move all persistent and product-level execution semantics behind one
facade.

Recommended internal order:

1. IDs, workspace, boundary, runtime identity, and runtime configuration.
2. State store, trace, artifacts, report, SQLite index, repair, and resume.
3. Memory and compaction services.
4. Official tools, approval/input adapters, MCP, and optional RAG.
5. Planning, StepRunner, evaluator, execution policy, and coordinator.
6. Durable `StreamEvent` translation and runtime facade.
7. Existing `Engine` compatibility re-exports.

Current verified progress:

- Order item 1 is implemented for IDs, resumable task/checkpoint and execution
  contracts, Workspace/path safety, prompt metadata/runtime identity, and the
  approval/input provider contracts needed by those boundaries.
- Order item 2 is implemented: canonical `StreamEvent`, StateStore, trace,
  task/report artifacts, SQLite index, repair, cleanup, and resume now live in
  `rove-runtime`.
- Order item 3 is implemented: token-aware context construction, structured
  model/deterministic compaction, memory paths, session storage, durable recall,
  and layered prompt memory now live in `rove-runtime`.
- Order item 4 is partially implemented: local echo, filesystem, shell, memory,
  and request-input tools, runtime invocation adapters, the existing
  stdio/legacy-SSE MCP proxy, the tool `Executor` pipeline, and pre/post-tool
  plus post-run hooks (including the session-summary hook) now live in
  `rove-runtime`. Product registry assembly remains root-owned. Optional RAG is
  explicitly frozen for a later user-led refactor; its full feature test suite is
  not part of this migration gate.
- Order items 5–7 are implemented for the remaining durable coordination surface:
  planner, plan evaluator, plan loop, step runner, unplanned run loop, tool turn,
  durable `AgentEvent -> StreamEvent` translation, session helper, and the
  persistent `Engine`/`RunStream` facade now live in `rove-runtime`. Root
  `rove::core::{engine,planner,session,executor}` and related paths remain
  compatibility re-exports. Product registry assembly, optional RAG, and
  first-party `AppConfig` remain root-owned until Phase 6.
- The root `rove::core::*`, `rove::state::*`, `rove::memory::*`, and
  `rove::hooks::*` paths remain compatibility re-exports. The corresponding
  local `rove::tools::*` paths are also compatibility re-exports for product
  registry assembly and optional RAG.
- Full first-party `AppConfig` remains in the root package because API/Web and
  provider assembly fields must be separated through `apps/bootstrap`; it was
  not pulled into `rove-runtime` merely to complete a directory move.
- Default and RAG Workspace tests, strict Clippy, old-artifact compatibility,
  path/tool safety, state/repair/resume E2E, API restart/SSE, event-name, and
  canonical custom-tool input lifecycle tests pass for the verified slices.
  Runtime-owned context order, token metadata, structured compaction,
  deterministic fallback/circuit behavior, session memory, durable recall, and
  memory-tool safety are covered by package and root compatibility tests.

At each sub-step, preserve serde defaults and run focused state/resume/safety
tests. Do not change artifact schema merely to simplify module movement.

Exit gate:

```text
rove-runtime local dependencies == { rove-models, rove-core }
rove-runtime -X-> apps
```

### Phase 6: Extract First-Party Rust Apps

Current verified progress:

- Order item 1 (`rove-app-bootstrap`) is implemented for product config loading,
  provider factory construction, non-RAG product tool registry assembly, and
  shared first-party Engine assembly helpers. Root `src/config.rs`,
  `src/models/factory.rs`, and interface assembly remain compatibility wrappers.
  Optional RAG tool registration still happens in the root product registry so
  bootstrap does not depend on the deferred RAG modules.
- Order item 2 (`rove-bench`) is implemented: deterministic suite schema,
  checks, evidence, runner, and the `rove-bench` binary live in `apps/bench`.
  Root `src/bench` remains a compatibility re-export for API/tests during the
  remaining Phase 6 window.


Goal: make each user-facing Rust surface a thin consumer of runtime.

Order:

1. `rove-app-bootstrap`: config loading, redaction, provider/runtime assembly.
2. `rove-bench`: reusable benchmark library and `rove-bench` binary.
3. `rove-api`: reusable server library and thin `rove-api` binary.
4. `rove-cli`: CLI library plus `rove` and feature-gated `rove-index` binaries.

Requirements:

- CLI/API/benchmark use the same runtime builder and event contract;
- API benchmark endpoints call the benchmark library without introducing an
  API-to-CLI dependency;
- TUI and terminal rendering stay in CLI;
- auth, CORS, rate limiting, OpenAPI, and SSE stay in API;
- app shutdown or client disconnection does not redefine durable runtime state.

### Phase 7: Rehome Tests And Remove The Root Package

Goal: make package ownership enforce the architecture.

Tasks:

1. Move unit and focused integration tests to their owning packages.
2. Create `rove-integration-tests` for cross-package contracts.
3. Move CLI binary tests into `rove-cli` so `CARGO_BIN_EXE_rove` remains valid.
4. Update code-hygiene root detection for the test package manifest location.
5. Remove compatibility re-exports after all repository imports use new crates.
6. Remove the old root `src/` package.
7. Convert the root manifest to a virtual Workspace.
8. Set `apps/cli` as the default member and update all repository gates to use
   `--workspace` where full coverage is intended.

### Phase 8: Move The Web Application

Goal: finish the product directory layout after Rust contracts are stable.

Tasks:

1. Move `web-ui/` to `apps/web/` without changing package behavior.
2. Update CI working directories and lockfile cache keys.
3. Update `scripts/dev.ps1`, `scripts/integration-smoke.ps1`, provider scripts,
   Playwright paths, and environment documentation.
4. Update current README, AGENTS, onboarding, and runtime path maps.
5. Run Web unit tests, typecheck, build, and browser-visible flow tests.

Do not mechanically rewrite historical design documents. Update maintained
current-state documents and add replacement notes where historical paths would
otherwise mislead maintainers.

### Phase 9: Final Architecture And Release Gate

Goal: prove the new repository is the same product with enforceable modular
boundaries.

Tasks:

1. Run metadata dependency assertions for all packages.
2. Run default, RAG, Web, CLI/TUI, API/SSE, benchmark, and integration gates.
3. Run old-artifact resume and repair fixtures against `rove-runtime`.
4. Verify local installation from `apps/cli` and deterministic fake-model use.
5. Inspect generated artifacts and screenshots for secrets or path regressions.
6. Update `docs/runtime/` to the new implemented paths and architecture.
7. Mark the modular architecture design implemented only after all acceptance
   evidence exists.
8. Remove the temporary baseline worktree only after explicit approval.

## 9. Verification Matrix

### Per-change Rust gate

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

During the transitional non-virtual Workspace, run the equivalent root-package
commands in addition to new package-focused tests.

### Focused package gates

```powershell
cargo test -p rove-models
cargo test -p rove-core
cargo test -p rove-runtime
cargo test -p rove-api
cargo test -p rove-cli
cargo test -p rove-bench
cargo test -p rove-integration-tests
```

### RAG gate

Use explicit package feature selection after the root feature disappears:

```powershell
cargo check -p rove-cli --features rag --bin rove-index
cargo clippy -p rove-runtime -p rove-cli --all-targets --features rag -- -D warnings
cargo test -p rove-runtime --features rag
cargo test -p rove-integration-tests --features rag
```

The exact command set may expand if API or bootstrap gains feature-specific
targets; it must never shrink coverage by relying on an ambiguous root feature.

### Web gate

From `apps/web/` after the move:

```powershell
pnpm test
pnpm typecheck
pnpm build
```

Run `pnpm test:e2e` for SSE, approval, input, cancellation, resume, proxy, or
other browser-visible contract changes.

### Full integration

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

Real provider, MCP, RAG-provider, and browser service gates remain opt-in. A
skip is not interoperability evidence.

## 10. Documentation Updates By Phase

Every implementation phase updates current documentation in the same change:

- root `AGENTS.md` repository map and verification commands;
- `README.md` quick start, binaries, install path, and project map;
- `docs/ONBOARDING.md` entry points and test commands;
- `docs/runtime/architecture.md` dependency and event flow;
- `docs/runtime/react-loop.md` actual source locations;
- `docs/runtime/subsystems.md` config/state/provider/tool/Web locations;
- `docs/runtime/implementation-guide.md` maintainer path map;
- CI and integration documentation when commands or paths change.

Each crate receives a README containing:

- responsibility and non-responsibility;
- local dependency list;
- minimal public API example;
- focused verification command;
- compatibility/stability status.

## 11. Rollback And Failure Handling

Each extraction commit must be independently revertible before the next layer
depends on it. Do not combine all physical moves and API rewrites into one
commit.

If a phase reveals a hidden dependency cycle:

1. stop the physical move;
2. restore the last green migration commit without discarding unrelated user
   work;
3. add an explicit interface in the owning lower layer or move the behavior to
   the correct upper layer;
4. add a regression/architecture test;
5. resume extraction only after the focused and repository gates pass.

If serialized output changes unexpectedly, treat it as a contract regression,
not formatting churn. Either restore compatibility or write a separately
reviewed migration with defaults, fixtures, and current documentation.

## 12. Completion Definition

- [ ] The canonical checked-out directory is named `rove`.
- [ ] The root manifest is a virtual Cargo Workspace with resolver `3`.
- [ ] `models/`, `core/`, `runtime/`, and existing `apps/` products own all
  current code; the old root `src/` package is gone.
- [ ] Cargo metadata proves the allowed one-way local dependency graph.
- [ ] `rove-core` embeds with Fake Model and a custom Tool without runtime
  state or app dependencies.
- [ ] `rove-runtime` provides persistence, approval, planning, resume, memory,
  MCP, official tools, and canonical durable events; optional RAG remains the
  explicitly deferred user-led refactor recorded in Phase 5 progress.
- [ ] CLI, API, benchmark, and Web consume the same runtime.
- [ ] CLI/API/SSE/Web behavior and binary names remain compatible.
- [ ] Existing trace/task-state/report/SQLite artifacts remain readable and
  completed/unknown side effects retain conservative resume behavior.
- [ ] Default Rust, RAG, Web, browser-relevant, and full integration gates pass.
- [ ] Current runtime documentation points to the new implementation paths.
- [ ] Every Rust crate has a boundary README and public usage example.
- [ ] The modular architecture design status is updated only after code, tests,
  and current docs agree.
- [ ] Temporary worktree, generated artifacts, and migration-only compatibility
  modules are absent from the final repository.
