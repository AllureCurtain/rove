# rove-models

## Responsibility

Provider-neutral model protocol and adapters:

- `Message`, `Role`, `ToolCallRef`, `Usage`
- model-visible `ModelToolSchema` (name/description/input only)
- `ModelClient`, `ModelEvent`, `ModelError`
- OpenAI Completions, OpenAI Responses, Anthropic, Ollama, and Fake protocols via `provider/*`
- provider stream parsing, routing, and health primitives

## Non-responsibility

Does **not** own Agent execution, workspaces, tools, persistence, product
`AppConfig`, CLI/API types, or provider selection from first-party config.

## Local dependencies

```text
(none)
```

## Minimal public API example

```rust
use futures::StreamExt;
use rove_models::{FakeModelClient, Message, ModelClient};

# async fn example() {
let client = FakeModelClient::new("hello".to_string());
let events = client
    .stream(&[Message::user("hi")], &[])
    .collect::<Vec<_>>()
    .await;
assert!(!events.is_empty());
# }
```

## Focused verification

```powershell
cargo test -p rove-models
```

Compatibility status: pre-1.0. Serialized protocol names and provider payload
behavior are covered by package and integration tests.
