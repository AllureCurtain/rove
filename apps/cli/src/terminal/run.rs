use std::future::Future;

use futures::{Stream, StreamExt};

use crate::terminal::view::{RunViewState, RunViewUpdate};
use rove_runtime::events::StreamEvent;
use rove_runtime::runtime_identity::RuntimeIdentity;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::store::{RunHandle, StateStore};
use rove_runtime::types::{TaskState, TerminationReason};
use rove_runtime::workspace::Workspace;

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

pub async fn drive_run_events<S, F, Fut>(
    stream: S,
    context: RunEventContext<'_>,
    mut on_update: F,
) -> RunEventOutcome
where
    S: Stream<Item = StreamEvent>,
    F: FnMut(RunViewUpdate) -> Fut,
    Fut: Future<Output = ()>,
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
    let mut completion_update = None;

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        let update = view_state.apply_event(&event);
        if let RunViewUpdate::RunCompleted {
            reason: completion, ..
        } = &update
        {
            reason = completion.clone();
            completion_update = Some(update);
        } else {
            on_update(update).await;
        }
    }

    recorder
        .finalize(state_store, workspace, model_id, &run_dir)
        .await;
    if let Some(update) = completion_update {
        on_update(update).await;
    }

    RunEventOutcome { reason, view_state }
}

#[cfg(test)]
mod tests {
    use std::future::ready;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_stream::stream;
    use futures::stream;

    use crate::terminal::view::RunViewUpdate;
    use rove_runtime::events::StreamEvent;
    use rove_runtime::state::store::StateStore;
    use rove_runtime::types::{JobId, RunId, SessionId, TerminationReason};
    use rove_runtime::workspace::Workspace;

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
            |_| {
                updates += 1;
                ready(())
            },
        )
        .await;

        assert_eq!(updates, 3);
        assert_eq!(outcome.reason, TerminationReason::Final);
        assert_eq!(outcome.view_state.assistant_text, "ready");
        assert!(run_dir.join("report.json").exists());
    }

    #[tokio::test]
    async fn driver_drains_the_engine_after_completion_before_publishing_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let report_path = run.run_dir.join("report.json");
        let drained = Arc::new(AtomicBool::new(false));
        let drained_by_stream = Arc::clone(&drained);
        let events = stream! {
            yield StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            };
            yield StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ready".to_string()),
            };
            drained_by_stream.store(true, Ordering::SeqCst);
        };
        let mut completion_observed = false;

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
            |update| {
                if matches!(update, RunViewUpdate::RunCompleted { .. }) {
                    assert!(drained.load(Ordering::SeqCst));
                    assert!(report_path.exists());
                    completion_observed = true;
                }
                ready(())
            },
        )
        .await;

        assert_eq!(outcome.reason, TerminationReason::Final);
        assert!(completion_observed);
    }
}
