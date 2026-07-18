use std::future::Future;

use tokio::sync::{mpsc, oneshot};

use crate::core::types::{CallId, UserInputProvider, UserInputRequest};
use crate::errors::ToolError;

/// Registered input passed from a tool execution to the Core event stream.
///
/// This type stays crate-private so tools cannot publish canonical lifecycle
/// events themselves.
#[derive(Debug)]
pub(crate) struct RegisteredUserInput {
    pub(crate) input_id: CallId,
    pub(crate) request: UserInputRequest,
    pub(crate) acknowledged: oneshot::Sender<()>,
}

#[derive(Clone)]
struct InputExecutionContext {
    call_id: CallId,
    events: Option<mpsc::Sender<RegisteredUserInput>>,
}

tokio::task_local! {
    static INPUT_EXECUTION_CONTEXT: InputExecutionContext;
}

pub(crate) async fn scope<F>(
    call_id: CallId,
    events: Option<mpsc::Sender<RegisteredUserInput>>,
    future: F,
) -> F::Output
where
    F: Future,
{
    INPUT_EXECUTION_CONTEXT
        .scope(InputExecutionContext { call_id, events }, future)
        .await
}

pub(crate) async fn request_input<P>(provider: &P, prompt: String) -> Result<String, ToolError>
where
    P: UserInputProvider + ?Sized,
{
    let execution = INPUT_EXECUTION_CONTEXT.try_with(Clone::clone).ok();
    let input_id = execution
        .as_ref()
        .map(|execution| execution.call_id)
        .unwrap_or_default();
    let request = UserInputRequest { prompt };
    let pending = provider.begin_input(input_id, request.clone()).await?;

    if let Some(events) = execution.and_then(|execution| execution.events) {
        let (acknowledged, acknowledgement) = oneshot::channel();
        events
            .send(RegisteredUserInput {
                input_id,
                request,
                acknowledged,
            })
            .await
            .map_err(|_| ToolError::ExecutionFailed {
                reason: "input event channel is closed".to_string(),
            })?;
        acknowledgement
            .await
            .map_err(|_| ToolError::ExecutionFailed {
                reason: "input event was not acknowledged".to_string(),
            })?;
    }

    pending.resolve().await
}
