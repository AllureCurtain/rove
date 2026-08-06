# rove-cli

## Responsibility

User-facing terminal product:

- CLI arguments, one-shot/exec, REPL
- sessions/state maintenance commands
- terminal rendering and full-screen TUI
- default-run `rove` binary
- explicit `--trust-project` activation for workspace config and MCP

## Non-responsibility

Does **not** own Axum/SSE, durable runtime internals, first-party config schema
definition (uses `rove-app-bootstrap`), or built-in vector/RAG indexing.

## Local dependencies

```text
rove-models
rove-core
rove-runtime
rove-app-bootstrap
```

## Minimal public API example

```rust
use rove_cli::tool_registry;
use rove_runtime::Workspace;

# fn example() -> anyhow::Result<()> {
let workspace = Workspace::detect(std::path::Path::new("."))?;
let registry = tool_registry(&workspace);
assert!(!registry.descriptors().is_empty());
# Ok(())
# }
```

## Focused verification

```powershell
cargo test -p rove-cli
cargo run -p rove-cli -- --help
cargo run -p rove-cli -- --model fake "echo hello"
cargo run -p rove-cli -- --trust-project --model fake "use project tools"
cargo test -p rove-integration-tests --test cli_repl
```
