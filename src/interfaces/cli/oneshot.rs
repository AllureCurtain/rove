use tokio_util::sync::CancellationToken;

use crate::core::engine::Engine;
use crate::core::types::{TaskState, TerminationReason};
use crate::interfaces::cli::render::{CliRunRenderContext, CliRunRenderOptions, render_run_events};
use crate::state::store::RunHandle;
use crate::state::store::StateStore;

/// Run a one-shot command: send user message, stream output, exit.
///
/// Collects stats from the event stream and writes a report.json at the end.
pub async fn run_oneshot(
    engine: &Engine,
    message: String,
    run: RunHandle,
    resume_state: Option<TaskState>,
    state_store: &StateStore,
) -> TerminationReason {
    run_oneshot_with_cancel(
        engine,
        message,
        run,
        resume_state,
        state_store,
        CancellationToken::new(),
    )
    .await
}

pub async fn run_oneshot_with_cancel(
    engine: &Engine,
    message: String,
    run: RunHandle,
    resume_state: Option<TaskState>,
    state_store: &StateStore,
    cancel: CancellationToken,
) -> TerminationReason {
    let req = run.request(message.clone(), resume_state.clone());
    let trace_writer = run.trace_writer.clone();
    let stream = engine.run_with_cancel(req, Some(trace_writer), cancel);
    render_run_events(
        stream,
        CliRunRenderContext {
            message,
            run,
            resume_state,
            state_store,
            workspace: engine.workspace(),
            model_id: engine.model_id(),
        },
        CliRunRenderOptions::default(),
    )
    .await
}
