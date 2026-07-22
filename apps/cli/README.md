# rove-cli

## Responsibility

User-facing terminal product:

- CLI arguments, one-shot/exec, REPL
- sessions/state maintenance commands
- terminal rendering and full-screen TUI
- feature-gated `rove-index` and RAG implementation
- default-run `rove` binary

## Non-responsibility

Does **not** own Axum/SSE, durable runtime internals, or first-party config
schema definition (uses `rove-app-bootstrap`). Default builds keep disabled RAG
stubs from bootstrap; real RAG requires `--features rag`.

## Local dependencies

```text
rove-models
rove-core
rove-runtime
rove-app-bootstrap
```

## Minimal public API example

```rust
use rove_cli::default_tool_registry;
use rove_runtime::Workspace;

# fn example() -> anyhow::Result<()> {
let workspace = Workspace::detect(std::path::Path::new("."))?;
let registry = default_tool_registry(&workspace);
assert!(!registry.schemas().is_empty());
# Ok(())
# }
```

## Focused verification

```powershell
cargo test -p rove-cli
cargo run -p rove-cli -- --help
cargo run -p rove-cli -- --model fake "echo hello"
cargo test -p rove-integration-tests --test cli_repl
```
