# Rove Benchmark Core Report

## Summary

The deterministic `local-full` gate passed on both default ports and custom ports after fixing two release blockers found during the smoke pass:

- Web history replay now handles `prompt_built` stream events instead of reducing state to `undefined`.
- The local and provider integration runners now merge the Web origin into `ROVE_API_CORS_ORIGINS` before starting the API for Web smoke.

Evidence package: `benchmarks/results/local-full-2026-06-12`
Commit: `ece76d2bcf1ff89592169f0c9d40e87bfbe0c23b`

## Harness Regression

Passed.

- `local-full-default`: API `127.0.0.1:8787`, Web `127.0.0.1:3000`, Playwright real-API suite `3 passed`.
- `local-full-custom`: API `127.0.0.1:18788`, Web `127.0.0.1:13000`, Playwright real-API suite `3 passed`.
- Each local-full profile captured 6 API smoke state files and `/runs?limit=25` output.
- Failure scenario evidence is a captured `tool_call_failed` event; the state snapshot may still show `running` because the runner waits for the failure event rather than terminal job status for that scenario.

## Context Ablation

Not run in this package. This pass is scoped to release smoke evidence, not context-window ablation.

## Working Memory Ablation

Not run in this package. Local-full covers memory/state isolation by using temp workspaces and `.rove-integration-state`, but no memory ablation experiment was executed.

## Recovery And Resume Ablation

Covered at smoke level only.

- Approval approve/reject flows were exercised through the API state artifacts.
- Request-input resume was exercised through both API smoke and Web real-API tests.
- Restart recovery and stress restart were not run in this package.

## Provider Gate Evidence

No real provider was configured in this environment.

- `cargo test --test provider_smoke` passed with all opt-in real-provider gates disabled.
- `.env.integration` was absent.
- OpenAI/Anthropic/provider integration variables were absent.
- Local Ollama was unavailable on `http://localhost:11434`.
- The provider integration runner was executed in an all-skipped, keyless `ollama` configuration to produce a redacted skipped-gate summary. This is not a provider connectivity pass.

## Failure Classification

Development failures found and fixed:

- `harness`: Web requests were rejected with `origin not allowed` when the smoke runner started Web on `127.0.0.1` without adding the matching API CORS origin.
- `runtime/web UI compatibility`: Web history replay crashed on `prompt_built` events because the TypeScript stream-event union and reducer did not include that event.
- `environment`: A cold worktree build hit Swagger UI zip TLS download and `aws-lc-sys` MSVC compile instability; final smoke runs reused the already-built shared target directory.

Current provider status:

- `external configuration missing`: no real-provider key/model/base URL and no reachable local Ollama service were available, so real provider smoke was not claimed.

