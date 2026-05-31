# Integration Testing

This document defines the first full end-to-end integration profile for rove and the optional gates that extend it to real providers, MCP, and RAG. The goal is to prove the product's own runtime loop before adding external-service variability.

## Profiles

| Profile | Required by first round | External dependencies | Purpose |
|---|---:|---|---|
| `local-full` | Yes | No | Proves fake provider, real API, real Web workbench, built-in tools, approval/input resume, persistent state, and Web history. |
| `provider-smoke` | No | Provider key, network, or local Ollama | Proves a configured real provider can answer and perform one native tool-use round trip. |
| `external-tools` | No | MCP server or RAG provider configuration | Proves configured external tools can be discovered, called, and shown in API/Web records. |
| `stress` | No | Depends on selected profile | Later profile for concurrent runs, long-running jobs, repeated resume, and restart recovery. |

The first integration baseline is `local-full`. Real model providers, MCP servers, RAG provider embeddings, and stress tests are gated follow-ups, not first-round blockers.

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
- real `web-ui`
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
  -> /runs and Web history show the record
```

## Required Scenarios

| Scenario | Request shape | Expected evidence |
|---|---|---|
| Plain run | `{"message":"local-full plain run","model":"fake","approval":"auto"}` | Job completes, `/runs` includes a done run, Web history shows the run. |
| Tool run | `{"message":"{\"tool\":\"echo\",\"args\":{\"message\":\"hello local-full\"}}","model":"fake-raw","approval":"auto"}` | Tool lifecycle events appear, final text includes the echo result, run detail/report records the tool step. |
| Approval approved | `{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"approved.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"ask","max_steps":1}` | Job exposes one pending `fs_write` approval, approve resumes the run, file is written inside the integration workspace, `/runs` reports done. |
| Approval rejected | Same as approval approved with `rejected.txt`, then reject | Job records the rejected tool decision, no file is written, terminal state is visible and explainable in API/Web. |
| Input resume | `{"message":"{\"tool\":\"request_input\",\"args\":{\"prompt\":\"Which branch should I use?\"}}","model":"fake-raw","approval":"auto","max_steps":1}` | Job exposes pending input, answer submission resumes the run, pending input clears, final output includes or reflects the supplied answer. |
| Failure record | A request that triggers a known tool failure, such as an invalid filesystem path inside the isolated workspace policy | API state and Web detail show the failed tool result or failed run status with diagnostic text. |
| History consistency | After all scenarios | `/runs`, `/runs/{run_id}/report`, Web history, and Web detail agree on run ids, statuses, timestamps, and tool/input/approval records. |

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

Start the Web workbench in another terminal:

```powershell
cd web-ui
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm dev
```

Open `http://localhost:3000` and run the same scenarios through the UI. The Web pass criteria are:

- creating a job from the UI works against the real API, not a mocked API;
- live status updates appear from the SSE stream;
- pending approvals and inputs are visible and actionable;
- history shows completed, interrupted, rejected, and failed runs as applicable;
- detail/report views show tool names, arguments or summaries, and final status;
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
7. Starts `pnpm exec next dev --port <port>` in `web-ui` with `ROVE_API_BASE` pointing to the API.
8. Runs the gated real-API Playwright suite with `ROVE_REAL_API_E2E=1`,
   `ROVE_WEB_PORT=<port>`, and `PLAYWRIGHT_BASE_URL=http://127.0.0.1:<port>`.
9. Stops API and Web processes even when a check fails.
10. Prints run ids and artifact paths.

The runner does not run `provider-smoke`, `external-tools`, or `stress`; those remain explicit follow-up gates.

## Real-API Playwright Design

The existing browser E2E tests mock the API at the browser boundary. The real-API suite is `web-ui/tests/e2e/real-api.spec.ts` and is gated by `ROVE_REAL_API_E2E=1`.

The real-API suite does not start the Rust API itself. The runner owns API/Web process lifecycle so failures can preserve both logs. The test:

- open the workbench against the runner's Web URL;
- create a plain fake-provider run;
- create an approval run and approve it from the UI;
- create a request-input run and answer it from the UI;
- verify history and detail records after each run;
- attach screenshots or traces when assertions fail.

## Optional Gates

### Provider Smoke

Use `docs/runtime/provider-smoke.md` as the source of truth. With no gate enabled, `cargo test --test provider_smoke` should pass by skipping real calls. Enable one provider at a time:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
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

### RAG

RAG remains feature-gated and deterministic by default:

```powershell
cargo check --features rag --bin rove-index
cargo test --features rag --test cli_index
cargo test --features rag --test rag
```

Provider embeddings or reranking should be enabled only through explicit environment gates and should write artifacts under the integration state directory.

## Pass Criteria

`local-full` passes only when all required scenarios have evidence from both API/state and Web:

- every expected run has a run id and terminal status;
- approval and input pending records are created, resolved, and no longer pending afterward;
- filesystem writes stay inside `<integration-root>/workspace`;
- `/runs` and `/runs/{run_id}/report` agree with live job state;
- Web history and detail views show the same statuses and important steps as the API;
- logs and artifacts are saved for the run.

If `local-full` passes but a gated provider or external tool profile fails, the first-round baseline still passes. Track the gated failure separately with its profile name, env configuration, and artifact path.

## Pre-Integration Unit Gates

Run the deterministic code gates before a full integration pass:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Feature and optional gates can be layered afterward:

```powershell
cargo test --features rag
cd web-ui
pnpm test:e2e
```
