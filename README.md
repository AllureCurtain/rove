# rove

<div align="center">
  <strong>Local-first agent runtime for work you can inspect, resume, and control.</strong>
  <br />
  Rust runtime. CLI. HTTP/SSE API. Web product shell.
  <br /><br />
  <a href="#quick-start">Quick start</a>
  &nbsp;|&nbsp;
  <a href="#product-surfaces">Product surfaces</a>
  &nbsp;|&nbsp;
  <a href="#architecture">Architecture</a>
  &nbsp;|&nbsp;
  <a href="#documentation">Documentation</a>
</div>

`rove` is a local-first, stateful Agent runtime written in Rust. It gives the
same execution model to a terminal REPL, a full-screen TUI, a local HTTP API,
a Next.js Web product shell, and deterministic benchmarks.

The runtime keeps the things that matter for dependable Agent work explicit:
provider-neutral messages, bounded tools, approval and input gates, canonical
events, resumable state, readable artifacts, and layered memory.

## At A Glance

| | Current main |
|---|---|
| Runtime | Rust 2024 virtual Cargo Workspace |
| Interfaces | CLI/REPL/TUI, HTTP/SSE API, Next.js Web shell |
| Providers | Fake, OpenAI Chat, OpenAI Responses, Anthropic, Ollama |
| Workspaces | Folder, Repo, and isolated Task workspaces |
| Tools | Filesystem, shell, memory, request-input, and MCP proxy |
| Persistence | SQLite index plus `trace.jsonl`, `task_state.json`, `report.json`, and Markdown memory |
| Local mode | Deterministic fake-provider runs require no network or provider key |

## Current MVP

The current `main` branch includes the production Web product shell, Web
Complete C0-C3, and the merged CDH G1-G7 control/evidence/settings delivery:

- API-authoritative workspaces, sessions, provider profiles, preferences, and
  exact product-session/runtime bindings.
- Workspace -> Session -> Chat navigation with a streaming transcript, inline
  approval/input, cancellation, durable refresh restore, and a run Inspector.
- Complete Settings routes for providers, approval defaults, workspace/session
  management, Memory, runtime health, and keyboard controls.
- Fail-closed browser migration with idempotent retry and preserved deep routes.
- CLI one-shot runs, REPL, bounded full-screen TUI, resume picker, tool
  details, configuration inspection, and session listing.
- Durable run state, canonical event streams, bounded tool execution, layered
  memory, provider routing, MCP stdio plus the existing legacy SSE path, and
  deterministic benchmark evidence.
- API-authoritative Steer and durable Follow-up controls, terminal-boundary
  session Fork with inherited read-only history, and session-scoped model/
  reasoning/approval/step-limit snapshots.
- Usage/cost/context inspection, bounded workspace files and artifacts, image
  validation, run/Git diff, and redacted JSON/HTML/Markdown evidence export.
- Workspace-scoped MCP catalog management with typed probes, secret-free
  persistence, bounded transports, and fail-closed configuration errors.

The default Web shell is the primary product surface. `/dev/workbench` remains
available as a bounded advanced escape hatch for direct runtime inspection.

The deterministic `local-full` fake-provider gate covers migration,
product-session continuation and refresh, tools, cancellation, Settings, deep
routes, Steer/Follow-up, session Fork/child continuation, and one bounded
workbench smoke.
The external-provider browser gate has not been run, so no external-provider
interoperability claim is made here.

## Product Surfaces

| Surface | Use it for | Entry point |
|---|---|---|
| Web product shell | Workspace and session navigation, chat, streaming runs, approvals, Settings, Memory, and runtime health | `http://localhost:3000` |
| CLI / REPL | Fast local prompts, interactive sessions, config inspection, and resume | `cargo run -p rove-cli -- ...` |
| Full-screen TUI | Keyboard-driven transcript, approvals, input, tool details, and resume | `cargo run -p rove-cli -- tui ...` |
| HTTP API | Jobs, SSE events, approvals, inputs, cancellation, resume, provider operations, and product control | `http://127.0.0.1:8787` |
| Benchmarks | Repeatable no-network tasks with reports and artifact checks | `apps/bench/` and `benchmarks/` |

## Quick Start

### Try the runtime without a provider key

Requirements: Git, Rust stable from [`rust-toolchain.toml`](rust-toolchain.toml),
and Cargo.

```bash
cargo run -p rove-cli -- --model fake "echo hello from rove"
```

Start an interactive REPL:

```bash
cargo run -p rove-cli -- --model fake
```

Run a non-interactive prompt:

```bash
cargo run -p rove-cli -- exec --model fake "inspect this workspace"
```

Run the deterministic benchmark suite:

```bash
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

### Run the Web product shell

Requirements: Node.js compatible with the lockfile and pnpm 10.

From the repository root, start the local API and Web app together:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

Open <http://localhost:3000>. The launcher starts the API on
`127.0.0.1:8787`, starts Next.js in fake-provider mode, and stops both process
trees when it exits. Use custom ports when needed:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 `
  -ApiAddr 127.0.0.1:18787 `
  -WebPort 3001
```

To start the pieces separately:

```powershell
cargo run -p rove-api
cd apps/web
pnpm install --frozen-lockfile
pnpm dev
```

The API publishes generated OpenAPI and Swagger UI at
<http://127.0.0.1:8787/api/openapi.json> and
<http://127.0.0.1:8787/swagger-ui>.

### Use a real provider

The fake provider is the default for local evaluation. For an OpenAI-compatible
Chat Completions API, relay, or gateway:

```powershell
$env:ROVE_PROVIDER = "openai"
$env:ROVE_MODEL = "<model-id>"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -Provider
```

The Web Settings surface can manage provider profiles and model selection. Raw
provider keys stay server-side; the browser sends environment variable names,
not secret values.

## Capabilities

| Area | Implemented behavior |
|---|---|
| Execution | Planned and unplanned loops share normalized model turns, tool turns, context checkpoints, history writeback, and canonical lifecycle events. |
| Tools | Filesystem and shell access are workspace-bounded. Shell timeout, output limits, environment policy, denylist, and approval controls are configurable. |
| Human control | Tool approval, `request_input`, cancellation, resume, Steer/Follow-up, session Fork, and terminal-visible status are part of the shared runtime/product contract. |
| Providers | Provider-specific payloads stay behind the model boundary. Routing supports retries and fallback models where configured. |
| Memory | Session memory, durable `MEMORY.md`, topics, and deterministic summaries are stored as readable Markdown. |
| Retrieval | Workspace context comes from bounded filesystem/shell tools and layered memory. There is no built-in vector database or embedding index. |
| Evidence | Each run records canonical trace events, resumable task state, a derived report, and a SQLite index for listing and restart-aware access. |

## Architecture

All first-party surfaces use the same runtime assembly and execution contracts:

```text
Web product shell -- HTTP/SSE --> API

CLI / API / benchmark
        |
        v
  app-bootstrap       config, provider factory, product assembly
        |
        v
  runtime             Engine, state, tools, memory, planning
        |
        v
  core                in-memory Agent and tool loop
        |
        v
  models              normalized protocol, providers, routing, fake model
```

The important boundaries are deliberate:

- Web, API, CLI, and benchmark surfaces do not own independent Agent loops;
  they reuse the persistent Runtime. Consolidating the embedded Core loop and
  Runtime's remaining iteration mechanics into one kernel is proposed work.
- Provider payloads do not leak into core execution.
- Tool descriptions, MCP annotations, prompts, and model requests cannot grant
  permission.
- Workspace paths are resolved and bounded by the selected workspace.
- `trace.jsonl` is the event fact log, `task_state.json` is resumable state, and
  `report.json` is a derived summary.

## Durable State

By default, runtime state is written under `.rove/`:

```text
.rove/
  state.sqlite
  runs/<run_id>/trace.jsonl
  runs/<run_id>/task_state.json
  runs/<run_id>/report.json
  memory/MEMORY.md
  memory/topics/*.md
  memory/sessions/<session_id>.md
```

SQLite is the index for listing, replay, and restart-aware API job state. The
readable files remain the durable run facts. `rove state repair` can rebuild
the index from task, trace, and report artifacts.

## Configuration

Project-owned configuration is disabled until the selected workspace is
explicitly activated with CLI `--trust-project` or its canonical path is listed
in process-level `ROVE_TRUSTED_WORKSPACES`. A workspace `.env` or config file
cannot grant activation to itself. Once activated, configuration is layered in
this order:

```text
defaults < .rove/config.toml < environment < CLI overrides
```

Common variables:

| Variable | Purpose |
|---|---|
| `ROVE_PROVIDER` | `fake`, `openai`, `openai-responses`, `anthropic`, or `ollama` |
| `ROVE_MODEL` | Primary model override; use `fake` for deterministic local runs |
| `ROVE_API_BIND_ADDR` | API bind address; defaults to `127.0.0.1:8787` |
| `ROVE_API_TOKEN` | Optional bearer token for protected API deployments |
| `ROVE_API_BASE` | Web server-side proxy target; defaults to the local API |
| `ROVE_WEB_PORT` | Web port used by `scripts/dev.ps1`; defaults to `3000` |
| `ROVE_FALLBACK_MODELS` | Comma-separated fallback model list |
| `ROVE_SHELL_TIMEOUT_MS` | Shell timeout; defaults to `30000` |
| `ROVE_SHELL_MAX_OUTPUT_BYTES` | Captured output limit per stream; defaults to `65536` |
| `ROVE_TRUSTED_WORKSPACES` | OS path-list of exact workspace roots allowed to load project config and MCP |

Use `cargo run -p rove-cli -- dump-config` to inspect effective values and
secret-redacted provider fields. The complete configuration and security
behavior are documented in [runtime subsystems](docs/runtime/subsystems.md).

## Verification

Default Rust checks:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Web checks:

```powershell
cd apps/web
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
```

For changes affecting browser-visible flows, SSE, approvals, input,
cancellation, resume, or the API proxy, also run:

```powershell
pnpm test:e2e
```

The local full-stack acceptance runner is:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

The aggregate product acceptance runner records real exit codes in
`PRODUCT_ACCEPTANCE_REPORT.json`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1
```

Provider, MCP, PTY, and stress checks are opt-in. A skipped external gate is
not evidence of external interoperability. See the integration testing and
release readiness documents before making release claims.

## Current Boundaries

Implemented on `main` does not include:

- A Tauri Desktop host or Browser/Desktop automation workspace.
- Hosted multi-user identity, billing, remote gateway, device pairing, or
  distributed rate limiting.
- Full shell sandboxing beyond the current local policy controls.
- Built-in vector or provider-backed RAG retrieval.
- MCP Streamable HTTP, negotiated sessions, rich result envelopes, or Tool
  Artifacts. The current proxy supports stdio and the existing legacy SSE path.
- Versioned AgentDefinition packages, runtime `AGENTS.md` discovery, or the
  proposed OnCall reference evaluation suite.
- One shared Core/Runtime Agent kernel, Project Trust activation, the shared
  Execution Environment, Coding Tool V2, Subagents, independent Finalizer,
  model-on-ambiguity plan evaluation, or public multidimensional budgets.

These boundaries are maintained explicitly in the current runtime docs and
future designs. Worktree-only changes are not part of this README's product
claim until they are merged and verified on `main`.

## Documentation

Current runtime source of truth is [`docs/runtime/`](docs/runtime/). Proposed
work is not part of the runtime contract until implementation, tests, and these
current-state documents agree.

| Read this | For |
|---|---|
| [Runtime README](docs/runtime/README.md) | Current-state documentation map and implementation boundary |
| [MVP definition](docs/runtime/mvp-definition.md) | Included capabilities, exclusions, golden paths, and evidence baseline |
| [Architecture](docs/runtime/architecture.md) | Runtime components and ownership boundaries |
| [Plan plus ReAct loop](docs/runtime/react-loop.md) | Current execution behavior and resume semantics |
| [Subsystems](docs/runtime/subsystems.md) | Config, state, providers, tools, memory, API, MCP, and Web details |
| [Integration testing](docs/runtime/integration-testing.md) | Local-full, provider, MCP, browser, PTY, and stress gates |
| [Release readiness](docs/runtime/release-readiness.md) | Evidence and security checklist for release-oriented claims |
| [Maintainer onboarding](docs/ONBOARDING.md) | Repository map, change workflows, and verification guidance |
| [`AGENTS.md`](AGENTS.md) | Repository-wide source-of-truth and engineering rules |
| [Post-CDH implementation plan](docs/plans/2026-08-05-post-cdh-agent-kernel-and-coding-capability.md) | Active serial, two-worktree delivery order for the next kernel/coding-capability program |

Future architecture is kept under [`docs/design/`](docs/design/) and marked as
proposed when it is not implemented. Historical material lives under
[`docs/Archive/`](docs/Archive/).
