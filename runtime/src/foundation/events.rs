use serde::{Deserialize, Serialize};

use crate::agents::procedure::ProcedureReference;
use crate::agents::{AgentDiagnostic, AgentProfileIdentity};
use crate::execution::{
    ExecutionBudgetSnapshot, ExecutionDegradation, ExecutionPhase, ExecutionPolicy,
    FinalizationRecord, PlanDecisionRecord, PlanIdentity, PlanRevision, ProcedureApplication,
    ProcedureDeviation, StepAttempt, StepRecord,
};
use crate::prompt_metadata::PromptBuildMetadata;
use crate::types::{JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason};
use rove_core::{CallId, ToolArtifactRef, ToolError, ToolExecutionMetadata, ToolResult};
use rove_models::{AssistantTurn, ToolCallRef, Usage};

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

    /// Immutable Agent profile selected before planner/model work begins.
    AgentProfileActivated {
        identity: Box<AgentProfileIdentity>,
        resumed_from_snapshot: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<AgentDiagnostic>,
    },

    /// Trusted workspace instruction bundle admitted for this run.
    WorkspaceInstructionsResolved {
        bundle_hash: String,
        layer_count: usize,
        rejected_count: usize,
        truncated: bool,
    },

    /// Resolved execution policy selected before planner/model work begins.
    ExecutionStrategySelected { policy: ExecutionPolicy },

    /// A nested workspace instruction became active for a concrete model-turn
    /// target. The body is intentionally absent from the event/trace.
    InstructionOverlayApplied {
        target_path: String,
        scope: String,
        source_path: String,
        content_hash: String,
        boundary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<CallId>,
    },

    /// Deterministic procedure selection, by identity only.
    ProceduresSelected {
        profile_hash: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        selected: Vec<ProcedureReference>,
        considered_count: usize,
        excluded_count: usize,
    },

    /// A selected procedure body was admitted into the bounded prompt prefix.
    ProcedureHydrated {
        reference: ProcedureReference,
        truncated: bool,
        dropped_bytes: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hydration_hash: Option<String>,
    },

    /// A bounded procedure projection was supplied to a concrete execution
    /// boundary. The body is never included; the application fact is enough
    /// for resume, diagnostics, and evidence correlation.
    ProcedureApplied {
        application: Box<ProcedureApplication>,
    },

    /// A step explicitly departed from selected procedure guidance. This is a
    /// fact for evaluation, not a permission or approval signal.
    ProcedureDeviation {
        record_id: String,
        deviation: Box<ProcedureDeviation>,
    },

    /// Monotonic multidimensional budget projection after a lifecycle phase or
    /// at an exhaustion boundary.
    ExecutionBudgetUpdated {
        phase: ExecutionPhase,
        /// Boxed to keep the canonical event small; the wire shape is unchanged.
        snapshot: Box<ExecutionBudgetSnapshot>,
    },

    /// Explicit fallback/degradation fact. Safe summaries never contain hidden
    /// reasoning or provider payloads.
    ExecutionDegraded { record: ExecutionDegradation },

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
        /// Canonical assistant turn. Older traces omit this additive field
        /// and are reconstructed through the legacy message projection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_turn: Option<Box<AssistantTurn>>,
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

    /// A tool payload became a durable artifact.
    ///
    /// Carries only the reference. The payload never travels on the event
    /// stream, so a large artifact cannot bloat a trace or an SSE frame.
    ToolArtifactStored {
        call_id: CallId,
        artifact: Box<ToolArtifactRef>,
    },

    /// A tool payload was refused by an artifact quota.
    ///
    /// Emitted so a quota event stays visible in the trace and in
    /// diagnostics even though no payload was retained.
    ToolArtifactRejected {
        call_id: CallId,
        /// Position of the originating content block.
        block_ordinal: u32,
        /// Stable machine-readable rejection code.
        reason: String,
        /// Bytes observed before the read was stopped.
        observed_bytes: u64,
    },

    /// A configured MCP server is unavailable or retained its last catalog
    /// after a failed refresh. Fields are bounded codes/identities only.
    McpServerDegraded {
        server_config_id: String,
        required: bool,
        failure_code: String,
    },

    /// A complete MCP catalog replaced the namespace used by future runs.
    McpCapabilitiesRefreshed {
        server_config_id: String,
        snapshot_id: String,
        added: Vec<String>,
        removed: Vec<String>,
        changed: Vec<String>,
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
        /// Boxed to keep the canonical event small; the wire shape is unchanged.
        #[serde(default)]
        budget: Box<ExecutionBudgetSnapshot>,
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

    /// Independent finalizer boundary. The started record contains no output.
    FinalizationStarted { record: Box<FinalizationRecord> },

    /// Finalizer terminal fact with a typed outcome and bounded output.
    FinalizationCompleted { record: Box<FinalizationRecord> },

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

    /// A steer message was accepted at a declared safe point and will shape
    /// the next model turn. Emitted at the safe point, not at submission.
    SteerAccepted {
        /// Stable steer id (matches the control record idempotency key).
        id: String,
        /// The steer text supplied by the user. Not secret; persisted to trace.
        content: String,
    },

    /// A previously accepted steer has been incorporated into a model request
    /// whose stream has been successfully polled. This is deliberately
    /// distinct from acceptance: a cancellation, token limit, or runtime
    /// failure can still intervene after the safe point and before a model
    /// turn begins.
    SteerApplied { id: String },

    /// A previously-queued steer was dropped without being applied, typically
    /// because the run reached a terminal state (cancel/error) before the
    /// next safe point.
    SteerDropped { id: String, reason: String },

    /// A follow-up message has been durably queued and will execute after the
    /// current assistant final. If the run ends with `Final`, the follow-up's
    /// content immediately starts the next turn; otherwise it remains in the
    /// durable queue for explicit confirmation.
    FollowupQueued { id: String, content: String },

    /// A queued follow-up has been dequeued and is about to drive a new turn
    /// (continuation of an existing job, or — on crash/restart — a freshly
    /// claimed product turn).
    FollowupDequeued { id: String },

    /// A queued follow-up was abandoned because the run ended with a
    /// non-final, potentially-indeterminate outcome. The control stays in
    /// the ProductStore with status `abandoned` until the user revokes or
    /// explicitly restarts.
    FollowupAbandoned { id: String, reason: String },
}

impl StreamEvent {
    /// Returns the event name string (for SSE `event:` field).
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run_started",
            Self::AgentProfileActivated { .. } => "agent_profile_activated",
            Self::WorkspaceInstructionsResolved { .. } => "workspace_instructions_resolved",
            Self::ExecutionStrategySelected { .. } => "execution_strategy_selected",
            Self::InstructionOverlayApplied { .. } => "instruction_overlay_applied",
            Self::ProceduresSelected { .. } => "procedures_selected",
            Self::ProcedureHydrated { .. } => "procedure_hydrated",
            Self::ProcedureApplied { .. } => "procedure_applied",
            Self::ProcedureDeviation { .. } => "procedure_deviation",
            Self::ExecutionBudgetUpdated { .. } => "execution_budget_updated",
            Self::ExecutionDegraded { .. } => "execution_degraded",
            Self::LlmChunk { .. } => "llm_chunk",
            Self::ModelStatus { .. } => "model_status",
            Self::LlmMessage { .. } => "llm_message",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallApprovalNeeded { .. } => "tool_call_approval_needed",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolCallFailed { .. } => "tool_call_failed",
            Self::ToolArtifactStored { .. } => "tool_artifact_stored",
            Self::ToolArtifactRejected { .. } => "tool_artifact_rejected",
            Self::McpServerDegraded { .. } => "mcp_server_degraded",
            Self::McpCapabilitiesRefreshed { .. } => "mcp_capabilities_refreshed",
            Self::InputNeeded { .. } => "input_needed",
            Self::PlanCreated { .. } => "plan_created",
            Self::PlanStepStarted { .. } => "plan_step_started",
            Self::StepResult { .. } => "step_result",
            Self::PlanDecision { .. } => "plan_decision",
            Self::PlanRevised { .. } => "plan_revised",
            Self::FinalizationStarted { .. } => "finalization_started",
            Self::FinalizationCompleted { .. } => "finalization_completed",
            Self::PromptCompacted { .. } => "prompt_compacted",
            Self::MemoryFlushed { .. } => "memory_flushed",
            Self::PromptBuilt { .. } => "prompt_built",
            Self::RunCompleted { .. } => "run_completed",
            Self::SteerAccepted { .. } => "steer_accepted",
            Self::SteerApplied { .. } => "steer_applied",
            Self::SteerDropped { .. } => "steer_dropped",
            Self::FollowupQueued { .. } => "followup_queued",
            Self::FollowupDequeued { .. } => "followup_dequeued",
            Self::FollowupAbandoned { .. } => "followup_abandoned",
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
