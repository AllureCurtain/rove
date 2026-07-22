# rove-runtime

`rove-runtime` owns Rove's persistent and product-level execution contracts.
During the modular Workspace migration it is being extracted in verified
slices from the temporary root `rove` compatibility package.

The current extracted slice contains:

- run, job, and session IDs;
- resumable task and prompt-checkpoint types;
- execution policy and plan-ledger data contracts;
- workspace detection and path-boundary enforcement;
- prompt metadata and runtime identity evaluation;
- approval and request-input provider contracts.

Persistent coordination, state storage, memory, built-in tools, MCP/RAG, and
the durable event loop still live in the root package until their later Phase 5
slices are moved. This crate must not depend on CLI, API, benchmark, or Web
packages.

Local project dependencies:

```text
rove-models <- rove-core <- rove-runtime
```

Minimal foundation usage:

```rust
use rove_runtime::{RunId, Workspace};

let workspace = Workspace::detect(std::path::Path::new("."))?;
let run_id = RunId::new();
println!("{run_id} in {}", workspace.root.display());
# Ok::<(), anyhow::Error>(())
```

Focused verification:

```powershell
cargo test -p rove-runtime
```

Compatibility status: pre-1.0 and transitional. The root `rove::core::*`
module paths re-export these contracts until application crates and tests have
migrated to the new package.
