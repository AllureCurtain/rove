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

    fn build_request_body(&self, messages: &[Message], _tools: &[ToolSchema]) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect();

        serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
        })
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
