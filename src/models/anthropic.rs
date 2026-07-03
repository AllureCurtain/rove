use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{
    StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};
use std::collections::BTreeMap;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelClientId, ModelEvent};

const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let (system_prompt, conversation) = extract_system(messages);

        let msgs: Vec<serde_json::Value> =
            conversation.iter().map(format_anthropic_message).collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": msgs,
            "stream": true,
        });

        if let Some(system) = system_prompt {
            body["system"] = serde_json::Value::String(system);
        }

        if !tools.is_empty() {
            let tool_defs: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        body
    }
}

fn format_anthropic_message(m: &Message) -> serde_json::Value {
    match m.role {
        Role::Assistant if !m.tool_calls.is_empty() => {
            let mut content_blocks: Vec<serde_json::Value> = Vec::new();
            if !m.content.is_empty() {
                content_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": m.content,
                }));
            }
            for tc in &m.tool_calls {
                content_blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": tc.args,
                }));
            }
            serde_json::json!({
                "role": "assistant",
                "content": content_blocks,
            })
        }
        Role::Tool => {
            let block = if let Some(ref id) = m.tool_call_id {
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": m.content,
                })
            } else {
                serde_json::json!({
                    "type": "text",
                    "text": m.content,
                })
            };
            serde_json::json!({
                "role": "user",
                "content": [block],
            })
        }
        _ => {
            serde_json::json!({
                "role": anthropic_role(&m.role),
                "content": m.content,
            })
        }
    }
}

fn extract_system(messages: &[Message]) -> (Option<String>, &[Message]) {
    if let Some(first) = messages.first()
        && first.role == Role::System
    {
        return (Some(first.content.clone()), &messages[1..]);
    }
    (None, messages)
}

fn anthropic_role(role: &Role) -> &'static str {
    match role {
        Role::User | Role::System | Role::Tool => "user",
        Role::Assistant => "assistant",
    }
}

fn parse_retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1000))
}

fn classify_anthropic_error(status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelError::AuthFailed;
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelError::RateLimited {
            retry_after_ms: parse_retry_after_ms(headers).unwrap_or(1000),
        };
    }

    let error_type = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json.get("error")?.get("type")?.as_str().map(str::to_string));

    if error_type.as_deref() == Some("invalid_request_error") && body.contains("token") {
        return ModelError::ContextLengthExceeded { used: 0, max: 0 };
    }

    ModelError::RequestFailed(format!("HTTP {}: {}", status, body))
}

#[derive(Debug, Default)]
struct AnthropicToolUseState {
    blocks: BTreeMap<u64, AnthropicPartialToolUse>,
    usage: Usage,
}

#[derive(Debug, Default)]
struct AnthropicPartialToolUse {
    id: String,
    name: String,
    input_json: String,
}

fn normalize_anthropic_event(
    state: &mut AnthropicToolUseState,
    data: &str,
) -> serde_json::Result<Vec<ModelEvent>> {
    let json = serde_json::from_str::<serde_json::Value>(data)?;
    let mut events = Vec::new();
    let event_type = json.get("type").and_then(|value| value.as_str());

    match event_type {
        Some("content_block_start")
            if json
                .get("content_block")
                .and_then(|block| block.get("type"))
                .and_then(|value| value.as_str())
                == Some("tool_use") =>
        {
            let index = json
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let block = json
                .get("content_block")
                .unwrap_or(&serde_json::Value::Null);
            let id = block
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("toolu_unknown")
                .to_string();
            let name = block
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("tool")
                .to_string();
            let input_json = block
                .get("input")
                .filter(|value| !value.as_object().is_some_and(|object| object.is_empty()))
                .map(serde_json::Value::to_string)
                .unwrap_or_default();
            state.blocks.insert(
                index,
                AnthropicPartialToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input_json,
                },
            );
            events.push(ModelEvent::ToolUseStart { id, name });
        }
        Some("content_block_delta") => {
            let delta = json.get("delta").unwrap_or(&serde_json::Value::Null);
            if let Some(text) = delta.get("text").and_then(|value| value.as_str())
                && !text.is_empty()
            {
                events.push(ModelEvent::TextDelta {
                    text: text.to_string(),
                });
            }
            if delta.get("type").and_then(|value| value.as_str()) == Some("input_json_delta") {
                let index = json
                    .get("index")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                if let Some(partial_json) =
                    delta.get("partial_json").and_then(|value| value.as_str())
                    && let Some(partial) = state.blocks.get_mut(&index)
                {
                    partial.input_json.push_str(partial_json);
                    events.push(ModelEvent::ToolUseDelta {
                        id: partial.id.clone(),
                        args_delta: partial_json.to_string(),
                    });
                }
            }
        }
        Some("content_block_stop") => {
            let index = json
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if let Some(partial) = state.blocks.remove(&index) {
                let args = serde_json::from_str::<serde_json::Value>(&partial.input_json)
                    .unwrap_or_else(|_| serde_json::Value::String(partial.input_json.clone()));
                events.push(ModelEvent::ToolUseDone {
                    id: partial.id,
                    name: partial.name,
                    args,
                });
            }
        }
        Some("message_start") => {
            if let Some(usage) = json.get("message").and_then(|message| message.get("usage")) {
                state.usage.prompt_tokens = usage
                    .get("input_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as u32;
            }
        }
        Some("message_delta") => {
            if let Some(usage) = json.get("usage") {
                state.usage.completion_tokens = usage
                    .get("output_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as u32;
                state.usage.total_tokens =
                    state.usage.prompt_tokens + state.usage.completion_tokens;
            }
        }
        Some("message_stop") => {
            events.push(ModelEvent::Usage {
                usage: state.usage.clone(),
            });
            events.push(ModelEvent::Done);
        }
        _ => {}
    }

    Ok(events)
}

#[async_trait]
impl ModelClient for AnthropicClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let body = self.build_request_body(messages, tools);
        let url = format!("{}/v1/messages", self.api_base);
        let api_key = self.api_key.clone();

        Box::pin(async_stream::stream! {
            let response = self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ModelError::RequestFailed(e.to_string()))?;

            if !response.status().is_success() {
                let status = response.status();
                let headers = response.headers().clone();
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_anthropic_error(status, &headers, &text));
                return;
            }

            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut stream_state = AnthropicToolUseState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ModelError::StreamInterrupted(e.to_string()));
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
                        && let Ok(events) = normalize_anthropic_event(&mut stream_state, data)
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
        ModelClientId::new("anthropic", &self.api_base, &self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Message, ToolSchema};
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    #[test]
    fn request_body_separates_system_message() {
        let client = AnthropicClient::new(
            "https://api.anthropic.com".to_string(),
            "sk-test".to_string(),
            "claude-sonnet-4-6-20250514".to_string(),
        );
        let body = client.build_request_body(
            &[Message::system("You are helpful."), Message::user("Hello")],
            &[],
        );

        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    #[test]
    fn request_body_includes_tools_with_input_schema() {
        let client = AnthropicClient::new(
            "https://api.anthropic.com".to_string(),
            "sk-test".to_string(),
            "claude-sonnet-4-6-20250514".to_string(),
        );
        let body = client.build_request_body(
            &[Message::user("inspect")],
            &[ToolSchema {
                name: "fs_read".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }),
                destructive: false,
                parallel_safe: true,
                capability: None,
            }],
        );

        assert_eq!(body["tools"][0]["name"], "fs_read");
        assert_eq!(body["tools"][0]["description"], "Read a file");
        assert!(body["tools"][0]["input_schema"].is_object());
    }

    #[test]
    fn request_body_formats_native_tool_history_for_replay() {
        let client = AnthropicClient::new(
            "https://api.anthropic.com".to_string(),
            "sk-test".to_string(),
            "claude-sonnet-4-6-20250514".to_string(),
        );
        let body = client.build_request_body(
            &[
                Message::assistant_with_tool_calls(
                    "I will inspect that.".to_string(),
                    vec![crate::core::types::ToolCallRef {
                        id: "toolu_1".to_string(),
                        name: "fs_read".to_string(),
                        args: serde_json::json!({ "path": "Cargo.toml" }),
                    }],
                ),
                Message::tool("file contents", Some("toolu_1".to_string())),
            ],
            &[],
        );

        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            "I will inspect that."
        );
        assert_eq!(body["messages"][0]["content"][1]["type"], "tool_use");
        assert_eq!(body["messages"][0]["content"][1]["id"], "toolu_1");
        assert_eq!(body["messages"][0]["content"][1]["name"], "fs_read");
        assert_eq!(
            body["messages"][0]["content"][1]["input"]["path"],
            "Cargo.toml"
        );
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(
            body["messages"][1]["content"][0]["content"],
            "file contents"
        );
    }

    #[test]
    fn anthropic_tool_use_blocks_are_normalized() {
        let mut state = AnthropicToolUseState::default();
        let start = serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "fs_read",
                "input": {}
            }
        })
        .to_string();
        let delta = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"path\":\"Cargo.toml\"}"
            }
        })
        .to_string();
        let stop = serde_json::json!({
            "type": "content_block_stop",
            "index": 0
        })
        .to_string();

        let mut events = normalize_anthropic_event(&mut state, &start).unwrap();
        events.extend(normalize_anthropic_event(&mut state, &delta).unwrap());
        events.extend(normalize_anthropic_event(&mut state, &stop).unwrap());

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseStart { id, name }
                    if id == "toolu_1" && name == "fs_read"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseDone { id, name, args }
                    if id == "toolu_1" && name == "fs_read" && args["path"] == "Cargo.toml"
            )
        }));
    }

    #[test]
    fn classify_error_maps_auth_statuses() {
        assert!(matches!(
            classify_anthropic_error(StatusCode::UNAUTHORIZED, &HeaderMap::new(), "invalid key"),
            ModelError::AuthFailed
        ));
        assert!(matches!(
            classify_anthropic_error(StatusCode::FORBIDDEN, &HeaderMap::new(), "forbidden"),
            ModelError::AuthFailed
        ));
    }

    #[test]
    fn classify_error_reads_retry_after_header() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));

        assert!(matches!(
            classify_anthropic_error(StatusCode::TOO_MANY_REQUESTS, &headers, "slow down"),
            ModelError::RateLimited {
                retry_after_ms: 5000
            }
        ));
    }

    #[test]
    fn classify_error_rate_limit_defaults_to_1000ms() {
        assert!(matches!(
            classify_anthropic_error(
                StatusCode::TOO_MANY_REQUESTS,
                &HeaderMap::new(),
                "slow down"
            ),
            ModelError::RateLimited {
                retry_after_ms: 1000
            }
        ));
    }

    #[test]
    fn classify_error_detects_context_length() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 100000 tokens > 200000 token limit"}}"#;
        assert!(matches!(
            classify_anthropic_error(StatusCode::BAD_REQUEST, &HeaderMap::new(), body),
            ModelError::ContextLengthExceeded { .. }
        ));
    }
}
