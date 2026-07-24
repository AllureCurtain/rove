# rove-core

## Responsibility

Embeddable, in-memory Agent harness:

- `Agent` loop and model/tool turn mechanics
- `Action`, tool-call correlation, stop outcome
- `Tool`, `ToolOutput`, `ToolRegistry`, operational `ToolDescriptor`
- runtime-neutral before/after tool policy hooks
- cancellation, steering, follow-up control
- `AgentEvent` and core budget accounting

## Non-responsibility

Does **not** own workspaces, durable state, approval providers, built-in tools,
memory files, planning ledgers, HTTP, terminal UI, or product configuration.

## Local dependencies

```text
rove-models
```

## Minimal public API example

```rust
use futures::StreamExt;
use rove_core::{Agent, AgentConfig, ToolRegistry};
use rove_models::FakeModelClient;

# async fn example() {
let agent = Agent::new(
    Box::new(FakeModelClient::new("hello".to_string())),
    ToolRegistry::new(),
    AgentConfig::default(),
);
let mut stream = agent.ask("say hello");
while let Some(_event) = stream.next().await {}
# }
```

## Focused verification

```powershell
cargo test -p rove-core
cargo test -p rove-integration-tests --test embedding_contract
```

Compatibility status: pre-1.0. The Fake Model + custom Tool embedding test
creates no `.rove/` state.
