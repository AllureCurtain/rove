# Release Readiness Checklist

This checklist defines what must be true before treating the current rove MVP as
ready for another developer to install, run, and evaluate. It is scoped to the
local-first single-user MVP.

## MVP Boundary

Included:

- CLI one-shot and explicit REPL runs, plus the default bounded full-screen TUI.
- Local HTTP API with jobs, SSE, approvals, inputs, cancel, resume, and history.
- Standalone Web product shell, plus the advanced
  `/dev/workbench`, backed by the local API.
- Web Complete C0 product-control APIs, API-global ProductStore, exact product
  session/runtime binding, canonical-event transcript reads, strict M1 browser
  migration contracts, and typed Web client modules.
- Web Complete C1 default-shell adoption: API-authoritative product state,
  durable deep routes, explicit complete/partial/error transcript restore,
  exact `product_session_id` turns, focused reattachment/background status,
  provider persistence, and bounded ambiguous-start reconciliation.
- Web Complete C2 complete Settings surface and C3 migration gate, final product
  polish, and deterministic live-API default-shell acceptance.
- CDH G1-G7 durable controls and Fork/lineage, session configuration snapshots,
  usage/context/cost, bounded files/artifacts/images/diff, redacted evidence
  export, and workspace-scoped Settings/MCP management.
- Productization B-D native-first tool-call recovery, ignore-aware deterministic
  repository retrieval, and Artifact-backed result-history projection.
- Productization E user-owned Provider catalog, API/Web catalog convergence,
  per-turn CLI assembly, migration, onboarding/probes, and TUI `/model`.
- Productization F unified Send Message lifecycle across Runtime, API/SSE,
  ProductStore, Web, and TUI: durable FIFO queue, safe-boundary intervention,
  successor claims, revoke/attention recovery, and idempotent CAS races.
  ProductStore v14 reconciles the v13 parallel-v12 layouts and adds durable
  Review rows/findings. Focused Runtime/API/TUI,
  Web unit/type/build, mocked browser, and five live local fake-provider cases
  pass. This is F.1-F.3 evidence; F.4 older-history pagination/windowing and
  F.5 complete TUI restart recovery remain open. Windows ConPTY/PTY and
  external-provider gates remain unverified.
- Final TUI real-use slice F4/T7 is met on the Windows release CLI path with the
  locked SiliconFlow `openai` profile and `deepseek-ai/DeepSeek-V3.2`: three
  credentialed `success/final` turns, native list/glob/search/read tools,
  approval-backed `edit_file`, grounded finals, and secret-free evidence under
  `<evidence-root>/tui-gate-10`. This does not claim Desktop D6, Windows
  ConPTY/manual terminal, or the final A Gate.
- Tauri Desktop D0 delivery shell with embedded authenticated API, shared static
  Web build, native workspace commands, and current-platform Windows MSI/process
  evidence. macOS/Linux packaging and manual installation remain unverified.
- Local state under `.rove/`.
- Hard read-only Review for Git targets: immutable snapshot/digest, dedicated
  read-only tools/environment, bounded sanitized findings, API/CLI/Web
  projections, cancellation, stale detection, and conservative restart recovery.
- Folder, Repo, and Task workspaces.
- Built-in tools, MCP proxy, memory tools, fake provider, OpenAI /
  Responses, Anthropic, Ollama, named provider profiles, and the opt-in
  external process adapter.

Not included:

- Hosted SaaS operation.
- Multi-user identity, billing, or distributed rate limiting.
- Browser/Desktop **automation workspace** implementations (the Desktop product
  shell is a delivery host, not a new workspace kind).
- Full shell sandboxing beyond the current local policy controls.
- Built-in vector or provider-backed RAG retrieval.

## Out-of-scope Reminders

Do not treat the following as release blockers for the current local-first MVP:

- hosted multi-user deployment;
- OAuth/login, billing, or admin controls;
- Browser and Desktop automation workspace runtime implementations;
- macOS/Linux bundled Desktop packages and manual installation evidence;
- distributed rate limiting across multiple API processes.

## Deterministic Gates

Run these before any release candidate:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/web
pnpm test
pnpm typecheck
pnpm build
cd ..
```

Run the machine-readable aggregate acceptance from the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1
```

Acceptance:

- every command exits with code 0;
- no generated runtime state, logs, screenshots, SQLite files, or secrets appear
  in `git status --short`;
- any `apps/web/next-env.d.ts` churn from Next.js is inspected before commit.

## Web Complete C0-C3 Evidence

C0 release evidence must include the default Rust and Web CI gates plus focused
coverage for these contracts:

- ProductStore workspace/session/profile/preferences CRUD, schema migration,
  monotonic preference revisions, and bounded durable migration preparations;
- runtime-aligned product OpenAPI failures (`500` operation failure and `503`
  unavailable), with no wired operation advertising `501`;
- exact product-session resume, single-active-turn ownership, terminal release,
  stream finalization/replay, and shutdown drain order;
- ordered canonical-event transcript projection with typed partial reasons;
- strict/idempotent M1 migration, exact retry payload preservation for
  `product_session_active`, and preference conflict reporting;
- a preparation-only 30-second deadline and API-supervised apply that survives
  HTTP disconnect;
- canonical sorted runtime-store reservation, workspace containment,
  `SQLITE_OPEN_NOFOLLOW`, and symlink-parent rejection;
- strict Web response validation and a replay-safe same-origin migration state
  machine that never uploads raw keys.

C1 release evidence additionally includes:

- unit coverage for product routes, API catalog conversion, transcript
  projection, reducer hydration/deduplication, and exact turn requests;
- mock-backed Playwright coverage for refresh restore, explicit partial/error
  recovery, deep-route landings, session-switch races, focused reattachment,
  background attention status, API-persisted providers, and ambiguous job-start
  responses without duplicate submission.

C2 release evidence additionally includes:

- API coverage for preference revision CAS, default approval resolution,
  bounded Memory management, and redacted runtime health;
- unit coverage for strict Settings clients, catalog export safety, Memory
  transitions, keyboard matching, and default-approval turn requests;
- mock-backed Playwright coverage for all nine Settings sections, provider
  update, approval/step persistence, workspace/session mutations, Memory,
  runtime health, shortcuts, and mobile overflow.

C3 release evidence additionally includes:

- a default-shell migration gate that runs before product catalog reads, permits
  only `not_needed` or verified `complete`, preserves exact pending retries, and
  keeps invalid or uncertain state fail closed;
- mock-backed migration, recovery, responsive, focus, keyboard, reduced-motion,
  theme, state, and narrow-layout coverage without reclassifying injected faults
  as live evidence;
- deterministic `local-full` coverage against the live Rust API for migration,
  exact interleaved A/B continuation across refresh, approval, input,
  cancellation, Settings, deep routes, and one bounded `/dev/workbench` smoke;
- an updated provider runner that correlates browser-returned job/run IDs with
  the exact report and product transcript instead of guessing a latest run.

The integrated C3 implementation on `main` passed its original three
`local-full` real-API browser cases after merge. Productization integration now
passes five cases by adding unified-message promotion/revocation and
Fork/independent-child continuation. The external-provider browser gate was not
run and must not be claimed from deterministic fake-provider evidence.

For release claims that include real-terminal TUI behavior, run the opt-in Unix
PTY smoke separately:

```powershell
python scripts/tui-pty-smoke.py --run
```

It checks a fake-model frame, resize/redraw, clean exit, termios, and restore
sequences on Unix PTYs. Windows exits `77` with a typed skip because native
ConPTY automation is not included. A Windows skip must be reported as an
unverified platform gate, not converted to a pass.

## Local-Full Integration

Run both default and custom-port profiles:

```powershell
$rootDefault = Join-Path $env:TEMP ("rove-integration-default-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 -IntegrationRoot $rootDefault

$rootCustom = Join-Path $env:TEMP ("rove-integration-custom-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 `
  -ApiAddr "127.0.0.1:18788" `
  -WebPort "13000" `
  -IntegrationRoot $rootCustom
```

Acceptance:

- both commands print `local-full integration smoke completed`;
- Playwright reports all five real-API cases passed for each run: migration,
  default A/B product lifecycle, unified-message promotion/revocation,
  completed-session Fork/independent-child continuation, and bounded advanced
  smoke;
- run ids from the output appear in each run's `/runs?limit=25` artifact;
- API state, exact product transcripts/reports, and Web assertions cover
  migration, A/B continuation, refresh, approval, input, cancellation, Settings,
  deep routes, unified-message delivery, Fork lineage, and the bounded workbench
  run.

## Provider Smoke

Provider smoke is required before claiming real-provider readiness. It is not
required for deterministic local MVP operation.

Prefer the generic provider runner for provider reachability, API jobs, product
shell evidence, stress evidence, and evidence capture across OpenAI, OpenAI
Responses, Anthropic, and Ollama profiles. Its browser step now uses an exact
API-backed product session and verifies the browser-returned job/run IDs against
the matching report and product transcript:

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini"
```

For relay or gateway APIs, replace `-ApiBase`, `-ApiKeyEnv`, and `-Model` with
that account's values. If the gateway does not expose `/models`, pass
`-SkipModelInventory` and record the provider's own model-selection evidence
separately.

OpenAI Responses is a separate native provider gate from OpenAI chat
completions:

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

Anthropic and Ollama use the same runner with provider-specific inventory and
smoke dispatch:

```powershell
$env:ANTHROPIC_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider anthropic `
  -ApiBase "https://api.anthropic.com" `
  -ApiKeyEnv ANTHROPIC_API_KEY `
  -Model "claude-3-5-haiku-latest"

powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2"
```

When quota allows, add `-RunStress -RunRestartRecovery` to the provider runner
before release. This runs sequential and concurrent provider job batches,
restarts the API against the same stress workspace, and records
`stress-summary.json`, `stress-runs-before-restart.json`, and
`stress-runs-after-restart.json`. Add `-RunLongSoak` for a release-readiness
soak. Add `-RunExternalMcp` to prove the local mock MCP fixture is visible
through API/report records without using the real provider for that
deterministic fixture.

## Provider Gate Matrix

| Provider | Required before release claim | Long stress required | Notes |
|---|---:|---:|---|
| OpenAI official API | Yes when claiming official API readiness | Yes when quota allows | Includes relay/gateway-compatible surface. |
| OpenAI Responses official API | Yes when claiming Codex-style/OpenAI Responses readiness | Yes when quota allows | Uses `/v1/responses`; separate from chat completions. |
| OpenAI relay/gateway | Yes when claiming relay/gateway readiness | Yes when quota allows | Record gateway model inventory or `-SkipModelInventory` reason. |
| Anthropic | Yes when claiming Anthropic readiness | Optional unless target release advertises Anthropic as verified | Native Messages API path. |
| Ollama | Yes when claiming local-model readiness | Optional but recommended | Requires local Ollama server and pulled model. |

For OpenAI official APIs or relay/gateway APIs, manual inventory is
the same shape regardless of vendor:

```powershell
$env:OPENAI_API_KEY = Read-Host "Provider API key"
$env:OPENAI_API_BASE = "https://<provider-or-gateway>/v1"
$headers = @{ Authorization = "Bearer $env:OPENAI_API_KEY" }
$models = Invoke-RestMethod -Uri "$env:OPENAI_API_BASE/models" -Headers $headers
$visible = $models.data | Where-Object { $_.id } | Sort-Object id

$artifactRoot = Join-Path $env:TEMP "rove-provider-smoke"
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$visible | Select-Object id, owned_by | ConvertTo-Json -Depth 20 |
  Set-Content -LiteralPath (Join-Path $artifactRoot "provider-models.json")
```

Select a chat/tool model from the authenticated inventory, then run:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "<selected model>"
cargo test -p rove-integration-tests --test provider_smoke openai_real_provider_smoke_when_enabled -- --exact --nocapture
```

Per-run API/Web profiles also support `anthropic`, `ollama`, and `fake`; use
the Web provider selector or include the provider profile in `POST /jobs`.

Acceptance:

- the model inventory is saved without secrets;
- the selected model id is saved as a non-secret artifact;
- the smoke test passes, or the failure is classified as key/configuration,
  quota/rate limit, model tool-call capability, or rove runtime defect.
- `scripts/provider-integration.ps1` writes `evidence-summary.json` for the
  provider/API gate;
- the credentialed TUI gate is recorded separately under
  `<evidence-root>/tui-gate-10` with three `success/final` runs and no raw
  secret material;
- when Web smoke is included, its result contains exact browser-returned
  `job_id`/`run_id`, and the saved report and product transcript contain that
  exact binding;
- the external-provider Web gate remains `not_run`; the credentialed TUI gate
  does not substitute for Desktop or Web interoperability evidence.

## External Tools

MCP deterministic gate:

```powershell
cargo test -p rove-integration-tests --test mcp
```

Official filesystem MCP opt-in gate:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test -p rove-integration-tests --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

Acceptance:

- deterministic gates pass;
- external-tool artifacts stay in isolated integration state;
- any real external-server failure is recorded separately from local MVP health.

## Stress And Restart Recovery

Run stress after deterministic, local-full, provider-smoke, and external-tool
gates have passed.

Minimum local stress:

- 100 sequential fake-provider API jobs;
- 5 concurrent fake-provider API jobs;
- API restart after completed jobs;
- `/runs?limit=25` still lists previous completed runs after restart.

Provider stress should start smaller, such as 20 sequential provider jobs and 5
concurrent provider jobs, then scale only if quota and rate limits allow it.

Acceptance:

- no Rust panic;
- no SQLite corruption or lock timeout loop;
- no missing run reports for terminal runs;
- provider 429s are handled as provider capacity limits, not local runtime
  regressions, unless rove misclassifies or loses the error.

## Packaging Notes

Before a packaged release:

- record `git rev-parse HEAD`;
- build release binaries with `cargo build --release`;
- run `pnpm build` for the standalone Web application;
- document whether users start API and Web separately or through
  `scripts/dev.ps1`;
- include required runtime prerequisites: Rust/Cargo for source builds, Node.js
  and pnpm for the Web application, provider keys for real models.

`main` contains the Tauri Desktop bundle through PR #30. The final delivery
produced Windows MSI and NSIS packages and passed a bounded release-process
smoke. It does not yet provide signed public installers, verified manual
installation, macOS/Linux packages, or a generally available distribution.

The 2026-08-18 Desktop real-use slice additionally builds explicit per-machine
MSI/NSIS packages and configures the NSIS Start menu folder. Per-machine scope
is required because `%LOCALAPPDATA%\rove` is a user-state root and must never be
used as an install/uninstall target. The native credential prompt and typed
WebView receipt boundary are implemented, but the installed credentialed
journey remains blocked on the shared Provider onboarding service and must not
be claimed from package-build evidence. See `desktop-real-use.md`.

## Security Posture

Current local-first posture:

- API binds to `127.0.0.1:8787` by default.
- Binding to a non-loopback address requires `ROVE_API_TOKEN` unless
  `ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH=true` is explicitly set.
- The Web proxy injects `ROVE_API_TOKEN` server-side and does not expose it to
  browser JavaScript.
- Product migration rejects unknown/unbounded fields and raw-key-shaped data,
  validates workspace/runtime paths before commit, and retains only safe
  preference/profile references in API-global `product.sqlite`.
- Product migration apply and product job starts are API-owned tasks: client
  disconnect cannot silently cancel an accepted commit or leave an untracked
  active-turn claim, and shutdown drains their owners in dependency order.
- Runtime databases used by verified product bindings are canonicalized,
  workspace-contained when external paths are disabled, and opened with
  no-follow guards.
- Filesystem tools reject traversal and symlink/reparse escapes outside the
  workspace.
- Shell execution has timeout, output-size, environment-inheritance, denylist,
  and approval-policy controls, but is not a full sandbox.
- Memory tools reject obvious secret-like content, but users should not
  intentionally save secrets.
- `.env.integration`, `.rove/config.toml`, `.rove/mcp_servers.json`, logs,
  SQLite files, screenshots, traces, and run artifacts must remain untracked.

## Final Evidence Package

For a complete readiness pass, preserve:

- command output or logs for deterministic gates;
- `benchmarks/results/<scenario>-<YYYY-MM-DD>/` evidence packages with
  `DATA_PROVENANCE.md`, `rove-benchmark-core-report.md`, and `metrics.json`;
- local-full default and custom-port artifact directories;
- provider model inventory and selected model id;
- provider smoke output or classified failure notes;
- MCP gate output when run;
- stress snapshots when run;
- `git status --short` after the pass;
- `git rev-parse HEAD`.

Do not preserve or commit raw provider keys, bearer tokens, `.env.integration`,
or local runtime state.
