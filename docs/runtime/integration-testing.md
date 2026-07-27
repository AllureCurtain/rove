# Integration Testing

This document defines the first full end-to-end integration profile for rove and the optional gates that extend it to real providers and MCP. The goal is to prove the product's own runtime loop before adding external-service variability.

## Profiles

| Profile | Required by first round | External dependencies | Purpose |
|---|---:|---|---|
| `local-full` | Yes | No | Proves fake provider, real API, built-in tools, persistent state, the default `/` product shell, and one bounded advanced `/dev/workbench` smoke. |
| `provider-smoke` | No | Provider key, network, or local Ollama | Proves a configured real provider can answer and perform one native tool-use round trip. |
| `provider-integration` | No | Provider key/network, except local Ollama | Proves a real OpenAI, OpenAI Responses, Anthropic, or Ollama provider through inventory, provider smoke, API jobs, an optional exact product-shell job, and saved evidence. The updated external-provider browser gate has not been run for C3. |
| `external-tools` | No | MCP server configuration | Proves configured external tools can be discovered, called, and shown in API/Web records. |
| `stress` | No | Depends on selected profile | Later profile for concurrent runs, long-running jobs, repeated resume, and restart recovery. |

The first integration baseline is `local-full`. Real model providers, MCP servers, and stress tests are gated follow-ups, not first-round blockers.

## Required Local Files

Use these templates as the starting point:

- `.rove/config.example.toml`
- `.rove/mcp_servers.example.json`
- `.env.integration.example`

Copy them to local-only files before running a manual integration pass:

```powershell
Copy-Item .rove/config.example.toml .rove/config.toml
Copy-Item .rove/mcp_servers.example.json .rove/mcp_servers.json
Copy-Item .env.integration.example .env.integration
```

Do not commit `.rove/config.toml`, `.rove/mcp_servers.json`, `.env.integration`, real API keys, SQLite state, logs, screenshots, traces, or generated run artifacts.

## Isolation

The `local-full` profile must not write into the normal `.rove` runtime state. By default, the runner uses `%TEMP%/rove-integration` on Windows. Prefer an absolute integration root outside the repository; if the workspace lives under this Git repo, workspace detection can walk upward to the repo root.

| Artifact | Path |
|---|---|
| Runtime state | `<integration-root>/workspace/.rove-integration-state` |
| SQLite index | `<integration-root>/workspace/.rove-integration-state/state.sqlite` |
| Memory | `<integration-root>/workspace/.rove-integration-state/memory` |
| Test workspace | `<integration-root>/workspace` |
| Logs, screenshots, traces | `<integration-root>/artifacts` |

The runner removes or recreates `<integration-root>/workspace` at the start of a clean run, then preserves `<integration-root>/artifacts` for debugging.

## `local-full` Scope

`local-full` runs against:

- provider `fake`
- model `fake` or `fake-raw` depending on the scenario
- real `rove-api`
- real `apps/web`
- built-in tools from the runtime registry
- local filesystem workspace rooted under `<integration-root>/workspace`

It must prove this flow:

```text
Web/API request
  -> API job creation
  -> Engine run
  -> fake provider event stream
  -> built-in tool call or pending interaction
  -> approval/input resolution
  -> run completion or expected failure
  -> trace/report/task state persisted
  -> exact product-session transcript/report binding is verified
  -> bounded /dev/workbench direct-run smoke remains available
```

## Required Scenarios

| Scenario | Request shape | Expected evidence |
|---|---|---|
| Plain run | `{"message":"local-full plain run","model":"fake","approval":"auto"}` | Job completes and `/runs` includes the exact done run. |
| Tool run | `{"message":"{\"tool\":\"echo\",\"args\":{\"message\":\"hello local-full\"}}","model":"fake-raw","approval":"auto"}` | Tool lifecycle events appear, final text includes the echo result, run detail/report records the tool step. |
| Approval approved | `{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"approved.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"ask","max_steps":1}` | Job exposes one pending `write_file` approval, approve resumes the run, file is written inside the integration workspace, `/runs` reports done. |
| Approval rejected | Same as approval approved with `rejected.txt`, then reject | Job records the rejected tool decision, no file is written, terminal state is visible and explainable in API/Web. |
| Input resume | `{"message":"{\"tool\":\"request_input\",\"args\":{\"prompt\":\"Which branch should I use?\"}}","model":"fake-raw","approval":"auto","max_steps":1}` | Job exposes pending input, answer submission resumes the run, pending input clears, final output includes or reflects the supplied answer. |
| Failure record | A request that triggers a known tool failure, such as an invalid filesystem path inside the isolated workspace policy | API state and Web detail show the failed tool result or failed run status with diagnostic text. |
| History consistency | After all scenarios | `/runs`, `/runs/{run_id}/report`, product transcript bindings, and Web assertions agree on exact run ids, statuses, and important tool/input/approval records. |
| Product migration | Seed safe M1 browser state, then open a legacy deep route | Migration completes before product catalog reads, imports no raw key, remaps the route, and does not replay after refresh. |
| Exact product continuity | Interleave turns in product sessions A and B, refresh A, then continue A | A resumes its own exact prior run rather than B's workspace-global latest; transcript, approval, input, cancellation, Settings, and deep routes remain usable. |

## Manual API Smoke Commands

These commands are useful before a runner exists. Start the API in one terminal:

```powershell
$env:ROVE_PROVIDER = "fake"
$env:ROVE_MODEL = "fake"
$root = Join-Path $env:TEMP "rove-integration"
$workspace = Join-Path $root "workspace"
$state = Join-Path $workspace ".rove-integration-state"
$env:ROVE_STATE_DIR = $state
$env:ROVE_STATE_SQLITE = Join-Path $state "state.sqlite"
$env:ROVE_MEMORY_SESSION_DIR = Join-Path $state "memory/sessions"
$env:ROVE_MEMORY_DURABLE_DIR = Join-Path $state "memory"
cargo run --bin rove-api -- --addr 127.0.0.1:8787 -C $workspace
```

Create a plain job:

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs `
  -ContentType application/json `
  -Body '{"message":"local-full plain run","model":"fake","approval":"auto"}'
```

List historical runs:

```powershell
Invoke-RestMethod http://127.0.0.1:8787/runs
```

### Generated API Reference

Fetch the generated OpenAPI document:

```powershell
Invoke-RestMethod http://127.0.0.1:8787/api/openapi.json
```

Open Swagger UI in a browser:

```text
http://127.0.0.1:8787/swagger-ui
```

If `api.token_auth` is configured, use Swagger UI's authorize control or pass
`Authorization: Bearer <token>` for business API calls.

For pending approval and input scenarios, poll job state:

```powershell
Invoke-RestMethod http://127.0.0.1:8787/jobs/<job_id>/state
```

Then submit a decision or answer:

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs/<job_id>/approvals/<call_id> `
  -ContentType application/json `
  -Body '{"decision":"approve"}'

Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs/<job_id>/inputs/<input_id> `
  -ContentType application/json `
  -Body '{"answer":"main"}'
```

## Manual Web Smoke

Start the Web application in another terminal:

```powershell
cd apps/web
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm dev
```

Open `http://localhost:3000` for the default M1 product shell. Open
`http://localhost:3000/dev/workbench` only for the advanced direct-run/history
surface. The product-shell manual pass criteria are:

- creating a job from the UI works against the real API, not a mocked API;
- live status updates appear from the SSE stream;
- pending approvals and inputs are visible and actionable;
- the selected product session and Inspector show completed, interrupted,
  rejected, and failed states as applicable;
- the Inspector shows tool names, arguments or summaries, and final status;
- Playwright screenshots and traces are retained under `<integration-root>/artifacts` when automated.

## Runner

`scripts/integration-smoke.ps1` runs the `local-full` profile:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

Run the same local-full profile on non-default ports when `8787` or `3000` is
busy:

```powershell
$root = Join-Path $env:TEMP "rove-integration-custom"
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 `
  -ApiAddr "127.0.0.1:18788" `
  -WebPort "13000" `
  -IntegrationRoot $root
```

Useful focused modes:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 -SkipWebE2E
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 -SkipApiSmoke
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 -KeepState
```

The runner:

1. Loads `.env.integration` when present and falls back to safe `local-full` defaults.
2. Verifies required commands are available: `cargo` and `pnpm`.
3. Creates `<integration-root>/workspace`, `<integration-root>/workspace/.rove-integration-state`, and `<integration-root>/artifacts`.
4. Starts `cargo run --bin rove-api -- --addr <addr> -C <workspace>` with integration env.
5. Waits until `GET /runs` succeeds.
6. Runs API smoke scenarios and saves JSON responses under `<integration-root>/artifacts/api`.
7. Starts `pnpm exec next dev --port <port>` in `apps/web` with `ROVE_API_BASE` pointing to the API.
8. Runs the gated real-API Playwright suite with `ROVE_REAL_API_E2E=1`,
   `ROVE_REAL_API_WORKBENCH_SMOKE=1`, `ROVE_WEB_PORT=<port>`, and
   `PLAYWRIGHT_BASE_URL=http://127.0.0.1:<port>`.
9. Stops API and Web processes even when a check fails.
10. Prints run ids and artifact paths.

The runner does not run `provider-smoke`, `external-tools`, or `stress`; those remain explicit follow-up gates.

The C3 `local-full` run passed all three `real-api.spec.ts` cases against the
live Rust API:

- M1 migration runs before product catalog boot and does not replay on refresh;
- the default product shell proves exact interleaved A/B continuation, refresh,
  approval, input, cancellation, Settings, and durable deep routes;
- `/dev/workbench` remains available through one bounded direct-run smoke.

## Generic Provider Runner

`scripts/provider-integration.ps1` is the provider gate for
OpenAI APIs, Anthropic, and local Ollama. It is intentionally not
tied to one vendor: the provider, base URL, API-key environment variable, model
id, ports, stress counts, restart recovery, long-soak settings, and model-list
endpoint are parameters.

The API and both Web surfaces support provider profiles at runtime. Browser
code submits `provider_type`, `api_base`, optional display `name`, and
`api_key_env` when a key is required. It cannot submit `wire_protocol`; the
system maps that diagnostic identity from `provider_type`. The Rust API reads
the named environment variable, while the raw key never enters the browser or
request payload. `POST /providers/test` verifies model inventory before a run,
and `POST /jobs` may include the same profile to route that job through OpenAI,
OpenAI Responses, Anthropic, Ollama, or Fake. Relay/gateway APIs use the
`openai` type with their own base URL.

The provider runner's optional browser step now creates or reuses an API-backed
provider profile, product workspace, and product session, persists the exact
selection, and navigates to that session in the default shell. It captures
`job_id`, `run_id`, and `resumed_from_run_id` from the browser's
`POST /api/jobs` response and verifies that exact report and transcript binding;
it does not guess a latest run by sorting IDs. This path is implemented in the
integrated C3 code but has not been executed against an external provider.

Fast provider/API-only OpenAI gate:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini" `
  -SkipWebSmoke
```

Relay or gateway provider/API-only gate:

```powershell
$env:OPENAI_API_KEY = "<relay-secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -SkipWebSmoke
```

Anthropic gate:

```powershell
$env:ANTHROPIC_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider anthropic `
  -ApiBase "https://api.anthropic.com" `
  -ApiKeyEnv ANTHROPIC_API_KEY `
  -Model "claude-3-5-haiku-latest" `
  -SkipWebSmoke
```

Ollama provider/API-only gate:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2" `
  -SkipWebSmoke
```

Omit `-SkipWebSmoke` when collecting the external-provider product-shell gate.
That run must preserve the runner's redacted Web result, exact report, and exact
product transcript artifacts. No such external-provider C3 run is currently
claimed.

Release stress with restart:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -SkipWebSmoke `
  -RunStress `
  -RunRestartRecovery `
  -StressSequentialCount 20 `
  -StressConcurrentCount 5
```

Long soak:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -SkipWebSmoke `
  -RunStress `
  -RunRestartRecovery `
  -RunLongSoak `
  -LongSoakCount 100 `
  -LongSoakDelayMs 1000
```

The runner:

1. Loads `.env.integration` without overriding existing shell variables.
2. Queries the provider-specific model inventory unless `-SkipModelInventory` is set.
3. Runs the provider-specific smoke unless `-SkipProviderSmoke` is set.
4. Starts an isolated `rove-api` and runs one plain job plus one `echo` tool job.
5. Unless `-SkipWebSmoke` is set, starts the Web application and attempts a
   real-provider `echo` run through the exact API-backed product session, then
   verifies the browser-returned job/run IDs against report and transcript data.
6. Runs sequential/concurrent provider stress only when `-RunStress` is passed,
   writes per-job state/report artifacts, and classifies failures.
7. Restarts the stress API and verifies completed run ids when
   `-RunRestartRecovery` is passed.
8. Runs a configurable long sequential soak when `-RunLongSoak` is passed.
9. Runs a configured MCP tool through API/report records only when
   `-RunExternalMcp` is passed.
10. Writes non-secret evidence under `<integration-root>/artifacts`, including
   `evidence-summary.json`.

Keep `.env.integration`, raw keys, bearer tokens, logs containing secrets, and
runtime state out of git. The runner records only `key_present`, key env names,
and provider bases, never key values.

## Real-API Playwright Design

`shell.spec.ts`, `continuity.spec.ts`, `settings.spec.ts`, `migration.spec.ts`,
and `polish.spec.ts` cover broad default-shell behavior with browser-boundary
mocks; `workbench.spec.ts` mock-covers the advanced surface. The real-API suite
is `apps/web/tests/e2e/real-api.spec.ts` and is gated by
`ROVE_REAL_API_E2E=1`. Its advanced case additionally requires
`ROVE_REAL_API_WORKBENCH_SMOKE=1`.

The real-API suite does not start the Rust API itself. The runner owns API/Web
process lifecycle so failures can preserve both logs. The suite:

- migrates safe M1 browser state before product catalog boot and verifies
  idempotent refresh behavior;
- opens `/`, creates interleaved A/B product sessions, verifies exact resume and
  refresh restore, and exercises approval, input, cancellation, Settings, and
  deep routes;
- opens `/dev/workbench` only for a bounded direct-run smoke;
- attach screenshots or traces when assertions fail.

The latest C3 `local-full` run passed these three cases. That deterministic fake
provider evidence does not substitute for the optional external-provider gate,
which has not been run.

## Optional Gates

### Full-screen TUI PTY Smoke

The TUI PTY gate is opt-in and uses only the local fake model; it never needs
provider credentials or a production endpoint. It builds `rove` when a binary
is not supplied, launches `rove tui --model fake` inside an isolated temporary
workspace, and checks a nonblank frame, a bounded resize/redraw, clean
`Ctrl+Q` exit, PTY termios restoration, and the alternate-screen,
bracketed-paste, and cursor restore sequences.

Run it explicitly from the repository root:

```powershell
python scripts/tui-pty-smoke.py --run
```

The harness is currently implemented for Unix PTYs where Python exposes
`pty`, `fcntl`, and `termios`. On Windows it emits a JSON `status: "skipped"`
with the reason that a native ConPTY runner is not yet included and exits with
code `77`; that skip is not interoperability evidence. Missing Unix PTY
modules produce the same typed skip. The gate has bounded build, runtime, and
output limits and strips provider-key-shaped variables from the child
environment. A prebuilt binary can be supplied with `--binary`; use
`--skip-build` to make a missing binary an explicit failure.

### Provider Smoke

Use `docs/runtime/provider-smoke.md` as the source of truth. With no gate enabled, `cargo test --test provider_smoke` should pass by skipping real calls. Enable one provider at a time:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_real_provider_smoke_when_enabled -- --exact --nocapture
```

Passing `provider-smoke` proves provider reachability, event normalization, and one native tool-use round trip. It does not replace `local-full`.

### External Tools

The deterministic MCP fixture is `tests/fixtures/mcp_mock_server.py`; `.rove/mcp_servers.example.json` points at it. Once copied to `.rove/mcp_servers.json`, expected tool names include:

- `mcp__mock_server__echo_remote`
- `mcp__mock_server__delete_remote`

Default verification remains:

```powershell
cargo test --test mcp
```

The official filesystem MCP smoke remains opt-in:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

## Pass Criteria

`local-full` passes only when all required scenarios have evidence from
API/state, the default product shell, and the bounded advanced smoke:

- every expected run has a run id and terminal status;
- approval and input pending records are created, resolved, and no longer pending afterward;
- filesystem writes stay inside `<integration-root>/workspace`;
- `/runs` and `/runs/{run_id}/report` agree with live job state;
- migration precedes catalog reads and does not replay after refresh;
- interleaved product sessions resume their own exact run chains after refresh;
- approval, input, cancellation, Settings, and deep routes work through `/`;
- the bounded `/dev/workbench` direct-run smoke completes;
- logs and artifacts are saved for the run.

If `local-full` passes but a gated provider or external tool profile fails, the first-round baseline still passes. Track the gated failure separately with its profile name, env configuration, and artifact path.

Passing `local-full` is the deterministic Web Complete product-shell gate. It is
not external-provider interoperability evidence.

## Pre-Integration Unit Gates

Run the deterministic code gates before a full integration pass:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

Feature and optional gates can be layered afterward:

```powershell

cd apps/web
pnpm test:e2e
```
