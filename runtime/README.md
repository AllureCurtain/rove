# rove-runtime

## Responsibility

Persistent Rove execution semantics:

- run/job/session IDs, `RunRequest`, resumable task state
- workspace detection and path-boundary enforcement
- planning, step runner, tool turns, hooks, `Executor`
- approval/input contracts and runtime policy adapters
- context/compaction, session/durable memory
- local built-in tools, MCP proxy
- canonical durable `StreamEvent`
- StateStore, trace/task/report artifacts, SQLite, repair, cleanup, resume
- persistent `Engine` facade

## Non-responsibility

Does **not** own first-party `AppConfig`, CLI/TUI rendering, Axum routes,
benchmark suite schemas, or product tool-registry composition.
Built-in vector RAG is not part of this crate or the default product.

## Local dependencies

```text
rove-models <- rove-core <- rove-runtime
```

## Source layout (domain folders)

```text
runtime/src/
  engine/         Engine facade + run/plan/tool/model turn loops
    facade.rs
    run_loop.rs
    plan_loop.rs
    step_runner.rs
    tool_turn.rs
    model_turn.rs
  planning/       ExecutionPolicy, planner, plan evaluator
    execution.rs
    planner.rs
    plan_evaluator.rs
  tools/          built-in tools, Executor, hooks, tool input
    executor.rs
    tool_input.rs
    hooks/
    fs.rs search.rs shell.rs memory.rs request_input.rs mcp_proxy.rs …
  state/          StateStore, trace, artifacts, resume, SQLite
  memory/         durable + session memory services
  context/        ContextManager, compaction, prompt metadata
    manager.rs
    compaction.rs
    prompt_metadata.rs
  workspace/      Workspace detection + path boundary
    root.rs
    boundary.rs
  foundation/     types, StreamEvent, session, runtime identity
    types.rs
    events.rs
    session.rs
    runtime_identity.rs
  lib.rs          domain modules + stable public path aliases
```

Historic flat public paths remain available as crate-root aliases so apps keep
importing `rove_runtime::{engine, execution, executor, hooks, types, events,
context, compaction, workspace, boundary, …}` without a mass rewrite.

## Minimal public API example

```rust
use rove_runtime::{RunId, Workspace};

# fn example() -> anyhow::Result<()> {
let workspace = Workspace::detect(std::path::Path::new("."))?;
let run_id = RunId::new();
println!("{run_id} in {}", workspace.root.display());
# Ok(())
# }
```

## Focused verification

```powershell
cargo test -p rove-runtime
```

Compatibility status: pre-1.0. Apps and integration tests consume this crate
directly after the modular workspace migration.
