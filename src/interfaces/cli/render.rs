use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use futures::{Stream, StreamExt};
use serde::Deserialize;

use crate::core::events::StreamEvent;
use crate::core::runtime_identity::RuntimeIdentity;
use crate::core::types::{CallId, TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::interfaces::terminal::view::{RunViewState, RunViewUpdate};
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
    let mut view_state = RunViewState::default();
    let mut render_state = ReplLineRenderState::new(relative_report_path(workspace, &run_dir));

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        let update = view_state.apply_event(&event);
        if let Some(reason) = render_repl_update(update, options, &mut render_state) {
            terminal_reason = reason;
            break;
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

struct ReplLineRenderState {
    plan_step_count: usize,
    tool_call_count: usize,
    tool_failure_count: usize,
    printed_plan: bool,
    printed_assistant: bool,
    assistant_at_line_start: bool,
    report_path: String,
    tool_names: HashMap<CallId, String>,
}

impl ReplLineRenderState {
    fn new(report_path: String) -> Self {
        Self {
            plan_step_count: 0,
            tool_call_count: 0,
            tool_failure_count: 0,
            printed_plan: false,
            printed_assistant: false,
            assistant_at_line_start: true,
            report_path,
            tool_names: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplToolStartBlock {
    Tool { name: String, args: String },
    Command { command: String },
}

#[derive(Debug, Deserialize)]
struct ShellOutputView {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

fn repl_tool_start_block(name: &str, args: &serde_json::Value) -> ReplToolStartBlock {
    if name == "shell"
        && let Some(command) = args.get("command").and_then(|value| value.as_str())
    {
        return ReplToolStartBlock::Command {
            command: command.to_string(),
        };
    }

    ReplToolStartBlock::Tool {
        name: name.to_string(),
        args: args.to_string(),
    }
}

fn shell_result_summary(result: &crate::core::types::ToolResult) -> Option<Vec<String>> {
    let output: ShellOutputView = serde_json::from_str(&result.output).ok()?;
    let mut lines = Vec::new();
    let exit = output
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    lines.push(format!("exit {exit}"));

    let stdout = output.stdout.trim();
    if !stdout.is_empty() {
        lines.push(format!("stdout {}", truncate(stdout, 200)));
    }

    let stderr = output.stderr.trim();
    if !stderr.is_empty() {
        lines.push(format!("stderr {}", truncate(stderr, 200)));
    }

    if output.stdout_truncated {
        lines.push("stdout truncated".to_string());
    }
    if output.stderr_truncated {
        lines.push("stderr truncated".to_string());
    }

    Some(lines)
}

fn render_repl_update(
    update: RunViewUpdate,
    options: CliRunRenderOptions,
    render_state: &mut ReplLineRenderState,
) -> Option<TerminationReason> {
    match update {
        RunViewUpdate::RunStarted { user_message, .. } if is_repl(options) => {
            print_repl_block("You", user_message);
            None
        }
        RunViewUpdate::AssistantDelta { delta } => {
            if is_repl(options) {
                if !render_state.printed_assistant {
                    eprintln!("\nAssistant");
                    render_state.printed_assistant = true;
                }
                print_indented(&delta, &mut render_state.assistant_at_line_start);
                let _ = std::io::stdout().flush();
            } else {
                print!("{}", delta);
                let _ = std::io::stdout().flush();
            }
            None
        }
        RunViewUpdate::ModelStatus { status, message } => {
            if is_repl(options) {
                eprintln!(
                    "\n{}",
                    repl_update_label(&RunViewUpdate::ModelStatus {
                        status,
                        message: message.clone(),
                    })?
                );
                eprintln!("  {message}");
            }
            None
        }
        RunViewUpdate::ToolCallStarted {
            call_id,
            name,
            args,
        } => {
            render_state.tool_call_count += 1;
            render_state.tool_names.insert(call_id, name.clone());
            if is_repl(options) {
                match repl_tool_start_block(&name, &args) {
                    ReplToolStartBlock::Command { command } => {
                        eprintln!("\nCommand");
                        eprintln!("  {command}");
                    }
                    ReplToolStartBlock::Tool { name, args } => {
                        eprintln!("\nTool · {name}");
                        eprintln!("  {}", truncate(&args, 200));
                    }
                }
            } else {
                eprintln!("\n  [tool] {}({})", name, args);
            }
            None
        }
        RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name,
            args,
            reason,
        } => {
            render_state.tool_names.insert(call_id, name.clone());
            if is_repl(options) {
                eprintln!(
                    "\n{}",
                    repl_update_label(&RunViewUpdate::ToolCallApprovalNeeded {
                        call_id,
                        name,
                        args: args.clone(),
                        reason: reason.clone(),
                    })?
                );
                eprintln!("  {}", truncate(&args.to_string(), 200));
                eprintln!("  {reason}");
            }
            None
        }
        RunViewUpdate::ToolCallCompleted { call_id, result } => {
            if is_repl(options) {
                if render_state
                    .tool_names
                    .get(&call_id)
                    .map(|name| name == "shell")
                    .unwrap_or(false)
                {
                    if let Some(lines) = shell_result_summary(&result) {
                        for line in lines {
                            eprintln!("  {line}");
                        }
                    } else {
                        eprintln!("  {}", truncate(&result.output, 200));
                    }
                } else {
                    eprintln!("  {}", truncate(&result.output, 200));
                }
            } else {
                eprintln!("  [result] {}", truncate(&result.output, 200));
            }
            None
        }
        RunViewUpdate::ToolCallFailed { call_id, error } => {
            render_state.tool_failure_count += 1;
            let tool_name = render_state
                .tool_names
                .remove(&call_id)
                .unwrap_or_else(|| "tool".to_string());
            if is_repl(options) {
                eprintln!("\nError · {tool_name}");
                eprintln!("  {}", error);
            } else {
                eprintln!("  [error] {}", error);
            }
            None
        }
        RunViewUpdate::InputNeeded { prompt, .. } => {
            if is_repl(options) {
                eprintln!(
                    "\n{}",
                    repl_update_label(&RunViewUpdate::InputNeeded {
                        input_id: CallId::new(),
                        prompt: prompt.clone(),
                    })?
                );
                eprintln!("  {prompt}");
            }
            None
        }
        RunViewUpdate::PlanCreated { plan: new_plan } => {
            render_state.plan_step_count = new_plan.steps.len();
            render_state.printed_plan = true;
            if is_repl(options) {
                eprintln!("\nPlan · {}", count_label(new_plan.steps.len(), "step"));
                for (index, step) in new_plan.steps.iter().enumerate() {
                    eprintln!("  {}. {}", index + 1, step.title);
                }
            } else {
                eprintln!("\n  [plan] {} steps", new_plan.steps.len());
            }
            None
        }
        RunViewUpdate::PlanStepStarted { step, index }
            if !is_repl(options) || !render_state.printed_plan =>
        {
            eprintln!("  [step {}] {}", index + 1, step.title);
            None
        }
        RunViewUpdate::PromptCompacted { summary, state } => {
            if is_repl(options) {
                eprintln!(
                    "\n{}",
                    repl_update_label(&RunViewUpdate::PromptCompacted {
                        summary: summary.clone(),
                        state,
                    })?
                );
                if let Some(summary) = summary {
                    eprintln!("  {}", truncate(&summary, 200));
                }
            }
            None
        }
        RunViewUpdate::MemoryFlushed { note_count } => {
            if is_repl(options) {
                eprintln!(
                    "\n{}",
                    repl_update_label(&RunViewUpdate::MemoryFlushed { note_count })?
                );
            }
            None
        }
        RunViewUpdate::RunCompleted { reason, output } => {
            Some(render_completion(reason, output, options, render_state))
        }
        _ => None,
    }
}

fn render_completion(
    reason: TerminationReason,
    output: Option<String>,
    options: CliRunRenderOptions,
    render_state: &ReplLineRenderState,
) -> TerminationReason {
    if let Some(ref text) = output
        && !matches!(&reason, TerminationReason::Final)
    {
        println!("\n{}", text);
    }
    if options.print_done_line {
        if is_repl(options) {
            eprintln!("\nDone");
            eprintln!(
                "  {} · {} · {} · {}",
                termination_reason_label(&reason),
                count_label(render_state.plan_step_count, "step"),
                count_label(render_state.tool_call_count, "tool"),
                count_label(render_state.tool_failure_count, "failure"),
            );
            eprintln!("  report {}", render_state.report_path);
        } else {
            eprintln!("\n  [done] {:?}", reason);
        }
    }
    reason
}

fn repl_update_label(update: &RunViewUpdate) -> Option<String> {
    match update {
        RunViewUpdate::ModelStatus { status, .. } => Some(format!("Status · {status}")),
        RunViewUpdate::ToolCallApprovalNeeded { name, .. } => Some(format!("Approval · {name}")),
        RunViewUpdate::InputNeeded { .. } => Some("Input".to_string()),
        RunViewUpdate::PromptCompacted { .. } => Some("Context · compacted".to_string()),
        RunViewUpdate::MemoryFlushed { note_count } => {
            Some(format!("Context · flushed {note_count} memory note(s)"))
        }
        _ => None,
    }
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
        CallId, JobId, PlanStep, PromptCompactionMode, PromptCompactionState, RunId, SessionId,
        TaskPlan, TerminationReason, ToolResult,
    };
    use crate::core::workspace::Workspace;
    use crate::errors::ToolError;
    use crate::interfaces::terminal::view::RunViewUpdate;
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
                    metadata: Default::default(),
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
                metadata: Default::default(),
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
    fn repl_update_labels_cover_pending_terminal_updates() {
        let call_id = CallId::new();

        assert_eq!(
            super::repl_update_label(&RunViewUpdate::ModelStatus {
                status: "thinking".to_string(),
                message: "checking files".to_string(),
            }),
            Some("Status · thinking".to_string())
        );
        assert_eq!(
            super::repl_update_label(&RunViewUpdate::ToolCallApprovalNeeded {
                call_id,
                name: "fs_write".to_string(),
                args: serde_json::json!({"path":"out.txt"}),
                reason: "writes a file".to_string(),
            }),
            Some("Approval · fs_write".to_string())
        );
        assert_eq!(
            super::repl_update_label(&RunViewUpdate::InputNeeded {
                input_id: CallId::new(),
                prompt: "Which branch?".to_string(),
            }),
            Some("Input".to_string())
        );
        assert_eq!(
            super::repl_update_label(&RunViewUpdate::PromptCompacted {
                summary: Some("short summary".to_string()),
                state: PromptCompactionState {
                    mode: PromptCompactionMode::Deterministic,
                    auto_triggered: false,
                    degraded: false,
                    consecutive_failures: 0,
                    circuit_open: false,
                    model: None,
                    prompt_version: None,
                    source_message_count: 4,
                    last_error: None,
                },
            }),
            Some("Context · compacted".to_string())
        );
    }

    #[test]
    fn repl_tool_start_block_uses_command_for_shell_tool() {
        assert_eq!(
            super::repl_tool_start_block("shell", &serde_json::json!({"command":"cargo test"})),
            super::ReplToolStartBlock::Command {
                command: "cargo test".to_string()
            }
        );
        assert_eq!(
            super::repl_tool_start_block("fs_read", &serde_json::json!({"path":"README.md"})),
            super::ReplToolStartBlock::Tool {
                name: "fs_read".to_string(),
                args: "{\"path\":\"README.md\"}".to_string()
            }
        );
    }

    #[test]
    fn shell_result_summary_formats_exit_code_and_streams() {
        let call_id = CallId::new();
        let result = ToolResult {
            call_id,
            output: serde_json::json!({
                "command": "cargo test",
                "success": true,
                "exit_code": 0,
                "stdout": "ok\n",
                "stderr": "",
                "stdout_truncated": false,
                "stderr_truncated": false
            })
            .to_string(),
            mutations: Vec::new(),
            metadata: Default::default(),
        };

        let lines = super::shell_result_summary(&result).unwrap();

        assert_eq!(lines[0], "exit 0");
        assert_eq!(lines[1], "stdout ok");
    }

    #[test]
    fn shell_result_summary_marks_truncated_output() {
        let call_id = CallId::new();
        let result = ToolResult {
            call_id,
            output: serde_json::json!({
                "command": "cargo test",
                "success": false,
                "exit_code": 101,
                "stdout": "",
                "stderr": "failure\n",
                "stdout_truncated": false,
                "stderr_truncated": true
            })
            .to_string(),
            mutations: Vec::new(),
            metadata: Default::default(),
        };

        let lines = super::shell_result_summary(&result).unwrap();

        assert_eq!(lines[0], "exit 101");
        assert_eq!(lines[1], "stderr failure");
        assert_eq!(lines[2], "stderr truncated");
    }

    #[tokio::test]
    async fn repl_compact_render_handles_pending_terminal_updates() {
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
                user_message: "needs context".to_string(),
            },
            StreamEvent::ModelStatus {
                status: "thinking".to_string(),
                message: "checking files".to_string(),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name: "fs_write".to_string(),
                args: serde_json::json!({"path":"out.txt"}),
                reason: "writes a file".to_string(),
            },
            StreamEvent::InputNeeded {
                input_id: CallId::new(),
                prompt: "Which branch?".to_string(),
            },
            StreamEvent::PromptCompacted {
                summary: Some("short summary".to_string()),
                state: PromptCompactionState {
                    mode: PromptCompactionMode::Deterministic,
                    auto_triggered: false,
                    degraded: false,
                    consecutive_failures: 0,
                    circuit_open: false,
                    model: None,
                    prompt_version: None,
                    source_message_count: 4,
                    last_error: None,
                },
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ok".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "needs context".to_string(),
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

    #[test]
    fn truncate_handles_multibyte_text() {
        assert_eq!(super::truncate("工具输出", 3), "工具输");
    }
}
