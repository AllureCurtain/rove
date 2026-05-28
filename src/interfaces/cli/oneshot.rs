use std::io::Write;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{TaskState, TerminationReason};
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

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
