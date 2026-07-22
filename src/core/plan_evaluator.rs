use crate::core::execution::{
    PlanDecision, PlanDecisionKind, PlanDecisionRecord, PlanFinishReason, StepRecord,
    StepRecordStatus,
};

pub(crate) const RECOVERABLE_STEP_FAILURE_CODE: &str = "step_recoverable_failure";

/// Apply deterministic lifecycle rules to one terminal step fact.
///
/// Model-on-ambiguity evaluation is intentionally deferred. This evaluator
/// never calls a provider and therefore remains replay-safe.
pub(crate) fn evaluate_step_record(
    record: &StepRecord,
    has_remaining_steps: bool,
) -> PlanDecisionRecord {
    let decision = match record.status {
        StepRecordStatus::Succeeded | StepRecordStatus::Skipped if has_remaining_steps => decision(
            PlanDecisionKind::Continue,
            vec!["step_terminal_success", "remaining_work_available"],
            "The step succeeded and the remaining plan can continue.",
            Vec::new(),
            None,
        ),
        StepRecordStatus::Succeeded | StepRecordStatus::Skipped => decision(
            PlanDecisionKind::Finish,
            vec!["all_planned_work_terminal"],
            "All planned work reached a terminal success.",
            Vec::new(),
            Some(PlanFinishReason::Completed),
        ),
        StepRecordStatus::Failed
            if record.error_code.as_deref() == Some(RECOVERABLE_STEP_FAILURE_CODE) =>
        {
            decision(
                PlanDecisionKind::ReplaceRemaining,
                vec!["recoverable_step_failure", "replace_remaining"],
                "The failed step requires a safe replacement for the remaining plan.",
                vec![format!(
                    "Replace failed step {} without replaying completed work or mutations.",
                    record.step_id
                )],
                None,
            )
        }
        StepRecordStatus::Failed => decision(
            PlanDecisionKind::Finish,
            vec!["fatal_step_failure"],
            "The step failed and execution cannot continue safely.",
            Vec::new(),
            Some(PlanFinishReason::Failed),
        ),
        StepRecordStatus::Blocked => decision(
            PlanDecisionKind::Finish,
            vec!["step_blocked"],
            "Required permission or capability is unavailable.",
            Vec::new(),
            Some(PlanFinishReason::Blocked),
        ),
        StepRecordStatus::BudgetExhausted => decision(
            PlanDecisionKind::Finish,
            vec!["budget_exhausted"],
            "Execution stopped because a configured budget was exhausted.",
            Vec::new(),
            Some(PlanFinishReason::BudgetExhausted),
        ),
        StepRecordStatus::Cancelled => decision(
            PlanDecisionKind::Finish,
            vec!["step_cancelled"],
            "Execution was cancelled before the step completed.",
            Vec::new(),
            Some(PlanFinishReason::Cancelled),
        ),
        StepRecordStatus::Interrupted => decision(
            PlanDecisionKind::Finish,
            vec!["step_interrupted", "unknown_side_effect_not_replayed"],
            "The step was interrupted and unknown side effects were not replayed.",
            Vec::new(),
            Some(PlanFinishReason::Interrupted),
        ),
        StepRecordStatus::Partial => decision(
            PlanDecisionKind::Finish,
            vec!["partial_step_result"],
            "The step produced only a partial result and cannot continue automatically.",
            Vec::new(),
            Some(PlanFinishReason::Partial),
        ),
    };

    let record = PlanDecisionRecord {
        trigger_step_record_id: record.record_id.clone(),
        decided_at: chrono::Utc::now().to_rfc3339(),
        decision,
    };
    debug_assert!(record.validate().is_ok());
    record
}

fn decision(
    kind: PlanDecisionKind,
    safe_reason_codes: Vec<&str>,
    safe_summary: &str,
    remaining_work_requirements: Vec<String>,
    finish_reason: Option<PlanFinishReason>,
) -> PlanDecision {
    PlanDecision {
        decision_id: ulid::Ulid::new().to_string(),
        kind,
        safe_reason_codes: safe_reason_codes.into_iter().map(str::to_string).collect(),
        safe_summary: safe_summary.to_string(),
        remaining_work_requirements,
        finish_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::{RECOVERABLE_STEP_FAILURE_CODE, evaluate_step_record};
    use crate::core::execution::{
        PlanDecisionKind, PlanFinishReason, StepCompletionBasis, StepRecord, StepRecordStatus,
    };
    use crate::core::types::Usage;

    fn step_record(status: StepRecordStatus, error_code: Option<&str>) -> StepRecord {
        StepRecord {
            record_id: "record-1".to_string(),
            plan_id: "plan-1".to_string(),
            plan_revision_id: "revision-1".to_string(),
            step_id: "step-1".to_string(),
            attempt: 1,
            status,
            started_at: "2026-07-20T00:00:00Z".to_string(),
            finished_at: "2026-07-20T00:00:01Z".to_string(),
            summary: "result".to_string(),
            completion_basis: StepCompletionBasis::RuntimeFailure,
            evidence_refs: Vec::new(),
            tool_call_ids: Vec::new(),
            artifact_refs: Vec::new(),
            mutations: Vec::new(),
            model_turns_used: 1,
            tool_calls_used: 0,
            token_usage: Usage::default(),
            error_code: error_code.map(str::to_string),
            safe_error_summary: None,
            supersedes_record_id: None,
        }
    }

    #[test]
    fn successful_steps_continue_or_finish_from_remaining_work() {
        let record = step_record(StepRecordStatus::Succeeded, None);

        let continuing = evaluate_step_record(&record, true);
        let finished = evaluate_step_record(&record, false);

        assert_eq!(continuing.decision.kind, PlanDecisionKind::Continue);
        assert_eq!(finished.decision.kind, PlanDecisionKind::Finish);
        assert_eq!(
            finished.decision.finish_reason,
            Some(PlanFinishReason::Completed)
        );
    }

    #[test]
    fn only_explicitly_recoverable_failure_replaces_remaining_work() {
        let recoverable = step_record(
            StepRecordStatus::Failed,
            Some(RECOVERABLE_STEP_FAILURE_CODE),
        );
        let fatal = step_record(StepRecordStatus::Failed, Some("step_runtime_failure"));

        assert_eq!(
            evaluate_step_record(&recoverable, true).decision.kind,
            PlanDecisionKind::ReplaceRemaining
        );
        let fatal = evaluate_step_record(&fatal, true);
        assert_eq!(fatal.decision.kind, PlanDecisionKind::Finish);
        assert_eq!(fatal.decision.finish_reason, Some(PlanFinishReason::Failed));
    }

    #[test]
    fn non_success_terminal_states_finish_with_typed_reasons() {
        for (status, expected) in [
            (StepRecordStatus::Blocked, PlanFinishReason::Blocked),
            (
                StepRecordStatus::BudgetExhausted,
                PlanFinishReason::BudgetExhausted,
            ),
            (StepRecordStatus::Cancelled, PlanFinishReason::Cancelled),
            (StepRecordStatus::Interrupted, PlanFinishReason::Interrupted),
            (StepRecordStatus::Partial, PlanFinishReason::Partial),
        ] {
            let decision = evaluate_step_record(&step_record(status, None), true);
            assert_eq!(decision.decision.kind, PlanDecisionKind::Finish);
            assert_eq!(decision.decision.finish_reason, Some(expected));
            decision.validate().unwrap();
        }
    }
}
