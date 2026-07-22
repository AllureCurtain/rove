use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use rove_core::ToolError;
use rove_runtime::types::{
    ApprovalDecision, CallId, PendingToolApproval, PendingUserInput, ToolApprovalProvider,
    ToolApprovalRequest, UserInputProvider, UserInputRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInteractionKind {
    Approval,
    Input,
}

/// An interaction request waiting for the terminal application to answer it.
///
/// These requests are process-local coordination messages. They do not define
/// or replace canonical runtime events.
#[derive(Debug)]
pub enum TerminalInteractionRequest {
    Approval {
        request: ToolApprovalRequest,
        respond_to: oneshot::Sender<ApprovalDecision>,
    },
    Input {
        input_id: CallId,
        request: UserInputRequest,
        respond_to: oneshot::Sender<String>,
    },
}

impl TerminalInteractionRequest {
    pub fn kind(&self) -> TerminalInteractionKind {
        match self {
            Self::Approval { .. } => TerminalInteractionKind::Approval,
            Self::Input { .. } => TerminalInteractionKind::Input,
        }
    }

    pub fn request_id(&self) -> CallId {
        match self {
            Self::Approval { request, .. } => request.call_id,
            Self::Input { input_id, .. } => *input_id,
        }
    }

    pub fn responder_is_closed(&self) -> bool {
        match self {
            Self::Approval { respond_to, .. } => respond_to.is_closed(),
            Self::Input { respond_to, .. } => respond_to.is_closed(),
        }
    }
}

struct QueuedInteractionRequest {
    request: TerminalInteractionRequest,
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
struct InteractionSender {
    sender: mpsc::Sender<QueuedInteractionRequest>,
    permits: Arc<Semaphore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionSendError {
    Full,
    Closed,
}

impl InteractionSender {
    fn try_send(&self, request: TerminalInteractionRequest) -> Result<(), InteractionSendError> {
        if self.sender.is_closed() {
            return Err(InteractionSendError::Closed);
        }
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| InteractionSendError::Full)?;
        let queued = QueuedInteractionRequest {
            request,
            _permit: permit,
        };
        self.sender.try_send(queued).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => InteractionSendError::Full,
            mpsc::error::TrySendError::Closed(_) => InteractionSendError::Closed,
        })
    }
}

/// Bounded terminal request receiver that filters cancelled responders.
///
/// Live requests encountered while scanning are buffered with their capacity
/// permit, so `drain_stale` cannot expand the transport beyond its configured
/// bound.
pub struct TerminalInteractionReceiver {
    receiver: mpsc::Receiver<QueuedInteractionRequest>,
    buffered: std::collections::VecDeque<QueuedInteractionRequest>,
    permits: Arc<Semaphore>,
    max_capacity: usize,
}

impl TerminalInteractionReceiver {
    /// Waits for the next request whose responder is still live.
    pub async fn recv(&mut self) -> Option<TerminalInteractionRequest> {
        loop {
            if let Some(queued) = self.buffered.pop_front() {
                if !queued.request.responder_is_closed() {
                    return Some(queued.request);
                }
                continue;
            }

            let queued = self.receiver.recv().await?;
            if !queued.request.responder_is_closed() {
                return Some(queued.request);
            }
        }
    }

    /// Returns the next currently queued live request without waiting.
    pub fn try_recv(&mut self) -> Result<TerminalInteractionRequest, mpsc::error::TryRecvError> {
        loop {
            let queued = if let Some(queued) = self.buffered.pop_front() {
                queued
            } else {
                self.receiver.try_recv()?
            };
            if !queued.request.responder_is_closed() {
                return Ok(queued.request);
            }
        }
    }

    /// Removes all currently queued requests with closed responders.
    ///
    /// Live requests retain FIFO order and remain available through `recv` or
    /// `try_recv`. The return value is the number of stale requests removed.
    pub fn drain_stale(&mut self) -> usize {
        let mut stale = 0;
        self.buffered.retain(|queued| {
            let live = !queued.request.responder_is_closed();
            stale += usize::from(!live);
            live
        });

        loop {
            match self.receiver.try_recv() {
                Ok(queued) if queued.request.responder_is_closed() => stale += 1,
                Ok(queued) => self.buffered.push_back(queued),
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
        stale
    }

    pub fn capacity(&self) -> usize {
        self.permits.available_permits()
    }

    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn len(&self) -> usize {
        self.buffered.len() + self.receiver.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffered.is_empty() && self.receiver.is_empty()
    }

    pub fn close(&mut self) {
        self.receiver.close();
    }
}

#[derive(Clone)]
pub struct TerminalInteractionProviders {
    pub approval_provider: Arc<dyn ToolApprovalProvider>,
    pub input_provider: Arc<dyn UserInputProvider>,
}

/// Creates fail-closed provider adapters backed by one bounded request queue.
pub fn bounded_interaction_channel(
    capacity: usize,
) -> (TerminalInteractionProviders, TerminalInteractionReceiver) {
    assert!(
        capacity > 0,
        "interaction channel capacity must be non-zero"
    );
    let (sender, receiver) = mpsc::channel(capacity);
    let permits = Arc::new(Semaphore::new(capacity));
    let sender = InteractionSender {
        sender,
        permits: Arc::clone(&permits),
    };

    (
        TerminalInteractionProviders {
            approval_provider: Arc::new(ChannelApprovalProvider {
                sender: sender.clone(),
            }),
            input_provider: Arc::new(ChannelInputProvider { sender }),
        },
        TerminalInteractionReceiver {
            receiver,
            buffered: std::collections::VecDeque::new(),
            permits,
            max_capacity: capacity,
        },
    )
}

struct ChannelApprovalProvider {
    sender: InteractionSender,
}

#[async_trait]
impl ToolApprovalProvider for ChannelApprovalProvider {
    async fn begin_approval(
        &self,
        request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        let (respond_to, response) = oneshot::channel();
        let message = TerminalInteractionRequest::Approval {
            request,
            respond_to,
        };

        if let Err(error) = self.sender.try_send(message) {
            return Err(approval_transport_error(error));
        }

        Ok(PendingToolApproval::new(async move {
            response.await.unwrap_or(ApprovalDecision::Reject)
        }))
    }
}

struct ChannelInputProvider {
    sender: InteractionSender,
}

#[async_trait]
impl UserInputProvider for ChannelInputProvider {
    async fn begin_input(
        &self,
        input_id: rove_runtime::types::CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        let (respond_to, response) = oneshot::channel();
        let message = TerminalInteractionRequest::Input {
            input_id,
            request,
            respond_to,
        };

        match self.sender.try_send(message) {
            Ok(()) => Ok(PendingUserInput::new(async move {
                response
                    .await
                    .map_err(|_| input_error("response was dropped"))
            })),
            Err(InteractionSendError::Full) => Err(input_error("request channel is full")),
            Err(InteractionSendError::Closed) => Err(input_error("request channel is closed")),
        }
    }
}

fn approval_transport_error(error: InteractionSendError) -> ToolError {
    match error {
        InteractionSendError::Full => approval_error("request channel is full"),
        InteractionSendError::Closed => approval_error("request channel is closed"),
    }
}

fn input_error(reason: &str) -> ToolError {
    ToolError::ExecutionFailed {
        reason: format!("terminal input unavailable: {reason}"),
    }
}

fn approval_error(reason: &str) -> ToolError {
    ToolError::ExecutionFailed {
        reason: format!("terminal approval unavailable: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rove_core::ToolError;
    use rove_runtime::types::{ApprovalDecision, CallId, ToolApprovalRequest, UserInputRequest};

    use super::{TerminalInteractionKind, TerminalInteractionRequest, bounded_interaction_channel};

    fn approval_request() -> ToolApprovalRequest {
        ToolApprovalRequest {
            call_id: CallId::new(),
            name: "fs_write".to_string(),
            args: serde_json::json!({"path": "result.txt"}),
            reason: "writes a file".to_string(),
        }
    }

    fn input_request() -> UserInputRequest {
        UserInputRequest {
            prompt: "Which branch?".to_string(),
        }
    }

    #[tokio::test]
    async fn approval_provider_returns_terminal_response() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let provider = Arc::clone(&providers.approval_provider);
        let decision = tokio::spawn(async move { provider.decide(approval_request()).await });

        let request = receiver.recv().await.unwrap();
        assert_eq!(request.kind(), TerminalInteractionKind::Approval);
        assert!(!request.responder_is_closed());
        match request {
            TerminalInteractionRequest::Approval {
                request,
                respond_to,
            } => {
                assert_eq!(request.name, "fs_write");
                respond_to.send(ApprovalDecision::Approve).unwrap();
            }
            TerminalInteractionRequest::Input { .. } => panic!("expected approval request"),
        }

        assert_eq!(decision.await.unwrap(), ApprovalDecision::Approve);
    }

    #[tokio::test]
    async fn input_provider_preserves_caller_id_and_returns_terminal_response() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let provider = Arc::clone(&providers.input_provider);
        let expected_first_id = CallId::new();
        let answer = tokio::spawn(async move {
            provider
                .begin_input(
                    expected_first_id,
                    UserInputRequest {
                        prompt: "Which branch?".to_string(),
                    },
                )
                .await?
                .resolve()
                .await
        });

        let first_id = match receiver.recv().await.unwrap() {
            TerminalInteractionRequest::Input {
                input_id,
                request,
                respond_to,
            } => {
                assert_eq!(request.prompt, "Which branch?");
                respond_to.send("feat/tui".to_string()).unwrap();
                input_id
            }
            TerminalInteractionRequest::Approval { .. } => panic!("expected input request"),
        };
        assert_eq!(answer.await.unwrap().unwrap(), "feat/tui");
        assert_eq!(first_id, expected_first_id);

        let provider = Arc::clone(&providers.input_provider);
        let expected_second_id = CallId::new();
        let answer = tokio::spawn(async move {
            provider
                .begin_input(
                    expected_second_id,
                    UserInputRequest {
                        prompt: "Which branch?".to_string(),
                    },
                )
                .await?
                .resolve()
                .await
        });
        let second_id = match receiver.recv().await.unwrap() {
            TerminalInteractionRequest::Input {
                input_id,
                request,
                respond_to,
            } => {
                respond_to.send("main".to_string()).unwrap();
                assert_eq!(request.prompt, "Which branch?");
                input_id
            }
            TerminalInteractionRequest::Approval { .. } => panic!("expected input request"),
        };
        assert_eq!(answer.await.unwrap().unwrap(), "main");
        assert_eq!(second_id, expected_second_id);
        assert_ne!(first_id, second_id);
    }

    #[tokio::test]
    async fn full_and_closed_approval_channels_reject() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let pending = providers
            .approval_provider
            .begin_approval(approval_request())
            .await
            .unwrap();
        assert_eq!(receiver.capacity(), 0);

        let full = providers
            .approval_provider
            .begin_approval(approval_request())
            .await
            .unwrap_err();
        assert!(matches!(
            full,
            ToolError::ExecutionFailed { ref reason } if reason.contains("full")
        ));
        assert_eq!(
            providers.approval_provider.decide(approval_request()).await,
            ApprovalDecision::Reject
        );

        receiver.close();
        let closed = providers
            .approval_provider
            .begin_approval(approval_request())
            .await
            .unwrap_err();
        assert!(matches!(
            closed,
            ToolError::ExecutionFailed { ref reason } if reason.contains("closed")
        ));
        assert_eq!(
            providers.approval_provider.decide(approval_request()).await,
            ApprovalDecision::Reject
        );
        drop(pending);
    }

    #[tokio::test]
    async fn full_and_closed_input_channels_return_typed_errors() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let pending = providers
            .input_provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap();
        assert_eq!(receiver.capacity(), 0);

        let full = providers
            .input_provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap_err();
        assert!(matches!(
            full,
            ToolError::ExecutionFailed { ref reason } if reason.contains("full")
        ));

        receiver.close();
        let closed = providers
            .input_provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap_err();
        assert!(matches!(
            closed,
            ToolError::ExecutionFailed { ref reason } if reason.contains("closed")
        ));
        drop(pending);
    }

    #[tokio::test]
    async fn dropped_approval_responder_rejects() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let provider = Arc::clone(&providers.approval_provider);
        let decision = tokio::spawn(async move { provider.decide(approval_request()).await });

        drop(receiver.recv().await.unwrap());

        assert_eq!(decision.await.unwrap(), ApprovalDecision::Reject);
    }

    #[tokio::test]
    async fn dropped_input_responder_returns_typed_error() {
        let (providers, mut receiver) = bounded_interaction_channel(1);
        let provider = Arc::clone(&providers.input_provider);
        let answer = tokio::spawn(async move {
            provider
                .begin_input(CallId::new(), input_request())
                .await?
                .resolve()
                .await
        });

        drop(receiver.recv().await.unwrap());

        let error = answer.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            ToolError::ExecutionFailed { ref reason } if reason.contains("response was dropped")
        ));
    }

    #[tokio::test]
    async fn stale_approval_is_skipped_before_the_next_live_request() {
        let (providers, mut receiver) = bounded_interaction_channel(2);
        let stale_id = CallId::new();
        let stale = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: stale_id,
                ..approval_request()
            })
            .await
            .unwrap();
        drop(stale);

        let live_id = CallId::new();
        let live = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: live_id,
                ..approval_request()
            })
            .await
            .unwrap();

        let request = receiver.recv().await.unwrap();
        assert_eq!(request.request_id(), live_id);
        assert_eq!(request.kind(), TerminalInteractionKind::Approval);
        match request {
            TerminalInteractionRequest::Approval { respond_to, .. } => {
                respond_to.send(ApprovalDecision::Approve).unwrap();
            }
            TerminalInteractionRequest::Input { .. } => panic!("expected approval request"),
        }
        assert_eq!(live.resolve().await, ApprovalDecision::Approve);
        assert_eq!(receiver.capacity(), receiver.max_capacity());
    }

    #[tokio::test]
    async fn stale_input_is_skipped_before_the_next_live_request() {
        let (providers, mut receiver) = bounded_interaction_channel(2);
        let stale = providers
            .input_provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap();
        drop(stale);

        let live_id = CallId::new();
        let live = providers
            .input_provider
            .begin_input(live_id, input_request())
            .await
            .unwrap();

        let request = receiver.recv().await.unwrap();
        assert_eq!(request.request_id(), live_id);
        assert_eq!(request.kind(), TerminalInteractionKind::Input);
        match request {
            TerminalInteractionRequest::Input { respond_to, .. } => {
                respond_to.send("main".to_string()).unwrap();
            }
            TerminalInteractionRequest::Approval { .. } => panic!("expected input request"),
        }
        assert_eq!(live.resolve().await.unwrap(), "main");
        assert_eq!(receiver.capacity(), receiver.max_capacity());
    }

    #[tokio::test]
    async fn draining_capacity_eight_discards_stale_and_preserves_live_fifo() {
        let (providers, mut receiver) = bounded_interaction_channel(8);
        for index in 0..8 {
            if index % 2 == 0 {
                let pending = providers
                    .approval_provider
                    .begin_approval(approval_request())
                    .await
                    .unwrap();
                drop(pending);
            } else {
                let pending = providers
                    .input_provider
                    .begin_input(CallId::new(), input_request())
                    .await
                    .unwrap();
                drop(pending);
            }
        }
        assert_eq!(receiver.capacity(), 0);
        assert_eq!(receiver.len(), 8);
        assert_eq!(receiver.drain_stale(), 8);
        assert!(receiver.is_empty());
        assert_eq!(receiver.capacity(), 8);

        let approval_id = CallId::new();
        let approval = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: approval_id,
                ..approval_request()
            })
            .await
            .unwrap();
        let input_id = CallId::new();
        let input = providers
            .input_provider
            .begin_input(input_id, input_request())
            .await
            .unwrap();

        assert_eq!(receiver.drain_stale(), 0);
        assert_eq!(receiver.capacity(), 6);
        let first = receiver.try_recv().unwrap();
        let second = receiver.try_recv().unwrap();
        assert_eq!(first.request_id(), approval_id);
        assert_eq!(second.request_id(), input_id);
        match first {
            TerminalInteractionRequest::Approval { respond_to, .. } => {
                respond_to.send(ApprovalDecision::Reject).unwrap();
            }
            TerminalInteractionRequest::Input { .. } => panic!("expected approval request"),
        }
        match second {
            TerminalInteractionRequest::Input { respond_to, .. } => {
                respond_to.send("main".to_string()).unwrap();
            }
            TerminalInteractionRequest::Approval { .. } => panic!("expected input request"),
        }
        assert_eq!(approval.resolve().await, ApprovalDecision::Reject);
        assert_eq!(input.resolve().await.unwrap(), "main");
        assert_eq!(receiver.capacity(), 8);
    }

    #[tokio::test]
    async fn dropping_receiver_rejects_approval_and_errors_input() {
        let (providers, receiver) = bounded_interaction_channel(2);
        let approval = providers
            .approval_provider
            .begin_approval(approval_request())
            .await
            .unwrap();
        let input = providers
            .input_provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap();

        drop(receiver);

        assert_eq!(approval.resolve().await, ApprovalDecision::Reject);
        let error = input.resolve().await.unwrap_err();
        assert!(matches!(
            error,
            ToolError::ExecutionFailed { ref reason } if reason.contains("response was dropped")
        ));
        assert_eq!(
            providers.approval_provider.decide(approval_request()).await,
            ApprovalDecision::Reject
        );
    }
}
