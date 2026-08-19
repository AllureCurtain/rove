use std::collections::BTreeMap;

use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER},
};

use crate::provider::{
    AuthStyle, Framing, OPENAI_COMPLETIONS_PROTOCOL, StreamDecoder, WireProtocol, WireProtocolId,
    WireRequest, WireRequestInput,
};
use crate::{Message, ModelError, ModelEvent, ModelToolSchema, Role, StopReason, Usage};

pub struct OpenAiCompletionsProtocol {
    id: WireProtocolId,
}

impl OpenAiCompletionsProtocol {
    pub fn new() -> Self {
        Self {
            id: WireProtocolId::new(OPENAI_COMPLETIONS_PROTOCOL)
                .expect("the built-in OpenAI Completions protocol id is valid"),
        }
    }
}

impl Default for OpenAiCompletionsProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl WireProtocol for OpenAiCompletionsProtocol {
    fn id(&self) -> &WireProtocolId {
        &self.id
    }

    fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
        let messages = input
            .messages
            .iter()
            .map(format_openai_message)
            .collect::<Vec<_>>();
        let tools = input
            .tools
            .iter()
            .map(format_openai_tool)
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "stream": true,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();

        if !tools.is_empty() {
            body.insert("tools".to_string(), serde_json::Value::Array(tools));
            body.insert(
                "tool_choice".to_string(),
                serde_json::Value::String("auto".to_string()),
            );
        }
        if let Some(max_tokens) = input.options.max_tokens {
            body.insert(
                "max_tokens".to_string(),
                serde_json::Value::Number(max_tokens.into()),
            );
        }
        insert_float_option(&mut body, "temperature", input.options.temperature);
        insert_float_option(&mut body, "top_p", input.options.top_p);
        insert_float_option(
            &mut body,
            "frequency_penalty",
            input.options.frequency_penalty,
        );
        insert_float_option(
            &mut body,
            "presence_penalty",
            input.options.presence_penalty,
        );

        Ok(WireRequest {
            method: Method::POST,
            path: "chat/completions".to_string(),
            headers: HeaderMap::from_iter([(
                CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )]),
            body: serde_json::Value::Object(body),
        })
    }

    fn framing(&self) -> Framing {
        Framing::ServerSentEvents
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(OpenAiChatDecoder::default())
    }

    fn classify_error(&self, status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
        classify_http_error(status, headers, body)
    }

    fn default_auth_style(&self) -> AuthStyle {
        AuthStyle::Bearer
    }

    fn capabilities(&self) -> crate::ProviderCapabilities {
        crate::ProviderCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
        }
    }
}

fn format_openai_tool(tool: &ModelToolSchema) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

fn format_openai_message(message: &Message) -> serde_json::Value {
    match message.role {
        Role::Assistant if !message.tool_calls.is_empty() => {
            let tool_calls = message
                .tool_calls
                .iter()
                .map(|tool_call| {
                    serde_json::json!({
                        "id": tool_call.id,
                        "type": "function",
                        "function": {
                            "name": tool_call.name,
                            "arguments": tool_call.args.to_string(),
                        }
                    })
                })
                .collect::<Vec<_>>();
            let mut formatted = serde_json::json!({
                "role": "assistant",
                "tool_calls": tool_calls,
            });
            if !message.content.is_empty() {
                formatted["content"] = serde_json::Value::String(message.content.clone());
            }
            formatted
        }
        Role::Tool if message.tool_call_id.is_some() => {
            serde_json::json!({
                "role": "tool",
                "content": message.content,
                "tool_call_id": message.tool_call_id,
            })
        }
        Role::Tool => serde_json::json!({
            "role": "user",
            "content": message.content,
        }),
        _ => serde_json::json!({
            "role": message.role,
            "content": message.content,
        }),
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

#[derive(Debug, Default)]
struct OpenAiChatDecoder {
    calls: BTreeMap<u64, OpenAiPartialToolCall>,
    usage: Option<Usage>,
}

#[derive(Debug, Default)]
struct OpenAiPartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    started: bool,
}

impl StreamDecoder for OpenAiChatDecoder {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
        if frame.trim() == "[DONE]" {
            let mut events = self.take_usage();
            events.push(ModelEvent::Done);
            return Ok(events);
        }
        let json = serde_json::from_str::<serde_json::Value>(frame).map_err(|_| {
            ModelError::StreamInterrupted("OpenAI Chat stream frame is invalid JSON".to_string())
        })?;
        if let Some(usage) = parse_chat_usage(&json) {
            // OpenAI-compatible gateways may repeat a cumulative usage
            // snapshot on every chunk. Publish only the final snapshot for a
            // model call so Runtime accounting cannot multiply token totals.
            self.usage = Some(usage);
        }
        Ok(normalize_chat_chunk(&mut self.calls, &json))
    }

    fn finish(&mut self) -> Result<Vec<ModelEvent>, ModelError> {
        Ok(self.take_usage())
    }
}

impl OpenAiChatDecoder {
    fn take_usage(&mut self) -> Vec<ModelEvent> {
        self.usage
            .take()
            .map(|usage| vec![ModelEvent::Usage { usage }])
            .unwrap_or_default()
    }
}

fn normalize_chat_chunk(
    calls: &mut BTreeMap<u64, OpenAiPartialToolCall>,
    json: &serde_json::Value,
) -> Vec<ModelEvent> {
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
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|items| items.as_array()) {
                for tool_call in tool_calls {
                    let index = tool_call
                        .get("index")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let partial = calls.entry(index).or_default();
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
                                    id: tool_call_id(index, partial),
                                    name: name.to_string(),
                                });
                            }
                        }
                        if let Some(arguments) =
                            function.get("arguments").and_then(|value| value.as_str())
                            && !arguments.is_empty()
                        {
                            partial.arguments.push_str(arguments);
                            events.push(ModelEvent::ToolUseDelta {
                                id: tool_call_id(index, partial),
                                args_delta: arguments.to_string(),
                            });
                        }
                    }
                }
            }
        }
        if let Some(finish_reason) = choice.get("finish_reason").and_then(|value| value.as_str()) {
            if finish_reason == "tool_calls" {
                for (index, partial) in calls.iter() {
                    if let Some(name) = &partial.name {
                        let args = serde_json::from_str::<serde_json::Value>(&partial.arguments)
                            .unwrap_or_else(|_| {
                                serde_json::Value::String(partial.arguments.clone())
                            });
                        events.push(ModelEvent::ToolUseDone {
                            id: tool_call_id(*index, partial),
                            name: name.clone(),
                            args,
                        });
                    }
                }
                calls.clear();
            }
            events.push(ModelEvent::StopReason {
                reason: openai_stop_reason(finish_reason),
            });
        }
    }

    events
}

fn parse_chat_usage(json: &serde_json::Value) -> Option<Usage> {
    let usage = json.get("usage")?;
    Some(Usage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
        total_tokens: usage
            .get("total_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
        cached_tokens: 0,
    })
}

fn openai_stop_reason(value: &str) -> StopReason {
    match value {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        "content_filter" => StopReason::ContentFilter,
        other => StopReason::Other(other.to_string()),
    }
}

fn tool_call_id(index: u64, partial: &OpenAiPartialToolCall) -> String {
    partial
        .id
        .clone()
        .unwrap_or_else(|| format!("tool_call_{index}"))
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
    ModelError::RequestFailed(format!("HTTP {status}: {body}"))
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
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

fn unsigned_numbers(text: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_ascii_digit() {
            current.push(character);
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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use futures::StreamExt;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::sleep,
    };

    use super::*;
    use crate::openai::{
        OpenAiClient, OpenAiToolCallState, classify_http_error as legacy_classify_http_error,
        normalize_openai_sse_data,
    };
    use crate::provider::{
        ProviderClient, ProviderClientConfig, ResolvedAuth, Transport, TransportConfig,
    };
    use crate::{ModelClient, ProviderOptions, ToolCallRef};

    #[test]
    fn maps_openai_finish_reasons_to_normalized_stop_reasons() {
        for (wire, expected) in [
            ("stop", StopReason::EndTurn),
            ("tool_calls", StopReason::ToolUse),
            ("length", StopReason::MaxTokens),
            ("content_filter", StopReason::ContentFilter),
        ] {
            let mut decoder = OpenAiCompletionsProtocol::new().decoder();
            let events = decoder
                .push(
                    &serde_json::json!({
                        "choices": [{"delta": {}, "finish_reason": wire}]
                    })
                    .to_string(),
                )
                .unwrap();
            assert!(events.iter().any(
                |event| matches!(event, ModelEvent::StopReason { reason } if reason == &expected)
            ));
        }
    }

    #[test]
    fn request_contains_native_tools_options_and_history() {
        let protocol = OpenAiCompletionsProtocol::new();
        let messages = [
            Message::assistant_with_tool_calls(
                "checking".to_string(),
                vec![ToolCallRef {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    args: serde_json::json!({"path":"Cargo.toml"}),
                }],
            ),
            Message::tool("contents", Some("call_1".to_string())),
        ];
        let tools = [ModelToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }];
        let options = ProviderOptions {
            max_tokens: Some(2048),
            temperature: Some(0.2),
            top_p: Some(0.8),
            frequency_penalty: Some(0.3),
            presence_penalty: Some(0.4),
        };
        let request = protocol
            .build_request(&WireRequestInput {
                model: "gpt-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap();

        assert_eq!(request.path, "chat/completions");
        assert_eq!(request.body["model"], "gpt-test");
        assert_eq!(request.body["messages"][0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(request.body["messages"][1]["tool_call_id"], "call_1");
        assert_eq!(request.body["tools"][0]["function"]["name"], "read_file");
        assert_eq!(request.body["max_tokens"], 2048);
        assert_eq!(request.body["temperature"], 0.2);
        assert_eq!(request.body["top_p"], 0.8);
        assert_eq!(request.body["frequency_penalty"], 0.3);
        assert_eq!(request.body["presence_penalty"], 0.4);
    }

    #[test]
    fn request_body_matches_legacy_openai_completions_client() {
        let messages = [
            Message::system("follow policy"),
            Message::user("inspect"),
            Message::assistant_with_tool_calls(
                "checking".to_string(),
                vec![ToolCallRef {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    args: serde_json::json!({"path":"Cargo.toml"}),
                }],
            ),
            Message::tool("contents", Some("call_1".to_string())),
            Message::tool("legacy output", None),
        ];
        let tools = [ModelToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        }];
        let options = ProviderOptions {
            max_tokens: Some(2048),
            temperature: Some(0.2),
            top_p: Some(0.8),
            frequency_penalty: Some(0.3),
            presence_penalty: Some(0.4),
        };
        let mut legacy = OpenAiClient::new(
            "https://example.test/v1".to_string(),
            "test-secret".to_string(),
            "gpt-test".to_string(),
        );
        legacy.apply_options(&options);
        let legacy_body = legacy.build_request_body(&messages, &tools);
        let migrated_body = OpenAiCompletionsProtocol::new()
            .build_request(&WireRequestInput {
                model: "gpt-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap()
            .body;

        assert_eq!(migrated_body, legacy_body);
    }

    #[test]
    fn decoder_normalizes_fragmented_tool_calls_usage_and_done() {
        let protocol = OpenAiCompletionsProtocol::new();
        let mut decoder = protocol.decoder();
        let frames = [
            serde_json::json!({"choices":[{"delta":{"content":"hello"}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":"{\"message\""}}]}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"ok\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}).to_string(),
            "[DONE]".to_string(),
        ];
        let mut events = Vec::new();
        for frame in frames {
            events.extend(decoder.push(&frame).unwrap());
        }

        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseStart { id, name } if id == "call_1" && name == "echo")));
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { id, name, args } if id == "call_1" && name == "echo" && args["message"] == "ok")));
        assert!(
            events.iter().any(
                |event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 5)
            )
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }

    #[test]
    fn decoder_publishes_only_the_final_cumulative_usage_snapshot() {
        let mut decoder = OpenAiCompletionsProtocol::new().decoder();
        let frames = [
            serde_json::json!({
                "choices": [{"delta":{"content":"one"}}],
                "usage": {"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}
            })
            .to_string(),
            serde_json::json!({
                "choices": [{"delta":{"content":"two"},"finish_reason":"stop"}],
                "usage": {"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}
            })
            .to_string(),
            "[DONE]".to_string(),
        ];
        let mut events = Vec::new();
        for frame in frames {
            events.extend(decoder.push(&frame).unwrap());
        }

        let usage = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::Usage { usage } => Some(usage),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].total_tokens, 12);
        assert!(matches!(events.last(), Some(ModelEvent::Done)));
    }

    #[test]
    fn decoder_events_match_legacy_openai_completions_client() {
        let frames = [
            serde_json::json!({"choices":[{"delta":{"content":"hello"}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":"{\"message\""}}]}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":":\"ok\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}).to_string(),
            "[DONE]".to_string(),
        ];
        let mut legacy_state = OpenAiToolCallState::default();
        let mut migrated_decoder = OpenAiCompletionsProtocol::new().decoder();
        let mut legacy_events = Vec::new();
        let mut migrated_events = Vec::new();

        for frame in &frames {
            legacy_events.extend(normalize_openai_sse_data(&mut legacy_state, frame).unwrap());
            migrated_events.extend(migrated_decoder.push(frame).unwrap());
        }

        let legacy_projection = migrated_events
            .iter()
            .filter(|event| !matches!(event, ModelEvent::StopReason { .. }))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(legacy_projection, legacy_events);
        assert!(migrated_events.iter().any(|event| matches!(
            event,
            ModelEvent::StopReason {
                reason: StopReason::ToolUse
            }
        )));
        assert!(migrated_events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { id, name, args } if id == "call_1" && name == "echo" && args["message"] == "ok")));
        assert!(
            migrated_events.iter().any(
                |event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 5)
            )
        );
        assert!(
            migrated_events
                .iter()
                .any(|event| matches!(event, ModelEvent::Done))
        );
    }

    #[test]
    fn http_error_classification_matches_legacy_openai_completions_client() {
        let mut rate_limit_headers = HeaderMap::new();
        rate_limit_headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));
        let context_body = serde_json::json!({
            "error": {
                "message": "This model's maximum context length is 8192 tokens. However, your messages resulted in 9001 tokens.",
                "code": "context_length_exceeded"
            }
        })
        .to_string();
        let cases = [
            (StatusCode::UNAUTHORIZED, HeaderMap::new(), "bad key"),
            (
                StatusCode::TOO_MANY_REQUESTS,
                rate_limit_headers,
                "rate limited",
            ),
            (
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                context_body.as_str(),
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                HeaderMap::new(),
                "temporarily unavailable",
            ),
        ];
        let protocol = OpenAiCompletionsProtocol::new();

        for (status, headers, body) in cases {
            let legacy = legacy_classify_http_error(status, &headers, body);
            let migrated = protocol.classify_error(status, &headers, body);
            assert_model_error_parity(migrated, legacy);
        }
    }

    fn assert_model_error_parity(migrated: ModelError, legacy: ModelError) {
        match (migrated, legacy) {
            (ModelError::AuthFailed, ModelError::AuthFailed) => {}
            (
                ModelError::RateLimited {
                    retry_after_ms: migrated,
                },
                ModelError::RateLimited {
                    retry_after_ms: legacy,
                },
            ) => assert_eq!(migrated, legacy),
            (
                ModelError::ContextLengthExceeded {
                    used: migrated_used,
                    max: migrated_max,
                },
                ModelError::ContextLengthExceeded {
                    used: legacy_used,
                    max: legacy_max,
                },
            ) => assert_eq!((migrated_used, migrated_max), (legacy_used, legacy_max)),
            (ModelError::RequestFailed(migrated), ModelError::RequestFailed(legacy))
            | (ModelError::StreamInterrupted(migrated), ModelError::StreamInterrupted(legacy))
            | (
                ModelError::InvalidConfiguration(migrated),
                ModelError::InvalidConfiguration(legacy),
            ) => assert_eq!(migrated, legacy),
            (migrated, legacy) => {
                panic!("error classification differs: migrated={migrated:?}, legacy={legacy:?}")
            }
        }
    }

    #[test]
    fn decoder_rejects_malformed_json_without_echoing_frame() {
        let mut decoder = OpenAiCompletionsProtocol::new().decoder();
        let error = decoder.push("not-json-secret-value").unwrap_err();
        assert!(matches!(error, ModelError::StreamInterrupted(_)));
        assert!(!error.to_string().contains("not-json-secret-value"));
    }

    async fn mock_openai_server() -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let frames = [
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices":[{"delta":{"content":"hello"}}]})
            ),
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":"{\"message\""}}]}}]})
            ),
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"ok\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}})
            ),
            "data: [DONE]\n\n".to_string(),
        ];
        let body = frames.concat();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = sender.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(17) {
                socket.write_all(chunk).await.unwrap();
                socket.flush().await.unwrap();
                sleep(Duration::from_millis(1)).await;
            }
        });
        (format!("http://{address}/v1"), receiver, handle)
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let header_end = loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                return request;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let content_length = String::from_utf8_lossy(&request[..header_end])
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::to_string)
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        request
    }

    #[tokio::test]
    async fn provider_client_streams_openai_completions_with_legacy_identity() {
        let (base_url, request_receiver, server) = mock_openai_server().await;
        let client = ProviderClient::new(
            ProviderClientConfig {
                client_namespace: "openai".to_string(),
                base_url,
                model: "gpt-test".to_string(),
                auth: ResolvedAuth::bearer("test-secret").unwrap(),
                headers: Vec::new(),
                options: ProviderOptions::default(),
                protocol_options: serde_json::json!({}),
            },
            Arc::new(OpenAiCompletionsProtocol::new()),
            Arc::new(Transport::new(TransportConfig::default()).unwrap()),
        )
        .unwrap();
        let events = client
            .stream(
                &[Message::user("use echo")],
                &[ModelToolSchema {
                    name: "echo".to_string(),
                    description: "Echo".to_string(),
                    parameters: serde_json::json!({"type":"object"}),
                }],
            )
            .collect::<Vec<_>>()
            .await;
        server.await.unwrap();

        let request = String::from_utf8(request_receiver.await.unwrap()).unwrap();
        let request_lower = request.to_ascii_lowercase();
        assert!(request_lower.contains("post /v1/chat/completions http/1.1"));
        assert!(request_lower.contains("authorization: bearer test-secret"));
        assert!(request.contains("\"model\":\"gpt-test\""));
        let legacy = OpenAiClient::new(
            client.config().base_url.clone(),
            "test-secret".to_string(),
            "gpt-test".to_string(),
        );
        assert_eq!(client.client_id(), legacy.client_id());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(events.iter().any(
            |event| matches!(event, ModelEvent::ToolUseDone { args, .. } if args["message"] == "ok")
        ));
        assert!(
            events.iter().any(
                |event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 5)
            )
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }
}
