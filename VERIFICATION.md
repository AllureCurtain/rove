# Review workflow verification

All commands run from the Review worktree root unless a different working
directory is shown. Results below are real local exit-code evidence, not source
inspection or external interoperability claims.

## Focused Review evidence

| Command | Result |
|---|---|
| `cargo check -p rove-runtime -p rove-api -p rove-cli` | PASS |
| `cargo clippy -p rove-runtime --all-targets -- -D warnings` | PASS |
| `cargo test -p rove-runtime review -- --nocapture` | PASS (21 Review-related tests) |
| `cargo test -p rove-runtime tools::executor::tests::review_read_output_is_not_retained_as_a_durable_tool_artifact -- --nocapture` | PASS |
| `cargo test -p rove-api product::store::schema -- --nocapture` | PASS (6 tests before the explicit v13-v14 case was added) |
| `cargo test -p rove-api product::store::schema::tests::integrated_v13_upgrades_to_v14_without_rewriting_existing_state -- --nocapture` | PASS |
| `cargo test -p rove-api product::store::tests::reopening_the_store_marks_interrupted_reviews_needs_attention -- --nocapture` | PASS |
| `cargo test -p rove-integration-tests --test review -- --nocapture` | PASS (3 tests) |
| `cargo test -p rove-integration-tests --test api -- --nocapture` | PASS (116 tests) |
| `cargo test -p rove-integration-tests --test cli_review -- --nocapture` | PASS (3 tests) |

The main API integration case compares Git status and target bytes before and
after Review, asserts no target `.rove` directory is created, checks external
snapshot capture, recursively scans the Review run directory for a source
marker, and verifies stale classification after API restart.

## Web evidence

Working directory: `apps/web`.

| Command | Result |
|---|---|
| `pnpm test` | PASS (37 files, 251 tests) |
| `pnpm typecheck` | PASS |
| `pnpm build` | PASS |

## Final workspace gates

These rows are updated only after the command completes with its real exit
code.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | PASS |
| `cargo test -p rove-integration-tests --test tool_safety` | PASS (16 tests) |
| `git diff --check` | PASS |

## Not run

- Credentialed external Provider smoke: requires credentials, network, quota,
  and explicit opt-in.
- Real third-party/official filesystem MCP interoperability: explicit opt-in
  gate, not required for the local deterministic contract.
- Windows ConPTY, macOS/Linux packaging, signing, installed-Desktop journey,
  and broader stress/soak gates.
- Review-specific Playwright E2E. The deterministic Web unit/type/build gates
  cover the wired UI contract; this document does not infer browser behavior
  from them.
