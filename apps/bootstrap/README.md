# rove-app-bootstrap

First-party product assembly shared by CLI and API:

- `.rove/config.toml`, environment, and explicit override loading
- complete first-party config document and source summary
- provider selection/construction from model-layer clients
- conversion helpers such as shell policy and memory path resolution

Local dependencies:

```text
rove-models
rove-runtime
```

Focused verification:

```powershell
cargo test -p rove-app-bootstrap
```
