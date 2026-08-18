# Review workflow diff summary

## Runtime and bootstrap

- Added `runtime::review` target/result contracts and snapshot/finalization
  implementation.
- Added snapshot-backed Review tools and a one-shot sanitized finding store.
- Added `RunMode`, read-only local environment adapters, pre-dispatch Review
  authorization, and shared Review persistence redaction.
- Added first-party Review registry and Engine assembly without hooks, MCP,
  process access, memory writes, or mutation capabilities.

## API and persistence

- Added Product Review request/status/result/finding contracts and routes.
- Added ProductStore schema v14 Review tables, indexes, repository operations,
  migration coverage, idempotency, single-flight behavior, pagination,
  cancellation, stale checks, and interrupted-run recovery.
- Reused the normal job supervisor, canonical event store, state artifacts,
  cancellation token, and Provider catalog while keeping Reviews outside chat
  turn claims and transcripts.

## CLI and Web

- Added `rove review` with uncommitted/base/commit targets, text/JSON/JSONL
  output, configured or deterministic Fake Provider assembly, Ctrl-C, and
  structured exit codes.
- Added the ProductApp Review launcher, Review Inspector tab, review state hook,
  typed API client/parsers, finding pagination/cancellation, Files navigation,
  state tests, and long-text/narrow-panel overflow guards.

## Tests and documentation

- Added Runtime, environment, executor, API store/migration, integration CLI/
  API, OpenAPI, and Web component/client coverage.
- Added current-state Review documentation and changed current ProductStore
  references from v13 to v14 while preserving the historical v12-v13 migration
  explanation.

## Intentionally unchanged

- No dependency crates or npm packages were added.
- No normal-run tool permission, Provider protocol, conversation lifecycle,
  canonical event family, or target-workspace state authority was replaced.
- No generated `.rove`, `target`, `.next`, `node_modules`, SQLite, log, or
  external Review state is intended for commit.
- The separate `.worktrees/user-state-migration` worktree was not inspected,
  modified, merged, or staged.
