use std::io::Write;
use std::path::PathBuf;

use futures::StreamExt;

use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{RunId, RunRequest, TaskPlan, TaskState, TerminationReason, Usage};
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
    let mut plan: Option<TaskPlan> = resume_state.as_ref().and_then(|state| state.plan.clone());

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
            StreamEvent::PlanCreated { plan: new_plan } => {
                eprintln!("\n  [plan] {} steps", new_plan.steps.len());
                plan = Some(new_plan);
                write_task_snapshot(SnapshotWrite {
                    state_store,
                    session_id,
                    job_id,
                    run_id,
                    goal: &message,
                    step: initial_step + steps,
                    history: &history,
                    summary: initial_summary.clone(),
                    plan: plan.clone(),
                })
                .await;
            }
            StreamEvent::PlanStepStarted { step, index } => {
                eprintln!("  [step {}] {}", index + 1, step.title);
            }
            StreamEvent::PlanStepCompleted { step, .. } => {
                if let Some(active_plan) = plan.as_mut()
                    && let Some(saved_step) = active_plan
                        .steps
                        .iter_mut()
                        .find(|saved_step| saved_step.id == step.id)
                {
                    saved_step.done = true;
                    active_plan.current_step = active_plan
                        .steps
                        .iter()
                        .position(|saved_step| !saved_step.done)
                        .unwrap_or(active_plan.steps.len());
                }
                write_task_snapshot(SnapshotWrite {
                    state_store,
                    session_id,
                    job_id,
                    run_id,
                    goal: &message,
                    step: initial_step + steps,
                    history: &history,
                    summary: initial_summary.clone(),
                    plan: plan.clone(),
                })
                .await;
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

    let goal = message.clone();
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

    write_task_snapshot(SnapshotWrite {
        state_store,
        session_id,
        job_id,
        run_id,
        goal: &goal,
        step: initial_step + steps,
        history: &history,
        summary: initial_summary,
        plan,
    })
    .await;

    // Write report.json
    let workspace = engine.workspace();
    let mut report = RunReport::new(
        session_id,
        job_id,
        run_id,
        workspace.root.clone(),
        workspace.kind.clone(),
        engine.model_id().to_string(),
        final_reason,
    );
    report.steps = steps;
    report.total_usage = total_usage;
    report.tool_calls = tool_calls;
    report.tool_failures = tool_failures;
    report.output = final_output;

    if let Err(e) = write_report(&run_dir, &report) {
        tracing::warn!("Failed to write report.json: {}", e);
    }
}

struct SnapshotWrite<'a> {
    state_store: &'a StateStore,
    session_id: crate::core::types::SessionId,
    job_id: crate::core::types::JobId,
    run_id: RunId,
    goal: &'a str,
    step: u32,
    history: &'a [crate::core::types::Message],
    summary: Option<String>,
    plan: Option<TaskPlan>,
}

async fn write_task_snapshot(args: SnapshotWrite<'_>) {
    let state = TaskState {
        schema_version: 1,
        session_id: args.session_id,
        job_id: args.job_id,
        run_id: args.run_id,
        goal: args
            .history
            .iter()
            .find(|message| matches!(message.role, crate::core::types::Role::User))
            .map(|message| message.content.clone())
            .unwrap_or_else(|| args.goal.to_string()),
        step: args.step,
        history: args.history.to_vec(),
        summary: args.summary,
        plan: args.plan,
    };
    if let Err(e) = args.state_store.write_task_state(&state).await {
        tracing::warn!("Failed to write task_state.json: {}", e);
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
