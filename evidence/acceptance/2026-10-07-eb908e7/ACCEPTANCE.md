# Acceptance run 2026-10-07 · `eb908e7`

## Verdict

**PASS** — 12 of 13 checks passed, 0 failed, 1 gated check not run.

## Scope

Full `scripts/product-acceptance.ps1` run on the external control surface
(`rove-api` + `apps/web`) covering: formatting, lints, the API contract suite
(single-threaded), MCP transport and hardening, the engine/planner loop,
tool-safety boundaries, product-store persistence, Web typecheck, Web
unit/component tests, the Web production build, the Playwright
browser-boundary suites against `next dev` mocks, and — new in this run —
`web-serve-e2e`, which builds the production bundle and replays the real-API
suite against `rove-api --web-dist` on a single origin
(`scripts/serve-acceptance.ps1`).

Validated with the deterministic `fake` model — no external provider
credentials or network calls were required. A credentialed-provider run is
tracked separately in `TODO.md`.

## Environment

- OS: Microsoft Windows 10.0.26200
- cargo 1.99.0 / rustc 1.99.0, node v24.9.0, pnpm 10.30.3
- `web-serve-e2e` pins `ROVE_PROVIDER=fake`, `ROVE_MODEL=fake`,
  `ROVE_DATA_ROOT`, `ROVE_CONFIG_ROOT`, and `ROVE_PROJECT_TRUST_STORE` to a
  per-run scratch directory and serializes Playwright workers
  (`ROVE_E2E_WORKERS=1`) because the suite shares live product state.

## Artifacts

- `PRODUCT_ACCEPTANCE_REPORT.json` — verbatim script output: per-check exit
  codes, durations, and output tails.

## Notes

- `mcp-filesystem-smoke` was `not_run`: it is gated behind
  `ROVE_MCP_FILESYSTEM_SMOKE` and exercises a real MCP filesystem server.
- `web-serve-e2e` (88.2s) proves the `rove-api --web-dist` hosting form: the
  API builds `web-dist`, serves it on `127.0.0.1:18787`, and the real-API
  Playwright suite exercises session create/restore, provider onboarding,
  approvals, queued messages, and settings against the single origin.
- This run follows three contract-alignment fixes merged in `eb908e7`
  (PR #21): mock specs aligned with the `status: "pass"` provider-test
  verdict and the terminal handling of a 404 live-job attach introduced by
  PR #20.
