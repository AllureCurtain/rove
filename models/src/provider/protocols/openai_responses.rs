use std::collections::BTreeMap;

use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER},
};

use crate::provider::{
    AuthStyle, Framing, OPENAI_RESPONSES_PROTOCOL, StreamDecoder, WireProtocol, WireProtocolId,
    WireRequest, WireRequestInput,
};
use crate::{Message, ModelError, ModelEvent, ModelToolSchema, Role, Usage};

pub struct OpenAiResponsesProtocol {
    id: WireProtocolId,
    prompt_cache_enabled: bool,
    prompt_cache_retention: Option<String>,
}

impl OpenAiResponsesProtocol {
    pub fn new() -> Self {
        Self {
            id: WireProtocolId::new(OPENAI_RESPONSES_PROTOCOL)
                .expect("the built-in OpenAI Responses protocol id is valid"),
            prompt_cache_enabled: false,
            prompt_cache_retention: None,
        }
    }

    pub fn with_prompt_cache(mut self, enabled: bool, retention: Option<String>) -> Self {
        self.prompt_cache_enabled = enabled;
        self.prompt_cache_retention = retention;
        self
    }
}

impl Default for OpenAiResponsesProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl WireProtocol for OpenAiResponsesProtocol {
    fn id(&self) -> &WireProtocolId {
        &self.id
    }

    fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
        let prompt_cache_enabled =
            protocol_bool_option(input.protocol_options, "prompt_cache_enabled")?
                .unwrap_or(self.prompt_cache_enabled);
        let prompt_cache_retention =
            protocol_string_option(input.protocol_options, "prompt_cache_retention")?
                .or(self.prompt_cache_retention.as_deref());
        let reasoning_effort = protocol_string_option(input.protocol_options, "reasoning_effort")?;
        if let Some(effort) = reasoning_effort {
            validate_reasoning_effort(effort)?;
        }
        let (instructions, messages) = format_responses_input(input.messages);
        let tools = input
            .tools
            .iter()
            .map(format_responses_tool)
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": input.model,
            "input": messages,
            "stream": true,
            "store": false,
            "parallel_tool_calls": true,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();

        if let Some(instructions) = instructions {
            body.insert(
                "instructions".to_string(),
                serde_json::Value::String(instructions),
            );
        }
        if !tools.is_empty() {
            body.insert("tools".to_string(), serde_json::Value::Array(tools));
        }
        if let Some(max_tokens) = input.options.max_tokens {
            body.insert(
                "max_output_tokens".to_string(),
                serde_json::Value::Number(max_tokens.into()),
            );
        }
        insert_float_option(&mut body, "temperature", input.options.temperature);
        insert_float_option(&mut body, "top_p", input.options.top_p);
        if let Some(effort) = reasoning_effort {
            body.insert(
                "reasoning".to_string(),
                serde_json::json!({ "effort": effort }),
            );
        }
        if prompt_cache_enabled {
            body.insert(
                "prompt_cache_key".to_string(),
                serde_json::Value::String(prompt_cache_key(input.messages, input.tools)),
            );
            if let Some(retention) = prompt_cache_retention {
                body.insert(
                    "prompt_cache_retention".to_string(),
                    serde_json::Value::String(retention.to_string()),
                );
            }
        }

        Ok(WireRequest {
            method: Method::POST,
            path: "responses".to_string(),
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
        Box::new(OpenAiResponsesDecoder::default())
    }

    fn classify_error(&self, status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError {
        classify_responses_http_error(status, headers, body)
    }

    fn default_auth_style(&self) -> AuthStyle {
        AuthStyle::Bearer
    }
}

fn validate_reasoning_effort(value: &str) -> Result<(), ModelError> {
    if matches!(value, "low" | "medium" | "high") {
        Ok(())
    } else {
        Err(ModelError::InvalidConfiguration(
            "OpenAI Responses protocol option 'reasoning_effort' must be low, medium, or high"
                .to_string(),
        ))
    }
}

fn protocol_bool_option(
    options: &serde_json::Value,
    key: &'static str,
) -> Result<Option<bool>, ModelError> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_bool().map(Some).ok_or_else(|| {
        ModelError::InvalidConfiguration(format!(
            "OpenAI Responses protocol option '{key}' must be a boolean"
        ))
    })
}

fn protocol_string_option<'a>(
    options: &'a serde_json::Value,
    key: &'static str,
) -> Result<Option<&'a str>, ModelError> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_str().map(Some).ok_or_else(|| {
        ModelError::InvalidConfiguration(format!(
            "OpenAI Responses protocol option '{key}' must be a string"
        ))
    })
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

fn format_responses_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions = None;
    let mut input = Vec::new();

    for message in messages {
        match message.role {
            Role::System if instructions.is_none() => {
                instructions = Some(message.content.clone());
            }
            Role::System | Role::User => {
                input.push(input_text_item("user", &message.content));
            }
            Role::Assistant if !message.tool_calls.is_empty() => {
                if !message.content.is_empty() {
                    input.push(output_text_item(&message.content));
                }
                for tool_call in &message.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.args.to_string(),
                    }));
                }
            }
            Role::Assistant => {
                input.push(output_text_item(&message.content));
            }
            Role::Tool if message.tool_call_id.is_some() => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": message.tool_call_id.as_deref().unwrap_or_default(),
                    "output": message.content,
                }));
            }
            Role::Tool => {
                input.push(input_text_item("user", &message.content));
            }
        }
    }

    (instructions, input)
}

fn input_text_item(role: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "content": [{ "type": "input_text", "text": text }],
    })
}

fn output_text_item(text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text }],
    })
}

fn format_responses_tool(tool: &ModelToolSchema) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "strict": false,
    })
}

fn prompt_cache_key(messages: &[Message], tools: &[ModelToolSchema]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(&(messages, tools)).unwrap_or_default() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("rove-responses-{hash:016x}")
}

#[derive(Debug, Default)]
struct OpenAiResponsesDecoder {
    function_calls: BTreeMap<String, ResponsesFunctionCall>,
}

#[derive(Debug, Default)]
struct ResponsesFunctionCall {
    call_id: String,
    name: String,
    arguments: String,
    done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponsesFatalError {
    Failed,
    Incomplete,
}

struct NormalizedResponse {
    events: Vec<ModelEvent>,
    fatal_error: Option<ResponsesFatalError>,
}

impl StreamDecoder for OpenAiResponsesDecoder {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let normalized =
            normalize_responses_event(&mut self.function_calls, frame).map_err(|_| {
                ModelError::StreamInterrupted(
                    "OpenAI Responses stream frame is invalid JSON".to_string(),
                )
            })?;
        if let Some(error) = normalized.fatal_error {
            let message = match error {
                ResponsesFatalError::Failed => "OpenAI Responses stream reported failure",
                ResponsesFatalError::Incomplete => "OpenAI Responses stream was incomplete",
            };
            return Err(ModelError::RequestFailed(message.to_string()));
        }
        Ok(normalized.events)
    }
}

fn normalize_responses_event(
    function_calls: &mut BTreeMap<String, ResponsesFunctionCall>,
    frame: &str,
) -> serde_json::Result<NormalizedResponse> {
    if frame.trim() == "[DONE]" {
        return Ok(NormalizedResponse {
            events: vec![ModelEvent::Done],
            fatal_error: None,
        });
    }

    let json = serde_json::from_str::<serde_json::Value>(frame)?;
    let event_type = json.get("type").and_then(|value| value.as_str());
    let mut events = Vec::new();
    let mut fatal_error = None;

    match event_type {
        Some("response.output_text.delta") => {
            if let Some(delta) = json.get("delta").and_then(|value| value.as_str())
                && !delta.is_empty()
            {
                events.push(ModelEvent::TextDelta {
                    text: delta.to_string(),
                });
            }
        }
        Some("response.output_item.added") => {
            if let Some(item) = json.get("item") {
                capture_function_call_start(function_calls, item, &mut events);
            }
        }
        Some("response.function_call_arguments.delta") => {
            let item_id = json
                .get("item_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let delta = json
                .get("delta")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !item_id.is_empty()
                && !delta.is_empty()
                && let Some(call) = function_calls.get_mut(item_id)
            {
                call.arguments.push_str(delta);
                events.push(ModelEvent::ToolUseDelta {
                    id: call.call_id.clone(),
                    args_delta: delta.to_string(),
                });
            }
        }
        Some("response.function_call_arguments.done") => {
            let item_id = json
                .get("item_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if let Some(call) = function_calls.get_mut(item_id) {
                if let Some(arguments) = json.get("arguments").and_then(|value| value.as_str()) {
                    call.arguments = arguments.to_string();
                }
                call.done = true;
                events.push(ModelEvent::ToolUseDone {
                    id: call.call_id.clone(),
                    name: call.name.clone(),
                    args: parse_arguments(&call.arguments),
                });
            }
        }
        Some("response.output_item.done") => {
            if let Some(item) = json.get("item") {
                capture_function_call_done(function_calls, item, &mut events);
            }
        }
        Some("response.completed") => {
            if let Some(usage) = json
                .get("response")
                .and_then(|response| response.get("usage"))
            {
                events.push(ModelEvent::Usage {
                    usage: parse_responses_usage(usage),
                });
            }
            events.push(ModelEvent::Done);
        }
        Some("response.failed") => fatal_error = Some(ResponsesFatalError::Failed),
        Some("response.incomplete") => fatal_error = Some(ResponsesFatalError::Incomplete),
        _ => {}
    }

    Ok(NormalizedResponse {
        events,
        fatal_error,
    })
}

fn capture_function_call_start(
    function_calls: &mut BTreeMap<String, ResponsesFunctionCall>,
    item: &serde_json::Value,
    events: &mut Vec<ModelEvent>,
) {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return;
    }
    let item_id = item
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let call_id = item
        .get("call_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&item_id)
        .to_string();
    let name = item
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool")
        .to_string();
    if item_id.is_empty() {
        return;
    }

    function_calls.insert(
        item_id,
        ResponsesFunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: item
                .get("arguments")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            done: false,
        },
    );
    events.push(ModelEvent::ToolUseStart { id: call_id, name });
}

fn capture_function_call_done(
    function_calls: &mut BTreeMap<String, ResponsesFunctionCall>,
    item: &serde_json::Value,
    events: &mut Vec<ModelEvent>,
) {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return;
    }
    let item_id = item
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let call_id = item
        .get("call_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&item_id)
        .to_string();
    let name = item
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool")
        .to_string();
    let arguments = item
        .get("arguments")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    if let Some(call) = function_calls.get_mut(&item_id) {
        if call.done {
            return;
        }
        call.done = true;
    }

    events.push(ModelEvent::ToolUseDone {
        id: call_id,
        name,
        args: parse_arguments(&arguments),
    });
}

fn parse_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()))
}

fn parse_responses_usage(usage: &serde_json::Value) -> Usage {
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(u64::from(prompt_tokens) + u64::from(completion_tokens))
        as u32;
    let cached_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;

    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
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

fn classify_responses_http_error(
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
) -> ModelError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelError::AuthFailed;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelError::RateLimited {
            retry_after_ms: parse_retry_after_ms(headers).unwrap_or(1000),
        };
    }
    let body_lower = body.to_ascii_lowercase();
    if body_lower.contains("context") && body_lower.contains("token") {
        return ModelError::ContextLengthExceeded { used: 0, max: 0 };
    }
    ModelError::RequestFailed(format!("HTTP {status}: {body}"))
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
    use crate::openai_responses::{
        OpenAiResponsesClient, ResponsesStreamState,
        classify_responses_http_error as legacy_classify_responses_http_error,
        normalize_responses_event as legacy_normalize_responses_event,
    };
    use crate::provider::{
        ProviderClient, ProviderClientConfig, ResolvedAuth, Transport, TransportConfig,
    };
    use crate::{ModelClient, ProviderOptions, ToolCallRef};

    fn messages() -> Vec<Message> {
        vec![
            Message::system("follow policy"),
            Message::system("additional context"),
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
            max_tokens: Some(1024),
            temperature: Some(0.4),
            top_p: Some(0.9),
            frequency_penalty: Some(0.2),
            presence_penalty: Some(0.3),
        }
    }

    fn response_frames() -> Vec<String> {
        vec![
            serde_json::json!({
                "type": "response.output_text.delta",
                "delta": "hello"
            })
            .to_string(),
            serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "read_file",
                    "arguments": ""
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "delta": "{\"path\""
            })
            .to_string(),
            serde_json::json!({
                "type": "response.function_call_arguments.done",
                "item_id": "fc_1",
                "arguments": "{\"path\":\"Cargo.toml\"}"
            })
            .to_string(),
            serde_json::json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "read_file",
                    "arguments": "{\"path\":\"Cargo.toml\"}"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15,
                        "input_tokens_details": {"cached_tokens": 4}
                    }
                }
            })
            .to_string(),
        ]
    }

    #[test]
    fn request_body_and_cache_fields_match_legacy_responses_client() {
        let messages = messages();
        let tools = tools();
        let options = options();
        let mut legacy = OpenAiResponsesClient::new(
            "https://api.openai.com/v1".to_string(),
            "test-secret".to_string(),
            "gpt-test".to_string(),
        )
        .with_prompt_cache(true, Some("24h".to_string()));
        legacy.apply_options(&options);
        let legacy_body = legacy.build_request_body(&messages, &tools);
        let protocol =
            OpenAiResponsesProtocol::new().with_prompt_cache(true, Some("24h".to_string()));
        let request = protocol
            .build_request(&WireRequestInput {
                model: "gpt-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap();

        assert_eq!(request.path, "responses");
        assert_eq!(request.body, legacy_body);
        assert_eq!(request.body["prompt_cache_retention"], "24h");

        let uncached = OpenAiResponsesProtocol::new()
            .with_prompt_cache(false, Some("24h".to_string()))
            .build_request(&WireRequestInput {
                model: "gpt-test",
                messages: &messages,
                tools: &tools,
                options: &options,
                protocol_options: &serde_json::json!({}),
            })
            .unwrap();
        assert!(uncached.body.get("prompt_cache_key").is_none());
        assert!(uncached.body.get("prompt_cache_retention").is_none());
    }

    #[test]
    fn reasoning_effort_is_only_emitted_as_responses_reasoning() {
        let messages = messages();
        let options = options();
        let request = OpenAiResponsesProtocol::new()
            .build_request(&WireRequestInput {
                model: "gpt-5",
                messages: &messages,
                tools: &[],
                options: &options,
                protocol_options: &serde_json::json!({
                    "reasoning_effort": "high"
                }),
            })
            .unwrap();
        assert_eq!(request.body["reasoning"]["effort"], "high");
        assert!(request.body.get("reasoning_effort").is_none());

        let error = OpenAiResponsesProtocol::new()
            .build_request(&WireRequestInput {
                model: "gpt-5",
                messages: &messages,
                tools: &[],
                options: &options,
                protocol_options: &serde_json::json!({
                    "reasoning_effort": "unsupported"
                }),
            })
            .unwrap_err();
        assert!(error.to_string().contains("reasoning_effort"));
    }

    #[test]
    fn decoder_events_match_legacy_responses_client() {
        let frames = response_frames();
        let mut legacy_state = ResponsesStreamState::default();
        let mut migrated_decoder = OpenAiResponsesProtocol::new().decoder();
        let mut legacy_events = Vec::new();
        let mut migrated_events = Vec::new();

        for frame in &frames {
            let legacy = legacy_normalize_responses_event(&mut legacy_state, frame).unwrap();
            assert!(legacy.fatal_error.is_none());
            legacy_events.extend(legacy.events);
            migrated_events.extend(migrated_decoder.push(frame).unwrap());
        }

        assert_eq!(migrated_events, legacy_events);
        assert!(migrated_events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { id, name, args } if id == "call_1" && name == "read_file" && args["path"] == "Cargo.toml")));
        assert!(migrated_events.iter().any(|event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 15 && usage.cached_tokens == 4)));
        assert!(
            migrated_events
                .iter()
                .any(|event| matches!(event, ModelEvent::Done))
        );
    }

    #[test]
    fn terminal_failures_preserve_error_class_without_echoing_provider_text() {
        let cases = [
            serde_json::json!({
                "type": "response.failed",
                "response": {"error": {"message": "stream-secret-value"}}
            })
            .to_string(),
            serde_json::json!({
                "type": "response.incomplete",
                "response": {
                    "incomplete_details": {"reason": "stream-secret-value"}
                }
            })
            .to_string(),
        ];

        for frame in cases {
            let mut legacy_state = ResponsesStreamState::default();
            let legacy = legacy_normalize_responses_event(&mut legacy_state, &frame).unwrap();
            let legacy = ModelError::RequestFailed(legacy.fatal_error.unwrap());
            let migrated = OpenAiResponsesProtocol::new()
                .decoder()
                .push(&frame)
                .unwrap_err();

            assert_eq!(migrated.error_code(), legacy.error_code());
            assert!(matches!(migrated, ModelError::RequestFailed(_)));
            assert!(!migrated.to_string().contains("stream-secret-value"));
        }
    }

    #[test]
    fn malformed_stream_frame_is_rejected_without_echoing_frame() {
        let error = OpenAiResponsesProtocol::new()
            .decoder()
            .push("not-json-secret-value")
            .unwrap_err();

        assert!(matches!(error, ModelError::StreamInterrupted(_)));
        assert!(!error.to_string().contains("not-json-secret-value"));
    }

    #[test]
    fn http_error_classification_matches_legacy_responses_client() {
        let mut rate_limit_headers = HeaderMap::new();
        rate_limit_headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));
        let cases = [
            (StatusCode::FORBIDDEN, HeaderMap::new(), "bad key"),
            (
                StatusCode::TOO_MANY_REQUESTS,
                rate_limit_headers,
                "rate limited",
            ),
            (
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                "context token limit exceeded",
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                HeaderMap::new(),
                "temporarily unavailable",
            ),
        ];
        let protocol = OpenAiResponsesProtocol::new();

        for (status, headers, body) in cases {
            let legacy = legacy_classify_responses_http_error(status, &headers, body);
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

    async fn mock_responses_server() -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let body = response_frames()
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
            for chunk in body.as_bytes().chunks(13) {
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

    fn request_json(request: &[u8]) -> serde_json::Value {
        let body_start = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
            .unwrap();
        serde_json::from_slice(&request[body_start..]).unwrap()
    }

    #[tokio::test]
    async fn provider_client_streams_responses_with_cache_and_legacy_identity() {
        let (base_url, request_receiver, server) = mock_responses_server().await;
        let client = ProviderClient::new(
            ProviderClientConfig {
                client_namespace: "openai-responses".to_string(),
                base_url,
                model: "gpt-test".to_string(),
                auth: ResolvedAuth::bearer("test-secret").unwrap(),
                headers: Vec::new(),
                options: options(),
                protocol_options: serde_json::json!({}),
            },
            Arc::new(
                OpenAiResponsesProtocol::new().with_prompt_cache(true, Some("24h".to_string())),
            ),
            Arc::new(Transport::new(TransportConfig::default()).unwrap()),
        )
        .unwrap();
        let events = client
            .stream(&messages(), &tools())
            .collect::<Vec<_>>()
            .await;
        server.await.unwrap();

        let request = request_receiver.await.unwrap();
        let request_text = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(request_text.contains("post /v1/responses http/1.1"));
        assert!(request_text.contains("authorization: bearer test-secret"));
        let body = request_json(&request);
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["prompt_cache_retention"], "24h");
        assert!(body["prompt_cache_key"].as_str().is_some());

        let legacy = OpenAiResponsesClient::new(
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
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { args, .. } if args["path"] == "Cargo.toml")));
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Usage { usage } if usage.total_tokens == 15 && usage.cached_tokens == 4)));
        assert!(events.iter().any(|event| matches!(event, ModelEvent::Done)));
    }
}
