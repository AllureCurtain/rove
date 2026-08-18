# User State Directory Migration — Diff Summary

## Path contract and migration engine

- `apps/bootstrap/src/user_state.rs`: cross-platform `ROVE_DATA_ROOT`, canonical
  workspace storage keys, identity marker, user-state layout, MCP authority,
  redaction, Windows case/8.3/extended UNC handling, and fail-closed boundaries.
- `apps/bootstrap/src/state_migration.rs`: bounded dry-run/apply/prune engine,
  file classification, hashing, SQLite snapshots, transactional state-index
  path rebasing, prepared/final journal, receipts, locks, conflict backup, and
  safe legacy pruning.
- `runtime/src/state/index.rs`: public transactional
  `rebase_artifact_paths` for the six indexed run/task/report path fields while
  preserving external or malformed values.
- `apps/bootstrap/tests/state_migration.rs`: 23 end-to-end migration behavior
  cases; `scripts/state-migration-smoke.ps1`: disposable Windows CLI smoke,
  verified under both Windows PowerShell 5.1 and PowerShell 7.

## Shared configuration and authorities

- `apps/bootstrap/src/config.rs`, `lib.rs`, `Cargo.toml`: empty-sentinel defaults,
  pinned user-state roots, resolved accessors, layout creation, ProductStore
  path, and exported contract/migration types.
- `apps/bootstrap/src/project_trust.rs`, `registry.rs`: effective MCP catalog
  digest/authority and bounded catalog loading without changing Trust grants or
  Runtime tool safety.
- `runtime/src/tools/mcp_config.rs`: side-effect-free absent-catalog reads and
  target-lock-serialized, one-time legacy promotion for Product Settings.
- `apps/bootstrap/src/user_config/*`: reusable permission hardening for
  user-scoped contract files.
- `.rove/config.example.toml`: documents unset contract defaults and explicit
  override compatibility.

## Product entry points

- `apps/cli/src/cli/{args,config,runtime,state,ui}.rs`, `main.rs`, and
  `tui/app.rs`: `state paths`, `state migrate`, data-root injection, resolved
  state/memory/task paths, and deterministic test roots.
- `apps/api/src/lib.rs`, `product/migration.rs`, `product/routes.rs`,
  `product/mcp.rs`: standalone and embedded layout pinning, API-global
  ProductStore, legacy-until-materialized M1 discovery, marker-bound canonical
  runtime-index validation, job rebind/MCP path pinning, and first-write MCP
  promotion with both legacy/contract health-cache invalidation.
- `tests/api.rs`, `tests/provider_smoke.rs`: isolated data roots and compatibility
  assertions so tests never touch the real user profile.
- `scripts/integration-smoke.ps1`, `scripts/provider-integration.ps1`: bind
  ProductStore/default contract state to each disposable integration
  `ROVE_DATA_ROOT`.

## Current documentation

- Root `README.md`, `docs/ONBOARDING.md`, `docs/runtime/{README,architecture,
  implementation-guide,implementation-status,subsystems,acceptance-matrix}.md`:
  current implemented contract and limitations.
- `docs/design/2026-08-16-user-state-directory-migration-design.md` and
  `docs/plans/2026-08-16-user-state-directory-migration.md`: implemented status
  and retained design/acceptance rationale.
- `STATE_LAYOUT_AND_MIGRATION.md`, `SUMMARY.md`, `IMPLEMENTATION_LOG.md`,
  `VERIFICATION.md`, and this file: operator contract and handoff evidence.

## Intentionally unchanged

- No changes to `.worktrees/read-only-review` or its uncommitted content.
- No Web/TypeScript UI, `core/`, model-provider protocol, Cargo lockfile, or npm
  dependency change.
- User config root `~/.rove`, Project Trust authority, canonical event schema,
  approval/input durability, and direct-runtime embedding fallbacks remain
  separate existing contracts.
