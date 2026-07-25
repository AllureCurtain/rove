use std::collections::BTreeMap;

use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER},
};

use crate::provider::{
    ANTHROPIC_MESSAGES_PROTOCOL, AuthStyle, Framing, StreamDecoder, WireProtocol, WireProtocolId,
    WireRequest, WireRequestInput,
};
use crate::{Message, ModelError, ModelEvent, ModelToolSchema, Role, Usage};

const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicMessagesProtocol {
    id: WireProtocolId,
}

impl AnthropicMessagesProtocol {
    pub fn new() -> Self {
        Self {
            id: WireProtocolId::new(ANTHROPIC_MESSAGES_PROTOCOL)
                .expect("the built-in Anthropic Messages protocol id is valid"),
        }
    }
}

impl Default for AnthropicMessagesProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl WireProtocol for AnthropicMessagesProtocol {
    fn id(&self) -> &WireProtocolId {
        &self.id
    }

    fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
        let (system_prompt, conversation) = extract_system(input.messages);
        let messages = conversation
            .iter()
            .map(format_anthropic_message)
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": input.model,
            "max_tokens": input.options.max_tokens_or(DEFAULT_MAX_TOKENS),
            "messages": messages,
            "stream": true,
        });

        insert_float_option(&mut body, "temperature", input.options.temperature);
        insert_float_option(&mut body, "top_p", input.options.top_p);
        if let Some(system) = system_prompt {
            body["system"] = serde_json::Value::String(system);
        }
        if !input.tools.is_empty() {
            body["tools"] =
                serde_json::Value::Array(input.tools.iter().map(format_anthropic_tool).collect());
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        Ok(WireRequest {
            method: Method::POST,
            path: "v1/messages".to_string(),
            headers,
            body,
        })
    }

    fn framing(&self) -> Framing {
        Framing::ServerSentEvents
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(AnthropicMessagesDecoder::default())
    }

    fn classify_error(&self, status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
        classify_anthropic_error(status, headers, body)
    }

    fn default_auth_style(&self) -> AuthStyle {
        AuthStyle::Header(HeaderName::from_static("x-api-key"))
    }
}

fn insert_float_option(body: &mut serde_json::Value, key: &str, value: Option<f64>) {
    if let Some(value) = value
        && let Some(number) = serde_json::Number::from_f64(value)
    {
        body[key] = serde_json::Value::Number(number);
    }
}

fn format_anthropic_tool(tool: &ModelToolSchema) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

fn format_anthropic_message(message: &Message) -> serde_json::Value {
    match message.role {
        Role::Assistant if !message.tool_calls.is_empty() => {
            let mut content = Vec::new();
            if !message.content.is_empty() {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": message.content,
                }));
            }
            for tool_call in &message.tool_calls {
                content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tool_call.id,
                    "name": tool_call.name,
                    "input": tool_call.args,
                }));
            }
            serde_json::json!({
                "role": "assistant",
                "content": content,
            })
        }
        Role::Tool => {
            let block = if let Some(id) = &message.tool_call_id {
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": message.content,
                })
            } else {
                serde_json::json!({
                    "type": "text",
                    "text": message.content,
                })
            };
            serde_json::json!({
                "role": "user",
                "content": [block],
            })
        }
        _ => serde_json::json!({
            "role": anthropic_role(&message.role),
            "content": message.content,
        }),
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
    ModelError::RequestFailed(format!("HTTP {status}: {body}"))
}

#[derive(Debug, Default)]
struct AnthropicMessagesDecoder {
    blocks: BTreeMap<u64, AnthropicPartialToolUse>,
    usage: Usage,
}

#[derive(Debug, Default)]
struct AnthropicPartialToolUse {
    id: String,
    name: String,
    input_json: String,
}

impl StreamDecoder for AnthropicMessagesDecoder {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let json = serde_json::from_str::<serde_json::Value>(frame).map_err(|_| {
            ModelError::StreamInterrupted(
                "Anthropic Messages stream frame is invalid JSON".to_string(),
            )
        })?;
        if json.get("type").and_then(|value| value.as_str()) == Some("error") {
            return Err(ModelError::RequestFailed(
                "Anthropic Messages stream reported failure".to_string(),
            ));
        }
        Ok(normalize_anthropic_value(self, &json))
    }
}

fn normalize_anthropic_value(
    state: &mut AnthropicMessagesDecoder,
    json: &serde_json::Value,
) -> Vec<ModelEvent> {
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

    events
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
    use crate::anthropic::{
        AnthropicClient, AnthropicToolUseState,
        classify_anthropic_error as legacy_classify_anthropic_error,
        normalize_anthropic_event as legacy_normalize_anthropic_event,
    };
    use crate::provider::{
        ProviderClient, ProviderClientConfig, ResolvedAuth, Transport, TransportConfig,
    };
    use crate::{ModelClient, ProviderOptions, ToolCallRef};

    fn messages() -> Vec<Message> {
        vec![
            Message::system("You are helpful."),
            Message::user("inspect"),
            Message::assistant_with_tool_calls(
                "checking".to_string(),
                vec![ToolCallRef {
                    id: "toolu_1".to_string(),
                    name: "read_file".to_string(),
                    args: serde_json::json!({"path":"Cargo.toml"}),
                }],
            ),
            Message::tool("contents", Some("toolu_1".to_string())),
            Message::tool("legacy output", None),
        ]
    }

    fn tools() -> Vec<ModelToolSchema> {
        vec![ModelToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        }]
    }

    fn options() -> ProviderOptions {
        ProviderOptions {
            max_tokens: Some(8192),
            temperature: Some(0.7),
            top_p: Some(0.95),
            ..Default::default()
        }
    }

    fn frames() -> Vec<String> {
        vec![
            serde_json::json!({
                "type": "message_start",
                "message": {"usage": {"input_tokens": 10}}
            })
            .to_string(),
            serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "hello"}
            })
            .to_string(),
            serde_json::json!({
                "type": "content_block_start",
                "index": 1,
                "content_block": {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "read_file",
                    "input": {}
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "{\"path\":\"Cargo.toml\"}"
                }
            })
            .to_string(),
            serde_json::json!({"type":"content_block_stop","index":1}).to_string(),
            serde_json::json!({
                "type": "message_delta",
                "usage": {"output_tokens": 5}
            })
            .to_string(),
            serde_json::json!({"type":"message_stop"}).to_string(),
        ]
    }

    #[test]
    fn request_body_and_headers_match_legacy_anthropic_client() {
        let messages = messages();
        let tools = tools();
        let options = options();
        let mut legacy = AnthropicClient::new(
            "https://api.anthropic.com".to_string(),
            "test-secret".to_string(),
            "claude-test".to_string(),
        );
        legacy.apply_options(&options);
        let legacy_body = legacy.build_request_body(&messages, &tools);
        let request = AnthropicMessagesProtocol::new()
            .build_request(&WireRequestInput {
                model: "claude-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap();

        assert_eq!(request.path, "v1/messages");
        assert_eq!(request.body, legacy_body);
        assert_eq!(
            request.headers.get("anthropic-version").unwrap(),
            ANTHROPIC_VERSION
        );
        assert_eq!(
            AnthropicMessagesProtocol::new().default_auth_style(),
            AuthStyle::Header(HeaderName::from_static("x-api-key"))
        );
    }

    #[test]
    fn decoder_events_match_legacy_anthropic_client() {
        let frames = frames();
        let mut legacy_state = AnthropicToolUseState::default();
        let mut migrated_decoder = AnthropicMessagesProtocol::new().decoder();
        let mut legacy_events = Vec::new();
        let mut migrated_events = Vec::new();

        for frame in &frames {
            legacy_events
                .extend(legacy_normalize_anthropic_event(&mut legacy_state, frame).unwrap());
            migrated_events.extend(migrated_decoder.push(frame).unwrap());
        }

        assert_eq!(migrated_events, legacy_events);
        assert!(
            migrated_events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(migrated_events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { id, name, args } if id == "toolu_1" && name == "read_file" && args["path"] == "Cargo.toml")));
        assert!(
            migrated_events.iter().any(
                |event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 15)
            )
        );
        assert!(
            migrated_events
                .iter()
                .any(|event| matches!(event, ModelEvent::Done))
        );
    }

    #[test]
    fn http_error_classification_matches_legacy_anthropic_client() {
        let mut rate_limit_headers = HeaderMap::new();
        rate_limit_headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
        let context_body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 100000 tokens > 200000 token limit"}}"#;
        let cases = [
            (StatusCode::UNAUTHORIZED, HeaderMap::new(), "bad key"),
            (
                StatusCode::TOO_MANY_REQUESTS,
                rate_limit_headers,
                "slow down",
            ),
            (StatusCode::BAD_REQUEST, HeaderMap::new(), context_body),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                HeaderMap::new(),
                "temporarily unavailable",
            ),
        ];

        for (status, headers, body) in cases {
            let legacy = legacy_classify_anthropic_error(status, &headers, body);
            let migrated = AnthropicMessagesProtocol::new().classify_error(status, &headers, body);
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
    fn stream_errors_are_typed_without_echoing_provider_text() {
        let frame = serde_json::json!({
            "type": "error",
            "error": {"message": "anthropic-stream-secret"}
        })
        .to_string();
        let error = AnthropicMessagesProtocol::new()
            .decoder()
            .push(&frame)
            .unwrap_err();
        assert!(matches!(error, ModelError::RequestFailed(_)));
        assert!(!error.to_string().contains("anthropic-stream-secret"));
    }

    async fn mock_anthropic_server() -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let body = frames()
            .into_iter()
            .map(|frame| format!("data: {frame}\n\n"))
            .collect::<String>();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = sender.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(11) {
                socket.write_all(chunk).await.unwrap();
                socket.flush().await.unwrap();
                sleep(Duration::from_millis(1)).await;
            }
        });
        (format!("http://{address}"), receiver, handle)
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
    async fn provider_client_streams_anthropic_with_native_auth_and_identity() {
        let (base_url, request_receiver, server) = mock_anthropic_server().await;
        let client = ProviderClient::new(
            ProviderClientConfig {
                client_namespace: "anthropic".to_string(),
                base_url,
                model: "claude-test".to_string(),
                auth: ResolvedAuth::header(HeaderName::from_static("x-api-key"), "test-secret")
                    .unwrap(),
                headers: Vec::new(),
                options: options(),
                protocol_options: serde_json::json!({}),
            },
            Arc::new(AnthropicMessagesProtocol::new()),
            Arc::new(Transport::new(TransportConfig::default()).unwrap()),
        )
        .unwrap();
        let events = client
            .stream(&messages(), &tools())
            .collect::<Vec<_>>()
            .await;
        server.await.unwrap();

        let request = String::from_utf8(request_receiver.await.unwrap()).unwrap();
        let request_lower = request.to_ascii_lowercase();
        assert!(request_lower.contains("post /v1/messages http/1.1"));
        assert!(request_lower.contains("x-api-key: test-secret"));
        assert!(request_lower.contains("anthropic-version: 2023-06-01"));
        let legacy = AnthropicClient::new(
            client.config().base_url.clone(),
            "test-secret".to_string(),
            "claude-test".to_string(),
        );
        assert_eq!(client.client_id(), legacy.client_id());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { args, .. } if args["path"] == "Cargo.toml")));
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }
}
