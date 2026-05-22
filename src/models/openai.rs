use async_trait::async_trait;
use futures::stream::BoxStream;

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
                let text = response.text().await.unwrap_or_default();
                if status.as_u16() == 429 {
                    yield Err(ModelError::RateLimited { retry_after_ms: 1000 });
                } else if status.as_u16() == 401 {
                    yield Err(ModelError::AuthFailed);
                } else {
                    yield Err(ModelError::RequestFailed(
                        format!("HTTP {}: {}", status, text),
                    ));
                }
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
}
