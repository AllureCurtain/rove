//! Bounded internal MCP protocol types shared by every transport.
//!
//! These types are the single protocol vocabulary for stdio, Streamable HTTP,
//! and legacy SSE. A transport adapter moves frames; it never decides tool
//! safety, retry, or how a result is projected.
//!
//! Everything crossing this boundary is untrusted remote input, so each type
//! carries explicit bounds and validation rather than trusting the peer.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Protocol version this client proposes during initialize.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Versions this client can actually speak, newest first.
///
/// Negotiation picks a server version only from this list. An unknown version is
/// refused rather than assumed compatible.
pub const SUPPORTED_MCP_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Header carrying the negotiated version on post-initialize HTTP requests.
pub const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";

/// Header carrying a server-assigned Streamable HTTP session ID.
pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

/// Version assumed when a server omits the header entirely.
pub const MCP_DEFAULT_NEGOTIATED_VERSION: &str = "2025-03-26";

pub const MAX_MCP_SESSION_ID_BYTES: usize = 512;
pub const MAX_MCP_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_MCP_TOOLS_PER_SERVER: usize = 128;
pub const MAX_MCP_TOOL_SCHEMA_BYTES: usize = 128 * 1024;
pub const MAX_MCP_CURSOR_BYTES: usize = 4 * 1024;
pub const MAX_MCP_LIST_PAGES: usize = 32;
pub const MAX_MCP_DIAGNOSTIC_CHARS: usize = 500;

/// JSON-RPC request identity. Only the shapes MCP actually uses are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    Text(String),
}

impl fmt::Display for JsonRpcId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::Text(value) => write!(formatter, "{value}"),
        }
    }
}

impl JsonRpcId {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_u64().map(Self::Number),
            Value::String(text) if !text.is_empty() && text.len() <= 256 => {
                Some(Self::Text(text.clone()))
            }
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Number(value) => json!(value),
            Self::Text(value) => json!(value),
        }
    }
}

/// A bounded, already-validated protocol error from the peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    /// Present only when the peer supplied structured data.
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// JSON-RPC reserved code for a method this client does not implement.
    pub const METHOD_NOT_FOUND: i64 = -32601;

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not supported by this client: {method}"),
            data: None,
        }
    }

    fn from_value(value: &Value) -> Self {
        let code = value.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .map(bounded_diagnostic)
            .unwrap_or_else(|| "unknown MCP error".to_string());
        Self {
            code,
            message,
            data: value.get("data").cloned(),
        }
    }

    fn to_value(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        error
    }
}

impl fmt::Display for JsonRpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "MCP error {}: {}", self.code, self.message)
    }
}

/// Every message class the dispatcher must distinguish.
///
/// Keeping these separate is what allows a notification or a server request to
/// be handled instead of being dropped for not matching a pending request ID.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcMessage {
    /// Client-to-server or server-to-client call expecting a response.
    Request {
        id: JsonRpcId,
        method: String,
        params: Value,
    },
    /// Fire-and-forget message in either direction.
    Notification { method: String, params: Value },
    /// Successful response to a request.
    Response { id: JsonRpcId, result: Value },
    /// Error response to a request.
    ErrorResponse { id: JsonRpcId, error: JsonRpcError },
}

impl JsonRpcMessage {
    pub fn request(id: JsonRpcId, method: impl Into<String>, params: Value) -> Self {
        Self::Request {
            id,
            method: method.into(),
            params,
        }
    }

    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self::Notification {
            method: method.into(),
            params,
        }
    }

    /// The correlation ID, when this message carries one.
    pub fn id(&self) -> Option<&JsonRpcId> {
        match self {
            Self::Request { id, .. }
            | Self::Response { id, .. }
            | Self::ErrorResponse { id, .. } => Some(id),
            Self::Notification { .. } => None,
        }
    }

    /// True when this message resolves a pending outbound request.
    pub fn is_response(&self) -> bool {
        matches!(self, Self::Response { .. } | Self::ErrorResponse { .. })
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Request { id, method, params } => json!({
                "jsonrpc": "2.0",
                "id": id.to_value(),
                "method": method,
                "params": params,
            }),
            Self::Notification { method, params } => json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            }),
            Self::Response { id, result } => json!({
                "jsonrpc": "2.0",
                "id": id.to_value(),
                "result": result,
            }),
            Self::ErrorResponse { id, error } => json!({
                "jsonrpc": "2.0",
                "id": id.to_value(),
                "error": error.to_value(),
            }),
        }
    }

    /// Parse one untrusted frame.
    ///
    /// A frame that does not match a known class is rejected with a bounded
    /// diagnostic instead of being silently ignored, so a peer cannot quietly
    /// desynchronize the session.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, McpProtocolError> {
        if bytes.len() > MAX_MCP_MESSAGE_BYTES {
            return Err(McpProtocolError::MessageTooLarge);
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| McpProtocolError::MalformedFrame)?;
        Self::from_value(value)
    }

    pub fn from_value(value: Value) -> Result<Self, McpProtocolError> {
        // The version field is validated but a missing one is tolerated: some
        // servers omit it on notifications, and refusing the whole session over
        // that would be less safe than proceeding with a parsed message.
        if let Some(version) = value.get("jsonrpc").and_then(Value::as_str)
            && version != "2.0"
        {
            return Err(McpProtocolError::UnsupportedJsonRpcVersion);
        }

        let id = value.get("id").and_then(JsonRpcId::from_value);
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);

        match (id, method) {
            (Some(id), Some(method)) => Ok(Self::Request {
                id,
                method,
                params: value.get("params").cloned().unwrap_or_else(|| json!({})),
            }),
            (None, Some(method)) => Ok(Self::Notification {
                method,
                params: value.get("params").cloned().unwrap_or_else(|| json!({})),
            }),
            (Some(id), None) => {
                if let Some(error) = value.get("error") {
                    Ok(Self::ErrorResponse {
                        id,
                        error: JsonRpcError::from_value(error),
                    })
                } else {
                    Ok(Self::Response {
                        id,
                        result: value.get("result").cloned().unwrap_or_else(|| json!({})),
                    })
                }
            }
            (None, None) => Err(McpProtocolError::UnknownMessageClass),
        }
    }
}

/// How far an outbound message provably got.
///
/// This is the sole input to the retry decision. It deliberately does not claim
/// to know whether the server began executing: once a body is committed, the
/// effect is unknown rather than absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendReceipt {
    /// Provably never left the client. Safe to retry.
    NotSent,
    /// Request headers reached the peer; the body did not fully commit.
    HeadersCommitted,
    /// The body was partially written. The peer may have parsed a prefix.
    BodyPartiallySent,
    /// The body was fully handed to the peer.
    BodyCommitted,
}

impl SendReceipt {
    /// Only a provably unsent request may be retried automatically.
    pub fn is_safely_retryable(self) -> bool {
        matches!(self, Self::NotSent)
    }

    /// True when the remote effect cannot be determined from the client side.
    pub fn is_indeterminate(self) -> bool {
        matches!(
            self,
            Self::HeadersCommitted | Self::BodyPartiallySent | Self::BodyCommitted
        )
    }
}

/// Which transport produced a message. Used for diagnostics and feature gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    Stdio,
    StreamableHttp,
    LegacySse,
}

impl McpTransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::StreamableHttp => "streamable_http",
            Self::LegacySse => "legacy_sse",
        }
    }

    /// Streamable HTTP is the only transport with a negotiated session ID.
    pub fn supports_http_session(self) -> bool {
        matches!(self, Self::StreamableHttp)
    }
}

/// Typed protocol failures. Each variant maps to a safe diagnostic; none carry
/// raw remote payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpProtocolError {
    MessageTooLarge,
    MalformedFrame,
    UnsupportedJsonRpcVersion,
    UnknownMessageClass,
    UnsupportedProtocolVersion {
        offered: String,
    },
    InvalidSessionId,
    SessionExpired,
    /// The transport closed while requests were still outstanding.
    Disconnected {
        detail: String,
    },
    /// A committed request whose remote effect cannot be determined.
    Indeterminate {
        detail: String,
    },
    Timeout {
        elapsed_ms: u64,
    },
    Cancelled,
    Transport {
        detail: String,
    },
    Server(JsonRpcError),
}

impl fmt::Display for McpProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageTooLarge => {
                write!(formatter, "MCP message exceeds the supported size")
            }
            Self::MalformedFrame => write!(formatter, "MCP frame is not valid JSON-RPC"),
            Self::UnsupportedJsonRpcVersion => {
                write!(
                    formatter,
                    "MCP frame declared an unsupported JSON-RPC version"
                )
            }
            Self::UnknownMessageClass => {
                write!(
                    formatter,
                    "MCP frame is neither request, response, nor notification"
                )
            }
            Self::UnsupportedProtocolVersion { offered } => write!(
                formatter,
                "MCP server offered unsupported protocol version {offered}"
            ),
            Self::InvalidSessionId => write!(formatter, "MCP session id is invalid"),
            Self::SessionExpired => {
                write!(formatter, "MCP session expired and must be re-initialized")
            }
            Self::Disconnected { detail } => {
                write!(formatter, "MCP transport disconnected: {detail}")
            }
            Self::Indeterminate { detail } => write!(
                formatter,
                "MCP request was committed but its remote effect is unknown: {detail}"
            ),
            Self::Timeout { elapsed_ms } => {
                write!(formatter, "MCP request timed out after {elapsed_ms}ms")
            }
            Self::Cancelled => write!(formatter, "MCP request was cancelled"),
            Self::Transport { detail } => write!(formatter, "MCP transport error: {detail}"),
            Self::Server(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for McpProtocolError {}

impl McpProtocolError {
    /// True when the caller may safely reissue the request.
    ///
    /// A committed request is never safely retryable, because a duplicate could
    /// repeat a side effect the server already performed.
    pub fn is_safely_retryable(&self) -> bool {
        matches!(self, Self::Disconnected { .. } | Self::Transport { .. })
    }

    /// True when the remote effect is unknown and must not be replayed.
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Self::Indeterminate { .. })
    }
}

/// Negotiate a protocol version from the server's initialize response.
///
/// An absent version means the server predates the header; the documented
/// default applies. An explicitly unsupported version fails closed rather than
/// letting a mismatched session proceed.
pub fn negotiate_protocol_version(offered: Option<&str>) -> Result<String, McpProtocolError> {
    let offered = match offered {
        None => return Ok(MCP_DEFAULT_NEGOTIATED_VERSION.to_string()),
        Some(version) if version.trim().is_empty() => {
            return Ok(MCP_DEFAULT_NEGOTIATED_VERSION.to_string());
        }
        Some(version) => version.trim(),
    };
    if SUPPORTED_MCP_PROTOCOL_VERSIONS.contains(&offered) {
        return Ok(offered.to_string());
    }
    Err(McpProtocolError::UnsupportedProtocolVersion {
        offered: bounded_diagnostic(offered),
    })
}

/// Validate a server-assigned session ID before it is echoed in any header.
///
/// A session ID becomes an outbound header value, so it is restricted to
/// visible ASCII without separators. This prevents header injection through a
/// hostile server-supplied identifier.
pub fn validate_session_id(candidate: &str) -> Result<String, McpProtocolError> {
    if candidate.is_empty() || candidate.len() > MAX_MCP_SESSION_ID_BYTES {
        return Err(McpProtocolError::InvalidSessionId);
    }
    let acceptable = candidate
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if !acceptable {
        return Err(McpProtocolError::InvalidSessionId);
    }
    Ok(candidate.to_string())
}

/// Validate an opaque pagination cursor supplied by the server.
pub fn validate_cursor(candidate: &str) -> Result<String, McpProtocolError> {
    if candidate.is_empty() || candidate.len() > MAX_MCP_CURSOR_BYTES {
        return Err(McpProtocolError::MalformedFrame);
    }
    Ok(candidate.to_string())
}

/// Truncate untrusted text to a bounded, single-line diagnostic.
///
/// Remote text reaches logs and safe summaries, so control characters are
/// collapsed and the length is capped.
pub fn bounded_diagnostic(value: &str) -> String {
    let collapsed: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= MAX_MCP_DIAGNOSTIC_CHARS {
        return trimmed.to_string();
    }
    let mut bounded: String = trimmed.chars().take(MAX_MCP_DIAGNOSTIC_CHARS).collect();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_response_and_notification_are_distinguished() {
        let request = JsonRpcMessage::from_slice(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
        )
        .unwrap();
        assert!(matches!(request, JsonRpcMessage::Request { .. }));

        let response =
            JsonRpcMessage::from_slice(br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
                .unwrap();
        assert!(response.is_response());
        assert_eq!(response.id(), Some(&JsonRpcId::Number(1)));

        let notification = JsonRpcMessage::from_slice(
            br#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#,
        )
        .unwrap();
        assert!(matches!(notification, JsonRpcMessage::Notification { .. }));
        assert_eq!(notification.id(), None, "a notification has no identity");
    }

    #[test]
    fn an_error_response_is_not_confused_with_a_result() {
        let message = JsonRpcMessage::from_slice(
            br#"{"jsonrpc":"2.0","id":"abc","error":{"code":-32000,"message":"boom"}}"#,
        )
        .unwrap();
        match message {
            JsonRpcMessage::ErrorResponse { id, error } => {
                assert_eq!(id, JsonRpcId::Text("abc".to_string()));
                assert_eq!(error.code, -32000);
                assert_eq!(error.message, "boom");
            }
            other => panic!("expected an error response, got {other:?}"),
        }
    }

    #[test]
    fn oversized_and_malformed_frames_are_rejected() {
        let oversized = vec![b'x'; MAX_MCP_MESSAGE_BYTES + 1];
        assert_eq!(
            JsonRpcMessage::from_slice(&oversized),
            Err(McpProtocolError::MessageTooLarge)
        );
        assert_eq!(
            JsonRpcMessage::from_slice(b"not json"),
            Err(McpProtocolError::MalformedFrame)
        );
        assert_eq!(
            JsonRpcMessage::from_slice(br#"{"jsonrpc":"1.0","id":1,"result":{}}"#),
            Err(McpProtocolError::UnsupportedJsonRpcVersion)
        );
        assert_eq!(
            JsonRpcMessage::from_slice(br#"{"jsonrpc":"2.0"}"#),
            Err(McpProtocolError::UnknownMessageClass)
        );
    }

    #[test]
    fn version_negotiation_accepts_known_versions_and_refuses_others() {
        assert_eq!(
            negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION)).unwrap(),
            MCP_PROTOCOL_VERSION
        );
        assert_eq!(
            negotiate_protocol_version(Some("2024-11-05")).unwrap(),
            "2024-11-05"
        );
        // An absent version means the server predates the header.
        assert_eq!(
            negotiate_protocol_version(None).unwrap(),
            MCP_DEFAULT_NEGOTIATED_VERSION
        );
        assert!(matches!(
            negotiate_protocol_version(Some("1999-01-01")),
            Err(McpProtocolError::UnsupportedProtocolVersion { .. })
        ));
    }

    #[test]
    fn a_session_id_that_could_inject_a_header_is_rejected() {
        assert_eq!(
            validate_session_id("abc-123_X.9:7").unwrap(),
            "abc-123_X.9:7"
        );
        for hostile in [
            "",
            "has space",
            "new\nline",
            "carriage\rreturn",
            "semi;colon",
            "quote\"mark",
        ] {
            assert_eq!(
                validate_session_id(hostile),
                Err(McpProtocolError::InvalidSessionId),
                "must reject {hostile:?}"
            );
        }
        assert_eq!(
            validate_session_id(&"a".repeat(MAX_MCP_SESSION_ID_BYTES + 1)),
            Err(McpProtocolError::InvalidSessionId)
        );
    }

    #[test]
    fn only_a_provably_unsent_request_is_retryable() {
        assert!(SendReceipt::NotSent.is_safely_retryable());
        for committed in [
            SendReceipt::HeadersCommitted,
            SendReceipt::BodyPartiallySent,
            SendReceipt::BodyCommitted,
        ] {
            assert!(
                !committed.is_safely_retryable(),
                "{committed:?} may have reached the server"
            );
            assert!(committed.is_indeterminate());
        }
    }

    #[test]
    fn a_diagnostic_is_bounded_and_single_line() {
        let bounded = bounded_diagnostic("line one\nline two\ttabbed");
        assert!(!bounded.contains('\n'));
        assert!(!bounded.contains('\t'));

        let long = bounded_diagnostic(&"x".repeat(MAX_MCP_DIAGNOSTIC_CHARS + 50));
        assert_eq!(long.chars().count(), MAX_MCP_DIAGNOSTIC_CHARS + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_round_trip_preserves_message_identity() {
        for message in [
            JsonRpcMessage::request(JsonRpcId::Number(7), "tools/list", json!({"cursor":"c"})),
            JsonRpcMessage::notification("notifications/initialized", json!({})),
            JsonRpcMessage::Response {
                id: JsonRpcId::Text("id-1".to_string()),
                result: json!({"tools":[]}),
            },
            JsonRpcMessage::ErrorResponse {
                id: JsonRpcId::Number(9),
                error: JsonRpcError::method_not_found("sampling/createMessage"),
            },
        ] {
            let encoded = serde_json::to_vec(&message.to_value()).unwrap();
            let decoded = JsonRpcMessage::from_slice(&encoded).unwrap();
            assert_eq!(decoded, message);
        }
    }
}
