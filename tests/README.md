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
cargo test -p rove-integration-tests
```

When tests need the `rove` binary and `CARGO_BIN_EXE_rove` is unavailable
(workspace integration package), they resolve/build `target/debug/rove` from
the workspace root.
