use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::events::StreamEvent;
use crate::execution::{
    DEFAULT_MAX_MODEL_REPAIRS, EvaluatorMode, ExecutionBudgetDimension, ExecutionBudgetTracker,
    ExecutionBudgetUsage, ExecutionDegradation, ExecutionLifecycleState, ExecutionPhase,
    FinalizationMode, FinalizerPolicy, PlanAmbiguity, PlanAmbiguityKind, PlanDecisionKind,
    PlanDecisionRecord, PlanFinishReason, PlanIdentity, PlanRevision, StepAttempt,
    StepCompletionBasis, StepLedgerState, StepRecord, StepRecordStatus,
    planned_step_failure_message,
};
use crate::finalizer::{FinalizationContext, Finalizer};
use crate::plan_evaluator::{
    ModelEvaluationContext, PlanEvaluator, RECOVERABLE_STEP_FAILURE_CODE, RuleEvaluation,
    deterministic_evaluation, evaluation_key,
};
use crate::planner::{Planner, PlannerContext};
use crate::run_loop::{LoopContext, LoopItem};
use crate::step_runner::{
    StepRunMetrics, StepRunnerInput, StepRunnerItem, StepRunnerOutcome, run_step,
};
use crate::types::{Message, PlanStep, TaskPlan, TerminationReason, Usage};

/// Step error code for a context hard-limit refusal. This is not a tracked
/// budget dimension, so the terminal compatibility reason keys off it directly.
const CONTEXT_TOKEN_LIMIT_CODE: &str = "context_token_limit";

const MAX_STEP_RECORD_SUMMARY_CHARS: usize = 1_000;

pub(crate) struct PlanLoopState {
    pub user_message: String,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub plan: Option<TaskPlan>,
    pub step_ledger: StepLedgerState,
    pub execution_lifecycle: ExecutionLifecycleState,
}

pub(crate) fn run_planned_loop<'a>(
    ctx: LoopContext<'a>,
    planner: &'a Planner,
    evaluator: &'a PlanEvaluator,
    finalizer: &'a Finalizer,
    mut state: PlanLoopState,
    cancel_token: CancellationToken,
) -> BoxStream<'a, LoopItem> {
    Box::pin(stream! {
        let mut compaction = ctx.compaction.clone();
        let mut ledger = std::mem::take(&mut state.step_ledger);
        let mut budget = ExecutionBudgetTracker::new(
            ctx.execution_policy.budgets.clone(),
            state.execution_lifecycle.budget_usage.clone(),
            false,
        );
        let mut plan_identity = ledger.plan_identity().unwrap_or_else(PlanIdentity::fresh);
        let capability_summary = ctx.capability_snapshot.planner_summary();
        let planner_context = PlannerContext {
            capability_snapshot_summary: Some(&capability_summary),
        };

        macro_rules! finish_planned {
            ($finish_reason:expr, $direct_output:expr) => {{
                let finish_reason = $finish_reason;
                let direct_output: Option<String> = $direct_output;
                let revisions = ledger.plan_lifecycle.revisions.clone();
                let records = ledger.step_records.clone();
                let transition = finalize_planned_run(
                    LifecycleContext {
                        model: ctx.model,
                        policy: &ctx.execution_policy,
                        original_goal: &state.user_message,
                        cancel: cancel_token.clone(),
                    },
                    finalizer,
                    FinalizationEvidence {
                        finish_reason,
                        revisions: &revisions,
                        records: &records,
                        direct_output: direct_output.as_deref(),
                    },
                    &mut budget,
                )
                .await;
                for event in transition.events {
                    yield LoopItem::Event(event);
                }
                yield LoopItem::Complete {
                    reason: transition.reason,
                    output: transition.output,
                };
                return;
            }};
        }

        if state.plan.is_none() {
            if let Err(exhaustion) = budget.reserve_model_turn(ExecutionPhase::Planner) {
                budget.mark_exhausted(exhaustion);
                yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Planner,
                    snapshot: Box::new(budget.snapshot()),
                });
                finish_planned!(PlanFinishReason::BudgetExhausted, None);
            }
            let draft_result = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    finish_planned!(PlanFinishReason::Cancelled, None);
                }
                result = planner.draft_accounted(
                    ctx.model,
                    &state.user_message,
                    &state.history,
                    planner_context,
                ) => result,
            };
            match draft_result {
                Ok(draft) => {
                    if let Err(exhaustion) = budget.record_tokens(
                        &draft.usage,
                        ExecutionPhase::Planner,
                    ) {
                        budget.mark_exhausted(exhaustion);
                        yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                            phase: ExecutionPhase::Planner,
                            snapshot: Box::new(budget.snapshot()),
                        });
                        finish_planned!(PlanFinishReason::BudgetExhausted, None);
                    }
                    let drafted = draft.plan;
                    if let Err(exhaustion) = budget.validate_plan_steps(
                        drafted.steps.len(),
                        ExecutionPhase::Planner,
                    ) {
                        budget.mark_exhausted(exhaustion);
                        yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                            phase: ExecutionPhase::Planner,
                            snapshot: Box::new(budget.snapshot()),
                        });
                        finish_planned!(PlanFinishReason::BudgetExhausted, None);
                    }
                    plan_identity = PlanIdentity::fresh();
                    ledger.set_plan_identity(&plan_identity);
                    let revision = match initial_plan_revision(
                        &drafted,
                        &plan_identity,
                        "planner_draft",
                        &ctx.capability_snapshot.snapshot_id,
                        budget.usage(),
                    ) {
                        Ok(revision) => revision,
                        Err(reason) => {
                            finish_planned!(PlanFinishReason::Failed, Some(reason));
                        }
                    };
                    ledger.plan_lifecycle.push_revision(revision.clone());
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: drafted.clone(),
                        identity: plan_identity.clone(),
                        plan_revision: Some(Box::new(revision)),
                    });
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Planner,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    state.plan = Some(drafted);
                }
                Err(err) => {
                    finish_planned!(
                        PlanFinishReason::Failed,
                        Some(format!("Planner error: {err}"))
                    );
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
                &ctx.capability_snapshot.snapshot_id,
                budget.usage(),
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

        if let Some(active_plan) = state.plan.as_ref()
            && let Err(exhaustion) =
                budget.validate_plan_steps(active_plan.steps.len(), ExecutionPhase::Planner)
        {
            budget.mark_exhausted(exhaustion);
            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                phase: ExecutionPhase::Planner,
                snapshot: Box::new(budget.snapshot()),
            });
            finish_planned!(PlanFinishReason::BudgetExhausted, None);
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
                let remaining_steps = remaining_steps_after_record(active_plan, &record.step_id);
                let active_revision = ledger
                    .plan_lifecycle
                    .revisions
                    .last()
                    .cloned()
                    .expect("planned execution has an active revision");
                let evaluation = evaluate_record_transition(
                    LifecycleContext {
                        model: ctx.model,
                        policy: &ctx.execution_policy,
                        original_goal: &state.user_message,
                        cancel: cancel_token.clone(),
                    },
                    evaluator,
                    EvaluationEvidence {
                        revision: &active_revision,
                        record: &record,
                        remaining_steps: &remaining_steps,
                        capability_summary: &capability_summary,
                        ledger: &ledger,
                    },
                    &mut budget,
                )
                .await;
                for event in evaluation.events {
                    yield LoopItem::Event(event);
                }
                let decision = evaluation.decision;
                ledger.plan_lifecycle.push_decision(decision.clone());
                yield LoopItem::Event(StreamEvent::PlanDecision {
                    record: Box::new(decision.clone()),
                });

                let (step, step_index) = compatibility_step(active_plan, &record);
                let reason = record_reason(&record);
                match decision.decision.kind {
                    PlanDecisionKind::Continue => {
                        mark_step_done(active_plan, &record.step_id);
                        continue;
                    }
                    PlanDecisionKind::ReplaceRemaining => {
                        if let Err(exhaustion) = budget.reserve_plan_revision() {
                            budget.mark_exhausted(exhaustion);
                            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                                phase: ExecutionPhase::Replanner,
                                snapshot: Box::new(budget.snapshot()),
                            });
                            finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                        }
                        if let Err(exhaustion) = budget.reserve_model_turn(ExecutionPhase::Replanner) {
                            budget.mark_exhausted(exhaustion);
                            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                                phase: ExecutionPhase::Replanner,
                                snapshot: Box::new(budget.snapshot()),
                            });
                            finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                        }
                        let (replacement, mut revision, usage) = match replan_and_build_revision(
                            planner,
                            ctx.model,
                            active_plan,
                            step_index,
                            &step.title,
                            &reason,
                            &record,
                            &decision,
                            &plan_identity,
                            &ctx.capability_snapshot.snapshot_id,
                            planner_context,
                            &mut state.history,
                            cancel_token.clone(),
                        )
                        .await
                        {
                            Ok(transition) => transition,
                            Err(ReplanError::Cancelled) => {
                                finish_planned!(PlanFinishReason::Cancelled, final_output.clone());
                            }
                            Err(ReplanError::Failed(reason)) => {
                                finish_planned!(PlanFinishReason::Partial, Some(reason));
                            }
                        };
                        if let Err(exhaustion) = budget.record_tokens(&usage, ExecutionPhase::Replanner) {
                            budget.mark_exhausted(exhaustion);
                            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                                phase: ExecutionPhase::Replanner,
                                snapshot: Box::new(budget.snapshot()),
                            });
                            finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                        }
                        if let Err(exhaustion) = budget.validate_plan_steps(
                            replacement.steps.len(),
                            ExecutionPhase::Replanner,
                        ) {
                            budget.mark_exhausted(exhaustion);
                            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                                phase: ExecutionPhase::Replanner,
                                snapshot: Box::new(budget.snapshot()),
                            });
                            finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                        }
                        revision.budget_snapshot = budget.usage().clone();
                        plan_identity = revision.identity();
                        ledger.set_plan_identity(&plan_identity);
                        ledger.plan_lifecycle.push_revision(revision.clone());
                        yield LoopItem::Event(StreamEvent::PlanRevised {
                            plan: replacement.clone(),
                            revision: Box::new(revision),
                        });
                        yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                            phase: ExecutionPhase::Replanner,
                            snapshot: Box::new(budget.snapshot()),
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
                        }
                        finish_planned!(
                            decision
                                .decision
                                .finish_reason
                                .unwrap_or(PlanFinishReason::Failed),
                            final_output.clone()
                        );
                    }
                }
            }

            // A crash after persisting a finish decision must not run another
            // step. Compatibility notifications are derived; the canonical
            // decision is sufficient to complete this resumed run.
            if let Some((record, decision)) = pending_finish_transition(&ledger, &plan_identity) {
                let finish_reason = decision
                    .decision
                    .finish_reason
                    .unwrap_or(PlanFinishReason::Failed);
                let direct = final_output
                    .clone()
                    .or_else(|| Some(record_reason(record)));
                finish_planned!(finish_reason, direct);
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
                if let Err(exhaustion) = budget.reserve_plan_revision() {
                    budget.mark_exhausted(exhaustion);
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Replanner,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                }
                if let Err(exhaustion) = budget.reserve_model_turn(ExecutionPhase::Replanner) {
                    budget.mark_exhausted(exhaustion);
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Replanner,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                }
                let (replacement, mut revision, usage) = match replan_and_build_revision(
                    planner,
                    ctx.model,
                    active_plan,
                    index,
                    &step.title,
                    &reason,
                    &record,
                    &decision,
                    &plan_identity,
                    &ctx.capability_snapshot.snapshot_id,
                    planner_context,
                    &mut state.history,
                    cancel_token.clone(),
                ).await {
                    Ok(transition) => transition,
                    Err(ReplanError::Cancelled) => {
                        finish_planned!(PlanFinishReason::Cancelled, final_output.clone());
                    }
                    Err(ReplanError::Failed(reason)) => {
                        finish_planned!(PlanFinishReason::Partial, Some(reason));
                    }
                };
                if let Err(exhaustion) = budget.record_tokens(&usage, ExecutionPhase::Replanner) {
                    budget.mark_exhausted(exhaustion);
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Replanner,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                }
                if let Err(exhaustion) = budget.validate_plan_steps(
                    replacement.steps.len(),
                    ExecutionPhase::Replanner,
                ) {
                    budget.mark_exhausted(exhaustion);
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Replanner,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                }
                revision.budget_snapshot = budget.usage().clone();
                plan_identity = revision.identity();
                ledger.set_plan_identity(&plan_identity);
                ledger.plan_lifecycle.push_revision(revision.clone());
                yield LoopItem::Event(StreamEvent::PlanRevised {
                    plan: replacement.clone(),
                    revision: Box::new(revision),
                });
                yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Replanner,
                    snapshot: Box::new(budget.snapshot()),
                });
                *active_plan = replacement;
                continue;
            }

            if cancel_token.is_cancelled() {
                if let Some(rx) = ctx.steer_rx.as_ref() {
                    let mut r = rx.lock().await;
                    while let Ok(msg) = r.try_recv() {
                        yield LoopItem::Event(StreamEvent::SteerDropped {
                            id: msg.id.0,
                            reason: "cancelled".to_string(),
                        });
                    }
                }
                finish_planned!(PlanFinishReason::Cancelled, None);
            }

            // SAFE POINT — drain queued steers at the step boundary, before the
            // next step's prompt is assembled. Mirrors the drain in the
            // unplanned loop.
            let mut accepted_steer_ids = Vec::new();
            if let Some(rx) = ctx.steer_rx.as_ref() {
                let mut r = rx.lock().await;
                while let Ok(msg) = r.try_recv() {
                    let id = msg.id.0;
                    state.working_memory.push(Message::user(msg.content.clone()));
                    if let Some(lifecycle) = ctx.steer_lifecycle.as_ref() {
                        lifecycle.accepted(id.clone()).await;
                    }
                    yield LoopItem::Event(StreamEvent::SteerAccepted {
                        id: id.clone(),
                        content: msg.content,
                    });
                    accepted_steer_ids.push(id);
                }
            }

            if active_plan.is_complete() {
                break;
            }

            if let Err(exhaustion) = budget.refresh_wall_time(ExecutionPhase::Step) {
                budget.mark_exhausted(exhaustion);
                yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Step,
                    snapshot: Box::new(budget.snapshot()),
                });
                finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
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

            if let Err(exhaustion) = budget.reserve_step_attempt() {
                budget.mark_exhausted(exhaustion);
                yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Step,
                    snapshot: Box::new(budget.snapshot()),
                });
                finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
            }
            let max_model_turns = match budget.remaining_model_turns_for_step() {
                Ok(limit) => limit,
                Err(exhaustion) => {
                    budget.mark_exhausted(exhaustion);
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Step,
                        snapshot: Box::new(budget.snapshot()),
                    });
                    finish_planned!(PlanFinishReason::BudgetExhausted, final_output.clone());
                }
            };
            let max_tool_calls = budget
                .limits()
                .max_tool_calls_per_step
                .unwrap_or(u32::MAX)
                .min(
                    budget
                        .limits()
                        .max_tool_calls
                        .map(|limit| limit.saturating_sub(budget.usage().tool_calls))
                        .unwrap_or(u32::MAX),
                );
            let max_repairs = budget
                .limits()
                .max_model_repairs
                .map(|limit| limit.saturating_sub(budget.usage().model_repairs))
                .unwrap_or(DEFAULT_MAX_MODEL_REPAIRS);
            let max_total_tokens = budget.remaining_tokens();

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
                budget: Box::new(budget.snapshot()),
            });

            let runner_input = StepRunnerInput {
                goal: active_plan.goal.clone(),
                step: current_step.clone(),
                working_memory: state.working_memory.clone(),
                compact_summary: state.compact_summary.take(),
                history: std::mem::take(&mut state.history),
                compaction: compaction.clone(),
                accepted_steer_ids,
                max_model_turns,
                max_tool_calls,
                max_repairs,
                max_total_tokens,
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
            if let Err(exhaustion) = budget.record_step_usage(
                runner_result.metrics.model_turns_used,
                u32::try_from(runner_result.metrics.tool_call_ids.len()).unwrap_or(u32::MAX),
                runner_result.metrics.repairs_used,
                &runner_result.metrics.token_usage,
            ) {
                budget.mark_exhausted(exhaustion);
            }
            yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                phase: ExecutionPhase::Step,
                snapshot: Box::new(budget.snapshot()),
            });
        }

        finish_planned!(PlanFinishReason::Completed, final_output);
    })
}

struct EvaluationTransition {
    decision: PlanDecisionRecord,
    events: Vec<StreamEvent>,
}

/// Collaborators that stay fixed for the whole planned run.
struct LifecycleContext<'a> {
    model: &'a dyn rove_models::ModelClient,
    policy: &'a crate::execution::ExecutionPolicy,
    original_goal: &'a str,
    cancel: CancellationToken,
}

/// The recorded evidence one evaluator decision is made from.
struct EvaluationEvidence<'a> {
    revision: &'a PlanRevision,
    record: &'a StepRecord,
    remaining_steps: &'a [PlanStep],
    capability_summary: &'a str,
    ledger: &'a StepLedgerState,
}

async fn evaluate_record_transition(
    lifecycle: LifecycleContext<'_>,
    evaluator: &PlanEvaluator,
    evidence: EvaluationEvidence<'_>,
    budget: &mut ExecutionBudgetTracker,
) -> EvaluationTransition {
    let LifecycleContext {
        model,
        policy,
        original_goal,
        cancel,
    } = lifecycle;
    let EvaluationEvidence {
        revision,
        record,
        remaining_steps,
        capability_summary,
        ledger,
    } = evidence;
    let (classification, fallback) = deterministic_evaluation(record, !remaining_steps.is_empty());
    if classification == RuleEvaluation::Decided || policy.evaluator_mode == EvaluatorMode::RuleOnly
    {
        return EvaluationTransition {
            decision: fallback,
            events: Vec::new(),
        };
    }

    let key = evaluation_key(revision, record, remaining_steps);
    if ledger
        .plan_lifecycle
        .decisions
        .iter()
        .any(|saved| saved.evaluation_key.as_deref() == Some(key.as_str()))
    {
        return EvaluationTransition {
            decision: fallback,
            events: vec![StreamEvent::ExecutionDegraded {
                record: degradation(
                    ExecutionPhase::Evaluator,
                    "evaluator_duplicate_suppressed",
                    "An identical ambiguity evaluation was not repeated.",
                ),
            }],
        };
    }

    let mut events = Vec::new();
    let mut repair_error: Option<String> = None;
    loop {
        if let Err(exhaustion) = budget.reserve_model_turn(ExecutionPhase::Evaluator) {
            budget.mark_exhausted(exhaustion);
            events.push(StreamEvent::ExecutionBudgetUpdated {
                phase: ExecutionPhase::Evaluator,
                snapshot: Box::new(budget.snapshot()),
            });
            events.push(StreamEvent::ExecutionDegraded {
                record: degradation(
                    ExecutionPhase::Evaluator,
                    "evaluator_budget_fallback",
                    "Model evaluation was skipped because its execution budget was unavailable.",
                ),
            });
            return EvaluationTransition {
                decision: fallback,
                events,
            };
        }
        let budget_snapshot = budget.snapshot();
        let context = ModelEvaluationContext {
            original_goal,
            revision,
            record,
            remaining_steps,
            capability_snapshot_summary: capability_summary,
            budget: &budget_snapshot,
            repair_error: repair_error.as_deref(),
        };
        match evaluator
            .evaluate_model(model, context, cancel.clone())
            .await
        {
            Ok(model_evaluation) => {
                if let Err(exhaustion) =
                    budget.record_tokens(&model_evaluation.usage, ExecutionPhase::Evaluator)
                {
                    budget.mark_exhausted(exhaustion);
                }
                events.push(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Evaluator,
                    snapshot: Box::new(budget.snapshot()),
                });
                return EvaluationTransition {
                    decision: model_evaluation.record,
                    events,
                };
            }
            Err(error) => {
                let can_repair = repair_error.is_none();
                if can_repair && budget.reserve_repair(ExecutionPhase::Evaluator).is_ok() {
                    repair_error = Some(error.to_string());
                    continue;
                }
                events.push(StreamEvent::ExecutionDegraded {
                    record: degradation(
                        ExecutionPhase::Evaluator,
                        "evaluator_safe_fallback",
                        &format!(
                            "Model evaluation was unavailable; deterministic safe fallback was used: {}",
                            bounded_step_summary(&error.to_string(), "evaluation failed")
                        ),
                    ),
                });
                events.push(StreamEvent::ExecutionBudgetUpdated {
                    phase: ExecutionPhase::Evaluator,
                    snapshot: Box::new(budget.snapshot()),
                });
                return EvaluationTransition {
                    decision: fallback,
                    events,
                };
            }
        }
    }
}

struct PlannedFinalization {
    events: Vec<StreamEvent>,
    reason: TerminationReason,
    output: Option<String>,
}

/// The recorded evidence the finalizer may ground its answer in.
struct FinalizationEvidence<'a> {
    finish_reason: PlanFinishReason,
    revisions: &'a [PlanRevision],
    records: &'a [StepRecord],
    direct_output: Option<&'a str>,
}

async fn finalize_planned_run(
    lifecycle: LifecycleContext<'_>,
    finalizer: &Finalizer,
    evidence: FinalizationEvidence<'_>,
    budget: &mut ExecutionBudgetTracker,
) -> PlannedFinalization {
    let LifecycleContext {
        model,
        policy,
        original_goal,
        cancel,
    } = lifecycle;
    let FinalizationEvidence {
        finish_reason,
        revisions,
        records,
        direct_output,
    } = evidence;
    let preferred_mode = if policy.finalizer_policy == FinalizerPolicy::ModelPreferred {
        FinalizationMode::Model
    } else {
        FinalizationMode::Deterministic
    };
    let budget_before = budget.usage().clone();
    let context = FinalizationContext {
        original_goal,
        strategy: policy.strategy,
        finish_reason,
        revisions,
        records,
        budget: &budget_before,
        direct_output,
    };
    let started = finalizer.started_record(&context, preferred_mode);
    let mut events = vec![StreamEvent::FinalizationStarted {
        record: Box::new(started.clone()),
    }];

    let mut result = None;
    if preferred_mode == FinalizationMode::Model {
        match budget.reserve_model_turn(ExecutionPhase::Finalizer) {
            Ok(()) => {
                let budget_after_reservation = budget.usage().clone();
                match finalizer
                    .model(
                        model,
                        &context,
                        started.clone(),
                        budget_after_reservation,
                        cancel,
                    )
                    .await
                {
                    Ok(mut finalized) => {
                        if let Err(exhaustion) =
                            budget.record_tokens(&finalized.usage, ExecutionPhase::Finalizer)
                        {
                            budget.mark_exhausted(exhaustion);
                        }
                        finalized.record.budget_after = budget.usage().clone();
                        result = Some(finalized);
                    }
                    Err(error) => events.push(StreamEvent::ExecutionDegraded {
                        record: degradation(
                            ExecutionPhase::Finalizer,
                            "finalizer_model_fallback",
                            &format!(
                                "Model finalization was unavailable; deterministic synthesis was used: {}",
                                bounded_step_summary(&error.to_string(), "finalizer failed")
                            ),
                        ),
                    }),
                }
            }
            Err(exhaustion) => {
                budget.mark_exhausted(exhaustion);
                events.push(StreamEvent::ExecutionDegraded {
                    record: degradation(
                        ExecutionPhase::Finalizer,
                        "finalizer_budget_fallback",
                        "Model finalization had no reserved budget; deterministic synthesis was used.",
                    ),
                });
            }
        }
    }

    let finalized = result.unwrap_or_else(|| {
        finalizer.deterministic(
            &context,
            started,
            preferred_mode == FinalizationMode::Model,
            budget.usage().clone(),
        )
    });
    events.push(StreamEvent::ExecutionBudgetUpdated {
        phase: ExecutionPhase::Finalizer,
        snapshot: Box::new(budget.snapshot()),
    });
    let output = finalized.record.output.clone();
    events.push(StreamEvent::FinalizationCompleted {
        record: Box::new(finalized.record),
    });

    let reason = match finish_reason {
        PlanFinishReason::Completed => TerminationReason::Final,
        PlanFinishReason::BudgetExhausted => {
            let exhausted_dimension = budget
                .snapshot()
                .exhausted
                .as_ref()
                .map(|exhaustion| exhaustion.dimension);
            // A context hard-limit refusal is a token boundary even though it is
            // not one of the tracked budget dimensions, so the compatibility
            // reason must not degrade to the generic step limit.
            let context_token_limited = records.last().is_some_and(|record| {
                record.error_code.as_deref() == Some(CONTEXT_TOKEN_LIMIT_CODE)
            });
            match exhausted_dimension {
                Some(ExecutionBudgetDimension::TotalTokens) => TerminationReason::TokenLimit,
                Some(ExecutionBudgetDimension::WallTime) => TerminationReason::TimeLimit,
                _ if context_token_limited => TerminationReason::TokenLimit,
                _ => TerminationReason::StepLimit,
            }
        }
        PlanFinishReason::Cancelled => TerminationReason::Cancelled,
        PlanFinishReason::Partial
        | PlanFinishReason::Blocked
        | PlanFinishReason::Rejected
        | PlanFinishReason::Failed
        | PlanFinishReason::Interrupted
        | PlanFinishReason::Indeterminate => TerminationReason::Error,
    };
    PlannedFinalization {
        events,
        reason,
        output,
    }
}

fn degradation(phase: ExecutionPhase, code: &str, safe_summary: &str) -> ExecutionDegradation {
    ExecutionDegradation {
        degradation_id: ulid::Ulid::new().to_string(),
        phase,
        code: code.to_string(),
        safe_summary: bounded_step_summary(safe_summary, "Lifecycle degradation occurred."),
        occurred_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn initial_plan_revision(
    plan: &TaskPlan,
    identity: &PlanIdentity,
    reason_code: &str,
    capability_snapshot_id: &str,
    budget_usage: &ExecutionBudgetUsage,
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
        capability_snapshot_id: Some(capability_snapshot_id.to_string()),
        budget_snapshot: budget_usage.clone(),
    };
    revision
        .validate()
        .map_err(|err| format!("invalid initial plan revision: {err}"))?;
    Ok(revision)
}

#[allow(clippy::too_many_arguments)]
async fn replan_and_build_revision(
    planner: &Planner,
    model: &dyn rove_models::ModelClient,
    active_plan: &TaskPlan,
    trigger_step_index: usize,
    trigger_step_title: &str,
    reason: &str,
    record: &StepRecord,
    decision: &PlanDecisionRecord,
    parent_identity: &PlanIdentity,
    capability_snapshot_id: &str,
    planner_context: PlannerContext<'_>,
    history: &mut Vec<Message>,
    cancel_token: CancellationToken,
) -> Result<(TaskPlan, PlanRevision, Usage), ReplanError> {
    history.push(Message::user(planned_step_failure_message(
        trigger_step_title,
        reason,
    )));
    let replacement = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => Err(ReplanError::Cancelled),
        result = planner.draft_accounted(
            model,
            &active_plan.goal,
            history,
            planner_context,
        ) => {
            result.map_err(|err| ReplanError::Failed(format!("Planner error: {err}")))
        }
    }?;
    let usage = replacement.usage;
    let replacement = replacement.plan;
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
    let parent_remaining: Vec<_> = active_plan
        .steps
        .iter()
        .skip(trigger_step_index.saturating_add(1))
        .filter(|step| !step.done)
        .map(|step| (&step.id, &step.title))
        .collect();
    let replacement_remaining: Vec<_> = replacement
        .steps
        .iter()
        .filter(|step| !step.done)
        .map(|step| (&step.id, &step.title))
        .collect();
    if parent_remaining == replacement_remaining {
        return Err(ReplanError::Failed(
            "replacement plan did not change the remaining work".to_string(),
        ));
    }
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
        capability_snapshot_id: Some(capability_snapshot_id.to_string()),
        budget_snapshot: ExecutionBudgetUsage::default(),
    };
    revision
        .validate()
        .map_err(|err| ReplanError::Failed(format!("invalid replacement plan revision: {err}")))?;
    Ok((replacement, revision, usage))
}

enum ReplanError {
    Cancelled,
    Failed(String),
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

fn remaining_steps_after_record(plan: &TaskPlan, step_id: &str) -> Vec<PlanStep> {
    plan.steps
        .iter()
        .position(|step| step.id == step_id)
        .map(|index| {
            plan.steps
                .iter()
                .skip(index.saturating_add(1))
                .filter(|step| !step.done)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
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
    let (status, summary, completion_basis, error_code, safe_error_summary, ambiguity) =
        match outcome {
            StepRunnerOutcome::Succeeded { output } => {
                let (summary, ambiguity) = parse_structured_step_conclusion(output);
                (
                    StepRecordStatus::Succeeded,
                    summary,
                    StepCompletionBasis::ModelConclusion,
                    None,
                    None,
                    ambiguity,
                )
            }
            StepRunnerOutcome::Failed { reason, replan }
                if !replan && is_permission_denied(reason) =>
            {
                let rejected = reason.to_ascii_lowercase().contains("reject");
                (
                    if rejected {
                        StepRecordStatus::Rejected
                    } else {
                        StepRecordStatus::Blocked
                    },
                    if rejected {
                        "Step stopped because required tool approval was rejected.".to_string()
                    } else {
                        "Step blocked because required tool permission was denied.".to_string()
                    },
                    StepCompletionBasis::RuntimeFailure,
                    Some(if rejected {
                        "approval_rejected".to_string()
                    } else {
                        "permission_denied".to_string()
                    }),
                    Some(if rejected {
                        "Required tool approval was rejected.".to_string()
                    } else {
                        "Required tool permission was denied.".to_string()
                    }),
                    None,
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
                None,
            ),
            StepRunnerOutcome::BudgetExhausted { dimension, reason } => (
                StepRecordStatus::BudgetExhausted,
                format!("Step stopped after exhausting its {dimension:?} budget."),
                StepCompletionBasis::RuntimeFailure,
                Some(format!(
                    "{}_budget_exhausted",
                    budget_dimension_code(*dimension)
                )),
                Some(bounded_step_summary(
                    reason,
                    "A configured per-step execution budget was exhausted.",
                )),
                None,
            ),
            StepRunnerOutcome::TokenLimit { reason } => (
                StepRecordStatus::BudgetExhausted,
                "Step stopped because the context token limit was exceeded.".to_string(),
                StepCompletionBasis::RuntimeFailure,
                Some(CONTEXT_TOKEN_LIMIT_CODE.to_string()),
                Some(bounded_step_summary(
                    reason,
                    "The configured context token limit was exceeded.",
                )),
                None,
            ),
            StepRunnerOutcome::Indeterminate { reason } => (
                StepRecordStatus::Indeterminate,
                "Step stopped with an indeterminate external effect that will not be replayed."
                    .to_string(),
                StepCompletionBasis::RuntimeFailure,
                Some("external_effect_indeterminate".to_string()),
                Some(bounded_step_summary(
                    reason,
                    "The external effect could not be determined safely.",
                )),
                None,
            ),
            StepRunnerOutcome::Cancelled => (
                StepRecordStatus::Cancelled,
                "Step was cancelled before completion.".to_string(),
                StepCompletionBasis::UserDecision,
                Some("cancelled".to_string()),
                Some("The step was cancelled before completion.".to_string()),
                None,
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
        ambiguity,
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
        ambiguity: None,
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

fn parse_structured_step_conclusion(output: &str) -> (String, Option<PlanAmbiguity>) {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StructuredConclusion {
        summary: String,
        #[serde(default)]
        lifecycle_ambiguity: Option<StructuredAmbiguity>,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StructuredAmbiguity {
        kind: PlanAmbiguityKind,
        safe_summary: String,
        #[serde(default)]
        evidence_refs: Vec<String>,
    }

    let Ok(parsed) = serde_json::from_str::<StructuredConclusion>(output.trim()) else {
        return (bounded_step_summary(output, "Step completed."), None);
    };
    let summary = bounded_step_summary(&parsed.summary, "Step completed.");
    let ambiguity = parsed.lifecycle_ambiguity.and_then(|value| {
        let ambiguity = PlanAmbiguity {
            kind: value.kind,
            safe_summary: value.safe_summary,
            evidence_refs: value.evidence_refs,
        };
        ambiguity.validate().is_ok().then_some(ambiguity)
    });
    (summary, ambiguity)
}

fn budget_dimension_code(dimension: ExecutionBudgetDimension) -> &'static str {
    match dimension {
        ExecutionBudgetDimension::PlanSteps => "plan_steps",
        ExecutionBudgetDimension::StepAttempts => "step_attempts",
        ExecutionBudgetDimension::ModelTurns => "model_turns",
        ExecutionBudgetDimension::ModelTurnsPerStep => "model_turns_per_step",
        ExecutionBudgetDimension::ToolCalls => "tool_calls",
        ExecutionBudgetDimension::ToolCallsPerStep => "tool_calls_per_step",
        ExecutionBudgetDimension::PlanRevisions => "plan_revisions",
        ExecutionBudgetDimension::ModelRepairs => "model_repairs",
        ExecutionBudgetDimension::FinalizationTurns => "finalization_turns",
        ExecutionBudgetDimension::WallTime => "wall_time",
        ExecutionBudgetDimension::TotalTokens => "total_tokens",
        ExecutionBudgetDimension::Cost => "cost",
    }
}

fn is_permission_denied(reason: &str) -> bool {
    reason.starts_with("Permission denied:")
}
