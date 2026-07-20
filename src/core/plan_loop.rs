use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::engine::planned_step_failure_message;
use crate::core::events::StreamEvent;
use crate::core::planner::Planner;
use crate::core::run_loop::{LoopContext, LoopItem};
use crate::core::step_runner::{StepRunnerInput, StepRunnerItem, StepRunnerOutcome, run_step};
use crate::core::types::{Message, TaskPlan, TerminationReason};

pub(crate) struct PlanLoopState {
    pub user_message: String,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub step: u32,
    pub plan: Option<TaskPlan>,
}

pub(crate) fn run_planned_loop<'a>(
    ctx: LoopContext<'a>,
    planner: &'a Planner,
    mut state: PlanLoopState,
    cancel_token: CancellationToken,
) -> BoxStream<'a, LoopItem> {
    Box::pin(stream! {
        let mut compaction = ctx.compaction.clone();

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
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: drafted.clone(),
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
            yield LoopItem::Event(StreamEvent::PlanStepStarted {
                step: current_step.clone(),
                index: current_index,
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
                yield LoopItem::Complete {
                    reason: TerminationReason::Error,
                    output: Some("step runner ended without a result".to_string()),
                };
                return;
            };

            state.history = runner_result.history;
            state.compact_summary = runner_result.compact_summary;
            compaction = runner_result.compaction;

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
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: replacement.clone(),
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
                    yield LoopItem::Complete {
                        reason: TerminationReason::TokenLimit,
                        output: Some(reason),
                    };
                    return;
                }
                StepRunnerOutcome::Cancelled => {
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
