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
