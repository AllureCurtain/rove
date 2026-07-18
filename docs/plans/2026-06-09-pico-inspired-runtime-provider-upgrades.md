# Pico-Inspired Runtime And Provider Upgrades Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make rove a production-grade pico second-development runtime by adding an OpenAI Responses provider path, explicit ReAct runtime documentation, runtime identity checkpoints, prompt build metadata, tool execution metadata, and evidence-oriented benchmark/report packaging.

**Architecture:** Keep rove's existing Rust split of `Engine -> PlanLoop/RunLoop -> ModelTurn -> ToolTurn`; do not collapse it into pico's single Python loop. Add pico-inspired contracts as first-class metadata around the existing core, and add OpenAI Responses as a separate provider adapter alongside the current OpenAI-compatible chat adapter.

**Tech Stack:** Rust, reqwest SSE streaming, serde/serde_json, async-stream, tokio, axum API profiles, existing rove model/tool/state modules, PowerShell provider integration runner.

---

## Source Basis

This plan is based on the current rove source tree and the updated sibling pico project at `D:\Study\project\agent\pico`.

Official OpenAI references to re-check during implementation:

- `https://platform.openai.com/docs/guides/migrate-to-responses`
- `https://platform.openai.com/docs/api-reference/responses/create`
- `https://platform.openai.com/docs/guides/function-calling`

Important local files already reviewed:

- `src/core/engine.rs`
- `src/core/run_loop.rs`
- `src/core/plan_loop.rs`
- `src/core/model_turn.rs`
- `src/core/tool_turn.rs`
- `src/models/traits.rs`
- `src/models/openai.rs`
- `src/models/anthropic.rs`
- `src/models/factory.rs`
- `src/models/routing.rs`
- `src/core/types.rs`
- `src/state/artifacts.rs`
- `src/state/report.rs`
- `tests/provider_smoke.rs`
- `tests/api.rs`
- `scripts/provider-integration.ps1`
- `..\pico\pico\agent_loop.py`
- `..\pico\pico\providers\clients.py`
- `..\pico\pico\checkpoint.py`
- `..\pico\pico\prompt_prefix.py`
- `..\pico\pico\tool_executor.py`

## Design Decisions

1. `openai-compatible` continues to mean OpenAI-style `/chat/completions`.
2. Add `openai-responses` as a new provider name for `/responses`.
3. Do not send Responses-only fields through the chat adapter.
4. Treat prompt-cache fields as Responses-provider metadata. Include `prompt_cache_key` only when `provider.responses_prompt_cache` is true.
5. Keep native OpenAI and Anthropic tool-use history in `Message.tool_calls` and `Message.tool_call_id`.
6. Add new serialized fields with `#[serde(default)]` so older `.rove/task_state.json` and `report.json` artifacts remain readable.
7. ReAct clarity is a documentation and small facade problem; it is not a reason to rewrite rove into pico's single-loop shape.

## File Structure

Create:

- `src/models/openai_responses.rs`
  OpenAI Responses API adapter. Builds `/responses` request bodies, normalizes SSE events into `ModelEvent`, formats `function_call` and `function_call_output` history, maps usage and HTTP errors.

- `src/core/runtime_identity.rs`
  Runtime execution contract helpers: workspace fingerprint, tool signature, prompt hash, and mismatch evaluation.

- `src/core/prompt_metadata.rs`
  Prompt build metadata types and helpers: stable prefix hash, workspace fingerprint, tool signature, token estimate, prompt cache key.

- `docs/runtime/react-loop.md`
  Human-readable explanation of rove's Plan + ReAct runtime.

- `docs/runtime/openai-responses-provider.md`
  Provider setup, behavior, differences from chat completions, and smoke commands.

- `benchmarks/results/README.md` if the directory does not already exist.
  Documents the evidence package format expected from benchmark runs.

Modify:

- `src/models/mod.rs`
  Export `openai_responses`.

- `src/models/factory.rs`
  Add `ProviderKind::OpenAiResponses`; build `OpenAiResponsesClient`.

- `src/config.rs`
  Accept `openai-responses` provider name; add optional provider fields for Responses cache behavior.

- `src/models/traits.rs`
  Extend `Usage` or add provider metadata only if needed by Responses usage/cache accounting. Prefer extending `Usage` in `src/core/types.rs` with serde defaults.

- `src/core/types.rs`
  Add `RuntimeIdentity`, `PromptBuildMetadata`, `ToolExecutionMetadata`, and metadata fields on `PromptCheckpoint`, `ToolResult`, and `TaskState` as needed.

- `src/core/context.rs`
  Return prompt metadata together with existing `ContextBuild`.

- `src/core/engine.rs`
  Construct runtime identity at run start and pass it to artifact recorder.

- `src/state/artifacts.rs`
  Persist runtime identity, prompt checkpoint metadata, prompt build metadata, and tool metadata.

- `src/state/report.rs`
  Include non-secret provider/runtime/tool evidence in `RunReport`.

- `src/core/executor.rs` and `src/core/tool_turn.rs`
  Standardize tool execution metadata for success and failure.

- `tests/provider_smoke.rs`
  Add opt-in real-provider smoke for `openai-responses`.

- `tests/api.rs`
  Add provider-profile coverage for `openai-responses`.

- `tests/code_hygiene.rs`
  Add doc/source-of-truth assertions for new runtime/provider docs and integration runner support.

- `scripts/provider-integration.ps1`
  Support `openai-responses` inventory, smoke dispatch, API/Web provider profiles, and evidence classification.

- `.env.integration.example`
  Add `ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES` and generic runner example values.

- `docs/runtime/provider-smoke.md`
- `docs/runtime/release-readiness.md`
- `README.md`

---

### Task 1: Add Provider Name And Config Surface

**Files:**
- Modify: `src/config.rs`
- Modify: `src/models/factory.rs`
- Modify: `src/models/mod.rs`
- Test: existing unit tests in `src/config.rs`

- [ ] **Step 1: Write failing config tests**

Add these tests inside `src/config.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn validation_accepts_openai_responses_provider_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join(".rove");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        r#"
[provider]
name = "openai-responses"
model = "gpt-4.1-mini"
api_base = "https://api.openai.com/v1"
"#,
    )
    .unwrap();

    let config = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap();

    assert_eq!(config.provider.name, "openai-responses");
}

#[test]
fn validation_accepts_openai_responses_fallback_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join(".rove");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        r#"
[provider]
name = "openai-compatible"
model = "primary-model"
fallback_providers = [
  { name = "openai-responses", api_base = "https://api.openai.com/v1", api_key = "secret", model = "gpt-4.1-mini" }
]
"#,
    )
    .unwrap();

    let config = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap();

    assert_eq!(config.provider.fallback_providers[0].name, "openai-responses");
}
```

- [ ] **Step 2: Run config tests and verify failure**

Run:

```powershell
cargo test config::tests::validation_accepts_openai_responses_provider_name config::tests::validation_accepts_openai_responses_fallback_provider
```

Expected: fails because `canonical_provider_name` does not accept `openai-responses`.

- [ ] **Step 3: Add provider config fields**

In `src/config.rs`, extend `ProviderConfig`:

```rust
pub responses_prompt_cache: bool,
pub responses_prompt_cache_retention: Option<String>,
```

Set defaults in `impl Default for ProviderConfig`:

```rust
responses_prompt_cache: false,
responses_prompt_cache_retention: None,
```

Extend `ProviderConfigLayer` with:

```rust
responses_prompt_cache: Option<bool>,
responses_prompt_cache_retention: Option<Option<String>>,
```

Extend `env_layer()`:

```rust
if let Some(value) = env_string("ROVE_OPENAI_RESPONSES_PROMPT_CACHE") {
    provider.responses_prompt_cache =
        Some(parse_env_bool("ROVE_OPENAI_RESPONSES_PROMPT_CACHE", &value)?);
    keys.push("ROVE_OPENAI_RESPONSES_PROMPT_CACHE".to_string());
}
if let Some(value) = env_string("ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION") {
    provider.responses_prompt_cache_retention = Some(Some(value));
    keys.push("ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION".to_string());
}
```

Extend `has_provider_values()`:

```rust
|| layer.responses_prompt_cache.is_some()
|| layer.responses_prompt_cache_retention.is_some()
```

- [ ] **Step 4: Accept `openai-responses` as a canonical provider**

Update `canonical_provider_name`:

```rust
fn canonical_provider_name(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-compatible" => Some("openai-compatible"),
        "openai-responses" | "responses" => Some("openai-responses"),
        "anthropic" => Some("anthropic"),
        "ollama" => Some("ollama"),
        "fake" => Some("fake"),
        _ => None,
    }
}
```

Update validation error strings to include `openai-responses`.

- [ ] **Step 5: Wire provider kind in factory**

In `src/models/mod.rs` add:

```rust
pub mod openai_responses;
```

In `src/models/factory.rs`, import the new client:

```rust
use crate::models::openai_responses::OpenAiResponsesClient;
```

Extend `ProviderKind`:

```rust
OpenAiResponses,
```

Update `ProviderKind::from_name`:

```rust
"openai-responses" | "responses" => Self::OpenAiResponses,
```

Update `ProviderSpec::primary`:

```rust
ProviderKind::OpenAiResponses => Self::openai_responses(config, model),
```

Add:

```rust
fn openai_responses(config: &AppConfig, model: String) -> Self {
    Self {
        kind: ProviderKind::OpenAiResponses,
        api_base: config.provider.api_base.clone(),
        api_key: config.provider.api_key.clone(),
        model,
    }
}
```

Update `build_provider_client`:

```rust
ProviderKind::OpenAiResponses => Box::new(OpenAiResponsesClient::new(
    spec.api_base,
    spec.api_key,
    spec.model,
)),
```

- [ ] **Step 6: Run config tests**

Run:

```powershell
cargo test config::tests::validation_accepts_openai_responses_provider_name config::tests::validation_accepts_openai_responses_fallback_provider
```

Expected: both tests pass after the client stub exists.

- [ ] **Step 7: Commit**

```powershell
git add src/config.rs src/models/factory.rs src/models/mod.rs
git commit -m "Add OpenAI Responses provider configuration"
```

---

### Task 2: Implement OpenAI Responses Adapter

**Files:**
- Create: `src/models/openai_responses.rs`
- Modify: `src/models/factory.rs`
- Test: unit tests inside `src/models/openai_responses.rs`

- [ ] **Step 1: Create failing tests for request body and event normalization**

Create `src/models/openai_responses.rs` with tests first. The tests should define these expected behaviors:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Message, ToolCallRef, ToolSchema};

    #[test]
    fn request_body_uses_responses_input_items_and_function_tools() {
        let client = OpenAiResponsesClient::new(
            "https://api.openai.com/v1".to_string(),
            "secret".to_string(),
            "gpt-4.1-mini".to_string(),
        );

        let body = client.build_request_body(
            &[Message::user("inspect")],
            &[ToolSchema {
                name: "fs_read".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
                destructive: false,
                parallel_safe: true,
                capability: None,
            }],
        );

        assert_eq!(body["model"], "gpt-4.1-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "fs_read");
    }

    #[test]
    fn request_body_formats_function_call_history() {
        let client = OpenAiResponsesClient::new(
            "https://api.openai.com/v1".to_string(),
            "secret".to_string(),
            "gpt-4.1-mini".to_string(),
        );

        let body = client.build_request_body(
            &[
                Message::assistant_with_tool_calls(
                    String::new(),
                    vec![ToolCallRef {
                        id: "call_1".to_string(),
                        name: "fs_read".to_string(),
                        args: serde_json::json!({ "path": "Cargo.toml" }),
                    }],
                ),
                Message::tool("file contents", Some("call_1".to_string())),
            ],
            &[],
        );

        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][0]["call_id"], "call_1");
        assert_eq!(body["input"][0]["name"], "fs_read");
        assert_eq!(body["input"][1]["type"], "function_call_output");
        assert_eq!(body["input"][1]["call_id"], "call_1");
        assert_eq!(body["input"][1]["output"], "file contents");
    }

    #[test]
    fn responses_stream_text_delta_is_normalized() {
        let mut state = ResponsesStreamState::default();
        let event = serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "hello"
        })
        .to_string();

        let events = normalize_responses_event(&mut state, &event).unwrap();

        assert_eq!(
            events,
            vec![ModelEvent::TextDelta {
                text: "hello".to_string()
            }]
        );
    }

    #[test]
    fn responses_function_call_item_is_normalized_to_tool_use() {
        let mut state = ResponsesStreamState::default();
        let item_done = serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "fs_read",
                "arguments": "{\"path\":\"Cargo.toml\"}"
            }
        })
        .to_string();

        let events = normalize_responses_event(&mut state, &item_done).unwrap();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseDone { id, name, args }
                    if id == "call_1" && name == "fs_read" && args["path"] == "Cargo.toml"
            )
        }));
    }

    #[test]
    fn responses_completed_usage_is_normalized() {
        let mut state = ResponsesStreamState::default();
        let completed = serde_json::json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15,
                    "input_tokens_details": { "cached_tokens": 4 }
                }
            }
        })
        .to_string();

        let events = normalize_responses_event(&mut state, &completed).unwrap();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::Usage { usage }
                    if usage.prompt_tokens == 10
                        && usage.completion_tokens == 5
                        && usage.total_tokens == 15
                        && usage.cached_tokens == 4
            )
        }));
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }
}
```

- [ ] **Step 2: Extend usage type for cached tokens**

Modify `src/core/types.rs`:

```rust
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
}
```

Update existing OpenAI and Anthropic usage construction to set `cached_tokens: 0`, or use struct update syntax:

```rust
Usage {
    prompt_tokens,
    completion_tokens,
    total_tokens,
    cached_tokens,
}
```

- [ ] **Step 3: Implement client skeleton**

At the top of `src/models/openai_responses.rs`:

```rust
use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{StatusCode, header::HeaderMap};
use std::collections::BTreeMap;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelClientId, ModelEvent};

pub struct OpenAiResponsesClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    prompt_cache_enabled: bool,
    prompt_cache_retention: Option<String>,
}

impl OpenAiResponsesClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
            prompt_cache_enabled: false,
            prompt_cache_retention: None,
        }
    }

    pub fn with_prompt_cache(mut self, enabled: bool, retention: Option<String>) -> Self {
        self.prompt_cache_enabled = enabled;
        self.prompt_cache_retention = retention;
        self
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let (instructions, input) = format_responses_input(messages);
        let tool_defs = tools.iter().map(format_responses_tool).collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
            "stream": true,
            "store": false,
            "parallel_tool_calls": true,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();

        if let Some(instructions) = instructions {
            body.insert("instructions".to_string(), serde_json::Value::String(instructions));
        }
        if !tool_defs.is_empty() {
            body.insert("tools".to_string(), serde_json::Value::Array(tool_defs));
        }
        if self.prompt_cache_enabled {
            let key = prompt_cache_key(messages, tools);
            body.insert("prompt_cache_key".to_string(), serde_json::Value::String(key));
            if let Some(retention) = &self.prompt_cache_retention {
                body.insert(
                    "prompt_cache_retention".to_string(),
                    serde_json::Value::String(retention.clone()),
                );
            }
        }

        serde_json::Value::Object(body)
    }
}
```

- [ ] **Step 4: Implement input formatting**

Add:

```rust
fn format_responses_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions = None;
    let mut input = Vec::new();

    for message in messages {
        match message.role {
            Role::System if instructions.is_none() => {
                instructions = Some(message.content.clone());
            }
            Role::System => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": message.content }]
                }));
            }
            Role::User => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": message.content }]
                }));
            }
            Role::Assistant if !message.tool_calls.is_empty() => {
                if !message.content.is_empty() {
                    input.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": message.content }]
                    }));
                }
                for tool_call in &message.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.args.to_string()
                    }));
                }
            }
            Role::Assistant => {
                input.push(serde_json::json!({
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": message.content }]
                }));
            }
            Role::Tool if message.tool_call_id.is_some() => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": message.tool_call_id.as_deref().unwrap_or_default(),
                    "output": message.content
                }));
            }
            Role::Tool => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": message.content }]
                }));
            }
        }
    }

    (instructions, input)
}

fn format_responses_tool(tool: &ToolSchema) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "strict": false
    })
}
```

- [ ] **Step 5: Implement event normalization**

Add:

```rust
#[derive(Debug, Default)]
struct ResponsesStreamState {
    function_calls: BTreeMap<String, ResponsesFunctionCall>,
}

#[derive(Debug, Default)]
struct ResponsesFunctionCall {
    call_id: String,
    name: String,
    arguments: String,
    done: bool,
}

fn normalize_responses_event(
    state: &mut ResponsesStreamState,
    data: &str,
) -> serde_json::Result<Vec<ModelEvent>> {
    if data.trim() == "[DONE]" {
        return Ok(vec![ModelEvent::Done]);
    }

    let json = serde_json::from_str::<serde_json::Value>(data)?;
    let event_type = json.get("type").and_then(|value| value.as_str());
    let mut events = Vec::new();

    match event_type {
        Some("response.output_text.delta") => {
            if let Some(delta) = json.get("delta").and_then(|value| value.as_str())
                && !delta.is_empty()
            {
                events.push(ModelEvent::TextDelta {
                    text: delta.to_string(),
                });
            }
        }
        Some("response.output_item.added") => {
            if let Some(item) = json.get("item") {
                capture_function_call_start(state, item, &mut events);
            }
        }
        Some("response.function_call_arguments.delta") => {
            let item_id = json
                .get("item_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let delta = json
                .get("delta")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !item_id.is_empty()
                && !delta.is_empty()
                && let Some(call) = state.function_calls.get_mut(&item_id)
            {
                call.arguments.push_str(delta);
                events.push(ModelEvent::ToolUseDelta {
                    id: call.call_id.clone(),
                    args_delta: delta.to_string(),
                });
            }
        }
        Some("response.function_call_arguments.done") => {
            let item_id = json
                .get("item_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if let Some(call) = state.function_calls.get_mut(&item_id) {
                call.done = true;
                let args = parse_arguments(&call.arguments);
                events.push(ModelEvent::ToolUseDone {
                    id: call.call_id.clone(),
                    name: call.name.clone(),
                    args,
                });
            }
        }
        Some("response.output_item.done") => {
            if let Some(item) = json.get("item") {
                capture_function_call_done(state, item, &mut events);
            }
        }
        Some("response.completed") => {
            if let Some(usage) = json
                .get("response")
                .and_then(|response| response.get("usage"))
            {
                events.push(ModelEvent::Usage {
                    usage: parse_responses_usage(usage),
                });
            }
            events.push(ModelEvent::Done);
        }
        Some("response.failed") | Some("response.incomplete") => {
            events.push(ModelEvent::Done);
        }
        _ => {}
    }

    Ok(events)
}

fn capture_function_call_start(
    state: &mut ResponsesStreamState,
    item: &serde_json::Value,
    events: &mut Vec<ModelEvent>,
) {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return;
    }
    let item_id = item
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let call_id = item
        .get("call_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&item_id)
        .to_string();
    let name = item
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool")
        .to_string();
    if item_id.is_empty() {
        return;
    }
    state.function_calls.insert(
        item_id,
        ResponsesFunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: item
                .get("arguments")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            done: false,
        },
    );
    events.push(ModelEvent::ToolUseStart { id: call_id, name });
}

fn capture_function_call_done(
    state: &mut ResponsesStreamState,
    item: &serde_json::Value,
    events: &mut Vec<ModelEvent>,
) {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return;
    }
    let item_id = item
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let call_id = item
        .get("call_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&item_id)
        .to_string();
    let name = item
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool")
        .to_string();
    let arguments = item
        .get("arguments")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    if let Some(call) = state.function_calls.get_mut(&item_id)
        && call.done
    {
        return;
    }

    events.push(ModelEvent::ToolUseDone {
        id: call_id,
        name,
        args: parse_arguments(&arguments),
    });
}

fn parse_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()))
}

fn parse_responses_usage(usage: &serde_json::Value) -> Usage {
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(prompt_tokens as u64 + completion_tokens as u64) as u32;
    let cached_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;

    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
    }
}
```

- [ ] **Step 6: Implement streaming HTTP client**

Add:

```rust
#[async_trait]
impl ModelClient for OpenAiResponsesClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let body = self.build_request_body(messages, tools);
        let url = format!("{}/responses", self.api_base.trim_end_matches('/'));
        let api_key = self.api_key.clone();

        Box::pin(async_stream::stream! {
            let response = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|err| ModelError::RequestFailed(err.to_string()))?;

            if !response.status().is_success() {
                let status = response.status();
                let headers = response.headers().clone();
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_responses_http_error(status, &headers, &text));
                return;
            }

            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut state = ResponsesStreamState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        yield Err(ModelError::StreamInterrupted(err.to_string()));
                        return;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(events) = normalize_responses_event(&mut state, data)
                    {
                        for event in events {
                            let done = matches!(event, ModelEvent::Done);
                            yield Ok(event);
                            if done {
                                return;
                            }
                        }
                    }
                }
            }
        })
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("openai-responses", &self.api_base, &self.model)
    }
}
```

Add HTTP classification by adapting `src/models/openai.rs` behavior:

```rust
fn classify_responses_http_error(
    status: StatusCode,
    _headers: &HeaderMap,
    body: &str,
) -> ModelError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelError::AuthFailed;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelError::RateLimited {
            retry_after_ms: 1000,
        };
    }
    if body.to_ascii_lowercase().contains("context")
        && body.to_ascii_lowercase().contains("token")
    {
        return ModelError::ContextLengthExceeded { used: 0, max: 0 };
    }
    ModelError::RequestFailed(format!("HTTP {}: {}", status, body))
}
```

- [ ] **Step 7: Wire prompt cache config into factory**

In `build_provider_client`, for `ProviderKind::OpenAiResponses`, pass cache fields from the provider spec. Extend `ProviderSpec`:

```rust
responses_prompt_cache: bool,
responses_prompt_cache_retention: Option<String>,
```

Set these fields in every `ProviderSpec` constructor. For non-Responses providers, set `false` and `None`.

Build:

```rust
ProviderKind::OpenAiResponses => Box::new(
    OpenAiResponsesClient::new(spec.api_base, spec.api_key, spec.model)
        .with_prompt_cache(spec.responses_prompt_cache, spec.responses_prompt_cache_retention),
),
```

- [ ] **Step 8: Run model adapter tests**

Run:

```powershell
cargo test openai_responses
```

Expected: all new adapter tests pass.

- [ ] **Step 9: Run existing model tests**

Run:

```powershell
cargo test models
```

Expected: existing OpenAI chat, Anthropic, Ollama, routing, and new Responses tests pass.

- [ ] **Step 10: Commit**

```powershell
git add src/models/openai_responses.rs src/models/factory.rs src/models/mod.rs src/core/types.rs
git commit -m "Add OpenAI Responses model adapter"
```

---

### Task 3: Add Provider Profiles, Smoke Gates, And Integration Runner Support

**Files:**
- Modify: `tests/provider_smoke.rs`
- Modify: `tests/api.rs`
- Modify: `scripts/provider-integration.ps1`
- Modify: `.env.integration.example`
- Modify: `docs/runtime/provider-smoke.md`
- Modify: `docs/runtime/release-readiness.md`
- Test: `tests/provider_smoke.rs`, `tests/api.rs`, `tests/code_hygiene.rs`

- [ ] **Step 1: Add provider smoke test**

In `tests/provider_smoke.rs`, add:

```rust
#[tokio::test]
async fn openai_responses_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES") {
        return;
    }
    require_env("OPENAI_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL")
        .unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    assert_provider_smoke("openai-responses", model).await;
}
```

- [ ] **Step 2: Add API profile test server route for `/v1/responses`**

In `tests/api.rs`, extend the existing OpenAI-compatible provider test server helper so it captures both chat and responses requests. Add captured fields:

```rust
responses_auth: Option<String>,
responses_model: Option<String>,
responses_body: Option<serde_json::Value>,
```

In the test server request router, add a branch:

```rust
if method == Method::POST && path == "/v1/responses" {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    {
        let mut captured = captured.lock().unwrap();
        captured.responses_auth = auth;
        captured.responses_model = body.get("model").and_then(|v| v.as_str()).map(str::to_string);
        captured.responses_body = Some(body);
    }
    return sse_response(vec![
        serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "responses profile ok"
        }),
        serde_json::json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 1,
                    "output_tokens": 1,
                    "total_tokens": 2
                }
            }
        }),
    ]);
}
```

If the existing helper does not have `sse_response`, add a local helper that returns `text/event-stream` with `data: <json>\n\n` frames.

- [ ] **Step 3: Add API job test**

Add:

```rust
#[tokio::test]
async fn api_jobs_accept_openai_responses_provider_profile_per_request() {
    let provider = start_openai_compatible_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_RESPONSES_PROVIDER_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-responses-provider-token");
    }

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "Reply with exactly: responses profile ok",
                    "model": "gpt-4.1-mini",
                        "approval": "auto",
                        "max_steps": 1,
                        "provider": {
                            "name": "openai-responses",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(created.status(), StatusCode::OK);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::RunCompleted {
                output: Some(output),
                ..
            } if output.contains("responses profile ok")
        )
    }));
    let captured = provider.captured.lock().unwrap();
    assert_eq!(
        captured.responses_auth.as_deref(),
        Some("Bearer dummy-responses-provider-token")
    );
    assert_eq!(captured.responses_model.as_deref(), Some("gpt-4.1-mini"));
}
```

- [ ] **Step 4: Update API provider test behavior**

The `/providers/test` route can use the same `/models` inventory endpoint as OpenAI-compatible. Ensure provider profile validation accepts `openai-responses` and reports:

```json
{
  "provider": "openai-responses",
  "status": "pass"
}
```

- [ ] **Step 5: Extend provider integration runner**

In `scripts/provider-integration.ps1`:

1. Update provider normalization:

```powershell
"openai-responses" { return "openai-responses" }
"responses" { return "openai-responses" }
```

2. Treat `openai-responses` as key-required:

```powershell
return $normalized -in @("openai-compatible", "openai-responses", "anthropic")
```

3. Use OpenAI-style model inventory for both OpenAI provider names.

4. Dispatch smoke:

```powershell
"openai-responses" {
    $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "1"
    $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL = $Model
    $testName = "openai_responses_real_provider_smoke_when_enabled"
}
```

5. Include `openai-responses` in generated provider profile bodies.

- [ ] **Step 6: Update environment example**

Add to `.env.integration.example`:

```env
ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES=0
ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL=gpt-4.1-mini
ROVE_OPENAI_RESPONSES_PROMPT_CACHE=false
ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION=
```

Update `ROVE_PROVIDER_INTEGRATION_PROVIDER` comment to mention `openai-responses`.

- [ ] **Step 7: Update docs**

In `docs/runtime/provider-smoke.md`, add a section:

```markdown
## OpenAI Responses

Use `openai-responses` for OpenAI's `/v1/responses` endpoint. This path is separate from `openai-compatible`, which continues to use `/chat/completions`.

```powershell
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-responses `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini"
```
```

In `docs/runtime/release-readiness.md`, add a Provider Gate Matrix row:

```markdown
| OpenAI Responses official API | Yes when claiming Codex-style/OpenAI Responses readiness | Yes when quota allows | Uses `/v1/responses`; separate from chat completions. |
```

- [ ] **Step 8: Add code hygiene assertions**

In `tests/code_hygiene.rs`, extend provider docs assertions:

```rust
assert!(script.contains("openai-responses"));
assert!(script.contains("openai_responses_real_provider_smoke_when_enabled"));
assert!(env_example.contains("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES"));
assert!(provider_docs.contains("-Provider openai-responses"));
assert!(readiness.contains("OpenAI Responses official API"));
```

- [ ] **Step 9: Run tests**

Run:

```powershell
cargo test --test provider_smoke
cargo test --test api api_jobs_accept_openai_responses_provider_profile_per_request -- --exact
cargo test --test code_hygiene provider_integration_runner_supports_native_provider_protocols -- --exact
```

Expected: deterministic tests pass; real-provider smoke exits early unless env flags are enabled.

- [ ] **Step 10: Commit**

```powershell
git add tests/provider_smoke.rs tests/api.rs tests/code_hygiene.rs scripts/provider-integration.ps1 .env.integration.example docs/runtime/provider-smoke.md docs/runtime/release-readiness.md
git commit -m "Add OpenAI Responses provider gates"
```

---

### Task 4: Add Prompt Build Metadata And Prompt Cache Evidence

**Files:**
- Create: `src/core/prompt_metadata.rs`
- Modify: `src/core/mod.rs`
- Modify: `src/core/context.rs`
- Modify: `src/core/events.rs`
- Modify: `src/state/artifacts.rs`
- Modify: `src/state/report.rs`
- Test: unit tests in `src/core/prompt_metadata.rs` and `src/core/context.rs`

- [ ] **Step 1: Create prompt metadata type**

Create `src/core/prompt_metadata.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::core::types::{Message, ToolSchema};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptBuildMetadata {
    pub prompt_hash: String,
    pub stable_prefix_hash: String,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
    pub token_estimate: usize,
    pub included_history_messages: usize,
    pub dropped_history_messages: usize,
    pub prompt_cache_key: Option<String>,
}

pub fn stable_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{digest:x}")
}

pub fn prompt_hash(messages: &[Message]) -> String {
    stable_hash(&serde_json::to_string(messages).unwrap_or_default())
}

pub fn tool_signature(tools: &[ToolSchema]) -> String {
    let mut sorted = tools.to_vec();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    stable_hash(&serde_json::to_string(&sorted).unwrap_or_default())
}

pub fn prompt_cache_key(stable_prefix_hash: &str, tool_signature: &str) -> String {
    stable_hash(&format!("{stable_prefix_hash}:{tool_signature}"))
}
```

- [ ] **Step 2: Make sha2 non-optional**

In `Cargo.toml`, change:

```toml
sha2 = { version = "0.10", optional = true }
```

to:

```toml
sha2 = "0.10"
```

Keep the `rag` feature list as-is except remove `dep:sha2` from it.

- [ ] **Step 3: Export module**

In `src/core/mod.rs`:

```rust
pub mod prompt_metadata;
```

- [ ] **Step 4: Extend ContextBuild**

In `src/core/context.rs`, add to `ContextBuild`:

```rust
pub metadata: crate::core::prompt_metadata::PromptBuildMetadata,
```

When building by message count and by token budget, populate:

```rust
let metadata = PromptBuildMetadata {
    prompt_hash: prompt_hash(&messages),
    stable_prefix_hash: stable_hash(&self.system_prompt),
    workspace_fingerprint: String::new(),
    tool_signature: String::new(),
    token_estimate,
    included_history_messages,
    dropped_history_messages,
    prompt_cache_key: None,
};
```

Leave workspace fingerprint and tool signature empty in `ContextManager`; fill them in the loop before emitting events because the context manager does not own workspace or registry.

- [ ] **Step 5: Add stream event**

In `src/core/events.rs`, add:

```rust
PromptBuilt {
    metadata: crate::core::prompt_metadata::PromptBuildMetadata,
},
```

Add `event_name()` arm:

```rust
Self::PromptBuilt { .. } => "prompt_built",
```

- [ ] **Step 6: Emit PromptBuilt in runtime loops**

In `src/core/run_loop.rs` and `src/core/plan_loop.rs`, after `build_with_checkpoint`, clone and enrich metadata:

```rust
let mut prompt_metadata = context.metadata.clone();
prompt_metadata.workspace_fingerprint =
    crate::core::runtime_identity::workspace_fingerprint(ctx.workspace);
prompt_metadata.tool_signature =
    crate::core::prompt_metadata::tool_signature(&ctx.registry.schemas());
prompt_metadata.prompt_cache_key = Some(crate::core::prompt_metadata::prompt_cache_key(
    &prompt_metadata.stable_prefix_hash,
    &prompt_metadata.tool_signature,
));
yield LoopItem::Event(StreamEvent::PromptBuilt {
    metadata: prompt_metadata,
});
```

- [ ] **Step 7: Persist metadata in artifacts**

In `RunArtifactRecorder`, add:

```rust
prompt_builds: Vec<PromptBuildMetadata>,
```

Initialize with `Vec::new()`. In `record_event`:

```rust
StreamEvent::PromptBuilt { metadata } => {
    self.prompt_builds.push(metadata.clone());
    self.write_snapshot(state_store).await;
}
```

Add `prompt_builds` to `RunReport`.

- [ ] **Step 8: Run tests**

Run:

```powershell
cargo test prompt_metadata
cargo test core::context
cargo test state::artifacts
```

Expected: all pass.

- [ ] **Step 9: Commit**

```powershell
git add Cargo.toml Cargo.lock src/core/prompt_metadata.rs src/core/mod.rs src/core/context.rs src/core/events.rs src/core/run_loop.rs src/core/plan_loop.rs src/state/artifacts.rs src/state/report.rs
git commit -m "Record prompt build metadata"
```

---

### Task 5: Add Runtime Identity To Checkpoints

**Files:**
- Create: `src/core/runtime_identity.rs`
- Modify: `src/core/mod.rs`
- Modify: `src/core/types.rs`
- Modify: `src/core/engine.rs`
- Modify: `src/state/artifacts.rs`
- Modify: `src/state/resume.rs`
- Test: unit tests in `src/core/runtime_identity.rs`, `src/state/resume.rs`

- [ ] **Step 1: Create runtime identity type and helpers**

Create `src/core/runtime_identity.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::core::prompt_metadata::{stable_hash, tool_signature};
use crate::core::types::{ApprovalPolicy, ToolSchema};
use crate::core::workspace::{Workspace, WorkspaceKind};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub cwd: String,
    pub workspace_kind: WorkspaceKind,
    pub model_id: String,
    pub provider_target: String,
    pub approval_policy: ApprovalPolicy,
    pub max_steps: u32,
    pub plan_enabled: bool,
    pub system_prompt_hash: String,
    pub planner_prompt_hash: String,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentityEvaluation {
    pub status: RuntimeIdentityStatus,
    pub mismatch_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeIdentityStatus {
    FullValid,
    RuntimeMismatch,
    Missing,
}

impl Default for RuntimeIdentityStatus {
    fn default() -> Self {
        Self::Missing
    }
}

pub fn workspace_fingerprint(workspace: &Workspace) -> String {
    stable_hash(&format!(
        "{}:{}",
        workspace.root.display(),
        workspace_kind_label(&workspace.kind)
    ))
}

pub fn build_runtime_identity(
    workspace: &Workspace,
    model_id: &str,
    provider_target: &str,
    approval_policy: ApprovalPolicy,
    max_steps: u32,
    plan_enabled: bool,
    system_prompt: &str,
    planner_prompt: &str,
    tools: &[ToolSchema],
) -> RuntimeIdentity {
    RuntimeIdentity {
        cwd: workspace.root.display().to_string(),
        workspace_kind: workspace.kind.clone(),
        model_id: model_id.to_string(),
        provider_target: provider_target.to_string(),
        approval_policy,
        max_steps,
        plan_enabled,
        system_prompt_hash: stable_hash(system_prompt),
        planner_prompt_hash: stable_hash(planner_prompt),
        workspace_fingerprint: workspace_fingerprint(workspace),
        tool_signature: tool_signature(tools),
    }
}

pub fn evaluate_runtime_identity(
    saved: Option<&RuntimeIdentity>,
    current: &RuntimeIdentity,
) -> RuntimeIdentityEvaluation {
    let Some(saved) = saved else {
        return RuntimeIdentityEvaluation {
            status: RuntimeIdentityStatus::Missing,
            mismatch_fields: Vec::new(),
        };
    };
    let mut mismatch_fields = Vec::new();
    if saved.cwd != current.cwd {
        mismatch_fields.push("cwd".to_string());
    }
    if saved.workspace_kind != current.workspace_kind {
        mismatch_fields.push("workspace_kind".to_string());
    }
    if saved.model_id != current.model_id {
        mismatch_fields.push("model_id".to_string());
    }
    if saved.provider_target != current.provider_target {
        mismatch_fields.push("provider_target".to_string());
    }
    if saved.approval_policy != current.approval_policy {
        mismatch_fields.push("approval_policy".to_string());
    }
    if saved.max_steps != current.max_steps {
        mismatch_fields.push("max_steps".to_string());
    }
    if saved.plan_enabled != current.plan_enabled {
        mismatch_fields.push("plan_enabled".to_string());
    }
    if saved.system_prompt_hash != current.system_prompt_hash {
        mismatch_fields.push("system_prompt_hash".to_string());
    }
    if saved.planner_prompt_hash != current.planner_prompt_hash {
        mismatch_fields.push("planner_prompt_hash".to_string());
    }
    if saved.workspace_fingerprint != current.workspace_fingerprint {
        mismatch_fields.push("workspace_fingerprint".to_string());
    }
    if saved.tool_signature != current.tool_signature {
        mismatch_fields.push("tool_signature".to_string());
    }
    RuntimeIdentityEvaluation {
        status: if mismatch_fields.is_empty() {
            RuntimeIdentityStatus::FullValid
        } else {
            RuntimeIdentityStatus::RuntimeMismatch
        },
        mismatch_fields,
    }
}

fn workspace_kind_label(kind: &WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Folder => "folder",
        WorkspaceKind::Repo => "repo",
        WorkspaceKind::Task => "task",
    }
}
```

- [ ] **Step 2: Export module**

In `src/core/mod.rs`:

```rust
pub mod runtime_identity;
```

- [ ] **Step 3: Add fields to persisted types**

In `src/core/types.rs`, add:

```rust
#[serde(default)]
pub runtime_identity: Option<crate::core::runtime_identity::RuntimeIdentity>,
```

to `TaskState` and `PromptCheckpoint`.

- [ ] **Step 4: Thread identity through artifact recorder**

In `RunArtifactRecorder`, add:

```rust
runtime_identity: Option<RuntimeIdentity>,
```

Add constructor parameter:

```rust
runtime_identity: Option<RuntimeIdentity>,
```

Store it in `TaskState` and `PromptCheckpoint`.

- [ ] **Step 5: Build identity in engine**

In `src/core/engine.rs`, before constructing `RunArtifactRecorder`, build:

```rust
let runtime_identity = build_runtime_identity(
    &self.workspace,
    self.model.model_id(),
    self.model.client_id().as_str(),
    self.approval_policy,
    self.config.max_steps,
    self.config.plan_enabled,
    self.context_manager.system_prompt(),
    self.planner.prompt(),
    &self.registry.schemas(),
);
```

If `Planner::prompt()` does not exist, add:

```rust
pub fn prompt(&self) -> &str {
    &self.prompt
}
```

- [ ] **Step 6: Resume mismatch behavior**

In `src/state/resume.rs`, do not reject old states. Add helper-level evaluation that returns the state plus mismatch fields. If the current resume API only returns `Option<TaskState>`, preserve it and record mismatch in the loaded state summary:

```rust
if let Some(saved_identity) = state
    .checkpoint
    .as_ref()
    .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
{
    let evaluation = evaluate_runtime_identity(Some(saved_identity), &current_identity);
    if evaluation.status == RuntimeIdentityStatus::RuntimeMismatch {
        tracing::warn!(
            mismatch_fields = ?evaluation.mismatch_fields,
            "resume runtime identity mismatch"
        );
    }
}
```

- [ ] **Step 7: Run tests**

Run:

```powershell
cargo test runtime_identity
cargo test state::resume
cargo test state::artifacts
```

Expected: old states still deserialize; new states include runtime identity.

- [ ] **Step 8: Commit**

```powershell
git add src/core/runtime_identity.rs src/core/mod.rs src/core/types.rs src/core/engine.rs src/core/planner.rs src/state/artifacts.rs src/state/resume.rs
git commit -m "Persist runtime identity checkpoints"
```

---

### Task 6: Add Tool Execution Metadata

**Files:**
- Modify: `src/core/types.rs`
- Modify: `src/core/executor.rs`
- Modify: `src/core/tool_turn.rs`
- Modify: `src/core/events.rs`
- Modify: `src/state/artifacts.rs`
- Modify: `src/state/report.rs`
- Test: unit tests in `src/core/executor.rs`, `src/core/tool_turn.rs`, existing API/CLI tests

- [ ] **Step 1: Add metadata types**

In `src/core/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Ok,
    Error,
    Rejected,
    PartialSuccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionMetadata {
    pub status: ToolExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_event_type: Option<String>,
    pub risk_level: ToolRiskLevel,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_paths: Vec<String>,
    pub workspace_changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff_summary: Vec<String>,
}

impl Default for ToolExecutionStatus {
    fn default() -> Self {
        Self::Ok
    }
}

impl Default for ToolRiskLevel {
    fn default() -> Self {
        Self::Low
    }
}
```

Extend `ToolResult`:

```rust
#[serde(default)]
pub metadata: ToolExecutionMetadata,
```

- [ ] **Step 2: Build success metadata in executor**

In `src/core/executor.rs`, after tool output:

```rust
let metadata = ToolExecutionMetadata {
    status: ToolExecutionStatus::Ok,
    error_code: None,
    security_event_type: None,
    risk_level: if schema.destructive {
        ToolRiskLevel::High
    } else {
        ToolRiskLevel::Low
    },
    read_only: !schema.destructive,
    affected_paths: output.mutations.iter().map(|mutation| mutation.path.clone()).collect(),
    workspace_changed: !output.mutations.is_empty(),
    diff_summary: output
        .mutations
        .iter()
        .map(|mutation| format!("{:?}: {}", mutation.operation, mutation.path))
        .collect(),
};
```

Use it in `ToolResult`.

- [ ] **Step 3: Add failure metadata**

Change `StreamEvent::ToolCallFailed`:

```rust
ToolCallFailed {
    call_id: CallId,
    error: ToolError,
    metadata: ToolExecutionMetadata,
},
```

In `run_tool_turn`, when an error occurs, build:

```rust
let metadata = ToolExecutionMetadata {
    status: ToolExecutionStatus::Error,
    error_code: Some(error.error_code().to_string()),
    security_event_type: security_event_type(&error),
    risk_level: ToolRiskLevel::High,
    read_only: false,
    affected_paths: Vec::new(),
    workspace_changed: false,
    diff_summary: Vec::new(),
};
yield ToolTurnItem::Event(StreamEvent::ToolCallFailed {
    call_id: execution.call.call_id,
    error,
    metadata,
});
```

Add helper:

```rust
fn security_event_type(error: &ToolError) -> Option<String> {
    let text = error.to_string();
    if text.contains("escapes workspace") {
        Some("path_escape".to_string())
    } else if text.contains("Permission denied") {
        Some("approval_denied".to_string())
    } else {
        None
    }
}
```

- [ ] **Step 4: Persist metadata**

In `RunArtifactRecorder`, add:

```rust
tool_execution_metadata: Vec<ToolExecutionMetadata>,
```

Push metadata from `ToolCallCompleted` and `ToolCallFailed`. Add it to `RunReport`.

- [ ] **Step 5: Run tests**

Run:

```powershell
cargo test core::executor
cargo test core::tool_turn
cargo test --test api
```

Expected: all matches on `ToolCallFailed` are updated and pass.

- [ ] **Step 6: Commit**

```powershell
git add src/core/types.rs src/core/executor.rs src/core/tool_turn.rs src/core/events.rs src/state/artifacts.rs src/state/report.rs
git commit -m "Record structured tool execution metadata"
```

---

### Task 7: Document ReAct Core And Pico Relationship

**Files:**
- Create: `docs/runtime/react-loop.md`
- Modify: `docs/runtime/README.md`
- Modify: `README.md`
- Modify: `tests/code_hygiene.rs`

- [ ] **Step 1: Write runtime doc**

Create `docs/runtime/react-loop.md`:

```markdown
# Runtime Loop: Plan Outside, ReAct Inside

rove uses a Plan + ReAct runtime shape.

The unplanned loop in `src/core/run_loop.rs` is the pure ReAct loop:

1. Build context with `ContextManager::build_with_checkpoint`.
2. Compact old history when the token budget requires it.
3. Run one model turn through `run_model_turn`.
4. Normalize native OpenAI/Anthropic/Responses tool-use into `Action`.
5. Run one tool turn through `run_tool_turn`.
6. Append assistant tool calls and tool results back into history.
7. Repeat until final answer, cancellation, token limit, step limit, or error.

The planned loop in `src/core/plan_loop.rs` keeps the same ReAct core inside each plan step:

1. Draft or resume a `TaskPlan`.
2. Convert the current plan step into a focused user prompt.
3. Run the same model turn and tool turn as the unplanned loop.
4. Mark the step complete on success.
5. Re-plan when a step fails and the failure is recoverable.

This differs from pico's `pico/agent_loop.py`, where prompt build, model call, parse, tool execution, checkpoint, and trace recording live in one readable loop. rove keeps those phases split into focused Rust modules so provider streaming, cancellation, artifact persistence, API jobs, and plan recovery can evolve independently.

The conceptual ReAct unit in rove is:

```text
ReactTurn =
  ContextBuild
  -> ModelTurn
  -> Action
  -> ToolTurn
  -> HistoryAppend
```

`Engine` is the orchestration shell. It loads resume state and memory, chooses planned or unplanned mode, streams events, and writes run artifacts.
```

- [ ] **Step 2: Link docs**

Add `react-loop.md` to `docs/runtime/README.md` and the root `README.md` runtime docs section.

- [ ] **Step 3: Add hygiene test**

In `tests/code_hygiene.rs`:

```rust
#[test]
fn runtime_docs_explain_plan_react_core() {
    let doc = std::fs::read_to_string("docs/runtime/react-loop.md").unwrap();
    let runtime_readme = std::fs::read_to_string("docs/runtime/README.md").unwrap();
    let root_readme = std::fs::read_to_string("README.md").unwrap();

    assert!(doc.contains("Plan Outside, ReAct Inside"));
    assert!(doc.contains("run_unplanned_loop"));
    assert!(doc.contains("run_planned_loop"));
    assert!(doc.contains("run_model_turn"));
    assert!(doc.contains("run_tool_turn"));
    assert!(doc.contains("ReactTurn"));
    assert!(runtime_readme.contains("react-loop.md"));
    assert!(root_readme.contains("react-loop.md"));
}
```

- [ ] **Step 4: Run test**

Run:

```powershell
cargo test --test code_hygiene runtime_docs_explain_plan_react_core -- --exact
```

Expected: passes.

- [ ] **Step 5: Commit**

```powershell
git add docs/runtime/react-loop.md docs/runtime/README.md README.md tests/code_hygiene.rs
git commit -m "Document Plan plus ReAct runtime core"
```

---

### Task 8: Add Benchmark Evidence Package Format

**Files:**
- Modify: `src/bench.rs`
- Create or modify: `benchmarks/results/README.md`
- Create or modify: `docs/runtime/benchmark-evidence.md`
- Modify: `tests/code_hygiene.rs`

- [ ] **Step 1: Document evidence package shape**

Create `benchmarks/results/README.md`:

```markdown
# Benchmark Result Packages

Each benchmark run should write a dated directory:

```text
benchmarks/results/<scenario>-<YYYY-MM-DD>/
  DATA_PROVENANCE.md
  rove-benchmark-core-report.md
  metrics.json
  artifacts/
```

`DATA_PROVENANCE.md` records command lines, git commit, provider mode, model id, whether network was used, workspace path, and whether artifacts contain secrets.

`rove-benchmark-core-report.md` summarizes:

- harness regression;
- context ablation;
- working memory ablation;
- recovery/resume ablation;
- provider behavior when a real provider was used;
- failures classified as model, provider, runtime, or harness.
```

- [ ] **Step 2: Add runtime benchmark docs**

Create `docs/runtime/benchmark-evidence.md` with:

```markdown
# Benchmark Evidence

Benchmark claims must be backed by a result package under `benchmarks/results/`.

Use deterministic fake-provider runs for runtime regressions. Use real-provider runs only for provider claims, and keep those claims separate from local runtime health.

Required files:

- `DATA_PROVENANCE.md`
- `rove-benchmark-core-report.md`
- `metrics.json`

The report should follow pico's latest evidence shape while using rove terminology: harness regression, context ablation, memory ablation, recovery/resume ablation, and provider gate evidence.
```

- [ ] **Step 3: Add code hygiene test**

In `tests/code_hygiene.rs`:

```rust
#[test]
fn benchmark_evidence_format_is_documented() {
    let results = std::fs::read_to_string("benchmarks/results/README.md").unwrap();
    let docs = std::fs::read_to_string("docs/runtime/benchmark-evidence.md").unwrap();

    assert!(results.contains("DATA_PROVENANCE.md"));
    assert!(results.contains("rove-benchmark-core-report.md"));
    assert!(results.contains("metrics.json"));
    assert!(docs.contains("harness regression"));
    assert!(docs.contains("recovery/resume ablation"));
}
```

- [ ] **Step 4: Run test**

Run:

```powershell
cargo test --test code_hygiene benchmark_evidence_format_is_documented -- --exact
```

Expected: passes.

- [ ] **Step 5: Commit**

```powershell
git add benchmarks/results/README.md docs/runtime/benchmark-evidence.md tests/code_hygiene.rs
git commit -m "Document benchmark evidence packages"
```

---

### Task 9: Full Verification And Release Notes

**Files:**
- Modify: `README.md`
- Modify: `docs/runtime/provider-smoke.md`
- Modify: `docs/runtime/release-readiness.md`
- Modify: `docs/runtime/implementation-status.md`

- [ ] **Step 1: Run deterministic Rust gates**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all commands exit 0.

- [ ] **Step 2: Run web gates if frontend dependencies are installed**

Run:

```powershell
cd web-ui
pnpm test
pnpm typecheck
pnpm build
cd ..
```

Expected: all commands exit 0. If dependencies are not installed, record the exact failure and do not claim web verification.

- [ ] **Step 3: Run provider smoke without real credentials**

Run:

```powershell
cargo test --test provider_smoke
```

Expected: pass with OpenAI-compatible, OpenAI Responses, Anthropic, and Ollama real-provider gates skipped.

- [ ] **Step 4: Run OpenAI Responses real gate when credentials exist**

Run:

```powershell
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_responses_real_provider_smoke_when_enabled -- --exact --nocapture
```

Expected: direct final-answer and echo tool-use smoke pass. If the selected model lacks tool-call support, classify it as model capability, not runtime failure.

- [ ] **Step 5: Run full provider integration for Responses when quota allows**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-responses `
  -ApiBase "https://api.openai.com/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "gpt-4.1-mini" `
  -RunStress `
  -RunRestartRecovery
```

Expected: `evidence-summary.json` records provider smoke, API smoke, Web smoke, stress, and restart recovery as pass or a classified external failure.

- [ ] **Step 6: Update implementation status**

In `docs/runtime/implementation-status.md`, add:

```markdown
## Pico-Inspired Runtime Provider Upgrades

- OpenAI Responses provider: implemented as `openai-responses`, separate from `openai-compatible`.
- Runtime loop: documented as Plan outside, ReAct inside.
- Runtime identity: persisted in checkpoints for resume diagnostics.
- Prompt build metadata: recorded in prompt events and run reports.
- Tool execution metadata: recorded for success and failure paths.
- Benchmark evidence: result package format documented.
```

- [ ] **Step 7: Inspect git status**

Run:

```powershell
git status --short
git diff --stat
```

Expected: only intentional source, test, script, and docs files changed. No `.rove/`, SQLite, logs, screenshots, provider keys, or temporary benchmark outputs are staged.

- [ ] **Step 8: Commit final docs/status**

```powershell
git add README.md docs/runtime/provider-smoke.md docs/runtime/release-readiness.md docs/runtime/implementation-status.md
git commit -m "Document pico-inspired provider and runtime upgrades"
```

---

## Handoff Prompt For A New Conversation

Use this prompt when opening the implementation conversation:

```text
We are in D:\Study\project\agent\rove. Please implement the plan at docs/plans/2026-06-09-pico-inspired-runtime-provider-upgrades.md task by task. Use the repo's existing Rust patterns. Keep OpenAI chat completions under provider name openai-compatible, and add OpenAI Responses as a separate provider name openai-responses. Preserve backward compatibility for existing .rove task_state/report artifacts with serde defaults. Run the verification commands in each task and report exact failures before moving on.
```

## Self-Review

- Spec coverage: The plan covers OpenAI Responses provider support, prompt cache evidence, provider profiles, integration gates, ReAct documentation, runtime identity checkpoints, prompt build metadata, tool execution metadata, benchmark evidence packaging, and full verification.
- Placeholder scan: The plan avoids unspecified implementation gaps and gives exact file paths, code snippets, commands, and expected outcomes.
- Type consistency: Provider name is consistently `openai-responses`; current chat path remains `openai-compatible`; runtime metadata types are named consistently across `types.rs`, `runtime_identity.rs`, `prompt_metadata.rs`, artifacts, and reports.
