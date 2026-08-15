# AGENTS.md

This file defines repository-wide working rules for coding agents and maintainers.
It applies to the entire repository unless a deeper directory contains its own
`AGENTS.md`. There are currently no nested overrides.

## 1. Start with the right source of truth

Use this precedence when sources disagree:

1. Reproducible behavior in current source code, tests, generated schemas, and CI.
2. `docs/runtime/` for current architecture, supported behavior, verification,
   and known gaps.
3. Root `README.md`, `MEMORY_DOCTRINE.md`, and `docs/ONBOARDING.md` for the
   maintained project overview.
4. `docs/design/` for active proposed or accepted target designs.
5. `docs/Archive/` for early design, plan, handoff, and comparison history only.

Do not silently choose one side of a code/document contradiction. Confirm the
behavior, keep the safer current contract, and update the stale current-state
document in the same change when the task includes implementation.

Every future spec must state whether it is implemented. A document marked
`Proposed / Not Implemented` is not evidence that the runtime already supports
the described types, events, configuration, or behavior.

## 2. Recommended reading order

For any non-trivial change, read:

1. [`docs/ONBOARDING.md`](docs/ONBOARDING.md)
2. [`docs/runtime/README.md`](docs/runtime/README.md)
3. The relevant current-state document under `docs/runtime/`
4. The implementation and tests for the affected subsystem
5. A future spec only when the requested work explicitly targets that design

Additional routing:

| Change area | Read first |
|---|---|
| Embedded Agent/tool loop | `core/src/`, `tests/embedding_contract.rs` |
| Persistent Engine/planning/events | `docs/runtime/react-loop.md`, `runtime/src/`, `tests/e2e.rs` |
| State/resume/artifacts | `docs/runtime/subsystems.md`, `runtime/src/state/`, `tests/` |
| Providers/routing | `docs/runtime/provider-smoke.md`, `models/src/`, `apps/bootstrap/src/factory.rs` |
| Tools/safety/MCP | `docs/runtime/subsystems.md`, `runtime/src/tools/`, `apps/bootstrap/src/registry.rs`, `tests/tool_safety.rs`, `tests/mcp.rs` |
| Memory/context | `MEMORY_DOCTRINE.md`, `runtime/src/memory/`, `runtime/src/context/manager.rs`, `runtime/src/context/compaction.rs` |
| Workspace retrieval | Tools (`fs`/`shell`) + layered MD memory (`MEMORY_DOCTRINE.md`); no built-in vector RAG |
| API | `docs/runtime/implementation-guide.md`, `apps/api/`, `tests/api.rs` |
| Web | `apps/web/` tests and package scripts |
| Benchmarks | `docs/runtime/benchmark-evidence.md`, `apps/bench/`, `tests/bench.rs` |

## 3. Repository map

| Path | Responsibility |
|---|---|
| `models/` | `rove-models`: normalized model protocol, providers, routing, fake provider |
| `core/` | `rove-core`: in-memory Agent loop, core events/control, tool contracts and registry |
| `apps/cli/` | `rove-cli`: CLI/REPL/TUI and local command surfaces |
| `apps/api/` | `rove-api`: HTTP/SSE/OpenAPI surface |
| `apps/bench/` | `rove-bench`: deterministic benchmark runner |
| `apps/bootstrap/` | `rove-app-bootstrap`: product config and assembly |
| `runtime/` | `rove-runtime`: contracts/events, workspace, context/compaction, memory, local built-in tools, MCP proxy, Executor/hooks, planning, Engine, state/artifacts/SQLite/repair/resume |
| `tests/` | `rove-integration-tests`: cross-package contracts |
| `benchmarks/` | Deterministic benchmark definitions and published evidence |
| `apps/web/` | Standalone Next.js product shell; developer workbench is an advanced escape hatch |
| `scripts/` | Local development and integration runners |
| `docs/runtime/` | Current implementation source of truth |
| `docs/design/` | Active proposed/target and implemented architecture documents |
| `docs/plans/` | Active and recent implementation plans |
| `docs/Archive/` | Historical early designs, plans, handoffs, and comparisons |

## 4. Architecture invariants

Preserve these boundaries unless the task explicitly changes the architecture
and updates its tests and current documentation:

- CLI, API, Web, and benchmark paths must reuse the shared runtime instead of
  growing independent agent loops.
- Provider-specific payloads stay behind the model/provider boundary. Core
  execution consumes normalized messages, tool calls, usage, and errors.
- Tool execution goes through `ToolRegistry` and the existing safety/approval
  path. A tool description, MCP annotation, prompt, or model request cannot
  grant permission.
- Workspace paths remain bounded by the resolved workspace. Never trust
  provider- or server-supplied paths as local paths.
- Canonical stream events are the lifecycle contract shared by persistence,
  API SSE, Web, tests, and reports. Do not introduce a private parallel event
  lifecycle for one interface.
- `trace.jsonl` records event facts, `task_state.json` records resumable state,
  and `report.json` is a derived summary. Do not treat the report as the only
  durable truth.
- Completed mutations and completed plan work must not be replayed on resume.
  Unknown in-flight side effects require conservative handling.
- Memory, optional external retrieval, workspace instructions, tool output, and
  runtime policy are distinct authorities. Retrieved or generated text is not
  automatically a trusted instruction.
- Local deterministic execution must remain available without provider keys or
  network access.
- Secrets must not appear in committed configuration, normal logs, trace,
  report, API responses, screenshots, fixtures, or benchmark evidence.

## 5. Current implementation boundaries

As of 2026-08-14:

- rove is a local-first Rust runtime with CLI, API, Web, persisted run state,
  resume, provider routing, tools, layered memory, optional future external retrieval, and
  deterministic benchmarks.
- The repository is a virtual Cargo Workspace with default member `apps/cli`.
  Package layout is `rove-models <- rove-core <- rove-runtime <-
  rove-app-bootstrap <- {rove-cli, rove-api, rove-bench}`, with
  `rove-api <- rove-desktop`, plus
  `rove-integration-tests`. `rove-models` has no local project dependency;
  `rove-core` depends only on `rove-models`. `rove-runtime` owns durable
  execution, state, memory, tools/MCP, planning, and the Engine facade.
  `rove-app-bootstrap` owns first-party AppConfig, provider factory, product
  registry assembly, and shared Engine assembly. Workspace retrieval is tool-based plus layered file memory; there is no built-in vector RAG.
- `docs/runtime/` describes the implemented MVP, Web M1 product shell, and the
  Web Complete C0–C3 persistence, continuity, Settings, migration, polish, and
  acceptance work integrated on `main` through PRs #24, #25, and #26, plus the
  CDH G1-G7 control, evidence, and Settings completion merged through PR #29.
- MCP supports stdio, deprecated legacy SSE, and negotiated Streamable HTTP.
  All three proxies map bounded rich result blocks into the shared Tool Result
  envelope and canonical durable Tool Artifact store. Streamable HTTP supports
  bounded `listChanged` rediscovery, atomic namespace replacement, run-pinned
  catalogs, required/optional degradation, circuit backoff, secret-free runtime
  identity/health, canonical refresh/degradation events, and Product API/Web
  diagnostics. Real third-party hosted MCP interoperability remains an optional
  unrun gate.
- Durable granular Project Trust and the Runtime-owned Execution Environment
  are implemented. Project activation remains restricted by default, exact-root
  and capability-specific; workspace `.env`, `.rove/config.toml`, and MCP
  startup stay deferred until authorized.
- Coding Tool V2 is implemented on the shared Execution Environment and
  authoritative Tool Registry. It provides bounded ranged reads, observed
  mutations, deterministic discovery continuation, process-local checkpoints,
  transient artifact projection, and foreground/background Shell lifecycle.
  Native PTY remains a typed unsupported capability; coding observations,
  checkpoints, projected artifacts, and process identities are not durable.
- Versioned AgentDefinition packages, immutable per-run profiles, trusted
  root/nested `AGENTS.md` discovery, typed procedural catalogs, deterministic
  selection, bounded hydration, exact unfinished-run resume snapshots, and
  CLI/API/Web selector surfaces are implemented. Nested instructions are
  activated only for matching paths; a newly discovered tool scope is deferred
  before dispatch until the model receives its overlay. Procedure-aware
  Planner/Evaluator/Finalizer phases and the deterministic OnCall reference
  evaluation suite are implemented; external-provider experiments and broader
  holdout matrices remain optional future work. The execution-lifecycle design is partially implemented:
  one Runtime-neutral Agent kernel drives embedded, unplanned, and planned-step
  model/tool coordination; Runtime hosts retain context, persistence, planning,
  approval/input, hooks, memory, and canonical events. Bounded planned
  StepRunner, append-only StepRecord ledger, immutable plan revisions,
  rule-first decisions with bounded model-on-ambiguity evaluation, an
  independent evidence-grounded Finalizer, public multidimensional execution
  budgets with per-run accounting, canonical lifecycle events, trace-tail
  reconciliation on resume, registration-pinned bounded Tool Schemas,
  pre-dispatch provider/tool validation, and Runtime-owned capability snapshots
  exist. Streamable HTTP MCP catalogs refresh live for future runs while active
  runs retain their pinned bindings; general non-MCP live capability refresh is
  not implemented.
- Web M1 is implemented: explicit Folder/Repo roots, fail-closed hard resume,
  and the Workspace → Session → Chat product shell are implemented. Web
  Complete C0 adds an API-global SQLite ProductStore,
  workspace/session/profile/preferences CRUD, exact server-owned
  product-session/runtime bindings, single-active-turn claims,
  canonical-event transcript projection, strict/idempotent M1 migration, and
  typed Web client/migration modules. Web Complete C1 wires the default shell
  to the API-authoritative workspace/session/preferences/profile catalog,
  canonical transcript restore with explicit partial/error states, durable
  workspace/session/Settings routes, exact `product_session_id` turns,
  focused-job SSE reattachment, background status polling, and bounded
  reconciliation when a job-start response is ambiguous. Provider profiles and
  their active selection now persist through the C0 API; raw keys remain out of
  browser state and requests. Web Complete C2 adds revision-safe approval
  defaults, bounded memory/runtime settings APIs, full provider CRUD, durable
  workspace/session management, real Memory and runtime health views, and four
  wired keyboard shortcuts across all nine non-placeholder Settings sections.
  Web Complete C3 invokes M1 migration before product catalog reads, preserves
  exact retry/deep-route behavior, and completes responsive, focus/keyboard,
  live-status, reduced-motion, theme, and screenshot-evidence polish. Mocked
  browser suites remain deterministic contract evidence, while `local-full`
  now runs live-API migration and default-product-shell acceptance plus one
  bounded `/dev/workbench` advanced smoke; productization integration adds
  unified-message promotion/revocation and completed-session Fork/independent
  child continuation, and the latest local fake-provider run passed all five
  Playwright scenarios. The external-provider gate was not run, so no external
  interoperability claim is made. C0–C3 are on `main` through merge commits
  `db8f970`, `abbd7d6`, and `e3c2403`. CDH G1-G7 add the durable compatibility
  controls now projected through unified Send Message, terminal-boundary
  Fork/lineage, session model/reasoning/approval/step-limit
  snapshots, usage/context/cost, bounded files/artifacts/images/diff, redacted
  evidence export, and workspace-scoped Settings/MCP management; they are on
  `main` through PR #29 at `f9e88a7`. Post-Coding-Tool full delivery,
  including the Tauri Desktop D0 host, is on `main` through PR #30 at
  `4b740d3`; PR #31 at `1b57b36` contains the whitespace-only documentation
  cleanup. Windows MSI/NSIS and release-process evidence are recorded, while
  macOS/Linux packaging and a complete installed-Desktop journey remain
  unverified.
- Productization workstreams A-E and the core F.1-F.3 conversation lifecycle
  are implemented in current source. Native-first
  tool-call recovery, deterministic ignore-aware repository retrieval,
  Artifact-backed result history, user Provider onboarding/TUI model selection,
  and one durable conversation-message lifecycle share the existing
  Runtime/Engine authorities. ProductStore schema v13 reconciles both parallel
  v12 productization layouts. Deterministic Rust/Web/TUI checks and five live
  local fake-provider browser cases pass. F.4 remains partial because the Web
  and TUI load only the latest bounded message page rather than supporting
  stable older-history prepend/windowing. F.5 remains partial because TUI
  restart recovery does not yet drain queued successors or reconcile a
  claimed successor that has no run. Workstream G remains partial because
  credentialed external Provider, real third-party/official filesystem MCP,
  Windows ConPTY, macOS/Linux packaging, signing, installed-Desktop, and broader
  stress/soak gates remain unverified.
- This repository-level `AGENTS.md` guides maintainers and coding agents. A
  trusted rove run also loads it as the workspace root instruction layer; that
  runtime admission does not turn its text into tool permission or approval.

The active design/runtime-evolution chain is:

- `docs/design/2026-07-14-agent-execution-lifecycle-design.md`
- `docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`
- `docs/design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`
- `docs/design/2026-07-15-oncall-reference-agent-evaluation-plan.md`

Read each document's visible status before using it. The lifecycle,
AgentDefinition, and MCP documents are partially implemented design records;
their remaining targets are future work. The OnCall document is a partially
implemented design record with the deterministic V2 suite in the repository;
external-provider and holdout evidence remain optional. Use `docs/runtime/` and
tests to describe current behavior.

The completed post-CDH and full-delivery implementation records are:

- `docs/plans/2026-08-06-kernel-message-provider-implementation.md`
- `docs/plans/2026-08-06-project-trust-execution-tools-implementation.md`
- `docs/plans/2026-08-07-authoritative-tool-schema-runtime-validation.md`
- `docs/plans/2026-08-07-coding-tool-v2-implementation.md`
- `docs/plans/2026-08-07-post-coding-tool-v2-master-program.md`
- `docs/plans/2026-08-07-post-coding-tool-v2-full-delivery.md`

The current productization implementation record is:

- `docs/plans/2026-08-10-post-full-delivery-productization.md`

Its workstreams A-E are implemented. F.1-F.3 are implemented while F.4/F.5 and
G are partially complete. The dated 2026-08-09 documents it links are
supporting audit evidence, not additional implementation plans.

The optional terminal-interface direction is documented separately in
`docs/design/2026-07-16-grok-build-reference-and-tui-design.md`.

## 6. Working in a dirty tree

- Inspect `git status --short` before editing.
- Existing modified or untracked files belong to the user unless the task says
  otherwise.
- Do not delete, reset, overwrite, stage, or commit unrelated work.
- Do not use destructive Git commands such as `git reset --hard` or
  `git checkout --` without explicit authorization.
- Keep changes scoped. If an existing edit overlaps the requested change,
  preserve it and report any ambiguity.
- Generated state such as `.rove/`, `target/`, `apps/web/.next/`,
  `apps/web/node_modules/`, test results, and temporary integration output must
  not be committed.

## 7. Editing rules

- Prefer small, reviewable changes with an explicit compatibility story.
- Use the repository's existing naming, error, event, and serialization
  patterns before inventing a new abstraction.
- Avoid broad mechanical rewrites unless they are necessary for the task.
- When adding serialized types or public API fields, define defaults,
  compatibility, migration, and test coverage.
- When changing events, update producers, persistence, API/OpenAPI consumers,
  Web consumers, and contract tests together.
- When changing tool safety or approval behavior, add negative tests, not only
  success tests.
- Do not add dependency crates or npm packages unless the implementation needs
  them. Explain why the existing stack is insufficient.
- Let Cargo/pnpm tooling update lockfiles only when dependencies actually
  change.
- Do not weaken lints, ignore errors, add blanket `allow` attributes, or remove
  tests to make a gate pass.

## 8. Verification

Run the smallest relevant check first, then expand in proportion to risk.

### Rust default path

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Focused examples:

```powershell
cargo test -p rove-integration-tests --test e2e
cargo test -p rove-integration-tests --test api
cargo test -p rove-integration-tests --test mcp
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test bench
```

### Web

From `apps/web/`:

```powershell
pnpm test
pnpm typecheck
pnpm build
```

Run `pnpm test:e2e` when the change affects browser-visible flows, SSE,
approval/input/cancel/resume, or the API proxy. Follow the opt-in gates in
`docs/runtime/integration-testing.md`.

### Integration and real services

- Use `scripts/integration-smoke.ps1` for the local full stack when appropriate.
- Use `scripts/product-acceptance.ps1` or `scripts/product-acceptance.sh` for the
  full gate sweep with a machine-readable `PRODUCT_ACCEPTANCE_REPORT.json`. Never
  hand-edit that report: a status without a real exit code is a fake PASS.
- Provider, real MCP, and real browser gates are opt-in. Never assume credentials
  or external services are available.
- A skipped real-service test only proves the skip path, not interoperability.
- Never point tests at production services.

### Documentation-only changes

For docs-only changes:

- inspect all relative links;
- ensure code fences are balanced;
- check heading structure and duplicate anchors;
- check trailing whitespace;
- verify current/proposed wording;
- inspect `git diff --check` and `git status --short`.

Code tests are not required solely because Markdown changed, unless the
document is parsed or asserted by tests, changes executable commands, or the
task explicitly asks for a full gate.

## 9. Documentation governance

- `docs/runtime/` describes current behavior and must be updated in the same
  implementation change that changes the contract.
- Future architecture belongs under `docs/design/` and must carry a
  visible status. Early historical designs live under `docs/Archive/`.
- Implementation plans belong under `docs/plans/` and must not require a
  particular external agent skill. Completed early plans live under
  `docs/Archive/plans/`.
- Do not update `docs/runtime/implementation-status.md` or
  `docs/runtime/acceptance-matrix.md` to `Met` before code and tests exist.
- Keep examples secret-free and portable; prefer relative paths and environment
  references.
- Link to current source/test evidence for non-obvious claims.
- Preserve historical documents unless the task explicitly retires them.
- When a design decision changes, record the replacement and affected
  migration rather than silently rewriting history.

## 10. Security checklist

Before handing off a change involving tools, API, providers, state, MCP,
artifacts, or Web:

- Are input size, path, timeout, and concurrency bounded?
- Can untrusted content become instructions or permissions?
- Are secrets redacted from errors, events, config dumps, traces, reports, and
  fixtures?
- Is approval required at the correct boundary?
- Are retries safe for side effects?
- Does cancellation leave an unknown external effect?
- Can resume replay completed work?
- Are remote URLs, redirects, filenames, MIME types, and resource identifiers
  validated?
- Are API and Web behavior protected by existing auth/CORS/rate-limit rules?
- Is the failure state typed and visible instead of reported as success?

## 11. Completion and handoff

A completed change should state:

- what changed;
- which current contract or proposed design it implements;
- files intentionally not changed;
- tests/checks run and their results;
- checks not run and why;
- known risks or follow-up work;
- current `git status`, especially untracked evidence or user-owned files.

Do not claim completion from prose alone. Implementation work is complete only
when the requested behavior exists, relevant verification passes, and current
documentation agrees with the code.
