# First-use acceptance — 2026-11-09

Commit: `092ef2d` (`fix/product-first-use`). Verdict: **FAIL** — 11 checks passed, hosted-bundle browser acceptance failed, one external MCP gate was not run. [Machine report](PRODUCT_ACCEPTANCE_REPORT.json) is preserved verbatim.

The full run included Rust formatting/clippy, API/MCP/engine/safety/ProductStore checks, Web typechecking/unit tests/build, regular browser tests and hosted-bundle real-API tests. It used macOS arm64, the pinned Playwright Chromium/headless-shell build 1223, two regular browser workers, `CARGO_TARGET_DIR` shared with the root checkout, and `TMPDIR=/private/tmp` to avoid SQLite rejecting macOS's `/var` alias under its no-follow guard. Toolchain versions are in the machine report.

The hosted suite stopped in the explicit cancellation case: after an input card became visible, its helper synchronously counted Stop buttons before the composer had settled and threw `expected an active run with a Stop control`. Migration passed; the two later serial cases did not run. A prior standalone hosted run passed all four product cases, but that does not change this aggregate verdict. The follow-up replaces the immediate count with Playwright's waiting visibility assertion and a normal pointer click.

Separately verified before this run: 1008 Web tests, 228 regular browser tests, 2 production-prefetch tests, 4 hosted real-API cases, and 2229 Rust workspace tests passed. Fresh blind reviews covered provider credential handling and terminal/restoration races. None of these results prove real-provider task quality, third-party MCP interoperability, installed Desktop behavior, code signing, or actual engineering deployment. Those remain separate gates.

No raw provider keys, real project inputs or private customer data were used; browser tasks operated on disposable workspaces with the deterministic fake model.
