# Data Provenance

Scenario: local-full release evidence
Evidence directory: `benchmarks/results/local-full-2026-06-12`
Recorded on: 2026-06-13 Asia/Shanghai
Git commit: `ece76d2bcf1ff89592169f0c9d40e87bfbe0c23b`
Branch/worktree: `release-evidence-2026-06-12`
Worktree path: `D:\Study\project\agent\rove\.worktrees\release-evidence-2026-06-12`

## Build And Dependency Context

- Web dependencies were installed with `pnpm install --frozen-lockfile`.
- Rust API smoke runs reused the shared target directory `D:\Study\project\agent\rove\target` via `CARGO_TARGET_DIR` to avoid cold-build variance in this worktree.
- Rust `check` and `clippy` were not rerun in this pass per user instruction; the user confirmed those gates were already correct.

## Local-Full Commands

Default ports:

```powershell
$env:CARGO_TARGET_DIR = 'D:\Study\project\agent\rove\target'
Remove-Item Env:\ROVE_API_CORS_ORIGINS -ErrorAction SilentlyContinue
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 `
  -IntegrationRoot "$env:TEMP\rove-release-evidence-2026-06-13-default-final"
```

Custom ports:

```powershell
$env:CARGO_TARGET_DIR = 'D:\Study\project\agent\rove\target'
Remove-Item Env:\ROVE_API_CORS_ORIGINS -ErrorAction SilentlyContinue
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 `
  -ApiAddr '127.0.0.1:18788' `
  -WebPort 13000 `
  -IntegrationRoot "$env:TEMP\rove-release-evidence-2026-06-13-custom-final"
```

Both runs used fake-provider local runtime paths only. No external provider key or external model endpoint was used.

## Provider Smoke Commands

Default opt-in provider smoke check:

```powershell
$env:CARGO_TARGET_DIR = 'D:\Study\project\agent\rove\target'
cargo test --test provider_smoke
```

Provider integration runner skipped summary:

```powershell
$env:CARGO_TARGET_DIR = 'D:\Study\project\agent\rove\target'
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase 'http://localhost:11434' `
  -Model 'not-configured' `
  -IntegrationRoot "$env:TEMP\rove-release-evidence-2026-06-13-provider-skipped" `
  -SkipModelInventory `
  -SkipProviderSmoke `
  -SkipApiSmoke `
  -SkipWebSmoke
```

No `.env.integration` file was present. The following real-provider variables were absent: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `ROVE_PROVIDER_INTEGRATION_PROVIDER`, `ROVE_PROVIDER_INTEGRATION_API_BASE`, `ROVE_PROVIDER_INTEGRATION_API_KEY_ENV`, `ROVE_PROVIDER_INTEGRATION_MODEL`. Local Ollama was not available on `http://localhost:11434`.

## Verification Commands

```powershell
$env:CARGO_TARGET_DIR = 'D:\Study\project\agent\rove\target'
cargo test --test code_hygiene
cargo test --test provider_smoke

cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

## Artifact Policy

Artifacts copied here include API JSON state snapshots, run lists, process logs, Playwright result markers, and redacted provider-runner summaries. Temp workspaces, SQLite files, `.rove-*` runtime state directories, `.env.integration`, and raw provider secrets were not copied into this evidence package.

