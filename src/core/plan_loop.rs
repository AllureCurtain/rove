use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::events::StreamEvent;
use crate::core::execution::{
    ExecutionBudgetUsage, PlanDecisionKind, PlanDecisionRecord, PlanFinishReason, PlanIdentity,
    PlanRevision, StepAttempt, StepCompletionBasis, StepLedgerState, StepRecord, StepRecordStatus,
    planned_step_failure_message,
};
use crate::core::plan_evaluator::{RECOVERABLE_STEP_FAILURE_CODE, evaluate_step_record};
use crate::core::planner::Planner;
use crate::core::run_loop::{LoopContext, LoopItem};
use crate::core::step_runner::{
    StepRunMetrics, StepRunnerInput, StepRunnerItem, StepRunnerOutcome, run_step,
};
use crate::core::types::{Message, PlanStep, TaskPlan, TerminationReason};

const MAX_STEP_RECORD_SUMMARY_CHARS: usize = 1_000;

pub(crate) struct PlanLoopState {
    pub user_message: String,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub step: u32,
    pub plan: Option<TaskPlan>,
    pub step_ledger: StepLedgerState,
}

pub(crate) fn run_planned_loop<'a>(
    ctx: LoopContext<'a>,
    planner: &'a Planner,
    mut state: PlanLoopState,
    cancel_token: CancellationToken,
) -> BoxStream<'a, LoopItem> {
    Box::pin(stream! {
        let mut compaction = ctx.compaction.clone();
        let mut ledger = std::mem::take(&mut state.step_ledger);
        let mut plan_identity = ledger.plan_identity().unwrap_or_else(PlanIdentity::fresh);

        if state.plan.is_none() {
            let draft_result = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    yield LoopItem::Complete {
                        reason: TerminationReason::Cancelled,
                        output: None,
                    };
                    return;
                }
                result = planner.draft(ctx.model, &state.user_message, &state.history) => result,
            };
            match draft_result {
                Ok(drafted) => {
                    plan_identity = PlanIdentity::fresh();
                    ledger.set_plan_identity(&plan_identity);
                    let revision = match initial_plan_revision(
                        &drafted,
                        &plan_identity,
                        "planner_draft",
                    ) {
                        Ok(revision) => revision,
                        Err(reason) => {
                            yield LoopItem::Complete {
                                reason: TerminationReason::Error,
                                output: Some(reason),
                            };
                            return;
                        }
                    };
                    ledger.plan_lifecycle.push_revision(revision.clone());
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: drafted.clone(),
                        identity: plan_identity.clone(),
                        plan_revision: Some(Box::new(revision)),
                    });
                    state.plan = Some(drafted);
                }
                Err(err) => {
                    yield LoopItem::Complete {
                        reason: TerminationReason::Error,
                        output: Some(format!("Planner error: {err}")),
                    };
                    return;
                }
            }
        } else if ledger.plan_lifecycle.revisions.is_empty() {
            // Older task snapshots persisted a mutable plan and optional
            // identity but no immutable revision chain. Wrap the active plan
            // once as revision zero so all new transitions have a stable
            // parent without inventing historical revisions.
            plan_identity.revision = 0;
            ledger.set_plan_identity(&plan_identity);
            let active_plan = match state.plan.as_ref() {
                Some(plan) => plan,
                None => unreachable!("legacy plan migration requires an active plan"),
            };
            let revision = match initial_plan_revision(
                active_plan,
                &plan_identity,
                "legacy_plan_migrated",
            ) {
                Ok(revision) => revision,
                Err(reason) => {
                    yield LoopItem::Complete {
                        reason: TerminationReason::Error,
                        output: Some(reason),
                    };
                    return;
                }
            };
            ledger.plan_lifecycle.push_revision(revision.clone());
            yield LoopItem::Event(StreamEvent::PlanCreated {
                plan: active_plan.clone(),
                identity: plan_identity.clone(),
                plan_revision: Some(Box::new(revision)),
            });
        } else {
            if let Some(active_revision) = ledger.plan_lifecycle.revisions.last() {
                plan_identity = active_revision.identity();
            }
            ledger.set_plan_identity(&plan_identity);
        }

        // An in-flight attempt from a previous process may have performed an
        // external side effect whose result was never persisted. Close that
        // attempt conservatively. The normal evaluator path below will record
        // the corresponding finish decision without replaying the step.
        if let Some(active_attempt) = ledger.active_step_attempt.take()
            && active_attempt.is_complete()
            && !has_terminal_record(&ledger, &active_attempt)
        {
            let record = interrupted_step_record(&active_attempt);
            ledger.step_records.push(record.clone());
            yield LoopItem::Event(StreamEvent::StepResult {
                record: Box::new(record),
            });
        }

        if let Some(active_plan) = state.plan.as_mut() {
            apply_persisted_continue_decisions(active_plan, &ledger, &plan_identity);
        }

        let mut final_output = ledger
            .step_records
            .iter()
            .rev()
            .find(|record| {
                record.plan_id == plan_identity.plan_id
                    && matches!(
                        record.status,
                        StepRecordStatus::Succeeded | StepRecordStatus::Skipped
                    )
            })
            .map(|record| record.summary.clone());

        while let Some(active_plan) = state.plan.as_mut() {

            // Resume may observe `step_result` after the task-state projection
            // already advanced the compatibility plan cursor. Select the
            // undecided fact from the ledger rather than only from
            // `TaskPlan::current_step`, then complete the transition exactly
            // once without replaying model or tool work.
            if let Some(record) = pending_undecided_record(&ledger, &plan_identity).cloned() {
                let has_remaining = has_remaining_steps_after_record(active_plan, &record.step_id);
                let decision = evaluate_step_record(&record, has_remaining);
                ledger.plan_lifecycle.push_decision(decision.clone());
                yield LoopItem::Event(StreamEvent::PlanDecision {
                    record: Box::new(decision.clone()),
                });

                let (compatibility_step, compatibility_index) =
                    compatibility_step(active_plan, &record);
                let compatibility_reason = record_reason(&record);
                match decision.decision.kind {
                    PlanDecisionKind::Continue => {
                        mark_step_done(active_plan, &record.step_id);
                        yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                            step: compatibility_step,
                            index: compatibility_index,
                        });
                        continue;
                    }
                    PlanDecisionKind::ReplaceRemaining => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: compatibility_step.clone(),
                            index: compatibility_index,
                            reason: compatibility_reason.clone(),
                        });
                        let (replacement, revision) = match replan_and_build_revision(
                            planner,
                            ctx.model,
                            active_plan,
                            compatibility_index,
                            &compatibility_step.title,
                            &compatibility_reason,
                            &record,
                            &decision,
                            &plan_identity,
                            &mut state.history,
                            cancel_token.clone(),
                        ).await {
                            Ok(transition) => transition,
                            Err(item) => {
                                yield item;
                                return;
                            }
                        };
                        plan_identity = revision.identity();
                        ledger.set_plan_identity(&plan_identity);
                        ledger.plan_lifecycle.push_revision(revision.clone());
                        yield LoopItem::Event(StreamEvent::PlanRevised {
                            plan: replacement.clone(),
                            revision: Box::new(revision),
                        });
                        *active_plan = replacement;
                        continue;
                    }
                    PlanDecisionKind::Finish => {
                        if matches!(
                            record.status,
                            StepRecordStatus::Succeeded | StepRecordStatus::Skipped
                        ) {
                            mark_step_done(active_plan, &record.step_id);
                            yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                                step: compatibility_step,
                                index: compatibility_index,
                            });
                        } else {
                            yield LoopItem::Event(StreamEvent::PlanStepFailed {
                                step: compatibility_step,
                                index: compatibility_index,
                                reason: compatibility_reason,
                            });
                        }
                        let (reason, output) = finish_transition(
                            &decision,
                            &record,
                            final_output.clone(),
                        );
                        yield LoopItem::Complete { reason, output };
                        return;
                    }
                }
            }

            // A crash after persisting a finish decision must not run another
            // step. Compatibility notifications are derived; the canonical
            // decision is sufficient to complete this resumed run.
            if let Some((record, decision)) = pending_finish_transition(&ledger, &plan_identity) {
                let (reason, output) = finish_transition(
                    decision,
                    record,
                    final_output.clone(),
                );
                yield LoopItem::Complete { reason, output };
                return;
            }

            // Replanner work starts only after the replace decision is durable.
            // If the process stopped at that stable boundary, resume the
            // missing child revision without re-running the failed step.
            if let Some((record, decision)) =
                pending_replace_transition(&ledger, &plan_identity)
                    .map(|(record, decision)| (record.clone(), decision.clone()))
            {
                let (step, index) = compatibility_step(active_plan, &record);
                let reason = record_reason(&record);
                let (replacement, revision) = match replan_and_build_revision(
                    planner,
                    ctx.model,
                    active_plan,
                    index,
                    &step.title,
                    &reason,
                    &record,
                    &decision,
                    &plan_identity,
                    &mut state.history,
                    cancel_token.clone(),
                ).await {
                    Ok(transition) => transition,
                    Err(item) => {
                        yield item;
                        return;
                    }
                };
                plan_identity = revision.identity();
                ledger.set_plan_identity(&plan_identity);
                ledger.plan_lifecycle.push_revision(revision.clone());
                yield LoopItem::Event(StreamEvent::PlanRevised {
                    plan: replacement.clone(),
                    revision: Box::new(revision),
                });
                *active_plan = replacement;
                continue;
            }

            if cancel_token.is_cancelled() {
                yield LoopItem::Complete {
                    reason: TerminationReason::Cancelled,
                    output: None,
                };
                return;
            }

            if active_plan.is_complete() {
                break;
            }

            if state.step >= ctx.max_steps {
                yield LoopItem::Complete {
                    reason: TerminationReason::StepLimit,
                    output: final_output,
                };
                return;
            }

            let Some(current_step) = active_plan.current_step().cloned() else {
                break;
            };
            let current_index = active_plan.current_step;

            if let Some(record) = latest_terminal_record(
                &ledger,
                &plan_identity,
                &current_step.id,
            ) {
                // Existing decisions are reconciled above. Reaching the same
                // terminal fact here means the compatibility cursor was stale;
                // advance it rather than replaying the attempt.
                if ledger
                    .plan_lifecycle
                    .decision_for_record(&record.record_id)
                    .is_some_and(|decision| {
                        decision.decision.kind == PlanDecisionKind::Continue
                    })
                {
                    mark_step_done(active_plan, &current_step.id);
                    continue;
                }
            }

            let attempt = StepAttempt {
                plan_id: plan_identity.plan_id.clone(),
                plan_revision_id: plan_identity.plan_revision_id.clone(),
                step_id: current_step.id.clone(),
                attempt: next_attempt(&ledger, &plan_identity, &current_step.id),
                started_at: chrono::Utc::now().to_rfc3339(),
            };
            ledger.active_step_attempt = Some(attempt.clone());
            yield LoopItem::Event(StreamEvent::PlanStepStarted {
                step: current_step.clone(),
                index: current_index,
                attempt: attempt.clone(),
            });

            state.step += 1;
            let runner_input = StepRunnerInput {
                goal: active_plan.goal.clone(),
                step: current_step.clone(),
                working_memory: state.working_memory.clone(),
                compact_summary: state.compact_summary.take(),
                history: std::mem::take(&mut state.history),
                compaction: compaction.clone(),
            };
            let mut runner = run_step(ctx.clone(), runner_input, cancel_token.clone());
            let runner_result = loop {
                match runner.next().await {
                    Some(StepRunnerItem::Event(event)) => yield LoopItem::Event(event),
                    Some(StepRunnerItem::Finished(result)) => break Some(result),
                    None => break None,
                }
            };
            let Some(runner_result) = runner_result else {
                let outcome = StepRunnerOutcome::Failed {
                    reason: "step runner ended without a result".to_string(),
                    replan: false,
                };
                let record = terminal_step_record(&attempt, &outcome, &StepRunMetrics::default());
                ledger.active_step_attempt = None;
                ledger.step_records.push(record.clone());
                yield LoopItem::Event(StreamEvent::StepResult {
                    record: Box::new(record),
                });
                continue;
            };

            state.history = runner_result.history;
            state.compact_summary = runner_result.compact_summary;
            compaction = runner_result.compaction;
            let record = terminal_step_record(
                &attempt,
                &runner_result.outcome,
                &runner_result.metrics,
            );
            if let StepRunnerOutcome::Succeeded { output } = &runner_result.outcome {
                final_output = Some(output.clone());
            }
            ledger.active_step_attempt = None;
            ledger.step_records.push(record.clone());
            yield LoopItem::Event(StreamEvent::StepResult {
                record: Box::new(record),
            });
        }

        yield LoopItem::Complete {
            reason: TerminationReason::Final,
            output: final_output,
        };
    })
}

fn initial_plan_revision(
    plan: &TaskPlan,
    identity: &PlanIdentity,
    reason_code: &str,
) -> Result<PlanRevision, String> {
    let revision = PlanRevision {
        plan_id: identity.plan_id.clone(),
        revision_id: identity.plan_revision_id.clone(),
        parent_revision_id: None,
        revision: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
        trigger_step_record_id: None,
        decision_id: ulid::Ulid::new().to_string(),
        safe_reason_codes: vec![reason_code.to_string()],
        retained_step_ids: Vec::new(),
        superseded_remaining_step_ids: Vec::new(),
        remaining_steps: plan
            .steps
            .iter()
            .filter(|step| !step.done)
            .cloned()
            .collect(),
        capability_snapshot_id: None,
        budget_snapshot: ExecutionBudgetUsage::default(),
    };
    revision
        .validate()
        .map_err(|err| format!("invalid initial plan revision: {err}"))?;
    Ok(revision)
}

#[allow(clippy::too_many_arguments)]
async fn replan_and_build_revision(
    planner: &Planner,
    model: &dyn crate::models::traits::ModelClient,
    active_plan: &TaskPlan,
    trigger_step_index: usize,
    trigger_step_title: &str,
    reason: &str,
    record: &StepRecord,
    decision: &PlanDecisionRecord,
    parent_identity: &PlanIdentity,
    history: &mut Vec<Message>,
    cancel_token: CancellationToken,
) -> Result<(TaskPlan, PlanRevision), LoopItem> {
    let replacement = replan_after_step_failure(
        planner,
        model,
        &active_plan.goal,
        trigger_step_title,
        reason,
        history,
        cancel_token,
    )
    .await?;
    let parent_remaining_ids: Vec<_> = active_plan
        .steps
        .iter()
        .skip(trigger_step_index.saturating_add(1))
        .filter(|step| !step.done)
        .map(|step| step.id.clone())
        .collect();
    let replacement_ids: std::collections::HashSet<_> = replacement
        .steps
        .iter()
        .filter(|step| !step.done)
        .map(|step| step.id.as_str())
        .collect();
    let retained_step_ids = parent_remaining_ids
        .iter()
        .filter(|step_id| replacement_ids.contains(step_id.as_str()))
        .cloned()
        .collect();
    let superseded_remaining_step_ids = parent_remaining_ids
        .into_iter()
        .filter(|step_id| !replacement_ids.contains(step_id.as_str()))
        .collect();
    let child_identity = parent_identity.next_revision();
    let revision = PlanRevision {
        plan_id: child_identity.plan_id.clone(),
        revision_id: child_identity.plan_revision_id.clone(),
        parent_revision_id: Some(parent_identity.plan_revision_id.clone()),
        revision: child_identity.revision,
        created_at: chrono::Utc::now().to_rfc3339(),
        trigger_step_record_id: Some(record.record_id.clone()),
        decision_id: decision.decision.decision_id.clone(),
        safe_reason_codes: decision.decision.safe_reason_codes.clone(),
        retained_step_ids,
        superseded_remaining_step_ids,
        remaining_steps: replacement
            .steps
            .iter()
            .filter(|step| !step.done)
            .cloned()
            .collect(),
        capability_snapshot_id: None,
        budget_snapshot: ExecutionBudgetUsage::default(),
    };
    revision.validate().map_err(|err| LoopItem::Complete {
        reason: TerminationReason::Error,
        output: Some(format!("invalid replacement plan revision: {err}")),
    })?;
    Ok((replacement, revision))
}

fn pending_undecided_record<'a>(
    ledger: &'a StepLedgerState,
    identity: &PlanIdentity,
) -> Option<&'a StepRecord> {
    ledger.step_records.iter().find(|record| {
        record.plan_id == identity.plan_id
            && record.plan_revision_id == identity.plan_revision_id
            && ledger
                .plan_lifecycle
                .decision_for_record(&record.record_id)
                .is_none()
    })
}

fn pending_finish_transition<'a>(
    ledger: &'a StepLedgerState,
    identity: &PlanIdentity,
) -> Option<(&'a StepRecord, &'a PlanDecisionRecord)> {
    ledger
        .plan_lifecycle
        .decisions
        .iter()
        .rev()
        .find_map(|decision| {
            (decision.decision.kind == PlanDecisionKind::Finish)
                .then(|| {
                    ledger.step_records.iter().find(|record| {
                        record.record_id == decision.trigger_step_record_id
                            && record.plan_id == identity.plan_id
                            && record.plan_revision_id == identity.plan_revision_id
                    })
                })
                .flatten()
                .map(|record| (record, decision))
        })
}

fn pending_replace_transition<'a>(
    ledger: &'a StepLedgerState,
    identity: &PlanIdentity,
) -> Option<(&'a StepRecord, &'a PlanDecisionRecord)> {
    ledger
        .plan_lifecycle
        .decisions
        .iter()
        .rev()
        .find_map(|decision| {
            (decision.decision.kind == PlanDecisionKind::ReplaceRemaining
                && ledger
                    .plan_lifecycle
                    .revision_for_trigger(&decision.trigger_step_record_id)
                    .is_none())
            .then(|| {
                ledger.step_records.iter().find(|record| {
                    record.record_id == decision.trigger_step_record_id
                        && record.plan_id == identity.plan_id
                        && record.plan_revision_id == identity.plan_revision_id
                })
            })
            .flatten()
            .map(|record| (record, decision))
        })
}

fn apply_persisted_continue_decisions(
    plan: &mut TaskPlan,
    ledger: &StepLedgerState,
    identity: &PlanIdentity,
) {
    let completed_step_ids: Vec<_> = ledger
        .plan_lifecycle
        .decisions
        .iter()
        .filter(|decision| decision.decision.kind == PlanDecisionKind::Continue)
        .filter_map(|decision| {
            ledger.step_records.iter().find(|record| {
                record.record_id == decision.trigger_step_record_id
                    && record.plan_id == identity.plan_id
                    && record.plan_revision_id == identity.plan_revision_id
                    && matches!(
                        record.status,
                        StepRecordStatus::Succeeded | StepRecordStatus::Skipped
                    )
            })
        })
        .map(|record| record.step_id.clone())
        .collect();
    for step_id in completed_step_ids {
        mark_step_done(plan, &step_id);
    }
}

fn has_remaining_steps_after_record(plan: &TaskPlan, step_id: &str) -> bool {
    plan.steps
        .iter()
        .position(|step| step.id == step_id)
        .is_some_and(|index| index.saturating_add(1) < plan.steps.len())
}

fn mark_step_done(plan: &mut TaskPlan, step_id: &str) {
    if let Some(step) = plan.steps.iter_mut().find(|step| step.id == step_id) {
        step.done = true;
    }
    plan.current_step = plan
        .steps
        .iter()
        .position(|step| !step.done)
        .unwrap_or(plan.steps.len());
}

fn compatibility_step(plan: &TaskPlan, record: &StepRecord) -> (PlanStep, usize) {
    plan.steps
        .iter()
        .enumerate()
        .find(|(_, step)| step.id == record.step_id)
        .map(|(index, step)| (step.clone(), index))
        .unwrap_or_else(|| {
            (
                PlanStep {
                    id: record.step_id.clone(),
                    title: format!("planned step {}", record.step_id),
                    done: false,
                },
                plan.current_step,
            )
        })
}

fn record_reason(record: &StepRecord) -> String {
    record
        .safe_error_summary
        .clone()
        .unwrap_or_else(|| record.summary.clone())
}

fn finish_transition(
    decision: &PlanDecisionRecord,
    record: &StepRecord,
    final_output: Option<String>,
) -> (TerminationReason, Option<String>) {
    let finish_reason = decision
        .decision
        .finish_reason
        .unwrap_or(PlanFinishReason::Failed);
    let safe_output = || Some(record_reason(record));
    match finish_reason {
        PlanFinishReason::Completed => (
            TerminationReason::Final,
            final_output.or_else(|| Some(record.summary.clone())),
        ),
        PlanFinishReason::BudgetExhausted => (
            if record.error_code.as_deref() == Some("context_token_limit") {
                TerminationReason::TokenLimit
            } else {
                TerminationReason::StepLimit
            },
            safe_output(),
        ),
        PlanFinishReason::Cancelled => (TerminationReason::Cancelled, None),
        PlanFinishReason::Partial
        | PlanFinishReason::Blocked
        | PlanFinishReason::Failed
        | PlanFinishReason::Interrupted => (TerminationReason::Error, safe_output()),
    }
}

fn next_attempt(ledger: &StepLedgerState, identity: &PlanIdentity, step_id: &str) -> u32 {
    ledger
        .step_records
        .iter()
        .filter(|record| {
            record.plan_id == identity.plan_id
                && record.plan_revision_id == identity.plan_revision_id
                && record.step_id == step_id
        })
        .map(|record| record.attempt)
        .max()
        .unwrap_or_default()
        .saturating_add(1)
}

fn latest_terminal_record<'a>(
    ledger: &'a StepLedgerState,
    identity: &PlanIdentity,
    step_id: &str,
) -> Option<&'a StepRecord> {
    ledger.step_records.iter().rev().find(|record| {
        record.plan_id == identity.plan_id
            && record.plan_revision_id == identity.plan_revision_id
            && record.step_id == step_id
    })
}

fn has_terminal_record(ledger: &StepLedgerState, attempt: &StepAttempt) -> bool {
    ledger.step_records.iter().any(|record| {
        record.plan_id == attempt.plan_id
            && record.plan_revision_id == attempt.plan_revision_id
            && record.step_id == attempt.step_id
            && record.attempt == attempt.attempt
    })
}

fn terminal_step_record(
    attempt: &StepAttempt,
    outcome: &StepRunnerOutcome,
    metrics: &StepRunMetrics,
) -> StepRecord {
    let (status, summary, completion_basis, error_code, safe_error_summary) = match outcome {
        StepRunnerOutcome::Succeeded { output } => (
            StepRecordStatus::Succeeded,
            bounded_step_summary(output, "Step completed."),
            StepCompletionBasis::ModelConclusion,
            None,
            None,
        ),
        StepRunnerOutcome::Failed { reason, replan } if !replan && is_permission_denied(reason) => {
            (
                StepRecordStatus::Blocked,
                "Step blocked because required tool permission was denied.".to_string(),
                StepCompletionBasis::RuntimeFailure,
                Some("permission_denied".to_string()),
                Some("Required tool permission was denied.".to_string()),
            )
        }
        StepRunnerOutcome::Failed { reason, replan } => (
            StepRecordStatus::Failed,
            if *replan {
                "Step failed; the runtime may replace the remaining plan.".to_string()
            } else {
                "Step failed and cannot continue safely.".to_string()
            },
            StepCompletionBasis::RuntimeFailure,
            Some(
                if *replan {
                    RECOVERABLE_STEP_FAILURE_CODE
                } else {
                    "step_runtime_failure"
                }
                .to_string(),
            ),
            Some(bounded_step_summary(
                reason,
                "The planned step ended with a runtime or model failure.",
            )),
        ),
        StepRunnerOutcome::BudgetExhausted { reason } => (
            StepRecordStatus::BudgetExhausted,
            "Step stopped after exhausting its model-turn budget.".to_string(),
            StepCompletionBasis::RuntimeFailure,
            Some("step_model_turn_budget_exhausted".to_string()),
            Some(bounded_step_summary(
                reason,
                "The configured per-step model-turn budget was exhausted.",
            )),
        ),
        StepRunnerOutcome::TokenLimit { reason } => (
            StepRecordStatus::BudgetExhausted,
            "Step stopped because the context token limit was exceeded.".to_string(),
            StepCompletionBasis::RuntimeFailure,
            Some("context_token_limit".to_string()),
            Some(bounded_step_summary(
                reason,
                "The configured context token limit was exceeded.",
            )),
        ),
        StepRunnerOutcome::Cancelled => (
            StepRecordStatus::Cancelled,
            "Step was cancelled before completion.".to_string(),
            StepCompletionBasis::UserDecision,
            Some("cancelled".to_string()),
            Some("The step was cancelled before completion.".to_string()),
        ),
    };

    let record = StepRecord {
        record_id: ulid::Ulid::new().to_string(),
        plan_id: attempt.plan_id.clone(),
        plan_revision_id: attempt.plan_revision_id.clone(),
        step_id: attempt.step_id.clone(),
        attempt: attempt.attempt,
        status,
        started_at: attempt.started_at.clone(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        summary,
        completion_basis,
        evidence_refs: metrics
            .tool_call_ids
            .iter()
            .map(|call_id| format!("tool_call:{call_id}"))
            .collect(),
        tool_call_ids: metrics.tool_call_ids.clone(),
        artifact_refs: Vec::new(),
        mutations: metrics.mutations.clone(),
        model_turns_used: metrics.model_turns_used,
        tool_calls_used: u32::try_from(metrics.tool_call_ids.len()).unwrap_or(u32::MAX),
        token_usage: metrics.token_usage.clone(),
        error_code,
        safe_error_summary,
        supersedes_record_id: None,
    };
    debug_assert!(record.validate().is_ok());
    record
}

fn interrupted_step_record(attempt: &StepAttempt) -> StepRecord {
    let record = StepRecord {
        record_id: ulid::Ulid::new().to_string(),
        plan_id: attempt.plan_id.clone(),
        plan_revision_id: attempt.plan_revision_id.clone(),
        step_id: attempt.step_id.clone(),
        attempt: attempt.attempt,
        status: StepRecordStatus::Interrupted,
        started_at: attempt.started_at.clone(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        summary: "Step attempt was interrupted; unknown side effects were not replayed."
            .to_string(),
        completion_basis: StepCompletionBasis::RuntimeFailure,
        evidence_refs: Vec::new(),
        tool_call_ids: Vec::new(),
        artifact_refs: Vec::new(),
        mutations: Vec::new(),
        model_turns_used: 0,
        tool_calls_used: 0,
        token_usage: Default::default(),
        error_code: Some("interrupted".to_string()),
        safe_error_summary: Some(
            "The process ended before the step attempt reached a terminal result.".to_string(),
        ),
        supersedes_record_id: None,
    };
    debug_assert!(record.validate().is_ok());
    record
}

fn bounded_step_summary(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return fallback.to_string();
    }
    let summary: String = value.chars().take(MAX_STEP_RECORD_SUMMARY_CHARS).collect();
    if value.chars().count() > MAX_STEP_RECORD_SUMMARY_CHARS {
        format!("{summary}...")
    } else {
        summary
    }
}

fn is_permission_denied(reason: &str) -> bool {
    reason.starts_with("Permission denied:")
}

async fn replan_after_step_failure(
    planner: &Planner,
    model: &dyn crate::models::traits::ModelClient,
    goal: &str,
    step_title: &str,
    reason: &str,
    history: &mut Vec<Message>,
    cancel_token: CancellationToken,
) -> Result<TaskPlan, LoopItem> {
    history.push(Message::user(planned_step_failure_message(
        step_title, reason,
    )));
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => Err(LoopItem::Complete {
            reason: TerminationReason::Cancelled,
            output: None,
        }),
        result = planner.draft(model, goal, history) => {
            result.map_err(|err| LoopItem::Complete {
                reason: TerminationReason::Error,
                output: Some(format!("Planner error: {err}")),
            })
        }
    }
}
