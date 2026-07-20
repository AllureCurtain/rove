use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::engine::planned_step_failure_message;
use crate::core::events::StreamEvent;
use crate::core::execution::{
    PlanIdentity, StepAttempt, StepCompletionBasis, StepLedgerState, StepRecord, StepRecordStatus,
};
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

        // An in-flight attempt from a previous process may have performed an
        // external side effect whose result was never persisted. Close that
        // attempt conservatively instead of replaying the step automatically.
        if let Some(active_attempt) = ledger.active_step_attempt.take()
            && active_attempt.is_complete()
            && !has_terminal_record(&ledger, &active_attempt)
        {
            let interrupted_step = state
                .plan
                .as_ref()
                .and_then(|plan| {
                    plan.steps
                        .iter()
                        .find(|step| step.id == active_attempt.step_id)
                })
                .cloned()
                .unwrap_or_else(|| PlanStep {
                    id: active_attempt.step_id.clone(),
                    title: format!("interrupted step {}", active_attempt.step_id),
                    done: false,
                });
            let record = interrupted_step_record(&active_attempt);
            ledger.step_records.push(record.clone());
            yield LoopItem::Event(StreamEvent::StepResult {
                record: Box::new(record),
            });
            let reason = "step attempt was interrupted before a terminal result was persisted"
                .to_string();
            yield LoopItem::Event(StreamEvent::PlanStepFailed {
                step: interrupted_step,
                index: state
                    .plan
                    .as_ref()
                    .map(|plan| plan.current_step)
                    .unwrap_or_default(),
                reason: reason.clone(),
            });
            yield LoopItem::Complete {
                reason: TerminationReason::Error,
                output: Some(reason),
            };
            return;
        }

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
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: drafted.clone(),
                        identity: plan_identity.clone(),
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
        } else {
            ledger.set_plan_identity(&plan_identity);
        }

        let mut final_output: Option<String> = None;
        while let Some(ref mut active_plan) = state.plan {
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

            // `step_result` is canonical and is persisted before the legacy
            // completion/failure notification. Resume must finish that
            // deterministic transition instead of replaying the attempt.
            if let Some(record) = latest_terminal_record(
                &ledger,
                &plan_identity,
                &current_step.id,
            ).cloned()
            {
                let reason = record
                    .safe_error_summary
                    .clone()
                    .unwrap_or_else(|| record.summary.clone());
                match record.status {
                    StepRecordStatus::Succeeded | StepRecordStatus::Skipped => {
                        active_plan.mark_current_done();
                        yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                            step: current_step,
                            index: current_index,
                        });
                        continue;
                    }
                    StepRecordStatus::Failed => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step.clone(),
                            index: current_index,
                            reason: reason.clone(),
                        });
                        let replacement = match replan_after_step_failure(
                            planner,
                            ctx.model,
                            &active_plan.goal,
                            &current_step.title,
                            &reason,
                            &mut state.history,
                            cancel_token.clone(),
                        ).await {
                            Ok(plan) => plan,
                            Err(item) => {
                                yield item;
                                return;
                            }
                        };
                        plan_identity = plan_identity.next_revision();
                        ledger.set_plan_identity(&plan_identity);
                        yield LoopItem::Event(StreamEvent::PlanCreated {
                            plan: replacement.clone(),
                            identity: plan_identity.clone(),
                        });
                        *active_plan = replacement;
                        continue;
                    }
                    StepRecordStatus::Blocked => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step,
                            index: current_index,
                            reason: reason.clone(),
                        });
                        yield LoopItem::Complete {
                            reason: TerminationReason::Final,
                            output: Some(reason),
                        };
                        return;
                    }
                    StepRecordStatus::BudgetExhausted => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step,
                            index: current_index,
                            reason: reason.clone(),
                        });
                        yield LoopItem::Complete {
                            reason: if record.error_code.as_deref() == Some("context_token_limit") {
                                TerminationReason::TokenLimit
                            } else {
                                TerminationReason::StepLimit
                            },
                            output: Some(reason),
                        };
                        return;
                    }
                    StepRecordStatus::Cancelled => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step,
                            index: current_index,
                            reason: reason.clone(),
                        });
                        yield LoopItem::Complete {
                            reason: TerminationReason::Cancelled,
                            output: None,
                        };
                        return;
                    }
                    StepRecordStatus::Partial | StepRecordStatus::Interrupted => {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step,
                            index: current_index,
                            reason: reason.clone(),
                        });
                        yield LoopItem::Complete {
                            reason: TerminationReason::Error,
                            output: Some(reason),
                        };
                        return;
                    }
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
                yield LoopItem::Event(StreamEvent::PlanStepFailed {
                    step: current_step,
                    index: current_index,
                    reason: "step runner ended without a result".to_string(),
                });
                yield LoopItem::Complete {
                    reason: TerminationReason::Error,
                    output: Some("step runner ended without a result".to_string()),
                };
                return;
            };

            state.history = runner_result.history;
            state.compact_summary = runner_result.compact_summary;
            compaction = runner_result.compaction;
            let record = terminal_step_record(
                &attempt,
                &runner_result.outcome,
                &runner_result.metrics,
            );
            ledger.active_step_attempt = None;
            ledger.step_records.push(record.clone());
            yield LoopItem::Event(StreamEvent::StepResult {
                record: Box::new(record),
            });

            match runner_result.outcome {
                StepRunnerOutcome::Succeeded { output } => {
                    final_output = Some(output);
                    active_plan.mark_current_done();
                    yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                        step: current_step,
                        index: current_index,
                    });
                }
                StepRunnerOutcome::Failed { reason, replan } => {
                    yield LoopItem::Event(StreamEvent::PlanStepFailed {
                        step: current_step.clone(),
                        index: current_index,
                        reason: reason.clone(),
                    });
                    if !replan {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Final,
                            output: Some(reason),
                        };
                        return;
                    }

                    let replacement = match replan_after_step_failure(
                        planner,
                        ctx.model,
                        &active_plan.goal,
                        &current_step.title,
                        &reason,
                        &mut state.history,
                        cancel_token.clone(),
                    ).await {
                        Ok(plan) => plan,
                        Err(item) => {
                            yield item;
                            return;
                        }
                    };
                    plan_identity = plan_identity.next_revision();
                    ledger.set_plan_identity(&plan_identity);
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: replacement.clone(),
                        identity: plan_identity.clone(),
                    });
                    *active_plan = replacement;
                }
                StepRunnerOutcome::BudgetExhausted { reason } => {
                    yield LoopItem::Event(StreamEvent::PlanStepFailed {
                        step: current_step,
                        index: current_index,
                        reason: reason.clone(),
                    });
                    yield LoopItem::Complete {
                        reason: TerminationReason::StepLimit,
                        output: Some(reason),
                    };
                    return;
                }
                StepRunnerOutcome::TokenLimit { reason } => {
                    yield LoopItem::Event(StreamEvent::PlanStepFailed {
                        step: current_step,
                        index: current_index,
                        reason: reason.clone(),
                    });
                    yield LoopItem::Complete {
                        reason: TerminationReason::TokenLimit,
                        output: Some(reason),
                    };
                    return;
                }
                StepRunnerOutcome::Cancelled => {
                    yield LoopItem::Event(StreamEvent::PlanStepFailed {
                        step: current_step,
                        index: current_index,
                        reason: "step cancelled".to_string(),
                    });
                    yield LoopItem::Complete {
                        reason: TerminationReason::Cancelled,
                        output: None,
                    };
                    return;
                }
            }
        }

        yield LoopItem::Complete {
            reason: TerminationReason::Final,
            output: final_output,
        };
    })
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
        StepRunnerOutcome::Failed { replan, .. } => (
            StepRecordStatus::Failed,
            if *replan {
                "Step failed; the runtime may replace the remaining plan.".to_string()
            } else {
                "Step failed and cannot continue safely.".to_string()
            },
            StepCompletionBasis::RuntimeFailure,
            Some("step_runtime_failure".to_string()),
            Some("The planned step ended with a runtime or model failure.".to_string()),
        ),
        StepRunnerOutcome::BudgetExhausted { .. } => (
            StepRecordStatus::BudgetExhausted,
            "Step stopped after exhausting its model-turn budget.".to_string(),
            StepCompletionBasis::RuntimeFailure,
            Some("step_model_turn_budget_exhausted".to_string()),
            Some("The configured per-step model-turn budget was exhausted.".to_string()),
        ),
        StepRunnerOutcome::TokenLimit { .. } => (
            StepRecordStatus::BudgetExhausted,
            "Step stopped because the context token limit was exceeded.".to_string(),
            StepCompletionBasis::RuntimeFailure,
            Some("context_token_limit".to_string()),
            Some("The configured context token limit was exceeded.".to_string()),
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
