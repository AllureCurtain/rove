use serde::Serialize;

use super::types::{
    CallId, JobId, PlanStep, RunId, TaskPlan, TerminationReason, ToolResult, Usage,
};
use crate::errors::ToolError;

/// All events emitted by the engine's streaming main loop.
///
/// Consumers (CLI, API/SSE, Web) pattern-match on these to render output.
/// Adding a new variant forces all consumers to handle it (exhaustive match).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// A new run has started.
    RunStarted {
        run_id: RunId,
        job_id: JobId,
        user_message: String,
    },

    /// A chunk of streaming text from the LLM.
    LlmChunk { delta: String },

    /// The LLM finished producing a complete message.
    LlmMessage { full: String, usage: Usage },

    /// A tool call has been requested by the LLM.
    ToolCallStarted {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
    },

    /// A destructive tool call requires explicit approval before it may run.
    ToolCallApprovalNeeded {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
        reason: String,
    },

    /// A tool call completed successfully.
    ToolCallCompleted { call_id: CallId, result: ToolResult },

    /// A tool call failed.
    ToolCallFailed { call_id: CallId, error: ToolError },

    /// A plan has been drafted for this run.
    PlanCreated { plan: TaskPlan },

    /// A persisted plan step is about to run.
    PlanStepStarted { step: PlanStep, index: usize },

    /// A persisted plan step completed.
    PlanStepCompleted { step: PlanStep, index: usize },

    /// A persisted plan step failed but the run may continue or retry.
    PlanStepFailed {
        step: PlanStep,
        index: usize,
        reason: String,
    },

    /// The run has completed.
    RunCompleted {
        reason: TerminationReason,
        output: Option<String>,
    },
}

impl StreamEvent {
    /// Returns the event name string (for SSE `event:` field).
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run_started",
            Self::LlmChunk { .. } => "llm_chunk",
            Self::LlmMessage { .. } => "llm_message",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallApprovalNeeded { .. } => "tool_call_approval_needed",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolCallFailed { .. } => "tool_call_failed",
            Self::PlanCreated { .. } => "plan_created",
            Self::PlanStepStarted { .. } => "plan_step_started",
            Self::PlanStepCompleted { .. } => "plan_step_completed",
            Self::PlanStepFailed { .. } => "plan_step_failed",
            Self::RunCompleted { .. } => "run_completed",
        }
    }
}
