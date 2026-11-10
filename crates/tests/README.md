# rove-integration-tests

## Responsibility

Cross-package contracts that do not belong to a single product crate:

- event/artifact compatibility
- CLI/API/E2E behavioral contracts
- workspace architecture dependency direction
- packaging hygiene scanners

## Non-responsibility

Does **not** ship a user-facing binary and is not a runtime dependency of apps.

## Local dependencies

```text
rove-models
rove-core
rove-runtime
rove-app-bootstrap
rove-cli
rove-api
rove-bench
```

## Focused verification

```powershell
cargo test -p rove-integration-tests                 # all three binaries
cargo test -p rove-integration-tests --test it e2e:: # one module of the shared binary
cargo test -p rove-integration-tests --test api      # API contracts
```

Most suites are modules of the shared `it` binary (`it.rs`); add new files
there rather than as new `[[test]]` targets. Only suites that need exclusive
process-global state get their own binary (see `Cargo.toml`).

When tests need the `rove` binary, `support::rove_bin` builds it once and
locates it next to the running test executable, so a custom
`CARGO_TARGET_DIR` works.
