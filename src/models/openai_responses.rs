use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{StatusCode, header::HeaderMap};
use std::collections::BTreeMap;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelClientId, ModelEvent};

/// OpenAI Responses API model client.
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
            body.insert(
                "instructions".to_string(),
                serde_json::Value::String(instructions),
            );
        }
        if !tool_defs.is_empty() {
            body.insert("tools".to_string(), serde_json::Value::Array(tool_defs));
        }
        if self.prompt_cache_enabled {
            body.insert(
                "prompt_cache_key".to_string(),
                serde_json::Value::String(prompt_cache_key(messages, tools)),
            );
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

fn format_responses_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions = None;
    let mut input = Vec::new();

    for message in messages {
        match message.role {
            Role::System if instructions.is_none() => {
                instructions = Some(message.content.clone());
            }
            Role::System | Role::User => {
                input.push(input_text_item("user", &message.content));
            }
            Role::Assistant if !message.tool_calls.is_empty() => {
                if !message.content.is_empty() {
                    input.push(output_text_item(&message.content));
                }
                for tool_call in &message.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.args.to_string(),
                    }));
                }
            }
            Role::Assistant => {
                input.push(output_text_item(&message.content));
            }
            Role::Tool if message.tool_call_id.is_some() => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": message.tool_call_id.as_deref().unwrap_or_default(),
                    "output": message.content,
                }));
            }
            Role::Tool => {
                input.push(input_text_item("user", &message.content));
            }
        }
    }

    (instructions, input)
}

fn input_text_item(role: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "content": [{ "type": "input_text", "text": text }],
    })
}

fn output_text_item(text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text }],
    })
}

fn format_responses_tool(tool: &ToolSchema) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "strict": false,
    })
}

fn prompt_cache_key(messages: &[Message], tools: &[ToolSchema]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(&(messages, tools)).unwrap_or_default() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("rove-responses-{hash:016x}")
}

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
                .unwrap_or_default();
            let delta = json
                .get("delta")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !item_id.is_empty()
                && !delta.is_empty()
                && let Some(call) = state.function_calls.get_mut(item_id)
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
                .unwrap_or_default();
            if let Some(call) = state.function_calls.get_mut(item_id) {
                if let Some(arguments) = json.get("arguments").and_then(|value| value.as_str()) {
                    call.arguments = arguments.to_string();
                }
                call.done = true;
                events.push(ModelEvent::ToolUseDone {
                    id: call.call_id.clone(),
                    name: call.name.clone(),
                    args: parse_arguments(&call.arguments),
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

    if let Some(call) = state.function_calls.get_mut(&item_id) {
        if call.done {
            return;
        }
        call.done = true;
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
        .unwrap_or(u64::from(prompt_tokens) + u64::from(completion_tokens))
        as u32;
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
    let body_lower = body.to_ascii_lowercase();
    if body_lower.contains("context") && body_lower.contains("token") {
        return ModelError::ContextLengthExceeded { used: 0, max: 0 };
    }
    ModelError::RequestFailed(format!("HTTP {}: {}", status, body))
}

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
                    if let Some(data) = line.strip_prefix("data:")
                        && let Ok(events) = normalize_responses_event(&mut state, data.trim_start())
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
    fn request_body_adds_prompt_cache_fields_only_when_enabled() {
        let client = OpenAiResponsesClient::new(
            "https://api.openai.com/v1".to_string(),
            "secret".to_string(),
            "gpt-4.1-mini".to_string(),
        );
        let uncached = client.build_request_body(&[Message::user("inspect")], &[]);

        assert!(uncached.get("prompt_cache_key").is_none());
        assert!(uncached.get("prompt_cache_retention").is_none());

        let cached = client
            .with_prompt_cache(true, Some("24h".to_string()))
            .build_request_body(&[Message::user("inspect")], &[]);

        assert!(
            cached["prompt_cache_key"]
                .as_str()
                .is_some_and(|value| value.starts_with("rove-responses-"))
        );
        assert_eq!(cached["prompt_cache_retention"], "24h");
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
