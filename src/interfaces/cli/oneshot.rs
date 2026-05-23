use std::io::Write;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{RunId, TaskState, TerminationReason};
use crate::state::artifacts::RunArtifactRecorder;
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
    let resume_state_for_recorder = resume_state.clone();
    let req = run.request(message.clone(), resume_state);
    let RunHandle {
        session_id,
        job_id,
        run_id,
        run_dir,
        trace_writer,
    } = run;

    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        message.clone(),
        resume_state_for_recorder.as_ref(),
    );
    let mut stream = std::pin::pin!(engine.run_with_cancel(req, Some(trace_writer), cancel));
    let mut terminal_reason = TerminationReason::Error;

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        match event {
            StreamEvent::LlmChunk { delta } => {
                print!("{}", delta);
                let _ = std::io::stdout().flush();
            }
            StreamEvent::ToolCallStarted { name, args, .. } => {
                eprintln!("\n  [tool] {}({})", name, args);
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                eprintln!("  [result] {}", truncate(&result.output, 200));
            }
            StreamEvent::ToolCallFailed { error, .. } => {
                eprintln!("  [error] {}", error);
            }
            StreamEvent::PlanCreated { plan: new_plan } => {
                eprintln!("\n  [plan] {} steps", new_plan.steps.len());
            }
            StreamEvent::PlanStepStarted { step, index } => {
                eprintln!("  [step {}] {}", index + 1, step.title);
            }
            StreamEvent::RunCompleted { reason, output } => {
                terminal_reason = reason.clone();
                if let Some(ref text) = output
                    && !matches!(reason, TerminationReason::Final)
                {
                    println!("\n{}", text);
                }
                eprintln!("\n  [done] {:?}", reason);
                break;
            }
            _ => {}
        }
    }
    println!();

    recorder
        .finalize(state_store, engine.workspace(), engine.model_id(), &run_dir)
        .await;
    terminal_reason
}

pub async fn resolve_resume_state(
    state_store: &StateStore,
    resume: Option<&str>,
) -> anyhow::Result<Option<TaskState>> {
    let Some(value) = resume else {
        return Ok(None);
    };

    if value == "latest" {
        return Ok(state_store.load_latest_task_state().await?);
    }

    let run_id = ulid::Ulid::from_string(value).map_err(|_| {
        anyhow::anyhow!("unsupported --resume value: {value}; expected latest or run_id")
    })?;
    Ok(Some(state_store.load_task_state(RunId(run_id)).await?))
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use super::resolve_resume_state;
    use crate::core::types::{JobId, RunId, SessionId, TaskState};
    use crate::state::store::StateStore;

    fn task_state(run_id: RunId, goal: &str) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id,
            goal: goal.to_string(),
            step: 1,
            history: vec![],
            summary: None,
            plan: None,
        }
    }

    #[tokio::test]
    async fn resolve_resume_state_supports_latest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let older = task_state(RunId::new(), "older");
        let newer = task_state(RunId::new(), "newer");
        store.write_task_state(&older).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        store.write_task_state(&newer).await.unwrap();

        let state = resolve_resume_state(&store, Some("latest"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(state.goal, "newer");
    }

    #[tokio::test]
    async fn resolve_resume_state_supports_run_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let target_run_id = RunId::new();
        let target = task_state(target_run_id, "target");
        let other = task_state(RunId::new(), "other");
        store.write_task_state(&target).await.unwrap();
        store.write_task_state(&other).await.unwrap();

        let state = resolve_resume_state(&store, Some(&target_run_id.to_string()))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(state.run_id, target_run_id);
        assert_eq!(state.goal, "target");
    }

    #[tokio::test]
    async fn resolve_resume_state_rejects_invalid_value() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());

        let err = resolve_resume_state(&store, Some("not-a-run-id"))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("expected latest or run_id"));
    }
}
