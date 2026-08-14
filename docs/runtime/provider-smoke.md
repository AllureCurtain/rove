# Provider Smoke

Provider smoke tests are opt-in checks for real model endpoints. They are not part of the default deterministic test suite because they require credentials, network access, local Ollama availability, or provider-specific quota.

The user-owned Provider catalog and CLI/TUI model-selection behavior have
deterministic coverage, but the external-provider gate has not been run for the
2026-08-12 implementation slice. A skipped gate proves only the skip path.

## User catalog setup

Normal CLI/TUI startup reads `~/.rove/config.toml` (or
`$env:ROVE_CONFIG_ROOT\config.toml` for an isolated test setup). Store only a
credential reference:

```toml
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
```

`auth.secret` may instead reference a bounded file or keyring entry. Never put
the credential value in TOML. With no configured profile, normal startup
returns `provider_onboarding_required`; use `--model fake` only when the local
deterministic path is intended. TUI `/model` lists the models configured in the
catalog (`inventory_fresh=false`), while the API inventory route performs the
live remote list operation.

## Default behavior

```powershell
cargo test -p rove-integration-tests --test provider_smoke
```

With no smoke gates enabled, the tests exit early and should pass.

## OpenAI

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "gpt-4.1-mini"
cargo test -p rove-integration-tests --test provider_smoke openai_real_provider_smoke_when_enabled -- --exact --nocapture
```

Set `OPENAI_API_BASE` when testing a compatible endpoint that is not OpenAI.

## OpenAI Responses

Use `openai-responses` for OpenAI's `/v1/responses` endpoint. This path is
separate from `openai`, which continues to use `/chat/completions`.

```powershell
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL = "gpt-4.1-mini"
cargo test -p rove-integration-tests --test provider_smoke openai_responses_real_provider_smoke_when_enabled -- --exact --nocapture
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
official OpenAI APIs, OpenAI Responses, relay or gateway APIs that
expose the OpenAI-style chat API, Anthropic, and local Ollama.

For product use through either Web surface, provider targets can be supplied as
per-run profiles: `provider_type`, API base URL, key environment-variable name
when needed, model id, and an optional display `name`. Clients cannot submit
`wire_protocol`; the system maps it from `provider_type` and may echo it only as
response/diagnostic metadata. The browser sends only the environment-variable
name, never the key value. The API route `POST /providers/test` checks model
inventory for OpenAI, OpenAI Responses, Anthropic, and Ollama profiles, then
`POST /jobs` can carry the same profile for the actual run. `fake` is accepted
for deterministic local runs.

The dedicated provider runner automates inventory, direct provider smoke, API
jobs, stress/restart options, and redacted evidence for OpenAI, OpenAI
Responses, Anthropic, and Ollama profiles. Its browser step now creates
API-backed product state, opens the exact product session in the default shell,
captures the browser's `POST /api/jobs` job/run IDs, and verifies those exact IDs
in the report and product transcript. `-SkipWebSmoke` remains available for an
intentional provider/API-only diagnostic; the current browser-flow
implementation does not require it.

The runner implementation is verified structurally on the integrated C3 code,
but no external-provider C3 browser gate has been run. Do not infer external
interoperability from deterministic fake-provider `local-full` evidence.

For official OpenAI APIs:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini"
```

For a relay or gateway API, set the relay base URL and choose a model visible to
that account:

```powershell
$env:OPENAI_API_KEY = "<relay-or-gateway-secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>"
```

For SiliconFlow, `deepseek-ai/DeepSeek-V3.2` is one tested example, not a
hard-coded product dependency:

```powershell
$env:SILICONFLOW_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
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
model id, smoke logs, API run reports, the product state snapshot, Web
screenshot/result, the exact Web run report/transcript, and
`evidence-summary.json`. Use `-SkipModelInventory` for gateways that do not
expose `/models`, and use `-SkipWebSmoke` or `-SkipApiSmoke` only for focused
diagnostics.

Add `-RunStress` to run small sequential and concurrent provider job batches
after the API/Web checks. The examples below skip repeating the browser gate and
assume it was already collected once for the same provider configuration. Add
`-RunRestartRecovery` to restart the API against
the same isolated stress workspace and verify all completed stress run ids are
still visible. Tune counts and timeouts when quota is limited:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -SkipWebSmoke `
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
  -Provider openai `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -SkipWebSmoke `
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
  -Provider openai `
  -ApiBase "https://<gateway-host>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<provider/model-id>" `
  -SkipWebSmoke `
  -RunExternalMcp `
  -ExternalMcpToolName "mcp__mock_server__echo_remote"
```

## Anthropic

```powershell
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "1"
$env:ANTHROPIC_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL = "claude-3-5-haiku-latest"
cargo test -p rove-integration-tests --test provider_smoke anthropic_real_provider_smoke_when_enabled -- --exact --nocapture
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
cargo test -p rove-integration-tests --test provider_smoke ollama_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Expected result

Each enabled smoke runs two tiny checks: a direct final-answer request and one
native `echo` tool-use request. Passing the smoke proves the configured provider
can be reached, stream events can be normalized, native tool-use events can
round trip through the engine, and the engine can complete or step-limit the
minimal tool run without losing the tool call or tool result.

All deterministic provider paths now pass their decoded events through the
shared `TurnAssembler` in Core. Strict provider clients require a terminal
stream event and reject malformed, duplicate, truncated, or oversized calls
before ToolRegistry execution. Capability checks are performed at the provider
client boundary before HTTP request dispatch. Existing embedded clients retain
the legacy EOF marker until they opt into `requires_terminal_event()`. These
guarantees are covered by the local models/core tests; the smoke commands below
remain opt-in interoperability evidence only.

The normalized stop reason is preserved in the canonical assistant turn. The
current protocol mappings are: OpenAI Chat `stop`, `tool_calls`, `length`, and
`content_filter`; OpenAI Responses completed/end-turn and function-call
completion; Anthropic `end_turn`, `tool_use`, `max_tokens`, and
`stop_sequence`/refusal; Ollama `done_reason` values; and the corresponding
Fake text/tool scripted turns. A plain `Done` is only a legacy terminal marker;
it does not overwrite an already decoded stop reason.

`ProviderCapabilities` is checked before a provider request is dispatched and
again at the normalized assistant-turn boundary. Tool calls from a provider
that does not declare tool support, or multiple calls from one that does not
declare parallel support, fail before any tool action is created. The built-in
native protocols currently declare streaming and tool-call support; their
parallel-call declaration is used by request construction and validation, not
as an unused compatibility hint.

Some OpenAI Chat Completions models keep calling the same tool after receiving a valid
tool result instead of producing the requested final text. The smoke therefore
requires the separate direct final-answer check for text generation, and the
tool-use check for native tool-call round trip. A tool-use run that reaches the
configured step limit after successful `echo` completion is classified as model
follow-up behavior, not as a transport or runtime failure.
