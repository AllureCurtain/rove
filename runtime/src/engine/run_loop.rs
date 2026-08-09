use std::sync::Arc;

use async_stream::stream;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::capability::CapabilitySnapshot;
use crate::compaction::{CompactionRuntime, maybe_compact_history};
use crate::context::ContextManager;
use crate::engine::control::{SteerLifecycle, SteerMessage};
use crate::environment::ExecutionEnvironment;
use crate::events::StreamEvent;
use crate::execution::{
    ExecutionBudgetTracker, ExecutionLifecycleState, ExecutionPhase, ExecutionPolicy,
    ExecutionStrategy, FinalizationMode, PlanFinishReason,
};
use crate::finalizer::{FinalizationContext, Finalizer};
use crate::hooks::HookRegistry;
use crate::memory::paths::MemoryPaths;
use crate::memory::session::append_session_notes_to_dir_sync;
use crate::model_turn::{ModelTurnItem, run_model_turn};
use crate::prompt_metadata::{
    PromptBuildMetadata, prompt_cache_key, tool_signature, workspace_fingerprint,
};
use crate::state::tool_artifacts::ToolArtifactStore;
use crate::tool_turn::{
    ToolAction, ToolTurnContext, ToolTurnItem, ToolTurnOutcome, append_tool_history, run_tool_turn,
};
use crate::types::{
    ApprovalDecision, ApprovalPolicy, Message, SessionId, TerminationReason, ToolApprovalProvider,
    ToolDescriptor, UserInputProvider,
};
use crate::workspace::Workspace;
use rove_core::{
    AgentKernelHost, KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem,
    KernelLimits, KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction,
    KernelToolTurnItem, ToolRegistry, run_agent_kernel,
};
use rove_models::ModelClient;

/// Shared receiver for in-flight steer messages. Wrapped in Arc<AsyncMutex> so
/// it can be cloned into LoopContext and polled at each safe point without
/// holding a mutable borrow across await boundaries.
pub(crate) type SteerReceiver = Arc<AsyncMutex<tokio::sync::mpsc::Receiver<SteerMessage>>>;

#[derive(Clone)]
pub(crate) struct LoopContext<'a> {
    pub model: &'a dyn ModelClient,
    pub registry: &'a ToolRegistry,
    pub capability_snapshot: &'a CapabilitySnapshot,
    pub context_manager: &'a ContextManager,
    pub workspace: &'a Workspace,
    pub environment: Arc<dyn ExecutionEnvironment>,
    pub memory_paths: &'a MemoryPaths,
    pub session_id: SessionId,
    pub max_steps: u32,
    pub execution_policy: ExecutionPolicy,
    pub finalizer: &'a Finalizer,
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub hooks: HookRegistry,
    pub compaction: CompactionRuntime,
    /// Inbound steer messages drained at the declared safe point (top of each
    /// loop iteration, BEFORE prompt construction for the next model turn).
    /// `None` for runs without a control plane (e.g. direct CLI exec).
    pub steer_rx: Option<SteerReceiver>,
    /// Tracks steers after the safe point and until the next model turn is
    /// actually handed to the model runner.
    pub steer_lifecycle: Option<SteerLifecycle>,
    /// Durable Tool Artifact authority for this run.
    pub tool_artifacts: Option<Arc<ToolArtifactStore>>,
}

impl<'a> LoopContext<'a> {
    pub(crate) fn tool_turn_context(&self, cancel_token: CancellationToken) -> ToolTurnContext<'a> {
        ToolTurnContext {
            registry: self.registry,
            workspace: self.workspace,
            environment: self.environment.clone(),
            memory_paths: self.memory_paths,
            approval_policy: self.approval_policy,
            approval_decision: self.approval_decision,
            approval_provider: self.approval_provider.clone(),
            input_provider: self.input_provider.clone(),
            hooks: self.hooks.clone(),
            cancel_token,
            tool_artifacts: self.tool_artifacts.clone(),
        }
    }
}

pub(crate) fn enrich_prompt_metadata(
    ctx: &LoopContext<'_>,
    mut metadata: PromptBuildMetadata,
    tools: &[ToolDescriptor],
) -> PromptBuildMetadata {
    metadata.workspace_fingerprint = workspace_fingerprint(ctx.workspace);
    metadata.tool_signature = tool_signature(tools);
    metadata.prompt_cache_key = Some(prompt_cache_key(
        &metadata.stable_prefix_hash,
        &metadata.tool_signature,
    ));
    metadata
}

/// Extract durable-worthy notes from messages that are about to be compacted.
pub(crate) fn extract_session_memory_notes(messages: &[Message]) -> Vec<String> {
    let mut notes = Vec::new();

    for msg in messages {
        let content = msg.content.trim();
        if content.is_empty() {
            continue;
        }
        if msg.role == crate::types::Role::Tool {
            let lower = content.to_ascii_lowercase();
            if lower.contains("created")
                || lower.contains("wrote")
                || lower.contains("modified")
                || lower.contains("saved")
            {
                let snippet: String = content.chars().take(160).collect();
                notes.push(format!("tool result: {snippet}"));
            }
        }
        if msg.role == crate::types::Role::Assistant {
            let lower = content.to_ascii_lowercase();
            if lower.contains("i decided")
                || lower.contains("i will")
                || lower.contains("decision:")
                || lower.contains("approach:")
                || lower.contains("plan:")
            {
                let snippet: String = content.chars().take(200).collect();
                notes.push(format!("assistant note: {snippet}"));
            }
        }
    }

    notes.sort();
    notes.dedup();
    notes
}

pub(crate) struct RunLoopState {
    pub user_message: String,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub step: u32,
    pub execution_lifecycle: ExecutionLifecycleState,
}

#[derive(Debug)]
pub(crate) enum LoopItem {
    Event(StreamEvent),
    Complete {
        reason: TerminationReason,
        output: Option<String>,
    },
}

pub(crate) fn run_unplanned_loop<'a>(
    ctx: LoopContext<'a>,
    state: RunLoopState,
    cancel_token: CancellationToken,
) -> BoxStream<'a, LoopItem> {
    Box::pin(stream! {
        let policy = ctx.execution_policy.clone();
        let finalizer = ctx.finalizer;
        let original_goal = state.user_message.clone();
        let mut consumed = state.execution_lifecycle.budget_usage.clone();
        // Old snapshots recorded the React model-turn count only as `step`.
        // Migrate it once without inventing tool/token usage.
        consumed.model_turns = consumed.model_turns.max(state.step);
        let mut budget = ExecutionBudgetTracker::new(
            policy.budgets.clone(),
            consumed,
            false,
        );
        let remaining_turns = policy
            .budgets
            .max_model_turns
            .map(|limit| limit.saturating_sub(budget.usage().model_turns))
            .unwrap_or_else(|| ctx.max_steps.saturating_sub(state.step));
        let remaining_tools = policy
            .budgets
            .max_tool_calls
            .map(|limit| limit.saturating_sub(budget.usage().tool_calls));
        let remaining_repairs = policy
            .budgets
            .max_model_repairs
            .map(|limit| limit.saturating_sub(budget.usage().model_repairs))
            .unwrap_or(remaining_turns);
        let initial_total_tokens = budget.usage().total_tokens;
        let kernel_state = KernelState::new(state.history);
        let host = UnplannedKernelHost {
            ctx,
            user_message: state.user_message,
            working_memory: state.working_memory,
            compact_summary: state.compact_summary,
            compaction: None,
            pending_steer_ids: Vec::new(),
            initial_total_tokens,
            max_total_tokens: policy.budgets.max_total_tokens,
        };
        let mut kernel = run_agent_kernel(
            host,
            kernel_state,
            KernelLimits {
                max_model_turns: Some(remaining_turns),
                max_tool_calls: remaining_tools,
                max_repairs: Some(remaining_repairs),
            },
            cancel_token,
        );

        while let Some(item) = kernel.next().await {
            match item {
                KernelItem::Event(event) => yield LoopItem::Event(event),
                KernelItem::Finished(result) => {
                    let (mut reason, direct_output, mut finish_reason) = match result.termination {
                        KernelTermination::Final { output } => {
                            (TerminationReason::Final, Some(output), PlanFinishReason::Completed)
                        }
                        KernelTermination::ModelTurnLimit
                        | KernelTermination::ToolCallLimit
                        | KernelTermination::RepairLimit => {
                            (TerminationReason::StepLimit, None, PlanFinishReason::BudgetExhausted)
                        }
                        KernelTermination::Cancelled => (
                            TerminationReason::Cancelled,
                            None,
                            PlanFinishReason::Cancelled,
                        ),
                        KernelTermination::ModelFailed(error) => (
                            TerminationReason::Error,
                            Some(format!("Model error: {error}")),
                            PlanFinishReason::Failed,
                        ),
                        KernelTermination::IncompleteBeforeModelTurn => (
                            TerminationReason::Error,
                            Some("before-model extension ended without a request".to_string()),
                            PlanFinishReason::Failed,
                        ),
                        KernelTermination::IncompleteModelTurn => (
                            TerminationReason::Error,
                            Some("model turn ended without a response".to_string()),
                            PlanFinishReason::Failed,
                        ),
                        KernelTermination::IncompleteToolTurn => (
                            TerminationReason::Error,
                            Some("tool turn ended without a result".to_string()),
                            PlanFinishReason::Indeterminate,
                        ),
                        KernelTermination::Extension {
                            reason: RuntimeKernelStop::TokenLimit,
                            output,
                        } => (
                            TerminationReason::TokenLimit,
                            output,
                            PlanFinishReason::BudgetExhausted,
                        ),
                    };
                    if let Err(exhaustion) = budget.record_step_usage(
                        result.state.model_turns,
                        result.state.tool_calls,
                        result.state.repairs,
                        &result.state.usage,
                    ) {
                        budget.mark_exhausted(exhaustion);
                        reason = TerminationReason::TokenLimit;
                        finish_reason = PlanFinishReason::BudgetExhausted;
                    }
                    yield LoopItem::Event(StreamEvent::ExecutionBudgetUpdated {
                        phase: ExecutionPhase::Run,
                        snapshot: Box::new(budget.snapshot()),
                    });

                    let context = FinalizationContext {
                        original_goal: &original_goal,
                        strategy: ExecutionStrategy::React,
                        finish_reason,
                        revisions: &[],
                        records: &[],
                        budget: budget.usage(),
                        direct_output: direct_output.as_deref(),
                    };
                    let mode = if finish_reason == PlanFinishReason::Completed {
                        FinalizationMode::Direct
                    } else {
                        FinalizationMode::Deterministic
                    };
                    let started = finalizer.started_record(&context, mode);
                    yield LoopItem::Event(StreamEvent::FinalizationStarted {
                        record: Box::new(started.clone()),
                    });
                    let finalized = if mode == FinalizationMode::Direct {
                        finalizer.direct(&context, started, budget.usage().clone())
                    } else {
                        finalizer.deterministic(
                            &context,
                            started,
                            false,
                            budget.usage().clone(),
                        )
                    };
                    let output = finalized.record.output.clone();
                    yield LoopItem::Event(StreamEvent::FinalizationCompleted {
                        record: Box::new(finalized.record),
                    });
                    yield LoopItem::Complete { reason, output };
                    return;
                }
            }
        }

        yield LoopItem::Complete {
            reason: TerminationReason::Error,
            output: Some("shared Agent kernel ended without completion".to_string()),
        };
    })
}

#[derive(Debug)]
enum RuntimeKernelStop {
    TokenLimit,
}

struct UnplannedKernelHost<'a> {
    ctx: LoopContext<'a>,
    user_message: String,
    working_memory: Vec<Message>,
    compact_summary: Option<String>,
    compaction: Option<CompactionRuntime>,
    pending_steer_ids: Vec<String>,
    initial_total_tokens: u64,
    max_total_tokens: Option<u64>,
}

impl AgentKernelHost for UnplannedKernelHost<'_> {
    type Event = StreamEvent;
    type Stop = RuntimeKernelStop;
    type ToolOutcome = ToolTurnOutcome;
    type Output = ();

    fn before_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelBeforeModelTurnItem<Self::Event, Self::Stop>> {
        Box::pin(stream! {
            if self.max_total_tokens.is_some_and(|limit| {
                self.initial_total_tokens
                    .saturating_add(u64::from(state.usage.total_tokens))
                    >= limit
            }) {
                yield KernelBeforeModelTurnItem::Stop {
                    reason: RuntimeKernelStop::TokenLimit,
                    output: Some("global model token budget exhausted".to_string()),
                };
                return;
            }
            if let Some(rx) = self.ctx.steer_rx.as_ref() {
                let mut receiver = rx.lock().await;
                while let Ok(steer) = receiver.try_recv() {
                    let id = steer.id.0;
                    self.working_memory
                        .push(Message::user(steer.content.clone()));
                    if let Some(lifecycle) = self.ctx.steer_lifecycle.as_ref() {
                        lifecycle.accepted(id.clone()).await;
                    }
                    yield KernelBeforeModelTurnItem::Event(StreamEvent::SteerAccepted {
                        id: id.clone(),
                        content: steer.content,
                    });
                    self.pending_steer_ids.push(id);
                }
            }

            let context = self.ctx.context_manager.build_with_checkpoint(
                &self.user_message,
                &self.working_memory,
                self.compact_summary.as_deref(),
                &state.history,
            );
            let tool_schemas = self.ctx.registry.descriptors();
            yield KernelBeforeModelTurnItem::Event(StreamEvent::PromptBuilt {
                metadata: enrich_prompt_metadata(
                    &self.ctx,
                    context.metadata.clone(),
                    &tool_schemas,
                ),
            });
            if context.over_hard_limit {
                yield KernelBeforeModelTurnItem::Stop {
                    reason: RuntimeKernelStop::TokenLimit,
                    output: Some("context exceeds configured hard token budget".to_string()),
                };
                return;
            }

            if context.auto_compaction_needed && context.dropped_history_messages > 0 {
                let compacted_count = context.dropped_history_messages.min(state.history.len());
                let mut flush_notes = Vec::new();
                if compacted_count > 0 {
                    let candidate_notes =
                        extract_session_memory_notes(&state.history[..compacted_count]);
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

                let compaction = self
                    .compaction
                    .get_or_insert_with(|| self.ctx.compaction.clone());
                if let Some(update) = maybe_compact_history(
                    compaction,
                    self.ctx.model,
                    &state.history[..compacted_count],
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
            }

            yield KernelBeforeModelTurnItem::Ready(context.messages);
        })
    }

    fn model_turn<'a>(
        &'a mut self,
        messages: Vec<Message>,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelModelTurnItem<Self::Event>> {
        let accepted = std::mem::take(&mut self.pending_steer_ids);
        run_kernel_model_turn(
            self.ctx.model,
            self.ctx.registry,
            messages,
            cancel_token,
            accepted,
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
        Box::pin(stream! {
            let mut inner = run_tool_turn(self.ctx.tool_turn_context(cancel_token), action);
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
        _outcome: &'a Self::ToolOutcome,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(()) })
    }

    fn after_final<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _output: &'a str,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<KernelFinalAction, Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(KernelFinalAction::Complete) })
    }

    fn finish_output(&mut self, _state: &KernelState) -> Self::Output {}
}

pub(crate) fn run_kernel_model_turn<'a>(
    model: &'a dyn ModelClient,
    registry: &'a ToolRegistry,
    messages: Vec<Message>,
    cancel_token: CancellationToken,
    accepted_steer_ids: Vec<String>,
    steer_lifecycle: Option<SteerLifecycle>,
) -> BoxStream<'a, KernelModelTurnItem<StreamEvent>> {
    Box::pin(stream! {
        let mut inner = run_model_turn(
            model,
            messages,
            registry.model_schemas(),
            cancel_token,
        );
        let mut applied = false;
        while let Some(item) = inner.next().await {
            if !applied {
                for id in &accepted_steer_ids {
                    if let Some(lifecycle) = steer_lifecycle.as_ref() {
                        lifecycle.applied(id).await;
                    }
                    yield KernelModelTurnItem::Event(StreamEvent::SteerApplied {
                        id: id.clone(),
                    });
                }
                applied = true;
            }
            match item {
                ModelTurnItem::Event(event) => yield KernelModelTurnItem::Event(event),
                ModelTurnItem::Finished(turn) => {
                    yield KernelModelTurnItem::Finished(turn);
                    return;
                }
                ModelTurnItem::Cancelled => {
                    yield KernelModelTurnItem::Cancelled;
                    return;
                }
                ModelTurnItem::Failed(error) => {
                    yield KernelModelTurnItem::Failed(error);
                    return;
                }
            }
        }
    })
}
