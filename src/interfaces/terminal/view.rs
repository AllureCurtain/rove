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
