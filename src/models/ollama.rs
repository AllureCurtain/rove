use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::StatusCode;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

const DEFAULT_OLLAMA_BASE: &str = "http://localhost:11434";

pub struct OllamaClient {
    client: reqwest::Client,
    api_base: String,
    model: String,
}

impl OllamaClient {
    pub fn new(api_base: String, model: String) -> Self {
        let base = if api_base.is_empty() {
            DEFAULT_OLLAMA_BASE.to_string()
        } else {
            api_base
        };
        Self {
            client: reqwest::Client::new(),
            api_base: base,
            model,
        }
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": ollama_role(&m.role),
                    "content": m.content,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
        });

        if !tools.is_empty() {
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
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        body
    }
}

fn ollama_role(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User | Role::Tool => "user",
        Role::Assistant => "assistant",
    }
}

fn classify_ollama_error(status: StatusCode, body: &str) -> ModelError {
    if status == StatusCode::NOT_FOUND {
        return ModelError::RequestFailed(format!("model not found: {body}"));
    }
    ModelError::RequestFailed(format!("HTTP {}: {}", status, body))
}

#[async_trait]
impl ModelClient for OllamaClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let body = self.build_request_body(messages, tools);
        let url = format!("{}/api/chat", self.api_base);

        Box::pin(async_stream::stream! {
            let response = self
                .client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ModelError::RequestFailed(e.to_string()))?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_ollama_error(status, &text));
                return;
            }

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

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                        let done = json.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

                        if let Some(content) = json
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                            && !content.is_empty()
                        {
                            yield Ok(StreamChunk {
                                delta: content.to_string(),
                                usage: None,
                            });
                        }

                        if done {
                            let usage = Usage {
                                prompt_tokens: json
                                    .get("prompt_eval_count")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as u32,
                                completion_tokens: json
                                    .get("eval_count")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as u32,
                                total_tokens: 0,
                            };
                            let total = usage.prompt_tokens + usage.completion_tokens;
                            yield Ok(StreamChunk {
                                delta: String::new(),
                                usage: Some(Usage { total_tokens: total, ..usage }),
                            });
                            return;
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
    fn request_body_uses_ollama_roles() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
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

        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["model"], "llama3");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn request_body_includes_tools() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
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
    }

    #[test]
    fn default_base_url_is_localhost() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
        assert_eq!(client.api_base, "http://localhost:11434");
    }

    #[test]
    fn classify_error_maps_not_found() {
        let err = classify_ollama_error(StatusCode::NOT_FOUND, "model not found");
        assert!(matches!(err, ModelError::RequestFailed(msg) if msg.contains("model not found")));
    }
}
