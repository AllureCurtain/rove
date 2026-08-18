# IMPLEMENTATION_LOG — 硬只读 Review 工作流

## Baseline

- Worktree: `.worktrees/read-only-review`（分支 `feature/read-only-review`）
- Baseline commit: `5fe9d70` (docs: add next-round productization and parallel task plans)
- Base of `main`: `f6676d1` (PR #33, productization integration)
- Working tree at start: clean

## Plan and design

- Plan: `docs/plans/2026-08-16-read-only-review-workflow.md`
- Design (confirmed by user on 2026-08-16):
  `docs/design/2026-08-16-read-only-review-workflow-design.md`

## Parallel-task boundary

- `.worktrees/user-state-migration` is a separate task line. Its uncommitted content is
  never read, modified, merged, or reviewed here. State/path capabilities are reused only
  through public interfaces (`StateStore`, `ProductStore`, `Workspace`).
- Merge order: this branch merges after the migration line; rebase onto migrated `main`
  before merging if needed.

## Verification commands (to be run with real exit codes; append results below)

Working directory: repository root unless noted.

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p rove-runtime
cargo test -p rove-integration-tests --test review
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test api
```

Web (from `apps/web/`):

```
pnpm test
pnpm typecheck
pnpm build
```

Not run (opt-in external gates, per plan): real external provider smoke, real third-party
MCP, Windows ConPTY, packaging/signing, installed-Desktop journey.

## Log

### 2026-08-16 — Phase 0: design review

- Verified current diff/Artifact/Run Inspector/ToolRegistry/Execution Environment/
  Project Trust/approval/Finalizer behavior against source at `5fe9d70`.
- Produced reviewable design; user confirmed; implementation started.

### 2026-08-16 — Phase 1: safety contract correction

- Reconciled the design with the current execution order: Review uses distinct
  HEAD/index/worktree identities, immutable snapshot-backed reads, external
  state, sanitized finding facts, and durable recovery rather than a
  non-resumable best effort.
- The implementation will add the Review mode guard before hooks/permission
  execution and keep the existing canonical Engine event lifecycle.

### 2026-08-16 to 2026-08-18 — Phase 2: runtime and persistence implementation

- Added the shared hard-read-only Review execution mode, immutable target
  snapshots, bounded target discovery/capture, sanitized finding facts, and
  versioned Review result/state contracts.
- Routed Review through the existing Runtime/Engine lifecycle and ToolRegistry;
  Review reads/searches use the same bounded workspace and execution-environment
  authorities while write, shell, MCP, hook, and approval paths are rejected.
- Added ProductStore v14 Review records, CRUD/claim/recovery APIs, explicit
  v13-to-v14 migration coverage, and restart reconciliation of stranded
  `queued`/`running` Reviews to `needs_attention`.
- Added API endpoints and recursive run-directory source-marker checks so source
  content cannot leak into durable Review artifacts or reports.

### 2026-08-17 to 2026-08-18 — Phase 3: product surfaces and hardening

- Added CLI `rove review` target capture, deterministic output, unavailable
  results with exit code `2` on capture failure, and process-level coverage.
- Added Web Review list/detail/refresh/launch surfaces, authoritative
  `GET /product/reviews/{id}` hydration (including stale detection), loading and
  error states, and long-title/path overflow protection.
- Added Runtime, API, CLI, and Web contract tests plus current-state runtime
  documentation and the Review workflow handoff documents.

### Verification and corrections

The final local gates were run with real exit codes:

- `cargo fmt --all --check` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --workspace` — PASS.
- `cargo test -p rove-integration-tests --test tool_safety` — PASS (16 tests).
- Review-focused Runtime, API, integration, and CLI suites — PASS; the CLI
  Review integration suite has 3 tests and the Review integration suite has 3
  tests.
- Web `pnpm test` — PASS (37 files, 251 tests); `pnpm typecheck` and
  `pnpm build` — PASS.
- `git diff --check` — PASS.

During implementation, the following issues were found and corrected before
the final gate:

- Clippy findings from the earlier implementation pass were fixed and the
  workspace `-D warnings` gate was rerun successfully.
- Review read/search output was initially eligible for durable Tool Artifact
  retention; retention was disabled for this mode and a regression test now
  proves the output is not persisted.
- A borrow-checker error around CLI `stable_hash` construction was corrected.
- CLI process tests initially exercised a stale executable; the helper now
  rebuilds the current binary once before process assertions.
- Final audit extended finding redaction to title/category/rule/evidence
  references, reports incomplete content hashes through `unchecked`, and
  excludes HEAD-relative changes that are byte-identical to an explicit base;
  focused Runtime Review coverage was expanded to 21 passing tests.

Optional external-provider, third-party MCP, Review-specific Playwright,
ConPTY, packaging/signing/installed-Desktop, and broader stress/soak gates were
not run because they require explicit services, credentials, or platform
environments.
