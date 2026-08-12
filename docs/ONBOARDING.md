# rove Maintainer Onboarding

> Status: **Current Maintainer Guide**
>
> Last reviewed: 2026-08-14. This guide explains the repository as it exists
> today. For exact subsystem contracts and implementation status, follow
> [`docs/runtime/`](runtime/README.md). Documents marked
> `Proposed / Not Implemented` describe future work.

This guide is the shortest path from a fresh checkout to making a safe,
evidence-backed change in rove.

The repository is a virtual Cargo Workspace. The dependency chain is
`rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap <-
{rove-cli, rove-api, rove-bench}` plus `rove-integration-tests`.

- `rove-models` owns the normalized model protocol, providers, and routing.
- `rove-core` is the implemented in-memory embedding layer and shared
  Runtime-neutral Agent kernel.
- `rove-runtime` owns durable execution, state, memory, tools/MCP, planning, and
  the Engine facade.
- `rove-app-bootstrap` owns first-party AppConfig, provider factory, product
  registry assembly, and shared Engine assembly.
- Product surfaces live under `apps/`. Built-in vector RAG has been removed.

## 1. What rove is

rove is a local-first, stateful Agent runtime written in Rust. The same runtime
core is used by:

- an interactive CLI and non-interactive exec mode;
- an HTTP API with job lifecycle and SSE events;
- a standalone Next.js product shell backed by the API/SSE contract;
- deterministic local benchmarks;
- tool-based workspace retrieval and layered file memory.

The runtime combines:

```text
Workspace
  -> Engine
     -> Context and memory
     -> Model provider/routing
     -> Tool registry, safety, approval
     -> Events
     -> State/checkpoint/report artifacts
```

The project is not:

- a hosted multi-user Agent service;
- a Browser/Desktop workspace implementation;
- a framework wrapper around LangChain/LangGraph;
- a production AIOps system;
- an excuse to hide provider, tool, or state semantics behind one opaque loop.

The current MVP boundary is defined in
[`runtime/mvp-definition.md`](runtime/mvp-definition.md).

## 2. Ten-minute orientation

Read in this order:

1. [root README](../README.md) — product boundary and quick start.
2. [runtime README](runtime/README.md) — current documentation map.
3. [runtime architecture](runtime/architecture.md) — major components.
4. [ReAct loop](runtime/react-loop.md) — current execution behavior.
5. [subsystems](runtime/subsystems.md) — config, state, providers, tools,
   memory, API, workspace retrieval, Web, and CI.
6. [implementation guide](runtime/implementation-guide.md) — code paths and
   verification commands.
7. [implementation status](runtime/implementation-status.md) — implemented
   versus remaining work.
8. [root `AGENTS.md`](../AGENTS.md) — repository working rules.

Read [`MEMORY_DOCTRINE.md`](../MEMORY_DOCTRINE.md) before changing context,
compaction, session summary, durable memory, or recall.

Only then open a future spec if your task explicitly implements it.

## 3. Prerequisites

Required for the Rust runtime:

- Git;
- Rust stable, selected by `rust-toolchain.toml`;
- Cargo.

Required for the Web product shell:

- a current Node.js version compatible with the lockfile;
- pnpm 10, as declared by `apps/web/package.json`.

Useful for repository scripts and optional checks:

- PowerShell;
- Python for local MCP fixture tests;
- Playwright browser dependencies for Web E2E;
- Node/npx for the opt-in official filesystem MCP smoke.

No provider key is required for the default fake-model flow or default
deterministic tests.

## 4. First checkout checks

Start by preserving local work:

```powershell
git status --short
git branch --show-current
git rev-parse HEAD
```

Confirm tools:

```powershell
rustc --version
cargo --version
pnpm --version
```

Do not clean a dirty tree just to obtain a pristine baseline. Existing modified
and untracked files may be user work.

## 5. Fast local start

### 5.1 CLI with no network

Default full-screen TUI:

```powershell
cargo run -p rove-cli -- --model fake
```

Line-oriented REPL:

```powershell
cargo run -p rove-cli -- repl --model fake
```

One initial prompt:

```powershell
cargo run -p rove-cli -- --model fake "echo hello from rove"
```

Non-interactive:

```powershell
cargo run -p rove-cli -- exec --model fake "echo hello from rove"
```

`Cargo.toml` sets `default-run = "rove"`, so plain `cargo run -p rove-cli -- ...` selects
the CLI binary.

### 5.2 API and Web

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

The launcher starts:

- Rust API, normally on `127.0.0.1:8787`;
- Next.js Web, normally on `localhost:3000`;
- process-tree cleanup when the launcher exits.

Use custom ports when necessary:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -ApiAddr 127.0.0.1:18787 -WebPort 3001
```

Manual startup remains possible:

```powershell
cargo run -p rove-api
cd apps/web
pnpm install --frozen-lockfile
pnpm dev
```

The API exposes generated OpenAPI at `/api/openapi.json` and Swagger UI at
`/swagger-ui`.

### 5.3 Benchmark smoke

```powershell
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

This is a deterministic, no-network smoke. The separate OnCall Reference V2
suite is also implemented and runs through `rove-bench` with independent truth
and hard safety oracles.

## 6. Main entry points

| Surface | Entry | Follow next |
|---|---|---|
| CLI | `apps/cli` | command dispatch, REPL, TUI |
| API | `apps/api` | HTTP/SSE surface |
| Benchmark | `apps/bench` | Deterministic benchmark runner |
| Web | `apps/web/` | API proxy, state hooks, components, tests |
| Agent core | `core/` | shared Agent kernel, in-memory embedding host, control, core events, contracts |
| Persistent runtime | `runtime/` | contracts/events, workspace, context/compaction, memory, local built-in tools, MCP proxy, StateStore, artifacts/SQLite, repair and resume |
| Persistent coordinator | `runtime/` | Engine, planning/run coordination, tool turns, memory-flush ordering, durable event translation |
| Models | `models/` | independent protocol, provider adapters, routing, fake provider |
| Provider assembly | `apps/bootstrap/` | AppConfig + named profiles -> ProviderClient registry path |
| Runtime tools | `runtime/src/tools/` | echo, filesystem, shell, memory, request-input, and invocation adapters |
| Tool assembly | `apps/bootstrap` + `runtime/src/tools` | product registry assembly and built-in tools |

## 7. Request lifecycle

The exact implementation is documented in
[`runtime/react-loop.md`](runtime/react-loop.md). At a high level:

```text
CLI/API/benchmark request
  -> resolve workspace/config/provider/tools
  -> create RunRequest and run identity
  -> build context
  -> call model
  -> normalize message/tool calls
  -> request approval/input when needed
  -> execute tools
  -> emit canonical events
  -> checkpoint and record artifacts
  -> continue, stop, cancel, or resume
  -> write final report
```

When tracing a bug, follow both:

1. model/tool mechanics in `core/src/` and durable product control flow in `runtime/src/`;
2. canonical events and persisted state implementation in `runtime/src/`.

Do not debug only from the final assistant text. It is a projection, not the
whole execution history.

## 8. Workspaces

rove resolves a workspace and keeps tool/state behavior relative to that
workspace.

Common modes:

- current folder/repository workspace;
- isolated Task workspace.

Example Task workspace:

```powershell
cargo run -p rove-cli -- --task-workspace invoice-check --task-base .rove/tasks --model fake "review this task"
```

Task workspaces isolate files, state, and default memory beneath the task
directory. Code that accepts a path must preserve workspace boundary checks;
do not trust model/provider/MCP strings as local paths.

## 9. Configuration

Configuration precedence:

```text
defaults < user ~/.rove/config.toml < trusted workspace selection
         < environment < CLI overrides
```

The user file is Provider catalog schema v1. It owns full Provider definitions;
a trusted workspace may only select an existing profile/model and cannot define
an endpoint, auth source, header, fallback, protocol option, or adapter command.
`ROVE_CONFIG_ROOT` selects an explicit user-config directory for tests and
embedders.

Minimal local Ollama example:

```toml
schema_version = 1

[model]
default_profile = "local"
default_model = "llama3.2"
reasoning = "default"

[provider.profiles.local]
provider_type = "ollama"
base_url = "http://localhost:11434"
model = "llama3.2"
auth = { style = "none" }
```

Remote profiles use `auth.secret = { env = "NAME" }`, `{ file = "path" }`,
or `{ keyring = { service = "...", account = "..." } }`. Literal secrets are
rejected. Catalog writes are revision-CAS protected, locked, and atomically
replaced. Normal startup without a Provider reports onboarding rather than
silently using Fake; `--model fake` remains the explicit no-network path.

Inspect the resolved, redacted configuration:

```powershell
cargo run -p rove-cli -- dump-config
cargo run -p rove-cli -- provider migrate
```

Important rules:

- use `fake` for local deterministic work;
- use environment references for provider keys;
- never commit `.env.integration` or Web `.env*`;
- config dumps must redact secrets;
- Web provider overrides must not send raw provider keys from browser code;
- API token configuration belongs server-side.

Legacy Provider imports use `rove provider migrate`, which is read-only unless
`--apply` is supplied. Conflicts require explicit `--rename` mappings, and
rewriting workspace Provider definitions requires both `--apply
--rewrite-workspace-config` and workspace trust.

Provider setup and opt-in verification live in
[`runtime/provider-smoke.md`](runtime/provider-smoke.md).

## 10. State and artifacts

The default local state root is `.rove/`. The current implementation uses a
layout centered on:

```text
.rove/
  state.sqlite
  runs/
    <run_id>/
      trace.jsonl
      task_state.json
      report.json
      ...
```

Exact filenames can evolve; use `runtime/src/state/` and current runtime docs as truth.

Semantics:

- `trace.jsonl` — append-oriented canonical event facts;
- `task_state.json` — resumable task state/checkpoint projection;
- `report.json` — human/API-oriented final summary;
- SQLite — index and lookup layer, not the only source of run facts.

Useful commands:

```powershell
cargo run -p rove-cli -- sessions
cargo run -p rove-cli -- state repair
cargo run -p rove-cli -- state cleanup
```

When changing persistence:

- preserve schema/backward compatibility or add explicit migration;
- test incomplete/corrupt artifacts;
- test resume without duplicate work;
- keep reports derivable from facts;
- never persist secrets for convenience.

## 11. Context and memory

The implemented context order is documented in
[`MEMORY_DOCTRINE.md`](../MEMORY_DOCTRINE.md):

```text
system
  -> durable memory
  -> session memory
  -> compact summary
  -> recent history
  -> current user message
```

Three memory layers:

- working memory — current in-process run;
- session memory — resumability-oriented summary;
- durable memory — stable cross-session facts/preferences/feedback/reference.

Optional external retrieval would be separate and is not implemented. Tool
output and workspace rules are also distinct authorities. Do not merge all
retrieved or generated content into one high-authority prompt.

## 12. Models and routing

Current provider families include:

- OpenAI Chat Completions (`provider_type=openai` -> `wire_protocol=openai-completions`);
- OpenAI Responses;
- Anthropic;
- Ollama;
- fake provider;
- configured routing/fallback.

CLI resolves the session selection and creates a fresh model/Engine
`RunAssembly` for each turn. The resulting secret-free model snapshot is stored
with the run, and resume fails with `provider_changed_for_resume` if the current
Provider identity no longer matches. In `rove tui`, `/model`, `/model current`,
`/model <query>`, and `/model reset` operate on a revisioned per-session
selection; changes are busy while a run is active and affect only the next
turn. The picker currently lists configured catalog models rather than live
remote inventory.

Core code should consume normalized provider results. Provider adapters own:

- request/response shape;
- native tool-call mapping;
- streaming details;
- usage/error normalization;
- provider-specific authentication.

These contracts and adapters now live in `rove-models`, which has no local
project dependencies. `src/models/factory.rs` remains in the root compatibility
package because it still translates first-party `AppConfig` into model-layer
constructors; it will move to `apps/bootstrap` later in the migration.

When adding or changing a provider:

- test payload shape without real network;
- preserve tool-call IDs and result correlation;
- preserve cancellation and error taxonomy;
- add opt-in real smoke only as supplemental evidence;
- update current provider docs after implementation.

## 13. Tools, safety, and MCP

All tools register through the shared tool registry. Safety metadata and
approval are runtime decisions; a model or remote server cannot grant itself
permission.

Current MCP truth:

- stdio, deprecated legacy SSE, and negotiated Streamable HTTP are implemented;
- all transports map bounded rich result blocks into the shared envelope and
  durable Tool Artifact authority;
- Streamable HTTP supports bounded `listChanged` refresh with active-run
  pinning, required/optional degradation, circuit backoff, and health;
- deterministic fixtures cover protocol, refresh, artifact, and safety paths;
- the official filesystem and real third-party MCP smokes remain opt-in.

See:

- `runtime/src/tools/mcp_proxy.rs`;
- `tests/mcp.rs`;
- [`runtime/subsystems.md`](runtime/subsystems.md);
- [partially implemented MCP design record](design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md).

Never treat MCP annotations as authorization or a remote `file://` URI as a
local workspace path.

## 14. Workspace retrieval

Built-in vector RAG has been removed. Workspace context comes from tools and layered file memory.


## 15. Web product shell

`apps/web/` is a standalone Next.js application. It consumes the API and SSE
rather than embedding a second runtime. The default product surface is the
Workspace → Session → Chat + Inspector shell; `/dev/workbench` is a bounded
advanced-only escape hatch.

Web Complete C0 adds an API-global SQLite
ProductStore, product workspace/session/profile/preferences CRUD, exact
server-owned product-session/runtime bindings, one active turn per product
session, canonical-event transcript reads with typed partial reasons, strict
M1 browser migration, and typed Web client/migration modules. Web Complete C1
wires the default shell to the API-authoritative catalog, safe preferences,
provider profiles, and transcript reads. It adds durable workspace/session and
Settings routes, exact `product_session_id` turns, explicit partial/error/retry
restore states, focused live-job reattachment, background status polling, and
bounded no-duplicate reconciliation after ambiguous job starts. C2 completes
Settings: every section has a real surface, provider CRUD
and approval defaults are durable, workspace/session and memory management use
the API, runtime health is live, and four documented shortcuts are wired. C3
runs the fail-closed M1 migration gate before any product catalog read, preserves
mapped deep routes and exact retry payloads, and completes the responsive,
keyboard/focus, live-status, reduced-motion, theme, and screenshot-evidence
polish for the default shell. C0–C3 are integrated on `main` through PRs
#24–#26 and passed the post-merge deterministic gates.

CDH G1-G7 merged through PR #29 at `f9e88a7`. The default product shell and API
retain Steer/Follow-up compatibility routes beneath the unified Send Message
lifecycle and also expose terminal-boundary Fork/lineage, immutable
session run configuration snapshots, usage/context/cost, bounded files and
artifacts, image validation, run/Git diff, redacted evidence export, and a
workspace-scoped MCP catalog shared by Settings and job assembly.

From `apps/web/`:

```powershell
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
```

Use:

```powershell
pnpm test:e2e
```

when changing browser-visible job, SSE, approval, input, cancellation, resume,
or proxy behavior. Follow the environment gates in
[`runtime/integration-testing.md`](runtime/integration-testing.md).

Current browser evidence remains explicit about its boundary: `shell.spec.ts`,
`continuity.spec.ts`, `settings.spec.ts`, `migration.spec.ts`, and
`polish.spec.ts` provide deterministic mocked-API coverage. The gated
`real-api.spec.ts` used by `local-full` now exercises live M1 migration and the
default `/` product shell across exact A/B continuation, refresh, tools,
cancellation, Settings, and deep routes; it retains one bounded
`/dev/workbench` advanced smoke. The current suite also covers unified-message
promotion/revocation and completed-session Fork with independent child
continuation. The latest local fake-provider run passed all five real-API
scenarios. The external-provider gate was not run and this is not
external-provider interoperability evidence.

Keep provider/API tokens server-side. Browser JavaScript must not receive raw
provider secrets.

## 16. Benchmark system

Current benchmark code is under `apps/bench/`. It supports:

- scripted fake-model turns;
- setup files;
- tool use and batches;
- expected output/files/summary;
- trace/report/artifact checks;
- deterministic cancel/resume;
- evidence packages.

Fast checks:

```powershell
cargo test -p rove-integration-tests --test bench
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

Published evidence and provenance rules are described in
[`runtime/benchmark-evidence.md`](runtime/benchmark-evidence.md).

The implemented deterministic V2 Agent evaluation and OnCall reference suite are
documented in
[`2026-07-15-oncall-reference-agent-evaluation-plan.md`](design/2026-07-15-oncall-reference-agent-evaluation-plan.md).

## 17. Verification matrix

### 17.1 Rust core

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Start focused when iterating:

```powershell
cargo test -p rove-integration-tests --test e2e
cargo test -p rove-integration-tests --test api
cargo test -p rove-integration-tests --test mcp
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test bench
```

### 17.2 Web

```powershell
cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

### 17.3 Local full integration

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

Use the full integration guide:

- [`runtime/integration-testing.md`](runtime/integration-testing.md)
- [`runtime/full-integration-runbook.md`](runtime/full-integration-runbook.md)

### 17.4 Release evidence

Before release-oriented claims, use:

- [`runtime/acceptance-matrix.md`](runtime/acceptance-matrix.md);
- [`runtime/release-readiness.md`](runtime/release-readiness.md);
- provider/MCP opt-in gates as applicable.

A skipped external smoke is not evidence that the external integration works.

## 18. Common change workflows

### 18.1 Fix a core/runtime bug

1. Reproduce with a focused test.
2. Identify the canonical event/state boundary.
3. Add a failing regression test.
4. Make the smallest implementation change.
5. Run focused test, fmt, clippy, then relevant/full tests.
6. Update `docs/runtime/` if behavior or a public contract changed.
7. Inspect artifacts for accidental secrets or duplicated work.

### 18.2 Add/change a Tool

1. Define input schema and bounded validation.
2. Define safety/destructive/parallel behavior.
3. Preserve workspace/path policy.
4. Define timeout, error, mutation, cancellation, and output limits.
5. Add approval-negative tests.
6. Add event/report checks.
7. Test resume if side effects exist.

### 18.3 Change API/Web contract

1. Change shared/API types.
2. Update OpenAPI and API tests.
3. Update Web proxy/client/types.
4. Update UI tests.
5. Run API + Web + relevant E2E.
6. Update current runtime docs.

### 18.4 Change persistence/resume

1. Identify stable serialized versions.
2. Define compatibility/migration.
3. Test old/missing/corrupt state.
4. Test no duplicate completed work or mutation.
5. Test report/trace/state agreement.
6. Update repair/cleanup behavior and docs.

### 18.5 Documentation-only design

1. Put future behavior under `docs/design/`.
2. Mark status visibly.
3. Separate current evidence, goals, non-goals, target design, risks, and
   acceptance criteria.
4. Link relevant current docs/source.
5. Do not mark runtime status `Met`.
6. Put task-by-task implementation checklists under `docs/plans/`; do not make
   them depend on a particular external agent skill.
7. Validate links, headings, fences, whitespace, and `git diff --check`.

## 19. Documentation map and status

### Current truth

- [runtime README](runtime/README.md)
- [MVP definition](runtime/mvp-definition.md)
- [architecture](runtime/architecture.md)
- [ReAct loop](runtime/react-loop.md)
- [subsystems](runtime/subsystems.md)
- [implementation guide](runtime/implementation-guide.md)
- [implementation status](runtime/implementation-status.md)
- [acceptance matrix](runtime/acceptance-matrix.md)
- [integration testing](runtime/integration-testing.md)
- [release readiness](runtime/release-readiness.md)

### Active design and evaluation chain

1. [Agent Execution Lifecycle](design/2026-07-14-agent-execution-lifecycle-design.md)
2. [Agent Definition and Procedural Knowledge](design/2026-07-14-agent-definition-and-procedural-knowledge-design.md)
3. [MCP Streamable HTTP and Tool Artifacts](design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)
4. [OnCall Reference Agent and Evaluation](design/2026-07-15-oncall-reference-agent-evaluation-plan.md)

The execution lifecycle is implemented through one Runtime-neutral Agent
kernel, bounded StepRunner, StepRecord ledger, PlanRevision, rule-first
PlanDecision with bounded model-on-ambiguity evaluation, an independent
evidence-grounded Finalizer, public multidimensional execution budgets, and
trace-tail reconciliation on resume. AgentDefinition/instruction/procedure
activation and MCP Streamable HTTP/rich artifacts/live refresh are also
implemented. Planner, StepRunner, Evaluator, and Finalizer consume bounded
procedure material, and the deterministic OnCall suite covers lifecycle and
safety oracles. External-provider experiments and broader holdout matrices are
optional future work. The linked lifecycle, AgentDefinition, MCP, and OnCall
documents are partially implemented design records.

### Active product delivery

- [Agent Desktop + Web shared UI](design/2026-07-25-agent-desktop-web-ui-design.md)
  — Web M1 and Web Complete C0–C3 are implemented on `main`; the Tauri Desktop
  D0 host is implemented on `main` through PR #30 and has Windows
  MSI/NSIS/process evidence, while macOS/Linux packaging remains unverified.
- [Web Complete design](design/2026-07-26-web-complete-design.md) and
  [delivery plan](plans/2026-07-26-web-complete.md) — C0–C3 implementation,
  ordered coordinator integration, and post-merge local acceptance are complete.
- [Web → Desktop coordinator plan](plans/2026-07-25-web-desktop-master-delivery.md)
  — historical Web delivery coordination; Desktop D0 now has its dedicated
  design and implementation plan.
- [CDH G1-G7 delivery](plans/2026-08-03-cdh-alder-merge.md) — completed through
  PR #29; G8 Desktop was out of scope.
- [Kernel, Message, and Provider Implementation](plans/2026-08-06-kernel-message-provider-implementation.md)
  — completed implementation record for typed message/session projection,
  provider protocol normalization, and the shared-kernel migration.
- [Authoritative Tool Schema and Runtime Validation](plans/2026-08-07-authoritative-tool-schema-runtime-validation.md)
  — completed implementation record for bounded schema compilation,
  deterministic/atomic tool catalogs, model preflight, and Runtime capability
  snapshot binding.
- [Project Trust, Execution Environment, and Coding Tools Implementation](plans/2026-08-06-project-trust-execution-tools-implementation.md)
  — completed implementation record for persistent Project Trust,
  Runtime-owned execution adapters, and the Coding Tool foundation.
- [Post-Full-Delivery Productization Program](plans/2026-08-10-post-full-delivery-productization.md)
  — implementation record for completed workstreams A-E and F.1-F.3, plus the
  partially complete F.4/F.5 and release-confidence workstream G. Its linked
  2026-08-09 documents are supporting audits and deferred-boundary records, not
  parallel plans.

### Independent terminal interface direction

- [Grok Build reference and TUI direction](design/2026-07-16-grok-build-reference-and-tui-design.md)

This design is implemented at the bounded single-session TUI MVP. Bare `rove`
starts the TUI, `rove tui` remains an explicit alias, and `rove repl` preserves
the line-oriented interface. The TUI supports bounded,
fail-closed approval/input modals, session navigation/resume selection, bounded
tool detail, keymap-derived help, a canonical-order visible timeline, and
terminal setup/restore hardening. Non-Windows terminals with keyboard-event
enhancement use direct `Y`/`Enter` actions; Windows uses `Y` followed by
non-text `F8` for approval and `F8` for input submission. Other terminals keep
the basic TUI but reject approval and input requests without opening a modal.
The opt-in PTY smoke is implemented for Unix standard-library PTYs. Windows
returns an explicit exit-code-77 skip because native ConPTY automation is not
included; that skip is not interoperability evidence. Multi-session tabs,
background task management, and mouse interaction remain future scope.

### Historical design

The numbered documents under `docs/` and handoff files preserve design history.
Use them for rationale, not as current API/runtime truth when they disagree with
`docs/runtime/`.

## 20. Known boundary reminders

- Browser/Desktop automation workspace specs are future. The Web product shell
  and C0–C3 persistence/continuity/Settings/migration/polish implementation
  exist on `main`; the Tauri Desktop product host is also on `main` through
  PR #30 with current-platform Windows evidence.
- Hosted multi-user identity and distributed rate limiting are outside the MVP.
- Built-in vector RAG is not provided.
- Real provider/MCP tests are gated.
- Streamable HTTP live catalog refresh and rich result mapping across all three
  transports are implemented. Stdio/deprecated SSE do not consume live
  `listChanged`; real third-party MCP interoperability remains unverified.
- The runtime compiles versioned AgentDefinition packages and discovers bounded
  root/nested `AGENTS.md` instruction layers after Project Trust authorization.
- Typed procedure catalog selection and bounded hydration run at activation;
  phase-specific Planner/Evaluator/Finalizer consumption and the deterministic
  OnCall reference benchmark are implemented; external-provider and holdout
  evidence remain optional.

These are boundaries, not reasons to describe the implemented MVP as absent.

## 21. Troubleshooting

### Cargo lock or build contention

- Check for another Cargo process before assuming a deadlock.
- Prefer focused tests while iterating.
- Do not delete `target/` as a first response.

### Port already in use

Use `scripts/dev.ps1` custom `-ApiAddr` and `-WebPort`.

### Web cannot reach API

Check:

- API address;
- `ROVE_API_BASE` in the Next.js server environment;
- token configured on both API and Web server;
- browser is calling the Web proxy, not exposing a provider key.

### Real provider test skipped

This is expected unless the documented gate and credentials are present. Run
deterministic tests regardless.

### MCP optional smoke skipped

This is expected unless `ROVE_MCP_FILESYSTEM_SMOKE=1` and its external
dependencies are available. `cargo test -p rove-integration-tests --test mcp`
remains the default local
contract.

### Dirty tree

Inspect each path. Do not reset user files. Scope your patch and report
unrelated changes at handoff.

## 22. First contribution checklist

- [ ] I read `AGENTS.md` and the relevant current runtime docs.
- [ ] I inspected the dirty tree.
- [ ] I reproduced or defined the requested behavior.
- [ ] I added/updated focused tests for implementation work.
- [ ] I preserved workspace, safety, event, state, and secret boundaries.
- [ ] I ran proportional verification.
- [ ] I updated current docs only for implemented behavior.
- [ ] I marked future designs as proposed.
- [ ] I checked generated artifacts and secrets.
- [ ] I can explain what was not tested and why.

## 23. Handoff template

Keep the handoff concise and evidence-based:

```text
Outcome
- What now works or what document was completed

Changed
- Files and contracts

Verified
- Exact commands and results

Not run
- Gate and reason

Notes
- Compatibility, risk, follow-up, dirty-tree ownership
```

The best onboarding outcome is not memorizing the repository. It is knowing
where current truth lives, how a request travels through the shared runtime,
which boundaries must not be weakened, and what evidence is required before
claiming a change is complete.
