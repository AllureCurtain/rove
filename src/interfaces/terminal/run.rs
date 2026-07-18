use futures::{Stream, StreamExt};

use crate::core::events::StreamEvent;
use crate::core::runtime_identity::RuntimeIdentity;
use crate::core::types::{TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::interfaces::terminal::view::{RunViewState, RunViewUpdate};
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::{RunHandle, StateStore};

pub struct RunEventContext<'a> {
    pub message: String,
    pub run: RunHandle,
    pub resume_state: Option<TaskState>,
    pub state_store: &'a StateStore,
    pub workspace: &'a Workspace,
    pub model_id: &'a str,
    pub runtime_identity: Option<RuntimeIdentity>,
}

#[derive(Debug)]
pub struct RunEventOutcome {
    pub reason: TerminationReason,
    pub view_state: RunViewState,
}

pub async fn drive_run_events<S, F>(
    stream: S,
    context: RunEventContext<'_>,
    mut on_update: F,
) -> RunEventOutcome
where
    S: Stream<Item = StreamEvent>,
    F: FnMut(RunViewUpdate, &RunViewState),
{
    futures::pin_mut!(stream);
    let RunEventContext {
        message,
        run,
        resume_state,
        state_store,
        workspace,
        model_id,
        runtime_identity,
    } = context;
    let RunHandle {
        session_id,
        job_id,
        run_id,
        run_dir,
        trace_writer: _,
    } = run;
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        message,
        resume_state.as_ref(),
        runtime_identity,
    );
    let mut reason = TerminationReason::Error;
    let mut view_state = RunViewState::default();

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        let update = view_state.apply_event(&event);
        let completion = match &update {
            RunViewUpdate::RunCompleted { reason, .. } => Some(reason.clone()),
            _ => None,
        };
        on_update(update, &view_state);
        if let Some(completion) = completion {
            reason = completion;
            break;
        }
    }

    recorder
        .finalize(state_store, workspace, model_id, &run_dir)
        .await;

    RunEventOutcome { reason, view_state }
}

#[cfg(test)]
mod tests {
    use futures::stream;

    use crate::core::events::StreamEvent;
    use crate::core::types::{JobId, RunId, SessionId, TerminationReason};
    use crate::core::workspace::Workspace;
    use crate::state::store::StateStore;

    use super::{RunEventContext, drive_run_events};

    #[tokio::test]
    async fn driver_projects_events_and_finalizes_shared_artifacts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let run_dir = run.run_dir.clone();
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
        let mut updates = 0;

        let outcome = drive_run_events(
            events,
            RunEventContext {
                message: "hello".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            |_, _| updates += 1,
        )
        .await;

        assert_eq!(updates, 3);
        assert_eq!(outcome.reason, TerminationReason::Final);
        assert_eq!(outcome.view_state.assistant_text, "ready");
        assert!(run_dir.join("report.json").exists());
    }
}
