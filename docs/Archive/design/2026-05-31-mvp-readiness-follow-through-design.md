# MVP Readiness Follow-Through Design

Date: 2026-05-31

## Purpose

Move rove from "local MVP works on the maintainer machine" toward "the runtime can be verified, started, and evaluated by another developer with fewer hidden assumptions." The work follows the gaps identified after the current audit:

- real provider gates need to be run and recorded;
- the local integration runner has a port configuration defect;
- first-run startup is still a multi-terminal manual flow;
- onboarding and release/security boundaries need sharper documentation.

This design does not expand the MVP into SaaS, Browser workspace, Desktop workspace, or multi-user hosted operation.

## Scope

### Included

1. Make the local full integration runner work on non-default Web ports.
2. Add a one-command local development launcher for API + Web.
3. Improve README and runtime docs for first-run, integration, provider, release, and security readiness.
4. Run deterministic verification after implementation.
5. Run SiliconFlow/OpenAI-compatible provider inventory and smoke gates when local credentials are present, recording redacted artifacts.

### Out of Scope

- Automating every provider-full, external-tools, and stress scenario in a single large runner.
- Shipping a packaged installer or binary release in this change.
- Adding hosted identity, billing, distributed rate limiting, Browser workspace, or Desktop workspace.
- Changing shell/tool security defaults without separate evidence that the current defaults are unsafe for the MVP target.

## Approach Options

### Option A: Verification-first and developer-readiness polish

Fix the integration runner defect, add a dev launcher, update docs, run deterministic gates, then run provider smoke with current local credentials. This keeps the change focused and immediately raises confidence in real use.

Recommended because it addresses the highest-risk usability and verification issues without destabilizing the runtime core.

### Option B: Full automation of every integration gate

Extend `scripts/integration-smoke.ps1` to run local-full, provider-full, external MCP, RAG, and stress in one profile system. This creates stronger automation but mixes network variability, quota failures, external tools, and long-running stress into one large script.

Not recommended for this step. It is too much surface area for a readiness polish pass.

### Option C: Product UI onboarding first

Add Web-side provider health cards and configuration diagnostics before touching scripts. This helps users, but it depends on first having reliable integration and startup flows.

Useful later, but not the first move.

## Design

### 1. Port-configurable Web E2E

`web-ui/playwright.config.ts` should derive the base URL from environment:

- `PLAYWRIGHT_BASE_URL` has highest priority;
- otherwise use `ROVE_WEB_PORT`;
- otherwise default to `http://localhost:3000`.

The Playwright `webServer.command` should start Next.js on the same port that the test base URL uses. The integration runner should set `PLAYWRIGHT_BASE_URL` and `ROVE_WEB_PORT` before invoking Playwright. This fixes the current defect where `scripts/integration-smoke.ps1 -WebPort 13000` starts Web on port 13000 while Playwright still tries to start/use port 3000.

Acceptance:

- `scripts/integration-smoke.ps1` passes with default ports.
- `scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:<free-port> -WebPort <free-port>` passes.

### 2. One-command local launcher

Add `scripts/dev.ps1` as a Windows-first development launcher because the current project and runbooks are PowerShell-oriented. It should:

- load `.env.integration` when present, without requiring it;
- support fake mode by default;
- support provider mode by respecting caller-supplied `ROVE_PROVIDER`, `ROVE_MODEL`, `OPENAI_API_BASE`, `OPENAI_API_KEY`, and related env vars;
- accept `-ApiAddr`, `-WebPort`, `-Workspace`, and `-InstallWebDeps`;
- check `cargo` and `pnpm`;
- check whether the requested API and Web ports are already listening;
- start `cargo run --bin rove-api -- --addr <addr> -C <workspace>`;
- start `pnpm exec next dev --port <port>` in `web-ui`;
- print API URL, Web URL, workspace path, provider/model, and state path;
- stop both process trees on Ctrl+C or script exit.

The script should not write secrets to logs, and it should not create tracked files.

Acceptance:

- launching fake mode starts API and Web on requested ports;
- the process tree is cleaned up on exit;
- docs explain how to use it.

### 3. Provider verification pass

Use existing local `.env.integration` if present. The pass should:

- query SiliconFlow `/v1/models` with the local key;
- filter out model ids starting with `Pro/`;
- save a redacted/non-secret inventory artifact outside the repo;
- choose a non-Pro candidate, preferring the configured `ROVE_PROVIDER_SMOKE_OPENAI_MODEL` if it appears in the authenticated inventory;
- run `cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture`;
- classify failures as configuration, quota/rate-limit, model capability, or runtime defect.

This pass may reveal external provider problems. Those are not automatically code defects unless the error points to rove request construction, stream parsing, or tool-use handling.

Acceptance:

- provider-smoke passes, or a redacted artifact records the external/provider reason it could not pass.

### 4. Docs and release/security readiness

Update existing docs instead of inventing a parallel documentation tree:

- root `README.md`: add `scripts/dev.ps1` quick start and clarify fake vs provider mode.
- `web-ui/README.md`: mention configurable port and proxy base.
- `docs/runtime/integration-testing.md`: document custom port support and the exact local-full commands.
- `docs/runtime/full-integration-runbook.md`: keep full gate sequence, add where new dev launcher fits, and record provider evidence expectations.
- add `docs/runtime/release-readiness.md`: MVP release checklist covering deterministic gates, integration gates, provider smoke, external tools, stress, packaging notes, security notes, and known out-of-scope items.

Acceptance:

- a new developer can identify how to run local fake mode, provider smoke, full local integration, and release-readiness checks without reading historical design docs.

### 5. Security posture

The current security posture is acceptable for local-first MVP if documented precisely:

- loopback API is unauthenticated by default;
- non-loopback API requires token auth unless explicitly marked unsafe;
- Web proxy injects server-side token and does not expose it to browser code;
- filesystem tools are workspace-bound;
- shell has timeout/output/env/denylist controls but is not a full sandbox;
- memory tools reject obvious secrets, but users should not intentionally save secrets;
- integration artifacts and `.env.integration` must remain untracked.

This pass should document the posture and add startup/readiness warnings where low-risk. It should not silently change default shell or approval behavior.

## Testing Plan

Deterministic gates:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Integration gates:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:<free-port> -WebPort <free-port>
```

Provider gate when credentials exist:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "<authenticated non-Pro model>"
cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Risks

- Next.js may rewrite `web-ui/next-env.d.ts` depending on dev/build mode. If that happens during verification, restore the tracked generated file format unless the project intentionally changes it.
- Provider smoke can fail because of key, quota, model availability, or model tool-call support. Preserve redacted evidence and do not conflate provider failures with local MVP failures.
- Long stress automation can consume time and provider quota. Keep stress as a documented follow-up unless specifically running the full external test pass.

## Completion Criteria

This follow-through is complete when:

- custom-port local-full integration passes;
- one-command dev launcher exists and is documented;
- deterministic Rust and Web gates pass;
- provider-smoke has either passed or produced a classified redacted failure artifact;
- release/security readiness docs exist;
- the worktree has no accidental generated-state changes.
