# rove-engine conventions

Global rules live in the root AGENTS.md; this file only covers `crates/engine/` specifics.

## Layers

```
src/engine/       Engine facade (facade.rs), unplanned/planned loops, StepRunner, model/tool turns, retry recovery, delegate turn-driver (subrun.rs)
src/planning/     plans, revisions, evaluation, execution policy and budgets
src/context/      ContextManager, compaction
src/conversation.rs  unified conversation message domain
```

Extracted domains live in sibling crates, not re-exported here:

- `rove-state` (crates/state): `state/` (StateStore, trace, index, …), `memory/`, `task_state` snapshots
- `rove-tools` (crates/tools): built-in tools, `Executor`, hooks, MCP client. Only the delegate turn-driver stays — in `engine/subrun.rs` — because it must call the turn primitives
- `rove-agents` (crates/agents): agent catalog, profiles, instructions, procedures
- `rove-foundation` (crates/foundation): serialized identity/lifecycle data, workspace boundary, environment port, prompt metadata
- `rove-review` (crates/review): read-only Review domain

## Conventions

- This crate depends only on `rove-models`, `rove-core`, `rove-protocol`, `rove-foundation`, `rove-agents`, `rove-state`, `rove-review`, `rove-tools`, and `rove-tools-text` — never on any app crate.
- Errors: internally use `thiserror` for typed errors with a stable `code`. Do not leak `anyhow` into the public API.
- New serializable fields always get `#[serde(default)]`, plus `skip_serializing_if` where appropriate, so old traces and snapshots still decode and unmodified records keep their exact bytes.
- Events may only be added, never have semantics changed. When `StreamEvent` changes, persistence, API/SSE, Web, and `tests/event_contract.rs` all change together.
- StateIndex schema may only change via versioned migrations in `crates/state/src/state/index.rs` (`CURRENT_SCHEMA_VERSION`). Refuse to open databases from a newer version. File artifacts are the source of truth; SQLite is the query and replay index.
- Recovery paths must not re-dispatch completed model or tool work; for unknown in-flight side effects, append an `interrupted` record and stop.
- Tool path resolution must go through the workspace boundary check and reject traversal and symlink escapes. Touching safety or approval logic requires negative tests.
- No crate-level `#[allow(dead_code)]`. Local exceptions must state their reason in place.
- Testing: in-module `#[cfg(test)]` tests invariants; `crates/engine/tests/*_contract.rs` tests cross-module contracts.
