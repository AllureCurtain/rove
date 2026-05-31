# Full Integration Test Runbook

This runbook is the handoff document for a new session that needs to verify the complete rove call chain, Web records, SiliconFlow real-provider access, external MCP tools, and stress/long-running behavior.

Last reviewed: 2026-05-31.

## Scope

Run the gates in this order:

| Gate | Required | External dependency | Purpose |
|---|---:|---|---|
| `local-full` | Yes | No | Proves the full deterministic API/Web/runtime/tool/state chain. |
| `siliconflow-model-inventory` | Yes before provider gates | SiliconFlow key | Lists the account-visible models and filters out `Pro/` models. |
| `provider-smoke` | Yes for real-provider readiness | SiliconFlow key and network | Proves one non-Pro SiliconFlow model can answer and perform native tool use through the OpenAI-compatible provider path. |
| `provider-full` | Yes after smoke passes | SiliconFlow key and network | Proves real provider + real API + real Web history/detail records. |
| `external-tools` | Yes after provider-full | Local mock MCP, then real filesystem MCP | Proves MCP discovery, execution, approval, failure records, and Web visibility. |
| `stress` | Yes after all functional gates | Depends on selected provider profile | Proves repeated jobs, concurrency, restart recovery, and long-run state consistency. |

The current automated runner implements `local-full` only:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

The provider, external MCP, and stress gates are documented here as the next explicit execution plan. Do not treat a provider/MCP/stress failure as a regression in `local-full`; record it under the failed gate.

## Secret Handling

Never commit real keys or generated runtime state. Do not paste keys into `.rove/config.toml`, `.env.integration`, docs, screenshots, or issue descriptions.

For a local session, set the SiliconFlow key only in the current PowerShell process:

```powershell
$env:SILICONFLOW_API_KEY = Read-Host "SiliconFlow API key"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
```

If a key was pasted into a chat, terminal transcript, or committed file, rotate it in the provider console before running long tests.

## SiliconFlow Facts

Use these official surfaces:

- Platform introduction: `https://api-docs.siliconflow.cn/docs/userguide/get_started/introduction`
- OpenAI-compatible chat endpoint: `https://api-docs.siliconflow.cn/docs/api/chat-completions-post`
- Model list endpoint: `https://api-docs.siliconflow.cn/docs/api/models-get`
- Public model center: `https://www.siliconflow.cn/models`

SiliconFlow exposes OpenAI-style chat completions and a model-list endpoint. The docs show model list as `GET /v1/models`; unauthenticated calls return `Invalid token`, so the account-visible list must be queried with the user's key.

Only use model ids that do not start with `Pro/`.

Public non-Pro candidates visible on the model center on 2026-05-31 included:

| Purpose | Candidate model ids |
|---|---|
| Text/tool-call smoke candidates | `Qwen/Qwen3-Coder-30B-A3B-Instruct`, `Qwen/Qwen3-32B`, `Qwen/Qwen3-14B`, `Qwen/Qwen3-8B`, `Qwen/Qwen2.5-72B-Instruct-128K`, `deepseek-ai/DeepSeek-V3.2`, `deepseek-ai/DeepSeek-V3.1-Terminus`, `deepseek-ai/DeepSeek-V3` |
| Smaller fallback candidates | `Qwen/Qwen2.5-7B-Instruct`, `Qwen/Qwen2.5-14B-Instruct`, `Qwen/Qwen2.5-32B-Instruct`, `deepseek-ai/DeepSeek-R1-0528-Qwen3-8B` |
| Embedding candidates | `BAAI/bge-m3`, `BAAI/bge-large-zh-v1.5`, `BAAI/bge-large-en-v1.5`, `Qwen/Qwen3-Embedding-0.6B`, `Qwen/Qwen3-Embedding-4B`, `Qwen/Qwen3-Embedding-8B` |
| Rerank candidates | `BAAI/bge-reranker-v2-m3`, `Qwen/Qwen3-Reranker-0.6B`, `Qwen/Qwen3-Reranker-4B`, `Qwen/Qwen3-Reranker-8B` |

This table is a starting point, not the source of truth. The source of truth is the authenticated `/v1/models` response for the account that runs the test.

## Gate 0: Preflight

Run deterministic gates before any network-backed test:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd web-ui
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
$env:ROVE_PROVIDER = "openai-compatible"
$env:ROVE_MODEL = "<non-Pro chat/tool model>"
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
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
  -> /runs and Web history/detail show matching records
```

Scenarios covered by the runner:

| Scenario | Evidence |
|---|---|
| Plain run | API job completes and Web shows final fake response. |
| `echo` tool | API state records tool lifecycle and output text. |
| Approved `fs_write` | Pending approval appears, approval resumes run, file is created inside isolated workspace. |
| Rejected `fs_write` | Rejection is recorded and target file is not created. |
| `request_input` | Pending input appears, submitted answer resumes run, answer is recorded. |
| Tool failure | Failed tool event is visible in API artifacts. |
| Web records | Real Web workbench creates/approves/answers jobs against the real API and shows run detail/history. |

Acceptance:

- command exits with code 0 and prints `local-full integration smoke completed`;
- Playwright reports 3 real API tests passed;
- artifacts are written under `%TEMP%\rove-integration\artifacts` unless `ROVE_INTEGRATION_ROOT` overrides it;
- printed run ids are present in `GET /runs?limit=25`;
- `api/*.state.json` and Web records agree on run ids, terminal statuses, approval/input resolution, and tool names.

## Gate 2: `siliconflow-model-inventory`

Set the key in the current shell:

```powershell
$env:SILICONFLOW_API_KEY = Read-Host "SiliconFlow API key"
```

Query account-visible models:

```powershell
$headers = @{ Authorization = "Bearer $env:SILICONFLOW_API_KEY" }
$models = Invoke-RestMethod -Uri "https://api.siliconflow.cn/v1/models" -Headers $headers
$nonPro = $models.data |
  Where-Object { $_.id -and -not $_.id.StartsWith("Pro/") } |
  Sort-Object id
$nonPro | Select-Object id, owned_by
```

Save evidence without secrets:

```powershell
$root = Join-Path $env:TEMP "rove-integration-siliconflow"
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$nonPro | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $artifacts "siliconflow-non-pro-models.json")
```

Also save the selected non-Pro chat/tool model id in a plain text artifact such
as `selected-provider-model.txt`. Do not save API keys or bearer tokens.

Choose the provider smoke model in this order:

1. A non-Pro model from the authenticated list that is documented or observed to support tool calls.
2. Prefer `Qwen/Qwen3-Coder-30B-A3B-Instruct`, `Qwen/Qwen3-32B`, or `deepseek-ai/DeepSeek-V3.2` if available.
3. If the first model answers text but fails tool use, keep the text result as evidence and try the next non-Pro model.

Acceptance:

- `/v1/models` returns HTTP 200 with a `data` array;
- saved model inventory contains no id starting with `Pro/`;
- selected chat model is present in the authenticated inventory;
- selected model id is recorded in the test notes and artifact directory.

## Gate 3: `provider-smoke`

Use rove's OpenAI-compatible provider path against SiliconFlow:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "<non-Pro chat/tool model>"

cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
```

The test performs:

- one final-answer request;
- one native `echo` tool-use request through the engine.

Acceptance:

- command exits with code 0;
- output contains the exact smoke phrase `rove provider smoke ok`;
- test observes an `echo` tool call and a completed tool output containing `rove provider tool smoke ok`;
- provider errors are classified clearly:
  - `401` or `403`: key/base/model access problem;
  - `429`: quota or rate limit, rerun with lower concurrency or later;
  - model/tool-call failure: try another authenticated non-Pro chat model.

## Gate 4: `provider-full`

This gate proves real provider + real API + real Web records. It is currently a manual profile; a later implementation can automate it by extending `scripts/integration-smoke.ps1` or adding a dedicated provider runner.

Prepare isolated state:

```powershell
$root = Join-Path $env:TEMP "rove-integration-siliconflow"
$workspace = Join-Path $root "workspace"
$state = Join-Path $workspace ".rove-integration-state"
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force -Path $workspace, $state, $artifacts | Out-Null

$env:ROVE_PROVIDER = "openai-compatible"
$env:ROVE_MODEL = "<non-Pro chat/tool model>"
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
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
cargo run --bin rove-api -- --addr 127.0.0.1:8787 -C $workspace
```

Start Web in terminal B:

```powershell
cd web-ui
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

1. Open `http://localhost:3000`.
2. Use the selected non-Pro model.
3. Submit a request asking the model to write a short file through the available file-write tool.
4. Approve the pending `fs_write` card in the Web panel.

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
- Web history shows the real-provider run ids after page reload;
- Web detail/report shows terminal status, model id, tool name, tool result, and final output;
- approval and input cards can be resolved from Web and the API state no longer reports them as pending afterward;
- all filesystem writes remain under the isolated `$workspace`;
- no secret appears in saved artifacts.

## Gate 5: `external-tools`

Run deterministic MCP tests first:

```powershell
cargo test --test mcp
```

Run official filesystem MCP smoke:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

Then verify MCP through API/Web using the local mock server:

```powershell
Copy-Item .rove/mcp_servers.example.json .rove/mcp_servers.json
$env:ROVE_MCP_CONFIG = ".rove/mcp_servers.json"
```

Start API/Web as in `provider-full`, but use either:

- `fake-raw` for deterministic MCP tool calls; or
- the selected SiliconFlow non-Pro model if provider tool-use smoke already passed.

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

- `cargo test --test mcp` exits with code 0;
- official filesystem smoke exits with code 0 when the opt-in gate is enabled;
- API job using `mcp__mock_server__echo_remote` reaches terminal `done`;
- report contains `remote: hello api mcp`;
- Web history/detail shows the MCP tool name and result;
- destructive MCP tools are marked destructive and require approval when policy is `ask`;
- rejected destructive MCP calls do not perform the destructive action and produce an explainable failed/rejected record.

## Gate 6: RAG Optional External Gate

Run deterministic RAG checks:

```powershell
cargo check --features rag --bin rove-index
cargo test --features rag --test cli_index
cargo test --features rag --test rag
```

If using SiliconFlow embeddings, first confirm the embedding model is in the authenticated non-Pro inventory. Then configure:

```powershell
$env:ROVE_RAG_DETERMINISTIC = "false"
$env:ROVE_RAG_EMBEDDING_PROVIDER = "openai-compatible"
$env:ROVE_RAG_EMBEDDING_MODEL = "<non-Pro embedding model>"
$env:ROVE_RAG_EMBEDDING_API_BASE = "https://api.siliconflow.cn/v1"
$env:ROVE_RAG_EMBEDDING_API_KEY = $env:SILICONFLOW_API_KEY
```

Acceptance:

- deterministic RAG tests pass;
- provider-backed embedding runs write all index, manifest, and eval artifacts under the isolated integration state directory;
- no RAG artifact is written into the normal `.rove` runtime state unless the test explicitly chooses that workspace.

## Gate 7: `stress`

Run stress only after `local-full`, `provider-smoke`, `provider-full`, and `external-tools` pass.

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
5. Open Web and confirm history reloads.

Acceptance:

- deterministic local stress has 100 completed runs and 0 internal failures;
- provider sequential stress has 20 completed runs with no unclassified rove errors;
- if the provider returns `429`, lower concurrency and rerun; the accepted final run must document the chosen concurrency and have no unhandled provider throttling;
- concurrent stress with 5 jobs does not panic, deadlock, corrupt SQLite, or lose run records;
- after restart, `/runs` still lists previous completed runs and Web history loads them;
- API logs contain no Rust panic, task join panic, SQLite lock timeout loop, or process cleanup leak;
- memory/state/artifacts remain inside the isolated integration root.

## Final Evidence Package

For a full pass, preserve these under the integration artifact directory:

- `siliconflow-non-pro-models.json`
- selected provider model id and endpoint base in a redacted `environment.txt`
- API stdout/stderr logs
- Web stdout/stderr logs
- Playwright screenshots/traces if Web automation is used
- `api/*.created.json`
- `api/*.state.json`
- `/runs` snapshots
- `/runs/{run_id}/report` snapshots
- stress created-job and terminal-state snapshots
- MCP config copy with secrets removed

The final report should list:

| Field | Required content |
|---|---|
| Date/time | Local date/time and timezone. |
| Git revision | `git rev-parse HEAD` plus dirty/clean status. |
| Gates run | `local-full`, `siliconflow-model-inventory`, `provider-smoke`, `provider-full`, `external-tools`, `stress`. |
| Models | Authenticated non-Pro model selected for chat/tool, embedding, and rerank if used. |
| Run ids | All run ids from API/Web/provider/stress gates. |
| Artifacts | Absolute artifact directory. |
| Failures | Gate name, command, error, classification, and whether rerun passed. |

The complete integration pass is accepted only when every required gate above passes and the evidence package is sufficient to reproduce or debug each run without exposing secrets.

## New Session Prompt

When starting a fresh session, paste this request:

```text
我们继续 rove 的完整集成测试。请先阅读 docs/runtime/full-integration-runbook.md、docs/runtime/integration-testing.md、docs/runtime/provider-smoke.md、.env.integration.example，以及本地 .env.integration。当前工作目录是 D:\Study\project\agent\rove。

目标：按 full-integration-runbook 的顺序执行全面测试：先确认 local-full 基线，再用 .env.integration 里的 SiliconFlow/OpenAI-compatible 配置查询 /v1/models，过滤掉 Pro/ 开头模型，选择一个非 Pro 模型做 provider-smoke，然后继续 provider-full、external-tools、stress/长跑测试。每一步都要保存 artifacts，核对 API /runs、/runs/{run_id}/report 和 Web 面板历史/详情记录。遇到失败先定位原因并修复，再继续。

注意：真实 provider key 已经写在本地忽略文件 .env.integration 里；不要把 key 写入 tracked 文件或最终报告。先运行 git status，保护已有未提交改动，不要 reset 或 checkout 用户改动。
```

If the new session should only prepare and not execute, replace `执行全面测试` with `先检查配置并给出执行计划，不要启动服务或发起真实 provider 请求`.
