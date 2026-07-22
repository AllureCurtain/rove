# rove-app-bootstrap

## Responsibility

First-party product assembly shared by CLI and API:

- `.rove/config.toml`, environment, and override loading
- complete first-party config document and source summary
- provider construction from model-layer clients
- non-RAG product tool registry and disabled RAG stubs
- shared product `Engine` assembly helpers

## Non-responsibility

Does **not** own Axum routes, clap CLI parsing, TUI rendering, benchmark
schemas, durable runtime internals, or the heavy RAG feature implementation.

## Local dependencies

```text
rove-models
rove-core
rove-runtime
```

## Minimal public API example

```rust
use rove_app_bootstrap::{AppConfig, AppConfigOverrides, build_model_client};

# fn example() -> anyhow::Result<()> {
let config = AppConfig::load(".", AppConfigOverrides::default())?;
let model = build_model_client(&config, config.provider.model.clone());
let _ = model;
# Ok(())
# }
```

## Focused verification

```powershell
cargo test -p rove-app-bootstrap
```
