use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_stream::stream;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::agents::{
    AgentRuntimeProfile, ScopedInstructionPrompt, scoped_instruction_path_hints,
    scoped_instruction_prompt,
};
use crate::capability::CapabilitySnapshot;
use crate::compaction::{CompactionRuntime, CompactionTrigger, maybe_compact_history};
use crate::context::ContextManager;
use crate::engine::control::{
    AcceptedSteer, SteerLifecycle, SteerMessage, steer_accepted_event, steer_applied_event,
};
use crate::engine::recovery::{ProviderRetryPolicy, RetryClass, retry_after_ms, retry_class};
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
    ToolAction, ToolTurnContext, ToolTurnItem, ToolTurnOutcome, append_tool_history,
    defer_tool_turn, run_tool_turn, workspace_target_paths,
};
use crate::types::{
    ApprovalDecision, ApprovalPolicy, Message, RunMode, SessionId, TerminationReason,
    ToolApprovalProvider, ToolDescriptor, UserInputProvider,
};
use crate::workspace::Workspace;
use rove_core::{
    AgentKernelHost, KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem,
    KernelLimits, KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction,
    KernelToolTurnItem, ToolRegistry, run_agent_kernel,
};
use rove_models::{ModelClient, ModelError, ModelToolSchema};

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
    /// Model-call retry budget applied at the model-turn boundary.
    pub provider_retry: ProviderRetryPolicy,
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
    /// Exact Agent snapshot for capability filtering and resume identity.
    pub agent_profile: Option<Arc<AgentRuntimeProfile>>,
    /// Content-free Agent/procedure summary supplied to the Planner.
    pub agent_planner_summary: Option<String>,
    /// Run-local deduplication for content-free overlay application events.
    pub instruction_overlays_seen: Arc<Mutex<BTreeSet<String>>>,
    pub run_mode: RunMode,
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
            agent_profile: self.agent_profile.clone(),
            run_mode: self.run_mode,
        }
    }

    pub(crate) fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.registry
            .descriptors()
            .into_iter()
            .filter(|descriptor| self.descriptor_allowed(descriptor))
            .collect()
    }

    pub(crate) fn model_schemas(&self) -> Vec<ModelToolSchema> {
        self.descriptors()
            .iter()
            .map(ToolDescriptor::model_schema)
            .collect()
    }

    fn descriptor_allowed(&self, descriptor: &ToolDescriptor) -> bool {
        let Some(profile) = self.agent_profile.as_ref() else {
            return true;
        };
        match descriptor.capability_id.as_deref() {
            Some(capability) => profile.effective_capabilities.contains(capability),
            None => profile.is_legacy(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActiveInstructionTarget {
    paths: Vec<String>,
    boundary: &'static str,
    call_id: Option<crate::types::CallId>,
}

impl ActiveInstructionTarget {
    pub(crate) fn from_text(ctx: &LoopContext<'_>, text: &str, boundary: &'static str) -> Self {
        let paths = ctx
            .agent_profile
            .as_deref()
            .map(|profile| scoped_instruction_path_hints(profile, text))
            .unwrap_or_default();
        Self {
            paths,
            boundary,
            call_id: None,
        }
    }

    pub(crate) fn for_tool(paths: Vec<String>, call_id: Option<crate::types::CallId>) -> Self {
        Self {
            paths,
            boundary: "tool_call",
            call_id,
        }
    }
}

pub(crate) fn scoped_prompt_for_target(
    ctx: &LoopContext<'_>,
    target: &ActiveInstructionTarget,
) -> ScopedInstructionPrompt {
    ctx.agent_profile
        .as_deref()
        .map(|profile| scoped_instruction_prompt(profile, &target.paths))
        .unwrap_or_default()
}

pub(crate) fn scoped_prompt_events(
    ctx: &LoopContext<'_>,
    target: &ActiveInstructionTarget,
    prompt: &ScopedInstructionPrompt,
) -> Vec<StreamEvent> {
    let mut seen = ctx
        .instruction_overlays_seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prompt
        .applications
        .iter()
        .filter_map(|application| {
            let key = format!("{}#{}", application.scope, application.content_hash);
            seen.insert(key)
                .then(|| StreamEvent::InstructionOverlayApplied {
                    target_path: application.target_path.clone(),
                    scope: application.scope.clone(),
                    source_path: application.source_path.clone(),
                    content_hash: application.content_hash.clone(),
                    boundary: target.boundary.to_string(),
                    call_id: target.call_id,
                })
        })
        .collect()
}

pub(crate) fn scoped_paths_for_action(ctx: &LoopContext<'_>, action: &ToolAction) -> Vec<String> {
    let mut paths = workspace_target_paths(action);
    if let Some(profile) = ctx.agent_profile.as_deref() {
        for call in action
            .calls()
            .into_iter()
            .filter(|call| call.name == "run_shell")
        {
            if call.args.get("paths").is_some() {
                continue;
            }
            if let Some(command) = call.args.get("command").and_then(serde_json::Value::as_str) {
                paths.extend(scoped_instruction_path_hints(profile, command));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn shell_path_declaration_missing(ctx: &LoopContext<'_>, action: &ToolAction) -> bool {
    let has_nested_overlays = ctx
        .agent_profile
        .as_deref()
        .and_then(|profile| profile.instructions.as_ref())
        .is_some_and(|bundle| !bundle.overlays.is_empty());
    has_nested_overlays
        && action.calls().into_iter().any(|call| {
            if call.name != "run_shell" {
                return false;
            }
            if let Some(declared) = call.args.get("paths") {
                return declared.as_array().is_none_or(|paths| {
                    paths.is_empty()
                        || paths.iter().any(|path| {
                            path.as_str()
                                .and_then(crate::agents::instructions::normalize_workspace_target)
                                .is_none()
                        })
                });
            }
            call.args
                .get("command")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|command| {
                    ctx.agent_profile.as_deref().is_none_or(|profile| {
                        scoped_instruction_path_hints(profile, command).is_empty()
                    })
                })
        })
}

pub(crate) fn active_target_covers(
    ctx: &LoopContext<'_>,
    active: &ActiveInstructionTarget,
    requested_paths: &[String],
) -> bool {
    let active_prompt = scoped_prompt_for_target(ctx, active);
    let requested = ctx
        .agent_profile
        .as_deref()
        .map(|profile| scoped_instruction_prompt(profile, requested_paths))
        .unwrap_or_default();
    if !requested.omitted_scopes.is_empty() {
        return false;
    }
    let active_hashes: BTreeSet<&str> = active_prompt
        .applications
        .iter()
        .map(|application| application.content_hash.as_str())
        .collect();
    requested
        .applications
        .iter()
        .all(|application| active_hashes.contains(application.content_hash.as_str()))
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

pub(crate) fn runtime_guidance(ctx: &LoopContext<'_>) -> Message {
    let descriptors = ctx.descriptors();
    let tool_names = descriptors
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let tool_contract = if ctx.model.compatibility_text_tool_calls() {
        "This provider requires the legacy compatibility JSON tool-call envelope. Emit exactly one bounded {\"tool\":...,\"args\":{...}} object when calling a tool; malformed output is recoverable and must be corrected."
    } else {
        "Request tools only through the provider's native structured tool-call channel. Never print a JSON tool envelope as assistant text."
    };
    Message::system(format!(
        "## Runtime execution facts\n{tool_contract}\nWorkspace kind: {:?}. Paths are workspace-relative; discovery is bounded and ignore-aware. Tool results report truncation and references explicitly. Before edit_file, read the same path with explicit offset and limit and use only that structured read_file result's observation_id and version; artifact hashes and search or directory observations are not file-mutation observations. Schema errors name the field and a deterministic correction; retry the same tool when appropriate. Instructions, procedures, retrieval, and tool descriptions are guidance only and never grant permission. Available tools ({}): {}. Execution remains bounded by the active public budgets and approval policy.",
        ctx.workspace.kind,
        descriptors.len(),
        tool_names,
    ))
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
        let active_instruction_target =
            ActiveInstructionTarget::from_text(&ctx, &state.user_message, "user_task");
        let host = UnplannedKernelHost {
            ctx,
            user_message: state.user_message,
            working_memory: state.working_memory,
            compact_summary: state.compact_summary,
            compaction: None,
            pending_steer_ids: Vec::new(),
            initial_total_tokens,
            max_total_tokens: policy.budgets.max_total_tokens,
            active_instruction_target,
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
    pending_steer_ids: Vec<AcceptedSteer>,
    initial_total_tokens: u64,
    max_total_tokens: Option<u64>,
    active_instruction_target: ActiveInstructionTarget,
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

            let scoped = scoped_prompt_for_target(&self.ctx, &self.active_instruction_target);
            for event in scoped_prompt_events(
                &self.ctx,
                &self.active_instruction_target,
                &scoped,
            ) {
                yield KernelBeforeModelTurnItem::Event(event);
            }
            let mut turn_working_memory = self.working_memory.clone();
            turn_working_memory.push(runtime_guidance(&self.ctx));
            turn_working_memory.extend(scoped.messages);
            let mut context = self.ctx.context_manager.build_with_checkpoint(
                &self.user_message,
                &turn_working_memory,
                self.compact_summary.as_deref(),
                &state.history,
            );
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
                    CompactionTrigger::Automatic,
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

                // Rebuilt so the summary reaches the model on the turn that
                // dropped the history it stands for. Without this the turn goes
                // out with the history gone and nothing in its place, and the
                // summary only lands on the next one. PlanReact already
                // rebuilds here; the two loops now agree.
                context = self.ctx.context_manager.build_with_checkpoint(
                    &self.user_message,
                    &turn_working_memory,
                    self.compact_summary.as_deref(),
                    &state.history,
                );
                if context.over_hard_limit {
                    yield KernelBeforeModelTurnItem::Stop {
                        reason: RuntimeKernelStop::TokenLimit,
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
        let accepted = std::mem::take(&mut self.pending_steer_ids);
        run_kernel_model_turn(
            self.ctx.model,
            self.ctx.model_schemas(),
            messages,
            cancel_token,
            accepted,
            self.ctx.steer_lifecycle.clone(),
            ModelTurnOptions {
                run_mode: self.ctx.run_mode,
                retry_policy: self.ctx.provider_retry.clone(),
            },
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

/// Per-model-call options the host resolves before the call.
///
/// Grouping them keeps the model-turn signature stable as recovery behavior
/// grows, and states explicitly that a retry reuses the same options instead of
/// re-deriving them from a mutated turn.
#[derive(Debug, Clone)]
pub(crate) struct ModelTurnOptions {
    pub run_mode: RunMode,
    pub retry_policy: ProviderRetryPolicy,
}

/// Decide whether a failed attempt may be retried, charging its class budget.
///
/// Returns the retry notice facts `(delay_ms, reason, attempt, max_attempts)`
/// and advances the matching counter, or `None` when the failure is terminal for
/// this model call. `attempt` is 1-based and counts the first call, so the first
/// retry is attempt 2.
fn plan_retry(
    error: &ModelError,
    output_started: bool,
    policy: &ProviderRetryPolicy,
    rate_limit_attempts: &mut u32,
    transient_attempts: &mut u32,
) -> Option<(u64, String, u32, u32)> {
    if output_started {
        return None;
    }
    let class = retry_class(error)?;
    let spent = match class {
        RetryClass::RateLimited => *rate_limit_attempts,
        RetryClass::Transient => *transient_attempts,
    };
    let max_attempts = policy.max_attempts(class);
    if spent >= max_attempts {
        return None;
    }
    let attempt = spent + 1;
    let delay_ms = policy.delay_ms(attempt, retry_after_ms(error));
    let reason = ProviderRetryPolicy::reason(class, error);
    match class {
        RetryClass::RateLimited => *rate_limit_attempts = attempt,
        RetryClass::Transient => *transient_attempts = attempt,
    }
    Some((delay_ms, reason, attempt, max_attempts))
}

pub(crate) fn run_kernel_model_turn<'a>(
    model: &'a dyn ModelClient,
    tool_schemas: Vec<ModelToolSchema>,
    messages: Vec<Message>,
    cancel_token: CancellationToken,
    accepted_steer_ids: Vec<AcceptedSteer>,
    steer_lifecycle: Option<SteerLifecycle>,
    options: ModelTurnOptions,
) -> BoxStream<'a, KernelModelTurnItem<StreamEvent>> {
    Box::pin(stream! {
        let ModelTurnOptions { run_mode, retry_policy } = options;
        // Steer application is a per-call fact: the retried attempt is the same
        // model call, so an accepted steer is announced exactly once.
        let mut applied = false;
        // A retry is only safe while the turn has produced nothing the user can
        // see. Once a chunk is streamed, a failure ends the turn instead of
        // re-generating text that may already be on screen.
        let mut output_started = false;
        // Attempts already spent, per independent budget. The first call counts
        // as attempt 1, so both start at 1.
        let mut rate_limit_attempts: u32 = 1;
        let mut transient_attempts: u32 = 1;
        // The retry inputs are kept only when a retry is possible at all, so a
        // deployment that disabled the budget pays nothing for it. The first
        // attempt uses the request as given; a retry re-sends the identical
        // request rather than re-deriving it from a mutated turn. That copy is
        // unavoidable while the budget is enabled: the turn stream owns its
        // inputs, so the only way to repeat the same request is to have kept an
        // unmodified one. An enabled-but-never-failing call therefore pays one
        // clone of the bounded message view and tool schemas per model turn.
        let retry_inputs = retry_policy
            .is_enabled()
            .then(|| (messages.clone(), tool_schemas.clone()));
        let mut next_messages = Some(messages);
        let mut next_tool_schemas = Some(tool_schemas);

        loop {
            let (attempt_messages, attempt_schemas) =
                match (next_messages.take(), next_tool_schemas.take()) {
                    (Some(messages), Some(schemas)) => (messages, schemas),
                    _ => {
                        let (messages, schemas) = retry_inputs
                            .as_ref()
                            .expect("a retry keeps its request inputs while the budget is enabled");
                        (messages.clone(), schemas.clone())
                    }
                };

            let mut inner = run_model_turn(
                model,
                attempt_messages,
                attempt_schemas,
                cancel_token.clone(),
                run_mode,
            );
            // Set when this attempt schedules another one instead of ending.
            let mut retry: Option<(u64, String, u32, u32)> = None;

            while let Some(item) = inner.next().await {
                if !applied {
                    for accepted in &accepted_steer_ids {
                        if let Some(lifecycle) = steer_lifecycle.as_ref() {
                            lifecycle.applied(&accepted.id).await;
                        }
                        yield KernelModelTurnItem::Event(steer_applied_event(accepted));
                    }
                    applied = true;
                }
                match item {
                    ModelTurnItem::Event(event) => {
                        if matches!(event, StreamEvent::LlmChunk { .. }) {
                            output_started = true;
                        }
                        yield KernelModelTurnItem::Event(event);
                    }
                    ModelTurnItem::Finished(turn) => {
                        yield KernelModelTurnItem::Finished(turn);
                        return;
                    }
                    ModelTurnItem::Cancelled => {
                        yield KernelModelTurnItem::Cancelled;
                        return;
                    }
                    ModelTurnItem::Failed(error) => {
                        if let Some(planned) = plan_retry(
                            &error,
                            output_started,
                            &retry_policy,
                            &mut rate_limit_attempts,
                            &mut transient_attempts,
                        ) {
                            retry = Some(planned);
                            break;
                        }
                        yield KernelModelTurnItem::Failed(error);
                        return;
                    }
                }
            }

            let Some((delay_ms, reason, attempt, max_attempts)) = retry else {
                // The inner stream ended without a terminal item, which the
                // model-turn contract does not allow. Ending the turn here keeps
                // the pre-budget behavior for that case.
                return;
            };

            yield KernelModelTurnItem::Event(StreamEvent::ProviderRetry {
                attempt,
                max_attempts,
                delay_ms,
                reason,
                phase: "model_call".to_string(),
            });
            if delay_ms > 0 {
                // The wait is cancellable: cancelling during a backoff ends the
                // turn immediately instead of leaving a pending sleep behind.
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        yield KernelModelTurnItem::Cancelled;
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                }
            }
            if cancel_token.is_cancelled() {
                yield KernelModelTurnItem::Cancelled;
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use rove_models::{FakeModelClient, FakeTurn, ModelError};

    use super::*;

    fn retry_immediately() -> ProviderRetryPolicy {
        ProviderRetryPolicy {
            backoff_base_ms: 1,
            backoff_max_ms: 1,
            jitter_ratio: 0.0,
            ..ProviderRetryPolicy::default()
        }
    }

    /// An accepted steer belongs to the model call, not to one attempt, so a
    /// retry must announce it exactly once instead of replaying it.
    #[tokio::test]
    async fn a_retried_model_turn_announces_an_accepted_steer_once() {
        let model = FakeModelClient::with_turns(
            "fallback".to_string(),
            vec![
                FakeTurn::Fail(ModelError::RequestFailed("reset".to_string())),
                FakeTurn::Text("recovered".to_string()),
            ],
        );
        let accepted = vec![AcceptedSteer {
            id: "steer-1".to_string(),
            unified_message: false,
        }];
        let mut stream = run_kernel_model_turn(
            &model,
            Vec::new(),
            vec![crate::types::Message::user("answer the request")],
            CancellationToken::new(),
            accepted,
            None,
            ModelTurnOptions {
                run_mode: RunMode::Normal,
                retry_policy: retry_immediately(),
            },
        );

        let mut applied = 0;
        let mut retries = 0;
        let mut finished = 0;
        while let Some(item) = stream.next().await {
            match item {
                KernelModelTurnItem::Event(StreamEvent::SteerApplied { .. }) => applied += 1,
                KernelModelTurnItem::Event(StreamEvent::ProviderRetry { .. }) => retries += 1,
                KernelModelTurnItem::Finished(_) => finished += 1,
                _ => {}
            }
        }

        assert_eq!(applied, 1, "a retry must not replay the accepted steer");
        assert_eq!(retries, 1, "one failed attempt means one retry notice");
        assert_eq!(finished, 1, "the recovered attempt must finish the turn");
    }

    /// The budget is spent per class and never after output exists, so a
    /// streaming turn ends the moment its first failure is terminal.
    #[tokio::test]
    async fn output_before_failure_ends_the_turn_with_the_provider_error() {
        let model = FakeModelClient::with_turns(
            "fallback".to_string(),
            vec![FakeTurn::TextThenFail {
                text: "partial".to_string(),
                error: ModelError::StreamInterrupted("closed".to_string()),
            }],
        );
        let mut stream = run_kernel_model_turn(
            &model,
            Vec::new(),
            vec![crate::types::Message::user("answer the request")],
            CancellationToken::new(),
            Vec::new(),
            None,
            ModelTurnOptions {
                run_mode: RunMode::Normal,
                retry_policy: retry_immediately(),
            },
        );

        let mut retries = 0;
        let mut failure = None;
        while let Some(item) = stream.next().await {
            match item {
                KernelModelTurnItem::Event(StreamEvent::ProviderRetry { .. }) => retries += 1,
                KernelModelTurnItem::Failed(error) => failure = Some(error),
                _ => {}
            }
        }

        assert_eq!(retries, 0, "a failure after output must not retry");
        assert!(
            matches!(failure, Some(ModelError::StreamInterrupted(_))),
            "the provider's own error must be the one the caller sees"
        );
    }

    /// Charging is per class and per model call: exhausting one class must not
    /// spend another, and output before the failure ends the call outright.
    #[test]
    fn retry_planning_charges_only_the_failing_class() {
        let policy = ProviderRetryPolicy {
            rate_limit_max_attempts: 2,
            transient_max_attempts: 2,
            backoff_base_ms: 10,
            // Above the provider's retry-after so the honoured value is visible
            // here; the ceiling clamp has its own policy test.
            backoff_max_ms: 1_000,
            jitter_ratio: 0.0,
        };
        let mut rate_limit = 1;
        let mut transient = 1;
        let transient_error = ModelError::RequestFailed("reset".to_string());
        let rate_limited = ModelError::RateLimited {
            retry_after_ms: 250,
        };

        let first = plan_retry(
            &transient_error,
            false,
            &policy,
            &mut rate_limit,
            &mut transient,
        )
        .expect("the first transient failure earns a retry");
        assert_eq!(
            (first.0, first.1, first.2, first.3),
            (10, "transient:request_failed".to_string(), 2, 2)
        );
        assert_eq!(
            (rate_limit, transient),
            (1, 2),
            "only the failing class is charged"
        );

        assert!(
            plan_retry(
                &transient_error,
                false,
                &policy,
                &mut rate_limit,
                &mut transient
            )
            .is_none(),
            "the transient budget is now spent"
        );
        let rate = plan_retry(
            &rate_limited,
            false,
            &policy,
            &mut rate_limit,
            &mut transient,
        )
        .expect("the rate-limit budget is untouched");
        assert_eq!(
            (rate.0, rate.1, rate.2, rate.3),
            (250, "rate_limited".to_string(), 2, 2),
            "a provider retry-after is used as given"
        );
        assert!(
            plan_retry(
                &transient_error,
                true,
                &policy,
                &mut rate_limit,
                &mut transient
            )
            .is_none(),
            "output before the failure makes the same failure terminal"
        );
    }
}
