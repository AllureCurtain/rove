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

    /// One unified product message was durably accepted for FIFO delivery.
    MessageQueued { id: String, content: String },
    /// The same queued identity won the CAS race for current-run delivery.
    MessageInterventionRequested { id: String },
    /// Harness included the intervention in a successfully-polled model turn.
    MessageAppliedCurrentRun { id: String },
    /// The terminal transition claimed this message for one successor turn.
    MessageClaimedSuccessor { id: String },
    /// Delivery stopped at an ambiguous or failed boundary and needs recovery.
    MessageNeedsAttention { id: String, reason: String },
    /// The still-eligible message was revoked through the durable authority.
    MessageRevoked { id: String },
}

/// One durable trace line's payload, split the way Codex splits
/// `ResponseItem` from `EventMsg`: model-visible history items are stored
/// explicitly so resume can rebuild model context without heuristics, while
/// every presentation/audit event stays in the existing [`StreamEvent`] shape.
///
/// Serde is untagged by design: [`HistoryItem`] serializes with a `kind` tag
/// and [`StreamEvent`] with a `type` tag, so both generations are
/// self-describing on disk and old envelope lines (Phase 1: bare events)
/// remain readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TraceEntry {
    /// A model-visible, replayable conversation item (Codex ResponseItem).
    History(rove_core::history::HistoryItem),
    /// A UI/audit event (Codex EventMsg). Wire format is unchanged.
    Ui(StreamEvent),
}

impl StreamEvent {
    /// Return the durable/public projection for a hard read-only Review run.
    ///
    /// Review tools intentionally return source text to the in-process model,
    /// but that text must not be copied into trace, resumable history, reports,
    /// or SSE/API event replay.  Keep identity, status, and bounded artifact
    /// references while replacing untrusted payloads with typed placeholders.
    pub fn redacted_for_review_persistence(&self) -> Self {
        match self {
            Self::RunStarted { run_id, job_id, .. } => Self::RunStarted {
                run_id: *run_id,
                job_id: *job_id,
                user_message: "[review request omitted]".to_string(),
            },
            Self::AgentProfileActivated {
                identity,
                resumed_from_snapshot,
                diagnostics,
            } => Self::AgentProfileActivated {
                identity: identity.clone(),
                resumed_from_snapshot: *resumed_from_snapshot,
                diagnostics: diagnostics
                    .iter()
                    .map(|diagnostic| AgentDiagnostic {
                        code: diagnostic.code.clone(),
                        subject: "[review diagnostic omitted]".to_string(),
                        message: "[review diagnostic omitted]".to_string(),
                    })
                    .collect(),
            },
            Self::InstructionOverlayApplied {
                scope,
                content_hash,
                boundary,
                call_id,
                ..
            } => Self::InstructionOverlayApplied {
                target_path: "[review path omitted]".to_string(),
                scope: scope.clone(),
                source_path: "[review path omitted]".to_string(),
                content_hash: content_hash.clone(),
                boundary: boundary.clone(),
                call_id: *call_id,
            },
            Self::ProceduresSelected {
                profile_hash,
                selected,
                considered_count,
                excluded_count,
            } => Self::ProceduresSelected {
                profile_hash: profile_hash.clone(),
                selected: selected
                    .iter()
                    .cloned()
                    .map(redact_procedure_reference)
                    .collect(),
                considered_count: *considered_count,
                excluded_count: *excluded_count,
            },
            Self::ProcedureHydrated {
                reference,
                truncated,
                dropped_bytes,
                step_id,
                hydration_hash,
            } => Self::ProcedureHydrated {
                reference: redact_procedure_reference(reference.clone()),
                truncated: *truncated,
                dropped_bytes: *dropped_bytes,
                step_id: step_id.clone(),
                hydration_hash: hydration_hash.clone(),
            },
            Self::ProcedureApplied { application } => Self::ProcedureApplied {
                application: Box::new(redact_procedure_application(application)),
            },
            Self::ProcedureDeviation {
                record_id,
                deviation,
            } => Self::ProcedureDeviation {
                record_id: record_id.clone(),
                deviation: Box::new(redact_procedure_deviation(deviation)),
            },
            Self::ModelStatus { status, .. } => Self::ModelStatus {
                status: status.clone(),
                message: "[review model status omitted]".to_string(),
            },
            Self::ExecutionDegraded { record } => {
                let mut record = record.clone();
                record.safe_summary = "[review degradation details omitted]".to_string();
                Self::ExecutionDegraded { record }
            }
            Self::LlmChunk { .. } => Self::LlmChunk {
                delta: "[review model output omitted]".to_string(),
            },
            Self::LlmMessage {
                usage,
                tool_calls,
                assistant_turn,
                ..
            } => {
                let mut assistant_turn = assistant_turn.as_deref().cloned().unwrap_or_default();
                assistant_turn.content = vec![rove_models::ContentBlock::text(
                    "[review model output omitted]",
                )];
                for call in &mut assistant_turn.tool_calls {
                    call.arguments = serde_json::json!({"redacted": true});
                }
                Self::LlmMessage {
                    full: "[review model output omitted]".to_string(),
                    usage: usage.clone(),
                    tool_calls: tool_calls
                        .iter()
                        .cloned()
                        .map(|mut call| {
                            call.args = serde_json::json!({"redacted": true});
                            call
                        })
                        .collect(),
                    assistant_turn: Some(Box::new(assistant_turn)),
                }
            }
            Self::ToolCallStarted {
                call_id,
                tool_use_id,
                name,
                ..
            } => Self::ToolCallStarted {
                call_id: *call_id,
                tool_use_id: tool_use_id.clone(),
                name: name.clone(),
                args: serde_json::json!({"redacted": true}),
            },
            Self::ToolCallApprovalNeeded {
                call_id,
                name,
                reason,
                ..
            } => Self::ToolCallApprovalNeeded {
                call_id: *call_id,
                name: name.clone(),
                args: serde_json::json!({"redacted": true}),
                reason: reason.clone(),
            },
            Self::ToolCallCompleted { call_id, result } => {
                let mut result = result.clone();
                result.output = "[review tool output omitted]".to_string();
                result.mutations.clear();
                if let Some(envelope) = result.envelope.as_mut() {
                    envelope.summary_text = "[review tool output omitted]".to_string();
                    envelope.content_blocks.clear();
                    envelope.structured_content = None;
                    envelope.mutations.clear();
                    envelope.external_effects.clear();
                    envelope.diagnostics.clear();
                }
                Self::ToolCallCompleted {
                    call_id: *call_id,
                    result,
                }
            }
            Self::ToolCallFailed {
                call_id, metadata, ..
            } => Self::ToolCallFailed {
                call_id: *call_id,
                error: ToolError::ExecutionFailed {
                    reason: "review tool failure details omitted".to_string(),
                },
                metadata: ToolExecutionMetadata {
                    affected_paths: Vec::new(),
                    diff_summary: Vec::new(),
                    ..metadata.clone()
                },
            },
            Self::InputNeeded { input_id, .. } => Self::InputNeeded {
                input_id: *input_id,
                prompt: "[review input request omitted]".to_string(),
            },
            Self::PlanCreated {
                plan,
                identity,
                plan_revision,
            } => Self::PlanCreated {
                plan: redact_task_plan(plan),
                identity: identity.clone(),
                plan_revision: plan_revision
                    .as_deref()
                    .map(redact_plan_revision)
                    .map(Box::new),
            },
            Self::PlanStepStarted {
                step,
                index,
                attempt,
                budget,
            } => Self::PlanStepStarted {
                step: redact_plan_step(step),
                index: *index,
                attempt: attempt.clone(),
                budget: budget.clone(),
            },
            Self::StepResult { record } => Self::StepResult {
                record: Box::new(redact_step_record(record)),
            },
            Self::PlanDecision { record } => Self::PlanDecision {
                record: Box::new(redact_plan_decision_record(record)),
            },
            Self::PlanRevised { plan, revision } => Self::PlanRevised {
                plan: redact_task_plan(plan),
                revision: Box::new(redact_plan_revision(revision)),
            },
            Self::FinalizationCompleted { record } => Self::FinalizationCompleted {
                record: Box::new(redact_finalization_record(record)),
            },
            Self::PromptCompacted { summary, state } => {
                let mut state = state.clone();
                state.last_error = state
                    .last_error
                    .as_ref()
                    .map(|_| "[review compaction details omitted]".to_string());
                Self::PromptCompacted {
                    summary: summary
                        .as_ref()
                        .map(|_| "[review compaction summary omitted]".to_string()),
                    state,
                }
            }
            Self::MemoryFlushed { notes } => Self::MemoryFlushed {
                notes: notes
                    .iter()
                    .map(|_| "[review memory note omitted]".to_string())
                    .collect(),
            },
            Self::RunCompleted { reason, output } => Self::RunCompleted {
                reason: reason.clone(),
                output: output
                    .as_ref()
                    .map(|_| "[review run output omitted]".to_string()),
            },
            Self::SteerAccepted { id, .. } => Self::SteerAccepted {
                id: id.clone(),
                content: "[review control content omitted]".to_string(),
            },
            Self::FollowupQueued { id, .. } => Self::FollowupQueued {
                id: id.clone(),
                content: "[review control content omitted]".to_string(),
            },
            Self::FollowupAbandoned { id, .. } => Self::FollowupAbandoned {
                id: id.clone(),
                reason: "[review control details omitted]".to_string(),
            },
            Self::MessageQueued { id, .. } => Self::MessageQueued {
                id: id.clone(),
                content: "[review control content omitted]".to_string(),
            },
            Self::MessageNeedsAttention { id, .. } => Self::MessageNeedsAttention {
                id: id.clone(),
                reason: "[review control details omitted]".to_string(),
            },
            _ => self.clone(),
        }
    }

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
            Self::MessageQueued { .. } => "message_queued",
            Self::MessageInterventionRequested { .. } => "message_intervention_requested",
            Self::MessageAppliedCurrentRun { .. } => "message_applied_current_run",
            Self::MessageClaimedSuccessor { .. } => "message_claimed_successor",
            Self::MessageNeedsAttention { .. } => "message_needs_attention",
            Self::MessageRevoked { .. } => "message_revoked",
        }
    }
}

fn redact_procedure_reference(mut reference: ProcedureReference) -> ProcedureReference {
    reference.source_path = "[review procedure source omitted]".to_string();
    reference
}

fn redact_procedure_application(application: &ProcedureApplication) -> ProcedureApplication {
    let mut application = application.clone();
    application.reference = redact_procedure_reference(application.reference);
    application.boundary = "[review procedure boundary omitted]".to_string();
    application
}

fn redact_procedure_deviation(deviation: &ProcedureDeviation) -> ProcedureDeviation {
    let mut deviation = deviation.clone();
    deviation.reference = redact_procedure_reference(deviation.reference);
    deviation.safe_summary = "[review procedure deviation omitted]".to_string();
    deviation.evidence_refs.clear();
    deviation
}

fn redact_plan_step(step: &PlanStep) -> PlanStep {
    PlanStep {
        id: step.id.clone(),
        title: "[review plan step omitted]".to_string(),
        done: step.done,
    }
}

fn redact_task_plan(plan: &TaskPlan) -> TaskPlan {
    TaskPlan {
        goal: "[review plan omitted]".to_string(),
        steps: plan.steps.iter().map(redact_plan_step).collect(),
        current_step: plan.current_step,
    }
}

fn redact_plan_revision(revision: &PlanRevision) -> PlanRevision {
    let mut revision = revision.clone();
    revision.remaining_steps = revision
        .remaining_steps
        .iter()
        .map(redact_plan_step)
        .collect();
    revision
}

fn redact_step_record(record: &StepRecord) -> StepRecord {
    let mut record = record.clone();
    record.summary = "[review step summary omitted]".to_string();
    record.safe_error_summary = record
        .safe_error_summary
        .as_ref()
        .map(|_| "[review step error omitted]".to_string());
    record.evidence_refs.clear();
    record.mutations.clear();
    record.procedure_applications = record
        .procedure_applications
        .iter()
        .map(redact_procedure_application)
        .collect();
    record.procedure_deviations = record
        .procedure_deviations
        .iter()
        .map(redact_procedure_deviation)
        .collect();
    if let Some(ambiguity) = record.ambiguity.as_mut() {
        ambiguity.safe_summary = "[review ambiguity omitted]".to_string();
        ambiguity.evidence_refs.clear();
    }
    record
}

fn redact_plan_decision_record(record: &PlanDecisionRecord) -> PlanDecisionRecord {
    let mut record = record.clone();
    record.decision.safe_summary = "[review plan decision omitted]".to_string();
    record.decision.remaining_work_requirements = record
        .decision
        .remaining_work_requirements
        .iter()
        .map(|_| "[review remaining work omitted]".to_string())
        .collect();
    record
}

fn redact_finalization_record(record: &FinalizationRecord) -> FinalizationRecord {
    let mut record = record.clone();
    record.output = record
        .output
        .as_ref()
        .map(|_| "[review finalization output omitted]".to_string());
    record.evidence_refs.clear();
    record
}

#[cfg(test)]
mod tests {
    use super::StreamEvent;
    use crate::execution::{
        ExecutionBudgetUsage, FinalOutcomeStatus, FinalizationMode, FinalizationPhase,
        FinalizationRecord,
    };
    use crate::types::{PlanStep, PromptCompactionState, TaskPlan};
    use rove_core::{CallId, ToolOutputEnvelope, ToolResult};

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

    #[test]
    fn review_persistence_projection_omits_tool_arguments_and_output() {
        let call_id = CallId::new();
        let secret = "token=do-not-persist";
        let started = StreamEvent::ToolCallStarted {
            call_id,
            tool_use_id: Some("wire-review".to_string()),
            name: "review_submit_findings".to_string(),
            args: serde_json::json!({"findings": [{"explanation": secret}]}),
        }
        .redacted_for_review_persistence();
        let completed = StreamEvent::ToolCallCompleted {
            call_id,
            result: ToolResult {
                call_id,
                output: secret.to_string(),
                mutations: Vec::new(),
                metadata: Default::default(),
                envelope: Some(Box::new(ToolOutputEnvelope::text(secret))),
            },
        }
        .redacted_for_review_persistence();

        let persisted = format!(
            "{}\n{}",
            serde_json::to_string(&started).unwrap(),
            serde_json::to_string(&completed).unwrap()
        );
        assert!(!persisted.contains(secret));
        assert!(persisted.contains("review tool output omitted"));
        assert!(persisted.contains("redacted"));
    }

    #[test]
    fn review_persistence_projection_omits_terminal_model_output() {
        let secret = "REVIEW_TERMINAL_SOURCE_SECRET";
        let event = StreamEvent::RunCompleted {
            reason: crate::types::TerminationReason::Final,
            output: Some(secret.to_string()),
        }
        .redacted_for_review_persistence();
        let persisted = serde_json::to_string(&event).unwrap();
        assert!(!persisted.contains(secret));
        assert!(persisted.contains("review run output omitted"));
    }

    #[test]
    fn review_persistence_projection_omits_lifecycle_text_payloads() {
        let secret = "REVIEW_LIFECYCLE_SOURCE_SECRET";
        let events = [
            StreamEvent::PlanCreated {
                plan: TaskPlan {
                    goal: secret.to_string(),
                    steps: vec![PlanStep {
                        id: "step-1".to_string(),
                        title: secret.to_string(),
                        done: false,
                    }],
                    current_step: 0,
                },
                identity: Default::default(),
                plan_revision: None,
            },
            StreamEvent::PromptCompacted {
                summary: Some(secret.to_string()),
                state: PromptCompactionState {
                    last_error: Some(secret.to_string()),
                    ..Default::default()
                },
            },
            StreamEvent::MemoryFlushed {
                notes: vec![secret.to_string()],
            },
            StreamEvent::FinalizationCompleted {
                record: Box::new(FinalizationRecord {
                    finalization_id: "finalization".to_string(),
                    phase: FinalizationPhase::Completed,
                    finish_reason: crate::execution::PlanFinishReason::Completed,
                    outcome: Some(FinalOutcomeStatus::Success),
                    mode: FinalizationMode::Direct,
                    started_at: "now".to_string(),
                    completed_at: Some("now".to_string()),
                    output: Some(secret.to_string()),
                    evidence_refs: vec![secret.to_string()],
                    incomplete_step_ids: Vec::new(),
                    budget_before: ExecutionBudgetUsage::default(),
                    budget_after: ExecutionBudgetUsage::default(),
                }),
            },
        ];
        let persisted = events
            .iter()
            .map(|event| serde_json::to_string(&event.redacted_for_review_persistence()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!persisted.contains(secret));
        assert!(persisted.contains("review finalization output omitted"));
    }
}
