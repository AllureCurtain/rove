use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{
    StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};

use crate::core::types::{Message, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

/// OpenAI-compatible model client.
///
/// Works with OpenAI, DeepSeek, local vLLM, and any OpenAI-compatible endpoint.
/// Uses reqwest directly for streaming (async-openai integration deferred to M1).
pub struct OpenAiClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
        }
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect();

        let tool_defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();

        if !tool_defs.is_empty() {
            body.insert("tools".to_string(), serde_json::Value::Array(tool_defs));
            body.insert(
                "tool_choice".to_string(),
                serde_json::Value::String("auto".to_string()),
            );
        }

        serde_json::Value::Object(body)
    }
}

fn classify_http_error(status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelError::AuthFailed;
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelError::RateLimited {
            retry_after_ms: retry_after_ms(headers).unwrap_or(1000),
        };
    }

    if is_context_length_error(body) {
        let (used, max) = context_length_token_counts(body);
        return ModelError::ContextLengthExceeded { used, max };
    }

    ModelError::RequestFailed(format!("HTTP {}: {}", status, body))
}

fn retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1000))
}

fn is_context_length_error(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("maximum context length")
        || lower.contains("context length exceeded")
}

fn context_length_token_counts(body: &str) -> (u32, u32) {
    let text = provider_error_message(body).unwrap_or_else(|| body.to_string());
    let lower = text.to_ascii_lowercase();
    let numbers = unsigned_numbers(&text);

    if lower.contains("maximum context length")
        && lower.contains("resulted in")
        && numbers.len() >= 2
    {
        return (numbers[1], numbers[0]);
    }

    (0, 0)
}

fn provider_error_message(body: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(body).ok()?;
    json.get("error")?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

fn unsigned_numbers(text: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(number) = current.parse::<u32>() {
                numbers.push(number);
            }
            current.clear();
        }
    }

    if !current.is_empty()
        && let Ok(number) = current.parse::<u32>()
    {
        numbers.push(number);
    }

    numbers
}

#[async_trait]
impl ModelClient for OpenAiClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let body = self.build_request_body(messages, tools);
        let url = format!("{}/chat/completions", self.api_base);
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
                .map_err(|e| ModelError::RequestFailed(e.to_string()))?;

            if !response.status().is_success() {
                let status = response.status();
                let headers = response.headers().clone();
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_http_error(status, &headers, &text));
                return;
            }

            // Parse SSE stream
            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ModelError::StreamInterrupted(e.to_string()));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE lines
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            return;
                        }

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                            // Extract delta content
                            if let Some(delta) = json
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("delta"))
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                yield Ok(StreamChunk {
                                    delta: delta.to_string(),
                                    usage: None,
                                });
                            }

                            // Check for usage in final chunk
                            if let Some(usage_obj) = json.get("usage") {
                                let usage = Usage {
                                    prompt_tokens: usage_obj
                                        .get("prompt_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32,
                                    completion_tokens: usage_obj
                                        .get("completion_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32,
                                    total_tokens: usage_obj
                                        .get("total_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32,
                                };
                                yield Ok(StreamChunk {
                                    delta: String::new(),
                                    usage: Some(usage),
                                });
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::types::{Message, Role, ToolSchema};
    use reqwest::{
        StatusCode,
        header::{HeaderMap, HeaderValue, RETRY_AFTER},
    };

    #[test]
    fn request_body_includes_tool_schemas_when_present() {
        let client = OpenAiClient::new(
            "https://example.invalid/v1".to_string(),
            "secret".to_string(),
            "gpt-4o".to_string(),
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

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "fs_read");
        assert_eq!(body["tools"][0]["function"]["description"], "Read a file");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn classify_http_error_maps_auth_statuses() {
        assert!(matches!(
            classify_http_error(StatusCode::UNAUTHORIZED, &HeaderMap::new(), "bad key"),
            ModelError::AuthFailed
        ));
        assert!(matches!(
            classify_http_error(StatusCode::FORBIDDEN, &HeaderMap::new(), "forbidden"),
            ModelError::AuthFailed
        ));
    }

    #[test]
    fn classify_http_error_uses_retry_after_for_rate_limits() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));

        assert!(matches!(
            classify_http_error(StatusCode::TOO_MANY_REQUESTS, &headers, "rate limited"),
            ModelError::RateLimited {
                retry_after_ms: 3000
            }
        ));
    }

    #[test]
    fn classify_http_error_detects_context_length_errors() {
        let body = serde_json::json!({
            "error": {
                "message": "This model's maximum context length was exceeded.",
                "type": "invalid_request_error",
                "code": "context_length_exceeded"
            }
        })
        .to_string();

        assert!(matches!(
            classify_http_error(StatusCode::BAD_REQUEST, &HeaderMap::new(), &body),
            ModelError::ContextLengthExceeded { .. }
        ));
    }

    #[test]
    fn classify_http_error_extracts_context_length_token_counts() {
        let body = serde_json::json!({
            "error": {
                "message": "This model's maximum context length is 8192 tokens. However, your messages resulted in 9001 tokens.",
                "type": "invalid_request_error",
                "code": "context_length_exceeded"
            }
        })
        .to_string();

        assert!(matches!(
            classify_http_error(StatusCode::BAD_REQUEST, &HeaderMap::new(), &body),
            ModelError::ContextLengthExceeded {
                used: 9001,
                max: 8192
            }
        ));
    }
}
