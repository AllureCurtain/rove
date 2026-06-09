# TUI-Ready Terminal Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare Rove's terminal surface for a future full TUI by extracting event-to-view state, line rendering, and terminal action contracts while preserving the current compact REPL behavior.

**Architecture:** Keep `core` unchanged and treat `StreamEvent` as the source of truth. Add a pure `interfaces::terminal` layer that converts events into view updates/state and maps terminal actions independently from any renderer. Refactor the existing CLI renderer to consume that layer, without adding `ratatui`, `crossterm`, alternate-screen mode, mouse handling, or full-screen layout.

**Tech Stack:** Rust 2024, existing `StreamEvent`, existing CLI/API runtime modules, `rustyline`, existing test stack. No new runtime dependencies in this pass.

---

## Worktree

Implementation must happen in the isolated worktree:

```powershell
cd D:\Study\project\agent\rove\.worktrees\tui-ready-terminal-architecture
git status --short --branch
```

Expected branch:

```text
## feature/tui-ready-terminal-architecture
```

Baseline already confirmed with:

```powershell
cargo test interfaces::cli::render --lib
```

Expected result:

```text
test result: ok. 5 passed; 0 failed
```

The earlier broad `cargo build` and `cargo test --lib` attempts timed out during first compile; no cargo or rustc process was left running afterward. Run the final full verification at the end of this plan.

## File Map

- Create `src/interfaces/terminal/mod.rs`: terminal-surface module exports.
- Create `src/interfaces/terminal/view.rs`: pure event-to-view state and event update types. No stdout/stderr, no API calls, no terminal dependencies.
- Create `src/interfaces/terminal/action.rs`: renderer-independent terminal actions for prompt submission, cancel, approval, input answer, resume, status, sessions, clear, and exit.
- Modify `src/interfaces/mod.rs`: export `terminal`.
- Modify `src/interfaces/cli/render.rs`: use `terminal::view` updates and state instead of owning display state directly.
- Modify `src/interfaces/cli/repl.rs`: map slash commands to `TerminalAction` while keeping current command behavior.
- Modify `src/interfaces/cli/ui.rs`: add line-format helpers for new update types when useful; keep welcome/status helpers intact.
- Modify `web-ui/lib/rove-types.ts`: no changes expected. Use it as a parity reference only.
- Modify `docs/runtime/implementation-guide.md`: document that compact REPL now uses the terminal view/action layer and remains non-full-screen.
- Test `src/interfaces/terminal/view.rs`: event mapping and state transitions.
- Test `src/interfaces/terminal/action.rs`: slash/action conversion behavior.
- Test `src/interfaces/cli/render.rs`: compact renderer still handles plan/tool/error/done, plus pending approval/input/model-status updates.
- Test `src/interfaces/cli/repl.rs`: slash parser still recognizes current commands.
- Test `tests/cli_repl.rs`: no-regression smoke tests for compact REPL.

## Task 1: Add Terminal Module Skeleton

**Files:**
- Create: `src/interfaces/terminal/mod.rs`
- Create: `src/interfaces/terminal/view.rs`
- Create: `src/interfaces/terminal/action.rs`
- Modify: `src/interfaces/mod.rs`

- [ ] **Step 1: Create failing module export test**

Create `src/interfaces/terminal/mod.rs` with:

```rust
pub mod action;
pub mod view;
```

Create `src/interfaces/terminal/view.rs` with:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunViewUpdate {
    RunStarted { user_message: String },
}
```

Create `src/interfaces/terminal/action.rs` with:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAction {
    ShowStatus,
}
```

Add this test at the bottom of `src/interfaces/terminal/view.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::terminal::view::RunViewUpdate;

    #[test]
    fn terminal_module_exports_view_and_action_types() {
        let update = RunViewUpdate::RunStarted {
            user_message: "hello".to_string(),
        };
        assert_eq!(
            update,
            RunViewUpdate::RunStarted {
                user_message: "hello".to_string()
            }
        );
        assert_eq!(TerminalAction::ShowStatus, TerminalAction::ShowStatus);
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```powershell
cargo test interfaces::terminal::view::tests::terminal_module_exports_view_and_action_types --lib
```

Expected: compile failure because `interfaces::terminal` is not exported from `src/interfaces/mod.rs`.

- [ ] **Step 3: Export the terminal module**

Modify `src/interfaces/mod.rs`:

```rust
pub mod api;
pub mod cli;
pub mod terminal;
```

- [ ] **Step 4: Run the module test**

Run:

```powershell
cargo test interfaces::terminal::view::tests::terminal_module_exports_view_and_action_types --lib
```

Expected: pass.

- [ ] **Step 5: Commit the module skeleton**

Run:

```powershell
git add src/interfaces/mod.rs src/interfaces/terminal/mod.rs src/interfaces/terminal/view.rs src/interfaces/terminal/action.rs
git commit -m "refactor: add terminal surface module"
```

Expected: commit succeeds.

## Task 2: Define Run View Updates And Exhaustive Event Mapping

**Files:**
- Modify: `src/interfaces/terminal/view.rs`

- [ ] **Step 1: Replace the stub update enum with the full update contract**

Replace the contents of `src/interfaces/terminal/view.rs` above the test module with:

```rust
use crate::core::events::StreamEvent;
use crate::core::types::{
    CallId, JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason,
    ToolResult, Usage,
};
use crate::errors::ToolError;

#[derive(Debug, Clone)]
pub enum RunViewUpdate {
    RunStarted {
        run_id: RunId,
        job_id: JobId,
        user_message: String,
    },
    AssistantDelta {
        delta: String,
    },
    ModelStatus {
        status: String,
        message: String,
    },
    LlmMessage {
        full: String,
        usage: Usage,
        tool_call_count: usize,
    },
    ToolCallStarted {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
    },
    ToolCallApprovalNeeded {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
        reason: String,
    },
    ToolCallCompleted {
        call_id: CallId,
        result: ToolResult,
    },
    ToolCallFailed {
        call_id: CallId,
        error: ToolError,
    },
    InputNeeded {
        input_id: CallId,
        prompt: String,
    },
    PlanCreated {
        plan: TaskPlan,
    },
    PlanStepStarted {
        step: PlanStep,
        index: usize,
    },
    PlanStepCompleted {
        step: PlanStep,
        index: usize,
    },
    PlanStepFailed {
        step: PlanStep,
        index: usize,
        reason: String,
    },
    PromptCompacted {
        summary: Option<String>,
        state: PromptCompactionState,
    },
    RunCompleted {
        reason: TerminationReason,
        output: Option<String>,
    },
}

impl From<&StreamEvent> for RunViewUpdate {
    fn from(event: &StreamEvent) -> Self {
        match event {
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message,
            } => Self::RunStarted {
                run_id: *run_id,
                job_id: *job_id,
                user_message: user_message.clone(),
            },
            StreamEvent::LlmChunk { delta } => Self::AssistantDelta {
                delta: delta.clone(),
            },
            StreamEvent::ModelStatus { status, message } => Self::ModelStatus {
                status: status.clone(),
                message: message.clone(),
            },
            StreamEvent::LlmMessage {
                full,
                usage,
                tool_calls,
            } => Self::LlmMessage {
                full: full.clone(),
                usage: usage.clone(),
                tool_call_count: tool_calls.len(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                name,
                args,
                ..
            } => Self::ToolCallStarted {
                call_id: *call_id,
                name: name.clone(),
                args: args.clone(),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name,
                args,
                reason,
            } => Self::ToolCallApprovalNeeded {
                call_id: *call_id,
                name: name.clone(),
                args: args.clone(),
                reason: reason.clone(),
            },
            StreamEvent::ToolCallCompleted { call_id, result } => Self::ToolCallCompleted {
                call_id: *call_id,
                result: result.clone(),
            },
            StreamEvent::ToolCallFailed { call_id, error } => Self::ToolCallFailed {
                call_id: *call_id,
                error: error.clone(),
            },
            StreamEvent::InputNeeded { input_id, prompt } => Self::InputNeeded {
                input_id: *input_id,
                prompt: prompt.clone(),
            },
            StreamEvent::PlanCreated { plan } => Self::PlanCreated { plan: plan.clone() },
            StreamEvent::PlanStepStarted { step, index } => Self::PlanStepStarted {
                step: step.clone(),
                index: *index,
            },
            StreamEvent::PlanStepCompleted { step, index } => Self::PlanStepCompleted {
                step: step.clone(),
                index: *index,
            },
            StreamEvent::PlanStepFailed {
                step,
                index,
                reason,
            } => Self::PlanStepFailed {
                step: step.clone(),
                index: *index,
                reason: reason.clone(),
            },
            StreamEvent::PromptCompacted { summary, state } => Self::PromptCompacted {
                summary: summary.clone(),
                state: state.clone(),
            },
            StreamEvent::RunCompleted { reason, output } => Self::RunCompleted {
                reason: reason.clone(),
                output: output.clone(),
            },
        }
    }
}
```

- [ ] **Step 2: Add event mapping tests**

Replace the temporary test module in `src/interfaces/terminal/view.rs` with:

```rust
#[cfg(test)]
mod tests {
    use crate::core::events::StreamEvent;
    use crate::core::types::{
        CallId, JobId, PlanStep, PromptCompactionMode, PromptCompactionState, RunId, TaskPlan,
        TerminationReason, ToolResult, Usage,
    };
    use crate::errors::ToolError;
    use crate::interfaces::terminal::view::RunViewUpdate;

    fn usage() -> Usage {
        Usage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }
    }

    fn step() -> PlanStep {
        PlanStep {
            id: "step-1".to_string(),
            title: "Read files".to_string(),
            done: false,
        }
    }

    #[test]
    fn maps_all_stream_events_to_terminal_updates() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let input_id = CallId::new();
        let plan = TaskPlan {
            goal: "goal".to_string(),
            steps: vec![step()],
            current_step: 0,
        };
        let compaction = PromptCompactionState {
            mode: PromptCompactionMode::Deterministic,
            auto_triggered: false,
            degraded: false,
            consecutive_failures: 0,
            circuit_open: false,
            model: None,
            prompt_version: None,
            source_message_count: 4,
            last_error: None,
        };
        let events = vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "hi".to_string(),
            },
            StreamEvent::ModelStatus {
                status: "thinking".to_string(),
                message: "planning".to_string(),
            },
            StreamEvent::LlmMessage {
                full: "full".to_string(),
                usage: usage(),
                tool_calls: Vec::new(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id: None,
                name: "fs_read".to_string(),
                args: serde_json::json!({"path":"README.md"}),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name: "fs_write".to_string(),
                args: serde_json::json!({"path":"out.txt"}),
                reason: "writes a file".to_string(),
            },
            StreamEvent::ToolCallCompleted {
                call_id,
                result: ToolResult {
                    call_id,
                    output: "done".to_string(),
                    mutations: Vec::new(),
                },
            },
            StreamEvent::ToolCallFailed {
                call_id,
                error: ToolError::ExecutionFailed {
                    reason: "boom".to_string(),
                },
            },
            StreamEvent::InputNeeded {
                input_id,
                prompt: "Which branch?".to_string(),
            },
            StreamEvent::PlanCreated { plan: plan.clone() },
            StreamEvent::PlanStepStarted {
                step: step(),
                index: 0,
            },
            StreamEvent::PlanStepCompleted {
                step: step(),
                index: 0,
            },
            StreamEvent::PlanStepFailed {
                step: step(),
                index: 0,
                reason: "failed".to_string(),
            },
            StreamEvent::PromptCompacted {
                summary: Some("summary".to_string()),
                state: compaction,
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ok".to_string()),
            },
        ];

        let updates: Vec<RunViewUpdate> = events.iter().map(RunViewUpdate::from).collect();

        assert_eq!(updates.len(), 15);
        assert!(matches!(
            updates[0],
            RunViewUpdate::RunStarted {
                user_message: ref message,
                ..
            } if message == "hello"
        ));
        assert!(matches!(
            updates[4],
            RunViewUpdate::ToolCallStarted { ref name, .. } if name == "fs_read"
        ));
        assert!(matches!(
            updates[8],
            RunViewUpdate::InputNeeded { ref prompt, .. } if prompt == "Which branch?"
        ));
        assert!(matches!(
            updates[14],
            RunViewUpdate::RunCompleted {
                reason: TerminationReason::Final,
                ..
            }
        ));
    }
}
```

- [ ] **Step 3: Run the event mapping tests**

Run:

```powershell
cargo test interfaces::terminal::view::tests::maps_all_stream_events_to_terminal_updates --lib
```

Expected: pass. If a `PromptCompactionState` field differs, inspect `src/core/types.rs`, update the test constructor with the actual fields, and keep the assertion coverage equivalent.

- [ ] **Step 4: Commit event mapping**

Run:

```powershell
git add src/interfaces/terminal/view.rs
git commit -m "refactor: map stream events to terminal updates"
```

Expected: commit succeeds.

## Task 3: Add Run View State Accumulator

**Files:**
- Modify: `src/interfaces/terminal/view.rs`

- [ ] **Step 1: Add failing state accumulator tests**

Add this test to the test module in `src/interfaces/terminal/view.rs`:

```rust
#[test]
fn run_view_state_tracks_plan_tools_pending_items_and_completion() {
    let run_id = RunId::new();
    let job_id = JobId::new();
    let call_id = CallId::new();
    let input_id = CallId::new();
    let mut state = super::RunViewState::default();

    state.apply_update(RunViewUpdate::RunStarted {
        run_id,
        job_id,
        user_message: "hello".to_string(),
    });
    state.apply_update(RunViewUpdate::AssistantDelta {
        delta: "hi".to_string(),
    });
    state.apply_update(RunViewUpdate::PlanCreated {
        plan: TaskPlan {
            goal: "goal".to_string(),
            steps: vec![step()],
            current_step: 0,
        },
    });
    state.apply_update(RunViewUpdate::ToolCallStarted {
        call_id,
        name: "fs_read".to_string(),
        args: serde_json::json!({"path":"README.md"}),
    });
    state.apply_update(RunViewUpdate::ToolCallApprovalNeeded {
        call_id,
        name: "fs_write".to_string(),
        args: serde_json::json!({"path":"out.txt"}),
        reason: "writes a file".to_string(),
    });
    state.apply_update(RunViewUpdate::InputNeeded {
        input_id,
        prompt: "Which branch?".to_string(),
    });
    state.apply_update(RunViewUpdate::RunCompleted {
        reason: TerminationReason::Final,
        output: Some("ok".to_string()),
    });

    assert_eq!(state.run_id, Some(run_id));
    assert_eq!(state.job_id, Some(job_id));
    assert_eq!(state.user_message.as_deref(), Some("hello"));
    assert_eq!(state.assistant_text, "hi");
    assert_eq!(state.plan.as_ref().unwrap().steps.len(), 1);
    assert_eq!(state.tool_calls.len(), 1);
    assert_eq!(state.pending_approvals.len(), 1);
    assert_eq!(state.pending_inputs.len(), 1);
    assert_eq!(
        state.completed.as_ref().unwrap().reason,
        TerminationReason::Final
    );
}
```

Expected compile failure until `RunViewState` and related view structs exist.

- [ ] **Step 2: Add view state structs**

Add these definitions above `impl From<&StreamEvent> for RunViewUpdate`:

```rust
#[derive(Debug, Clone)]
pub struct ToolCallView {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub status: ToolCallStatus,
    pub output: Option<String>,
    pub error: Option<ToolError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Started,
    WaitingApproval,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInputView {
    pub input_id: CallId,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct RunCompletionView {
    pub reason: TerminationReason,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunViewState {
    pub run_id: Option<RunId>,
    pub job_id: Option<JobId>,
    pub user_message: Option<String>,
    pub assistant_text: String,
    pub model_status: Option<(String, String)>,
    pub last_usage: Option<Usage>,
    pub plan: Option<TaskPlan>,
    pub current_step: Option<(usize, PlanStep)>,
    pub failed_steps: Vec<(usize, PlanStep, String)>,
    pub prompt_compaction: Option<(Option<String>, PromptCompactionState)>,
    pub tool_calls: Vec<ToolCallView>,
    pub pending_approvals: Vec<PendingApprovalView>,
    pub pending_inputs: Vec<PendingInputView>,
    pub completed: Option<RunCompletionView>,
}
```

- [ ] **Step 3: Implement state application**

Add this impl below the struct definitions:

```rust
impl RunViewState {
    pub fn apply_event(&mut self, event: &StreamEvent) -> RunViewUpdate {
        let update = RunViewUpdate::from(event);
        self.apply_update(update.clone());
        update
    }

    pub fn apply_update(&mut self, update: RunViewUpdate) {
        match update {
            RunViewUpdate::RunStarted {
                run_id,
                job_id,
                user_message,
            } => {
                self.run_id = Some(run_id);
                self.job_id = Some(job_id);
                self.user_message = Some(user_message);
            }
            RunViewUpdate::AssistantDelta { delta } => {
                self.assistant_text.push_str(&delta);
            }
            RunViewUpdate::ModelStatus { status, message } => {
                self.model_status = Some((status, message));
            }
            RunViewUpdate::LlmMessage { usage, .. } => {
                self.last_usage = Some(usage);
            }
            RunViewUpdate::ToolCallStarted {
                call_id,
                name,
                args,
            } => {
                self.tool_calls.push(ToolCallView {
                    call_id,
                    name,
                    args,
                    status: ToolCallStatus::Started,
                    output: None,
                    error: None,
                });
            }
            RunViewUpdate::ToolCallApprovalNeeded {
                call_id,
                name,
                args,
                reason,
            } => {
                self.pending_approvals.push(PendingApprovalView {
                    call_id,
                    name: name.clone(),
                    args: args.clone(),
                    reason,
                });
                upsert_tool_status(
                    &mut self.tool_calls,
                    call_id,
                    name,
                    args,
                    ToolCallStatus::WaitingApproval,
                );
            }
            RunViewUpdate::ToolCallCompleted { call_id, result } => {
                if let Some(tool) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|tool| tool.call_id == call_id)
                {
                    tool.status = ToolCallStatus::Completed;
                    tool.output = Some(result.output);
                }
                self.pending_approvals
                    .retain(|approval| approval.call_id != call_id);
            }
            RunViewUpdate::ToolCallFailed { call_id, error } => {
                if let Some(tool) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|tool| tool.call_id == call_id)
                {
                    tool.status = ToolCallStatus::Failed;
                    tool.error = Some(error);
                }
                self.pending_approvals
                    .retain(|approval| approval.call_id != call_id);
            }
            RunViewUpdate::InputNeeded { input_id, prompt } => {
                self.pending_inputs.push(PendingInputView { input_id, prompt });
            }
            RunViewUpdate::PlanCreated { plan } => {
                self.plan = Some(plan);
            }
            RunViewUpdate::PlanStepStarted { step, index } => {
                self.current_step = Some((index, step));
            }
            RunViewUpdate::PlanStepCompleted { step, index } => {
                self.current_step = Some((index, step));
            }
            RunViewUpdate::PlanStepFailed {
                step,
                index,
                reason,
            } => {
                self.failed_steps.push((index, step, reason));
            }
            RunViewUpdate::PromptCompacted { summary, state } => {
                self.prompt_compaction = Some((summary, state));
            }
            RunViewUpdate::RunCompleted { reason, output } => {
                self.completed = Some(RunCompletionView { reason, output });
            }
        }
    }
}

fn upsert_tool_status(
    tools: &mut Vec<ToolCallView>,
    call_id: CallId,
    name: String,
    args: serde_json::Value,
    status: ToolCallStatus,
) {
    if let Some(tool) = tools.iter_mut().rev().find(|tool| tool.call_id == call_id) {
        tool.status = status;
    } else {
        tools.push(ToolCallView {
            call_id,
            name,
            args,
            status,
            output: None,
            error: None,
        });
    }
}
```

- [ ] **Step 4: Run state tests**

Run:

```powershell
cargo test interfaces::terminal::view --lib
```

Expected: all terminal view tests pass.

- [ ] **Step 5: Commit state accumulator**

Run:

```powershell
git add src/interfaces/terminal/view.rs
git commit -m "refactor: accumulate terminal run view state"
```

Expected: commit succeeds.

## Task 4: Define Terminal Actions And Slash Command Mapping

**Files:**
- Modify: `src/interfaces/terminal/action.rs`
- Modify: `src/interfaces/cli/repl.rs`

- [ ] **Step 1: Define terminal action contract**

Replace `src/interfaces/terminal/action.rs` with:

```rust
use crate::core::types::CallId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAction {
    SubmitPrompt(String),
    CancelRun,
    ApproveTool { call_id: CallId },
    RejectTool { call_id: CallId },
    SubmitInput { input_id: CallId, answer: String },
    ResumeLatest,
    ResumeRun(String),
    ShowStatus,
    ShowSessions,
    Clear,
    Help,
    Exit,
    Unknown(String),
}
```

Add tests:

```rust
#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;

    #[test]
    fn terminal_action_keeps_resume_target() {
        assert_eq!(
            TerminalAction::ResumeRun("01ABC".to_string()),
            TerminalAction::ResumeRun("01ABC".to_string())
        );
    }
}
```

- [ ] **Step 2: Run action tests**

Run:

```powershell
cargo test interfaces::terminal::action --lib
```

Expected: pass.

- [ ] **Step 3: Add slash-to-action conversion tests**

In `src/interfaces/cli/repl.rs`, extend the test module imports:

```rust
use crate::interfaces::terminal::action::TerminalAction;
```

Add this test:

```rust
#[test]
fn slash_commands_convert_to_terminal_actions() {
    assert_eq!(SlashCommand::parse("/help").to_action(), TerminalAction::Help);
    assert_eq!(SlashCommand::parse("/status").to_action(), TerminalAction::ShowStatus);
    assert_eq!(SlashCommand::parse("/clear").to_action(), TerminalAction::Clear);
    assert_eq!(SlashCommand::parse("/sessions").to_action(), TerminalAction::ShowSessions);
    assert_eq!(
        SlashCommand::parse("/resume latest").to_action(),
        TerminalAction::ResumeLatest
    );
    assert_eq!(
        SlashCommand::parse("/resume 01ARYZ6S41").to_action(),
        TerminalAction::ResumeRun("01ARYZ6S41".to_string())
    );
    assert_eq!(SlashCommand::parse("/exit").to_action(), TerminalAction::Exit);
    assert_eq!(
        SlashCommand::parse("/model gpt").to_action(),
        TerminalAction::Unknown("/model".to_string())
    );
}
```

Expected compile failure until `to_action` exists.

- [ ] **Step 4: Implement slash-to-action conversion**

In `src/interfaces/cli/repl.rs`, add:

```rust
use crate::interfaces::terminal::action::TerminalAction;
```

Add an impl below `impl SlashCommand`:

```rust
impl SlashCommand {
    pub fn to_action(&self) -> TerminalAction {
        match self {
            Self::Help => TerminalAction::Help,
            Self::Status => TerminalAction::ShowStatus,
            Self::Exit => TerminalAction::Exit,
            Self::Clear => TerminalAction::Clear,
            Self::Sessions => TerminalAction::ShowSessions,
            Self::ResumeLatest => TerminalAction::ResumeLatest,
            Self::ResumeRun(run_id) => TerminalAction::ResumeRun(run_id.clone()),
            Self::Unknown(command) => TerminalAction::Unknown(command.clone()),
        }
    }
}
```

Keep `handle_slash_command` behavior unchanged in this task. This contract is a bridge for future TUI key handling, not a behavior change.

- [ ] **Step 5: Run action and REPL parser tests**

Run:

```powershell
cargo test interfaces::terminal::action --lib
cargo test interfaces::cli::repl --lib
```

Expected: pass.

- [ ] **Step 6: Commit terminal actions**

Run:

```powershell
git add src/interfaces/terminal/action.rs src/interfaces/cli/repl.rs
git commit -m "refactor: define terminal action contract"
```

Expected: commit succeeds.

## Task 5: Refactor CLI Renderer To Use Terminal View Updates

**Files:**
- Modify: `src/interfaces/cli/render.rs`

- [ ] **Step 1: Add focused render coverage for previously ignored updates**

In `src/interfaces/cli/render.rs` tests, add tests that feed these events through `render_run_events` in `CliRunRenderMode::ReplCompact`:

```rust
StreamEvent::ModelStatus {
    status: "thinking".to_string(),
    message: "checking files".to_string(),
}
StreamEvent::ToolCallApprovalNeeded {
    call_id,
    name: "fs_write".to_string(),
    args: serde_json::json!({"path":"out.txt"}),
    reason: "writes a file".to_string(),
}
StreamEvent::InputNeeded {
    input_id: CallId::new(),
    prompt: "Which branch?".to_string(),
}
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
}
```

The test may assert only returned `TerminationReason::Final`; the important behavior is that the compact renderer handles every current update path without silently ignoring events needed by a future TUI.

- [ ] **Step 2: Import terminal view types**

In `src/interfaces/cli/render.rs`, add:

```rust
use crate::interfaces::terminal::view::{RunViewState, RunViewUpdate};
```

- [ ] **Step 3: Initialize terminal view state**

In `render_run_events`, replace direct ad hoc state as much as practical. Keep counters that are specific to line summary output.

Add before the loop:

```rust
let mut view_state = RunViewState::default();
```

Inside the loop, after recording the event:

```rust
let update = view_state.apply_event(&event);
```

Then match on `update` instead of matching directly on `event`.

- [ ] **Step 4: Render updates through small helper functions**

Add helpers near the bottom of `src/interfaces/cli/render.rs`:

```rust
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
                eprintln!("\nStatus · {status}");
                eprintln!("  {message}");
            }
            None
        }
        RunViewUpdate::ToolCallApprovalNeeded {
            name, args, reason, ..
        } => {
            if is_repl(options) {
                eprintln!("\nApproval · {name}");
                eprintln!("  {}", truncate(&args.to_string(), 200));
                eprintln!("  {reason}");
            }
            None
        }
        RunViewUpdate::InputNeeded { prompt, .. } => {
            if is_repl(options) {
                eprintln!("\nInput");
                eprintln!("  {prompt}");
            }
            None
        }
        RunViewUpdate::PromptCompacted { summary, .. } => {
            if is_repl(options) {
                eprintln!("\nContext · compacted");
                if let Some(summary) = summary {
                    eprintln!("  {}", truncate(&summary, 200));
                }
            }
            None
        }
        RunViewUpdate::RunCompleted { reason, output } => Some(render_completion(
            reason,
            output,
            options,
            render_state,
        )),
        other => {
            render_existing_update(other, options, render_state);
            None
        }
    }
}
```

Because `render_existing_update` and `render_completion` need access to counters and paths, define a focused state struct instead of passing many mutable variables:

```rust
struct ReplLineRenderState {
    plan_step_count: usize,
    tool_call_count: usize,
    tool_failure_count: usize,
    printed_plan: bool,
    printed_assistant: bool,
    assistant_at_line_start: bool,
    report_path: String,
    tool_names: std::collections::HashMap<CallId, String>,
}
```

If the helper split becomes too large for one step, keep the match inline but still use `RunViewUpdate` and `RunViewState`. The required boundary is event-to-view extraction, not a specific helper name.

- [ ] **Step 5: Preserve existing one-shot behavior**

Run:

```powershell
cargo run -- --model fake --approval never "hello renderer"
```

Expected stdout contains:

```text
fake response: hello renderer
```

Expected stderr may contain done metadata, as before. It must not print the REPL `You` block in one-shot mode.

- [ ] **Step 6: Run renderer tests**

Run:

```powershell
cargo test interfaces::cli::render --lib
```

Expected: pass.

- [ ] **Step 7: Commit renderer refactor**

Run:

```powershell
git add src/interfaces/cli/render.rs
git commit -m "refactor: render CLI output from terminal view updates"
```

Expected: commit succeeds.

## Task 6: Wire Compact REPL Parity And Documentation

**Files:**
- Modify: `tests/cli_repl.rs`
- Modify: `docs/runtime/implementation-guide.md`

- [ ] **Step 1: Add REPL smoke assertion for compact output stability**

In `tests/cli_repl.rs`, ensure the fake run smoke test checks stable semantic labels rather than exact spacing:

```rust
assert!(stderr.contains("You"));
assert!(stderr.contains("Done"));
assert!(stderr.contains("report"));
assert!(stdout.contains("fake response:"));
```

If such a test already exists, extend it only with the missing assertions.

- [ ] **Step 2: Run REPL smoke tests**

Run:

```powershell
cargo test --test cli_repl
```

Expected: pass.

- [ ] **Step 3: Document the TUI-ready boundary**

In `docs/runtime/implementation-guide.md`, add a short paragraph to the REPL/CLI section:

```markdown
The compact REPL is backed by a terminal view/action layer. `StreamEvent` values
are first projected into terminal view updates and accumulated into view state;
the current REPL renders those updates as line-oriented output. This keeps the
terminal surface ready for a future full TUI without adding full-screen terminal
dependencies or moving UI concerns into `core`.
```

Also document the non-goal:

```markdown
This pass does not add `ratatui`, `crossterm`, alternate-screen rendering,
mouse interaction, panels, or a full-screen session picker.
```

- [ ] **Step 4: Run docs hygiene**

Run:

```powershell
cargo test --test code_hygiene
```

Expected: pass.

- [ ] **Step 5: Commit REPL parity docs**

Run:

```powershell
git add tests/cli_repl.rs docs/runtime/implementation-guide.md
git commit -m "docs: describe TUI-ready terminal boundary"
```

Expected: commit succeeds.

## Task 7: Final Verification

**Files:**
- All files modified above.

- [ ] **Step 1: Check formatting**

Run:

```powershell
cargo fmt --all --check
```

Expected: exit 0.

- [ ] **Step 2: Run linting**

Run:

```powershell
cargo clippy --all-targets -- -D warnings
```

Expected: exit 0.

- [ ] **Step 3: Run focused tests**

Run:

```powershell
cargo test interfaces::terminal --lib
cargo test interfaces::cli::render --lib
cargo test interfaces::cli::repl --lib
cargo test --test cli_repl
cargo test --test code_hygiene
```

Expected: all pass.

- [ ] **Step 4: Run full Rust tests**

Run:

```powershell
cargo test
```

Expected: all pass. If this takes longer than the command timeout, rerun with a longer timeout before making a completion claim.

- [ ] **Step 5: Inspect changed files**

Run:

```powershell
git status --short
git log --oneline -5
```

Expected: only intentional changes remain uncommitted, or no changes remain if each task commit succeeded. The recent commits should include:

```text
refactor: add terminal surface module
refactor: map stream events to terminal updates
refactor: accumulate terminal run view state
refactor: define terminal action contract
refactor: render CLI output from terminal view updates
docs: describe TUI-ready terminal boundary
```

## Non-Goals For This Plan

- Do not add `ratatui`, `crossterm`, `termion`, or a full-screen TUI crate.
- Do not introduce alternate-screen rendering.
- Do not change the core engine event model unless exhaustive matching reveals a compile-time break that must be fixed.
- Do not change Web UI behavior.
- Do not add dynamic `/model`, `/provider`, or `/cwd` behavior.
- Do not implement concurrent prompt entry while a run is active.
- Do not remove the existing compact REPL output contract.

## Follow-Up Plan After This One

After this plan lands, the next feature can safely add a first full-screen TUI prototype behind a feature flag or separate subcommand, for example:

```powershell
rove tui
```

That later plan should choose the terminal crate, define layout panels, handle keybindings, and consume `RunViewState` plus `TerminalAction` instead of reading `StreamEvent` directly.
