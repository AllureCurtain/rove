# rove-bench

## Responsibility

Deterministic no-network benchmark surface:

- suite/task/check/report schemas
- process-evidence checks
- suite generation and runner
- `rove-bench` binary

## Non-responsibility

Does **not** own HTTP routes (API depends on this library, not the reverse),
CLI/TUI UI, product config loading as a required path, or provider networking.

## Local dependencies

```text
rove-models
rove-runtime
rove-app-bootstrap
```

## Minimal public API example

```rust
use rove_bench::{available_suites, resolve_suite};

# fn example() -> std::io::Result<()> {
let suites = available_suites();
assert!(!suites.is_empty());
let suite = resolve_suite("agent-smoke", "default")?;
assert!(!suite.tasks.is_empty());
# Ok(())
# }
```

## Focused verification

```powershell
cargo test -p rove-bench
cargo run -p rove-bench -- --list
cargo test -p rove-integration-tests --test bench
```
