//! Runtime control plane: in-flight steer message injection.
//!
//! Steer messages are injected at declared safe points so they never land in
//! the middle of a tool side-effect and never bypass approval or
//! cancellation. A cloneable [`RunControlHandle`] is the only public handle
//! external callers (API, CLI/TUI) use; the receiver side is owned by the
//! engine run-loop and drained at step/iteration boundaries.
//!
//! Follow-up messages (queued-after-completion auto-runs) are an API/
//! ProductStore concern — they do not require runtime plumbing. The event
//! variants for follow-up are defined in `StreamEvent` and emitted by the
//! API supervisor, not by the engine.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

/// Unique identifier for a steer injection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SteerId(pub String);

impl SteerId {
    pub fn new() -> Self {
        Self(crate::foundation::types::SessionId::new().to_string())
    }
}

impl Default for SteerId {
    fn default() -> Self {
        Self::new()
    }
}

/// A steering message queued for the *currently running* turn.
///
/// Steer content is drained at the next declared safe point (top of a step
/// iteration, before the next model turn is built). It is never injected in
/// the middle of a tool side-effect, during approval wait, or after the run
/// has reached a terminal state.
#[derive(Debug, Clone)]
pub struct SteerMessage {
    pub id: SteerId,
    pub content: String,
}

impl SteerMessage {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            id: SteerId::new(),
            content: content.into(),
        }
    }

    /// Build from an externally-supplied id (used when the API already
    /// persisted a control record).
    pub fn with_id(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: SteerId(id.into()),
            content: content.into(),
        }
    }
}

/// Bounded queue size. Caps in-memory buffering — durability lives in the
/// ProductStore control table; if the buffer is full the API returns a typed
/// "busy" response rather than blocking the HTTP handler.
const STEER_BUFFER: usize = 64;

/// Cloneable handle given to the API / CLI for submitting in-flight steers.
///
/// Dropping the last clone closes the channel; the engine observes closure
/// and treats it as "no more external controls will arrive", which is the
/// normal case for runs started without a control plane attached.
#[derive(Clone, Default)]
pub struct RunControlHandle {
    pub steer: Option<mpsc::Sender<SteerMessage>>,
}

/// Tracks steers that have crossed a safe point but have not yet been handed
/// to the next model turn. The runtime uses this to emit a terminal
/// `steer_dropped` fact when a budget, cancellation, or failure prevents the
/// prepared next turn from starting.
#[derive(Clone, Default)]
pub(crate) struct SteerLifecycle {
    accepted_ids: Arc<Mutex<Vec<String>>>,
}

impl SteerLifecycle {
    pub(crate) async fn accepted(&self, id: String) {
        self.accepted_ids.lock().await.push(id);
    }

    pub(crate) async fn applied(&self, id: &str) {
        self.accepted_ids
            .lock()
            .await
            .retain(|pending| pending != id);
    }

    pub(crate) async fn take_unapplied(&self) -> Vec<String> {
        std::mem::take(&mut *self.accepted_ids.lock().await)
    }
}

impl RunControlHandle {
    /// Create a disconnected handle (no in-flight controls will arrive).
    pub fn disconnected() -> Self {
        Self::default()
    }

    /// Best-effort steer submission. Returns `false` if the buffer is full or
    /// the receiver has been dropped, letting the caller decide what to tell
    /// the user without blocking.
    pub fn try_send_steer(&self, msg: SteerMessage) -> bool {
        match &self.steer {
            Some(tx) => tx.try_send(msg).is_ok(),
            None => false,
        }
    }
}

/// Create a matched sender/receiver pair. The handle is given to the API; the
/// receiver is threaded into the run loops via LoopContext.
pub fn control_channel() -> (RunControlHandle, mpsc::Receiver<SteerMessage>) {
    let (steer_tx, steer_rx) = mpsc::channel(STEER_BUFFER);
    (
        RunControlHandle {
            steer: Some(steer_tx),
        },
        steer_rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnected_handle_rejects_sends() {
        let h = RunControlHandle::disconnected();
        assert!(!h.try_send_steer(SteerMessage::new("x")));
    }

    #[tokio::test]
    async fn drain_returns_all_pending_messages() {
        let (handle, mut receiver) = control_channel();
        handle.try_send_steer(SteerMessage::new("a"));
        handle.try_send_steer(SteerMessage::new("b"));
        let mut drained = Vec::new();
        while let Ok(msg) = receiver.try_recv() {
            drained.push(msg);
        }
        assert_eq!(drained.len(), 2);
    }
}
