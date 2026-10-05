# rove-runtime conventions

Global rules live in the root AGENTS.md; this file only covers `runtime/` specifics.

## Layers

```
src/engine/       Engine facade (facade.rs), unplanned/planned loops, StepRunner, model/tool turns, retry recovery
src/planning/     plans, revisions, evaluation, execution policy and budgets
src/state/        StateStore, trace, task_state, report, SQLite index (index.rs), repair, reconciliation, Tool Artifacts
src/tools/        built-in tools, Executor, hooks, MCP (mcp_proxy.rs, mcp/)
src/agents/       AgentDefinition, run profiles, AGENTS.md instruction discovery, procedure catalog
src/context/      ContextManager, compaction
src/memory/       session memory and persistent memory
src/foundation/   IDs, events, sessions, runtime identity, secret redaction
src/review/       read-only Review
src/workspace/    workspace identification
src/environment.rs   ExecutionEnvironment (filesystem/process ports, observation, checkpoints)
src/conversation.rs  unified conversation message domain
```

## Conventions

- This crate depends only on `rove-models`, `rove-core`, `rove-protocol`, and `rove-tools-text` — never on any app crate.
- Errors: internally use `thiserror` for typed errors with a stable `code`. Do not leak `anyhow` into the public API.
- New serializable fields always get `#[serde(default)]`, plus `skip_serializing_if` where appropriate, so old traces and snapshots still decode and unmodified records keep their exact bytes.
- Events may only be added, never have semantics changed. When `StreamEvent` changes, persistence, API/SSE, Web, and `tests/event_contract.rs` all change together.
- StateIndex schema may only change via versioned migrations in `state/index.rs` (`CURRENT_SCHEMA_VERSION`). Refuse to open databases from a newer version. File artifacts are the source of truth; SQLite is the query and replay index.
- Recovery paths must not re-dispatch completed model or tool work; for unknown in-flight side effects, append an `interrupted` record and stop.
- Tool path resolution must go through the workspace boundary check and reject traversal and symlink escapes. Touching safety or approval logic requires negative tests.
- No crate-level `#[allow(dead_code)]`. Local exceptions must state their reason in place.
- Testing: in-module `#[cfg(test)]` tests invariants; `runtime/tests/*_contract.rs` tests cross-module contracts.
