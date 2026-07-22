use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::compaction::maybe_compact_history;
use crate::core::events::StreamEvent;
use crate::core::model_turn::{ModelTurnItem, run_model_turn};
use crate::core::run_loop::{LoopContext, enrich_prompt_metadata, extract_session_memory_notes};
use crate::core::tool_turn::{ToolAction, ToolTurnItem, append_tool_history, run_tool_turn};
use crate::core::types::{Action, CallId, Message, PlanStep, ToolMutation, Usage};
use crate::memory::session::append_session_notes_to_dir_sync;

/// Inputs owned by one bounded planned-step attempt.
///
/// The step history is kept separate while the attempt is running. It is
/// injected as working-memory prefix material on every model turn, so a small
/// global history window or an in-progress compaction cannot drop the tool
/// result that the next turn must read.
pub(crate) struct StepRunnerInput {
    pub goal: String,
    pub step: PlanStep,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub compaction: crate::core::compaction::CompactionRuntime,
}

#[derive(Debug)]
pub(crate) enum StepRunnerOutcome {
    Succeeded { output: String },
    Failed { reason: String, replan: bool },
    BudgetExhausted { reason: String },
    TokenLimit { reason: String },
    Cancelled,
}

/// Metrics collected from the canonical model/tool events emitted while one
/// planned step is running.  These are deliberately event-derived so the
/// ledger cannot drift from the trace.
#[derive(Debug, Clone, Default)]
pub(crate) struct StepRunMetrics {
    pub model_turns_used: u32,
    pub tool_call_ids: Vec<CallId>,
    pub mutations: Vec<ToolMutation>,
    pub token_usage: Usage,
}

#[derive(Debug)]
pub(crate) struct StepRunnerResult {
    pub outcome: StepRunnerOutcome,
    pub history: Vec<Message>,
    pub compact_summary: Option<String>,
    pub compaction: crate::core::compaction::CompactionRuntime,
    pub metrics: StepRunMetrics,
}

#[derive(Debug)]
pub(crate) enum StepRunnerItem {
    Event(StreamEvent),
    Finished(StepRunnerResult),
}

/// Run one plan step as a bounded ReAct loop.
pub(crate) fn run_step<'a>(
    ctx: LoopContext<'a>,
    input: StepRunnerInput,
    cancel_token: CancellationToken,
) -> BoxStream<'a, StepRunnerItem> {
    Box::pin(stream! {
        let StepRunnerInput {
            goal,
            step,
            working_memory,
            compact_summary: initial_compact_summary,
            history,
            mut compaction,
        } = input;
        let mut compact_summary = initial_compact_summary;
        let mut step_history = Vec::new();
        let mut model_turns = 0u32;
        let mut metrics = StepRunMetrics::default();
        let max_model_turns = ctx.max_model_turns_per_step;
        let step_prompt = format!(
            "Goal: {goal}\nCurrent step {}: {}\nComplete this step and report the result. A tool result is evidence, not step completion; continue this step until you can state its conclusion.",
            step.id, step.title
        );

        macro_rules! emit_step_event {
            ($event:expr) => {{
                let event = $event;
                record_step_event(&mut metrics, &event);
                yield StepRunnerItem::Event(event);
            }};
        }

        macro_rules! drive_tool_turn {
            ($action:expr) => {{
                let mut tool_stream = run_tool_turn(
                    ctx.tool_turn_context(cancel_token.clone()),
                    $action,
                );
                loop {
                    match tool_stream.next().await {
                        Some(ToolTurnItem::Event(event)) => {
                            emit_step_event!(event);
                        }
                        Some(ToolTurnItem::Finished(outcome)) => break Ok(outcome),
                        Some(ToolTurnItem::Cancelled) => break Err(StepRunnerOutcome::Cancelled),
                        None => break Err(StepRunnerOutcome::Failed {
                            reason: "tool turn ended without a result".to_string(),
                            replan: true,
                        }),
                    }
                }
            }};
        }

        loop {
            if cancel_token.is_cancelled() {
                yield StepRunnerItem::Finished(finish_result(
                    StepRunnerOutcome::Cancelled,
                    history,
                    step_history,
                    compact_summary,
                    compaction,
                    metrics,
                ));
                return;
            }

            if model_turns >= max_model_turns {
                yield StepRunnerItem::Finished(finish_result(
                    StepRunnerOutcome::BudgetExhausted {
                        reason: format!(
                            "step model-turn budget exhausted (max_model_turns_per_step={max_model_turns})"
                        ),
                    },
                    history,
                    step_history,
                    compact_summary,
                    compaction,
                    metrics,
                ));
                return;
            }

            // Current-step messages are prefix material for this attempt. This
            // preserves the model/tool round trip even when the global history
            // window is configured to zero or compaction is in progress.
            let mut turn_working_memory = working_memory.clone();
            turn_working_memory.extend(step_history.iter().cloned());
            let mut context = ctx.context_manager.build_with_checkpoint(
                &step_prompt,
                &turn_working_memory,
                compact_summary.as_deref(),
                &history,
            );

            if context.over_hard_limit {
                yield StepRunnerItem::Finished(finish_result(
                    StepRunnerOutcome::TokenLimit {
                        reason: "context exceeds configured hard token budget".to_string(),
                    },
                    history,
                    step_history,
                    compact_summary,
                    compaction,
                    metrics,
                ));
                return;
            }

            if context.auto_compaction_needed && context.dropped_history_messages > 0 {
                let compacted_count = context.dropped_history_messages.min(history.len());

                // Flush durable-worthy notes before the old global history is
                // summarized. Step-local messages remain in the prefix above.
                let mut flush_notes = Vec::new();
                if compacted_count > 0 {
                    let candidate_notes = extract_session_memory_notes(&history[..compacted_count]);
                    if !candidate_notes.is_empty()
                        && append_session_notes_to_dir_sync(
                            &ctx.memory_paths.session_dir,
                            ctx.session_id,
                            &candidate_notes,
                        )
                        .is_ok()
                    {
                        flush_notes = candidate_notes;
                        emit_step_event!(StreamEvent::MemoryFlushed {
                            notes: flush_notes.clone(),
                        });
                    }
                }

                if let Some(update) = maybe_compact_history(
                    &mut compaction,
                    ctx.model,
                    &history[..compacted_count],
                    flush_notes,
                    cancel_token.clone(),
                )
                .await
                {
                    let summary_for_event = update.summary.clone();
                    if let Some(summary) = update.summary {
                        compact_summary = Some(summary);
                    }
                    emit_step_event!(StreamEvent::PromptCompacted {
                        summary: summary_for_event,
                        state: update.state,
                    });
                }

                // Compaction changes the prompt prefix. Rebuild the actual
                // model input so this turn observes the new summary and the
                // current step's tool history.
                context = ctx.context_manager.build_with_checkpoint(
                    &step_prompt,
                    &turn_working_memory,
                    compact_summary.as_deref(),
                    &history,
                );
                if context.over_hard_limit {
                    yield StepRunnerItem::Finished(finish_result(
                        StepRunnerOutcome::TokenLimit {
                            reason: "context exceeds configured hard token budget after compaction"
                                .to_string(),
                        },
                        history,
                        step_history,
                        compact_summary,
                        compaction,
                        metrics,
                    ));
                    return;
                }
            }

            let tool_schemas = ctx.registry.schemas();
            emit_step_event!(StreamEvent::PromptBuilt {
                metadata: enrich_prompt_metadata(&ctx, context.metadata.clone(), &tool_schemas),
            });

            model_turns += 1;
            metrics.model_turns_used = model_turns;
            let mut turn_stream = run_model_turn(
                ctx.model,
                context.messages,
                ctx.registry.model_schemas(),
                cancel_token.clone(),
            );
            let model_turn = loop {
                match turn_stream.next().await {
                    Some(ModelTurnItem::Event(event)) => emit_step_event!(event),
                    Some(ModelTurnItem::Finished(turn)) => break Ok(turn),
                    Some(ModelTurnItem::Cancelled) => break Err(StepRunnerOutcome::Cancelled),
                    Some(ModelTurnItem::Failed(err)) => break Err(StepRunnerOutcome::Failed {
                        reason: format!("Model error: {err}"),
                        replan: true,
                    }),
                    None => break Err(StepRunnerOutcome::Failed {
                        reason: "model turn ended without a response".to_string(),
                        replan: true,
                    }),
                }
            };

            let model_turn = match model_turn {
                Ok(turn) => turn,
                Err(outcome) => {
                    yield StepRunnerItem::Finished(finish_result(
                        outcome,
                        history,
                        step_history,
                        compact_summary,
                        compaction,
                        metrics,
                    ));
                    return;
                }
            };

            match model_turn.action {
                Action::Final { text } => {
                    step_history.push(Message::assistant(text.clone()));
                    yield StepRunnerItem::Finished(finish_result(
                        StepRunnerOutcome::Succeeded { output: text },
                        history,
                        step_history,
                        compact_summary,
                        compaction,
                        metrics,
                    ));
                    return;
                }
                Action::Malformed { reason } => {
                    step_history.push(Message::assistant(model_turn.full_response));
                    step_history.push(Message::user(format!(
                        "Your previous output could not be parsed: {reason}. Please try again for the current plan step and return a valid tool call or a step conclusion."
                    )));
                }
                Action::ToolCall {
                    call_id,
                    tool_use_id,
                    name,
                    args,
                } => {
                    let outcome = match drive_tool_turn!(ToolAction::Call(
                        crate::core::types::ToolCallAction {
                            call_id,
                            tool_use_id,
                            name,
                            args,
                        }
                    )) {
                        Ok(outcome) => outcome,
                        Err(outcome) => {
                            yield StepRunnerItem::Finished(finish_result(
                                outcome,
                                history,
                                step_history,
                                compact_summary,
                                compaction,
                                metrics,
                            ));
                            return;
                        }
                    };
                    append_tool_history(&mut step_history, &model_turn.full_response, &outcome);
                    if let Some(reason) = outcome.first_error_reason()
                        && is_permission_denied(&reason)
                    {
                        yield StepRunnerItem::Finished(finish_result(
                            StepRunnerOutcome::Failed {
                                reason,
                                replan: false,
                            },
                            history,
                            step_history,
                            compact_summary,
                            compaction,
                            metrics,
                        ));
                        return;
                    }
                }
                Action::ToolBatch { calls } => {
                    let outcome = match drive_tool_turn!(ToolAction::Batch(calls)) {
                        Ok(outcome) => outcome,
                        Err(outcome) => {
                            yield StepRunnerItem::Finished(finish_result(
                                outcome,
                                history,
                                step_history,
                                compact_summary,
                                compaction,
                                metrics,
                            ));
                            return;
                        }
                    };
                    append_tool_history(&mut step_history, &model_turn.full_response, &outcome);
                    if let Some(reason) = outcome.first_error_reason()
                        && is_permission_denied(&reason)
                    {
                        yield StepRunnerItem::Finished(finish_result(
                            StepRunnerOutcome::Failed {
                                reason,
                                replan: false,
                            },
                            history,
                            step_history,
                            compact_summary,
                            compaction,
                            metrics,
                        ));
                        return;
                    }
                }
            }
        }
    })
}

fn finish_result(
    outcome: StepRunnerOutcome,
    mut history: Vec<Message>,
    mut step_history: Vec<Message>,
    compact_summary: Option<String>,
    compaction: crate::core::compaction::CompactionRuntime,
    metrics: StepRunMetrics,
) -> StepRunnerResult {
    history.append(&mut step_history);
    StepRunnerResult {
        outcome,
        history,
        compact_summary,
        compaction,
        metrics,
    }
}

fn record_step_event(metrics: &mut StepRunMetrics, event: &StreamEvent) {
    match event {
        StreamEvent::LlmMessage { usage, .. } => {
            metrics.token_usage.prompt_tokens = metrics
                .token_usage
                .prompt_tokens
                .saturating_add(usage.prompt_tokens);
            metrics.token_usage.completion_tokens = metrics
                .token_usage
                .completion_tokens
                .saturating_add(usage.completion_tokens);
            metrics.token_usage.total_tokens = metrics
                .token_usage
                .total_tokens
                .saturating_add(usage.total_tokens);
            metrics.token_usage.cached_tokens = metrics
                .token_usage
                .cached_tokens
                .saturating_add(usage.cached_tokens);
        }
        StreamEvent::ToolCallStarted { call_id, .. }
            if !metrics.tool_call_ids.contains(call_id) =>
        {
            metrics.tool_call_ids.push(*call_id);
        }
        StreamEvent::ToolCallCompleted { result, .. } => {
            metrics.mutations.extend(result.mutations.clone());
        }
        _ => {}
    }
}

fn is_permission_denied(reason: &str) -> bool {
    reason.starts_with("Permission denied:")
}
