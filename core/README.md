# rove-core

`rove-core` is Rove's embeddable, in-memory Agent harness. It owns the model
and tool-call loop, action parsing, cancellation/control, core events, tool
contracts, registry, operational descriptors, and runtime-neutral tool policy
hooks.

It does not own workspaces, approval decisions, built-in tools, memory,
planning, persistence, resume, HTTP, terminal UI, or product configuration.

Local project dependencies: `rove-models` only.

```rust
use futures::StreamExt;
use rove_core::{Agent, AgentConfig, ToolRegistry};
use rove_models::FakeModelClient;

async fn run() {
let agent = Agent::new(
    Box::new(FakeModelClient::new("hello".to_string())),
    ToolRegistry::new(),
    AgentConfig::default(),
);
let events = agent.ask("say hello").collect::<Vec<_>>().await;
assert!(!events.is_empty());
}
```

Focused verification:

```powershell
cargo test -p rove-core
```

Compatibility status: pre-1.0. The root `rove` package temporarily re-exports
the extracted contracts while persistent execution moves into `rove-runtime`.
