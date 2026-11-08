# Hosted delivery acceptance — 2026-11-09

Commit: `b00232e` (`feat/posix-delivery`). Verdict: **FAIL**, 10 required checks passed, 2 failed, 1 external MCP gate not run. The machine report is preserved verbatim.

Environment: macOS arm64 (Darwin 27), Rust 1.99.1, Node 24.22.1, pnpm 12.16.1, pinned Playwright Chromium/headless-shell 1223; two regular browser workers, one hosted worker; canonical `TMPDIR=/private/tmp`. Node/pnpm differ from the documented 22/10 baseline. Clippy generated one untracked macOS schema; no tracked source changed during acceptance.

Passed: fmt, workspace clippy, MCP transport, Engine/planning, tool safety, ProductStore, TypeScript, 1011 Web unit/component tests, production build and 234 regular browser tests (7 specialized cases skipped).

Failures:

- API: 197 passed, one local MCP stdio fixture probe returned 504 instead of 200 within its 2-second budget. Its environmental cause is not established; retain the failure rather than increasing the timeout.
- Hosted browser: migration passed, then continuity/input/cancellation timed out before the next message was submitted; two later tests did not run, one optional advanced case skipped. Trace review found a real pointer-send race: while the click was being dispatched, terminal transcript restoration disabled Send and swallowed the click, although the draft remained. A separate fix and pointer-specific regression are required; this is not a successful acceptance.

Additional manual scratch-fixture inspection showed that rollback incorrectly rejects an unchanged created file without a final newline. The modified-file negative check preserved newer bytes. Rollback remediation is tracked separately; no successful manual rollback claim is made.

No credentialed provider tasks, clean-machine installation, installed Desktop, signing, external MCP interop or actual engineering deployment is claimed.
