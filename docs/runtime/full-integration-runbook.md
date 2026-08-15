# Full Integration Test Runbook

This runbook is the handoff document for a new session that needs to verify the complete rove call chain, Web records, real-provider access, external MCP tools, and stress/long-running behavior.

Last reviewed: 2026-07-27.

## Scope

Run the gates in this order:

| Gate | Required | External dependency | Purpose |
|---|---:|---|---|
| `local-full` | Yes | No | Proves the deterministic API/runtime/tool/state chain, the default product shell against the live Rust API, and one bounded `/dev/workbench` smoke. |
| `provider-model-inventory` | Yes before provider gates | Provider key/network, except local providers | Lists the account-visible models for an official API, relay/gateway API, or local provider. |
| `provider-smoke` | Yes for real-provider readiness | Provider key/network, except local providers | Proves one selected model can answer and perform native tool use through the selected provider path. |
| `provider-full` | Yes after smoke passes | Provider key/network, except local providers | Proves real provider + real API records and the exact product-shell job/report/transcript binding. |
| `external-tools` | Yes after provider-full | Local mock MCP, then real filesystem MCP | Proves MCP discovery, execution, approval, failure records, and Web visibility. |
| `stress` | Yes after all functional gates | Depends on selected provider profile | Proves repeated jobs, concurrency, restart recovery, and long-run state consistency. |

The local deterministic runner implements `local-full`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

The provider runner implements real provider gates for OpenAI,
Anthropic, and Ollama profiles:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "<provider-or-gateway-v1-base>" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>"
```

The runner's Web step now creates API-backed product state, opens an exact
product session in `/`, captures the browser's `POST /api/jobs` IDs, and verifies
that exact report/transcript binding. Use `-SkipWebSmoke` only for an intentional
provider/API-only diagnostic. No external-provider C3 browser gate has been run;
the completed deterministic `local-full` gate is not a substitute for it.

The external MCP and stress gates are still documented here as explicit
execution plans. Do not treat a provider/MCP/stress failure as a regression in
`local-full`; record it under the failed gate.

## Continuous Gates (CI)

Two GitHub Actions workflows automate the gates above:

- `.github/workflows/ci.yml` runs on every push and pull request: Rust
  `fmt` / `clippy` / `test` and Web `test` / `typecheck` / `build`. This is the
  fast per-change gate and stays free of external dependencies.
- `.github/workflows/release-gate.yml` runs nightly (and on manual
  `workflow_dispatch`) on `windows-latest`, because the integration scripts are
  Windows PowerShell. It has two jobs:
  - `local-full` runs `scripts/integration-smoke.ps1` and uploads
    `local-full-artifacts`. It needs no secrets.
  - `provider-gate` runs `scripts/provider-integration.ps1` with
    the functional provider gates by default. Its configured-secret Web path
    now uses the exact product-session flow described above. No C3 execution of
    that external-provider job is currently claimed. When the `run_stress` input
    is set, it also passes `-RunStress -RunRestartRecovery -RunLongSoak` and
    uploads `provider-gate-artifacts`. It is skipped
    automatically when the provider key secret is absent, so it never fails a
    repo that has not configured a provider.

Configure the provider gate once at the repository level:

| Kind | Name | Purpose |
|---|---|---|
| Secret | `ROVE_PROVIDER_API_KEY` | Provider key, injected as `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` for the run. |
| Variable | `ROVE_PROVIDER_NAME` | Runner provider type (defaults to `openai`); this environment-variable name predates the request field `provider_type`. |
| Variable | `ROVE_PROVIDER_API_BASE` | Provider `/v1` (or native) base URL. |
| Variable | `ROVE_PROVIDER_MODEL` | Model id to smoke test. |
| Variable | `ROVE_PROVIDER_MODELS_ENDPOINT` | Optional explicit models inventory endpoint. |

A manual run can override the provider, API base, model, and models endpoint via
the `workflow_dispatch` inputs without touching repository variables. The key
still comes only from the secret, never from an input, matching the secret
handling rules below.

## Secret Handling

Never commit real keys or generated runtime state. Do not paste keys into `.rove/config.toml`, `.env.integration`, docs, screenshots, or issue descriptions.

For a local session, set provider keys only in the current PowerShell process:

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI key"
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic key"
```

If a key was pasted into a chat, terminal transcript, or committed file, rotate it in the provider console before running long tests.

## Provider Facts

Provider profiles are not vendor-specific. The runtime can use:

- official OpenAI APIs and compatible relay/gateway `/v1` APIs;
- native Anthropic Messages API;
- local Ollama `/api/chat`;
- deterministic fake providers for local verification.

Both Web surfaces and `POST /jobs` use the same per-run request shape:
`provider_type`, API base, key environment-variable name when needed, and model
id. `name` is only an optional display label. Clients cannot write
`wire_protocol`; the system maps it from `provider_type` and may echo it in
responses. Browser code sends only the environment-variable name, never the raw
key.

SiliconFlow is one OpenAI example. Useful public surfaces:

- Platform introduction: `https://api-docs.siliconflow.cn/docs/userguide/get_started/introduction`
- OpenAI chat endpoint: `https://api-docs.siliconflow.cn/docs/api/chat-completions-post`
- Model list endpoint: `https://api-docs.siliconflow.cn/docs/api/models-get`
- Public model center: `https://www.siliconflow.cn/models`

SiliconFlow exposes OpenAI-style chat completions and a model-list endpoint. The docs show model list as `GET /v1/models`; unauthenticated calls return `Invalid token`, so the account-visible list must be queried with the user's key.

Only use model ids that do not start with `Pro/`.

Public non-Pro candidates visible on the model center on 2026-05-31 included:

| Purpose | Candidate model ids |
|---|---|
| Text/tool-call smoke candidates | `Qwen/Qwen3-Coder-30B-A3B-Instruct`, `Qwen/Qwen3-32B`, `Qwen/Qwen3-14B`, `Qwen/Qwen3-8B`, `Qwen/Qwen2.5-72B-Instruct-128K`, `deepseek-ai/DeepSeek-V3.2`, `deepseek-ai/DeepSeek-V3.1-Terminus`, `deepseek-ai/DeepSeek-V3` |
| Smaller fallback candidates | `Qwen/Qwen2.5-7B-Instruct`, `Qwen/Qwen2.5-14B-Instruct`, `Qwen/Qwen2.5-32B-Instruct`, `deepseek-ai/DeepSeek-R1-0528-Qwen3-8B` |

This table is a starting point, not the source of truth. The source of truth is
the authenticated `/v1/models` response for the account that runs the test.
Built-in embedding/rerank RAG was removed and is not part of this gate.

## Gate 0: Preflight

Run deterministic gates before any network-backed test:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/web
pnpm test
pnpm typecheck
pnpm build
pnpm test:e2e
cd ..
```

Acceptance:

- all commands exit with code 0;
- `pnpm test:e2e` skips the gated real-API suite unless `ROVE_REAL_API_E2E=1`;
- no generated secret/state/log files appear as tracked Git changes.

## Developer Launch

For interactive local verification, use the launcher instead of starting API and
Web by hand:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

For provider-backed local use, set provider environment first, then pass
`-Provider` so the launcher does not force fake mode:

```powershell
$env:ROVE_PROVIDER = "openai"
$env:ROVE_MODEL = "<chat/tool model>"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1 -Provider
```

The launcher is not the full integration evidence package. It is a convenience
for manual inspection; the gates below still provide the accepted evidence.

## Gate 1: `local-full`

Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

Expected flow:

```text
Web/API request
  -> rove-api job
  -> engine run
  -> fake provider event stream
  -> built-in tool call or pending interaction
  -> approval/input resolution
  -> terminal run
  -> trace/report/task state persisted
  -> exact product-session transcript/report binding is verified
  -> bounded /dev/workbench direct-run smoke remains available
```

Scenarios covered by the runner:

| Scenario | Evidence |
|---|---|
| Plain run | API job completes and Web shows final fake response. |
| `echo` tool | API state records tool lifecycle and output text. |
| Approved `write_file` | Pending approval appears, approval resumes run, file is created inside isolated workspace. |
| Rejected `write_file` | Rejection is recorded and target file is not created. |
| `request_input` | Pending input appears, submitted answer resumes run, answer is recorded. |
| Tool failure | Failed tool event is visible in API artifacts. |
| Product migration | Safe M1 browser state migrates before catalog boot, the legacy route is remapped, and refresh does not replay the import. |
| Product continuity | Interleaved A/B sessions resume their own exact chains across refresh; approval, input, cancellation, Settings, and deep routes work through `/`. |
| Unified message delivery | Active-run messages preserve one durable identity through promotion or revoke and never duplicate transcript content. |
| Product Fork | A completed session forks at the verified terminal boundary and the child continues without changing the parent history. |
| Advanced escape hatch | One bounded `/dev/workbench` direct-run smoke completes against the same live API. |

Acceptance:

- command exits with code 0 and prints `local-full integration smoke completed`;
- Playwright reports all five real-API cases passed: migration, default A/B
  product lifecycle, unified-message promotion/revocation, completed-session
  Fork/independent-child continuation, and bounded advanced smoke;
- artifacts are written under `%TEMP%\rove-integration\artifacts` unless `ROVE_INTEGRATION_ROOT` overrides it;
- printed run ids are present in `GET /runs?limit=25`;
- `api/*.state.json`, product transcripts, reports, and Web assertions agree on
  exact run identities, terminal statuses, approval/input resolution, and tool
  names.

The integrated C3 implementation on `main` originally passed this `local-full`
gate 3/3 after the ordered coordinator merge. Productization integration extends
the same live local gate to 5/5 with unified-message and Fork/child scenarios.

## Gate 2: `provider-model-inventory`

For OpenAI official APIs or relay/gateway APIs, set the key in the
current shell and query the configured model list endpoint:

```powershell
$env:OPENAI_API_KEY = Read-Host "Provider API key"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
```

Query account-visible models:

```powershell
$headers = @{ Authorization = "Bearer $env:OPENAI_API_KEY" }
$models = Invoke-RestMethod -Uri "$env:OPENAI_API_BASE/models" -Headers $headers
$visible = $models.data |
  Where-Object { $_.id } |
  Sort-Object id
$visible | Select-Object id, owned_by
```

Save evidence without secrets:

```powershell
$root = Join-Path $env:TEMP "rove-integration-provider"
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$visible | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $artifacts "provider-models.json")
```

Also save the selected chat/tool model id in a plain text artifact such
as `selected-provider-model.txt`. Do not save API keys or bearer tokens.

Choose the provider smoke model in this order:

1. A model from the authenticated list that is documented or observed to support tool calls.
2. Prefer the model configured for the target account or relay route.
3. If the first model answers text but fails tool use, keep the text result as evidence and try the next compatible model.

For Anthropic, use the provider console or `GET /v1/models` with
`x-api-key` and `anthropic-version` headers. For Ollama, use
`GET <ollama-base>/api/tags`.

Acceptance:

- the provider's model inventory endpoint returns HTTP 200;
- selected chat model is present in the authenticated inventory;
- selected model id is recorded in the test notes and artifact directory.

## Gate 3: `provider-smoke`

Use the generic provider runner for OpenAI endpoints, relay/gateway endpoints,
Anthropic, and Ollama. The following commands intentionally collect focused
provider/API evidence with `-SkipWebSmoke`; omit that switch for the separate
external-provider product-shell gate:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini" `
  -SkipWebSmoke
```

For SiliconFlow, `deepseek-ai/DeepSeek-V3.2` is one verified example:

```powershell
$env:SILICONFLOW_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.siliconflow.cn/v1" `
  -ApiKeyEnv SILICONFLOW_API_KEY `
  -Model "deepseek-ai/DeepSeek-V3.2" `
  -SkipWebSmoke
```

Anthropic uses native model inventory and smoke dispatch:

```powershell
$env:ANTHROPIC_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider anthropic `
  -ApiBase "https://api.anthropic.com" `
  -ApiKeyEnv ANTHROPIC_API_KEY `
  -Model "claude-3-5-haiku-latest" `
  -SkipWebSmoke
```

Ollama requires a local server and pulled model, but no API key:

```powershell
ollama list
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2" `
  -SkipWebSmoke
```

The API and Web surfaces accept `provider_type` values `openai`,
`openai-responses`, `anthropic`, `ollama`, and `fake`. Browser code sends key
environment-variable names only; it never sends raw key values.

The browser path is implemented but was not run against an external provider in
the C3 verification pass. Record it as `not_run`, not `pass`, until a credentialed
run produces the Web result, exact run report, and exact product transcript.

For a smoke-only manual check, use rove's OpenAI provider path
directly:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "<chat/tool model>"

cargo test -p rove-integration-tests --test provider_smoke openai_real_provider_smoke_when_enabled -- --exact --nocapture
```

The test performs:

- one final-answer request;
- one native `echo` tool-use request through the engine.

Acceptance:

- command exits with code 0;
- output contains the exact smoke phrase `rove provider smoke ok`;
- test observes an `echo` tool call and a completed tool output containing `rove provider tool smoke ok`;
- when using `scripts/provider-integration.ps1`, `evidence-summary.json`
  records `model_inventory`, `provider_smoke`, `provider_full_api`, and
  `web_provider` gate statuses;
- provider errors are classified clearly:
  - `401` or `403`: key/base/model access problem;
  - `429`: quota or rate limit, rerun with lower concurrency or later;
  - model/tool-call failure: try another authenticated chat model.

## Gate 4: `provider-full`

This gate proves real provider + real API records and, when `-SkipWebSmoke` is
omitted, the exact product-shell run/report/transcript binding. Prefer the
generic runner for OpenAI official APIs, relay/gateway APIs, Anthropic, and
Ollama. Use the manual profile below only for diagnostics when the runner needs
to be decomposed. The external-provider Web portion remains unrun for C3.

Prepare isolated state:

```powershell
$root = Join-Path $env:TEMP "rove-integration-provider"
$workspace = Join-Path $root "workspace"
$state = Join-Path $workspace ".rove-integration-state"
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force -Path $workspace, $state, $artifacts | Out-Null

$env:ROVE_PROVIDER = "openai"
$env:ROVE_MODEL = "<chat/tool model>"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_STATE_DIR = $state
$env:ROVE_STATE_SQLITE = Join-Path $state "state.sqlite"
$env:ROVE_MEMORY_SESSION_DIR = Join-Path $state "memory/sessions"
$env:ROVE_MEMORY_DURABLE_DIR = Join-Path $state "memory"
$env:ROVE_API_BIND_ADDR = "127.0.0.1:8787"
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
$env:ROVE_WEB_PORT = "3000"
```

Start the API in terminal A:

```powershell
cargo run -p rove-api -- --addr 127.0.0.1:8787 -C $workspace
```

Start Web in terminal B:

```powershell
cd apps/web
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm exec next dev --port 3000
```

Run API scenarios in terminal C.

Plain answer:

```powershell
$plain = Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs `
  -ContentType application/json `
  -Body (@{
    message = "Reply with exactly: rove provider full plain ok"
    model = $env:ROVE_MODEL
    approval = "auto"
    max_steps = 4
  } | ConvertTo-Json -Compress)
```

Tool call:

```powershell
$tool = Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs `
  -ContentType application/json `
  -Body (@{
    message = "Use the echo tool exactly once with message `"rove provider full tool ok`", then reply with exactly: rove provider full done"
    model = $env:ROVE_MODEL
    approval = "auto"
    max_steps = 4
  } | ConvertTo-Json -Compress)
```

Approval through Web:

1. Create or select an API-backed product workspace/session, then open the
   corresponding `/w/<workspace_id>/s/<product_session_id>` route.
2. In Settings -> Providers, create/select the provider/model profile without
   entering a raw key value in the browser.
3. Submit a request asking the model to write a short file through the available file-write tool.
4. Approve the pending `write_file` card in the Web panel.

Input through Web:

1. Submit a request asking the model to call the input-request tool with the prompt `Which branch should I use?`.
2. Answer `main` in the Web input card.
3. Wait for completion.

Collect evidence:

```powershell
Invoke-RestMethod http://127.0.0.1:8787/runs?limit=25 |
  ConvertTo-Json -Depth 20 |
  Set-Content -LiteralPath (Join-Path $artifacts "provider-full-runs.json")

# Replace <run_id> with each run id produced above.
Invoke-RestMethod http://127.0.0.1:8787/runs/<run_id>/report |
  ConvertTo-Json -Depth 30 |
  Set-Content -LiteralPath (Join-Path $artifacts "provider-full-<run_id>-report.json")
```

Acceptance:

- at least one plain real-provider run reaches terminal `done`;
- at least one native tool-use run records `ToolCallStarted` and `ToolCallCompleted` for `echo`;
- the browser result records the exact `job_id` and `run_id` returned by
  `POST /jobs`, without selecting a guessed latest run;
- `/runs/{run_id}/report` and the selected product session transcript contain
  that exact job/run binding and show terminal status, model id, tool name,
  result, and final output;
- approval and input cards can be resolved from Web and the API state no longer reports them as pending afterward;
- all filesystem writes remain under the isolated `$workspace`;
- no secret appears in saved artifacts.

## Gate 5: `external-tools`

Run deterministic MCP tests first:

```powershell
cargo test -p rove-integration-tests --test mcp
```

Run official filesystem MCP smoke:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test -p rove-integration-tests --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

Then verify MCP through API/Web using the local mock server:

```powershell
Copy-Item .rove/mcp_servers.example.json .rove/mcp_servers.json
$env:ROVE_MCP_CONFIG = ".rove/mcp_servers.json"
```

Start API/Web as in `provider-full`, but use either:

- `fake-raw` for deterministic MCP tool calls; or
- the selected provider model if provider tool-use smoke already passed.

Deterministic API request:

```powershell
$mcp = Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:8787/jobs `
  -ContentType application/json `
  -Body '{
    "message":"{\"tool\":\"mcp__mock_server__echo_remote\",\"args\":{\"message\":\"hello api mcp\"}}",
    "model":"fake-raw",
    "approval":"auto",
    "max_steps":1
  }'
```

Expected local mock tool names:

- `mcp__mock_server__echo_remote`
- `mcp__mock_server__delete_remote`

Acceptance:

- `cargo test -p rove-integration-tests --test mcp` exits with code 0;
- official filesystem smoke exits with code 0 when the opt-in gate is enabled;
- API job using `mcp__mock_server__echo_remote` reaches terminal `done`;
- report contains `remote: hello api mcp`;
- advanced `/dev/workbench` history/detail shows the MCP tool name and result;
- destructive MCP tools are marked destructive and require approval when policy is `ask`;
- rejected destructive MCP calls do not perform the destructive action and produce an explainable failed/rejected record.

## Gate 6: `stress`

Run stress only after `local-full`, `provider-smoke`, `provider-full`, and `external-tools` pass.

The provider runner owns the preferred stress evidence path. These commands use
`-SkipWebSmoke` only because Gate 4 should already have collected the browser
evidence once for the same provider configuration:

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

For a long soak:

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

The runner writes `stress-summary.json`, per-job state artifacts, per-job report
artifacts, `stress-runs-before-restart.json`,
`stress-runs-after-restart.json`, `stress-api.out.log`,
`stress-api.err.log`, and restart API logs when restart recovery is enabled.
Failed jobs include a provider/runtime classification in the stress summary.

### Local deterministic stress

Start API with fake provider and isolated state, then run 100 sequential jobs:

```powershell
$ApiBase = "http://127.0.0.1:8787"
$results = @()
for ($i = 1; $i -le 100; $i++) {
  $created = Invoke-RestMethod `
    -Method Post `
    -Uri "$ApiBase/jobs" `
    -ContentType application/json `
    -Body (@{
      message = "local stress plain $i"
      model = "fake"
      approval = "auto"
      max_steps = 4
    } | ConvertTo-Json -Compress)
  $results += $created
}
$results | ConvertTo-Json -Depth 20 | Set-Content "$env:TEMP\rove-integration\artifacts\stress-local-created.json"
```

Poll each job until terminal and save `/runs`.

### Provider sequential stress

Use a low request count first to avoid rate-limit noise:

```powershell
$ApiBase = "http://127.0.0.1:8787"
$ProviderStressCount = 20
for ($i = 1; $i -le $ProviderStressCount; $i++) {
  Invoke-RestMethod `
    -Method Post `
    -Uri "$ApiBase/jobs" `
    -ContentType application/json `
    -Body (@{
      message = "Reply with exactly: rove provider stress ok $i"
      model = $env:ROVE_MODEL
      approval = "auto"
      max_steps = 4
    } | ConvertTo-Json -Compress) | Out-Null
}
```

### Concurrent stress

Use small concurrency for real providers:

```powershell
$ApiBase = "http://127.0.0.1:8787"
$jobs = 1..5 | ForEach-Object {
  Start-Job -ScriptBlock {
    param($ApiBase, $Model, $Index)
    Invoke-RestMethod `
      -Method Post `
      -Uri "$ApiBase/jobs" `
      -ContentType application/json `
      -Body (@{
        message = "Reply with exactly: rove concurrent stress ok $Index"
        model = $Model
        approval = "auto"
        max_steps = 4
      } | ConvertTo-Json -Compress)
  } -ArgumentList $ApiBase, $env:ROVE_MODEL, $_
}
$jobs | Receive-Job -Wait -AutoRemoveJob
```

### Restart recovery

1. Start 10 jobs.
2. Stop `rove-api`.
3. Restart it with the same `$workspace` and `$state`.
4. Query `GET /runs?limit=25`.
5. Open `/dev/workbench` and confirm history reloads.

Acceptance:

- deterministic local stress has 100 completed runs and 0 internal failures;
- provider sequential stress has 20 completed runs with no unclassified rove errors;
- if the provider returns `429`, lower concurrency and rerun; the accepted final run must document the chosen concurrency and have no unhandled provider throttling;
- concurrent stress with 5 jobs does not panic, deadlock, corrupt SQLite, or lose run records;
- after restart, `/runs` still lists previous completed runs and advanced
  `/dev/workbench` history loads them;
- API logs contain no Rust panic, task join panic, SQLite lock timeout loop, or process cleanup leak;
- memory/state/artifacts remain inside the isolated integration root.

## Final Evidence Package

For a full pass, preserve these under the integration artifact directory:

- `provider-models.json`
- selected provider model id and endpoint base in a redacted `environment.txt`
- `environment.redacted.json`
- `evidence-summary.json`
- API stdout/stderr logs
- Web stdout/stderr logs
- Playwright screenshots/traces if Web automation is used
- `api/*.created.json`
- `api/*.state.json`
- `/runs` snapshots
- `/runs/{run_id}/report` snapshots
- `stress-summary.json`, stress state snapshots, stress report snapshots, and
  restart snapshots when stress was run
- MCP config copy with secrets removed

The final report should list:

| Field | Required content |
|---|---|
| Date/time | Local date/time and timezone. |
| Git revision | `git rev-parse HEAD` plus dirty/clean status. |
| Gates run | `local-full`, `provider-model-inventory`, `provider-smoke`, `provider-full`, `external-tools`, `stress`. |
| Models | Authenticated model selected for chat/tool use. |
| Run ids | All run ids from API/Web/provider/stress gates. |
| Artifacts | Absolute artifact directory. |
| Failures | Gate name, command, error, classification, and whether rerun passed. |

The complete integration pass is accepted only when every required gate above passes and the evidence package is sufficient to reproduce or debug each run without exposing secrets.

## New Session Prompt

When starting a fresh session, paste this request:

```text
我们继续 rove 的完整集成测试。请先阅读 docs/runtime/full-integration-runbook.md、docs/runtime/integration-testing.md、docs/runtime/provider-smoke.md、.env.integration.example，以及本地 .env.integration。当前工作目录是 D:\Study\project\agent\rove。

目标：按 full-integration-runbook 的顺序执行全面测试：先确认 local-full 基线，再用 .env.integration 里的 provider 配置查询模型 inventory，选择一个可用的 chat/tool 模型做 provider-smoke，然后继续 provider-full、external-tools、stress/长跑测试。local-full 必须核对默认 `/` 产品壳的迁移、A/B 精确续跑、刷新、交互和深路由，以及单独的 bounded `/dev/workbench` smoke。provider runner 的 Web 步骤必须捕获浏览器 `POST /api/jobs` 返回的精确 job/run IDs，并核对对应 report 和 product transcript，不能按 ID 排序猜 latest。C3 尚无外部 provider Web 实跑证据，不要把未运行写成通过。每一步都要保存 artifacts；遇到失败先定位原因并修复，再继续。

注意：真实 provider key 已经写在本地忽略文件 .env.integration 里；不要把 key 写入 tracked 文件或最终报告。先运行 git status，保护已有未提交改动，不要 reset 或 checkout 用户改动。
```

If the new session should only prepare and not execute, replace `执行全面测试` with `先检查配置并给出执行计划，不要启动服务或发起真实 provider 请求`.
