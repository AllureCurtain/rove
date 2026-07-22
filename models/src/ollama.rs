use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{
    StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};

use crate::traits::{ModelClient, ModelClientId, ModelEvent};
use crate::{Message, ModelError, ProviderOptions, Role, ToolSchema, Usage};

const DEFAULT_OLLAMA_BASE: &str = "http://localhost:11434";

pub struct OllamaClient {
    client: reqwest::Client,
    api_base: String,
    model: String,
    options: OllamaRequestOptions,
}

#[derive(Debug, Clone, Copy, Default)]
struct OllamaRequestOptions {
    num_predict: Option<u32>,
    temperature: Option<f64>,
    top_p: Option<f64>,
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
            options: OllamaRequestOptions::default(),
        }
    }

    pub fn apply_options(&mut self, provider_options: &ProviderOptions) {
        self.options.num_predict = provider_options.max_tokens;
        self.options.temperature = provider_options.temperature;
        self.options.top_p = provider_options.top_p;
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages.iter().map(format_ollama_message).collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
        });

        let options = self.request_options_json();
        if !options.as_object().is_some_and(|object| object.is_empty()) {
            body["options"] = options;
        }

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

    fn request_options_json(&self) -> serde_json::Value {
        let mut options = serde_json::json!({});
        if let Some(num_predict) = self.options.num_predict {
            options["num_predict"] = serde_json::Value::Number(num_predict.into());
        }
        if let Some(temperature) = self.options.temperature
            && let Some(number) = serde_json::Number::from_f64(temperature)
        {
            options["temperature"] = serde_json::Value::Number(number);
        }
        if let Some(top_p) = self.options.top_p
            && let Some(number) = serde_json::Number::from_f64(top_p)
        {
            options["top_p"] = serde_json::Value::Number(number);
        }
        options
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
        Role::Tool if m.tool_call_id.is_some() => {
            serde_json::json!({
                "role": "tool",
                "content": m.content,
            })
        }
        Role::Tool => {
            serde_json::json!({
                "role": "user",
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

fn parse_retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1000))
}

fn classify_ollama_error(status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
    match status {
        StatusCode::NOT_FOUND => ModelError::RequestFailed(format!("model not found: {body}")),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ModelError::AuthFailed,
        StatusCode::TOO_MANY_REQUESTS => ModelError::RateLimited {
            retry_after_ms: parse_retry_after_ms(headers).unwrap_or(1000),
        },
        StatusCode::BAD_REQUEST if is_ollama_context_length_error(body) => {
            ModelError::ContextLengthExceeded { used: 0, max: 0 }
        }
        _ => ModelError::RequestFailed(format!("HTTP {}: {}", status, body)),
    }
}

fn is_ollama_context_length_error(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    (lower.contains("context") && lower.contains("token"))
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("prompt is too long")
        || lower.contains("input too long")
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
            cached_tokens: 0,
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
                let headers = response.headers().clone();
                let text = response.text().await.unwrap_or_default();
                yield Err(classify_ollama_error(status, &headers, &text));
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

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("ollama", &self.api_base, &self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ProviderOptions, ToolCallRef, ToolSchema};
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    #[test]
    fn request_body_uses_ollama_roles() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
        let body = client.build_request_body(
            &[Message::system("You are helpful."), Message::user("Hello")],
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
            }],
        );

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "fs_read");
    }

    #[test]
    fn request_body_includes_provider_options() {
        let mut client = OllamaClient::new(String::new(), "llama3".to_string());
        client.apply_options(&ProviderOptions {
            max_tokens: Some(2048),
            temperature: Some(0.8),
            top_p: Some(0.9),
            ..Default::default()
        });

        let body = client.build_request_body(&[Message::user("hi")], &[]);

        assert_eq!(body["options"]["num_predict"], 2048);
        assert_eq!(body["options"]["temperature"], 0.8);
        assert_eq!(body["options"]["top_p"], 0.9);
    }

    #[test]
    fn legacy_tool_result_without_id_falls_back_to_user_message() {
        let msg = format_ollama_message(&Message::tool("plain parsed tool output", None));

        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"], "plain parsed tool output");
        assert!(msg.get("tool_calls").is_none());
    }

    #[test]
    fn request_body_formats_native_tool_history_for_replay() {
        let client = OllamaClient::new(String::new(), "llama3".to_string());
        let body = client.build_request_body(
            &[
                Message::assistant_with_tool_calls(
                    "I will inspect that.".to_string(),
                    vec![ToolCallRef {
                        id: "ollama_tool_call_0".to_string(),
                        name: "fs_read".to_string(),
                        args: serde_json::json!({ "path": "Cargo.toml" }),
                    }],
                ),
                Message::tool("file contents", Some("ollama_tool_call_0".to_string())),
            ],
            &[],
        );

        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"], "I will inspect that.");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["name"],
            "fs_read"
        );
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"]["path"],
            "Cargo.toml"
        );
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["content"], "file contents");
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
    fn classify_error_maps_auth_to_auth_failed() {
        assert!(matches!(
            classify_ollama_error(StatusCode::UNAUTHORIZED, &HeaderMap::new(), "unauthorized"),
            ModelError::AuthFailed
        ));
        assert!(matches!(
            classify_ollama_error(StatusCode::FORBIDDEN, &HeaderMap::new(), "forbidden"),
            ModelError::AuthFailed
        ));
    }

    #[test]
    fn classify_error_maps_rate_limit_with_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));

        assert!(matches!(
            classify_ollama_error(StatusCode::TOO_MANY_REQUESTS, &headers, "slow down"),
            ModelError::RateLimited {
                retry_after_ms: 2000
            }
        ));
    }

    #[test]
    fn classify_error_maps_context_length() {
        let body = r#"{"error":"context window exceeded: input too long"}"#;

        assert!(matches!(
            classify_ollama_error(StatusCode::BAD_REQUEST, &HeaderMap::new(), body),
            ModelError::ContextLengthExceeded { .. }
        ));
    }

    #[test]
    fn classify_error_maps_not_found() {
        let err =
            classify_ollama_error(StatusCode::NOT_FOUND, &HeaderMap::new(), "model not found");
        assert!(matches!(err, ModelError::RequestFailed(msg) if msg.contains("model not found")));
    }
}
