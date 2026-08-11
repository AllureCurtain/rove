//! Transport adapter boundary shared by stdio, Streamable HTTP, and legacy SSE.
//!
//! An adapter reliably moves protocol messages and reports how far an outbound
//! message provably got. It deliberately does not decide tool safety, choose
//! retries, project results, or treat an HTTP success as a tool success.

use async_trait::async_trait;

use super::protocol::{JsonRpcMessage, McpProtocolError, McpTransportKind, SendReceipt};

/// Feature support for one transport, used for honest diagnostics.
///
/// A transport reports only what it actually implements. Claiming an unsupported
/// capability would let a caller believe a session has guarantees it lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportFeatures {
    pub concurrent_requests: bool,
    pub server_notifications: bool,
    pub http_session: bool,
    pub get_stream: bool,
    pub delete_session: bool,
    pub resumable_stream: bool,
    pub child_lifecycle: bool,
}

impl TransportFeatures {
    pub const fn stdio() -> Self {
        Self {
            concurrent_requests: true,
            server_notifications: true,
            http_session: false,
            get_stream: false,
            delete_session: false,
            resumable_stream: false,
            child_lifecycle: true,
        }
    }

    pub const fn streamable_http() -> Self {
        Self {
            concurrent_requests: true,
            server_notifications: true,
            http_session: true,
            get_stream: true,
            delete_session: true,
            resumable_stream: true,
            child_lifecycle: false,
        }
    }

    /// Legacy SSE is intentionally narrow: it must not advertise session,
    /// DELETE, or POST-SSE abilities it does not have.
    pub const fn legacy_sse() -> Self {
        Self {
            concurrent_requests: false,
            server_notifications: true,
            http_session: false,
            get_stream: true,
            delete_session: false,
            resumable_stream: false,
            child_lifecycle: false,
        }
    }
}

/// Why a transport is being closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Normal shutdown initiated locally.
    ClientShutdown,
    /// The session must be re-established.
    SessionExpired,
    /// A protocol violation or unrecoverable transport failure.
    Failed,
}

/// Byte/message I/O for one MCP connection.
#[async_trait]
pub trait McpTransportAdapter: Send + Sync {
    /// Send one message and report how far it provably got.
    ///
    /// The receipt is the sole input to the retry decision, so an implementation
    /// must not report `NotSent` once any byte may have reached the peer.
    async fn send(&self, message: &JsonRpcMessage) -> Result<SendReceipt, McpProtocolError>;

    /// Close the connection and release its resources.
    async fn close(&self, reason: CloseReason) -> Result<(), McpProtocolError>;

    fn transport_kind(&self) -> McpTransportKind;

    fn features(&self) -> TransportFeatures;

    /// Session ID, when the transport negotiated one.
    async fn session_id(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    use super::super::protocol::JsonRpcId;

    /// Records what was sent and replays a scripted receipt sequence.
    struct ScriptedTransport {
        sent: Mutex<Vec<JsonRpcMessage>>,
        receipts: Mutex<Vec<SendReceipt>>,
        closed: Mutex<Option<CloseReason>>,
    }

    impl ScriptedTransport {
        fn new(receipts: Vec<SendReceipt>) -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                receipts: Mutex::new(receipts),
                closed: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl McpTransportAdapter for ScriptedTransport {
        async fn send(&self, message: &JsonRpcMessage) -> Result<SendReceipt, McpProtocolError> {
            self.sent.lock().unwrap().push(message.clone());
            Ok(self
                .receipts
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(SendReceipt::BodyCommitted))
        }

        async fn close(&self, reason: CloseReason) -> Result<(), McpProtocolError> {
            *self.closed.lock().unwrap() = Some(reason);
            Ok(())
        }

        fn transport_kind(&self) -> McpTransportKind {
            McpTransportKind::Stdio
        }

        fn features(&self) -> TransportFeatures {
            TransportFeatures::stdio()
        }
    }

    #[tokio::test]
    async fn an_adapter_reports_receipts_and_records_closure() {
        let transport = ScriptedTransport::new(vec![SendReceipt::NotSent]);

        let receipt = transport
            .send(&JsonRpcMessage::request(
                JsonRpcId::Number(1),
                "tools/list",
                json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(receipt, SendReceipt::NotSent);
        assert!(receipt.is_safely_retryable());
        assert_eq!(transport.sent.lock().unwrap().len(), 1);

        transport.close(CloseReason::ClientShutdown).await.unwrap();
        assert_eq!(
            *transport.closed.lock().unwrap(),
            Some(CloseReason::ClientShutdown)
        );
    }

    #[test]
    fn transport_features_do_not_overclaim() {
        let stdio = TransportFeatures::stdio();
        assert!(!stdio.http_session, "stdio has no HTTP session");
        assert!(!stdio.delete_session);
        assert!(stdio.child_lifecycle);

        let http = TransportFeatures::streamable_http();
        assert!(http.http_session && http.delete_session && http.resumable_stream);
        assert!(!http.child_lifecycle, "HTTP owns no child process");

        // Legacy SSE must not claim Streamable HTTP abilities.
        let legacy = TransportFeatures::legacy_sse();
        assert!(!legacy.http_session);
        assert!(!legacy.delete_session);
        assert!(!legacy.resumable_stream);
        assert!(!legacy.concurrent_requests);
    }
}
