# rove-integration-tests

Cross-package contracts that do not belong to a single product crate.

This package is a workspace member and uses the real package APIs
(`rove-runtime`, `rove-app-bootstrap`, `rove-cli`, `rove-api`, `rove-bench`).

Focused verification:

```powershell
cargo test -p rove-integration-tests
```
