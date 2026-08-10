//! Streamable HTTP transport.
//!
//! Implements the POST-JSON, POST-SSE, accepted/no-body, session-header,
//! version-header, GET-stream, and DELETE shapes of the Streamable HTTP
//! transport on top of the shared dispatcher.
//!
//! Two rules drive the error handling:
//!
//! - A send receipt decides retry. Once a request body is committed the remote
//!   effect is unknown, so the failure is reported as typed indeterminate rather
//!   than retried, because a duplicate `tools/call` could repeat a side effect.
//! - HTTP success is not tool success. A 200 only means the transport worked;
//!   the JSON-RPC body still decides the outcome.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::dispatcher::{JsonRpcDispatcher, SharedDispatcher};
use super::http_safety::{
    HttpEndpointPolicy, MCP_JSON_CONTENT_TYPE, MCP_SSE_CONTENT_TYPE, McpResponseKind,
    SseFrameParser, ValidatedEndpoint, classify_response, validate_endpoint, validate_redirect,
};
use super::protocol::{
    JsonRpcMessage, MAX_MCP_MESSAGE_BYTES, MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER,
    MCP_SESSION_ID_HEADER, McpProtocolError, McpTransportKind, SendReceipt, bounded_diagnostic,
    negotiate_protocol_version, validate_session_id,
};
use super::transport::{CloseReason, McpTransportAdapter, TransportFeatures};

/// Bound on one SSE response body.
pub const MAX_MCP_SSE_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Bound on reconnect attempts for a resumable stream.
pub const MAX_MCP_RECONNECT_ATTEMPTS: u32 = 3;

/// Tunable transport policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamableHttpPolicy {
    pub request_timeout_ms: u64,
    pub endpoint: HttpEndpointPolicy,
    pub max_reconnect_attempts: u32,
}

impl Default for StreamableHttpPolicy {
    fn default() -> Self {
        Self {
            request_timeout_ms: 30_000,
            endpoint: HttpEndpointPolicy::default(),
            max_reconnect_attempts: MAX_MCP_RECONNECT_ATTEMPTS,
        }
    }
}

/// Negotiated session state for one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpSessionState {
    pub session_id: Option<String>,
    pub protocol_version: Option<String>,
    /// Hash of the validated initialize serverInfo/capabilities projection.
    pub server_identity_hash: Option<String>,
    /// Last SSE event ID, used to resume a dropped stream.
    pub last_event_id: Option<String>,
}

/// Streamable HTTP connection state.
///
/// `Debug` is derived deliberately: the endpoint is validated to exclude
/// userinfo, so no credential can reach a debug rendering.
#[derive(Debug)]
pub struct StreamableHttpTransport {
    http: reqwest::Client,
    endpoint: ValidatedEndpoint,
    policy: StreamableHttpPolicy,
    dispatcher: SharedDispatcher,
    session: Arc<Mutex<HttpSessionState>>,
    cancel: CancellationToken,
}

impl StreamableHttpTransport {
    /// Validate `url` and build a transport. No request is issued here.
    pub fn connect(
        url: &str,
        policy: StreamableHttpPolicy,
        dispatcher: SharedDispatcher,
    ) -> Result<Self, McpProtocolError> {
        let endpoint = validate_endpoint(url, policy.endpoint)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(policy.request_timeout_ms))
            // Redirects are re-validated explicitly so a hop cannot silently
            // move the session to an unvetted host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpProtocolError::Transport {
                detail: bounded_diagnostic(&error.to_string()),
            })?;
        Ok(Self {
            http,
            endpoint,
            policy,
            dispatcher,
            session: Arc::new(Mutex::new(HttpSessionState::default())),
            cancel: CancellationToken::new(),
        })
    }

    pub fn dispatcher(&self) -> SharedDispatcher {
        Arc::clone(&self.dispatcher)
    }

    pub async fn session_state(&self) -> HttpSessionState {
        self.session.lock().await.clone()
    }

    /// Record the negotiated version and session ID from an initialize result.
    pub async fn record_initialize(
        &self,
        result: &serde_json::Value,
        session_header: Option<&str>,
    ) -> Result<(), McpProtocolError> {
        let offered = result.get("protocolVersion").and_then(|v| v.as_str());
        let negotiated = negotiate_protocol_version(offered)?;
        let identity_hash = super::protocol::server_identity_hash(result)?;
        let session_id = session_header.map(validate_session_id).transpose()?;
        let mut session = self.session.lock().await;
        session.protocol_version = Some(negotiated);
        session.server_identity_hash = Some(identity_hash);
        session.session_id = session_id;
        session.last_event_id = None;
        Ok(())
    }

    async fn headers(&self, accept_sse: bool) -> Result<HeaderMap, McpProtocolError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(MCP_JSON_CONTENT_TYPE),
        );
        let accept = if accept_sse {
            HeaderValue::from_static("application/json, text/event-stream")
        } else {
            HeaderValue::from_static(MCP_JSON_CONTENT_TYPE)
        };
        headers.insert(ACCEPT, accept);

        let session = self.session.lock().await;
        // The negotiated version is echoed on every post-initialize request.
        let version = session
            .protocol_version
            .clone()
            .unwrap_or_else(|| MCP_PROTOCOL_VERSION.to_string());
        headers.insert(
            HeaderName::from_static(MCP_PROTOCOL_VERSION_HEADER),
            HeaderValue::from_str(&version).map_err(|_| McpProtocolError::Transport {
                detail: "negotiated MCP protocol version is not a valid header".to_string(),
            })?,
        );
        if let Some(session_id) = &session.session_id {
            // Already validated on receipt; re-checked here so a stored value
            // can never become an injected header.
            let validated = validate_session_id(session_id)?;
            headers.insert(
                HeaderName::from_static(MCP_SESSION_ID_HEADER),
                HeaderValue::from_str(&validated)
                    .map_err(|_| McpProtocolError::InvalidSessionId)?,
            );
        }
        Ok(headers)
    }

    /// POST one message and route whatever the response contains.
    ///
    /// Returns the session header when the server assigned one.
    pub async fn post_message(
        &self,
        message: &JsonRpcMessage,
    ) -> Result<PostOutcome, McpProtocolError> {
        let body =
            serde_json::to_vec(&message.to_value()).map_err(|_| McpProtocolError::Transport {
                detail: "MCP request could not be encoded".to_string(),
            })?;
        if body.len() > MAX_MCP_MESSAGE_BYTES {
            return Err(McpProtocolError::MessageTooLarge);
        }
        let headers = self.headers(true).await?;
        if self.cancel.is_cancelled() {
            return Err(McpProtocolError::Cancelled);
        }

        let response = tokio::select! {
            _ = self.cancel.cancelled() => return Err(McpProtocolError::Indeterminate {
                detail: "request cancelled after dispatch began".to_string(),
            }),
            result = self
                .http
                .post(self.endpoint.as_str())
                .headers(headers)
                .body(body)
                .send() => result,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                // A connect/timeout failure never delivered a body; anything
                // later may have. `SendReceipt` records which case applies.
                let receipt = if error.is_connect() {
                    SendReceipt::NotSent
                } else if error.is_timeout() {
                    SendReceipt::BodyCommitted
                } else {
                    SendReceipt::BodyPartiallySent
                };
                return Err(self.classify_send_failure(receipt, &error.to_string()));
            }
        };

        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            // Validated for diagnostics; the hop is not followed automatically.
            validate_redirect(&self.endpoint, &location, self.policy.endpoint)
                .map_err(McpProtocolError::after_commit)?;
            return Err(McpProtocolError::Transport {
                detail: "MCP endpoint redirect requires explicit reconfiguration".to_string(),
            }
            .after_commit());
        }

        let session_header = response
            .headers()
            .get(MCP_SESSION_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let has_body = response.content_length().is_none_or(|length| length > 0);

        let kind = classify_response(status, content_type.as_deref(), has_body)
            .map_err(McpProtocolError::after_commit)?;
        let outcome = match kind {
            McpResponseKind::Empty => Ok(PostOutcome {
                session_header,
                routed_messages: 0,
                kind,
            }),
            McpResponseKind::Json => {
                let bytes = self
                    .read_bounded_body(response)
                    .await
                    .map_err(McpProtocolError::after_commit)?;
                let routed = self
                    .route_frame(&bytes)
                    .await
                    .map_err(McpProtocolError::after_commit)?;
                Ok(PostOutcome {
                    session_header,
                    routed_messages: routed,
                    kind,
                })
            }
            McpResponseKind::EventStream => {
                let routed = self
                    .consume_sse(response)
                    .await
                    .map_err(McpProtocolError::after_commit)?;
                Ok(PostOutcome {
                    session_header,
                    routed_messages: routed,
                    kind,
                })
            }
        };
        outcome.map_err(McpProtocolError::after_commit)
    }

    /// Translate a send failure into a retry-safe or indeterminate error.
    fn classify_send_failure(&self, receipt: SendReceipt, detail: &str) -> McpProtocolError {
        let detail = bounded_diagnostic(detail);
        if receipt.is_safely_retryable() {
            return McpProtocolError::NotSent { detail };
        }
        McpProtocolError::Indeterminate { detail }
    }

    async fn read_bounded_body(
        &self,
        response: reqwest::Response,
    ) -> Result<Vec<u8>, McpProtocolError> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MCP_MESSAGE_BYTES as u64)
        {
            return Err(McpProtocolError::MessageTooLarge);
        }
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| McpProtocolError::Transport {
                detail: bounded_diagnostic(&error.to_string()),
            })?;
            // Decompressed size is bounded too, so a compressed payload cannot
            // expand past the limit.
            if body.len().saturating_add(chunk.len()) > MAX_MCP_MESSAGE_BYTES {
                return Err(McpProtocolError::MessageTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn route_frame(&self, bytes: &[u8]) -> Result<usize, McpProtocolError> {
        // A JSON body may hold one message or a batch.
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| McpProtocolError::MalformedFrame)?;
        let frames = match value {
            serde_json::Value::Array(items) => items,
            single => vec![single],
        };
        let mut routed = 0;
        for frame in frames {
            let message = JsonRpcMessage::from_value(frame)?;
            self.route_message(message).await?;
            routed += 1;
        }
        Ok(routed)
    }

    async fn route_message(&self, message: JsonRpcMessage) -> Result<(), McpProtocolError> {
        if let Some(reply) = self.dispatcher.dispatch(message).await {
            // A server request must be answered even mid-response.
            self.send(&reply).await?;
        }
        Ok(())
    }

    async fn consume_sse(&self, response: reqwest::Response) -> Result<usize, McpProtocolError> {
        let mut parser = SseFrameParser::new(MAX_MCP_SSE_BODY_BYTES);
        let mut stream = response.bytes_stream();
        let mut routed = 0;
        let mut total = 0_usize;

        loop {
            let chunk = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Err(McpProtocolError::Cancelled),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|error| McpProtocolError::Transport {
                detail: bounded_diagnostic(&error.to_string()),
            })?;
            total = total.saturating_add(chunk.len());
            if total > MAX_MCP_SSE_BODY_BYTES {
                return Err(McpProtocolError::MessageTooLarge);
            }
            let text = String::from_utf8_lossy(&chunk).to_string();
            for frame in parser.push(&text)? {
                if let Some(id) = frame.id.clone() {
                    // Remembered so a dropped stream can resume from here.
                    self.session.lock().await.last_event_id = Some(id);
                }
                if frame.data.trim().is_empty() {
                    continue;
                }
                match JsonRpcMessage::from_slice(frame.data.as_bytes()) {
                    Ok(message) => {
                        self.route_message(message).await?;
                        routed += 1;
                    }
                    Err(error) => {
                        // One bad frame is tolerated; a stream of them fails.
                        if let Some(terminal) = self.dispatcher.record_invalid_frame(error).await {
                            return Err(terminal);
                        }
                    }
                }
            }
        }
        Ok(routed)
    }

    /// Open the optional GET notification stream.
    ///
    /// `Last-Event-ID` is sent when a prior stream reported one, so a reconnect
    /// resumes rather than replaying from the beginning.
    pub async fn open_notification_stream(&self) -> Result<usize, McpProtocolError> {
        let mut headers = self.headers(true).await?;
        headers.insert(ACCEPT, HeaderValue::from_static(MCP_SSE_CONTENT_TYPE));
        if let Some(last_event_id) = self.session.lock().await.last_event_id.clone()
            && let Ok(value) = HeaderValue::from_str(&last_event_id)
        {
            headers.insert(HeaderName::from_static("last-event-id"), value);
        }

        let response = self
            .http
            .get(self.endpoint.as_str())
            .headers(headers)
            .send()
            .await
            .map_err(|error| McpProtocolError::Transport {
                detail: bounded_diagnostic(&error.to_string()),
            })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        // A server that does not offer the stream answers 405; that is not a
        // failure of the session.
        if status == 405 {
            return Ok(0);
        }
        match classify_response(status, content_type.as_deref(), true)? {
            McpResponseKind::EventStream => self.consume_sse(response).await,
            McpResponseKind::Empty => Ok(0),
            McpResponseKind::Json => {
                let bytes = self.read_bounded_body(response).await?;
                self.route_frame(&bytes).await
            }
        }
    }

    /// Explicitly terminate the session with DELETE, when one exists.
    pub async fn delete_session(&self) -> Result<bool, McpProtocolError> {
        let session_id = self.session.lock().await.session_id.clone();
        let Some(_) = session_id else {
            return Ok(false);
        };
        let headers = self.headers(false).await?;
        let response = self
            .http
            .delete(self.endpoint.as_str())
            .headers(headers)
            .send()
            .await
            .map_err(|error| McpProtocolError::Transport {
                detail: bounded_diagnostic(&error.to_string()),
            })?;
        let status = response.status().as_u16();
        let mut session = self.session.lock().await;
        session.session_id = None;
        session.last_event_id = None;
        // 405 means the server keeps session lifetime for itself; the local
        // session is still released.
        Ok(status == 405 || (200..300).contains(&status) || status == 404)
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// What one POST produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostOutcome {
    pub session_header: Option<String>,
    pub routed_messages: usize,
    pub kind: McpResponseKind,
}

#[async_trait]
impl McpTransportAdapter for StreamableHttpTransport {
    async fn send(&self, message: &JsonRpcMessage) -> Result<SendReceipt, McpProtocolError> {
        self.post_message(message).await?;
        Ok(SendReceipt::BodyCommitted)
    }

    async fn close(&self, reason: CloseReason) -> Result<(), McpProtocolError> {
        self.cancel.cancel();
        if matches!(reason, CloseReason::ClientShutdown) {
            let _ = self.delete_session().await;
        }
        self.dispatcher
            .fail_all(McpProtocolError::Disconnected {
                detail: "transport closed".to_string(),
            })
            .await;
        Ok(())
    }

    fn transport_kind(&self) -> McpTransportKind {
        McpTransportKind::StreamableHttp
    }

    fn features(&self) -> TransportFeatures {
        TransportFeatures::streamable_http()
    }

    async fn session_id(&self) -> Option<String> {
        self.session.lock().await.session_id.clone()
    }
}

/// Build a transport sharing a fresh dispatcher.
pub fn streamable_http_transport(
    url: &str,
    policy: StreamableHttpPolicy,
) -> Result<StreamableHttpTransport, McpProtocolError> {
    StreamableHttpTransport::connect(url, policy, Arc::new(JsonRpcDispatcher::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn loopback_policy() -> StreamableHttpPolicy {
        StreamableHttpPolicy {
            request_timeout_ms: 2_000,
            endpoint: HttpEndpointPolicy::loopback_permitted(),
            max_reconnect_attempts: 2,
        }
    }

    #[test]
    fn a_non_loopback_plaintext_endpoint_is_refused() {
        let error = streamable_http_transport("http://example.com/mcp", loopback_policy())
            .expect_err("plaintext to a public host must be refused");
        assert!(matches!(error, McpProtocolError::Transport { .. }));
    }

    #[test]
    fn an_https_endpoint_is_accepted_under_the_default_policy() {
        let transport = streamable_http_transport(
            "https://mcp.example.com/rpc",
            StreamableHttpPolicy::default(),
        )
        .unwrap();
        assert_eq!(transport.transport_kind(), McpTransportKind::StreamableHttp);
        assert!(transport.features().http_session);
    }

    #[tokio::test]
    async fn initialize_records_the_negotiated_version_and_validated_session() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();

        transport
            .record_initialize(
                &json!({
                    "protocolVersion": "2025-03-26",
                    "serverInfo": { "name": "fixture", "version": "1" }
                }),
                Some("session-abc.123"),
            )
            .await
            .unwrap();

        let state = transport.session_state().await;
        assert_eq!(state.protocol_version.as_deref(), Some("2025-03-26"));
        assert_eq!(state.session_id.as_deref(), Some("session-abc.123"));
        assert_eq!(
            transport.session_id().await.as_deref(),
            Some("session-abc.123")
        );
    }

    #[tokio::test]
    async fn a_hostile_session_header_is_refused() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();

        let error = transport
            .record_initialize(
                &json!({ "serverInfo": { "name": "fixture", "version": "1" } }),
                Some("bad\r\nInjected: yes"),
            )
            .await
            .expect_err("a header-injecting session id must be refused");
        assert_eq!(error, McpProtocolError::InvalidSessionId);
        let state = transport.session_state().await;
        assert!(state.session_id.is_none());
        assert!(state.protocol_version.is_none());
        assert!(state.server_identity_hash.is_none());
    }

    #[tokio::test]
    async fn an_unsupported_protocol_version_fails_closed() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();

        let error = transport
            .record_initialize(
                &json!({
                    "protocolVersion": "1999-01-01",
                    "serverInfo": { "name": "fixture", "version": "1" }
                }),
                None,
            )
            .await
            .expect_err("an unknown version must not be assumed compatible");
        assert!(matches!(
            error,
            McpProtocolError::UnsupportedProtocolVersion { .. }
        ));
    }

    #[tokio::test]
    async fn an_absent_version_uses_the_documented_default() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        transport
            .record_initialize(
                &json!({ "serverInfo": { "name": "fixture", "version": "1" } }),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            transport.session_state().await.protocol_version.as_deref(),
            Some(super::super::protocol::MCP_DEFAULT_NEGOTIATED_VERSION)
        );
    }

    #[tokio::test]
    async fn headers_carry_the_version_and_session_without_injection() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        transport
            .record_initialize(
                &json!({
                    "protocolVersion": "2025-06-18",
                    "serverInfo": { "name": "fixture", "version": "1" }
                }),
                Some("sid-1"),
            )
            .await
            .unwrap();

        let headers = transport.headers(true).await.unwrap();
        assert_eq!(
            headers.get(MCP_PROTOCOL_VERSION_HEADER).unwrap(),
            "2025-06-18"
        );
        assert_eq!(headers.get(MCP_SESSION_ID_HEADER).unwrap(), "sid-1");
        assert_eq!(
            headers.get(ACCEPT).unwrap(),
            "application/json, text/event-stream"
        );
    }

    #[tokio::test]
    async fn a_connection_failure_is_retryable_and_a_committed_one_is_indeterminate() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();

        let retryable = transport.classify_send_failure(SendReceipt::NotSent, "connect refused");
        assert!(retryable.is_safely_retryable());
        assert!(!retryable.is_indeterminate());

        for committed in [
            SendReceipt::HeadersCommitted,
            SendReceipt::BodyPartiallySent,
            SendReceipt::BodyCommitted,
        ] {
            let error = transport.classify_send_failure(committed, "reset after send");
            assert!(
                error.is_indeterminate(),
                "{committed:?} must not be replayed"
            );
            assert!(!error.is_safely_retryable());
        }
    }

    #[tokio::test]
    async fn deleting_a_session_that_was_never_established_is_a_no_op() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        assert!(!transport.delete_session().await.unwrap());
    }

    #[tokio::test]
    async fn closing_fails_every_outstanding_request() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        let dispatcher = transport.dispatcher();
        let pending = dispatcher.register(dispatcher.next_id()).await.unwrap();

        transport.close(CloseReason::Failed).await.unwrap();

        let error = pending.wait().await.unwrap_err();
        assert!(matches!(error, McpProtocolError::Disconnected { .. }));
        assert!(transport.cancel_token().is_cancelled());
    }

    #[tokio::test]
    async fn a_batched_json_body_routes_every_message() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        let dispatcher = transport.dispatcher();
        let first = dispatcher.register(dispatcher.next_id()).await.unwrap();
        let second = dispatcher.register(dispatcher.next_id()).await.unwrap();

        let body = json!([
            { "jsonrpc": "2.0", "id": 1, "result": { "n": 1 } },
            { "jsonrpc": "2.0", "id": 2, "result": { "n": 2 } },
        ]);
        let routed = transport
            .route_frame(serde_json::to_vec(&body).unwrap().as_slice())
            .await
            .unwrap();

        assert_eq!(routed, 2);
        assert_eq!(first.wait().await.unwrap()["n"], 1);
        assert_eq!(second.wait().await.unwrap()["n"], 2);
    }

    #[tokio::test]
    async fn a_malformed_json_body_is_rejected() {
        let transport =
            streamable_http_transport("http://127.0.0.1:9/mcp", loopback_policy()).unwrap();
        assert_eq!(
            transport.route_frame(b"not json").await,
            Err(McpProtocolError::MalformedFrame)
        );
    }
}
