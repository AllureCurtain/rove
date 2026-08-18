# Parallel Workstreams - Verification

These sections retain the branch-local evidence recorded for both workstreams.
Combined post-merge verification is reported in the merge handoff.

## Post-merge integration verification (2026-08-18)

| Check | Result | Evidence |
|---|---|---|
| Rust formatting | PASS | `cargo fmt --all --check` |
| Workspace Clippy | PASS | `cargo clippy --workspace --all-targets -- -D warnings` |
| User-state migration tests | PASS | `cargo test -p rove-app-bootstrap --test state_migration`: 23 passed |
| Review Runtime/API integration | PASS | `cargo test -p rove-integration-tests --test review`: 3 passed |
| Review CLI integration | PASS | `cargo test -p rove-integration-tests --test cli_review`: 3 passed |
| API integration | PASS | `cargo test -p rove-integration-tests --test api`: 117 passed |
| Web unit tests | PASS | `pnpm test`: 37 files / 251 tests |
| Web typecheck/build | PASS | `pnpm typecheck`; `pnpm build` |
| Full workspace Rust tests | NOT COMPLETED | `cargo test --workspace -j 1` exceeded the 20-minute execution limit while compiling `rove-api`; no assertion result was reported and the process was terminated. |

---

# User State Directory Migration — Verification

> Evidence date: 2026-08-18
> Worktree: `.worktrees/user-state-migration`
> Branch/baseline: `feature/user-state-migration` / `5fe9d70`

## Deterministic checks

| Check | Result | Evidence |
|---|---|---|
| Rust formatting | PASS | `cargo fmt --all --check` |
| Bootstrap unit tests | PASS | Full workspace run: 82 passed, including explicit-path compatibility and contract/legacy discovery fallback |
| Migration behavior tests | PASS | `cargo test -p rove-app-bootstrap --test state_migration -- --nocapture`: 23 passed |
| Runtime Product MCP configuration | PASS | focused `tools::mcp_config` suite: 6 passed, including zero-side-effect reads and one-time legacy promotion |
| API state/MCP boundaries | PASS | marker-bound contract-state regression and Product MCP focused regressions passed; API suite: 133 passed |
| Runtime SQLite error mapping | PASS | focused `rove-runtime` test: 1 passed |
| Workspace clippy | PASS | `cargo clippy --workspace --all-targets -- -D warnings` |
| Workspace Rust tests | PASS | `cargo test --workspace -j 1` (all packages and integration/doc tests passed) |
| Windows CLI smoke | PASS | 19/19 assertions in both Windows PowerShell 5.1 and PowerShell 7 |
| Web unit tests | PASS | `pnpm test`: 36 files / 241 tests |
| Web type/build checks | PASS | `pnpm typecheck`; `pnpm build` |
| Mocked browser suite | PASS | `pnpm test:e2e`: 56 passed; 5 explicitly gated real-API cases skipped |
| Live product integration | PASS | isolated non-default-port `local-full`: 5/5 real-API Playwright cases passed |
| Diff/document hygiene | PASS | `git diff --check`; Markdown links/fences/headings/trailing whitespace reviewed |

The first full-workspace attempt used Cargo's default parallelism and hit the
Windows linker's shared PDB limit (`LNK1318: PDB LIMIT (12)`) without any Rust
test or assertion failure. The serial rerun (`-j 1`) completed successfully and
is the authoritative workspace result above.

The 23 migration cases cover fresh/no-source behavior, dry-run purity and
classification, apply/idempotency, ordinary and prepared-SQLite interruption
recovery, corrupt journal tolerance, corrupt SQLite rejection, workspace lock
contention, invalid/nested data roots, source/target/migration-metadata symlink
boundaries, keep/backup conflicts, replaced SQLite conflict, API-global
ProductStore conflict, safe prune/partial prune, usable state indexes, real
post-prune resume, and MCP Trust digest continuity.

Two final M1 compatibility regressions prove that an identity marker alone
does not hide unmigrated legacy runs, that discovery switches to contract
state only after `state.sqlite` or `runs/` materializes, and that API artifact
verification accepts only marker-bound contract state outside the project
workspace.

The final MCP compatibility regressions prove that reading a missing catalog
does not create its parent, the first Product Settings mutation validates its
request before materializing a marker-bound contract layout, legacy servers
are promoted exactly once under the target lock, a later legacy edit cannot
overwrite an existing contract catalog, and both legacy/contract health-cache
keys are invalidated after a mutation.

The Windows smoke starts the built `rove.exe` against a disposable workspace
and data root. Its 19 assertions cover fresh path inspection, legacy discovery,
dry-run zero writes, apply, idempotency, conflict keep/backup, prune, and
post-migration path inspection. Exact resume is intentionally evidenced by the
real `StateStore` Rust regression rather than fabricated CLI artifacts.
The script was also run through both `powershell.exe` (5.1) and `pwsh` after
hardening native stderr handling; both hosts returned exit code 0.

The final `local-full` run used API `127.0.0.1:18787`, Web port `13000`, and
the disposable root `<TEMP>/rove-user-state-live-324082…`.
All five live product-shell cases passed. The runner explicitly set
`ROVE_DATA_ROOT=<integration-root>/data-root`; inspection confirmed the
ProductStore was isolated there rather than in the operator's real profile.
`provider-integration.ps1` now applies the same isolation contract.

## Not run / unverified

- Credentialed external Provider: **Not Run / Unverified**.
- Real third-party or official filesystem MCP interoperability:
  **Not Run / Unverified**.
- Installed Desktop journey and signing: **Not Run / Unverified**.
- macOS/Linux packaging and platform execution: **Not Run / Unverified**;
  cross-platform branches compile in the Rust workspace but were not executed
  on those operating systems.
- Windows ConPTY and broader stress/soak: **Not Run / Unverified**.

These skipped gates are not claimed as interoperability or release evidence.
---

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
