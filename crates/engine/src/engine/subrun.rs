//! Driver half of Fork-A subagent delegation.
//!
//! The request/outcome types, the [`SubRunSpawn`] trait, and the `task` tool
//! itself live in `rove-tools`; this module holds everything that must touch
//! the engine's turn primitives (`run_model_turn`, `run_tool_turn`,
//! `ToolTurnContext`, `append_tool_history`) to actually drive a child kernel.
//! `tool_turn.rs` attaches the spawner to each invocation's `ToolContext` as
//! `Arc<dyn SubRunSpawn>` so `DelegateTool` resolves it without naming this
//! type.

use std::sync::Arc;

use async_stream::stream;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use rove_core::{
    AgentKernelHost, KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem,
    KernelLimits, KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction,
    KernelToolTurnItem, ToolRegistry, run_agent_kernel,
};
use rove_models::{Message, ModelClient, ModelToolSchema};
use rove_tools::delegate::{
    DELEGATE_TOOL_NAME, SubRunApprovalParts, SubRunError, SubRunOutcome, SubRunRequest,
    SubRunSpawn, SubUsageLedger, SubagentRegistry,
};
use rove_tools::runtime_context::RuntimeToolServices;

use crate::engine::model_turn::{ModelTurnItem, run_model_turn};
use crate::engine::tool_turn::{
    ToolAction, ToolTurnContext, ToolTurnItem, ToolTurnOutcome, append_tool_history, run_tool_turn,
};
use rove_foundation::events::StreamEvent;

/// Hard cap on delegation depth. The child is built without a model or event
/// sink on its tool context, so it can never spawn in this phase; the guard is
/// a second line of defence for embeddings that attach a spawner deeper
/// anyway. Named because stage 3 (per-subagent stop / observability) needs the
/// same bound.
const MAX_DELEGATION_DEPTH: u8 = 2;

/// Default cap on a sub-run's model turns when the caller does not ask for one.
const DEFAULT_MAX_TURNS: u32 = 16;
/// Ceiling for `max_turns` so a caller cannot hand the child an unbounded turn
/// count.
const MAX_TURNS_LIMIT: u32 = 64;
/// Bounded tool-call and repair budgets so a child cannot spin inside the
/// parent's tool call.
const MAX_TOOL_CALLS_LIMIT: u32 = 256;
const MAX_REPAIRS_LIMIT: u32 = 4;

/// Removes a sub-run's registry entry when dropped, so `SubRunSpawner::run`
/// cannot leak a dead token through any of its early-return paths.
struct SubagentRegistration {
    registry: SubagentRegistry,
    parent_call_id: rove_core::CallId,
}

impl Drop for SubagentRegistration {
    fn drop(&mut self) {
        self.registry.remove(&self.parent_call_id);
    }
}

/// Runs one delegation request to completion inside the current tool call.
///
/// Owned by the engine through `ToolContext::with_extension` (as
/// `Arc<dyn SubRunSpawn>`). Holds everything a child kernel needs that the
/// tool itself does not know: the live model client, a frozen registry
/// snapshot (with `task` already removed), runtime services for child tool
/// contexts, the side-channel into the parent event stream, the parent call
/// identity, and the depth guard.
pub(crate) struct SubRunSpawner {
    model: Arc<dyn ModelClient>,
    /// Frozen when the tool turn started, so a catalog refresh during the
    /// child run cannot swap tools underneath it. It still contains `task` —
    /// `ToolRegistry` has no remove API — but the child's model schemas filter
    /// it out and its tool context is built without a model or event sink, so
    /// a dispatched `task` there fails through `DelegateTool`'s absent-spawner
    /// error instead of recursing.
    registry: ToolRegistry,
    services: RuntimeToolServices,
    /// Runtime-tool-context extras the child tool turn mirrors from the parent.
    approval: SubRunApprovalParts,
    /// Events pushed here are already `SubagentEvent`-wrapped; the tool turn
    /// drains them into the parent stream attributed to `parent_call_id`.
    events: mpsc::Sender<StreamEvent>,
    /// The parent `task` call this spawner serves — the attribution edge.
    parent_call_id: rove_core::CallId,
    /// `0` for a top-level run; each spawned child would get +1.
    depth: u8,
    /// Shared sink the child run's kernel counters land in; the parent run
    /// loop drains it when it charges its own budget.
    usage_ledger: Arc<std::sync::Mutex<SubUsageLedger>>,
    /// Registry this sub-run publishes its child token under `parent_call_id`
    /// into while it runs, so an embedding's per-subagent stop can find and
    /// cancel exactly this child without touching the parent run.
    subagents: SubagentRegistry,
}

impl SubRunSpawner {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        model: Arc<dyn ModelClient>,
        registry: ToolRegistry,
        services: RuntimeToolServices,
        approval: SubRunApprovalParts,
        events: mpsc::Sender<StreamEvent>,
        parent_call_id: rove_core::CallId,
        depth: u8,
        usage_ledger: Arc<std::sync::Mutex<SubUsageLedger>>,
        subagents: SubagentRegistry,
    ) -> Self {
        Self {
            model,
            registry,
            services,
            approval,
            events,
            parent_call_id,
            depth,
            usage_ledger,
            subagents,
        }
    }

    /// The token a stage-3 per-subagent stop can cancel for just this child.
    /// [`SubRunSpawner::run`] registers the token it actually drives under
    /// `parent_call_id`; this helper stays for callers that need a child token
    /// without running the kernel.
    fn child_token_for(&self, parent: &CancellationToken) -> CancellationToken {
        parent.child_token()
    }

    /// Drive one child kernel to completion.
    ///
    /// `parent_token` is the invoking tool context's cancel token; the child
    /// runs under a `child_token()` of it, so a parent cancel cascades but a
    /// future per-subagent stop can target just this child. Every
    /// `KernelItem::Event` is pushed into the side channel as
    /// `SubagentEvent { parent_call_id }`.
    async fn run(
        &self,
        request: SubRunRequest,
        parent_token: &CancellationToken,
    ) -> Result<SubRunOutcome, SubRunError> {
        if self.depth >= MAX_DELEGATION_DEPTH {
            return Err(SubRunError::DepthLimit);
        }
        let child_cancel = parent_token.child_token();
        // Publish the child token before the kernel starts so a per-subagent
        // stop issued while the run is being set up still lands. The guard
        // removes it on every exit from `run` — early error returns included —
        // so the registry never advertises a token nobody is polling.
        self.subagents
            .register(self.parent_call_id, child_cancel.clone());
        let _registration = SubagentRegistration {
            registry: self.subagents.clone(),
            parent_call_id: self.parent_call_id,
        };

        let max_turns = request
            .max_turns
            .unwrap_or(DEFAULT_MAX_TURNS)
            .min(MAX_TURNS_LIMIT);
        let limits = KernelLimits {
            max_model_turns: Some(max_turns),
            max_tool_calls: Some(MAX_TOOL_CALLS_LIMIT),
            max_repairs: Some(MAX_REPAIRS_LIMIT),
        };

        // `task` is filtered out of the child's model schemas here, and its
        // tool context is built without a model or side-channel, so a nested
        // `task` call cannot be offered to the model and would fail through
        // `DelegateTool`'s absent-spawner error even if emitted.
        let host = SubagentKernelHost {
            model: Arc::clone(&self.model),
            tool_schemas: self
                .registry
                .model_schemas()
                .into_iter()
                .filter(|schema| schema.name != DELEGATE_TOOL_NAME)
                .collect(),
            registry: self.registry.clone(),
            services: self.services.clone(),
            approval: self.approval.clone(),
            depth: self.depth,
            subagents: self.subagents.clone(),
        };
        let state = KernelState::new(vec![
            Message::system(subagent_system_prompt(&self.services)),
            Message::user(request.prompt.clone()),
        ]);

        let mut kernel = run_agent_kernel(host, state, limits, child_cancel);
        let parent_call_id = self.parent_call_id;
        let events = self.events.clone();
        let mut finished = None;
        while let Some(item) = kernel.next().await {
            match item {
                KernelItem::Event(event) => {
                    // Best-effort: if the parent stopped listening (its tool
                    // call returned, or cancel raced ahead) keep driving the
                    // child to its own terminal rather than deadlock on a full
                    // channel.
                    let _ = events
                        .send(StreamEvent::SubagentEvent {
                            parent_call_id,
                            event: Box::new(event),
                        })
                        .await;
                }
                KernelItem::Finished(result) => {
                    finished = Some(result);
                    break;
                }
            }
        }

        let Some(result) = finished else {
            return Err(SubRunError::Incomplete);
        };
        // Charge the child's kernel counters to the parent budget's shared
        // ledger regardless of how the run ended — turns it spent are real.
        self.usage_ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(
                result.state.model_turns,
                result.state.tool_calls,
                result.state.repairs,
                result.state.usage.clone(),
            );
        let outcome = SubRunOutcome {
            output: match &result.termination {
                KernelTermination::Final { output } => Some(output.clone()),
                KernelTermination::Extension { output, .. } => output.clone(),
                _ => None,
            },
            usage: result.state.usage,
            model_turns: result.state.model_turns,
            tool_calls: result.state.tool_calls,
            repairs: result.state.repairs,
        };
        match result.termination {
            KernelTermination::Final { .. } | KernelTermination::Extension { .. } => Ok(outcome),
            KernelTermination::Cancelled => Err(SubRunError::Cancelled),
            KernelTermination::ModelFailed(error) => {
                Err(SubRunError::ModelFailed(error.to_string()))
            }
            KernelTermination::ModelTurnLimit => Err(SubRunError::TurnLimit),
            KernelTermination::ToolCallLimit => Err(SubRunError::ToolCallLimit),
            KernelTermination::RepairLimit => Err(SubRunError::RepairLimit),
            KernelTermination::IncompleteBeforeModelTurn
            | KernelTermination::IncompleteModelTurn
            | KernelTermination::IncompleteToolTurn => Err(SubRunError::Incomplete),
        }
    }
}

impl SubRunSpawn for SubRunSpawner {
    fn run<'a>(
        &'a self,
        request: SubRunRequest,
        parent_token: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<SubRunOutcome, SubRunError>> {
        Box::pin(SubRunSpawner::run(self, request, parent_token))
    }

    fn child_token_for(&self, parent: &CancellationToken) -> CancellationToken {
        SubRunSpawner::child_token_for(self, parent)
    }
}

fn subagent_system_prompt(services: &RuntimeToolServices) -> String {
    format!(
        "You are a delegated subagent inside one rove run. Complete the task \
        given as the user message and return a concise final answer. \
        Workspace root: {}. You cannot delegate further.",
        services.workspace.root.display()
    )
}

/// Minimal `AgentKernelHost` for a delegated run.
///
/// Far simpler than `UnplannedKernelHost`: no steers, no compaction, no
/// instruction overlays, no agent profile, no silent-turn recovery. It reuses
/// the parent's `Executor` through a borrowed `ToolTurnContext` built per tool
/// turn, so approval, boundary, and artifact rules match a top-level run.
struct SubagentKernelHost {
    model: Arc<dyn ModelClient>,
    tool_schemas: Vec<ModelToolSchema>,
    /// Owned snapshot the borrowed `ToolTurnContext` points into; the context
    /// is rebuilt per `tool_turn` call so the stream lifetime stays inside
    /// this host's borrow.
    registry: ToolRegistry,
    services: RuntimeToolServices,
    approval: SubRunApprovalParts,
    /// Depth this child itself is at; its own tool context inherits it so a
    /// nested `task` spawner would be created at the right depth if one ever
    /// existed (it does not, because `model`/`events` are `None` there).
    depth: u8,
    /// The run's shared subagent registry, forwarded onto the child's own
    /// tool-turn context so the `ToolTurnContext` shape stays complete even
    /// though a child can never reach the spawn path.
    subagents: SubagentRegistry,
}

impl SubagentKernelHost {
    fn tool_turn_context(&self, cancel_token: CancellationToken) -> ToolTurnContext<'_> {
        ToolTurnContext {
            registry: &self.registry,
            workspace: &self.services.workspace,
            environment: Arc::clone(&self.services.environment),
            memory_paths: &self.services.memory_paths,
            approval_policy: self.services.approval_policy,
            approval_decision: self.approval.approval_decision,
            approval_provider: self.approval.approval_provider.clone(),
            input_provider: self.services.input_provider.clone(),
            hooks: self.approval.hooks.clone(),
            cancel_token,
            tool_artifacts: self.services.tool_artifacts.clone(),
            agent_profile: None,
            run_mode: self.approval.run_mode,
            // No model on the child's own tool context: even if the child
            // model somehow emits a `task` call (e.g. via the compatibility
            // text path), `build_subrun_spawner` returns `None` and
            // `DelegateTool` errors cleanly instead of recursing.
            model: None,
            subagent_depth: self.depth,
            subagent_usage_ledger: Arc::new(std::sync::Mutex::new(SubUsageLedger::default())),
            subagents: self.subagents.clone(),
        }
    }
}

impl AgentKernelHost for SubagentKernelHost {
    type Event = StreamEvent;
    type Stop = SubRunError;
    type ToolOutcome = ToolTurnOutcome;
    type Output = ();

    fn before_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        _cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelBeforeModelTurnItem<Self::Event, Self::Stop>> {
        // The child history already carries its system guidance plus every
        // prior turn; there is no prompt assembly, so `Ready` is just the
        // current history verbatim.
        Box::pin(stream! {
            yield KernelBeforeModelTurnItem::Ready(state.history.clone());
        })
    }

    fn model_turn<'a>(
        &'a mut self,
        messages: Vec<Message>,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelModelTurnItem<Self::Event>> {
        let model = Arc::clone(&self.model);
        let schemas = self.tool_schemas.clone();
        Box::pin(stream! {
            let mut inner = run_model_turn(
                model.as_ref(),
                messages,
                schemas,
                cancel_token,
                rove_foundation::types::RunMode::Normal,
            );
            while let Some(item) = inner.next().await {
                match item {
                    ModelTurnItem::Event(event) => {
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
                        yield KernelModelTurnItem::Failed(error);
                        return;
                    }
                }
            }
        })
    }

    fn after_model_turn<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _turn: &'a rove_core::model_turn::ModelTurn,
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
            // Built inside the stream so the borrowed `ToolTurnContext` lives
            // exactly as long as the tool-turn future borrowing it. The kernel
            // passes the child cancel token straight through, so the context
            // sees the same token the turn was invoked with.
            let ctx = self.tool_turn_context(cancel_token);
            let mut inner = run_tool_turn(ctx, action);
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use rove_core::{CallId, Tool, ToolContext, ToolRegistry};
    use rove_models::{FakeModelClient, FakeTurn};
    use rove_tools::delegate::{
        DelegateTool, SubRunApprovalParts, SubRunError, SubRunRequest, SubRunSpawn, SubUsageLedger,
        SubagentRegistry,
    };
    use rove_tools::echo::EchoTool;
    use rove_tools::runtime_context::RuntimeToolServices;

    use super::{MAX_DELEGATION_DEPTH, SubRunSpawner};
    use rove_foundation::environment::local_environment;
    use rove_foundation::events::StreamEvent;
    use rove_foundation::root::Workspace;
    use rove_foundation::types::{ApprovalDecision, ApprovalPolicy, RunMode};
    use rove_state::memory::paths::MemoryPaths;
    use rove_tools::hooks::HookRegistry;

    fn fixture_services(workspace: &Workspace) -> RuntimeToolServices {
        RuntimeToolServices {
            workspace: workspace.clone(),
            environment: local_environment(workspace),
            memory_paths: MemoryPaths::from_workspace(workspace, 8),
            approval_policy: ApprovalPolicy::Auto,
            input_provider: None,
            tool_artifacts: None,
            run_mode: RunMode::Normal,
        }
    }

    fn fixture_spawner(
        model: Arc<dyn rove_models::ModelClient>,
        registry: ToolRegistry,
        workspace: &Workspace,
        events: tokio::sync::mpsc::Sender<StreamEvent>,
        parent_call_id: CallId,
        depth: u8,
    ) -> (SubRunSpawner, Arc<Mutex<SubUsageLedger>>, SubagentRegistry) {
        let ledger = Arc::new(Mutex::new(SubUsageLedger::default()));
        let subagents = SubagentRegistry::default();
        let spawner = SubRunSpawner::new(
            model,
            registry,
            fixture_services(workspace),
            SubRunApprovalParts {
                approval_decision: ApprovalDecision::Approve,
                approval_provider: None,
                hooks: HookRegistry::default(),
                run_mode: RunMode::Normal,
            },
            events,
            parent_call_id,
            depth,
            Arc::clone(&ledger),
            subagents.clone(),
        );
        (spawner, ledger, subagents)
    }

    #[tokio::test]
    async fn depth_guard_refuses_nested_delegation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let (events, _rx) = tokio::sync::mpsc::channel(8);
        let model = Arc::new(FakeModelClient::new("unused".to_string()));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let (spawner, _ledger, _subagents) = fixture_spawner(
            model,
            registry,
            &workspace,
            events,
            CallId::new(),
            MAX_DELEGATION_DEPTH,
        );

        let result = SubRunSpawn::run(
            &spawner,
            SubRunRequest {
                prompt: "go deeper".to_string(),
                agent: None,
                max_turns: None,
            },
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(SubRunError::DepthLimit)));
    }

    /// The core Fork-A contract: a scripted child turn runs a real tool call,
    /// every child event reaches the parent channel already wrapped in
    /// `SubagentEvent` with the parent's call id, the child's counters land in
    /// the usage ledger, and the final text is the tool result.
    #[tokio::test]
    async fn sub_run_streams_wrapped_events_and_returns_final_text() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let parent_call_id = CallId::new();
        let (events, mut rx) = tokio::sync::mpsc::channel(64);

        // Turn 1: call `echo`; turn 2: final answer.
        let model = Arc::new(FakeModelClient::with_turns(
            "fallback".to_string(),
            vec![
                FakeTurn::ToolUse {
                    id: "call-1".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::json!({"message": "child-tool-output"}),
                },
                FakeTurn::Text("the child answer".to_string()),
            ],
        ));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let (spawner, ledger, _subagents) =
            fixture_spawner(model, registry, &workspace, events, parent_call_id, 0);

        let outcome = SubRunSpawn::run(
            &spawner,
            SubRunRequest {
                prompt: "say hi via echo".to_string(),
                agent: None,
                max_turns: Some(8),
            },
            &CancellationToken::new(),
        )
        .await
        .expect("the sub-run completes");

        assert_eq!(outcome.output.as_deref(), Some("the child answer"));
        assert_eq!(outcome.model_turns, 2);
        assert_eq!(outcome.tool_calls, 1);

        // Drain every wrapped event the child pushed while it ran.
        let mut wrapped = Vec::new();
        while let Ok(event) = rx.try_recv() {
            wrapped.push(event);
        }
        assert!(!wrapped.is_empty());
        for event in &wrapped {
            match event {
                StreamEvent::SubagentEvent {
                    parent_call_id: id,
                    event,
                } => {
                    assert_eq!(*id, parent_call_id);
                    // A nested SubagentEvent must never occur.
                    assert!(!matches!(**event, StreamEvent::SubagentEvent { .. }));
                }
                other => panic!("expected wrapped SubagentEvent, got {other:?}"),
            }
        }
        // The child's echo lifecycle appears, attributed to the parent call.
        let inner_names: Vec<String> = wrapped
            .iter()
            .filter_map(|event| match event {
                StreamEvent::SubagentEvent { event, .. } => Some(event.event_name().to_string()),
                _ => None,
            })
            .collect();
        assert!(inner_names.iter().any(|name| name == "llm_message"));
        assert!(inner_names.iter().any(|name| name == "tool_call_started"));
        assert!(inner_names.iter().any(|name| name == "tool_call_completed"));

        // The parent budget's ledger charged the child's kernel counters once.
        let charged = ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain();
        assert_eq!(charged.len(), 1);
        assert_eq!(charged[0].0, 2);
        assert_eq!(charged[0].1, 1);
        assert!(
            ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain()
                .is_empty()
        );
    }

    /// A `task` call routed through the real `Tool` path end-to-end: the
    /// extension on `ToolContext` is what makes delegation available.
    #[tokio::test]
    async fn delegate_tool_runs_a_sub_run_through_the_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let (events, _rx) = tokio::sync::mpsc::channel(32);
        let model = Arc::new(FakeModelClient::new("final from fake".to_string()));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let (spawner, _ledger, _subagents) =
            fixture_spawner(model, registry, &workspace, events, CallId::new(), 0);

        let spawner: Arc<dyn SubRunSpawn> = Arc::new(spawner);
        let ctx = ToolContext::new(CallId::new(), CancellationToken::new())
            .with_extension(Arc::new(spawner));
        let output = DelegateTool::new()
            .execute(serde_json::json!({"prompt": "answer"}), &ctx)
            .await
            .unwrap();
        assert_eq!(output.content, "final from fake");
    }

    #[test]
    fn subagent_registry_register_cancel_remove_round_trip() {
        let registry = SubagentRegistry::default();
        let call_id = CallId::new();
        assert!(registry.active_ids().is_empty());
        assert!(!registry.cancel(&call_id), "unknown id is a miss");

        let token = CancellationToken::new();
        registry.register(call_id, token.clone());
        assert_eq!(registry.active_ids(), vec![call_id]);

        assert!(registry.cancel(&call_id));
        assert!(token.is_cancelled());
        // A second cancel on the dead token still reports a miss: the entry
        // stays until `run` removes it, but it names nothing live.
        assert!(!registry.cancel(&call_id));

        registry.remove(&call_id);
        assert!(registry.active_ids().is_empty());
    }

    /// The stage-3 contract: a live sub-run is visible in the registry under
    /// its parent call id, and cancelling through the registry terminates the
    /// child kernel — which the tool surfaces as `SubRunError::Cancelled` —
    /// while the parent token stays untouched.
    #[tokio::test]
    async fn sub_run_registers_then_cancels_through_the_registry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let parent_call_id = CallId::new();
        let (events, _rx) = tokio::sync::mpsc::channel(64);
        // The child model turn holds forever; only a cancel ends it.
        let model = Arc::new(FakeModelClient::with_turns(
            "unused".to_string(),
            vec![FakeTurn::hold("held-open child turn")],
        ));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let (spawner, _ledger, subagents) =
            fixture_spawner(model, registry, &workspace, events, parent_call_id, 0);

        let parent_token = CancellationToken::new();
        let run = tokio::spawn({
            let parent_token = parent_token.clone();
            async move {
                SubRunSpawn::run(
                    &spawner,
                    SubRunRequest {
                        prompt: "hold".to_string(),
                        agent: None,
                        max_turns: Some(4),
                    },
                    &parent_token,
                )
                .await
            }
        });

        // Wait until the sub-run has published its child token.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !subagents.active_ids().contains(&parent_call_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the sub-run registers its child token while it runs");

        assert!(subagents.cancel(&parent_call_id));
        let result = run.await.expect("the sub-run task completes");
        assert!(matches!(result, Err(SubRunError::Cancelled)));
        assert!(
            !parent_token.is_cancelled(),
            "a child-scoped stop must not reach the parent token"
        );
        assert!(
            subagents.active_ids().is_empty(),
            "the ended sub-run removes its registry entry"
        );
    }

    /// The completion path also removes the entry: a finished sub-run must not
    /// leave a dead token behind for a later stop request to hit.
    #[tokio::test]
    async fn sub_run_removes_its_registration_on_completion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let parent_call_id = CallId::new();
        let (events, _rx) = tokio::sync::mpsc::channel(64);
        let model = Arc::new(FakeModelClient::new("done".to_string()));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let (spawner, _ledger, subagents) =
            fixture_spawner(model, registry, &workspace, events, parent_call_id, 0);

        SubRunSpawn::run(
            &spawner,
            SubRunRequest {
                prompt: "finish".to_string(),
                agent: None,
                max_turns: Some(4),
            },
            &CancellationToken::new(),
        )
        .await
        .expect("the sub-run completes");

        assert!(subagents.active_ids().is_empty());
        assert!(!subagents.cancel(&parent_call_id));
    }
}
