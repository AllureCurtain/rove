use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use futures::{Stream, StreamExt};

use crate::core::events::StreamEvent;
use crate::core::runtime_identity::RuntimeIdentity;
use crate::core::types::{CallId, TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::{RunHandle, StateStore};

pub struct CliRunRenderContext<'a> {
    pub message: String,
    pub run: RunHandle,
    pub resume_state: Option<TaskState>,
    pub state_store: &'a StateStore,
    pub workspace: &'a Workspace,
    pub model_id: &'a str,
    pub runtime_identity: Option<RuntimeIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRunRenderMode {
    OneShot,
    ReplCompact,
}

#[derive(Debug, Clone, Copy)]
pub struct CliRunRenderOptions {
    pub mode: CliRunRenderMode,
    pub print_done_line: bool,
    pub print_trailing_newline: bool,
}

impl Default for CliRunRenderOptions {
    fn default() -> Self {
        Self {
            mode: CliRunRenderMode::OneShot,
            print_done_line: true,
            print_trailing_newline: true,
        }
    }
}

pub async fn render_run_events<S>(
    stream: S,
    context: CliRunRenderContext<'_>,
    options: CliRunRenderOptions,
) -> TerminationReason
where
    S: Stream<Item = StreamEvent>,
{
    futures::pin_mut!(stream);
    let CliRunRenderContext {
        message,
        run,
        resume_state,
        state_store,
        workspace,
        model_id,
        runtime_identity,
    } = context;
    let resume_state_for_recorder = resume_state.clone();
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
        resume_state_for_recorder.as_ref(),
        runtime_identity,
    );
    let mut terminal_reason = TerminationReason::Error;
    let mut plan_step_count = 0usize;
    let mut tool_call_count = 0usize;
    let mut tool_failure_count = 0usize;
    let mut printed_plan = false;
    let mut printed_assistant = false;
    let mut assistant_at_line_start = true;
    let mut tool_names: HashMap<CallId, String> = HashMap::new();

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        match event {
            StreamEvent::RunStarted { user_message, .. } if is_repl(options) => {
                print_repl_block("You", user_message);
            }
            StreamEvent::LlmChunk { delta } => {
                if is_repl(options) {
                    if !printed_assistant {
                        eprintln!("\nAssistant");
                        printed_assistant = true;
                    }
                    print_indented(&delta, &mut assistant_at_line_start);
                    let _ = std::io::stdout().flush();
                } else {
                    print!("{}", delta);
                    let _ = std::io::stdout().flush();
                }
            }
            StreamEvent::ToolCallStarted {
                call_id,
                name,
                args,
                ..
            } => {
                tool_call_count += 1;
                tool_names.insert(call_id, name.clone());
                if is_repl(options) {
                    eprintln!("\nTool · {name}");
                    eprintln!("  {}", truncate(&args.to_string(), 200));
                } else {
                    eprintln!("\n  [tool] {}({})", name, args);
                }
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                if is_repl(options) {
                    eprintln!("  {}", truncate(&result.output, 200));
                } else {
                    eprintln!("  [result] {}", truncate(&result.output, 200));
                }
            }
            StreamEvent::ToolCallFailed { call_id, error } => {
                tool_failure_count += 1;
                let tool_name = tool_names
                    .remove(&call_id)
                    .unwrap_or_else(|| "tool".to_string());
                if is_repl(options) {
                    eprintln!("\nError · {tool_name}");
                    eprintln!("  {}", error);
                } else {
                    eprintln!("  [error] {}", error);
                }
            }
            StreamEvent::PlanCreated { plan: new_plan } => {
                plan_step_count = new_plan.steps.len();
                printed_plan = true;
                if is_repl(options) {
                    eprintln!("\nPlan · {}", count_label(new_plan.steps.len(), "step"));
                    for (index, step) in new_plan.steps.iter().enumerate() {
                        eprintln!("  {}. {}", index + 1, step.title);
                    }
                } else {
                    eprintln!("\n  [plan] {} steps", new_plan.steps.len());
                }
            }
            StreamEvent::PlanStepStarted { step, index } if !is_repl(options) || !printed_plan => {
                eprintln!("  [step {}] {}", index + 1, step.title);
            }
            StreamEvent::RunCompleted { reason, output } => {
                terminal_reason = reason.clone();
                if let Some(ref text) = output
                    && !matches!(reason, TerminationReason::Final)
                {
                    println!("\n{}", text);
                }
                if options.print_done_line {
                    if is_repl(options) {
                        eprintln!("\nDone");
                        eprintln!(
                            "  {} · {} · {} · {}",
                            termination_reason_label(&reason),
                            count_label(plan_step_count, "step"),
                            count_label(tool_call_count, "tool"),
                            count_label(tool_failure_count, "failure"),
                        );
                        eprintln!("  report {}", relative_report_path(workspace, &run_dir));
                    } else {
                        eprintln!("\n  [done] {:?}", reason);
                    }
                }
                break;
            }
            _ => {}
        }
    }

    if options.print_trailing_newline {
        println!();
    }

    recorder
        .finalize(state_store, workspace, model_id, &run_dir)
        .await;
    terminal_reason
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

fn is_repl(options: CliRunRenderOptions) -> bool {
    matches!(options.mode, CliRunRenderMode::ReplCompact)
}

fn relative_report_path(workspace: &Workspace, run_dir: &Path) -> String {
    let report_path = run_dir.join("report.json");
    report_path
        .strip_prefix(&workspace.root)
        .unwrap_or(&report_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn print_repl_block(label: &str, detail: impl AsRef<str>) {
    eprintln!("{label}");
    let detail = detail.as_ref();
    if !detail.is_empty() {
        for line in detail.lines() {
            eprintln!("  {line}");
        }
    }
}

fn print_indented(text: &str, at_line_start: &mut bool) {
    for ch in text.chars() {
        if *at_line_start {
            print!("  ");
            *at_line_start = false;
        }
        print!("{ch}");
        if ch == '\n' {
            *at_line_start = true;
        }
    }
}

fn count_label(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn termination_reason_label(reason: &TerminationReason) -> &'static str {
    match reason {
        TerminationReason::Final => "final",
        TerminationReason::StepLimit => "step_limit",
        TerminationReason::TokenLimit => "token_limit",
        TerminationReason::TimeLimit => "time_limit",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use futures::stream;

    use crate::core::events::StreamEvent;
    use crate::core::types::{
        CallId, JobId, PlanStep, RunId, SessionId, TaskPlan, TerminationReason, ToolResult,
    };
    use crate::core::workspace::Workspace;
    use crate::errors::ToolError;
    use crate::state::store::StateStore;

    use super::{CliRunRenderContext, CliRunRenderMode, CliRunRenderOptions, render_run_events};

    #[tokio::test]
    async fn render_events_returns_terminal_reason() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "hi".to_string(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("hi".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "hello".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            CliRunRenderOptions::default(),
        )
        .await;

        assert_eq!(reason, TerminationReason::Final);
    }

    #[tokio::test]
    async fn repl_compact_render_prints_terminal_reason() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("hi".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "hello".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            CliRunRenderOptions {
                mode: CliRunRenderMode::ReplCompact,
                print_done_line: true,
                print_trailing_newline: true,
            },
        )
        .await;

        assert_eq!(reason, TerminationReason::Final);
    }

    #[tokio::test]
    async fn repl_compact_render_handles_plan_and_tool_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let call_id = CallId::new();
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "use echo".to_string(),
            },
            StreamEvent::PlanCreated {
                plan: TaskPlan {
                    goal: "use echo".to_string(),
                    steps: vec![PlanStep {
                        id: "step-1".to_string(),
                        title: "Run echo".to_string(),
                        done: false,
                    }],
                    current_step: 0,
                },
            },
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id: None,
                name: "echo".to_string(),
                args: serde_json::json!({"message":"hello"}),
            },
            StreamEvent::ToolCallCompleted {
                call_id,
                result: ToolResult {
                    call_id,
                    output: "hello".to_string(),
                    mutations: Vec::new(),
                },
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("hello".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "use echo".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            CliRunRenderOptions {
                mode: CliRunRenderMode::ReplCompact,
                print_done_line: true,
                print_trailing_newline: true,
            },
        )
        .await;

        assert_eq!(reason, TerminationReason::Final);
    }

    #[tokio::test]
    async fn repl_compact_render_handles_tool_failure_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let call_id = CallId::new();
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "use echo".to_string(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id: None,
                name: "echo".to_string(),
                args: serde_json::json!({"message":"hello"}),
            },
            StreamEvent::ToolCallFailed {
                call_id,
                error: ToolError::ExecutionFailed {
                    reason: "boom".to_string(),
                },
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Error,
                output: Some("boom".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "use echo".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
                runtime_identity: None,
            },
            CliRunRenderOptions {
                mode: CliRunRenderMode::ReplCompact,
                print_done_line: true,
                print_trailing_newline: true,
            },
        )
        .await;

        assert_eq!(reason, TerminationReason::Error);
    }

    #[test]
    fn truncate_handles_multibyte_text() {
        assert_eq!(super::truncate("工具输出", 3), "工具输");
    }
}
