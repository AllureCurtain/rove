use std::future::Future;

use futures::Stream;

use crate::terminal::run::drive_run_events;
use crate::terminal::view::RunViewUpdate;
use rove_runtime::events::StreamEvent;

pub use crate::terminal::run::{
    RunEventContext as TuiRunContext, RunEventOutcome as TuiRunOutcome,
};

/// Projects canonical runtime events for the TUI while retaining the shared
/// artifact recording and report finalization path.
pub async fn drive_tui_run_events<S, F, Fut>(
    stream: S,
    context: TuiRunContext<'_>,
    on_update: F,
) -> TuiRunOutcome
where
    S: Stream<Item = StreamEvent>,
    F: FnMut(RunViewUpdate) -> Fut,
    Fut: Future<Output = ()>,
{
    drive_run_events(stream, context, on_update).await
}

#[cfg(test)]
mod tests {
    use futures::stream;

    use crate::terminal::view::RunViewUpdate;
    use rove_runtime::events::StreamEvent;
    use rove_runtime::state::store::StateStore;
    use rove_runtime::types::{JobId, RunId, SessionId, TerminationReason};
    use rove_runtime::workspace::Workspace;

    use super::{TuiRunContext, drive_tui_run_events};

    #[tokio::test]
    async fn adapter_uses_shared_projection_and_artifact_finalization() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let report_path = run.run_dir.join("report.json");
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "ready".to_string(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ready".to_string()),
            },
        ]);
        let mut completions = 0;

        let outcome = drive_tui_run_events(
            events,
            TuiRunContext {
                message: "hello".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            |update| {
                if matches!(update, RunViewUpdate::RunCompleted { .. }) {
                    completions += 1;
                }
                std::future::ready(())
            },
        )
        .await;

        assert_eq!(completions, 1);
        assert_eq!(outcome.reason, TerminationReason::Final);
        assert_eq!(outcome.view_state.assistant_text, "ready");
        assert!(report_path.exists());
    }
}
