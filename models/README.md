# rove-models

`rove-models` owns Rove's provider-neutral message protocol, model-visible tool
schema, usage/error normalization, model client contract, provider adapters,
deterministic fake provider, routing, and provider health primitives.

It does not own Agent execution, workspace policy, tools, persistence,
configuration loading, CLI/API types, or product assembly. First-party
provider selection from `AppConfig` remains above this crate.

Local project dependencies: none.

```rust
use futures::StreamExt;
use rove_models::{FakeModelClient, Message, ModelClient};

# async fn example() {
let client = FakeModelClient::new("hello".to_string());
let events = client.stream(&[Message::user("hi")], &[]).collect::<Vec<_>>().await;
assert!(!events.is_empty());
# }
```

Focused verification:

```powershell
cargo test -p rove-models
```

Compatibility status: pre-1.0 and extracted behind the temporary root `rove`
facade. Serialized protocol names and provider payload behavior remain covered
by existing tests.
