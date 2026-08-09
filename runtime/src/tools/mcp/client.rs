//! Protocol client core over the shared dispatcher and a transport adapter.
//!
//! Owns initialize/version/session, request lifecycle, catalog discovery, and
//! the retry decision. It is transport-agnostic: stdio, Streamable HTTP, and
//! legacy SSE all reach the same protocol semantics through this type.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::catalog::{CatalogBuilder, McpCatalogSnapshot, parse_catalog_page};
use super::dispatcher::{InboundEvent, SharedDispatcher};
use super::protocol::{JsonRpcMessage, McpProtocolError, McpTransportKind, bounded_diagnostic};
use super::streamable_http::StreamableHttpTransport;
use super::transport::{CloseReason, McpTransportAdapter, TransportFeatures};

/// Request-level policy for the client core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpClientPolicy {
    pub request_timeout_ms: u64,
    /// Retries permitted for a provably unsent request.
    pub max_retries: u32,
}

impl Default for McpClientPolicy {
    fn default() -> Self {
        Self {
            request_timeout_ms: 30_000,
            max_retries: 1,
        }
    }
}

/// Client core bound to a Streamable HTTP transport.
///
/// The transport is concrete rather than boxed because Streamable HTTP is the
/// only transport that carries session/version headers through this path today;
/// stdio and legacy SSE keep their existing clients while sharing the protocol,
/// dispatcher, and catalog modules.
pub struct StreamableHttpClient {
    transport: Arc<StreamableHttpTransport>,
    dispatcher: SharedDispatcher,
    policy: McpClientPolicy,
    server_name: String,
}

impl StreamableHttpClient {
    pub fn new(
        server_name: impl Into<String>,
        transport: StreamableHttpTransport,
        policy: McpClientPolicy,
    ) -> Self {
        let dispatcher = transport.dispatcher();
        Self {
            transport: Arc::new(transport),
            dispatcher,
            policy,
            server_name: server_name.into(),
        }
    }

    pub fn transport_kind(&self) -> McpTransportKind {
        McpTransportKind::StreamableHttp
    }

    pub fn features(&self) -> TransportFeatures {
        TransportFeatures::streamable_http()
    }

    pub async fn session_id(&self) -> Option<String> {
        self.transport.session_state().await.session_id
    }

    pub async fn negotiated_version(&self) -> Option<String> {
        self.transport.session_state().await.protocol_version
    }

    /// Perform the initialize handshake and send `notifications/initialized`.
    pub async fn initialize(&self) -> Result<Value, McpProtocolError> {
        let params = json!({
            "protocolVersion": super::protocol::MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "rove", "version": env!("CARGO_PKG_VERSION") }
        });
        let (result, session_header) = self.request_capturing_session("initialize", params).await?;
        self.transport
            .record_initialize(&result, session_header.as_deref())
            .await?;
        // The server learns the handshake completed; a failure here is not fatal
        // to a session that already negotiated successfully.
        let _ = self
            .transport
            .post_message(&JsonRpcMessage::notification(
                "notifications/initialized",
                json!({}),
            ))
            .await;
        Ok(result)
    }

    /// Issue one request and await its correlated response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, McpProtocolError> {
        self.request_capturing_session(method, params)
            .await
            .map(|(result, _)| result)
    }

    async fn request_capturing_session(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(Value, Option<String>), McpProtocolError> {
        let mut attempt = 0_u32;
        loop {
            let id = self.dispatcher.next_id();
            let pending = self.dispatcher.register(id.clone()).await?;
            let message = JsonRpcMessage::request(id.clone(), method, params.clone());

            match self.transport.post_message(&message).await {
                Ok(outcome) => {
                    let result = self.await_response(pending).await?;
                    return Ok((result, outcome.session_header));
                }
                Err(error) => {
                    self.dispatcher.abandon(&id, error.clone()).await;
                    // Only a provably unsent request may be reissued. A
                    // committed one could duplicate a remote side effect.
                    let retryable =
                        error.is_safely_retryable() && attempt < self.policy.max_retries;
                    if !retryable {
                        return Err(error);
                    }
                    attempt += 1;
                }
            }
        }
    }

    async fn await_response(
        &self,
        pending: super::dispatcher::PendingRequest,
    ) -> Result<Value, McpProtocolError> {
        let timeout = Duration::from_millis(self.policy.request_timeout_ms);
        match tokio::time::timeout(timeout, pending.wait()).await {
            Ok(result) => result,
            Err(_) => Err(McpProtocolError::Timeout {
                elapsed_ms: self.policy.request_timeout_ms,
            }),
        }
    }

    /// Discover the complete tool catalog, following pagination.
    ///
    /// Discovery is all-or-nothing: a failure anywhere aborts the catalog rather
    /// than registering a partial tool set.
    pub async fn discover_catalog(&self) -> Result<McpCatalogSnapshot, McpProtocolError> {
        let version = self
            .negotiated_version()
            .await
            .unwrap_or_else(|| super::protocol::MCP_PROTOCOL_VERSION.to_string());
        let mut builder = CatalogBuilder::new(self.server_name.clone(), version);
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            let page = parse_catalog_page(&self.server_name, &result)?;
            match builder.push_page(page)? {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        builder.finish()
    }

    /// Call one remote tool by its exact remote name.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpProtocolError> {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    /// Drain inbound notifications and unsupported server requests.
    pub async fn take_events(&self) -> Vec<InboundEvent> {
        self.dispatcher.take_events().await
    }

    /// Open the optional GET notification stream, returning routed message count.
    pub async fn poll_notifications(&self) -> Result<usize, McpProtocolError> {
        self.transport.open_notification_stream().await
    }

    /// True when a `tools/list_changed` notification has been observed.
    pub async fn tools_changed(&self) -> bool {
        self.dispatcher.take_events().await.iter().any(|event| {
            matches!(
                event,
                InboundEvent::Notification { method, .. }
                    if method == "notifications/tools/list_changed"
            )
        })
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.transport.cancel_token()
    }

    /// Close the session, releasing the server session when one exists.
    pub async fn close(&self) -> Result<(), McpProtocolError> {
        McpTransportAdapter::close(self.transport.as_ref(), CloseReason::ClientShutdown).await
    }

    /// Safe one-line description for diagnostics.
    pub fn safe_summary(&self) -> String {
        bounded_diagnostic(&format!(
            "{} via {}",
            self.server_name,
            self.transport_kind().as_str()
        ))
    }
}
