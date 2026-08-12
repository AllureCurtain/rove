use async_stream::stream;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::compaction::maybe_compact_history;
use crate::engine::control::{AcceptedSteer, steer_accepted_event};
use crate::events::StreamEvent;
use crate::execution::ExecutionBudgetDimension;
use crate::memory::session::append_session_notes_to_dir_sync;
use crate::run_loop::{
    ActiveInstructionTarget, LoopContext, active_target_covers, enrich_prompt_metadata,
    extract_session_memory_notes, run_kernel_model_turn, scoped_paths_for_action,
    scoped_prompt_events, scoped_prompt_for_target, shell_path_declaration_missing,
};
use crate::tool_turn::{
    ToolAction, ToolTurnItem, ToolTurnOutcome, append_tool_history, defer_tool_turn, run_tool_turn,
};
use crate::types::{CallId, Message, PlanStep, ToolMutation, Usage};
use rove_core::{
    AgentKernelHost, KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem,
    KernelLimits, KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction,
    KernelToolTurnItem, run_agent_kernel,
};

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
    pub procedure_messages: Vec<Message>,
    pub compaction: crate::compaction::CompactionRuntime,
    /// Steers accepted before this runner started. Steers arriving while the
    /// step is running are accepted at its internal model-turn safe points.
    pub accepted_steer_ids: Vec<AcceptedSteer>,
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_repairs: u32,
    pub max_total_tokens: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum StepRunnerOutcome {
    Succeeded {
        output: String,
    },
    Failed {
        reason: String,
        replan: bool,
    },
    BudgetExhausted {
        dimension: ExecutionBudgetDimension,
        reason: String,
    },
    TokenLimit {
        reason: String,
    },
    Indeterminate {
        reason: String,
    },
    Cancelled,
}

/// Metrics collected from the canonical model/tool events emitted while one
/// planned step is running. These are deliberately event-derived so the
/// ledger cannot drift from the trace.
#[derive(Debug, Clone, Default)]
pub(crate) struct StepRunMetrics {
    pub model_turns_used: u32,
    pub tool_call_ids: Vec<CallId>,
    pub mutations: Vec<ToolMutation>,
    pub token_usage: Usage,
    pub repairs_used: u32,
}

#[derive(Debug)]
pub(crate) struct StepRunnerResult {
    pub outcome: StepRunnerOutcome,
    pub history: Vec<Message>,
    pub compact_summary: Option<String>,
    pub compaction: crate::compaction::CompactionRuntime,
    pub metrics: StepRunMetrics,
}

#[derive(Debug)]
pub(crate) enum StepRunnerItem {
    Event(StreamEvent),
    Finished(StepRunnerResult),
}

/// Run one plan step through the same Runtime-neutral kernel used by the
/// embedded Agent and the unplanned Runtime path.
pub(crate) fn run_step<'a>(
    ctx: LoopContext<'a>,
    input: StepRunnerInput,
    cancel_token: CancellationToken,
) -> BoxStream<'a, StepRunnerItem> {
    Box::pin(stream! {
        let max_model_turns = input.max_model_turns;
        let max_tool_calls = input.max_tool_calls;
        let max_repairs = input.max_repairs;
        let step_prompt = format!(
            "Goal: {}\nCurrent step {}: {}\nComplete this step and report the result. A tool result is evidence, not step completion; continue this step until you can state its conclusion. If evidence, unavailable capability, unmet preconditions, a user constraint, staleness, or a safer path requires departing from supplied procedure guidance, return a structured conclusion with summary and procedure_deviations; a deviation never grants permission.",
            input.goal, input.step.id, input.step.title
        );
        let active_instruction_target =
            ActiveInstructionTarget::from_text(&ctx, &step_prompt, "plan_step");
        let host = StepKernelHost {
            ctx,
            step_prompt,
            working_memory: input
                .working_memory
                .into_iter()
                .chain(input.procedure_messages)
                .collect(),
            compact_summary: input.compact_summary,
            history: input.history,
            compaction: Some(input.compaction),
            pending_steer_ids: input.accepted_steer_ids,
            max_total_tokens: input.max_total_tokens,
            active_instruction_target,
        };
        let mut metrics = StepRunMetrics::default();
        let mut kernel = run_agent_kernel(
            host,
            KernelState::default(),
            KernelLimits {
                max_model_turns: Some(max_model_turns),
                max_tool_calls: Some(max_tool_calls),
                max_repairs: Some(max_repairs),
            },
            cancel_token,
        );

        while let Some(item) = kernel.next().await {
            match item {
                KernelItem::Event(event) => {
                    record_step_event(&mut metrics, &event);
                    yield StepRunnerItem::Event(event);
                }
                KernelItem::Finished(result) => {
                    metrics.model_turns_used = result.state.model_turns;
                    let outcome = match result.termination {
                        KernelTermination::Final { output } => {
                            StepRunnerOutcome::Succeeded { output }
                        }
                        KernelTermination::ModelTurnLimit => {
                            StepRunnerOutcome::BudgetExhausted {
                                dimension: ExecutionBudgetDimension::ModelTurnsPerStep,
                                reason: format!(
                                    "step model-turn budget exhausted (max_model_turns_per_step={max_model_turns})"
                                ),
                            }
                        }
                        KernelTermination::ToolCallLimit => {
                            StepRunnerOutcome::BudgetExhausted {
                                dimension: ExecutionBudgetDimension::ToolCallsPerStep,
                                reason: "step tool-call budget exhausted".to_string(),
                            }
                        }
                        KernelTermination::RepairLimit => StepRunnerOutcome::BudgetExhausted {
                            dimension: ExecutionBudgetDimension::ModelRepairs,
                            reason: "step structured-output repair budget exhausted".to_string(),
                        },
                        KernelTermination::Cancelled => StepRunnerOutcome::Cancelled,
                        KernelTermination::ModelFailed(error) => StepRunnerOutcome::Failed {
                            reason: format!("Model error: {error}"),
                            replan: true,
                        },
                        KernelTermination::IncompleteBeforeModelTurn => {
                            StepRunnerOutcome::Failed {
                                reason: "before-model extension ended without a request".to_string(),
                                replan: true,
                            }
                        }
                        KernelTermination::IncompleteModelTurn => StepRunnerOutcome::Failed {
                            reason: "model turn ended without a response".to_string(),
                            replan: true,
                        },
                        KernelTermination::IncompleteToolTurn => StepRunnerOutcome::Indeterminate {
                            reason: "tool turn ended without a result; external effect is unknown"
                                .to_string(),
                        },
                        KernelTermination::Extension {
                            reason: StepKernelStop::TokenLimit,
                            output,
                        } => StepRunnerOutcome::TokenLimit {
                            reason: output.unwrap_or_else(|| {
                                "context exceeds configured hard token budget".to_string()
                            }),
                        },
                        KernelTermination::Extension {
                            reason: StepKernelStop::GlobalTokenBudget,
                            output,
                        } => StepRunnerOutcome::BudgetExhausted {
                            dimension: ExecutionBudgetDimension::TotalTokens,
                            reason: output.unwrap_or_else(|| {
                                "global model token budget exhausted".to_string()
                            }),
                        },
                        KernelTermination::Extension {
                            reason: StepKernelStop::ToolFailure { replan },
                            output,
                        } => StepRunnerOutcome::Failed {
                            reason: output.unwrap_or_else(|| "tool execution failed".to_string()),
                            replan,
                        },
                    };
                    metrics.repairs_used = result.state.repairs;
                    let extension = result.extension;
                    yield StepRunnerItem::Finished(StepRunnerResult {
                        outcome,
                        history: extension.history,
                        compact_summary: extension.compact_summary,
                        compaction: extension.compaction,
                        metrics,
                    });
                    return;
                }
            }
        }
    })
}

#[derive(Debug)]
enum StepKernelStop {
    TokenLimit,
    GlobalTokenBudget,
    ToolFailure { replan: bool },
}

struct StepKernelOutput {
    history: Vec<Message>,
    compact_summary: Option<String>,
    compaction: crate::compaction::CompactionRuntime,
}

struct StepKernelHost<'a> {
    ctx: LoopContext<'a>,
    step_prompt: String,
    working_memory: Vec<Message>,
    compact_summary: Option<String>,
    history: Vec<Message>,
    compaction: Option<crate::compaction::CompactionRuntime>,
    pending_steer_ids: Vec<AcceptedSteer>,
    max_total_tokens: Option<u64>,
    active_instruction_target: ActiveInstructionTarget,
}

impl AgentKernelHost for StepKernelHost<'_> {
    type Event = StreamEvent;
    type Stop = StepKernelStop;
    type ToolOutcome = ToolTurnOutcome;
    type Output = StepKernelOutput;

    fn before_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelBeforeModelTurnItem<Self::Event, Self::Stop>> {
        Box::pin(stream! {
            if self
                .max_total_tokens
                .is_some_and(|limit| u64::from(state.usage.total_tokens) >= limit)
            {
                yield KernelBeforeModelTurnItem::Stop {
                    reason: StepKernelStop::GlobalTokenBudget,
                    output: Some("global model token budget exhausted".to_string()),
                };
                return;
            }
            if let Some(rx) = self.ctx.steer_rx.as_ref() {
                let mut receiver = rx.lock().await;
                while let Ok(steer) = receiver.try_recv() {
                    let accepted = AcceptedSteer {
                        id: steer.id.0.clone(),
                        unified_message: steer.unified_message,
                    };
                    self.working_memory
                        .push(Message::user(steer.content.clone()));
                    if let Some(lifecycle) = self.ctx.steer_lifecycle.as_ref() {
                        lifecycle.accepted(accepted.clone()).await;
                    }
                    yield KernelBeforeModelTurnItem::Event(steer_accepted_event(steer));
                    self.pending_steer_ids.push(accepted);
                }
            }

            let mut turn_working_memory = self.working_memory.clone();
            turn_working_memory.extend(state.history.iter().cloned());
            let scoped = scoped_prompt_for_target(&self.ctx, &self.active_instruction_target);
            for event in scoped_prompt_events(
                &self.ctx,
                &self.active_instruction_target,
                &scoped,
            ) {
                yield KernelBeforeModelTurnItem::Event(event);
            }
            turn_working_memory.extend(scoped.messages);
            let mut context = self.ctx.context_manager.build_with_checkpoint(
                &self.step_prompt,
                &turn_working_memory,
                self.compact_summary.as_deref(),
                &self.history,
            );

            if context.over_hard_limit {
                yield KernelBeforeModelTurnItem::Stop {
                    reason: StepKernelStop::TokenLimit,
                    output: Some("context exceeds configured hard token budget".to_string()),
                };
                return;
            }

            if context.auto_compaction_needed && context.dropped_history_messages > 0 {
                let compacted_count = context.dropped_history_messages.min(self.history.len());
                let mut flush_notes = Vec::new();
                if compacted_count > 0 {
                    let candidate_notes =
                        extract_session_memory_notes(&self.history[..compacted_count]);
                    if !candidate_notes.is_empty()
                        && append_session_notes_to_dir_sync(
                            &self.ctx.memory_paths.session_dir,
                            self.ctx.session_id,
                            &candidate_notes,
                        )
                        .is_ok()
                    {
                        flush_notes = candidate_notes;
                        yield KernelBeforeModelTurnItem::Event(StreamEvent::MemoryFlushed {
                            notes: flush_notes.clone(),
                        });
                    }
                }

                if let Some(update) = maybe_compact_history(
                    self.compaction
                        .as_mut()
                        .expect("step compaction state should be present"),
                    self.ctx.model,
                    &self.history[..compacted_count],
                    flush_notes,
                    cancel_token,
                )
                .await
                {
                    let summary_for_event = update.summary.clone();
                    if let Some(summary) = update.summary {
                        self.compact_summary = Some(summary);
                    }
                    yield KernelBeforeModelTurnItem::Event(StreamEvent::PromptCompacted {
                        summary: summary_for_event,
                        state: update.state,
                    });
                }

                context = self.ctx.context_manager.build_with_checkpoint(
                    &self.step_prompt,
                    &turn_working_memory,
                    self.compact_summary.as_deref(),
                    &self.history,
                );
                if context.over_hard_limit {
                    yield KernelBeforeModelTurnItem::Stop {
                        reason: StepKernelStop::TokenLimit,
                        output: Some(
                            "context exceeds configured hard token budget after compaction"
                                .to_string(),
                        ),
                    };
                    return;
                }
            }

            let tool_schemas = self.ctx.descriptors();
            yield KernelBeforeModelTurnItem::Event(StreamEvent::PromptBuilt {
                metadata: enrich_prompt_metadata(
                    &self.ctx,
                    context.metadata.clone(),
                    &tool_schemas,
                ),
            });
            yield KernelBeforeModelTurnItem::Ready(context.messages);
        })
    }

    fn model_turn<'a>(
        &'a mut self,
        messages: Vec<Message>,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelModelTurnItem<Self::Event>> {
        run_kernel_model_turn(
            self.ctx.model,
            self.ctx.model_schemas(),
            messages,
            cancel_token,
            std::mem::take(&mut self.pending_steer_ids),
            self.ctx.steer_lifecycle.clone(),
        )
    }

    fn after_model_turn<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _turn: &'a crate::model_turn::ModelTurn,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(()) })
    }

    fn tool_turn<'a>(
        &'a mut self,
        action: KernelToolAction,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelToolTurnItem<Self::Event, Self::ToolOutcome>> {
        let action = match action {
            KernelToolAction::Call(call) => ToolAction::Call(call),
            KernelToolAction::Batch(calls) => ToolAction::Batch(calls),
        };
        let paths = scoped_paths_for_action(&self.ctx, &action);
        let call_id = action.calls().first().map(|call| call.call_id);
        let missing_shell_paths = shell_path_declaration_missing(&self.ctx, &action);
        let needs_scoped_context = !paths.is_empty()
            && !active_target_covers(&self.ctx, &self.active_instruction_target, &paths);
        let inner: BoxStream<'a, ToolTurnItem> = if missing_shell_paths {
            defer_tool_turn(
                action,
                "run_shell must declare bounded workspace-relative paths when nested AGENTS.md scopes exist"
                    .to_string(),
            )
        } else if needs_scoped_context {
            self.active_instruction_target = ActiveInstructionTarget::for_tool(paths, call_id);
            defer_tool_turn(
                action,
                "path-scoped workspace instructions were activated; reconsider the call and retry if it remains appropriate"
                    .to_string(),
            )
        } else {
            if !paths.is_empty() {
                self.active_instruction_target = ActiveInstructionTarget::for_tool(paths, call_id);
            }
            run_tool_turn(self.ctx.tool_turn_context(cancel_token), action)
        };
        Box::pin(stream! {
            let mut inner = inner;
            while let Some(item) = inner.next().await {
                match item {
                    ToolTurnItem::Event(event) => yield KernelToolTurnItem::Event(event),
                    ToolTurnItem::Finished(outcome) => {
                        yield KernelToolTurnItem::Finished(outcome);
                        return;
                    }
                    ToolTurnItem::Cancelled => {
                        yield KernelToolTurnItem::Cancelled;
                        return;
                    }
                }
            }
        })
    }

    fn tool_history(&mut self, full_response: &str, outcome: &Self::ToolOutcome) -> Vec<Message> {
        let mut history = Vec::new();
        append_tool_history(&mut history, full_response, outcome);
        history
    }

    fn after_tool_turn<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        outcome: &'a Self::ToolOutcome,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>> {
        Box::pin(async move {
            if let Some(reason) = outcome.first_error_reason()
                && is_permission_denied(&reason)
            {
                KernelHook::Stop {
                    reason: StepKernelStop::ToolFailure { replan: false },
                    output: Some(reason),
                    events: Vec::new(),
                }
            } else {
                KernelHook::continue_with(())
            }
        })
    }

    fn after_final<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _output: &'a str,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<KernelFinalAction, Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(KernelFinalAction::Complete) })
    }

    fn malformed_retry_message(&self, reason: &str) -> String {
        format!(
            "Your previous output could not be parsed: {reason}. Please try again for the current plan step and return a valid tool call or a step conclusion."
        )
    }

    fn finish_output(&mut self, state: &KernelState) -> Self::Output {
        let mut history = std::mem::take(&mut self.history);
        history.extend(state.history.iter().cloned());
        StepKernelOutput {
            history,
            compact_summary: self.compact_summary.take(),
            compaction: self
                .compaction
                .take()
                .expect("step compaction state should be present"),
        }
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
