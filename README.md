# rove

`rove` is a local-first, stateful agent runtime written in Rust. It provides a CLI (including an optional full-screen TUI), an HTTP API, a standalone web workbench, tool execution, resumable run state, layered memory, provider routing, and tool-based workspace retrieval.

The repository is a virtual Cargo Workspace. Its implemented dependency and
product flow is:

```text
apps/web --HTTP/SSE--> apps/api

apps/cli / apps/api / apps/bench
    -> apps/bootstrap      product configuration and assembly
        -> runtime         durable Engine, state, tools, memory, planning
            -> core        in-memory Agent loop and tool contracts
                -> models  normalized providers, routing, fake model
```

## Current MVP

As of 2026-07-24, the modular Workspace migration and Provider Layer redesign
are on `main`. rove has reached its local-first MVP: CLI, API, Web workbench,
streaming events, bounded tool execution, persisted state, resume, deterministic
benchmarks, and current runtime docs are all present. The exact boundary is
documented in [docs/runtime/mvp-definition.md](docs/runtime/mvp-definition.md).

Browser/Desktop workspaces, hosted multi-user identity, distributed rate
limiting, and optional external semantic retrieval are outside this MVP.

## Quick Start

Start the interactive terminal REPL without network credentials:

```bash
cargo run -p rove-cli -- --model fake
```

Start the full-screen TUI without network credentials. This command requires
an interactive terminal:

```bash
cargo run -p rove-cli -- tui --model fake
```

Approval and input modals additionally require reliable key events. Non-Windows
terminals with Crossterm keyboard enhancement use direct `Y`/`Enter` actions. Windows
uses a non-text `F8` confirmation boundary (`Y` selects approval, `F8` confirms;
`F8` submits input), because its backend cannot reliably identify pasted text.
On an unsupported terminal the rest of the TUI continues to run, but approval
is rejected and `request_input` returns an unavailable error without opening a
modal.

While idle, `Ctrl+R` opens the bounded resume picker; `Ctrl+T` opens completed
or failed tool details; `F1` shows help generated from the active keymap. The
transcript follows a bounded visible timeline in canonical event-delivery order
while the existing trace, task state, report, and SQLite artifacts remain the
durable run facts. The optional real-terminal smoke uses a Unix PTY:

```bash
python scripts/tui-pty-smoke.py --run
```

Windows reports an explicit exit-code-77 skip because the repository does not
yet include native ConPTY automation; that skip is not a passing result.

Start the same REPL with an initial prompt:

```bash
cargo run -p rove-cli -- --model fake "echo hello from rove"
```

Run a non-interactive exec prompt and exit:

```bash
cargo run -p rove-cli -- exec --model fake "echo hello from rove"
```

Start the local API and Web workbench together in fake-provider mode:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

Open `http://localhost:3000`. The launcher starts `rove-api`, starts the
Next.js workbench, prints the API/Web URLs, and stops both process trees when it
exits. Use custom ports when the defaults are busy:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -ApiAddr 127.0.0.1:18787 -WebPort 3001
```

Start the same API/Web flow with a real OpenAI provider by setting
provider environment first. This can be an official API, a relay, or a gateway
that exposes an OpenAI Chat Completions `/v1` API:

```powershell
$env:ROVE_PROVIDER = "openai"
$env:ROVE_MODEL = "<model-id>"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -Provider
```

The Web workbench can also override the provider per run. It supports runtime
default, OpenAI, OpenAI Responses, Anthropic, Ollama, and fake
profiles without sending raw provider keys from the browser.

Run in an isolated standalone Task workspace:

```bash
cargo run -p rove-cli -- --task-workspace invoice-check --task-base .rove/tasks --model fake "review this task"
```

Task workspaces keep their files, `.rove` state, and default memory under the
task directory. Remove that directory when the isolated task is no longer
needed.

Inspect effective configuration:

```bash
cargo run -p rove-cli -- dump-config
```

List resumable local task states:

```bash
cargo run -p rove-cli -- sessions
```

Run deterministic local benchmark tasks:

```bash
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

Manual API/Web startup remains available. Start the local API server:

```bash
cargo run -p rove-api
```

Start the web workbench in another shell:

```bash
cd apps/web
pnpm install --frozen-lockfile
pnpm dev
```

By default the API binds to `127.0.0.1:8787`, and the web workbench proxies `/api/*` to that local API.
Generated HTTP API reference is available from the API server at `/swagger-ui`
and `/api/openapi.json`.
For token-protected API deployments, set the same `ROVE_API_TOKEN` in the Rust
API environment and the Next.js server environment. The web proxy injects the
bearer token server-side and does not expose it to browser JavaScript.

## Main Entry Points

| Area | Path | Purpose |
|---|---|---|
| CLI / TUI | `apps/cli/` | Rich terminal REPL, full-screen TUI, exec, config dump, and sessions. |
| API | `apps/api/` | HTTP job lifecycle, SSE event streaming, approvals, inputs, cancellation, and provider inventory/test. |
| Benchmarks | `apps/bench/`, `benchmarks/` | Deterministic no-network benchmark tasks with artifact-path reports. |
| Bootstrap | `apps/bootstrap/` | First-party AppConfig, provider factory, product registry, shared Engine assembly. |
| Web | `apps/web/` | Next.js workbench that consumes the API and SSE job stream. |
| Agent core | `core/` | Independent `rove-core` crate: in-memory Agent/model/tool loop and tool contracts. |
| Persistent runtime | `runtime/` | Independent `rove-runtime` crate: durable execution, tools/MCP, planning, Engine, state/memory. |
| Models | `models/` | Independent `rove-models` crate: normalized protocol and provider adapters. |
| Integration tests | `tests/` | Cross-package contracts package `rove-integration-tests`. |
| Docs | `docs/runtime/` | Current architecture, subsystem boundaries, and implementation status. |
| History | `docs/Archive/` | Early designs, plans, handoffs, and comparisons. |
| Maintainers | `AGENTS.md`, `docs/ONBOARDING.md` | Repository rules, source-of-truth order, code map, workflows, and verification. |


## Workspace retrieval and memory

rove obtains workspace context through tools (`fs_read`/`fs_write`, `shell`) and layered file memory (session + durable `MEMORY.md` / topics). There is **no built-in vector database or embedding index** in the default product.
## Configuration

Configuration is layered as:

```text
defaults < .rove/config.toml < environment < CLI overrides
```

Common environment variables:

| Variable | Purpose |
|---|---|
| `ROVE_MODEL` | Primary model override. Use `fake` for local deterministic smoke runs. |
| `ROVE_PROVIDER` | Provider name: `openai`, `openai-responses`, `anthropic`, `ollama`, or `fake`. |
| `OPENAI_API_KEY` | OpenAI API key (or compatible relay). |
| `OPENAI_API_BASE` | OpenAI API base URL (or compatible relay). |
| `ANTHROPIC_API_KEY` | Anthropic API key. |
| `ROVE_API_BIND_ADDR` | API bind address override. Defaults to `127.0.0.1:8787`. |
| `ROVE_API_TOKEN` | Bearer token required by the Rust API and injected by the Web proxy when set server-side. |
| `ROVE_API_BASE` | Web proxy upstream API URL. Defaults to `http://127.0.0.1:8787`. |
| `ROVE_WEB_PORT` | Next.js workbench port used by `scripts/dev.ps1` and Playwright. Defaults to `3000`. |
| `PLAYWRIGHT_BASE_URL` | Browser E2E base URL override. Defaults to `http://localhost:$ROVE_WEB_PORT`. |
| `ROVE_FALLBACK_MODELS` | Comma-separated fallback model list using the primary provider. |
| `ROVE_ROUTING_RETRY_MAX_ATTEMPTS` | Routed provider attempts before fallback. Defaults to `1`. |
| `ROVE_ROUTING_RETRY_BACKOFF_BASE_MS` | Base retry backoff for routed providers. Defaults to `250`. |
| `ROVE_ROUTING_RETRY_BACKOFF_MAX_MS` | Maximum retry backoff for routed providers. Defaults to `5000`. |
| `ROVE_MODEL_COMPACTION_ENABLED` | Enable optional model-generated prompt compaction. Defaults to `false`. |
| `ROVE_COMPACTION_FAILURE_THRESHOLD` | Consecutive model-compaction failures before circuit-open fallback. Defaults to `3`. |
| `ROVE_SHELL_TIMEOUT_MS` | Shell command timeout. Defaults to `30000`. |
| `ROVE_SHELL_MAX_OUTPUT_BYTES` | Max captured stdout/stderr bytes per stream. Defaults to `65536`. |
| `ROVE_SHELL_INHERIT_ENVIRONMENT` | Whether shell commands inherit the process environment. Defaults to `true`. |
| `ROVE_SHELL_DENYLIST` | Comma-separated shell command substrings to reject before execution. |

`dump-config` prints the effective config, source summary, resolved paths, and secret-redacted provider fields.

## State Layout

Runtime state is written under `.rove/` by default:

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

Files are the readable artifacts. SQLite is the index used for listing, replay, and restart-aware API job state. `rove state repair` can rebuild the index from task, trace, and report artifacts, skipping corrupted trace lines instead of aborting the whole repair.

`memory.session_dir` and `memory.durable_dir` can move session summaries and durable memory away from the default `.rove/memory` layout. Session summaries are deterministic markdown with the goal, final status, output excerpt, completed plan steps, tools used, and reported file changes.


## Verification

Default Rust and web checks:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd apps/web
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
```

Deterministic benchmark checks:

```bash
cargo test --test bench
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

## Runtime Docs

Start here:

- [Runtime Architecture](docs/runtime/architecture.md)
- [Plan plus ReAct Runtime Loop](docs/runtime/react-loop.md)
- [Subsystem Design](docs/runtime/subsystems.md)
- [Implementation Status](docs/runtime/implementation-status.md)
- [Acceptance Matrix](docs/runtime/acceptance-matrix.md)
- [Benchmark Evidence](docs/runtime/benchmark-evidence.md)

Current runtime source of truth is the `docs/runtime/` directory. Older design
notes and implementation plans remain in `docs/`, `docs/design/`, and
`docs/plans/` as historical context unless they explicitly point back to the
runtime docs.
