use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_stream::stream;
use futures::stream::{BoxStream, Stream, StreamExt};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::compaction::CompactionRuntime;
use crate::context::{ContextManager, durable_memory_message, session_summary_message};
use crate::engine::control::{RunControlHandle, SteerLifecycle, control_channel};
use crate::environment::{ExecutionEnvironment, local_environment};
use crate::events::StreamEvent;
use crate::execution::{DEFAULT_MAX_MODEL_TURNS_PER_STEP, ExecutionPolicy, ExecutionStrategy};
use crate::hooks::{HookRegistry, PostRunHookContext, RunSummary};
use crate::memory::layered::load_prompt_memory_from_paths_sync;
use crate::memory::paths::MemoryPaths;
use crate::plan_loop::{PlanLoopState, run_planned_loop};
use crate::planner::Planner;
use crate::run_loop::{LoopContext, LoopItem, RunLoopState, SteerReceiver, run_unplanned_loop};
use crate::runtime_identity::{
    RuntimeIdentity, RuntimeIdentityInput, RuntimeIdentityStatus, build_runtime_identity,
};
use crate::state::trace::TraceWriter;
use crate::types::{
    ApprovalDecision, ApprovalPolicy, JobId, Message, RunId, RunRequest, SessionId,
    TerminationReason, ToolApprovalProvider, UserInputProvider,
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
    inner: Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'e>>,
}

impl<'e> RunStream<'e> {
    fn new(
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        cancel_token: CancellationToken,
        control: RunControlHandle,
        inner: impl Stream<Item = StreamEvent> + Send + 'e,
    ) -> Self {
        Self {
            session_id,
            job_id,
            run_id,
            cancel_token,
            control,
            inner: Box::pin(inner),
        }
    }

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
    /// Project sugar fields into the typed policy used by the engine.
    ///
    /// `max_steps` / `plan_enabled` remain convenience inputs; `ExecutionPolicy`
    /// is the sole execution-config truth.
    pub fn to_execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::from_max_steps_and_plan_flag(self.max_steps, self.plan_enabled)
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_steps: 20,
            plan_enabled: false,
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
    context_manager: ContextManager,
    config: EngineConfig,
    planner: Planner,
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
        Self {
            model,
            registry,
            context_manager,
            config,
            planner: Planner::default(),
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
        let tools = self.registry.descriptors();
        build_runtime_identity(RuntimeIdentityInput {
            workspace: &self.workspace,
            model_id: self.model.model_id(),
            provider_target: self.model.client_id().as_str(),
            approval_policy: self.approval_policy,
            max_steps: self.config.max_steps,
            plan_enabled: self.config.plan_enabled,
            system_prompt: self.context_manager.system_prompt(),
            planner_prompt: self.planner.prompt(),
            tools: &tools,
        })
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
        let runtime_identity = self.runtime_identity();
        let (control_handle, steer_rx) = control_channel();
        let steer_rx: SteerReceiver = Arc::new(AsyncMutex::new(steer_rx));
        let steer_lifecycle = SteerLifecycle::default();

        RunStream::new(
            session_id,
            job_id,
            run_id,
            cancel,
            control_handle,
            stream! {
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

                if stream_cancel.is_cancelled() {
                    complete_run!(TerminationReason::Cancelled, None);
                }

                warn_on_runtime_identity_mismatch(resume_state.as_ref(), &runtime_identity);

                let resume_checkpoint = resume_state
                    .as_ref()
                    .and_then(|state| state.checkpoint.as_ref());
                let history: Vec<Message> = resume_checkpoint
                    .map(|checkpoint| checkpoint.preserved_tail.clone())
                    .or_else(|| resume_state.as_ref().map(|state| state.history.clone()))
                    .unwrap_or_default();
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
                let mut working_memory: Vec<Message> = Vec::new();
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

                let execution_policy = self.config.to_execution_policy();
                let max_model_turns_per_step = execution_policy
                    .budgets
                    .max_model_turns_per_step
                    .unwrap_or(DEFAULT_MAX_MODEL_TURNS_PER_STEP);
                let loop_context = LoopContext {
                    model: self.model.as_ref(),
                    registry: &self.registry,
                    context_manager: &self.context_manager,
                    workspace: &self.workspace,
                    environment: self.environment.clone(),
                    memory_paths: &self.memory_paths,
                    session_id,
                    max_steps: self.config.max_steps,
                    max_model_turns_per_step,
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
                };

                let mut runtime: BoxStream<'_, LoopItem> = match execution_policy.strategy {
                    ExecutionStrategy::PlanReact => run_planned_loop(
                        loop_context,
                        &self.planner,
                        PlanLoopState {
                            user_message,
                            working_memory,
                            compact_summary,
                            history,
                            step,
                            plan,
                            step_ledger: resume_state
                                .as_ref()
                                .map(|state| state.step_ledger.clone())
                                .unwrap_or_default(),
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
                            step,
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
            },
        )
    }
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
