use std::io::Write;
use std::path::PathBuf;

use futures::StreamExt;

use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{RunId, RunRequest, TaskState, TerminationReason};
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::StateStore;
use crate::state::trace::TraceWriter;

/// Run a one-shot command: send user message, stream output, exit.
///
/// Collects stats from the event stream and writes a report.json at the end.
pub async fn run_oneshot(
    engine: &Engine,
    message: String,
    trace_writer: Option<TraceWriter>,
    run_id: RunId,
    run_dir: PathBuf,
    resume_state: Option<TaskState>,
    state_store: &StateStore,
) {
    let session_id = resume_state
        .as_ref()
        .map(|state| state.session_id)
        .unwrap_or_default();
    let job_id = resume_state
        .as_ref()
        .map(|state| state.job_id)
        .unwrap_or_default();
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        message.clone(),
        resume_state.as_ref(),
    );

    let req = RunRequest {
        session_id,
        job_id,
        run_id,
        user_message: message.clone(),
        resume_state,
    };
    let mut stream = std::pin::pin!(engine.run(req, trace_writer));

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
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
