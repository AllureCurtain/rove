use tokio_util::sync::CancellationToken;

use crate::cli::oneshot::run_oneshot_with_cancel;
use rove_engine::engine::Engine;
use rove_foundation::types::TerminationReason;
use rove_state::state::store::{RunHandle, StateStore};
use rove_state::task_state::TaskState;

/// Run a non-interactive exec prompt: stream output, write artifacts, and exit.
pub async fn run_exec_with_cancel(
    engine: &Engine,
    message: String,
    run: RunHandle,
    resume_state: Option<TaskState>,
    state_store: &StateStore,
    cancel: CancellationToken,
) -> TerminationReason {
    run_oneshot_with_cancel(engine, message, run, resume_state, state_store, cancel).await
}
