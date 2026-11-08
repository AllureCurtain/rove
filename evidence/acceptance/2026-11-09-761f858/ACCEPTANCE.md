# First-use acceptance — 2026-11-09

Commit: `761f858` (`fix/product-first-use`). Final verdict: **PASS** — all 12 required checks ran and passed; one external MCP gate was not run. [Final machine report](PRODUCT_ACCEPTANCE_REPORT.json) is preserved verbatim.

The [initial attempt at the same commit](PRODUCT_ACCEPTANCE_REPORT.initial.json) recorded **FAIL**: 11 pass, one failed API suite, one gated check not run. Two stdio MCP fixture probes exceeded their 2-second timeout. A targeted rerun passed all five MCP API cases, then the entire acceptance was rerun without changing source or timeout settings and passed. The initial failure is retained; its exact environmental cause is not established.

Environment: macOS arm64 (Darwin 27), Rust 1.99.1, Node 24.22.1, pnpm 12.16.1, pinned Playwright Chromium/headless-shell 1223; two workers for regular browser tests, one for hosted tests; canonical `TMPDIR=/private/tmp`. Node/pnpm differ from the documented 22/10 baseline. The report marks one dirty file because clippy regenerates the untracked `apps/desktop/gen/schemas/macOS-schema.json`; source files remained at the stated commit.

The final run passed formatting, workspace clippy, 198 API tests, MCP transport, 117 Engine/planning tests, tool safety, 161 ProductStore tests, Web typecheck, 1008 Web unit/component tests, production build, 230 regular browser cases (seven specialized cases skipped in that profile), and four hosted-bundle cases (one optional advanced-workbench case skipped). Hosted tests cover migration, exact A/B continuity, refresh, pointer approvals and file-byte verification, input, cancellation, queue promotion/revocation, fork lineage and independent continuation. A regression first reproduced 400px of reading-position drift with offsetting row-height changes; it now passes and checks that a new user gesture takes over.

Additional checks preceding this run: 2229 Rust workspace tests and two production route-prefetch cases passed; fresh blind reviews covered credentials, terminal/successor recovery and the final history/acceptance changes. The POSIX report writer now preserves JSON validity on ANSI failure output and refuses PASS if required browser checks are skipped.

No credentialed real-provider tasks, real third-party MCP interop, installed Desktop flow, signing or actual engineering deployment is claimed. The fake model validates orchestration and interface behavior, not natural-language task quality. See earlier failures at [092ef2d](../2026-11-09-092ef2d/ACCEPTANCE.md) and [027e060](../2026-11-09-027e060/ACCEPTANCE.md).
