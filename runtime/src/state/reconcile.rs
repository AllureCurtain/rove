//! Trace-tail reconciliation for resumed runs.
//!
//! `trace.jsonl` is the append-only record of event facts and `task_state.json`
//! is the resumable snapshot. The snapshot is written after the trace line, so a
//! crash between those two writes leaves durable lifecycle facts in the trace
//! that the snapshot does not yet reflect.
//!
//! Reconciliation replays only the trace tail newer than
//! `checkpoint.last_event_seq` and applies those facts to the loaded snapshot.
//! It is a projection, never an executor:
//!
//! - Completed work is never replayed. Tool calls, model turns, and mutations
//!   are not re-dispatched; only their already-recorded facts are projected.
//! - Application is idempotent. Replaying the same tail twice yields the same
//!   state, so a crash during reconciliation is safe.
//! - A non-success terminal state is never relabelled as completed.
//! - Unparsable tail lines are skipped with a bounded count rather than
//!   failing the resume or silently trusting a truncated tail.

use std::path::Path;

use crate::events::StreamEvent;
use crate::execution::{ExecutionLifecycleState, StepLedgerState, StepRecordStatus};
use crate::types::TaskState;

/// Bounded outcome of reconciling one run's trace tail into its snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceReconciliation {
    /// Highest event sequence represented by the returned state.
    pub last_event_seq: Option<u64>,
    /// Trace lines newer than the snapshot that were applied.
    pub applied_event_count: usize,
    /// Trace lines newer than the snapshot that could not be parsed.
    pub corrupt_line_count: usize,
    /// True when at least one applied fact changed the snapshot.
    pub changed: bool,
}

impl TraceReconciliation {
    fn observed(&mut self, seq: u64) {
        self.last_event_seq = Some(self.last_event_seq.map_or(seq, |saved| saved.max(seq)));
    }
}

/// Reconcile `state` with the trace tail in `run_dir`.
///
/// Returns the reconciliation summary. `state` is mutated in place only for
/// facts newer than its own checkpoint sequence.
pub async fn reconcile_task_state_with_trace(
    run_dir: &Path,
    state: &mut TaskState,
) -> std::io::Result<TraceReconciliation> {
    let path = run_dir.join("trace.jsonl");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TraceReconciliation {
                last_event_seq: snapshot_seq(state),
                ..TraceReconciliation::default()
            });
        }
        Err(error) => return Err(error),
    };

    let applied_through = snapshot_seq(state);
    let mut outcome = TraceReconciliation {
        last_event_seq: applied_through,
        ..TraceReconciliation::default()
    };

    // Trace sequence numbers are assigned 1-based in append order, matching
    // `import_trace_events` during index repair.
    let mut seq: u64 = 0;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        seq += 1;
        if applied_through.is_some_and(|applied| seq <= applied) {
            continue;
        }
        match serde_json::from_str::<StreamEvent>(line) {
            Ok(event) => {
                if apply_event(state, &event) {
                    outcome.changed = true;
                }
                outcome.applied_event_count += 1;
                outcome.observed(seq);
            }

            Err(error) => {
                outcome.corrupt_line_count += 1;
                tracing::warn!(
                    path = %path.display(),
                    line = seq,
                    error = %error,
                    "Skipping corrupted trace line during resume reconciliation"
                );
            }
        }
    }

    if outcome.changed || outcome.last_event_seq != applied_through {
        // Recomputed from the reconciled state so the bounded checkpoint
        // projection cannot disagree with the full snapshot it summarizes.
        let last_event_seq = outcome.last_event_seq;
        let step_ledger = state.step_ledger.checkpoint();
        let execution_lifecycle = state.execution_lifecycle.checkpoint();
        let plan = state.plan.clone();
        let last_step = state.step;
        if let Some(checkpoint) = state.checkpoint.as_mut() {
            checkpoint.last_event_seq = last_event_seq;
            checkpoint.step_ledger = step_ledger;
            checkpoint.execution_lifecycle = execution_lifecycle;
            checkpoint.plan = plan;
            checkpoint.last_step = last_step;
        }
    }

    Ok(outcome)
}

fn snapshot_seq(state: &TaskState) -> Option<u64> {
    state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.last_event_seq)
}

/// Apply one trace fact to the snapshot. Returns true when state changed.
///
/// Only durable lifecycle and planning facts are projected. Streaming deltas,
/// approval prompts, and in-flight tool dispatch are deliberately ignored: they
/// are not resumable state, and an in-flight tool whose outcome never reached
/// the trace must stay indeterminate rather than be assumed complete.
fn apply_event(state: &mut TaskState, event: &StreamEvent) -> bool {
    match event {
        StreamEvent::ExecutionStrategySelected { policy } => {
            let next = Some(policy.clone());
            replace_if_changed(&mut state.execution_lifecycle.policy, next)
        }
        StreamEvent::ExecutionBudgetUpdated { snapshot, .. } => {
            // Consumption is monotonic, so a newer trace fact always wins over
            // an older snapshot value.
            let usage_changed = replace_if_changed(
                &mut state.execution_lifecycle.budget_usage,
                snapshot.consumed.clone(),
            );
            // An exhaustion fact is sticky: a later projection without one must
            // not clear a recorded boundary.
            let exhaustion_changed = match snapshot.exhausted.clone() {
                Some(exhausted) => replace_if_changed(
                    &mut state.execution_lifecycle.budget_exhaustion,
                    Some(exhausted),
                ),
                None => false,
            };
            usage_changed || exhaustion_changed
        }
        StreamEvent::ExecutionDegraded { record } => {
            if state
                .execution_lifecycle
                .degradations
                .iter()
                .any(|saved| saved.degradation_id == record.degradation_id)
            {
                return false;
            }
            state.execution_lifecycle.degradations.push(record.clone());
            true
        }
        StreamEvent::FinalizationStarted { record }
        | StreamEvent::FinalizationCompleted { record } => {
            apply_finalization(&mut state.execution_lifecycle, record)
        }
        StreamEvent::PlanCreated {
            plan,
            identity,
            plan_revision,
        } => {
            let mut changed = replace_if_changed(&mut state.plan, Some(plan.clone()));
            let before = state.step_ledger.clone();
            state.step_ledger.set_plan_identity(identity);
            if let Some(revision) = plan_revision.as_deref() {
                push_revision(&mut state.step_ledger, revision);
            }
            changed |= state.step_ledger != before;
            changed
        }
        StreamEvent::PlanRevised { plan, revision } => {
            let mut changed = replace_if_changed(&mut state.plan, Some(plan.clone()));
            let before = state.step_ledger.clone();
            state.step_ledger.set_plan_identity(&revision.identity());
            push_revision(&mut state.step_ledger, revision);
            changed |= state.step_ledger != before;
            changed
        }
        StreamEvent::PlanStepStarted { attempt, .. } => {
            if !attempt.is_complete() {
                return false;
            }
            // An in-flight attempt is recorded so a crash mid-step can be
            // closed conservatively instead of silently retried.
            replace_if_changed(
                &mut state.step_ledger.active_step_attempt,
                Some(attempt.clone()),
            )
        }
        StreamEvent::StepResult { record } => {
            if state
                .step_ledger
                .step_records
                .iter()
                .any(|saved| saved.record_id == record.record_id)
            {
                return false;
            }
            state.step_ledger.step_records.push(record.as_ref().clone());
            // The terminal fact for an attempt clears the in-flight marker for
            // that same attempt only.
            if state
                .step_ledger
                .active_step_attempt
                .as_ref()
                .is_some_and(|active| {
                    active.step_id == record.step_id && active.attempt == record.attempt
                })
            {
                state.step_ledger.active_step_attempt = None;
            }
            mark_plan_step_done(state, record.step_id.as_str(), record.status);
            true
        }
        StreamEvent::PlanDecision { record } => {
            // `push_decision` is itself idempotent on decision and trigger
            // identity; compare before/after so the caller learns whether this
            // fact actually advanced the snapshot.
            let before = state.step_ledger.plan_lifecycle.decisions.len();
            state
                .step_ledger
                .plan_lifecycle
                .push_decision(record.as_ref().clone());
            state.step_ledger.plan_lifecycle.decisions.len() != before
        }
        StreamEvent::PromptCompacted { summary, .. } => match summary.clone() {
            Some(summary) => replace_if_changed(&mut state.summary, Some(summary)),
            None => false,
        },
        // A terminal run fact never rewrites the snapshot's outcome here. The
        // finalization record is the outcome authority, and a non-success
        // reason must not be projected as completed work.
        StreamEvent::RunCompleted { .. } => false,
        _ => false,
    }
}

/// Finalization is single-authority and monotonic: a `completed` record
/// supersedes a `started` record for the same finalization, and an existing
/// completed record is never downgraded.
fn apply_finalization(
    lifecycle: &mut ExecutionLifecycleState,
    record: &crate::execution::FinalizationRecord,
) -> bool {
    match lifecycle.finalization.as_ref() {
        Some(saved) if saved == record => false,
        // Keep the record that carries a resolved outcome.
        Some(saved) if saved.outcome.is_some() && record.outcome.is_none() => false,
        _ => {
            lifecycle.finalization = Some(record.clone());
            true
        }
    }
}

fn push_revision(ledger: &mut StepLedgerState, revision: &crate::execution::PlanRevision) {
    if ledger
        .plan_lifecycle
        .revisions
        .iter()
        .any(|saved| saved.revision_id == revision.revision_id)
    {
        return;
    }
    ledger.plan_lifecycle.push_revision(revision.clone());
}

/// Reflect a terminal step fact into the persisted plan projection.
///
/// Only a successful record marks plan progress. A failed, cancelled, or
/// indeterminate record leaves the step open so resume cannot treat unproven
/// work as finished.
fn mark_plan_step_done(state: &mut TaskState, step_id: &str, status: StepRecordStatus) {
    if status != StepRecordStatus::Succeeded {
        return;
    }
    if let Some(plan) = state.plan.as_mut()
        && let Some(step) = plan.steps.iter_mut().find(|step| step.id == step_id)
    {
        step.done = true;
    }
}

fn replace_if_changed<T: PartialEq>(slot: &mut T, next: T) -> bool {
    if *slot == next {
        return false;
    }
    *slot = next;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{
        ExecutionBudgetDimension, ExecutionBudgetExhaustion, ExecutionBudgetSnapshot,
        ExecutionBudgetUsage, ExecutionDegradation, ExecutionPhase, ExecutionPolicy,
        FinalOutcomeStatus, FinalizationMode, FinalizationPhase, FinalizationRecord,
        PlanFinishReason, StepCompletionBasis, StepRecord,
    };
    use crate::types::{
        JobId, PlanStep, PromptCheckpoint, PromptCompactionState, RunId, SessionId, TaskPlan,
        TaskState, TerminationReason,
    };

    fn checkpoint(last_event_seq: Option<u64>) -> PromptCheckpoint {
        PromptCheckpoint {
            summary: None,
            preserved_tail: Vec::new(),
            session: None,
            plan: None,
            session_memory_pointer: None,
            durable_memory_pointer: None,
            last_step: 0,
            last_event_seq,
            token_estimate: 0,
            compacted_history_messages: 0,
            compaction: PromptCompactionState::default(),
            runtime_identity: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        }
    }

    fn state(last_event_seq: Option<u64>) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "reconcile".to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: Some(checkpoint(last_event_seq)),
            plan: None,
            runtime_identity: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        }
    }

    fn step_record(record_id: &str, step_id: &str, status: StepRecordStatus) -> StepRecord {
        StepRecord {
            record_id: record_id.to_string(),
            plan_id: "plan-1".to_string(),
            plan_revision_id: "rev-1".to_string(),
            step_id: step_id.to_string(),
            attempt: 1,
            status,
            started_at: "2026-08-08T00:00:00Z".to_string(),
            finished_at: "2026-08-08T00:00:01Z".to_string(),
            summary: "did the thing".to_string(),
            completion_basis: StepCompletionBasis::DeterministicRule,
            evidence_refs: Vec::new(),
            tool_call_ids: Vec::new(),
            artifact_refs: Vec::new(),
            mutations: Vec::new(),
            model_turns_used: 1,
            tool_calls_used: 1,
            token_usage: Default::default(),
            error_code: None,
            safe_error_summary: None,
            supersedes_record_id: None,
            ambiguity: None,
        }
    }

    async fn write_trace(dir: &Path, events: &[StreamEvent]) {
        let mut body = String::new();
        for event in events {
            body.push_str(&serde_json::to_string(event).unwrap());
            body.push('\n');
        }
        tokio::fs::write(dir.join("trace.jsonl"), body)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn missing_trace_leaves_state_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut snapshot = state(Some(4));
        let before = snapshot.clone();

        let outcome = reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(outcome.applied_event_count, 0);
        assert!(!outcome.changed);
        assert_eq!(outcome.last_event_seq, Some(4));
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }

    #[tokio::test]
    async fn events_already_covered_by_the_checkpoint_are_not_reapplied() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_trace(
            tmp.path(),
            &[
                StreamEvent::ExecutionDegraded {
                    record: ExecutionDegradation {
                        degradation_id: "deg-1".to_string(),
                        phase: ExecutionPhase::Evaluator,
                        code: "evaluator_unavailable".to_string(),
                        safe_summary: "fell back to deterministic rules".to_string(),
                        occurred_at: "2026-08-08T00:00:00Z".to_string(),
                    },
                },
                StreamEvent::ExecutionDegraded {
                    record: ExecutionDegradation {
                        degradation_id: "deg-2".to_string(),
                        phase: ExecutionPhase::Finalizer,
                        code: "finalizer_unavailable".to_string(),
                        safe_summary: "used deterministic finalizer".to_string(),
                        occurred_at: "2026-08-08T00:00:02Z".to_string(),
                    },
                },
            ],
        )
        .await;

        // The snapshot already reflects the first line.
        let mut snapshot = state(Some(1));
        let outcome = reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(outcome.applied_event_count, 1);
        assert_eq!(outcome.last_event_seq, Some(2));
        assert_eq!(snapshot.execution_lifecycle.degradations.len(), 1);
        assert_eq!(
            snapshot.execution_lifecycle.degradations[0].degradation_id,
            "deg-2"
        );
    }

    #[tokio::test]
    async fn reconciliation_is_idempotent_across_repeated_runs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ExecutionPolicy::from_max_steps_and_plan_flag(8, true);
        write_trace(
            tmp.path(),
            &[
                StreamEvent::ExecutionStrategySelected {
                    policy: policy.clone(),
                },
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-1", "s1", StepRecordStatus::Succeeded)),
                },
                StreamEvent::ExecutionDegraded {
                    record: ExecutionDegradation {
                        degradation_id: "deg-1".to_string(),
                        phase: ExecutionPhase::Evaluator,
                        code: "evaluator_unavailable".to_string(),
                        safe_summary: "deterministic fallback".to_string(),
                        occurred_at: "2026-08-08T00:00:03Z".to_string(),
                    },
                },
            ],
        )
        .await;

        let mut first = state(None);
        first.plan = Some(TaskPlan {
            goal: "reconcile".to_string(),
            steps: vec![PlanStep {
                id: "s1".to_string(),
                title: "inspect".to_string(),
                done: false,
            }],
            current_step: 0,
        });
        let mut second = first.clone();

        let first_outcome = reconcile_task_state_with_trace(tmp.path(), &mut first)
            .await
            .unwrap();
        assert!(first_outcome.changed);
        assert_eq!(first_outcome.applied_event_count, 3);

        // Re-running against the already-reconciled state applies the same tail
        // again and must converge to an identical snapshot.
        reconcile_task_state_with_trace(tmp.path(), &mut second)
            .await
            .unwrap();
        let replay = reconcile_task_state_with_trace(tmp.path(), &mut second)
            .await
            .unwrap();

        assert!(!replay.changed);
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(first.step_ledger.step_records.len(), 1);
        assert_eq!(first.execution_lifecycle.degradations.len(), 1);
    }

    #[tokio::test]
    async fn only_successful_step_facts_mark_plan_progress() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_trace(
            tmp.path(),
            &[
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-1", "s1", StepRecordStatus::Succeeded)),
                },
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-2", "s2", StepRecordStatus::Failed)),
                },
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-3", "s3", StepRecordStatus::Indeterminate)),
                },
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-4", "s4", StepRecordStatus::Cancelled)),
                },
            ],
        )
        .await;

        let mut snapshot = state(None);
        snapshot.plan = Some(TaskPlan {
            goal: "reconcile".to_string(),
            steps: ["s1", "s2", "s3", "s4"]
                .into_iter()
                .map(|id| PlanStep {
                    id: id.to_string(),
                    title: id.to_string(),
                    done: false,
                })
                .collect(),
            current_step: 0,
        });

        reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        let steps = snapshot.plan.as_ref().unwrap().steps.clone();
        assert!(steps[0].done, "a succeeded record marks the step done");
        for step in &steps[1..] {
            assert!(
                !step.done,
                "non-success record must not mark {} done",
                step.id
            );
        }
    }

    #[tokio::test]
    async fn a_recorded_exhaustion_boundary_is_never_cleared_by_a_later_projection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exhaustion = ExecutionBudgetExhaustion {
            dimension: ExecutionBudgetDimension::ModelTurns,
            phase: ExecutionPhase::Step,
            limit: 4,
            consumed: 4,
            safe_summary: "model turn budget reached".to_string(),
        };
        write_trace(
            tmp.path(),
            &[
                StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Step,
                    snapshot: Box::new(ExecutionBudgetSnapshot {
                        limits: Default::default(),
                        consumed: ExecutionBudgetUsage {
                            model_turns: 4,
                            ..Default::default()
                        },
                        exhausted: Some(exhaustion.clone()),
                        cost_enforced: false,
                    }),
                },
                StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Finalizer,
                    snapshot: Box::new(ExecutionBudgetSnapshot {
                        limits: Default::default(),
                        consumed: ExecutionBudgetUsage {
                            model_turns: 5,
                            ..Default::default()
                        },
                        exhausted: None,
                        cost_enforced: false,
                    }),
                },
            ],
        )
        .await;

        let mut snapshot = state(None);
        reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(
            snapshot.execution_lifecycle.budget_exhaustion,
            Some(exhaustion),
            "exhaustion is a sticky boundary fact"
        );
        assert_eq!(snapshot.execution_lifecycle.budget_usage.model_turns, 5);
    }

    #[tokio::test]
    async fn a_resolved_finalization_outcome_is_not_downgraded() {
        let tmp = tempfile::TempDir::new().unwrap();
        let completed = FinalizationRecord {
            finalization_id: "fin-1".to_string(),
            phase: FinalizationPhase::Completed,
            finish_reason: PlanFinishReason::Partial,
            outcome: Some(FinalOutcomeStatus::Partial),
            mode: FinalizationMode::Deterministic,
            started_at: "2026-08-08T00:00:00Z".to_string(),
            completed_at: Some("2026-08-08T00:00:01Z".to_string()),
            output: Some("partial work".to_string()),
            evidence_refs: Vec::new(),
            incomplete_step_ids: Vec::new(),
            budget_before: Default::default(),
            budget_after: Default::default(),
        };
        let started = FinalizationRecord {
            phase: FinalizationPhase::Started,
            outcome: None,
            completed_at: None,
            output: None,
            ..completed.clone()
        };
        write_trace(
            tmp.path(),
            &[
                StreamEvent::FinalizationCompleted {
                    record: Box::new(completed.clone()),
                },
                // An out-of-order or replayed `started` fact must not erase the
                // resolved outcome.
                StreamEvent::FinalizationStarted {
                    record: Box::new(started),
                },
            ],
        )
        .await;

        let mut snapshot = state(None);
        reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(snapshot.execution_lifecycle.finalization, Some(completed));
    }

    #[tokio::test]
    async fn a_terminal_run_event_does_not_relabel_the_outcome() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_trace(
            tmp.path(),
            &[StreamEvent::RunCompleted {
                reason: TerminationReason::Cancelled,
                output: Some("stopped".to_string()),
            }],
        )
        .await;

        let mut snapshot = state(None);
        let outcome = reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(outcome.applied_event_count, 1);
        assert!(!outcome.changed);
        assert!(
            snapshot.execution_lifecycle.finalization.is_none(),
            "the finalization record is the only outcome authority"
        );
    }

    #[tokio::test]
    async fn an_in_flight_attempt_is_recorded_and_cleared_only_by_its_own_terminal_fact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let attempt = crate::execution::StepAttempt {
            plan_id: "plan-1".to_string(),
            plan_revision_id: "rev-1".to_string(),
            step_id: "s1".to_string(),
            attempt: 1,
            started_at: "2026-08-08T00:00:00Z".to_string(),
        };
        write_trace(
            tmp.path(),
            &[
                StreamEvent::PlanStepStarted {
                    step: PlanStep {
                        id: "s1".to_string(),
                        title: "inspect".to_string(),
                        done: false,
                    },
                    index: 0,
                    attempt: attempt.clone(),
                    budget: Default::default(),
                },
                // A terminal fact for a different step must not clear it.
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-9", "s2", StepRecordStatus::Succeeded)),
                },
            ],
        )
        .await;

        let mut snapshot = state(None);
        reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();
        assert_eq!(
            snapshot.step_ledger.active_step_attempt.as_ref(),
            Some(&attempt),
            "an unresolved attempt stays in-flight for conservative resume"
        );

        write_trace(
            tmp.path(),
            &[
                StreamEvent::PlanStepStarted {
                    step: PlanStep {
                        id: "s1".to_string(),
                        title: "inspect".to_string(),
                        done: false,
                    },
                    index: 0,
                    attempt: attempt.clone(),
                    budget: Default::default(),
                },
                StreamEvent::StepResult {
                    record: Box::new(step_record("rec-1", "s1", StepRecordStatus::Succeeded)),
                },
            ],
        )
        .await;

        let mut resolved = state(None);
        reconcile_task_state_with_trace(tmp.path(), &mut resolved)
            .await
            .unwrap();
        assert!(resolved.step_ledger.active_step_attempt.is_none());
    }

    #[tokio::test]
    async fn corrupt_tail_lines_are_counted_and_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let good = serde_json::to_string(&StreamEvent::ExecutionDegraded {
            record: ExecutionDegradation {
                degradation_id: "deg-1".to_string(),
                phase: ExecutionPhase::Evaluator,
                code: "evaluator_unavailable".to_string(),
                safe_summary: "deterministic fallback".to_string(),
                occurred_at: "2026-08-08T00:00:00Z".to_string(),
            },
        })
        .unwrap();
        // A truncated final line is the realistic crash shape.
        let body = format!("{good}\n{{\"type\":\"execution_deg\n");
        tokio::fs::write(tmp.path().join("trace.jsonl"), body)
            .await
            .unwrap();

        let mut snapshot = state(None);
        let outcome = reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        assert_eq!(outcome.applied_event_count, 1);
        assert_eq!(outcome.corrupt_line_count, 1);
        assert_eq!(outcome.last_event_seq, Some(1));
        assert_eq!(snapshot.execution_lifecycle.degradations.len(), 1);
    }

    #[tokio::test]
    async fn the_checkpoint_projection_is_refreshed_after_reconciliation() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_trace(
            tmp.path(),
            &[StreamEvent::StepResult {
                record: Box::new(step_record("rec-1", "s1", StepRecordStatus::Succeeded)),
            }],
        )
        .await;

        let mut snapshot = state(None);
        reconcile_task_state_with_trace(tmp.path(), &mut snapshot)
            .await
            .unwrap();

        let checkpoint = snapshot.checkpoint.as_ref().unwrap();
        assert_eq!(checkpoint.last_event_seq, Some(1));
        assert_eq!(
            checkpoint.step_ledger.step_record_count, 1,
            "the bounded checkpoint projection tracks the reconciled ledger"
        );
    }
}
