use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::StatusCode;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

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

        let msgs: Vec<serde_json::Value> = conversation
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": anthropic_role(&m.role),
                    "content": m.content,
                })
            })
            .collect();

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

fn classify_anthropic_error(status: StatusCode, body: &str) -> ModelError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelError::AuthFailed;
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelError::RateLimited {
            retry_after_ms: 1000,
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

#[async_trait]
impl ModelClient for AnthropicClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
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
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_anthropic_error(status, &text));
                return;
            }

            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut usage = Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            };

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
                        && let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
                    {
                        let event_type = json.get("type").and_then(|v| v.as_str());

                        match event_type {
                            Some("content_block_delta") => {
                                if let Some(text) = json
                                    .get("delta")
                                    .and_then(|d| d.get("text"))
                                    .and_then(|t| t.as_str())
                                {
                                    yield Ok(StreamChunk {
                                        delta: text.to_string(),
                                        usage: None,
                                    });
                                }
                            }
                            Some("message_start") => {
                                if let Some(u) = json
                                    .get("message")
                                    .and_then(|m| m.get("usage"))
                                {
                                    usage.prompt_tokens = u
                                        .get("input_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                }
                            }
                            Some("message_delta") => {
                                if let Some(u) = json.get("usage") {
                                    usage.completion_tokens = u
                                        .get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                    usage.total_tokens =
                                        usage.prompt_tokens + usage.completion_tokens;
                                }
                            }
                            Some("message_stop") => {
                                yield Ok(StreamChunk {
                                    delta: String::new(),
                                    usage: Some(usage.clone()),
                                });
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Message, Role, ToolSchema};

    #[test]
    fn request_body_separates_system_message() {
        let client = AnthropicClient::new(
            "https://api.anthropic.com".to_string(),
            "sk-test".to_string(),
            "claude-sonnet-4-6-20250514".to_string(),
        );
        let body = client.build_request_body(
            &[
                Message {
                    role: Role::System,
                    content: "You are helpful.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
            ],
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
            &[Message {
                role: Role::User,
                content: "inspect".to_string(),
            }],
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
            }],
        );

        assert_eq!(body["tools"][0]["name"], "fs_read");
        assert_eq!(body["tools"][0]["description"], "Read a file");
        assert!(body["tools"][0]["input_schema"].is_object());
    }

    #[test]
    fn classify_error_maps_auth_statuses() {
        assert!(matches!(
            classify_anthropic_error(StatusCode::UNAUTHORIZED, "invalid key"),
            ModelError::AuthFailed
        ));
        assert!(matches!(
            classify_anthropic_error(StatusCode::FORBIDDEN, "forbidden"),
            ModelError::AuthFailed
        ));
    }

    #[test]
    fn classify_error_maps_rate_limit() {
        assert!(matches!(
            classify_anthropic_error(StatusCode::TOO_MANY_REQUESTS, "slow down"),
            ModelError::RateLimited { .. }
        ));
    }

    #[test]
    fn classify_error_detects_context_length() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 100000 tokens > 200000 token limit"}}"#;
        assert!(matches!(
            classify_anthropic_error(StatusCode::BAD_REQUEST, body),
            ModelError::ContextLengthExceeded { .. }
        ));
    }
}
