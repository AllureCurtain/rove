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
benchmark suite schemas, or product tool-registry composition. Optional heavy
Built-in vector RAG is not part of this crate or the default product.

## Local dependencies

```text
rove-models <- rove-core <- rove-runtime
```

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
