use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::types::{CallId, PlanStep, ToolMutation, Usage};

/// Version of the typed execution policy introduced by the lifecycle
/// foundation. It is an in-memory policy version until persistence is added.
pub const EXECUTION_POLICY_VERSION: u32 = 1;

/// Compatibility ceiling for the first bounded planned-step runner.
///
/// The legacy runtime exposes only `max_steps`, which maps to the number of
/// planned step attempts. A separate, named ceiling keeps a single step from
/// consuming that entire run budget while the public multidimensional config
/// is still being introduced.
pub const DEFAULT_MAX_MODEL_TURNS_PER_STEP: u32 = 4;

/// Explicit runtime execution strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    React,
    PlanReact,
}

impl ExecutionStrategy {
    pub fn from_legacy(plan_enabled: bool) -> Self {
        if plan_enabled {
            Self::PlanReact
        } else {
            Self::React
        }
    }

    pub fn is_planned(self) -> bool {
        matches!(self, Self::PlanReact)
    }
}

/// Where the selected strategy came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategySelectionSource {
    Request,
    Session,
    Config,
    CompatibilityDefault,
    LegacyPlanEnabled,
}

/// Independent limits for the lifecycle phases and their observable work.
/// `None` means that this phase has not resolved that dimension yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionBudgetLimits {
    pub max_plan_steps: Option<u32>,
    pub max_step_attempts: Option<u32>,
    pub max_model_turns: Option<u32>,
    pub max_model_turns_per_step: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_tool_calls_per_step: Option<u32>,
    pub max_plan_revisions: Option<u32>,
    pub max_wall_time_ms: Option<u64>,
    pub max_total_tokens: Option<u64>,
    /// Cost enforcement is deferred until provider price metadata exists.
    pub max_cost_microunits: Option<u64>,
}

impl ExecutionBudgetLimits {
    /// Resolve the old single counter without assigning it to multiple units.
    pub fn from_legacy(max_steps: u32, strategy: ExecutionStrategy) -> Self {
        let mut limits = Self::default();
        match strategy {
            ExecutionStrategy::React => limits.max_model_turns = Some(max_steps),
            ExecutionStrategy::PlanReact => {
                limits.max_step_attempts = Some(max_steps);
                limits.max_model_turns_per_step = Some(DEFAULT_MAX_MODEL_TURNS_PER_STEP);
            }
        }
        limits
    }

    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        let limits = [
            ("max_plan_steps", self.max_plan_steps),
            ("max_step_attempts", self.max_step_attempts),
            ("max_model_turns", self.max_model_turns),
            ("max_model_turns_per_step", self.max_model_turns_per_step),
            ("max_tool_calls", self.max_tool_calls),
            ("max_tool_calls_per_step", self.max_tool_calls_per_step),
            ("max_plan_revisions", self.max_plan_revisions),
        ];
        for (field, value) in limits {
            if value == Some(0) {
                return Err(ExecutionValidationError::ZeroLimit { field });
            }
        }
        if let (Some(per_step), Some(global)) =
            (self.max_model_turns_per_step, self.max_model_turns)
            && per_step > global
        {
            return Err(ExecutionValidationError::PerStepLimitExceedsGlobal {
                per_step: "max_model_turns_per_step",
                global: "max_model_turns",
            });
        }
        if let (Some(per_step), Some(global)) = (self.max_tool_calls_per_step, self.max_tool_calls)
            && per_step > global
        {
            return Err(ExecutionValidationError::PerStepLimitExceedsGlobal {
                per_step: "max_tool_calls_per_step",
                global: "max_tool_calls",
            });
        }
        Ok(())
    }
}

/// Counters consumed by an execution policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionBudgetUsage {
    pub plan_steps: u32,
    pub step_attempts: u32,
    pub model_turns: u32,
    pub tool_calls: u32,
    pub plan_revisions: u32,
    pub wall_time_ms: u64,
    pub total_tokens: u64,
    pub cost_microunits: u64,
}

/// A resolved, inspectable execution policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub version: u32,
    pub strategy: ExecutionStrategy,
    pub selection_source: StrategySelectionSource,
    pub budgets: ExecutionBudgetLimits,
}

impl ExecutionPolicy {
    pub fn from_legacy(max_steps: u32, plan_enabled: bool) -> Self {
        let strategy = ExecutionStrategy::from_legacy(plan_enabled);
        Self {
            version: EXECUTION_POLICY_VERSION,
            strategy,
            selection_source: StrategySelectionSource::LegacyPlanEnabled,
            budgets: ExecutionBudgetLimits::from_legacy(max_steps, strategy),
        }
    }

    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        self.budgets.validate()
    }
}

/// Terminal outcome of one bounded step attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRecordStatus {
    Succeeded,
    Partial,
    Failed,
    Blocked,
    Skipped,
    BudgetExhausted,
    Cancelled,
    Interrupted,
}

/// Safe explanation for how a step was concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepCompletionBasis {
    ModelConclusion,
    DeterministicRule,
    UserDecision,
    RuntimeFailure,
}

/// Append-only facts for a terminal step attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub record_id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub step_id: String,
    pub attempt: u32,
    pub status: StepRecordStatus,
    pub started_at: String,
    pub finished_at: String,
    pub summary: String,
    pub completion_basis: StepCompletionBasis,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_call_ids: Vec<CallId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<ToolMutation>,
    pub model_turns_used: u32,
    pub tool_calls_used: u32,
    #[serde(default)]
    pub token_usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_error_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_record_id: Option<String>,
}

impl StepRecord {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        for (field, value) in [
            ("record_id", self.record_id.as_str()),
            ("plan_id", self.plan_id.as_str()),
            ("plan_revision_id", self.plan_revision_id.as_str()),
            ("step_id", self.step_id.as_str()),
            ("started_at", self.started_at.as_str()),
            ("finished_at", self.finished_at.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ExecutionValidationError::MissingStepRecordField { field });
            }
        }
        if self.attempt == 0 {
            return Err(ExecutionValidationError::ZeroAttempt);
        }
        Ok(())
    }
}

/// Stable identity for one logical plan and one immutable-compatible
/// revision.  The current runtime uses the identity to correlate the
/// append-only step ledger while retaining the legacy `TaskPlan` wire shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanIdentity {
    pub plan_id: String,
    pub plan_revision_id: String,
    pub revision: u32,
}

impl PlanIdentity {
    pub fn fresh() -> Self {
        Self {
            plan_id: ulid::Ulid::new().to_string(),
            plan_revision_id: ulid::Ulid::new().to_string(),
            revision: 0,
        }
    }

    pub fn next_revision(&self) -> Self {
        Self {
            plan_id: self.plan_id.clone(),
            plan_revision_id: ulid::Ulid::new().to_string(),
            revision: self.revision.saturating_add(1),
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.plan_id.trim().is_empty() && !self.plan_revision_id.trim().is_empty()
    }
}

/// Identity persisted while a planned step attempt is in flight.
///
/// It deliberately contains no model/tool output.  If a process disappears
/// after this projection is written, resume can close the attempt as
/// `interrupted` without replaying an unknown side effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepAttempt {
    pub plan_id: String,
    pub plan_revision_id: String,
    pub step_id: String,
    pub attempt: u32,
    pub started_at: String,
}

impl StepAttempt {
    pub fn is_complete(&self) -> bool {
        !self.plan_id.trim().is_empty()
            && !self.plan_revision_id.trim().is_empty()
            && !self.step_id.trim().is_empty()
            && self.attempt > 0
            && !self.started_at.trim().is_empty()
    }
}

/// Materialized projection of the append-only step ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepLedgerState {
    pub active_plan_id: Option<String>,
    pub active_plan_revision_id: Option<String>,
    pub active_plan_revision: u32,
    pub step_records: Vec<StepRecord>,
    pub active_step_attempt: Option<StepAttempt>,
    pub plan_lifecycle: PlanLifecycleState,
}

impl StepLedgerState {
    pub fn set_plan_identity(&mut self, identity: &PlanIdentity) {
        if !identity.is_complete() {
            return;
        }
        self.active_plan_id = Some(identity.plan_id.clone());
        self.active_plan_revision_id = Some(identity.plan_revision_id.clone());
        self.active_plan_revision = identity.revision;
    }

    pub fn plan_identity(&self) -> Option<PlanIdentity> {
        let identity = PlanIdentity {
            plan_id: self.active_plan_id.clone().unwrap_or_default(),
            plan_revision_id: self.active_plan_revision_id.clone().unwrap_or_default(),
            revision: self.active_plan_revision,
        };
        identity.is_complete().then_some(identity)
    }

    pub fn is_empty(&self) -> bool {
        self.active_plan_id.is_none()
            && self.active_plan_revision_id.is_none()
            && self.active_plan_revision == 0
            && self.step_records.is_empty()
            && self.active_step_attempt.is_none()
            && self.plan_lifecycle.is_empty()
    }

    pub fn checkpoint(&self) -> StepLedgerCheckpoint {
        StepLedgerCheckpoint {
            active_plan_id: self.active_plan_id.clone(),
            active_plan_revision_id: self.active_plan_revision_id.clone(),
            active_plan_revision: self.active_plan_revision,
            step_record_count: self.step_records.len(),
            active_step_attempt: self.active_step_attempt.clone(),
            plan_lifecycle: self.plan_lifecycle.checkpoint(),
        }
    }
}

/// Bounded ledger metadata copied into a prompt checkpoint.  Full records
/// remain in `TaskState` and the canonical trace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepLedgerCheckpoint {
    pub active_plan_id: Option<String>,
    pub active_plan_revision_id: Option<String>,
    pub active_plan_revision: u32,
    pub step_record_count: usize,
    pub active_step_attempt: Option<StepAttempt>,
    pub plan_lifecycle: PlanLifecycleCheckpoint,
}

/// An immutable revision of the remaining plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRevision {
    pub plan_id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    pub revision: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_step_record_id: Option<String>,
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_reason_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_step_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_remaining_step_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_steps: Vec<PlanStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    #[serde(default)]
    pub budget_snapshot: ExecutionBudgetUsage,
}

impl PlanRevision {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        for (field, value) in [
            ("plan_id", self.plan_id.as_str()),
            ("revision_id", self.revision_id.as_str()),
            ("created_at", self.created_at.as_str()),
            ("decision_id", self.decision_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ExecutionValidationError::MissingPlanRevisionField { field });
            }
        }
        if self.revision == 0 {
            if self.parent_revision_id.is_some() {
                return Err(ExecutionValidationError::InitialRevisionHasParent);
            }
        } else if self.parent_revision_id.is_none() {
            return Err(ExecutionValidationError::NonInitialRevisionMissingParent);
        }

        let mut step_ids = std::collections::HashSet::new();
        if self
            .remaining_steps
            .iter()
            .any(|step| !step_ids.insert(step.id.as_str()))
        {
            return Err(ExecutionValidationError::DuplicateRemainingStepId);
        }
        if self.remaining_steps.iter().any(|step| step.done) {
            return Err(ExecutionValidationError::CompletedStepInRemainingPlan);
        }

        let retained: std::collections::HashSet<_> =
            self.retained_step_ids.iter().map(String::as_str).collect();
        if self
            .superseded_remaining_step_ids
            .iter()
            .any(|step_id| retained.contains(step_id.as_str()))
        {
            return Err(ExecutionValidationError::RetainedAndSupersededOverlap);
        }
        Ok(())
    }

    pub fn identity(&self) -> PlanIdentity {
        PlanIdentity {
            plan_id: self.plan_id.clone(),
            plan_revision_id: self.revision_id.clone(),
            revision: self.revision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecisionKind {
    Continue,
    ReplaceRemaining,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanFinishReason {
    Completed,
    Partial,
    Blocked,
    BudgetExhausted,
    Failed,
    Cancelled,
    Interrupted,
}

/// Rule-first decision made after a terminal step record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDecision {
    pub decision_id: String,
    pub kind: PlanDecisionKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_reason_codes: Vec<String>,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_work_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<PlanFinishReason>,
}

impl PlanDecision {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.decision_id.trim().is_empty() {
            return Err(ExecutionValidationError::MissingDecisionId);
        }
        if self.safe_summary.trim().is_empty() {
            return Err(ExecutionValidationError::MissingDecisionSummary);
        }

        match self.kind {
            PlanDecisionKind::Continue => {
                if self.finish_reason.is_some() || !self.remaining_work_requirements.is_empty() {
                    return Err(ExecutionValidationError::InvalidDecisionShape {
                        kind: "continue",
                    });
                }
            }
            PlanDecisionKind::ReplaceRemaining => {
                if self.finish_reason.is_some() || self.remaining_work_requirements.is_empty() {
                    return Err(ExecutionValidationError::InvalidDecisionShape {
                        kind: "replace_remaining",
                    });
                }
            }
            PlanDecisionKind::Finish => {
                if self.finish_reason.is_none() || !self.remaining_work_requirements.is_empty() {
                    return Err(ExecutionValidationError::InvalidDecisionShape { kind: "finish" });
                }
            }
        }
        Ok(())
    }
}

/// Persisted correlation between one terminal step fact and its rule-first
/// plan decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDecisionRecord {
    pub trigger_step_record_id: String,
    pub decided_at: String,
    pub decision: PlanDecision,
}

impl PlanDecisionRecord {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        for (field, value) in [
            (
                "trigger_step_record_id",
                self.trigger_step_record_id.as_str(),
            ),
            ("decided_at", self.decided_at.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ExecutionValidationError::MissingPlanDecisionRecordField { field });
            }
        }
        self.decision.validate()
    }
}

/// Materialized projection of immutable plan revisions and the decisions that
/// connect terminal step records to lifecycle transitions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanLifecycleState {
    pub revisions: Vec<PlanRevision>,
    pub decisions: Vec<PlanDecisionRecord>,
}

impl PlanLifecycleState {
    pub fn is_empty(&self) -> bool {
        self.revisions.is_empty() && self.decisions.is_empty()
    }

    pub fn decision_for_record(&self, record_id: &str) -> Option<&PlanDecisionRecord> {
        self.decisions
            .iter()
            .rev()
            .find(|record| record.trigger_step_record_id == record_id)
    }

    pub fn revision_for_trigger(&self, record_id: &str) -> Option<&PlanRevision> {
        self.revisions
            .iter()
            .rev()
            .find(|revision| revision.trigger_step_record_id.as_deref() == Some(record_id))
    }

    pub fn push_decision(&mut self, record: PlanDecisionRecord) {
        if self.decisions.iter().all(|saved| {
            saved.decision.decision_id != record.decision.decision_id
                && saved.trigger_step_record_id != record.trigger_step_record_id
        }) {
            debug_assert!(record.validate().is_ok());
            self.decisions.push(record);
        }
    }

    pub fn push_revision(&mut self, revision: PlanRevision) {
        if self
            .revisions
            .iter()
            .all(|saved| saved.revision_id != revision.revision_id)
        {
            debug_assert!(revision.validate().is_ok());
            self.revisions.push(revision);
        }
    }

    pub fn checkpoint(&self) -> PlanLifecycleCheckpoint {
        PlanLifecycleCheckpoint {
            active_revision_id: self
                .revisions
                .last()
                .map(|revision| revision.revision_id.clone()),
            revision_count: self.revisions.len(),
            decision_count: self.decisions.len(),
        }
    }
}

/// Bounded plan lifecycle metadata copied into prompt checkpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanLifecycleCheckpoint {
    pub active_revision_id: Option<String>,
    pub revision_count: usize,
    pub decision_count: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecutionValidationError {
    #[error("{field} must be greater than zero")]
    ZeroLimit { field: &'static str },
    #[error("{per_step} cannot exceed {global}")]
    PerStepLimitExceedsGlobal {
        per_step: &'static str,
        global: &'static str,
    },
    #[error("step record field {field} must not be empty")]
    MissingStepRecordField { field: &'static str },
    #[error("step record attempt must be greater than zero")]
    ZeroAttempt,
    #[error("plan revision field {field} must not be empty")]
    MissingPlanRevisionField { field: &'static str },
    #[error("initial plan revision cannot have a parent")]
    InitialRevisionHasParent,
    #[error("non-initial plan revision must have a parent")]
    NonInitialRevisionMissingParent,
    #[error("remaining plan contains duplicate step IDs")]
    DuplicateRemainingStepId,
    #[error("remaining plan cannot contain completed steps")]
    CompletedStepInRemainingPlan,
    #[error("retained and superseded step IDs overlap")]
    RetainedAndSupersededOverlap,
    #[error("plan decision ID must not be empty")]
    MissingDecisionId,
    #[error("plan decision summary must not be empty")]
    MissingDecisionSummary,
    #[error("plan decision record field {field} must not be empty")]
    MissingPlanDecisionRecordField { field: &'static str },
    #[error("invalid {kind} plan decision shape")]
    InvalidDecisionShape { kind: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_limits_keep_react_and_plan_units_distinct() {
        let react = ExecutionPolicy::from_legacy(5, false);
        assert_eq!(react.strategy, ExecutionStrategy::React);
        assert_eq!(react.budgets.max_model_turns, Some(5));
        assert_eq!(react.budgets.max_step_attempts, None);

        let planned = ExecutionPolicy::from_legacy(5, true);
        assert_eq!(planned.strategy, ExecutionStrategy::PlanReact);
        assert_eq!(planned.budgets.max_step_attempts, Some(5));
        assert_eq!(planned.budgets.max_model_turns, None);
        assert_eq!(
            planned.budgets.max_model_turns_per_step,
            Some(DEFAULT_MAX_MODEL_TURNS_PER_STEP)
        );
    }

    #[test]
    fn strategy_and_decision_use_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&ExecutionStrategy::PlanReact).unwrap(),
            "\"plan_react\""
        );
        let decision = PlanDecision {
            decision_id: "d1".to_string(),
            kind: PlanDecisionKind::Finish,
            safe_reason_codes: vec!["completed".to_string()],
            safe_summary: "all required work is complete".to_string(),
            remaining_work_requirements: Vec::new(),
            finish_reason: Some(PlanFinishReason::Completed),
        };
        assert_eq!(
            serde_json::to_value(&decision).unwrap()["finish_reason"],
            "completed"
        );
        decision.validate().unwrap();
    }

    #[test]
    fn budget_validation_rejects_zero_and_inverted_limits() {
        let zero = ExecutionBudgetLimits {
            max_tool_calls: Some(0),
            ..ExecutionBudgetLimits::default()
        };
        assert!(matches!(
            zero.validate(),
            Err(ExecutionValidationError::ZeroLimit {
                field: "max_tool_calls"
            })
        ));

        let inverted = ExecutionBudgetLimits {
            max_model_turns: Some(2),
            max_model_turns_per_step: Some(3),
            ..ExecutionBudgetLimits::default()
        };
        assert!(matches!(
            inverted.validate(),
            Err(ExecutionValidationError::PerStepLimitExceedsGlobal {
                per_step: "max_model_turns_per_step",
                global: "max_model_turns"
            })
        ));
    }

    #[test]
    fn revision_validation_preserves_append_only_invariants() {
        let revision = PlanRevision {
            plan_id: "p1".to_string(),
            revision_id: "r1".to_string(),
            parent_revision_id: None,
            revision: 0,
            created_at: "2026-07-19T00:00:00Z".to_string(),
            trigger_step_record_id: None,
            decision_id: "d1".to_string(),
            safe_reason_codes: Vec::new(),
            retained_step_ids: vec!["1".to_string()],
            superseded_remaining_step_ids: vec!["2".to_string()],
            remaining_steps: vec![PlanStep {
                id: "2".to_string(),
                title: "remaining".to_string(),
                done: false,
            }],
            capability_snapshot_id: None,
            budget_snapshot: ExecutionBudgetUsage::default(),
        };
        revision.validate().unwrap();

        let mut invalid = revision;
        invalid.remaining_steps[0].done = true;
        assert_eq!(
            invalid.validate(),
            Err(ExecutionValidationError::CompletedStepInRemainingPlan)
        );
    }

    #[test]
    fn decision_validation_requires_shape_specific_fields() {
        let decision = PlanDecision {
            decision_id: "d1".to_string(),
            kind: PlanDecisionKind::ReplaceRemaining,
            safe_reason_codes: Vec::new(),
            safe_summary: "replace the remaining work".to_string(),
            remaining_work_requirements: Vec::new(),
            finish_reason: None,
        };
        assert_eq!(
            decision.validate(),
            Err(ExecutionValidationError::InvalidDecisionShape {
                kind: "replace_remaining"
            })
        );
    }
}
