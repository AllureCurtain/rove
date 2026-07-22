# rove-runtime

`rove-runtime` owns Rove's persistent and product-level execution contracts.
During the modular Workspace migration it is extracted in verified slices from
the temporary root `rove` compatibility package.

The current crate owns:

- run, job, and session IDs;
- resumable task and prompt-checkpoint types;
- execution policy and plan-ledger data contracts;
- workspace detection and path-boundary enforcement;
- prompt metadata and runtime identity evaluation;
- approval and request-input provider contracts;
- canonical durable `StreamEvent`;
- token-aware context construction and deterministic/model compaction;
- session/durable memory paths, storage, recall, and prompt assembly;
- local built-in tools, invocation adapters, and the existing MCP proxy;
- the tool `Executor` pipeline and pre/post-tool plus post-run hooks;
- planner, plan evaluator, plan loop, step runner, unplanned run loop, and
  tool-turn coordination;
- durable `AgentEvent -> StreamEvent` translation and the persistent `Engine`
  facade;
- StateStore, trace, task/report artifacts, SQLite index, repair, cleanup, and
  resume.

Product tool-registry assembly, optional RAG, and first-party `AppConfig`
remain in the root compatibility package until Phase 6 extracts apps and
bootstrap. This crate must not depend on CLI, API, benchmark, or Web packages.

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

Compatibility status: pre-1.0 and transitional. The root `rove::core::*`,
`rove::state::*`, `rove::memory::*`, and `rove::hooks::*` module paths re-export
these contracts until application crates and tests have migrated to the new
package.
