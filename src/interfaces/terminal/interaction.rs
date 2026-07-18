use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::core::types::{
    ApprovalDecision, PendingToolApproval, PendingUserInput, ToolApprovalProvider,
    ToolApprovalRequest, UserInputProvider, UserInputRequest,
};
use crate::errors::ToolError;

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
        input_id: crate::core::types::CallId,
        request: UserInputRequest,
        respond_to: oneshot::Sender<String>,
    },
}

pub type TerminalInteractionReceiver = mpsc::Receiver<TerminalInteractionRequest>;

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

    (
        TerminalInteractionProviders {
            approval_provider: Arc::new(ChannelApprovalProvider {
                sender: sender.clone(),
            }),
            input_provider: Arc::new(ChannelInputProvider { sender }),
        },
        receiver,
    )
}

struct ChannelApprovalProvider {
    sender: mpsc::Sender<TerminalInteractionRequest>,
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

        if self.sender.try_send(message).is_err() {
            return Err(approval_error("request channel is full or closed"));
        }

        Ok(PendingToolApproval::new(async move {
            response.await.unwrap_or(ApprovalDecision::Reject)
        }))
    }
}

struct ChannelInputProvider {
    sender: mpsc::Sender<TerminalInteractionRequest>,
}

#[async_trait]
impl UserInputProvider for ChannelInputProvider {
    async fn begin_input(
        &self,
        input_id: crate::core::types::CallId,
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
            Err(mpsc::error::TrySendError::Full(_)) => Err(input_error("request channel is full")),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(input_error("request channel is closed"))
            }
        }
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

    use tokio::sync::{mpsc, oneshot};

    use crate::core::types::{
        ApprovalDecision, CallId, ToolApprovalProvider, ToolApprovalRequest, UserInputProvider,
        UserInputRequest,
    };
    use crate::errors::ToolError;

    use super::{
        ChannelApprovalProvider, ChannelInputProvider, TerminalInteractionRequest,
        bounded_interaction_channel,
    };

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
        let (sender, receiver) = mpsc::channel(1);
        let provider = ChannelApprovalProvider {
            sender: sender.clone(),
        };
        let (respond_to, _response) = oneshot::channel();
        sender
            .try_send(TerminalInteractionRequest::Approval {
                request: approval_request(),
                respond_to,
            })
            .unwrap();

        assert_eq!(
            provider.decide(approval_request()).await,
            ApprovalDecision::Reject
        );

        drop(receiver);
        assert_eq!(
            provider.decide(approval_request()).await,
            ApprovalDecision::Reject
        );
    }

    #[tokio::test]
    async fn full_and_closed_input_channels_return_typed_errors() {
        let (sender, receiver) = mpsc::channel(1);
        let provider = ChannelInputProvider {
            sender: sender.clone(),
        };
        let (respond_to, _response) = oneshot::channel();
        sender
            .try_send(TerminalInteractionRequest::Input {
                input_id: CallId::new(),
                request: input_request(),
                respond_to,
            })
            .unwrap();

        let full = provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap_err();
        assert!(matches!(
            full,
            ToolError::ExecutionFailed { ref reason } if reason.contains("full")
        ));

        drop(receiver);
        let closed = provider
            .begin_input(CallId::new(), input_request())
            .await
            .unwrap_err();
        assert!(matches!(
            closed,
            ToolError::ExecutionFailed { ref reason } if reason.contains("closed")
        ));
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
}
