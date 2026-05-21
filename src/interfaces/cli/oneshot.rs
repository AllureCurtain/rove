use std::io::Write;
use std::path::PathBuf;

use futures::StreamExt;

use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{RunId, RunRequest, TaskState, TerminationReason, Usage};
use crate::state::report::{RunReport, write_report};
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
    let initial_history = resume_state
        .as_ref()
        .map(|state| state.history.clone())
        .unwrap_or_default();
    let initial_step = resume_state.as_ref().map(|state| state.step).unwrap_or(0);
    let initial_summary = resume_state
        .as_ref()
        .and_then(|state| state.summary.clone());

    let req = RunRequest {
        session_id,
        job_id,
        run_id,
        user_message: message.clone(),
        resume_state,
    };
    let mut stream = std::pin::pin!(engine.run(req, trace_writer));

    let mut steps: u32 = 0;
    let mut tool_calls: u32 = 0;
    let mut tool_failures: u32 = 0;
    let mut total_usage = Usage::default();
    let mut final_reason = TerminationReason::Error;
    let mut final_output: Option<String> = None;
    let mut history = initial_history;

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::LlmChunk { delta } => {
                print!("{}", delta);
                let _ = std::io::stdout().flush();
            }
            StreamEvent::LlmMessage { usage, .. } => {
                steps += 1;
                total_usage.prompt_tokens += usage.prompt_tokens;
                total_usage.completion_tokens += usage.completion_tokens;
                total_usage.total_tokens += usage.total_tokens;
            }
            StreamEvent::ToolCallStarted { name, args, .. } => {
                tool_calls += 1;
                eprintln!("\n  [tool] {}({})", name, args);
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                eprintln!("  [result] {}", truncate(&result.output, 200));
            }
            StreamEvent::ToolCallFailed { error, .. } => {
                tool_failures += 1;
                eprintln!("  [error] {}", error);
            }
            StreamEvent::RunCompleted { reason, output } => {
                if let Some(ref text) = output
                    && !matches!(reason, TerminationReason::Final)
                {
                    println!("\n{}", text);
                }
                eprintln!("\n  [done] {:?}", reason);
                final_reason = reason;
                final_output = output;
                break;
            }
            _ => {}
        }
    }
    println!();

    history.push(crate::core::types::Message {
        role: crate::core::types::Role::User,
        content: message,
    });
    if let Some(output) = &final_output {
        history.push(crate::core::types::Message {
            role: crate::core::types::Role::Assistant,
            content: output.clone(),
        });
    }

    let state = TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: history
            .iter()
            .find(|message| matches!(message.role, crate::core::types::Role::User))
            .map(|message| message.content.clone())
            .unwrap_or_default(),
        step: initial_step + steps,
        history,
        summary: initial_summary,
    };
    if let Err(e) = state_store.write_task_state(&state).await {
        tracing::warn!("Failed to write task_state.json: {}", e);
    }

    // Write report.json
    let mut report = RunReport::new(run_id, final_reason);
    report.steps = steps;
    report.total_usage = total_usage;
    report.tool_calls = tool_calls;
    report.tool_failures = tool_failures;
    report.output = final_output;

    if let Err(e) = write_report(&run_dir, &report) {
        tracing::warn!("Failed to write report.json: {}", e);
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
