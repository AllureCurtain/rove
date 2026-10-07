# Acceptance run 2026-10-07 · `31705ec`

## Verdict

**PASS** — 11 of 12 checks passed, 0 failed, 1 gated check not run.

## Scope

Full `scripts/product-acceptance.ps1` run on the external control surface
(`rove-api` + `apps/web`) covering: formatting, lints, the API contract suite
(195 tests, single-threaded), MCP transport and hardening, the engine/planner
loop, tool-safety boundaries, product-store persistence, Web typecheck, Web
unit/component tests, the Web production build, and the Playwright
browser-boundary suites.

Validated with the deterministic `fake` model — no external provider
credentials or network calls were required. A credentialed-provider run is
tracked separately in `TODO.md`.

## Environment

- OS: Microsoft Windows 10.0.26200
- cargo 1.99.0 / rustc 1.99.0, node v24.9.0, pnpm 10.30.3
- `ROVE_E2E_WORKERS=4` (bounded Playwright parallelism for this host)
- `ROVE_PROJECT_TRUST_STORE` pinned to a per-run scratch path by the
  acceptance script (see `scripts/product-acceptance.ps1`)

## Artifacts

- `PRODUCT_ACCEPTANCE_REPORT.json` — verbatim script output: per-check exit
  codes, durations, and output tails.

## Notes

- `mcp-filesystem-smoke` was `not_run`: it is gated behind
  `ROVE_MCP_FILESYSTEM_SMOKE` and exercises a real MCP filesystem server.
- The first run of this suite surfaced a real defect: the operator trust
  store's legacy-migration size bound was applied to the live sqlite store,
  so an operator store that grew past 512 KiB made every trust decision fail.
  Fixed in `d2c4b57` (PR #15); this run validates the fix end to end.
