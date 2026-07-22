# rove-cli

CLI arguments, one-shot/exec, REPL, sessions/state commands, terminal rendering,
full-screen TUI, and the feature-gated `rove-index` binary.

Local dependencies:

```text
rove-models
rove-core
rove-runtime
rove-app-bootstrap
```

Focused verification:

```powershell
cargo test -p rove-cli
cargo run -p rove-cli -- --help
```
