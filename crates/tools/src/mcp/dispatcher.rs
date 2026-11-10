//! One JSON-RPC dispatcher shared by every MCP transport.
//!
//! The dispatcher owns correlation, not I/O. A transport hands it inbound
//! frames and it routes each one by message class:
//!
//! - a response resolves exactly one pending request, by ID;
//! - a notification goes to the notification sink;
//! - a server-to-client request gets a reply (a typed `method not found` for
//!   anything this client does not implement) so the peer is never left waiting;
//! - an unmatched response is counted as a protocol anomaly rather than
//!   silently dropped.
//!
//! Because pending requests live in a table keyed by ID, multiple requests can
//! be outstanding at once. The previous per-client mutex serialized every call
//! and dropped any frame whose ID did not match the one request being awaited.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::sync::{Mutex, oneshot};

use super::protocol::{
    JsonRpcError, JsonRpcId, JsonRpcMessage, McpProtocolError, bounded_diagnostic,
};

/// Bound on simultaneously outstanding requests for one server.
pub const MAX_MCP_PENDING_REQUESTS: usize = 256;

/// Bound on consecutive undecodable inbound frames before the session fails.
///
/// A peer that streams garbage must not be able to keep a session alive
/// indefinitely, but one corrupt frame should not tear down healthy work.
pub const MAX_MCP_INVALID_FRAMES: u32 = 8;

/// A notification or server request the dispatcher could not answer itself.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundEvent {
    Notification {
        method: String,
        params: Value,
    },
    /// A server-to-client request that was answered with `method not found`.
    UnsupportedServerRequest {
        id: JsonRpcId,
        method: String,
    },
}

/// Outcome counters used for diagnostics and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatcherStats {
    pub responses_routed: u64,
    pub notifications_routed: u64,
    pub server_requests_answered: u64,
    pub unmatched_responses: u64,
    pub invalid_frames: u64,
}

#[derive(Debug)]
struct DispatcherState {
    pending: HashMap<JsonRpcId, oneshot::Sender<Result<Value, McpProtocolError>>>,
    events: Vec<InboundEvent>,
    stats: DispatcherStats,
    consecutive_invalid_frames: u32,
    /// Set once the session fails; every later request fails the same way.
    terminal: Option<McpProtocolError>,
}

/// Shared correlation state for one MCP connection.
#[derive(Debug)]
pub struct JsonRpcDispatcher {
    next_id: AtomicU64,
    state: Mutex<DispatcherState>,
}

/// A registered request awaiting its response.
#[derive(Debug)]
pub struct PendingRequest {
    pub id: JsonRpcId,
    receiver: oneshot::Receiver<Result<Value, McpProtocolError>>,
}

impl PendingRequest {
    /// Await the response for this request.
    ///
    /// Returns the transport/protocol error if the session failed while the
    /// request was outstanding.
    pub async fn wait(self) -> Result<Value, McpProtocolError> {
        match self.receiver.await {
            Ok(result) => result,
            // The sender is dropped only when the pending entry is discarded
            // without a verdict, which means the connection went away.
            Err(_) => Err(McpProtocolError::Disconnected {
                detail: "connection closed before the response arrived".to_string(),
            }),
        }
    }
}

impl Default for JsonRpcDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonRpcDispatcher {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            state: Mutex::new(DispatcherState {
                pending: HashMap::new(),
                events: Vec::new(),
                stats: DispatcherStats::default(),
                consecutive_invalid_frames: 0,
                terminal: None,
            }),
        }
    }

    /// Allocate a fresh request ID. IDs are never reused within a connection.
    pub fn next_id(&self) -> JsonRpcId {
        JsonRpcId::Number(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Register a request before it is written to the transport.
    ///
    /// Registering first closes the race where a fast server responds before the
    /// caller has somewhere to receive it.
    pub async fn register(&self, id: JsonRpcId) -> Result<PendingRequest, McpProtocolError> {
        let mut state = self.state.lock().await;
        if let Some(terminal) = state.terminal.clone() {
            return Err(terminal);
        }
        if state.pending.len() >= MAX_MCP_PENDING_REQUESTS {
            return Err(McpProtocolError::Transport {
                detail: "too many concurrent MCP requests".to_string(),
            });
        }
        let (sender, receiver) = oneshot::channel();
        state.pending.insert(id.clone(), sender);
        Ok(PendingRequest { id, receiver })
    }

    /// Discard a pending entry whose request never reached the transport.
    ///
    /// Used when the write fails provably before commit, so the table does not
    /// keep an entry no response will ever arrive for.
    pub async fn abandon(&self, id: &JsonRpcId, error: McpProtocolError) {
        let mut state = self.state.lock().await;
        if let Some(sender) = state.pending.remove(id) {
            let _ = sender.send(Err(error));
        }
    }

    /// Route one already-parsed inbound message.
    pub async fn dispatch(&self, message: JsonRpcMessage) -> Option<JsonRpcMessage> {
        let mut state = self.state.lock().await;
        state.consecutive_invalid_frames = 0;
        match message {
            JsonRpcMessage::Response { id, result } => {
                match state.pending.remove(&id) {
                    Some(sender) => {
                        state.stats.responses_routed += 1;
                        let _ = sender.send(Ok(result));
                    }
                    // A response for an unknown ID is a peer anomaly. It is
                    // counted rather than dropped so it stays diagnosable.
                    None => state.stats.unmatched_responses += 1,
                }
                None
            }
            JsonRpcMessage::ErrorResponse { id, error } => {
                match state.pending.remove(&id) {
                    Some(sender) => {
                        state.stats.responses_routed += 1;
                        let _ = sender.send(Err(McpProtocolError::Server(error)));
                    }
                    None => state.stats.unmatched_responses += 1,
                }
                None
            }
            JsonRpcMessage::Notification { method, params } => {
                state.stats.notifications_routed += 1;
                state
                    .events
                    .push(InboundEvent::Notification { method, params });
                None
            }
            // A server request must always be answered. This client implements
            // no server-initiated methods yet, so it replies with the reserved
            // JSON-RPC error instead of leaving the peer waiting or letting the
            // request look like an accepted capability.
            JsonRpcMessage::Request { id, method, .. } => {
                state.stats.server_requests_answered += 1;
                state.events.push(InboundEvent::UnsupportedServerRequest {
                    id: id.clone(),
                    method: method.clone(),
                });
                Some(JsonRpcMessage::ErrorResponse {
                    id,
                    error: JsonRpcError::method_not_found(&method),
                })
            }
        }
    }

    /// Record an undecodable frame.
    ///
    /// Returns the terminal error once the consecutive-anomaly threshold is
    /// crossed, at which point the caller must fail the session.
    pub async fn record_invalid_frame(&self, error: McpProtocolError) -> Option<McpProtocolError> {
        let mut state = self.state.lock().await;
        state.stats.invalid_frames += 1;
        state.consecutive_invalid_frames += 1;
        if state.consecutive_invalid_frames < MAX_MCP_INVALID_FRAMES {
            return None;
        }
        let terminal = McpProtocolError::Transport {
            detail: bounded_diagnostic(&format!(
                "{} consecutive invalid MCP frames; last error: {error}",
                state.consecutive_invalid_frames
            )),
        };
        Some(terminal)
    }

    /// Fail every outstanding request with the same cause.
    ///
    /// A disconnect is not a cancellation: each waiter learns why it failed, and
    /// a committed request surfaces as indeterminate rather than retryable.
    pub async fn fail_all(&self, error: McpProtocolError) {
        let mut state = self.state.lock().await;
        if state.terminal.is_none() {
            state.terminal = Some(error.clone());
        }
        let pending: Vec<_> = state.pending.drain().collect();
        for (_, sender) in pending {
            let _ = sender.send(Err(error.clone()));
        }
    }

    /// Number of requests currently awaiting a response.
    pub async fn pending_count(&self) -> usize {
        self.state.lock().await.pending.len()
    }

    /// Drain the inbound events observed so far.
    pub async fn take_events(&self) -> Vec<InboundEvent> {
        std::mem::take(&mut self.state.lock().await.events)
    }

    pub async fn stats(&self) -> DispatcherStats {
        self.state.lock().await.stats
    }

    /// The terminal error, if the session has already failed.
    pub async fn terminal_error(&self) -> Option<McpProtocolError> {
        self.state.lock().await.terminal.clone()
    }
}

/// Convenience alias for the shared dispatcher handle.
pub type SharedDispatcher = Arc<JsonRpcDispatcher>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn concurrent_requests_are_correlated_by_id_out_of_order() {
        let dispatcher = JsonRpcDispatcher::new();
        let first = dispatcher.register(dispatcher.next_id()).await.unwrap();
        let second = dispatcher.register(dispatcher.next_id()).await.unwrap();
        let third = dispatcher.register(dispatcher.next_id()).await.unwrap();
        assert_eq!(dispatcher.pending_count().await, 3);

        // Respond in a different order than the requests were issued.
        for (id, marker) in [(third.id.clone(), "third"), (first.id.clone(), "first")] {
            dispatcher
                .dispatch(JsonRpcMessage::Response {
                    id,
                    result: json!({ "marker": marker }),
                })
                .await;
        }
        dispatcher
            .dispatch(JsonRpcMessage::ErrorResponse {
                id: second.id.clone(),
                error: JsonRpcError {
                    code: -32000,
                    message: "second failed".to_string(),
                    data: None,
                },
            })
            .await;

        assert_eq!(first.wait().await.unwrap()["marker"], "first");
        assert_eq!(third.wait().await.unwrap()["marker"], "third");
        let error = second.wait().await.unwrap_err();
        assert!(matches!(error, McpProtocolError::Server(_)), "{error:?}");
        assert_eq!(dispatcher.pending_count().await, 0);
    }

    #[tokio::test]
    async fn a_notification_is_delivered_rather_than_dropped_for_a_mismatched_id() {
        let dispatcher = JsonRpcDispatcher::new();
        let pending = dispatcher.register(dispatcher.next_id()).await.unwrap();

        let reply = dispatcher
            .dispatch(JsonRpcMessage::notification(
                "notifications/tools/list_changed",
                json!({}),
            ))
            .await;

        assert!(reply.is_none(), "a notification is never answered");
        let events = dispatcher.take_events().await;
        assert_eq!(
            events,
            vec![InboundEvent::Notification {
                method: "notifications/tools/list_changed".to_string(),
                params: json!({}),
            }]
        );
        // The unrelated pending request is untouched.
        assert_eq!(dispatcher.pending_count().await, 1);
        drop(pending);
    }

    #[tokio::test]
    async fn a_server_request_is_answered_with_method_not_found() {
        let dispatcher = JsonRpcDispatcher::new();

        let reply = dispatcher
            .dispatch(JsonRpcMessage::request(
                JsonRpcId::Number(42),
                "sampling/createMessage",
                json!({}),
            ))
            .await
            .expect("a server request must always be answered");

        match reply {
            JsonRpcMessage::ErrorResponse { id, error } => {
                assert_eq!(id, JsonRpcId::Number(42));
                assert_eq!(error.code, JsonRpcError::METHOD_NOT_FOUND);
            }
            other => panic!("expected an error response, got {other:?}"),
        }
        assert_eq!(dispatcher.stats().await.server_requests_answered, 1);
    }

    #[tokio::test]
    async fn an_unmatched_response_is_counted_and_does_not_disturb_pending_work() {
        let dispatcher = JsonRpcDispatcher::new();
        let pending = dispatcher.register(dispatcher.next_id()).await.unwrap();

        dispatcher
            .dispatch(JsonRpcMessage::Response {
                id: JsonRpcId::Number(9_999),
                result: json!({}),
            })
            .await;

        assert_eq!(dispatcher.stats().await.unmatched_responses, 1);
        assert_eq!(dispatcher.pending_count().await, 1);
        drop(pending);
    }

    #[tokio::test]
    async fn a_disconnect_fans_out_to_every_waiter_with_the_same_cause() {
        let dispatcher = JsonRpcDispatcher::new();
        let first = dispatcher.register(dispatcher.next_id()).await.unwrap();
        let second = dispatcher.register(dispatcher.next_id()).await.unwrap();

        dispatcher
            .fail_all(McpProtocolError::Disconnected {
                detail: "child exited".to_string(),
            })
            .await;

        for pending in [first, second] {
            match pending.wait().await.unwrap_err() {
                McpProtocolError::Disconnected { detail } => assert_eq!(detail, "child exited"),
                other => panic!("expected a disconnect, got {other:?}"),
            }
        }
        assert_eq!(dispatcher.pending_count().await, 0);
        // A failed session refuses new work instead of hanging.
        assert!(dispatcher.register(JsonRpcId::Number(1)).await.is_err());
    }

    #[tokio::test]
    async fn an_abandoned_request_reports_its_own_cause() {
        let dispatcher = JsonRpcDispatcher::new();
        let pending = dispatcher.register(dispatcher.next_id()).await.unwrap();
        let id = pending.id.clone();

        dispatcher
            .abandon(
                &id,
                McpProtocolError::NotSent {
                    detail: "write failed before send".to_string(),
                },
            )
            .await;

        let error = pending.wait().await.unwrap_err();
        assert!(
            error.is_safely_retryable(),
            "a provably unsent write may retry"
        );
        assert_eq!(dispatcher.pending_count().await, 0);
    }

    #[tokio::test]
    async fn invalid_frames_fail_the_session_only_after_the_threshold() {
        let dispatcher = JsonRpcDispatcher::new();

        for _ in 0..(MAX_MCP_INVALID_FRAMES - 1) {
            assert!(
                dispatcher
                    .record_invalid_frame(McpProtocolError::MalformedFrame)
                    .await
                    .is_none(),
                "one corrupt frame must not tear down healthy work"
            );
        }
        let terminal = dispatcher
            .record_invalid_frame(McpProtocolError::MalformedFrame)
            .await
            .expect("the threshold must eventually fail the session");
        assert!(matches!(terminal, McpProtocolError::Transport { .. }));
        assert_eq!(
            dispatcher.stats().await.invalid_frames,
            u64::from(MAX_MCP_INVALID_FRAMES)
        );
    }

    #[tokio::test]
    async fn a_valid_frame_resets_the_invalid_frame_streak() {
        let dispatcher = JsonRpcDispatcher::new();
        for _ in 0..(MAX_MCP_INVALID_FRAMES - 1) {
            dispatcher
                .record_invalid_frame(McpProtocolError::MalformedFrame)
                .await;
        }

        dispatcher
            .dispatch(JsonRpcMessage::notification(
                "notifications/progress",
                json!({}),
            ))
            .await;

        // The streak restarted, so the next bad frame is not terminal.
        assert!(
            dispatcher
                .record_invalid_frame(McpProtocolError::MalformedFrame)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_pending_table_is_bounded() {
        let dispatcher = JsonRpcDispatcher::new();
        let mut held = Vec::new();
        for _ in 0..MAX_MCP_PENDING_REQUESTS {
            held.push(dispatcher.register(dispatcher.next_id()).await.unwrap());
        }

        let error = dispatcher
            .register(dispatcher.next_id())
            .await
            .expect_err("the table must be bounded");
        assert!(matches!(error, McpProtocolError::Transport { .. }));
    }

    #[tokio::test]
    async fn request_ids_are_unique_within_a_connection() {
        let dispatcher = JsonRpcDispatcher::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            assert!(seen.insert(dispatcher.next_id()), "ids must not repeat");
        }
    }
}
