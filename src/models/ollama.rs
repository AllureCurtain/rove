use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::StatusCode;

use crate::core::types::{Message, Role, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelEvent};

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
            .map(|m| format_ollama_message(m))
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

fn format_ollama_message(m: &Message) -> serde_json::Value {
    match m.role {
        Role::Assistant if !m.tool_calls.is_empty() => {
            let tool_calls: Vec<serde_json::Value> = m
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "function": {
                            "name": tc.name,
                            "arguments": tc.args,
                        }
                    })
                })
                .collect();
            let mut msg = serde_json::json!({
                "role": "assistant",
                "tool_calls": tool_calls,
            });
            if !m.content.is_empty() {
                msg["content"] = serde_json::Value::String(m.content.clone());
            }
            msg
        }
        Role::Tool => {
            serde_json::json!({
                "role": "tool",
                "content": m.content,
            })
        }
        _ => {
            serde_json::json!({
                "role": ollama_role(&m.role),
                "content": m.content,
            })
        }
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

fn normalize_ollama_chat_line(line: &str) -> serde_json::Result<Vec<ModelEvent>> {
    let json = serde_json::from_str::<serde_json::Value>(line)?;
    let mut events = Vec::new();

    if let Some(content) = json
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        && !content.is_empty()
    {
        events.push(ModelEvent::TextDelta {
            text: content.to_string(),
        });
    }

    if let Some(tool_calls) = json
        .get("message")
        .and_then(|message| message.get("tool_calls"))
        .and_then(|tool_calls| tool_calls.as_array())
    {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            if let Some(function) = tool_call.get("function") {
                let name = function
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let id = format!("ollama_tool_call_{index}");
                let args = function
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                events.push(ModelEvent::ToolUseStart {
                    id: id.clone(),
                    name: name.clone(),
                });
                events.push(ModelEvent::ToolUseDone { id, name, args });
            }
        }
    }

    if json.get("done").and_then(|value| value.as_bool()) == Some(true) {
        let usage = Usage {
            prompt_tokens: json
                .get("prompt_eval_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: json
                .get("eval_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            total_tokens: 0,
        };
        let total_tokens = usage.prompt_tokens + usage.completion_tokens;
        events.push(ModelEvent::Usage {
            usage: Usage {
                total_tokens,
                ..usage
            },
        });
        events.push(ModelEvent::Done);
    }

    Ok(events)
}

#[async_trait]
impl ModelClient for OllamaClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
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

                    if let Ok(events) = normalize_ollama_chat_line(&line) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Message, ToolSchema};

    #[test]
    fn request_body_uses_ollama_roles() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
        let body = client.build_request_body(
            &[
                Message::system("You are helpful."),
                Message::user("Hello"),
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
            }],
        );

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "fs_read");
    }

    #[test]
    fn ollama_tool_calls_are_normalized() {
        let line = serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "fs_read",
                        "arguments": { "path": "Cargo.toml" }
                    }
                }]
            },
            "done": false
        })
        .to_string();

        let events = normalize_ollama_chat_line(&line).unwrap();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseStart { name, .. } if name == "fs_read"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseDone { name, args, .. }
                    if name == "fs_read" && args["path"] == "Cargo.toml"
            )
        }));
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
