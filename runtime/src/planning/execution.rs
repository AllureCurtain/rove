use serde::{Deserialize, Serialize};
use std::time::Instant;
use thiserror::Error;

use crate::agents::procedure::{ProcedureReference, RiskLevel, SideEffect};
use crate::capability::CapabilityMutationClass;
use crate::types::PlanStep;
use rove_core::{CallId, ToolMutation};
use rove_models::Usage;

/// Version of the typed execution policy introduced by the lifecycle
/// foundation. It is an in-memory policy version until persistence is added.
pub const EXECUTION_POLICY_VERSION: u32 = 1;

/// Compatibility ceiling for the first bounded planned-step runner.
///
/// The public surface still exposes sugar `max_steps`, which maps to the number
/// of planned step attempts under PlanReact. A separate, named ceiling keeps a
/// single step from consuming that entire run budget while the multidimensional
/// config is still being introduced.
pub const DEFAULT_MAX_MODEL_TURNS_PER_STEP: u32 = 4;

/// Default repair ceiling for malformed structured lifecycle model output.
pub const DEFAULT_MAX_MODEL_REPAIRS: u32 = 1;

/// A planned run reserves one model turn for final synthesis when model
/// finalization is enabled. Compatibility policies use deterministic
/// finalization and therefore reserve no turn.
pub const DEFAULT_MAX_FINALIZATION_TURNS: u32 = 1;

pub fn planned_step_failure_message(step_title: &str, reason: &str) -> String {
    format!("Planned step failed: {step_title}. Reason: {reason}. Re-plan the remaining work.")
}

/// Explicit runtime execution strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    React,
    PlanReact,
}

impl ExecutionStrategy {
    /// Map the sugar `plan_enabled` flag into the resolved strategy.
    pub fn from_plan_enabled(plan_enabled: bool) -> Self {
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategySelectionSource {
    Request,
    Session,
    Config,
    /// Older records and unset callers deserialize to this value.
    #[default]
    CompatibilityDefault,
    /// Derived from the sugar `plan_enabled` / `max_steps` fields.
    MaxStepsAndPlanFlag,
}

/// Whether semantic plan ambiguity may invoke the model evaluator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorMode {
    /// Deterministic decision table only. This is retained for legacy embedded
    /// policies and tests that intentionally avoid additional model calls.
    RuleOnly,
    /// Deterministic rules first; a model call is allowed only when a terminal
    /// record carries a validated [`PlanAmbiguity`].
    #[default]
    RuleFirstModelOnAmbiguity,
}

/// How the independent run finalizer is allowed to synthesize the user answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizerPolicy {
    /// Use a deterministic evidence-grounded answer. React finals remain
    /// direct because the model already produced the user-facing answer.
    #[default]
    Deterministic,
    /// Prefer a bounded model synthesis and fall back deterministically on any
    /// error, invalid result, cancellation, or budget boundary.
    ModelPreferred,
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
    pub max_model_repairs: Option<u32>,
    pub max_finalization_turns: Option<u32>,
    pub max_wall_time_ms: Option<u64>,
    pub max_total_tokens: Option<u64>,
    /// Cost enforcement is deferred until provider price metadata exists.
    pub max_cost_microunits: Option<u64>,
}

impl ExecutionBudgetLimits {
    /// Project the sugar `max_steps` counter into strategy-specific budget units.
    ///
    /// React uses model turns; PlanReact uses planned step attempts and keeps the
    /// independent per-step model-turn ceiling. The single counter is never
    /// assigned to multiple units at once.
    pub fn from_max_steps(max_steps: u32, strategy: ExecutionStrategy) -> Self {
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
            ("max_model_repairs", self.max_model_repairs),
            ("max_finalization_turns", self.max_finalization_turns),
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
        if let (Some(finalization), Some(global)) =
            (self.max_finalization_turns, self.max_model_turns)
            && finalization > global
        {
            return Err(ExecutionValidationError::PerStepLimitExceedsGlobal {
                per_step: "max_finalization_turns",
                global: "max_model_turns",
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
    pub model_repairs: u32,
    pub planner_turns: u32,
    pub evaluator_turns: u32,
    pub replanner_turns: u32,
    pub finalization_turns: u32,
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
    #[serde(default)]
    pub evaluator_mode: EvaluatorMode,
    #[serde(default)]
    pub finalizer_policy: FinalizerPolicy,
}

impl ExecutionPolicy {
    /// Build a resolved policy from the sugar `max_steps` / `plan_enabled` fields.
    ///
    /// Callers that still expose those fields write into `ExecutionPolicy` here;
    /// the policy remains the sole execution-config truth.
    pub fn from_max_steps_and_plan_flag(max_steps: u32, plan_enabled: bool) -> Self {
        let strategy = ExecutionStrategy::from_plan_enabled(plan_enabled);
        Self {
            version: EXECUTION_POLICY_VERSION,
            strategy,
            selection_source: StrategySelectionSource::MaxStepsAndPlanFlag,
            budgets: ExecutionBudgetLimits::from_max_steps(max_steps, strategy),
            evaluator_mode: EvaluatorMode::RuleOnly,
            finalizer_policy: FinalizerPolicy::Deterministic,
        }
    }

    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        self.budgets.validate()
    }
}

/// Lifecycle phases used in budget and degradation facts. These values are
/// safe diagnostics; they do not contain model reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Planner,
    Step,
    Evaluator,
    Replanner,
    Finalizer,
    Run,
}

/// A specific exhausted execution dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBudgetDimension {
    PlanSteps,
    StepAttempts,
    ModelTurns,
    ModelTurnsPerStep,
    ToolCalls,
    ToolCallsPerStep,
    PlanRevisions,
    ModelRepairs,
    FinalizationTurns,
    WallTime,
    TotalTokens,
    Cost,
}

/// Typed budget refusal emitted before new work begins, or immediately after
/// an indivisible model response crosses a token/cost boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBudgetExhaustion {
    pub dimension: ExecutionBudgetDimension,
    pub phase: ExecutionPhase,
    pub limit: u64,
    pub consumed: u64,
    pub safe_summary: String,
}

/// Public resolved budget snapshot used by events, checkpoints, state, API,
/// reports, and UIs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionBudgetSnapshot {
    pub limits: ExecutionBudgetLimits,
    pub consumed: ExecutionBudgetUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exhausted: Option<ExecutionBudgetExhaustion>,
    /// Cost is enforceable only when priced usage has been supplied. Generic
    /// provider clients expose zero here rather than claiming enforcement.
    pub cost_enforced: bool,
}

/// In-memory accounting authority for one run. Persisted callers store the
/// serializable snapshot, then reconstruct the tracker with consumed usage on
/// explicit resume.
pub struct ExecutionBudgetTracker {
    limits: ExecutionBudgetLimits,
    usage: ExecutionBudgetUsage,
    started: Instant,
    exhausted: Option<ExecutionBudgetExhaustion>,
    cost_enforced: bool,
}

impl ExecutionBudgetTracker {
    pub fn new(
        limits: ExecutionBudgetLimits,
        usage: ExecutionBudgetUsage,
        cost_enforced: bool,
    ) -> Self {
        Self {
            limits,
            usage,
            started: Instant::now(),
            exhausted: None,
            cost_enforced,
        }
    }

    pub fn usage(&self) -> &ExecutionBudgetUsage {
        &self.usage
    }

    pub fn limits(&self) -> &ExecutionBudgetLimits {
        &self.limits
    }

    pub fn snapshot(&self) -> ExecutionBudgetSnapshot {
        ExecutionBudgetSnapshot {
            limits: self.limits.clone(),
            consumed: self.usage.clone(),
            exhausted: self.exhausted.clone(),
            cost_enforced: self.cost_enforced,
        }
    }

    pub fn refresh_wall_time(
        &mut self,
        phase: ExecutionPhase,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        self.usage.wall_time_ms =
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.check_limit(
            ExecutionBudgetDimension::WallTime,
            phase,
            self.limits.max_wall_time_ms,
            self.usage.wall_time_ms,
        )
    }

    pub fn validate_plan_steps(
        &mut self,
        steps: usize,
        phase: ExecutionPhase,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        let steps = u32::try_from(steps).unwrap_or(u32::MAX);
        self.check_limit(
            ExecutionBudgetDimension::PlanSteps,
            phase,
            self.limits.max_plan_steps.map(u64::from),
            u64::from(steps),
        )?;
        self.usage.plan_steps = steps;
        Ok(())
    }

    pub fn reserve_step_attempt(&mut self) -> Result<(), ExecutionBudgetExhaustion> {
        let next = self.usage.step_attempts.saturating_add(1);
        self.check_limit(
            ExecutionBudgetDimension::StepAttempts,
            ExecutionPhase::Step,
            self.limits.max_step_attempts.map(u64::from),
            u64::from(next),
        )?;
        self.usage.step_attempts = next;
        Ok(())
    }

    pub fn reserve_plan_revision(&mut self) -> Result<(), ExecutionBudgetExhaustion> {
        let next = self.usage.plan_revisions.saturating_add(1);
        self.check_limit(
            ExecutionBudgetDimension::PlanRevisions,
            ExecutionPhase::Replanner,
            self.limits.max_plan_revisions.map(u64::from),
            u64::from(next),
        )?;
        self.usage.plan_revisions = next;
        Ok(())
    }

    pub fn reserve_repair(
        &mut self,
        phase: ExecutionPhase,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        let next = self.usage.model_repairs.saturating_add(1);
        self.check_limit(
            ExecutionBudgetDimension::ModelRepairs,
            phase,
            self.limits.max_model_repairs.map(u64::from),
            u64::from(next),
        )?;
        self.usage.model_repairs = next;
        Ok(())
    }

    pub fn reserve_model_turn(
        &mut self,
        phase: ExecutionPhase,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        self.refresh_wall_time(phase)?;
        let next = self.usage.model_turns.saturating_add(1);
        let effective_limit = if phase == ExecutionPhase::Finalizer {
            self.limits.max_model_turns
        } else {
            self.limits.max_model_turns.map(|limit| {
                limit.saturating_sub(self.limits.max_finalization_turns.unwrap_or_default())
            })
        };
        self.check_limit(
            ExecutionBudgetDimension::ModelTurns,
            phase,
            effective_limit.map(u64::from),
            u64::from(next),
        )?;
        if phase == ExecutionPhase::Finalizer {
            let finalization = self.usage.finalization_turns.saturating_add(1);
            self.check_limit(
                ExecutionBudgetDimension::FinalizationTurns,
                phase,
                self.limits.max_finalization_turns.map(u64::from),
                u64::from(finalization),
            )?;
            self.usage.finalization_turns = finalization;
        }
        self.usage.model_turns = next;
        match phase {
            ExecutionPhase::Planner => {
                self.usage.planner_turns = self.usage.planner_turns.saturating_add(1)
            }
            ExecutionPhase::Evaluator => {
                self.usage.evaluator_turns = self.usage.evaluator_turns.saturating_add(1)
            }
            ExecutionPhase::Replanner => {
                self.usage.replanner_turns = self.usage.replanner_turns.saturating_add(1)
            }
            ExecutionPhase::Finalizer | ExecutionPhase::Step | ExecutionPhase::Run => {}
        }
        Ok(())
    }

    pub fn remaining_model_turns_for_step(&mut self) -> Result<u32, ExecutionBudgetExhaustion> {
        self.refresh_wall_time(ExecutionPhase::Step)?;
        let per_step = self
            .limits
            .max_model_turns_per_step
            .unwrap_or(DEFAULT_MAX_MODEL_TURNS_PER_STEP);
        let global = self
            .limits
            .max_model_turns
            .map(|limit| {
                limit
                    .saturating_sub(self.usage.model_turns)
                    .saturating_sub(self.limits.max_finalization_turns.unwrap_or_default())
            })
            .unwrap_or(u32::MAX);
        let remaining = per_step.min(global);
        if remaining == 0 {
            return Err(self.exhaustion(
                ExecutionBudgetDimension::ModelTurns,
                ExecutionPhase::Step,
                self.limits.max_model_turns.unwrap_or_default().into(),
                self.usage.model_turns.into(),
            ));
        }
        Ok(remaining)
    }

    pub fn remaining_tool_calls_for_step(&self) -> Result<u32, ExecutionBudgetExhaustion> {
        let per_step = self.limits.max_tool_calls_per_step.unwrap_or(u32::MAX);
        let global = self
            .limits
            .max_tool_calls
            .map(|limit| limit.saturating_sub(self.usage.tool_calls))
            .unwrap_or(u32::MAX);
        let remaining = per_step.min(global);
        if remaining == 0 {
            return Err(ExecutionBudgetExhaustion {
                dimension: ExecutionBudgetDimension::ToolCalls,
                phase: ExecutionPhase::Step,
                limit: self.limits.max_tool_calls.unwrap_or_default().into(),
                consumed: self.usage.tool_calls.into(),
                safe_summary: "The configured tool-call budget is exhausted.".to_string(),
            });
        }
        Ok(remaining)
    }

    pub fn remaining_tokens(&self) -> Option<u64> {
        self.limits
            .max_total_tokens
            .map(|limit| limit.saturating_sub(self.usage.total_tokens))
    }

    pub fn record_step_usage(
        &mut self,
        model_turns: u32,
        tool_calls: u32,
        repairs: u32,
        usage: &rove_models::Usage,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        self.usage.model_turns = self.usage.model_turns.saturating_add(model_turns);
        self.usage.tool_calls = self.usage.tool_calls.saturating_add(tool_calls);
        self.usage.model_repairs = self.usage.model_repairs.saturating_add(repairs);
        self.record_tokens(usage, ExecutionPhase::Step)?;
        self.check_limit(
            ExecutionBudgetDimension::ModelRepairs,
            ExecutionPhase::Step,
            self.limits.max_model_repairs.map(u64::from),
            self.usage.model_repairs.into(),
        )
    }

    pub fn record_tokens(
        &mut self,
        usage: &rove_models::Usage,
        phase: ExecutionPhase,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        self.usage.total_tokens = self
            .usage
            .total_tokens
            .saturating_add(u64::from(usage.total_tokens));
        self.check_limit(
            ExecutionBudgetDimension::TotalTokens,
            phase,
            self.limits.max_total_tokens,
            self.usage.total_tokens,
        )
    }

    pub fn mark_exhausted(&mut self, exhaustion: ExecutionBudgetExhaustion) {
        self.exhausted = Some(exhaustion);
    }

    fn check_limit(
        &mut self,
        dimension: ExecutionBudgetDimension,
        phase: ExecutionPhase,
        limit: Option<u64>,
        consumed: u64,
    ) -> Result<(), ExecutionBudgetExhaustion> {
        if let Some(limit) = limit
            && consumed > limit
        {
            let exhaustion = self.exhaustion(dimension, phase, limit, consumed);
            return Err(exhaustion);
        }
        Ok(())
    }

    fn exhaustion(
        &mut self,
        dimension: ExecutionBudgetDimension,
        phase: ExecutionPhase,
        limit: u64,
        consumed: u64,
    ) -> ExecutionBudgetExhaustion {
        let exhaustion = ExecutionBudgetExhaustion {
            dimension,
            phase,
            limit,
            consumed,
            safe_summary: format!(
                "The {dimension:?} execution budget is exhausted (limit {limit}, consumed {consumed})."
            ),
        };
        self.exhausted = Some(exhaustion.clone());
        exhaustion
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
    Rejected,
    Skipped,
    BudgetExhausted,
    Cancelled,
    Interrupted,
    Indeterminate,
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

/// One concrete tool binding visible to a procedure at a run/step boundary.
/// A binding describes availability and policy; it never grants permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureCapabilityBinding {
    pub capability_id: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_class: Option<CapabilityMutationClass>,
    #[serde(default)]
    pub approval_required: bool,
}

/// Bounded procedure material supplied to one execution boundary.
///
/// `hydration_hash` identifies the exact section projection admitted to the
/// model. The source body remains pinned by `reference.content_hash` and the
/// Agent profile snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureApplication {
    pub application_id: String,
    pub reference: ProcedureReference,
    pub hydration_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_ids: Vec<String>,
    pub capability_snapshot_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_bindings: Vec<ProcedureCapabilityBinding>,
    pub risk_level: RiskLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub side_effects: Vec<SideEffect>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub boundary: String,
}

impl ProcedureApplication {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        for (field, value) in [
            ("application_id", self.application_id.as_str()),
            ("hydration_hash", self.hydration_hash.as_str()),
            (
                "capability_snapshot_id",
                self.capability_snapshot_id.as_str(),
            ),
            ("boundary", self.boundary.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ExecutionValidationError::MissingProcedureApplicationField { field });
            }
        }
        if self.section_ids.len() > 64
            || self
                .section_ids
                .iter()
                .any(|section| section.trim().is_empty() || section.chars().count() > 160)
        {
            return Err(ExecutionValidationError::InvalidProcedureSections);
        }
        Ok(())
    }
}

/// Typed reason why execution departed from selected procedure guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureDeviationReason {
    EvidenceContradiction,
    CapabilityUnavailable,
    PreconditionsUnsatisfied,
    UserConstraint,
    ProcedureStale,
    SaferAlternative,
    RuntimeFailure,
}

/// Safe, persisted deviation fact. It can inform evaluation, but cannot
/// weaken approval, schema, workspace, or capability enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureDeviation {
    pub deviation_id: String,
    pub reference: ProcedureReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    pub reason: ProcedureDeviationReason,
    pub safe_summary: String,
    #[serde(default)]
    pub material: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
}

impl ProcedureDeviation {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.deviation_id.trim().is_empty() {
            return Err(ExecutionValidationError::MissingProcedureDeviationId);
        }
        if self.safe_summary.trim().is_empty() || self.safe_summary.chars().count() > 500 {
            return Err(ExecutionValidationError::InvalidProcedureDeviationSummary);
        }
        if self.evidence_refs.len() > 32
            || self
                .evidence_refs
                .iter()
                .any(|reference| reference.trim().is_empty() || reference.chars().count() > 256)
        {
            return Err(ExecutionValidationError::InvalidProcedureDeviationEvidence);
        }
        Ok(())
    }
}

/// Semantic uncertainty that deterministic lifecycle rules cannot resolve.
/// This marker is produced only from a validated structured step conclusion;
/// arbitrary prose never grants access to the model evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmbiguityKind {
    RemainingWorkMayBeUnnecessary,
    PlanAssumptionMayBeInvalid,
    RecoverableAlternativeMayExist,
    GoalMayBePartiallySatisfied,
    RemainingDependenciesMayNeedReordering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmbiguity {
    pub kind: PlanAmbiguityKind,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
}

impl PlanAmbiguity {
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.safe_summary.trim().is_empty() {
            return Err(ExecutionValidationError::MissingAmbiguitySummary);
        }
        if self.safe_summary.chars().count() > 500 {
            return Err(ExecutionValidationError::AmbiguitySummaryTooLong);
        }
        if self.evidence_refs.len() > 32
            || self
                .evidence_refs
                .iter()
                .any(|reference| reference.trim().is_empty() || reference.chars().count() > 256)
        {
            return Err(ExecutionValidationError::InvalidAmbiguityEvidence);
        }
        Ok(())
    }
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedure_applications: Vec<ProcedureApplication>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedure_deviations: Vec<ProcedureDeviation>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambiguity: Option<PlanAmbiguity>,
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
        if let Some(ambiguity) = &self.ambiguity {
            ambiguity.validate()?;
        }
        for application in &self.procedure_applications {
            application.validate()?;
        }
        for deviation in &self.procedure_deviations {
            deviation.validate()?;
            if let Some(application_id) = deviation.application_id.as_deref()
                && self
                    .procedure_applications
                    .iter()
                    .all(|application| application.application_id != application_id)
            {
                return Err(ExecutionValidationError::UnknownProcedureApplication);
            }
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
    Rejected,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecisionSource {
    #[default]
    Rule,
    Model,
    SafeFallback,
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
    #[serde(default)]
    pub source: PlanDecisionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_key: Option<String>,
    #[serde(default)]
    pub model_turns_used: u32,
    #[serde(default)]
    pub repairs_used: u32,
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

/// User-visible terminal classification. This is deliberately separate from
/// process-level `RunStatus` and compatibility `TerminationReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalOutcomeStatus {
    Success,
    Partial,
    Blocked,
    Rejected,
    Cancelled,
    Interrupted,
    Exhausted,
    Indeterminate,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizationMode {
    Direct,
    Model,
    Deterministic,
    DeterministicFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizationPhase {
    Started,
    Completed,
}

/// Durable finalization record. The output is bounded before persistence by
/// the Finalizer and contains only user-visible synthesis, never hidden model
/// reasoning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizationRecord {
    pub finalization_id: String,
    pub phase: FinalizationPhase,
    pub finish_reason: PlanFinishReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<FinalOutcomeStatus>,
    pub mode: FinalizationMode,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incomplete_step_ids: Vec<String>,
    #[serde(default)]
    pub budget_before: ExecutionBudgetUsage,
    #[serde(default)]
    pub budget_after: ExecutionBudgetUsage,
}

/// Explicit, safe degradation fact. Fallbacks are never silent and do not
/// change permissions or erase previously recorded evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDegradation {
    pub degradation_id: String,
    pub phase: ExecutionPhase,
    pub code: String,
    pub safe_summary: String,
    pub occurred_at: String,
}

/// Materialized run lifecycle projection stored in task state and checkpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionLifecycleState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ExecutionPolicy>,
    pub budget_usage: ExecutionBudgetUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_exhaustion: Option<ExecutionBudgetExhaustion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization: Option<FinalizationRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degradations: Vec<ExecutionDegradation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedure_applications: Vec<ProcedureApplication>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedure_deviations: Vec<ProcedureDeviation>,
}

impl ExecutionLifecycleState {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none()
            && self.budget_usage == ExecutionBudgetUsage::default()
            && self.budget_exhaustion.is_none()
            && self.finalization.is_none()
            && self.degradations.is_empty()
            && self.procedure_applications.is_empty()
            && self.procedure_deviations.is_empty()
    }

    pub fn checkpoint(&self) -> ExecutionLifecycleCheckpoint {
        ExecutionLifecycleCheckpoint {
            policy: self.policy.clone(),
            budget_usage: self.budget_usage.clone(),
            budget_exhaustion: self.budget_exhaustion.clone(),
            finalization: self.finalization.clone(),
            degradation_count: self.degradations.len(),
            procedure_application_count: self.procedure_applications.len(),
            procedure_deviation_count: self.procedure_deviations.len(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionLifecycleCheckpoint {
    pub policy: Option<ExecutionPolicy>,
    pub budget_usage: ExecutionBudgetUsage,
    pub budget_exhaustion: Option<ExecutionBudgetExhaustion>,
    pub finalization: Option<FinalizationRecord>,
    pub degradation_count: usize,
    pub procedure_application_count: usize,
    pub procedure_deviation_count: usize,
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
    #[error("procedure application field {field} must not be empty")]
    MissingProcedureApplicationField { field: &'static str },
    #[error("procedure application contains invalid section identities")]
    InvalidProcedureSections,
    #[error("procedure deviation ID must not be empty")]
    MissingProcedureDeviationId,
    #[error("procedure deviation summary is empty or exceeds its bound")]
    InvalidProcedureDeviationSummary,
    #[error("procedure deviation evidence is invalid")]
    InvalidProcedureDeviationEvidence,
    #[error("procedure deviation references an unknown application")]
    UnknownProcedureApplication,
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
    #[error("plan ambiguity summary must not be empty")]
    MissingAmbiguitySummary,
    #[error("plan ambiguity summary exceeds 500 characters")]
    AmbiguitySummaryTooLong,
    #[error("plan ambiguity evidence references are invalid or exceed bounds")]
    InvalidAmbiguityEvidence,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_steps_sugar_keeps_react_and_plan_units_distinct() {
        let react = ExecutionPolicy::from_max_steps_and_plan_flag(5, false);
        assert_eq!(react.strategy, ExecutionStrategy::React);
        assert_eq!(react.budgets.max_model_turns, Some(5));
        assert_eq!(react.budgets.max_step_attempts, None);

        let planned = ExecutionPolicy::from_max_steps_and_plan_flag(5, true);
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
