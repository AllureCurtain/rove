<p align="center">
  <img src="apps/desktop/icons/128x128.png" width="88" height="88" alt="Rove icon">
</p>

<h1 align="center">Rove</h1>

<p align="center">
  <strong>A local-first coding agent you can inspect, interrupt, resume, and trust.</strong>
</p>

<p align="center">
  One durable runtime behind Desktop, Web, CLI, API, and deterministic evaluation.
</p>

<p align="center">
  <a href="https://github.com/AllureCurtain/rove/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/AllureCurtain/rove/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Version" src="https://img.shields.io/badge/version-0.1.0-2563eb">
  <img alt="Desktop" src="https://img.shields.io/badge/Desktop-Windows%20verified-0f766e">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24c8db">
  <img alt="Next.js" src="https://img.shields.io/badge/Next.js-16-111111">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-runtime-b7410e">
</p>

<p align="center">
  <a href="#one-minute-overview">Overview</a>
  · <a href="#core-workflow">Workflow</a>
  · <a href="#what-ships-today">Capabilities</a>
  · <a href="#get-started">Get started</a>
  · <a href="#architecture">Architecture</a>
  · <a href="#documentation">Documentation</a>
</p>

## One-minute overview

Rove is a local-first Agent product for repository work. You choose a workspace,
ask for a task, watch the run as it happens, approve sensitive operations, and
review the resulting files, diffs, artifacts, usage, and evidence.

The product surfaces are different views over the same runtime. Desktop embeds
the same authenticated API and static Web application; Web, CLI, TUI, API, and
benchmarks do not maintain separate Agent loops or competing state.

| Project fact | Current state |
|---|---|
| Version | <code>0.1.0</code>, source-first pre-release |
| Product surfaces | Tauri Desktop, Next.js Web, CLI/REPL/TUI, HTTP/SSE API |
| Local evaluation | Deterministic fake provider, no key or network required |
| Model providers | OpenAI Chat, OpenAI Responses, Anthropic, Ollama, Fake, external adapter |
| Workspaces | Explicit Folder, Repo, and isolated Task roots |
| Durable state | SQLite index plus readable trace, task state, report, artifacts, and Markdown memory |
| Desktop evidence | Windows MSI/NSIS build and release-process smoke |
| Accounts and cloud | No hosted account, billing, or Rove cloud service |

Rove is currently best suited to local development and controlled dogfooding.
It is not yet a signed, generally available desktop release.

## Current MVP

The current MVP is implemented across the shared runtime and its first-party
surfaces. The sections below describe what is available in the current source;
release and external-interoperability limits remain explicit under
[Current boundaries](#current-boundaries).

## Who it is for

| You need to | Rove provides |
|---|---|
| Work on a real repository without losing control | Exact workspace roots, Project Trust, approvals, bounded tools, and observable mutations |
| Continue after a disconnect or restart | Canonical events, resumable task state, checkpoints, and fail-closed reconciliation |
| Understand what the Agent actually did | Transcript, plan and tool activity, changed files, diffs, artifacts, usage, and redacted evidence export |
| Use different model endpoints | Named provider profiles, protocol-normalized adapters, model discovery, connectivity tests, routing, and fallback |
| Run without credentials or network | Fake-provider CLI, integration, browser, and benchmark paths |
| Extend tool access safely | One authoritative Tool Registry plus stdio, legacy SSE, and Streamable HTTP MCP |

## Core workflow

| Step | User action | Product behavior |
|---|---|---|
| Step | User action | Product behavior |
|---|---|---|
| 1. Open | Select a Folder or Repo | Rove binds an exact canonical workspace root |
| 2. Trust | Review project capabilities | Project config, instructions, MCP, hooks, credentials, and external paths remain deferred until explicitly granted |
| 3. Configure | Choose Fake or a provider profile | Raw keys stay in the server/Desktop process environment, never browser state |
| 4. Ask | Send a task in Chat, CLI, or TUI | The shared Engine selects the profile, plan, budgets, tools, and immutable run identity |
| 5. Control | Approve, answer, steer, queue follow-up work, stop, or resume | Every control crosses the same durable runtime boundary |
| 6. Review | Inspect the answer, diff, files, artifacts, and evidence | Canonical events remain the source for persistence and product projections |

## What ships today

### Agent execution

- One Runtime-neutral Core Agent kernel drives embedded, unplanned, and
  planned-step model/tool coordination.
- Rule-first plan decisions can use bounded model evaluation only for typed
  ambiguity; deterministic fallback remains available.
- An independent evidence-grounded Finalizer reports success, partial,
  blocked, rejected, cancelled, exhausted, indeterminate, and failed outcomes
  without relabeling them as complete.
- Multidimensional execution budgets, immutable plan revisions, StepRecords,
  trace-tail reconciliation, cancellation, and resume are part of the durable
  lifecycle.

### Workspace intelligence and tools

- Bounded ranged reads, deterministic search/glob/list continuation, observed
  writes and edits, workspace checkpoints, artifact projection, and foreground
  or background shell execution.
- Exact path, traversal, symlink/reparse, timeout, output, approval, and
  capability checks remain Runtime-owned.
- Versioned AgentDefinition packages, trusted root/nested <code>AGENTS.md</code>
  discovery, immutable run profiles, and typed procedure selection/hydration.
- Planner, StepRunner, Evaluator, and Finalizer consume bounded procedure
  material; the OnCall V2 suite evaluates behavior against independent truth
  and hard safety gates.

### Product continuity and control

- API-authoritative workspaces, sessions, preferences, provider profiles, and
  exact product-session/runtime bindings.
- Streaming canonical transcript restore with explicit loading, partial,
  failure, retry, reconnect, and background-attention states.
- One durable Send Message lifecycle: an active-run message queues in FIFO order
  and can be promoted or revoked without changing identity; idle delivery claims
  the successor turn. Legacy Steer/Follow-up routes remain compatibility-only.
- Terminal-boundary session Fork, per-session model/reasoning/approval/step-limit
  snapshots, and single-active-turn ownership.
- Memory, runtime health, workspace files, images, artifacts, run/Git diff,
  usage/context/cost, and JSON/HTML/Markdown evidence views.

### Providers, MCP, and artifacts

- Provider-specific payloads stay behind normalized model messages, tool calls,
  usage, and errors.
- OpenAI Chat, OpenAI Responses, Anthropic Messages, Ollama, Fake, and an
  opt-in external process adapter share the first-party factory.
- MCP supports stdio, deprecated HTTP+SSE, and negotiated Streamable HTTP.
- Rich tool results use bounded content blocks and canonical durable Tool
  Artifacts. Streamable HTTP catalogs can refresh for future runs while active
  runs retain pinned tool bindings.

## Get started

### Requirements

- Git
- Rust stable from [rust-toolchain.toml](rust-toolchain.toml)
- Node.js 22 and pnpm 10 for Web/Desktop development

### Try the CLI with no provider key

~~~powershell
cargo run -p rove-cli -- --model fake "echo hello from rove"
~~~

Start the full-screen TUI:

~~~powershell
cargo run -p rove-cli -- --model fake
~~~

Start the line-oriented REPL:

~~~powershell
cargo run -p rove-cli -- repl --model fake
~~~

Run a non-interactive prompt:

~~~powershell
cargo run -p rove-cli -- exec --model fake "inspect this workspace"
~~~

### Run the Web product

Install the locked Web dependencies once:

~~~powershell
cd apps/web
pnpm install --frozen-lockfile
cd ../..
~~~

Start the local API and Web application together in fake-provider mode:

~~~powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
~~~

Open <http://localhost:3000>. The API listens on
<http://127.0.0.1:8787>; OpenAPI and Swagger UI are available at
<http://127.0.0.1:8787/api/openapi.json> and
<http://127.0.0.1:8787/swagger-ui>.

### Run Desktop

From <code>apps/desktop</code>:

~~~powershell
pnpm dlx @tauri-apps/cli@2 dev
~~~

Build Windows packages:

~~~powershell
pnpm dlx @tauri-apps/cli@2 build --target x86_64-pc-windows-msvc
~~~

Desktop is a thin Tauri host. It starts the shared API on a random loopback
port, injects a persistent local bearer token before page scripts run, and
loads the same static Web product. It does not own a second Engine or
ProductStore.

### Configure a real provider

Rove configures real providers through the user-owned catalog at
`~/.rove/config.toml`. Choose a provider type, endpoint, model, and credential
reference; never put the credential value in this file.

~~~toml
schema_version = 1

[model]
default_profile = "openai-main"
default_model = "gpt-4.1-mini"
reasoning = "default"

[provider.profiles.openai-main]
label = "OpenAI"
provider_type = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4.1-mini"
auth = { style = "bearer", secret = { env = "OPENAI_API_KEY" } }
~~~

Normal CLI/TUI startup uses that default without `--model`. With no configured
Provider it reports onboarding instead of silently choosing Fake. In `rove tui`,
`/model` opens the configured-model picker; the selection applies to the next
turn only. Use `--model fake` for an explicit deterministic no-network run.

For Web development, export the key before starting the API:

~~~powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -Provider
~~~

Then open **Settings -> Providers**, create a profile, set the key reference to
<code>OPENAI_API_KEY</code>, test the endpoint/model, and activate the profile.
Official services and compatible gateways use the same profile types; only
endpoint, model, and credential reference differ. Settings mutates the same
user catalog with revision/CAS protection; the browser never receives the key.

For the installed terminal product, configure a named provider profile once,
then launch the full-screen interface from the directory to work in:

~~~powershell
cargo install --path apps/cli
cd D:\path\to\workspace
rove
~~~

`cargo install --path apps/cli` builds and installs the `rove` executable; it
does not install this workspace's Rust dependency libraries. The selected real
provider and model come from the normal configuration layers, so `rove` does
not require a repeated `--model` flag. Keep credentials in referenced
environment variables, not in `.rove/config.toml`.

See [Provider smoke testing](docs/runtime/provider-smoke.md) for CLI/config
examples and honest external-service gates.

## Architecture

~~~text
Tauri Desktop ---+
Next.js Web -----+-- HTTP / SSE --> rove-api --+
                                               |
CLI / TUI -------------------------------------+
Benchmarks ------------------------------------+
                                               v
                         rove-app-bootstrap
                         config, trust, assembly
                                    |
                                    v
                             rove-runtime
                 Engine, planning, state, tools, memory
                                    |
                                    v
                               rove-core
                   Agent kernel and Tool Registry contract
                                    |
                                    v
                              rove-models
                  normalized providers, routing, fake model
~~~

The package direction is intentionally one-way:

~~~text
rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap
                                               ^
                                               |
                         rove-cli / rove-api / rove-bench
                                      ^
                                      |
                                rove-desktop
~~~

## Data and safety

Rove is local-first, not permission-free.

- The API binds to loopback by default; remote binding requires bearer
  authentication unless an explicit unsafe override is supplied.
- Project-owned config, instructions, MCP processes, hooks, credential
  selectors, and external paths are restricted until the exact workspace and
  capability digest are trusted.
- Tool descriptions, prompts, procedures, MCP annotations, and model output
  cannot grant approval or capability.
- Workspace paths are resolved at execution boundaries. Remote names, URIs,
  MIME metadata, and artifact IDs never become trusted local paths.
- Secrets are excluded from browser requests, normal logs, traces, reports,
  evidence, screenshots, and committed fixtures.

Default durable runtime data lives under the user data root. Workspace state
is isolated in `<data_root>/workspaces/<storage_key>/`; the ProductStore is
API-global at `<data_root>/product.sqlite`. Windows
`%LOCALAPPDATA%\rove`, macOS `~/Library/Application Support/rove`, Linux
`$XDG_DATA_HOME/rove`; override with `ROVE_DATA_ROOT`. Run `rove state
paths` to inspect the resolved locations and `rove state migrate` to import
a legacy project-local `.rove/`. Project directories keep only trust-gated
project configuration. See [STATE_LAYOUT_AND_MIGRATION.md](STATE_LAYOUT_AND_MIGRATION.md).

~~~text
<data_root>/
  product.sqlite                 # API-global ProductStore
  workspaces/<storage_key>/
    workspace.json
    state.sqlite
    mcp_servers.json
    runs/<run_id>/trace.jsonl
    runs/<run_id>/task_state.json
    runs/<run_id>/report.json
    runs/<run_id>/tool_artifacts/
    memory/MEMORY.md
    memory/topics/
    memory/sessions/
~~~

<code>trace.jsonl</code> records canonical event facts,
<code>task_state.json</code> records resumable state, and
<code>report.json</code> is a derived summary. The report is not the only
durable truth.

## Verification

Rust:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
~~~

Web:

~~~powershell
cd apps/web
pnpm test
pnpm typecheck
pnpm build
pnpm test:e2e
~~~

Aggregate local acceptance writes real exits to
<code>PRODUCT_ACCEPTANCE_REPORT.json</code>:

~~~powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1
~~~

The final full-delivery report passed all 11 required checks. Provider, real
filesystem MCP, platform packaging, and other opt-in gates remain separate;
skipped evidence is never counted as interoperability.

## Current boundaries

| Area | Current boundary |
|---|---|
| Distribution | Windows MSI/NSIS packages have been built and the release process launched; public signing, installation, and general release are not complete |
| Platforms | Linux CI compiles/tests Desktop; macOS/Linux package and interactive evidence remain unverified |
| Providers | Deterministic and protocol tests pass; the final external-provider interoperability gate was not run |
| MCP | Mock transports and negotiated behavior are covered; the optional official filesystem smoke and broader third-party interoperability remain unrun |
| Shell isolation | Local shell is policy-bounded and approval-gated, not container/seccomp/user-namespace sandboxed |
| Retrieval | Workspace tools plus layered Markdown memory; no built-in vector database, embedding index, or semantic RAG |
| Delegation | Tool batches can run safe calls concurrently; product subagents are not implemented |
| PTY | Native PTY remains a typed unsupported execution capability |
| Hosted product | No multi-user identity, billing, device sync, remote execution gateway, or distributed rate limiting |

## Documentation

Current runtime source of truth is [docs/runtime/](docs/runtime/). A proposed
design is not evidence that the runtime supports it.

| Read this | For |
|---|---|
| [Maintainer onboarding](docs/ONBOARDING.md) | Repository map, source-of-truth order, development workflow, and verification |
| [Runtime documentation](docs/runtime/README.md) | Current implementation map and supported behavior |
| [Architecture](docs/runtime/architecture.md) | Package ownership and cross-surface boundaries |
| [Plan plus ReAct loop](docs/runtime/react-loop.md) | Current execution behavior, tool turns, planning, and resume semantics |
| [Runtime subsystems](docs/runtime/subsystems.md) | Trust, config, providers, tools, state, memory, MCP, API, Web, and Desktop |
| [Implementation status](docs/runtime/implementation-status.md) | Implemented contracts and remaining gaps |
| [Acceptance matrix](docs/runtime/acceptance-matrix.md) | Evidence commands and optional-gate classification |
| [Integration testing](docs/runtime/integration-testing.md) | Local, browser, provider, MCP, PTY, and stress gates |
| [Release readiness](docs/runtime/release-readiness.md) | Release-oriented evidence and security checklist |
| [Productization program](docs/plans/2026-08-10-post-full-delivery-productization.md) | Implemented A-E/F.1-F.3 record and remaining F.4/F.5/G gates |
| [Repository rules](AGENTS.md) | Engineering invariants and coding-agent instructions |

Historical designs and early plans remain under [docs/Archive/](docs/Archive/).
The dated 2026-08-09 audit documents are supporting evidence for the
productization program, not independent implementation plans.
