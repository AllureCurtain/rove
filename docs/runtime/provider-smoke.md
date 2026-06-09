# Provider Smoke

Provider smoke tests are opt-in checks for real model endpoints. They are not part of the default deterministic test suite because they require credentials, network access, local Ollama availability, or provider-specific quota.

## Default behavior

```powershell
cargo test --test provider_smoke
```

With no smoke gates enabled, the tests exit early and should pass.

## OpenAI-compatible

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
```

Set `OPENAI_API_BASE` when testing a compatible endpoint that is not OpenAI.

## OpenAI Responses

Use `openai-responses` for OpenAI's `/v1/responses` endpoint. This path is
separate from `openai-compatible`, which continues to use `/chat/completions`.

```powershell
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_responses_real_provider_smoke_when_enabled -- --exact --nocapture
```

For a full provider evidence package when quota allows:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-responses `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini" `
  -RunStress `
  -RunRestartRecovery
```

## Generic Provider Integration

Use `scripts/provider-integration.ps1` when a provider should be treated as a
real release gate instead of only a unit-level smoke. The runner is generic for
official OpenAI-compatible APIs, OpenAI Responses, relay or gateway APIs that
expose the OpenAI-style chat API, Anthropic, and local Ollama.

For product use through the Web workbench, provider targets can be supplied as
per-run profiles: provider name, API base URL, key environment variable name
when needed, and model id. The browser sends only the environment variable name,
never the key value. The API route `POST /providers/test` checks model
inventory for OpenAI-compatible, OpenAI Responses, Anthropic, and Ollama profiles, then
`POST /jobs` can carry the same profile for the actual run. `fake` is accepted
for deterministic local runs.

The dedicated provider runner now automates the release gate for
OpenAI-compatible, OpenAI Responses, Anthropic, and Ollama profiles. It
normalizes provider names, queries the provider-specific model inventory
endpoint, dispatches the matching provider smoke test, submits API jobs with a
per-run provider profile, selects the same profile in the Web workbench, and
writes a redacted evidence summary.

For official OpenAI-compatible APIs:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini"
```

For a relay or gateway API, set the relay base URL and choose a model visible to
that account:

```powershell
$env:OPENAI_API_KEY = "<relay-or-gateway-secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>"
```

For SiliconFlow, `deepseek-ai/DeepSeek-V3.2` is one tested example, not a
hard-coded product dependency:

```powershell
$env:SILICONFLOW_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://api.siliconflow.cn/v1" `
  -ApiKeyEnv SILICONFLOW_API_KEY `
  -Model "deepseek-ai/DeepSeek-V3.2"
```

For Anthropic:

```powershell
$env:ANTHROPIC_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider anthropic `
  -ApiBase "https://api.anthropic.com" `
  -ApiKeyEnv ANTHROPIC_API_KEY `
  -Model "claude-3-5-haiku-latest"
```

For Ollama, start the local server and use a pulled model:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2"
```

The runner writes only non-secret artifacts: provider model inventory, selected
model id, smoke logs, API run reports, Web screenshot/report, and
`evidence-summary.json`. Use `-SkipModelInventory` for gateways that do not
expose `/models`, and use `-SkipWebSmoke` or `-SkipApiSmoke` for focused
diagnostics.

Add `-RunStress` to run small sequential and concurrent provider job batches
after the API/Web checks. Add `-RunRestartRecovery` to restart the API against
the same isolated stress workspace and verify all completed stress run ids are
still visible. Tune counts and timeouts when quota is limited:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -StressSequentialCount 5 `
  -StressConcurrentCount 3 `
  -StressJobTimeoutSeconds 180
```

Use `-RunLongSoak` only for a release-readiness pass where provider quota and
latency can absorb a longer run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -RunLongSoak `
  -LongSoakCount 100 `
  -LongSoakDelayMs 1000
```

Add `-RunExternalMcp` to verify a named MCP tool through the API/report path.
The runner prepares an isolated copy of `.rove/mcp_servers.example.json` for
the local mock fixture, and the default tool name matches that fixture:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -RunExternalMcp `
  -ExternalMcpToolName "mcp__mock_server__echo_remote"
```

## Anthropic

```powershell
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "1"
$env:ANTHROPIC_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL = "claude-3-5-haiku-latest"
cargo test --test provider_smoke anthropic_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Ollama

Start Ollama locally before running the smoke:

```powershell
ollama serve
```

Then run:

```powershell
$env:ROVE_PROVIDER_SMOKE_OLLAMA = "1"
$env:ROVE_PROVIDER_SMOKE_OLLAMA_MODEL = "llama3.2"
cargo test --test provider_smoke ollama_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Expected result

Each enabled smoke runs two tiny checks: a direct final-answer request and one
native `echo` tool-use request. Passing the smoke proves the configured provider
can be reached, stream events can be normalized, native tool-use events can
round trip through the engine, and the engine can complete or step-limit the
minimal tool run without losing the tool call or tool result.

Some OpenAI-compatible models keep calling the same tool after receiving a valid
tool result instead of producing the requested final text. The smoke therefore
requires the separate direct final-answer check for text generation, and the
tool-use check for native tool-call round trip. A tool-use run that reaches the
configured step limit after successful `echo` completion is classified as model
follow-up behavior, not as a transport or runtime failure.
