use std::{
    collections::BTreeSet,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use async_stream::stream;
use futures::stream::{BoxStream, Stream, StreamExt};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::agents::definition::PromptSlotRole;
use crate::agents::{
    AgentActivationConfig, AgentActivationError, AgentRuntime, AgentRuntimeProfile,
    ResolvedRuntimeFacts,
};
use crate::capability::CapabilitySnapshot;
use crate::compaction::CompactionRuntime;
use crate::context::{ContextManager, durable_memory_message, session_summary_message};
use crate::engine::control::{RunControlHandle, SteerLifecycle, control_channel};
use crate::environment::{ExecutionEnvironment, local_environment};
use crate::events::StreamEvent;
use crate::execution::{ExecutionPolicy, ExecutionStrategy};
use crate::finalizer::Finalizer;
use crate::hooks::{HookRegistry, PostRunHookContext, RunSummary};
use crate::memory::layered::load_prompt_memory_from_paths_sync;
use crate::memory::paths::MemoryPaths;
use crate::plan_evaluator::PlanEvaluator;
use crate::plan_loop::{PlanLoopState, run_planned_loop};
use crate::planner::Planner;
use crate::run_loop::{LoopContext, LoopItem, RunLoopState, SteerReceiver, run_unplanned_loop};
use crate::runtime_identity::{
    RuntimeIdentity, RuntimeIdentityInput, RuntimeIdentityStatus, build_runtime_identity,
};
use crate::session::CHECKPOINT_SESSION_TAIL_ENTRIES;
use crate::state::tool_artifacts::ToolArtifactStore;
use crate::state::trace::TraceWriter;
use crate::tools::mcp_proxy::{McpLifecycleFact, McpRuntimeState, McpServerRuntimeSnapshot};
use crate::types::{
    ApprovalDecision, ApprovalPolicy, JobId, Message, RunId, RunRequest, SessionId,
    TerminationReason, ToolApprovalProvider, ToolDescriptor, UserInputProvider,
};
use crate::workspace::Workspace;
use rove_core::ToolRegistry;
use rove_models::ModelClient;

/// A running engine stream plus immediate identity, cancellation, and a
/// control handle for steer/follow-up submission.
pub struct RunStream<'e> {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    cancel_token: CancellationToken,
    control: RunControlHandle,
    runtime_identity: RuntimeIdentity,
    agent_profile: Option<AgentRuntimeProfile>,
    inner: Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'e>>,
}

impl RunStream<'_> {
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Handle for submitting steer/followup messages to the in-flight run.
    pub fn control(&self) -> &RunControlHandle {
        &self.control
    }

    /// Exact runtime identity for this run, including the resolved Agent.
    pub fn runtime_identity(&self) -> &RuntimeIdentity {
        &self.runtime_identity
    }

    /// Exact Agent snapshot retained by task/checkpoint persistence.
    pub fn agent_profile(&self) -> Option<&AgentRuntimeProfile> {
        self.agent_profile.as_ref()
    }
}

impl Stream for RunStream<'_> {
    type Item = StreamEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

impl Unpin for RunStream<'_> {}

impl Drop for RunStream<'_> {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Configuration for the engine's execution limits and planner prompt.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_steps: u32,
    pub plan_enabled: bool,
    /// Fully resolved execution policy. When present it is authoritative and
    /// the `max_steps` / `plan_enabled` sugar is used only for compatibility
    /// projections. When absent the policy is derived from that sugar.
    pub execution_policy: Option<ExecutionPolicy>,
}

/// Invocation-scoped authority used when constructing an Engine for a
/// workspace. Keeping the environment beside the approval settings makes it
/// explicit that both are shared by the entire run.
pub struct EngineEnvironmentOptions {
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
    pub environment: Arc<dyn ExecutionEnvironment>,
}

impl EngineConfig {
    /// Build a config from the compatibility sugar fields, deriving the
    /// execution policy deterministically.
    ///
    /// Callers that resolve a full policy (for example from operator
    /// configuration) use [`EngineConfig::with_execution_policy`] instead.
    pub fn new(max_steps: u32, plan_enabled: bool) -> Self {
        Self {
            max_steps,
            plan_enabled,
            execution_policy: None,
        }
    }

    /// Attach a fully resolved policy, which then takes precedence over the
    /// sugar fields.
    pub fn with_execution_policy(mut self, policy: ExecutionPolicy) -> Self {
        self.execution_policy = Some(policy);
        self
    }

    /// Project sugar fields into the typed policy used by the engine.
    ///
    /// `max_steps` / `plan_enabled` remain convenience inputs; `ExecutionPolicy`
    /// is the sole execution-config truth.
    pub fn to_execution_policy(&self) -> ExecutionPolicy {
        self.execution_policy.clone().unwrap_or_else(|| {
            ExecutionPolicy::from_max_steps_and_plan_flag(self.max_steps, self.plan_enabled)
        })
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_steps: 20,
            plan_enabled: false,
            execution_policy: None,
        }
    }
}

/// The core engine that drives the agent loop.
///
/// Owns the model client, tool registry, context manager, and config.
/// Produces a `Stream<Item = StreamEvent>` that any interface can consume.
pub struct Engine {
    model: Box<dyn ModelClient>,
    registry: ToolRegistry,
    capability_snapshot: CapabilitySnapshot,
    context_manager: ContextManager,
    config: EngineConfig,
    planner: Planner,
    evaluator: PlanEvaluator,
    finalizer: Finalizer,
    execution_policy: ExecutionPolicy,
    workspace: Workspace,
    environment: Arc<dyn ExecutionEnvironment>,
    approval_policy: ApprovalPolicy,
    approval_decision: ApprovalDecision,
    approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    hooks: HookRegistry,
    memory_paths: MemoryPaths,
    model_compaction_enabled: bool,
    compaction_failure_threshold: u32,
    agent_runtime: AgentRuntime,
}

impl Engine {
    pub fn new(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
    ) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let workspace = Workspace::detect(&cwd).unwrap_or_else(|_| Workspace {
            root: cwd.clone(),
            kind: crate::workspace::WorkspaceKind::Folder,
            state_dir: cwd.join(".rove"),
        });

        Self::with_workspace_and_approval_decision(
            model,
            registry,
            context_manager,
            config,
            workspace,
            ApprovalPolicy::Auto,
            ApprovalDecision::Approve,
        )
    }

    pub fn with_workspace(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
        workspace: Workspace,
        approval_policy: ApprovalPolicy,
    ) -> Self {
        Self::with_workspace_and_approval_decision(
            model,
            registry,
            context_manager,
            config,
            workspace,
            approval_policy,
            ApprovalDecision::Reject,
        )
    }

    pub fn with_workspace_and_approval_decision(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
        workspace: Workspace,
        approval_policy: ApprovalPolicy,
        approval_decision: ApprovalDecision,
    ) -> Self {
        let environment = local_environment(&workspace);
        Self::with_workspace_and_approval_decision_and_environment(
            model,
            registry,
            context_manager,
            config,
            workspace,
            EngineEnvironmentOptions {
                approval_policy,
                approval_decision,
                environment,
            },
        )
    }

    pub fn with_workspace_and_approval_decision_and_environment(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
        workspace: Workspace,
        options: EngineEnvironmentOptions,
    ) -> Self {
        let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
        let capability_snapshot = CapabilitySnapshot::from_registry(&registry);
        let execution_policy = config.to_execution_policy();
        let agent_runtime = AgentRuntime::load(
            &workspace,
            AgentActivationConfig::default(),
            resolved_agent_facts(model.as_ref(), &registry, None),
        )
        .expect("builtin legacy Agent activation is infallible");
        Self {
            model,
            registry,
            capability_snapshot,
            context_manager,
            config,
            planner: Planner::default(),
            evaluator: PlanEvaluator::default(),
            finalizer: Finalizer::default(),
            execution_policy,
            workspace,
            environment: options.environment,
            approval_policy: options.approval_policy,
            approval_decision: options.approval_decision,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::with_default_post_run_hooks(),
            memory_paths,
            model_compaction_enabled: false,
            compaction_failure_threshold: 3,
            agent_runtime,
        }
    }

    pub fn with_hooks(mut self, hooks: HookRegistry) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn with_planner_prompt(mut self, planner_prompt: impl Into<String>) -> Self {
        self.planner = Planner::new(planner_prompt);
        self
    }

    pub fn with_evaluator_prompt(mut self, evaluator_prompt: impl Into<String>) -> Self {
        self.evaluator = PlanEvaluator::new(evaluator_prompt);
        self
    }

    pub fn with_finalizer_prompt(mut self, finalizer_prompt: impl Into<String>) -> Self {
        self.finalizer = Finalizer::new(finalizer_prompt);
        self
    }

    /// Replace compatibility sugar with a fully resolved public execution
    /// policy. Validation happens before the Engine can start a run.
    pub fn with_execution_policy(
        mut self,
        policy: ExecutionPolicy,
    ) -> Result<Self, crate::execution::ExecutionValidationError> {
        policy.validate()?;
        self.execution_policy = policy;
        Ok(self)
    }

    pub fn with_approval_provider(
        mut self,
        approval_provider: Arc<dyn ToolApprovalProvider>,
    ) -> Self {
        self.approval_provider = Some(approval_provider);
        self
    }

    pub fn with_input_provider(mut self, input_provider: Arc<dyn UserInputProvider>) -> Self {
        self.input_provider = Some(input_provider);
        self
    }

    pub fn with_memory_recall_limit(mut self, memory_recall_limit: usize) -> Self {
        self.memory_paths.recall_limit = memory_recall_limit;
        self
    }

    pub fn with_memory_paths(mut self, memory_paths: MemoryPaths) -> Self {
        self.memory_paths = memory_paths;
        self
    }

    pub fn with_model_compaction(mut self, enabled: bool, failure_threshold: u32) -> Self {
        self.model_compaction_enabled = enabled;
        self.compaction_failure_threshold = failure_threshold.max(1);
        self
    }

    /// Configure the Runtime-owned Agent source for this Engine.
    pub fn with_agent_activation(
        mut self,
        config: AgentActivationConfig,
    ) -> Result<Self, AgentActivationError> {
        let facts =
            resolved_agent_facts(self.model.as_ref(), &self.registry, config.context_tokens);
        self.agent_runtime = AgentRuntime::load(&self.workspace, config, facts)?;
        Ok(self)
    }

    pub fn model_id(&self) -> &str {
        self.model.model_id()
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn execution_environment(&self) -> &Arc<dyn ExecutionEnvironment> {
        &self.environment
    }

    pub fn runtime_identity(&self) -> RuntimeIdentity {
        let agent = self.agent_runtime.source_profile_identity();
        let tools = self.registry.descriptors();
        let mcp_servers = self
            .registry
            .extension::<McpRuntimeState>()
            .map(|state| state.snapshots())
            .unwrap_or_default();
        self.build_runtime_identity(EngineRuntimeIdentityInput {
            execution_policy: &self.execution_policy,
            capability_snapshot: &self.capability_snapshot,
            agent: &agent,
            tools: &tools,
            planner_prompt: self.planner.prompt(),
            evaluator_prompt: self.evaluator.prompt(),
            finalizer_prompt: self.finalizer.prompt(),
            mcp_servers: &mcp_servers,
        })
    }

    fn build_runtime_identity(&self, input: EngineRuntimeIdentityInput<'_>) -> RuntimeIdentity {
        let mut identity = build_runtime_identity(RuntimeIdentityInput {
            workspace: &self.workspace,
            model_id: self.model.model_id(),
            provider_target: self.model.client_id().as_str(),
            approval_policy: self.approval_policy,
            max_steps: self.config.max_steps,
            plan_enabled: self.config.plan_enabled,
            system_prompt: self.context_manager.system_prompt(),
            planner_prompt: input.planner_prompt,
            evaluator_prompt: input.evaluator_prompt,
            finalizer_prompt: input.finalizer_prompt,
            execution_policy: input.execution_policy.clone(),
            tools: input.tools,
            capability_snapshot_id: Some(&input.capability_snapshot.snapshot_id),
            execution_environment: Some(self.environment.identity()),
            execution_capabilities: Some(self.environment.capabilities()),
            agent: Some(input.agent),
        });
        identity.mcp_servers = input.mcp_servers.to_vec();
        identity
    }

    async fn run_post_run_hooks(&self, ctx: CompletedRunContext) {
        let ctx = PostRunHookContext {
            workspace: &self.workspace,
            memory_paths: &self.memory_paths,
            session_id: ctx.session_id,
            job_id: ctx.job_id,
            run_id: ctx.run_id,
            reason: ctx.reason,
            output: ctx.output,
            summary: ctx.summary,
            cancel_token: ctx.cancel_token,
        };
        self.hooks.run_post_run(&ctx).await;
    }

    /// Run the agent loop for a user message.
    ///
    /// Returns a stream of events. The stream completes when the run terminates.
    pub fn ask(&self, user_message: String, trace_writer: Option<TraceWriter>) -> RunStream<'_> {
        let req = RunRequest {
            session_id: crate::types::SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message,
            resume_state: None,
        };

        self.run(req, trace_writer)
    }

    /// Run the agent loop for an explicit request.
    ///
    /// The caller owns run identity so persisted artifacts and streamed events stay aligned.
    pub fn run(&self, req: RunRequest, trace_writer: Option<TraceWriter>) -> RunStream<'_> {
        self.run_with_cancel(req, trace_writer, CancellationToken::new())
    }

    /// Run the agent loop with an interface-owned cancellation token.
    pub fn run_with_cancel(
        &self,
        req: RunRequest,
        trace_writer: Option<TraceWriter>,
        cancel: CancellationToken,
    ) -> RunStream<'_> {
        let session_id = req.session_id;
        let job_id = req.job_id;
        let run_id = req.run_id;
        let user_message = req.user_message;
        let resume_state = req.resume_state;
        let stream_cancel = cancel.clone();
        // Dynamic catalogs publish only into the live registry. This private
        // copy freezes both schemas and implementations for the whole run.
        let run_registry = self.registry.snapshot();
        let run_mcp_servers = run_registry
            .extension::<McpRuntimeState>()
            .map(|state| state.snapshots())
            .unwrap_or_default();
        let mcp_lifecycle_facts = run_registry
            .extension::<McpRuntimeState>()
            .map(|state| state.take_facts())
            .unwrap_or_default();
        let pinned_agent_profile = resume_state.as_ref().and_then(|state| {
            // A successor run created from an interrupted snapshot must keep
            // the exact profile that was admitted before the interruption.
            // A completed product turn is a new user turn, so it may select a
            // fresh procedure set. Explicit same-run requests remain pinned
            // even when their lifecycle record already contains finalization.
            let same_run = state.run_id == run_id;
            let unfinished = state
                .execution_lifecycle
                .finalization
                .as_ref()
                .is_none_or(|finalization| finalization.completed_at.is_none());
            (same_run || unfinished)
                .then(|| {
                    state
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.agent_profile.as_ref())
                        .or(state.agent_profile.as_ref())
                })
                .flatten()
        });
        let run_capabilities = run_registry
            .descriptors()
            .into_iter()
            .filter_map(|descriptor| descriptor.capability_id)
            .collect();
        let resolved_agent = self.agent_runtime.resolve_for_run_with_capabilities(
            &user_message,
            pinned_agent_profile,
            run_capabilities,
        );
        let run_policy = resolved_agent
            .as_ref()
            .map(|agent| execution_policy_for_agent(&self.execution_policy, &agent.profile))
            .unwrap_or_else(|_| self.execution_policy.clone());
        let run_descriptors = resolved_agent
            .as_ref()
            .map(|agent| descriptors_for_agent(&run_registry, &agent.profile))
            .unwrap_or_else(|_| run_registry.descriptors());
        let run_capability_snapshot = CapabilitySnapshot::from_descriptors(&run_descriptors);
        let run_planner_prompt = resolved_agent
            .as_ref()
            .map(|agent| {
                composed_prompt(
                    self.planner.prompt(),
                    agent.profile.prompt_slot(PromptSlotRole::Planner),
                    "planner",
                )
            })
            .unwrap_or_else(|_| self.planner.prompt().to_string());
        let run_evaluator_prompt = resolved_agent
            .as_ref()
            .map(|agent| {
                composed_prompt(
                    self.evaluator.prompt(),
                    agent.profile.prompt_slot(PromptSlotRole::Evaluator),
                    "evaluator",
                )
            })
            .unwrap_or_else(|_| self.evaluator.prompt().to_string());
        let run_finalizer_prompt = resolved_agent
            .as_ref()
            .map(|agent| {
                composed_prompt(
                    self.finalizer.prompt(),
                    agent.profile.prompt_slot(PromptSlotRole::Finalizer),
                    "finalizer",
                )
            })
            .unwrap_or_else(|_| self.finalizer.prompt().to_string());
        let runtime_identity = resolved_agent
            .as_ref()
            .map(|agent| {
                self.build_runtime_identity(EngineRuntimeIdentityInput {
                    execution_policy: &run_policy,
                    capability_snapshot: &run_capability_snapshot,
                    agent: &agent.identity,
                    tools: &run_descriptors,
                    planner_prompt: &run_planner_prompt,
                    evaluator_prompt: &run_evaluator_prompt,
                    finalizer_prompt: &run_finalizer_prompt,
                    mcp_servers: &run_mcp_servers,
                })
            })
            .unwrap_or_else(|_| self.runtime_identity());
        let stream_agent_profile = resolved_agent
            .as_ref()
            .ok()
            .map(|agent| agent.profile.clone());
        let (control_handle, steer_rx) = control_channel();
        let steer_rx: SteerReceiver = Arc::new(AsyncMutex::new(steer_rx));
        let steer_lifecycle = SteerLifecycle::default();

        RunStream {
            session_id,
            job_id,
            run_id,
            cancel_token: cancel,
            control: control_handle,
            runtime_identity: runtime_identity.clone(),
            agent_profile: stream_agent_profile,
            inner: Box::pin(stream! {
                let mut run_summary = RunSummary::new(user_message.clone());

                macro_rules! complete_run {
                    ($reason:expr, $output:expr) => {{
                        let reason = $reason;
                        let output = $output;
                        // The last safe point may have passed while an LLM
                        // turn was resolving. Close any remaining steers
                        // before the terminal fact so every accepted API
                        // control has an explicit lifecycle outcome.
                        let mut pending_steers = steer_rx.lock().await;
                        // Reject a concurrent API submission before draining.
                        // Any sender that won the race is already buffered and
                        // is surfaced as a dropped lifecycle event below.
                        pending_steers.close();
                        while let Ok(steer) = pending_steers.try_recv() {
                            let dropped = StreamEvent::SteerDropped {
                                id: steer.id.0,
                                reason: "run completed before the steer reached a safe point"
                                    .to_string(),
                            };
                            run_summary.record_event(&dropped);
                            append_trace(&trace_writer, &dropped);
                            yield dropped;
                        }
                        drop(pending_steers);
                        for id in steer_lifecycle.take_unapplied().await {
                            let dropped = StreamEvent::SteerDropped {
                                id,
                                reason: "run completed before the accepted steer reached a model turn"
                                    .to_string(),
                            };
                            run_summary.record_event(&dropped);
                            append_trace(&trace_writer, &dropped);
                            yield dropped;
                        }
                        let event = StreamEvent::RunCompleted {
                            reason: reason.clone(),
                            output: output.clone(),
                        };
                        append_trace(&trace_writer, &event);
                        yield event;
                        self.run_post_run_hooks(CompletedRunContext {
                            session_id,
                            job_id,
                            run_id,
                            reason,
                            output,
                            summary: run_summary.clone(),
                            cancel_token: stream_cancel.clone(),
                        })
                        .await;
                        return;
                    }};
                }
                macro_rules! yield_traced {
                    ($event:expr) => {{
                        let event = $event;
                        append_trace(&trace_writer, &event);
                        yield event;
                    }};
                }

                let start_event = StreamEvent::RunStarted {
                    run_id,
                    job_id,
                    user_message: user_message.clone(),
                };
                append_trace(&trace_writer, &start_event);
                yield start_event;

                for fact in mcp_lifecycle_facts {
                    let event = match fact {
                        McpLifecycleFact::Degraded {
                            server_config_id,
                            failure_code,
                        } => {
                            let required = run_mcp_servers
                                .iter()
                                .find(|snapshot| snapshot.server_config_id == server_config_id)
                                .is_some_and(|snapshot| snapshot.required);
                            StreamEvent::McpServerDegraded {
                                server_config_id,
                                required,
                                failure_code,
                            }
                        }
                        McpLifecycleFact::CapabilitiesRefreshed {
                            server_config_id,
                            snapshot_id,
                            added,
                            removed,
                            changed,
                        } => StreamEvent::McpCapabilitiesRefreshed {
                            server_config_id,
                            snapshot_id,
                            added,
                            removed,
                            changed,
                        },
                    };
                    yield_traced!(event);
                }

                let resolved_agent = match resolved_agent {
                    Ok(agent) => agent,
                    Err(error) => {
                        let event = StreamEvent::ModelStatus {
                            status: "agent_activation_failed".to_string(),
                            message: format!("{}: {error}", error.code()),
                        };
                        yield_traced!(event);
                        complete_run!(
                            TerminationReason::Error,
                            Some("Agent activation failed before model execution".to_string())
                        );
                    }
                };

                yield_traced!(StreamEvent::AgentProfileActivated {
                    identity: Box::new(resolved_agent.identity.clone()),
                    resumed_from_snapshot: resolved_agent.resumed_from_snapshot,
                    diagnostics: resolved_agent.diagnostics.clone(),
                });
                if let Some(bundle) = resolved_agent.profile.instructions.as_ref() {
                    yield_traced!(StreamEvent::WorkspaceInstructionsResolved {
                        bundle_hash: bundle.bundle_hash(),
                        layer_count: bundle.all_layers().len(),
                        rejected_count: bundle.rejected.len(),
                        truncated: bundle.truncated,
                    });
                }
                yield_traced!(StreamEvent::ExecutionStrategySelected {
                    policy: run_policy.clone(),
                });
                if let Some(selection) = resolved_agent.profile.procedures.as_ref() {
                    yield_traced!(StreamEvent::ProceduresSelected {
                        profile_hash: resolved_agent.profile.profile_hash.clone(),
                        selected: selection
                            .selected
                            .iter()
                            .map(|selected| selected.reference.clone())
                            .collect(),
                        considered_count: selection.considered.len(),
                        excluded_count: selection
                            .risk_excluded
                            .len()
                            .saturating_add(selection.conflict_excluded.len()),
                    });
                }
                for procedure in &resolved_agent.profile.hydrated_procedures {
                    yield_traced!(StreamEvent::ProcedureHydrated {
                        reference: procedure.reference.clone(),
                        truncated: procedure.truncated,
                        dropped_bytes: procedure.dropped_bytes,
                    });
                }

                if stream_cancel.is_cancelled() {
                    complete_run!(TerminationReason::Cancelled, None);
                }

                warn_on_runtime_identity_mismatch(resume_state.as_ref(), &runtime_identity);

                let resume_checkpoint = resume_state
                    .as_ref()
                    .and_then(|state| state.checkpoint.as_ref());
                let history: Vec<Message> = if let Some(checkpoint) = resume_checkpoint {
                    if let Some(session) = checkpoint.session.as_ref() {
                        let mut session = session.clone();
                        let projection = session
                            .close_unresolved_tool_calls()
                            .and_then(|_| {
                                session
                                    .suffix(CHECKPOINT_SESSION_TAIL_ENTRIES)
                                    .messages_for_provider(&self.model.history_protocol())
                            });
                        match projection {
                            Ok(messages) => messages,
                            Err(error) => {
                                let message = StreamEvent::ModelStatus {
                                    status: "resume_rejected".to_string(),
                                    message: format!("canonical session cannot be projected safely: {error}"),
                                };
                                yield_traced!(message);
                                complete_run!(
                                    TerminationReason::Error,
                                    Some("resume rejected due to invalid canonical session history".to_string())
                                );
                            }
                        }
                    } else {
                        checkpoint.preserved_tail.clone()
                    }
                } else {
                    resume_state
                        .as_ref()
                        .map(|state| state.history.clone())
                        .unwrap_or_default()
                };
                let compact_summary = resume_checkpoint
                    .and_then(|checkpoint| checkpoint.summary.clone());
                let resume_summary = resume_state
                    .as_ref()
                    .and_then(|state| state.summary.as_deref());
                let prompt_memory = load_prompt_memory_from_paths_sync(
                    &self.memory_paths,
                    session_id,
                    resume_summary,
                    &user_message,
                )
                .unwrap_or_default();
                let mut working_memory: Vec<Message> = resolved_agent.prompt.messages.clone();
                if let Some(index) = prompt_memory.durable_index {
                    working_memory.push(durable_memory_message(&index));
                }
                if let Some(summary) = prompt_memory.session_summary {
                    working_memory.push(session_summary_message(&summary));
                }
                let step: u32 = resume_checkpoint
                    .map(|checkpoint| checkpoint.last_step)
                    .or_else(|| resume_state.as_ref().map(|state| state.step))
                    .unwrap_or(0);
                let plan = resume_checkpoint
                    .and_then(|checkpoint| checkpoint.plan.clone())
                    .or_else(|| resume_state.as_ref().and_then(|state| state.plan.clone()));

                // Execution budgets are per-run accounting. A genuine resume of
                // the same run must restore consumed usage so a restart cannot
                // hand out a fresh allowance, but a new turn that merely
                // continues a session starts from zero. Inheriting usage across
                // turns would progressively starve a long session until no work
                // could run at all.
                let resumes_same_run = resume_state
                    .as_ref()
                    .is_some_and(|state| state.run_id == run_id);
                let execution_lifecycle = if resumes_same_run {
                    resume_state
                        .as_ref()
                        .map(|state| state.execution_lifecycle.clone())
                        .unwrap_or_default()
                } else {
                    crate::execution::ExecutionLifecycleState::default()
                };
                // The React migration below reads `step` as a model-turn count.
                // It applies only to the run that actually consumed those turns.
                let budget_step = if resumes_same_run { step } else { 0 };

                let execution_policy = run_policy.clone();
                let run_planner = Planner::new(run_planner_prompt.clone());
                let run_evaluator = PlanEvaluator::new(run_evaluator_prompt.clone());
                let run_finalizer = Finalizer::new(run_finalizer_prompt.clone());
                let loop_context = LoopContext {
                    model: self.model.as_ref(),
                    registry: &run_registry,
                    capability_snapshot: &run_capability_snapshot,
                    context_manager: &self.context_manager,
                    workspace: &self.workspace,
                    environment: self.environment.clone(),
                    memory_paths: &self.memory_paths,
                    session_id,
                    max_steps: self.config.max_steps,
                    execution_policy: execution_policy.clone(),
                    finalizer: &run_finalizer,
                    agent_profile: Some(Arc::new(resolved_agent.profile.clone())),
                    agent_planner_summary: Some(resolved_agent.prompt.planner_summary.clone()),
                    instruction_overlays_seen: Arc::new(Mutex::new(BTreeSet::new())),
                    approval_policy: self.approval_policy,
                    approval_decision: self.approval_decision,
                    approval_provider: self.approval_provider.clone(),
                    input_provider: self.input_provider.clone(),
                    hooks: self.hooks.clone(),
                    compaction: CompactionRuntime::new(
                        self.model_compaction_enabled,
                        self.compaction_failure_threshold,
                    ),
                    steer_rx: Some(steer_rx.clone()),
                    steer_lifecycle: Some(steer_lifecycle.clone()),
                    // Artifacts live beside the trace and state snapshot for
                    // this run, so a resume reads the same ledger the original
                    // attempt wrote and download requests resolve against the
                    // run they name rather than an ambient directory.
                    tool_artifacts: Some(Arc::new(ToolArtifactStore::new(
                        self.workspace
                            .state_dir
                            .join("runs")
                            .join(run_id.to_string()),
                    ))),
                };

                let mut runtime: BoxStream<'_, LoopItem> = match execution_policy.strategy {
                    ExecutionStrategy::PlanReact => run_planned_loop(
                        loop_context,
                        &run_planner,
                        &run_evaluator,
                        &run_finalizer,
                        PlanLoopState {
                            user_message,
                            working_memory,
                            compact_summary,
                            history,
                            plan,
                            step_ledger: resume_state
                                .as_ref()
                                .map(|state| state.step_ledger.clone())
                                .unwrap_or_default(),
                            execution_lifecycle: execution_lifecycle.clone(),
                        },
                        stream_cancel.clone(),
                    ),
                    ExecutionStrategy::React => run_unplanned_loop(
                        loop_context,
                        RunLoopState {
                            user_message,
                            working_memory,
                            compact_summary,
                            history,
                            step: budget_step,
                            execution_lifecycle,
                        },
                        stream_cancel.clone(),
                    ),
                };

                while let Some(item) = runtime.next().await {
                    match item {
                        LoopItem::Event(event) => {
                            run_summary.record_event(&event);
                            yield_traced!(event);
                        }
                        LoopItem::Complete { reason, output } => {
                            complete_run!(reason, output);
                        }
                    }
                }

                complete_run!(
                    TerminationReason::Error,
                    Some("runtime loop ended without completion".to_string())
                );
            }),
        }
    }
}

struct EngineRuntimeIdentityInput<'a> {
    execution_policy: &'a ExecutionPolicy,
    capability_snapshot: &'a CapabilitySnapshot,
    agent: &'a crate::agents::AgentProfileIdentity,
    tools: &'a [ToolDescriptor],
    planner_prompt: &'a str,
    evaluator_prompt: &'a str,
    finalizer_prompt: &'a str,
    mcp_servers: &'a [McpServerRuntimeSnapshot],
}

struct CompletedRunContext {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    reason: TerminationReason,
    output: Option<String>,
    summary: RunSummary,
    cancel_token: CancellationToken,
}

fn resolved_agent_facts(
    model: &dyn ModelClient,
    registry: &ToolRegistry,
    context_tokens: Option<u32>,
) -> ResolvedRuntimeFacts {
    let available_capabilities = registry
        .descriptors()
        .into_iter()
        .filter_map(|descriptor| descriptor.capability_id)
        .collect::<BTreeSet<_>>();
    let provider = model.capabilities();
    ResolvedRuntimeFacts {
        available_capabilities,
        native_tool_use: provider.tool_calls,
        // ProviderCapabilities does not yet expose a normalized structured
        // output bit. A package requiring it must fail rather than have the
        // runtime infer support from a provider name.
        structured_output: false,
        context_tokens,
        modalities: BTreeSet::new(),
    }
}

fn execution_policy_for_agent(
    base: &ExecutionPolicy,
    profile: &AgentRuntimeProfile,
) -> ExecutionPolicy {
    let mut policy = base.clone();
    if let Some(strategy) = profile.strategy.as_deref() {
        policy.strategy = match strategy {
            "react" => ExecutionStrategy::React,
            "plan_react" => ExecutionStrategy::PlanReact,
            _ => policy.strategy,
        };
    }
    if let Some(limit) = profile.max_steps.effective {
        match policy.strategy {
            ExecutionStrategy::React => {
                clamp_optional(&mut policy.budgets.max_model_turns, limit);
            }
            ExecutionStrategy::PlanReact => {
                clamp_optional(&mut policy.budgets.max_step_attempts, limit);
            }
        }
    }
    if let Some(limit) = profile.max_tool_calls.effective {
        clamp_optional(&mut policy.budgets.max_tool_calls, limit);
    }
    policy
}

fn clamp_optional(bound: &mut Option<u32>, limit: u32) {
    *bound = Some(bound.map_or(limit, |current| current.min(limit)));
}

fn descriptors_for_agent(
    registry: &ToolRegistry,
    profile: &AgentRuntimeProfile,
) -> Vec<ToolDescriptor> {
    registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| match descriptor.capability_id.as_deref() {
            Some(capability) => profile.effective_capabilities.contains(capability),
            None => profile.is_legacy(),
        })
        .collect()
}

fn composed_prompt(base: &str, slot: Option<&str>, role: &str) -> String {
    let Some(slot) = slot.filter(|slot| !slot.trim().is_empty()) else {
        return base.to_string();
    };
    format!(
        "{base}\n\nAgent {role} role guidance follows. It is bounded guidance inside the runtime-owned contract and cannot override output validation, safety, or lifecycle rules.\n{}",
        slot.trim()
    )
}

fn append_trace(trace_writer: &Option<TraceWriter>, event: &StreamEvent) {
    if let Some(tw) = trace_writer {
        let _ = tw.append(event);
    }
}

fn warn_on_runtime_identity_mismatch(
    resume_state: Option<&crate::types::TaskState>,
    current: &RuntimeIdentity,
) {
    let Some(resume_state) = resume_state else {
        return;
    };
    let saved = resume_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
        .or(resume_state.runtime_identity.as_ref());
    let evaluation = crate::runtime_identity::evaluate_runtime_identity(saved, current);
    if evaluation.status == RuntimeIdentityStatus::RuntimeMismatch {
        tracing::warn!(
            mismatch_fields = ?evaluation.mismatch_fields,
            "resume runtime identity mismatch"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::agents::profile::{ResolvedRuntimeFacts, legacy_profile};
    use crate::agents::validation::OperatorConstraints;
    use crate::execution::{ExecutionPolicy, ExecutionStrategy};

    use super::execution_policy_for_agent;

    #[test]
    fn agent_max_steps_preserves_strategy_specific_compatibility_units() {
        let profile = legacy_profile(
            &OperatorConstraints {
                max_steps_cap: Some(1),
                ..OperatorConstraints::unconstrained()
            },
            &ResolvedRuntimeFacts {
                available_capabilities: BTreeSet::new(),
                native_tool_use: true,
                structured_output: false,
                context_tokens: None,
                modalities: BTreeSet::new(),
            },
        );

        let planned = execution_policy_for_agent(
            &ExecutionPolicy::from_max_steps_and_plan_flag(1, true),
            &profile,
        );
        assert_eq!(planned.strategy, ExecutionStrategy::PlanReact);
        assert_eq!(planned.budgets.max_step_attempts, Some(1));
        assert_eq!(planned.budgets.max_model_turns, None);

        let react = execution_policy_for_agent(
            &ExecutionPolicy::from_max_steps_and_plan_flag(1, false),
            &profile,
        );
        assert_eq!(react.strategy, ExecutionStrategy::React);
        assert_eq!(react.budgets.max_model_turns, Some(1));
        assert_eq!(react.budgets.max_step_attempts, None);
    }
}
