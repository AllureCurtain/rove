use serde::{Deserialize, Serialize};

use crate::execution::{PlanDecisionRecord, PlanIdentity, PlanRevision, StepAttempt, StepRecord};
use crate::prompt_metadata::PromptBuildMetadata;
use crate::types::{JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason};
use rove_core::{CallId, ToolError, ToolExecutionMetadata, ToolResult};
use rove_models::{ToolCallRef, Usage};

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
        /// Full immutable initial revision. Older events omit this field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_revision: Option<Box<PlanRevision>>,
    },

    /// A persisted plan step is about to run.
    PlanStepStarted {
        step: PlanStep,
        index: usize,
        /// Stable identity of the in-flight attempt.
        #[serde(flatten)]
        attempt: StepAttempt,
    },

    /// Canonical terminal result for one planned step attempt.
    ///
    /// This is the append-only lifecycle fact used by resume, UI projection,
    /// and stream consumers. Compatibility `plan_step_completed` /
    /// `plan_step_failed` dual-fire events are intentionally not emitted.
    StepResult { record: Box<StepRecord> },

    /// Rule-first lifecycle decision for one terminal step record.
    PlanDecision { record: Box<PlanDecisionRecord> },

    /// Immutable replacement of only the remaining plan work.
    PlanRevised {
        plan: TaskPlan,
        revision: Box<PlanRevision>,
    },

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
            Self::StepResult { .. } => "step_result",
            Self::PlanDecision { .. } => "plan_decision",
            Self::PlanRevised { .. } => "plan_revised",
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
