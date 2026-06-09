use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::compaction::maybe_compact_history;
use crate::core::engine::planned_step_failure_message;
use crate::core::events::StreamEvent;
use crate::core::model_turn::{ModelTurnItem, run_model_turn};
use crate::core::planner::Planner;
use crate::core::run_loop::{LoopContext, LoopItem, enrich_prompt_metadata};
use crate::core::tool_turn::{ToolAction, ToolTurnItem, append_tool_history, run_tool_turn};
use crate::core::types::{Action, Message, TaskPlan, TerminationReason, ToolCallAction};

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
        macro_rules! drive_tool_turn {
            ($action:expr) => {{
                let mut tool_stream = run_tool_turn(
                    ctx.tool_turn_context(cancel_token.clone()),
                    $action,
                );
                loop {
                    match tool_stream.next().await {
                        Some(ToolTurnItem::Event(event)) => yield LoopItem::Event(event),
                        Some(ToolTurnItem::Finished(outcome)) => break outcome,
                        Some(ToolTurnItem::Cancelled) => {
                            yield LoopItem::Complete {
                                reason: TerminationReason::Cancelled,
                                output: None,
                            };
                            return;
                        }
                        None => {
                            yield LoopItem::Complete {
                                reason: TerminationReason::Error,
                                output: Some("tool turn ended without a result".to_string()),
                            };
                            return;
                        }
                    }
                }
            }};
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
            let step_prompt = format!(
                "Goal: {}\nCurrent step {}: {}\nComplete this step and report the result.",
                active_plan.goal, current_step.id, current_step.title
            );
            let context = ctx.context_manager.build_with_checkpoint(
                &step_prompt,
                &state.working_memory,
                state.compact_summary.as_deref(),
                &state.history,
            );
            let tool_schemas = ctx.registry.schemas();
            yield LoopItem::Event(StreamEvent::PromptBuilt {
                metadata: enrich_prompt_metadata(&ctx, context.metadata.clone(), &tool_schemas),
            });
            if context.over_hard_limit {
                yield LoopItem::Complete {
                    reason: TerminationReason::TokenLimit,
                    output: Some("context exceeds configured hard token budget".to_string()),
                };
                return;
            }
            if context.auto_compaction_needed && context.dropped_history_messages > 0 {
                let compacted_count = context.dropped_history_messages.min(state.history.len());
                if let Some(update) = maybe_compact_history(
                    &mut compaction,
                    ctx.model,
                    &state.history[..compacted_count],
                    cancel_token.clone(),
                )
                .await
                {
                    let summary_for_event = update.summary.clone();
                    if let Some(summary) = update.summary {
                        state.compact_summary = Some(summary);
                    }
                    yield LoopItem::Event(StreamEvent::PromptCompacted {
                        summary: summary_for_event,
                        state: update.state,
                    });
                }
            }

            let mut turn_stream = run_model_turn(
                ctx.model,
                context.messages,
                tool_schemas,
                cancel_token.clone(),
            );
            let model_turn = loop {
                match turn_stream.next().await {
                    Some(ModelTurnItem::Event(event)) => yield LoopItem::Event(event),
                    Some(ModelTurnItem::Finished(turn)) => break turn,
                    Some(ModelTurnItem::Cancelled) => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Cancelled,
                            output: None,
                        };
                        return;
                    }
                    Some(ModelTurnItem::Failed(err)) => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Error,
                            output: Some(format!("Model error: {err}")),
                        };
                        return;
                    }
                    None => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Error,
                            output: Some("model turn ended without a response".to_string()),
                        };
                        return;
                    }
                }
            };

            match model_turn.action {
                Action::ToolCall {
                    call_id,
                    tool_use_id,
                    name,
                    args,
                } => {
                    let outcome = drive_tool_turn!(
                        ToolAction::Call(ToolCallAction {
                            call_id,
                            tool_use_id,
                            name,
                            args,
                        })
                    );
                    append_tool_history(&mut state.history, &model_turn.full_response, &outcome);
                    if let Some(reason) = outcome.first_error_reason() {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step.clone(),
                            index: current_index,
                            reason: reason.clone(),
                        });
                        if is_permission_denied_tool_failure(&reason) {
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
                    } else {
                        if let Some(record) = outcome.records.last() {
                            final_output = Some(record.history_output.clone());
                        }
                        active_plan.mark_current_done();
                        yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                            step: current_step,
                            index: current_index,
                        });
                    }
                }
                Action::ToolBatch { calls } => {
                    let outcome = drive_tool_turn!(ToolAction::Batch(calls));
                    append_tool_history(&mut state.history, &model_turn.full_response, &outcome);
                    if let Some(reason) = outcome.first_error_reason() {
                        yield LoopItem::Event(StreamEvent::PlanStepFailed {
                            step: current_step.clone(),
                            index: current_index,
                            reason: reason.clone(),
                        });
                        if is_permission_denied_tool_failure(&reason) {
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
                    } else {
                        if let Some(record) = outcome.records.last() {
                            final_output = Some(record.history_output.clone());
                        }
                        active_plan.mark_current_done();
                        yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                            step: current_step,
                            index: current_index,
                        });
                    }
                }
                Action::Final { text } => {
                    state.history.push(Message::assistant(text.clone()));
                    final_output = Some(text);
                    active_plan.mark_current_done();
                    yield LoopItem::Event(StreamEvent::PlanStepCompleted {
                        step: current_step,
                        index: current_index,
                    });
                }
                Action::Malformed { reason } => {
                    state.history.push(Message::assistant(model_turn.full_response));
                    state.history.push(Message::user(format!(
                        "Your previous output could not be parsed: {}. Please try again.",
                        reason
                    )));
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
                    yield LoopItem::Event(StreamEvent::PlanCreated {
                        plan: replacement.clone(),
                    });
                    *active_plan = replacement;
                }
            }
        }

        yield LoopItem::Complete {
            reason: TerminationReason::Final,
            output: final_output,
        };
    })
}

fn is_permission_denied_tool_failure(reason: &str) -> bool {
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
