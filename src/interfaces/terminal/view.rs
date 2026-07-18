use crate::core::events::StreamEvent;
use crate::core::prompt_metadata::PromptBuildMetadata;
use crate::core::types::{
    CallId, JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason, ToolResult,
    Usage,
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
    MemoryFlushed {
        note_count: usize,
    },
    PromptBuilt {
        metadata: PromptBuildMetadata,
    },
    RunCompleted {
        reason: TerminationReason,
        output: Option<String>,
    },
}

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
    Interrupted,
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
                self.model_status = None;
            }
            RunViewUpdate::ToolCallStarted {
                call_id,
                name,
                args,
            } => {
                self.model_status = None;
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
                if let Some(approval) = self
                    .pending_approvals
                    .iter_mut()
                    .find(|approval| approval.call_id == call_id)
                {
                    approval.name = name.clone();
                    approval.args = args.clone();
                    approval.reason = reason;
                } else {
                    self.pending_approvals.push(PendingApprovalView {
                        call_id,
                        name: name.clone(),
                        args: args.clone(),
                        reason,
                    });
                }
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
                self.pending_inputs
                    .retain(|input| input.input_id != call_id);
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
                self.pending_inputs
                    .retain(|input| input.input_id != call_id);
            }
            RunViewUpdate::InputNeeded { input_id, prompt } => {
                if let Some(input) = self
                    .pending_inputs
                    .iter_mut()
                    .find(|input| input.input_id == input_id)
                {
                    input.prompt = prompt;
                } else {
                    self.pending_inputs
                        .push(PendingInputView { input_id, prompt });
                }
            }
            RunViewUpdate::PlanCreated { plan } => {
                self.plan = Some(plan);
            }
            RunViewUpdate::PlanStepStarted { step, index } => {
                if let Some(plan) = &mut self.plan {
                    plan.current_step = index.min(plan.steps.len());
                    if let Some(stored_step) = plan.steps.get_mut(index) {
                        *stored_step = step.clone();
                    }
                }
                self.model_status = None;
                self.current_step = Some((index, step));
            }
            RunViewUpdate::PlanStepCompleted { step, index } => {
                if let Some(plan) = &mut self.plan {
                    if let Some(stored_step) = plan.steps.get_mut(index) {
                        stored_step.id = step.id;
                        stored_step.title = step.title;
                        stored_step.done = true;
                    }
                    plan.current_step = plan
                        .steps
                        .iter()
                        .position(|candidate| !candidate.done)
                        .unwrap_or(plan.steps.len());
                }
                self.current_step = None;
            }
            RunViewUpdate::PlanStepFailed {
                step,
                index,
                reason,
            } => {
                self.failed_steps.push((index, step, reason));
                self.current_step = None;
            }
            RunViewUpdate::PromptCompacted { summary, state } => {
                self.prompt_compaction = Some((summary, state));
            }
            RunViewUpdate::MemoryFlushed { .. } => {}
            RunViewUpdate::PromptBuilt { .. } => {}
            RunViewUpdate::RunCompleted { reason, output } => {
                self.model_status = None;
                self.current_step = None;
                self.pending_approvals.clear();
                self.pending_inputs.clear();
                for tool in &mut self.tool_calls {
                    if matches!(
                        tool.status,
                        ToolCallStatus::Started | ToolCallStatus::WaitingApproval
                    ) {
                        tool.status = ToolCallStatus::Interrupted;
                    }
                }
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
            StreamEvent::ToolCallFailed { call_id, error, .. } => Self::ToolCallFailed {
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
            StreamEvent::MemoryFlushed { notes } => Self::MemoryFlushed {
                note_count: notes.len(),
            },
            StreamEvent::PromptBuilt { metadata } => Self::PromptBuilt {
                metadata: metadata.clone(),
            },
            StreamEvent::RunCompleted { reason, output } => Self::RunCompleted {
                reason: reason.clone(),
                output: output.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::events::StreamEvent;
    use crate::core::prompt_metadata::PromptBuildMetadata;
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
            cached_tokens: 0,
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
                    metadata: Default::default(),
                },
            },
            StreamEvent::ToolCallFailed {
                call_id,
                error: ToolError::ExecutionFailed {
                    reason: "boom".to_string(),
                },
                metadata: Default::default(),
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
            StreamEvent::MemoryFlushed {
                notes: vec!["tool result: created file src/memory/session.rs".to_string()],
            },
            StreamEvent::PromptBuilt {
                metadata: PromptBuildMetadata::default(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ok".to_string()),
            },
        ];

        let updates: Vec<RunViewUpdate> = events.iter().map(RunViewUpdate::from).collect();

        assert_eq!(updates.len(), 17);
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
            RunViewUpdate::MemoryFlushed { note_count: 1 }
        ));
        assert!(matches!(updates[15], RunViewUpdate::PromptBuilt { .. }));
        assert!(matches!(
            updates[16],
            RunViewUpdate::RunCompleted {
                reason: TerminationReason::Final,
                ..
            }
        ));
    }

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
        assert!(state.pending_approvals.is_empty());
        assert!(state.pending_inputs.is_empty());
        assert_eq!(
            state.completed.as_ref().unwrap().reason,
            TerminationReason::Final
        );
    }

    #[test]
    fn completed_plan_steps_update_the_stored_plan_and_cursor() {
        let mut state = super::RunViewState::default();
        let first = step();
        let second = PlanStep {
            id: "step-2".to_string(),
            title: "Write tests".to_string(),
            done: false,
        };
        state.apply_update(RunViewUpdate::PlanCreated {
            plan: TaskPlan {
                goal: "finish the TUI".to_string(),
                steps: vec![first.clone(), second],
                current_step: 0,
            },
        });
        state.apply_update(RunViewUpdate::PlanStepStarted {
            step: first.clone(),
            index: 0,
        });
        state.apply_update(RunViewUpdate::PlanStepCompleted {
            step: first,
            index: 0,
        });

        let plan = state.plan.as_ref().unwrap();
        assert!(plan.steps[0].done);
        assert_eq!(plan.current_step, 1);
        assert!(state.current_step.is_none());
    }

    #[test]
    fn pending_interactions_are_idempotent_and_input_clears_with_its_tool() {
        let call_id = CallId::new();
        let mut state = super::RunViewState::default();
        let approval = RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name: "fs_write".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        };
        let input = RunViewUpdate::InputNeeded {
            input_id: call_id,
            prompt: "Which branch?".to_string(),
        };

        state.apply_update(approval.clone());
        state.apply_update(approval);
        state.apply_update(input.clone());
        state.apply_update(input);

        assert_eq!(state.pending_approvals.len(), 1);
        assert_eq!(state.pending_inputs.len(), 1);

        state.apply_update(RunViewUpdate::ToolCallFailed {
            call_id,
            error: ToolError::ExecutionFailed {
                reason: "cancelled".to_string(),
            },
        });

        assert!(state.pending_approvals.is_empty());
        assert!(state.pending_inputs.is_empty());
    }

    #[test]
    fn llm_message_clears_transient_status_without_repeating_streamed_text() {
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::ModelStatus {
            status: "thinking".to_string(),
            message: "working".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "answer".to_string(),
        });
        state.apply_update(RunViewUpdate::LlmMessage {
            full: "answer".to_string(),
            usage: usage(),
            tool_call_count: 0,
        });

        assert_eq!(state.assistant_text, "answer");
        assert!(state.model_status.is_none());
    }
}
