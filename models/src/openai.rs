use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{
    StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};
use std::collections::BTreeMap;

use crate::traits::{ModelClient, ModelClientId, ModelEvent};
use crate::{Message, ModelError, ProviderOptions, Role, ToolSchema, Usage};

/// OpenAI-compatible model client.
///
/// Works with OpenAI, DeepSeek, local vLLM, and any OpenAI-compatible endpoint.
/// Uses reqwest directly for streaming (async-openai integration deferred to M1).
pub struct OpenAiClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    options: ProviderOptions,
}

impl OpenAiClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
            options: ProviderOptions::default(),
        }
    }

    pub fn apply_options(&mut self, options: &ProviderOptions) {
        self.options = *options;
    }

    fn build_request_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages.iter().map(format_openai_message).collect();

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

        if let Some(max_tokens) = self.options.max_tokens {
            body.insert(
                "max_tokens".to_string(),
                serde_json::Value::Number(max_tokens.into()),
            );
        }
        insert_float_option(&mut body, "temperature", self.options.temperature);
        insert_float_option(&mut body, "top_p", self.options.top_p);
        insert_float_option(
            &mut body,
            "frequency_penalty",
            self.options.frequency_penalty,
        );
        insert_float_option(&mut body, "presence_penalty", self.options.presence_penalty);

        serde_json::Value::Object(body)
    }
}

fn insert_float_option(
    body: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<f64>,
) {
    if let Some(value) = value
        && let Some(number) = serde_json::Number::from_f64(value)
    {
        body.insert(key.to_string(), serde_json::Value::Number(number));
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

#[derive(Debug, Default)]
struct OpenAiToolCallState {
    calls: BTreeMap<u64, OpenAiPartialToolCall>,
}

#[derive(Debug, Default)]
struct OpenAiPartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    started: bool,
}

fn normalize_openai_chat_chunk(
    state: &mut OpenAiToolCallState,
    data: &str,
) -> serde_json::Result<Vec<ModelEvent>> {
    let json = serde_json::from_str::<serde_json::Value>(data)?;
    let mut events = Vec::new();

    for choice in json
        .get("choices")
        .and_then(|choices| choices.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(|content| content.as_str())
                && !text.is_empty()
            {
                events.push(ModelEvent::TextDelta {
                    text: text.to_string(),
                });
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(|calls| calls.as_array()) {
                for tool_call in tool_calls {
                    let index = tool_call
                        .get("index")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let partial = state.calls.entry(index).or_default();
                    if let Some(id) = tool_call.get("id").and_then(|value| value.as_str()) {
                        partial.id = Some(id.to_string());
                    }
                    if let Some(function) = tool_call.get("function") {
                        if let Some(name) = function.get("name").and_then(|value| value.as_str()) {
                            if !name.is_empty() {
                                partial.name = Some(name.to_string());
                            }
                            if !name.is_empty() && !partial.started {
                                partial.started = true;
                                events.push(ModelEvent::ToolUseStart {
                                    id: openai_tool_call_id(index, partial),
                                    name: name.to_string(),
                                });
                            }
                        }
                        if let Some(args_delta) =
                            function.get("arguments").and_then(|value| value.as_str())
                            && !args_delta.is_empty()
                        {
                            partial.arguments.push_str(args_delta);
                            events.push(ModelEvent::ToolUseDelta {
                                id: openai_tool_call_id(index, partial),
                                args_delta: args_delta.to_string(),
                            });
                        }
                    }
                }
            }
        }

        if choice.get("finish_reason").and_then(|value| value.as_str()) == Some("tool_calls") {
            for (index, partial) in &state.calls {
                if let Some(name) = &partial.name {
                    let args = serde_json::from_str::<serde_json::Value>(&partial.arguments)
                        .unwrap_or_else(|_| serde_json::Value::String(partial.arguments.clone()));
                    events.push(ModelEvent::ToolUseDone {
                        id: openai_tool_call_id(*index, partial),
                        name: name.clone(),
                        args,
                    });
                }
            }
            state.calls.clear();
        }
    }

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
            cached_tokens: 0,
        };
        events.push(ModelEvent::Usage { usage });
    }

    Ok(events)
}

fn normalize_openai_sse_data(
    state: &mut OpenAiToolCallState,
    data: &str,
) -> serde_json::Result<Vec<ModelEvent>> {
    if data.trim() == "[DONE]" {
        return Ok(vec![ModelEvent::Done]);
    }

    normalize_openai_chat_chunk(state, data)
}

fn openai_tool_call_id(index: u64, partial: &OpenAiPartialToolCall) -> String {
    partial
        .id
        .clone()
        .unwrap_or_else(|| format!("tool_call_{index}"))
}

fn format_openai_message(m: &Message) -> serde_json::Value {
    match m.role {
        Role::Assistant if !m.tool_calls.is_empty() => {
            let tool_calls: Vec<serde_json::Value> = m
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.args.to_string(),
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
            let mut msg = serde_json::json!({
                "role": "tool",
                "content": m.content,
            });
            if let Some(ref id) = m.tool_call_id {
                msg["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            msg
        }
        Role::Tool => {
            serde_json::json!({
                "role": "user",
                "content": m.content,
            })
        }
        _ => {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        }
    }
}

#[async_trait]
impl ModelClient for OpenAiClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
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
            let mut tool_call_state = OpenAiToolCallState::default();

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

                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(events) = normalize_openai_sse_data(&mut tool_call_state, data)
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
        ModelClientId::new("openai-compatible", &self.api_base, &self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Message, ProviderOptions, ToolCallRef, ToolSchema};
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
        assert_eq!(body["tools"][0]["function"]["description"], "Read a file");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn request_body_includes_provider_options() {
        let mut client = OpenAiClient::new(
            "https://example.invalid/v1".to_string(),
            "secret".to_string(),
            "gpt-4o".to_string(),
        );
        client.apply_options(&ProviderOptions {
            max_tokens: Some(2048),
            temperature: Some(0.2),
            top_p: Some(0.8),
            frequency_penalty: Some(0.3),
            presence_penalty: Some(0.4),
        });

        let body = client.build_request_body(&[Message::user("inspect")], &[]);

        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.8);
        assert_eq!(body["frequency_penalty"], 0.3);
        assert_eq!(body["presence_penalty"], 0.4);
    }

    #[test]
    fn legacy_tool_result_without_id_falls_back_to_user_message() {
        let msg = format_openai_message(&Message::tool("plain parsed tool output", None));

        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"], "plain parsed tool output");
        assert!(msg.get("tool_call_id").is_none());
    }

    #[test]
    fn request_body_formats_native_tool_history_for_replay() {
        let client = OpenAiClient::new(
            "https://example.invalid/v1".to_string(),
            "secret".to_string(),
            "gpt-4o".to_string(),
        );
        let body = client.build_request_body(
            &[
                Message::assistant_with_tool_calls(
                    "I will inspect that.".to_string(),
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

        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"], "I will inspect that.");
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(body["messages"][0]["tool_calls"][0]["type"], "function");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["name"],
            "fs_read"
        );
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"Cargo.toml\"}"
        );
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "call_1");
        assert_eq!(body["messages"][1]["content"], "file contents");
    }

    #[test]
    fn openai_delta_tool_call_events_are_normalized() {
        let mut state = OpenAiToolCallState::default();
        let first = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "fs_read",
                            "arguments": "{\"path\""
                        }
                    }]
                }
            }]
        })
        .to_string();
        let second = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {
                            "arguments": ":\"Cargo.toml\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();

        let mut events = normalize_openai_chat_chunk(&mut state, &first).unwrap();
        events.extend(normalize_openai_chat_chunk(&mut state, &second).unwrap());

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseStart { id, name }
                    if id == "call_1" && name == "fs_read"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseDone { id, name, args }
                    if id == "call_1" && name == "fs_read" && args["path"] == "Cargo.toml"
            )
        }));
    }

    #[test]
    fn openai_tool_call_empty_name_deltas_do_not_overwrite_started_name() {
        let mut state = OpenAiToolCallState::default();
        let first = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "echo",
                            "arguments": ""
                        }
                    }]
                }
            }]
        })
        .to_string();
        let second = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": null,
                        "type": null,
                        "function": {
                            "name": "",
                            "arguments": "{\"message\":\"hello\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();

        let mut events = normalize_openai_chat_chunk(&mut state, &first).unwrap();
        events.extend(normalize_openai_chat_chunk(&mut state, &second).unwrap());

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::ToolUseDone { id, name, args }
                    if id == "call_1" && name == "echo" && args["message"] == "hello"
            )
        }));
    }

    #[test]
    fn openai_done_sentinel_is_normalized() {
        let mut state = OpenAiToolCallState::default();
        let events = normalize_openai_sse_data(&mut state, "[DONE]").unwrap();

        assert_eq!(events, vec![ModelEvent::Done]);
    }

    #[test]
    fn client_id_includes_provider_endpoint_and_model() {
        let left = OpenAiClient::new(
            "https://primary.test/v1".to_string(),
            "key".to_string(),
            "same-model".to_string(),
        );
        let right = OpenAiClient::new(
            "https://fallback.test/v1".to_string(),
            "key".to_string(),
            "same-model".to_string(),
        );

        assert_ne!(left.client_id(), right.client_id());
        assert_eq!(left.model_id(), right.model_id());
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
