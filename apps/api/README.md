# rove-api

## Responsibility

HTTP application surface:

- Axum router, job lifecycle, SSE
- OpenAPI/Swagger
- bearer auth, CORS, rate limiting
- provider profile probes
- benchmark endpoints via `rove-bench`
- `rove-api` binary

## Non-responsibility

Does **not** own the durable Engine loop, CLI/TUI, product config document
definition (loaded via bootstrap), or browser UI.

## Local dependencies

```text
rove-models
rove-core
rove-runtime
rove-app-bootstrap
rove-bench
```

## Minimal public API example

```rust
use std::net::SocketAddr;
use std::path::PathBuf;
use rove_api::serve;

# async fn example() -> anyhow::Result<()> {
serve(
    Some(SocketAddr::from(([127, 0, 0, 1], 8787))),
    PathBuf::from("."),
)
.await
# }
```

## Focused verification

```powershell
cargo test -p rove-api
cargo test -p rove-integration-tests --test api
cargo run -p rove-api
```
