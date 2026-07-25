use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER},
};

use crate::provider::{
    AuthStyle, Framing, OLLAMA_PROTOCOL, StreamDecoder, WireProtocol, WireProtocolId, WireRequest,
    WireRequestInput,
};
use crate::{Message, ModelError, ModelEvent, ModelToolSchema, Role, Usage};

pub struct OllamaChatProtocol {
    id: WireProtocolId,
}

impl OllamaChatProtocol {
    pub fn new() -> Self {
        Self {
            id: WireProtocolId::new(OLLAMA_PROTOCOL)
                .expect("the built-in Ollama protocol id is valid"),
        }
    }
}

impl Default for OllamaChatProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl WireProtocol for OllamaChatProtocol {
    fn id(&self) -> &WireProtocolId {
        &self.id
    }

    fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
        let messages = input
            .messages
            .iter()
            .map(format_ollama_message)
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "stream": true,
        });
        let options = request_options_json(input.options);
        if !options.as_object().is_some_and(|object| object.is_empty()) {
            body["options"] = options;
        }
        if !input.tools.is_empty() {
            body["tools"] =
                serde_json::Value::Array(input.tools.iter().map(format_ollama_tool).collect());
        }

        Ok(WireRequest {
            method: Method::POST,
            path: "api/chat".to_string(),
            headers: HeaderMap::from_iter([(
                CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )]),
            body,
        })
    }

    fn framing(&self) -> Framing {
        Framing::JsonLines
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(OllamaChatDecoder)
    }

    fn classify_error(&self, status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
        classify_ollama_error(status, headers, body)
    }

    fn default_auth_style(&self) -> AuthStyle {
        AuthStyle::None
    }
}

fn format_ollama_tool(tool: &ModelToolSchema) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

fn format_ollama_message(message: &Message) -> serde_json::Value {
    match message.role {
        Role::Assistant if !message.tool_calls.is_empty() => {
            let tool_calls = message
                .tool_calls
                .iter()
                .map(|tool_call| {
                    serde_json::json!({
                        "function": {
                            "name": tool_call.name,
                            "arguments": tool_call.args,
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
        Role::Tool if message.tool_call_id.is_some() => serde_json::json!({
            "role": "tool",
            "content": message.content,
        }),
        Role::Tool => serde_json::json!({
            "role": "user",
            "content": message.content,
        }),
        _ => serde_json::json!({
            "role": ollama_role(&message.role),
            "content": message.content,
        }),
    }
}

fn ollama_role(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User | Role::Tool => "user",
        Role::Assistant => "assistant",
    }
}

fn request_options_json(options: &crate::ProviderOptions) -> serde_json::Value {
    let mut value = serde_json::json!({});
    if let Some(num_predict) = options.max_tokens {
        value["num_predict"] = serde_json::Value::Number(num_predict.into());
    }
    if let Some(temperature) = options.temperature
        && let Some(number) = serde_json::Number::from_f64(temperature)
    {
        value["temperature"] = serde_json::Value::Number(number);
    }
    if let Some(top_p) = options.top_p
        && let Some(number) = serde_json::Number::from_f64(top_p)
    {
        value["top_p"] = serde_json::Value::Number(number);
    }
    value
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
        _ => ModelError::RequestFailed(format!("HTTP {status}: {body}")),
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

struct OllamaChatDecoder;

impl StreamDecoder for OllamaChatDecoder {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let json = serde_json::from_str::<serde_json::Value>(frame).map_err(|_| {
            ModelError::StreamInterrupted("Ollama Chat stream line is invalid JSON".to_string())
        })?;
        if json.get("error").is_some() {
            return Err(ModelError::RequestFailed(
                "Ollama Chat stream reported failure".to_string(),
            ));
        }
        Ok(normalize_ollama_value(&json))
    }
}

fn normalize_ollama_value(json: &serde_json::Value) -> Vec<ModelEvent> {
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
        events.push(ModelEvent::Usage {
            usage: Usage {
                total_tokens: usage.prompt_tokens + usage.completion_tokens,
                ..usage
            },
        });
        events.push(ModelEvent::Done);
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
    use crate::ollama::{
        OllamaClient, classify_ollama_error as legacy_classify_ollama_error,
        normalize_ollama_chat_line as legacy_normalize_ollama_chat_line,
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
                    id: "ollama_tool_call_0".to_string(),
                    name: "read_file".to_string(),
                    args: serde_json::json!({"path":"Cargo.toml"}),
                }],
            ),
            Message::tool("contents", Some("ollama_tool_call_0".to_string())),
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
            max_tokens: Some(2048),
            temperature: Some(0.8),
            top_p: Some(0.9),
            ..Default::default()
        }
    }

    fn lines() -> Vec<String> {
        vec![
            serde_json::json!({
                "message": {"role": "assistant", "content": "hello"},
                "done": false
            })
            .to_string(),
            serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "function": {
                            "name": "read_file",
                            "arguments": {"path": "Cargo.toml"}
                        }
                    }]
                },
                "done": false
            })
            .to_string(),
            serde_json::json!({
                "message": {"role": "assistant", "content": ""},
                "done": true,
                "prompt_eval_count": 10,
                "eval_count": 5
            })
            .to_string(),
        ]
    }

    #[test]
    fn request_body_matches_legacy_ollama_client() {
        let messages = messages();
        let tools = tools();
        let options = options();
        let mut legacy = OllamaClient::new(
            "http://localhost:11434".to_string(),
            "llama-test".to_string(),
        );
        legacy.apply_options(&options);
        let legacy_body = legacy.build_request_body(&messages, &tools);
        let request = OllamaChatProtocol::new()
            .build_request(&WireRequestInput {
                model: "llama-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap();

        assert_eq!(request.path, "api/chat");
        assert_eq!(request.body, legacy_body);
        assert_eq!(
            OllamaChatProtocol::new().default_auth_style(),
            AuthStyle::None
        );
    }

    #[test]
    fn decoder_events_match_legacy_ollama_client() {
        let lines = lines();
        let mut migrated_decoder = OllamaChatProtocol::new().decoder();
        let mut legacy_events = Vec::new();
        let mut migrated_events = Vec::new();

        for line in &lines {
            legacy_events.extend(legacy_normalize_ollama_chat_line(line).unwrap());
            migrated_events.extend(migrated_decoder.push(line).unwrap());
        }

        assert_eq!(migrated_events, legacy_events);
        assert!(
            migrated_events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(migrated_events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { name, args, .. } if name == "read_file" && args["path"] == "Cargo.toml")));
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
    fn http_error_classification_matches_legacy_ollama_client() {
        let mut rate_limit_headers = HeaderMap::new();
        rate_limit_headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        let cases = [
            (StatusCode::NOT_FOUND, HeaderMap::new(), "missing-model"),
            (StatusCode::UNAUTHORIZED, HeaderMap::new(), "bad key"),
            (
                StatusCode::TOO_MANY_REQUESTS,
                rate_limit_headers,
                "slow down",
            ),
            (
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                "context window exceeded: input too long",
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                HeaderMap::new(),
                "temporarily unavailable",
            ),
        ];

        for (status, headers, body) in cases {
            let legacy = legacy_classify_ollama_error(status, &headers, body);
            let migrated = OllamaChatProtocol::new().classify_error(status, &headers, body);
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
    fn malformed_and_error_lines_fail_without_echoing_provider_text() {
        for line in [
            "not-json-ollama-secret".to_string(),
            serde_json::json!({"error":"ollama-stream-secret"}).to_string(),
        ] {
            let error = OllamaChatProtocol::new().decoder().push(&line).unwrap_err();
            assert!(!error.to_string().contains("ollama-secret"));
            assert!(!error.to_string().contains("ollama-stream-secret"));
        }
    }

    async fn mock_ollama_server() -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let body = lines().join("\n");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = sender.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(7) {
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
    async fn provider_client_streams_ollama_final_jsonl_line_and_preserves_identity() {
        let (base_url, request_receiver, server) = mock_ollama_server().await;
        let client = ProviderClient::new(
            ProviderClientConfig {
                client_namespace: "ollama".to_string(),
                base_url,
                model: "llama-test".to_string(),
                auth: ResolvedAuth::none(),
                headers: Vec::new(),
                options: options(),
                protocol_options: serde_json::json!({}),
            },
            Arc::new(OllamaChatProtocol::new()),
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
        assert!(request_lower.contains("post /api/chat http/1.1"));
        assert!(!request_lower.contains("authorization:"));
        let legacy = OllamaClient::new(client.config().base_url.clone(), "llama-test".to_string());
        assert_eq!(client.client_id(), legacy.client_id());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelEvent::TextDelta { text } if text == "hello"))
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { args, .. } if args["path"] == "Cargo.toml")));
        assert!(
            events.iter().any(
                |event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 15)
            )
        );
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }
}
