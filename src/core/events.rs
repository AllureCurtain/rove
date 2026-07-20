use serde::{Deserialize, Serialize};

use super::execution::{PlanIdentity, StepAttempt, StepRecord};
use super::types::{
    CallId, JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason,
    ToolCallRef, ToolExecutionMetadata, ToolResult, Usage,
};
use crate::core::prompt_metadata::PromptBuildMetadata;
use crate::errors::ToolError;

/// All events emitted by the engine's streaming main loop.
///
/// Consumers (CLI, API/SSE, Web) pattern-match on these to render output.
/// Adding a new variant forces all consumers to handle it (exhaustive match).
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// A safe runtime progress note. This must not contain hidden reasoning text.
    ModelStatus { status: String, message: String },

    /// The LLM finished producing a complete message.
    LlmMessage {
        full: String,
        usage: Usage,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCallRef>,
    },

    /// A tool call has been requested by the LLM.
    ToolCallStarted {
        call_id: CallId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
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
    ToolCallFailed {
        call_id: CallId,
        error: ToolError,
        #[serde(default)]
        metadata: ToolExecutionMetadata,
    },

    /// The `request_input` tool is waiting for user input.
    InputNeeded { input_id: CallId, prompt: String },

    /// A plan has been drafted for this run.
    PlanCreated {
        plan: TaskPlan,
        /// Stable identity for the logical plan/revision. Flattening keeps
        /// the wire shape readable and lets older events deserialize with
        /// default metadata.
        #[serde(flatten)]
        identity: PlanIdentity,
    },

    /// A persisted plan step is about to run.
    PlanStepStarted {
        step: PlanStep,
        index: usize,
        /// Stable identity of the in-flight attempt.
        #[serde(flatten)]
        attempt: StepAttempt,
    },

    /// A persisted plan step completed.
    PlanStepCompleted { step: PlanStep, index: usize },

    /// A persisted plan step failed but the run may continue or retry.
    PlanStepFailed {
        step: PlanStep,
        index: usize,
        reason: String,
    },

    /// Canonical terminal result for one planned step attempt.
    ///
    /// This is the append-only lifecycle fact.  The legacy plan-step
    /// completed/failed events remain derived compatibility notifications.
    StepResult { record: Box<StepRecord> },

    /// Prompt history was compacted for future resume.
    PromptCompacted {
        summary: Option<String>,
        state: PromptCompactionState,
    },

    /// Durable-worthy notes were flushed from the soon-to-be-compacted history
    /// to session memory before compaction collapsed the detail away.
    MemoryFlushed { notes: Vec<String> },

    /// Prompt context has been assembled for a model turn.
    PromptBuilt { metadata: PromptBuildMetadata },

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
            Self::ModelStatus { .. } => "model_status",
            Self::LlmMessage { .. } => "llm_message",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallApprovalNeeded { .. } => "tool_call_approval_needed",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolCallFailed { .. } => "tool_call_failed",
            Self::InputNeeded { .. } => "input_needed",
            Self::PlanCreated { .. } => "plan_created",
            Self::PlanStepStarted { .. } => "plan_step_started",
            Self::PlanStepCompleted { .. } => "plan_step_completed",
            Self::PlanStepFailed { .. } => "plan_step_failed",
            Self::StepResult { .. } => "step_result",
            Self::PromptCompacted { .. } => "prompt_compacted",
            Self::MemoryFlushed { .. } => "memory_flushed",
            Self::PromptBuilt { .. } => "prompt_built",
            Self::RunCompleted { .. } => "run_completed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StreamEvent;

    #[test]
    fn legacy_plan_events_without_lifecycle_identity_still_deserialize() {
        let created: StreamEvent = serde_json::from_value(serde_json::json!({
            "type": "plan_created",
            "plan": {
                "goal": "legacy",
                "steps": [{"id": "1", "title": "inspect", "done": false}],
                "current_step": 0
            }
        }))
        .unwrap();
        let started: StreamEvent = serde_json::from_value(serde_json::json!({
            "type": "plan_step_started",
            "step": {"id": "1", "title": "inspect", "done": false},
            "index": 0
        }))
        .unwrap();

        assert!(matches!(
            created,
            StreamEvent::PlanCreated { identity, .. } if !identity.is_complete()
        ));
        assert!(matches!(
            started,
            StreamEvent::PlanStepStarted { attempt, .. } if !attempt.is_complete()
        ));
    }
}
