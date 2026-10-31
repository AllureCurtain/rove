//! Fork-A subagent delegation: a `task` tool that runs a nested
//! [`run_agent_kernel`] synchronously inside the parent run.
//!
//! The parent model sees a single `task` tool result. The child kernel's own
//! [`StreamEvent`]s travel to the parent event stream already wrapped in
//! [`StreamEvent::SubagentEvent`], so every consumer sees one flat stream and
//! model-history projection drops the wrapped events automatically.
//!
//! The `SubRunSpawner` is attached to the invocation's [`ToolContext`] by the
//! engine at run time (it needs the live model client and the event sink, which
//! do not exist when the tool registry is built). An embedding that never
//! attaches a spawner gets a clean
//! `delegation is not available in this embedding` error rather than a panic.

use std::collections::HashMap;
use std::sync::Arc;

use async_stream::stream;
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use rove_core::{
    AgentKernelHost, KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem,
    KernelLimits, KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction,
    KernelToolTurnItem, Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry,
    run_agent_kernel,
};
use rove_models::{Message, ModelClient, ModelToolSchema, Usage};

use crate::engine::model_turn::{ModelTurnItem, run_model_turn};
use crate::engine::tool_turn::{
    ToolAction, ToolTurnContext, ToolTurnItem, ToolTurnOutcome, append_tool_history, run_tool_turn,
};
use crate::events::StreamEvent;
use crate::hooks::HookRegistry;
use crate::tools::runtime_context::RuntimeToolServices;

/// Model-facing name of the delegation tool. Exported so the engine can filter
/// it out of a child's own registry snapshot (a delegated run may not delegate).
pub const DELEGATE_TOOL_NAME: &str = "task";

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

/// What the parent model is allowed to ask for.
#[derive(Debug, Clone)]
pub struct SubRunRequest {
    /// The child's task, presented as its single user message.
    pub prompt: String,
    /// Optional agent label; validated but not selected against — there is
    /// exactly one child profile in this phase. Kept so the wire schema does
    /// not change when stage 3 adds routing.
    #[allow(dead_code)]
    pub agent: Option<String>,
    /// Optional cap on the child's model turns. Clamped to `MAX_TURNS_LIMIT`.
    pub max_turns: Option<u32>,
}

/// What the parent engine needs back: the child's final answer for the tool
/// result, plus the counters the parent budget must absorb.
#[derive(Debug)]
pub struct SubRunOutcome {
    /// The child's final text, or `None` when it never produced one.
    pub output: Option<String>,
    /// Token usage accumulated across all child model turns.
    pub usage: Usage,
    /// Kernel counters the parent budget should also charge.
    pub model_turns: u32,
    pub tool_calls: u32,
    pub repairs: u32,
}

/// Live child cancellation handles keyed by the parent `task` call that
/// spawned them.
///
/// The map is the control surface for per-subagent stop: a `SubRunSpawner`
/// registers its child token under its `parent_call_id` while the sub-run is
/// driving its kernel, and removes the entry when the run ends, so a lookup
/// that misses means the subagent is already gone. The `Engine` holds the
/// registry for a run; embeddings that need child-scoped cancel (the API's
/// `/jobs/{job_id}/subagents/{call_id}/stop`) inject a shared clone so the
/// handle the route cancels is the same token the kernel is polling.
///
/// `std::sync::Mutex` is deliberate: critical sections are map inserts and
/// removes only — no `.await` is ever held across one.
#[derive(Debug, Default, Clone)]
pub struct SubagentRegistry {
    tokens: Arc<std::sync::Mutex<HashMap<rove_core::CallId, CancellationToken>>>,
}

impl SubagentRegistry {
    /// Record the child token a sub-run is about to drive. A second
    /// registration under the same `parent_call_id` replaces the first;
    /// `parent_call_id` is the engine's per-call identity, so a collision
    /// would already mean two tool calls share one id.
    pub fn register(&self, parent_call_id: rove_core::CallId, token: CancellationToken) {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(parent_call_id, token);
    }

    /// Drop the entry when its sub-run ends. An absent id is fine — the entry
    /// may never have been registered (e.g. a depth-refused run).
    pub fn remove(&self, parent_call_id: &rove_core::CallId) {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(parent_call_id);
    }

    /// Cancel the live subagent under `parent_call_id`. `false` means the id
    /// is unknown or its sub-run already ended — the caller maps that to a
    /// not-found rather than treating it as a second stop of a dead child.
    pub fn cancel(&self, parent_call_id: &rove_core::CallId) -> bool {
        let token = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(parent_call_id)
            .cloned();
        match token {
            Some(token) if !token.is_cancelled() => {
                token.cancel();
                true
            }
            _ => false,
        }
    }

    /// Parent call ids with a live registration, for introspection and tests.
    pub fn active_ids(&self) -> Vec<rove_core::CallId> {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
    }
}

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

/// Usage a delegated sub-run has spent, pushed to a shared ledger the parent
/// run loop drains when it charges its own `ExecutionBudgetTracker`.
///
/// Kept as a plain tuple list rather than a running total so each `task` call
/// contributes exactly one entry and nothing is double-charged on a retry.
#[derive(Debug, Default)]
pub struct SubUsageLedger {
    entries: Vec<(u32, u32, u32, Usage)>,
}

impl SubUsageLedger {
    pub fn record(&mut self, model_turns: u32, tool_calls: u32, repairs: u32, usage: Usage) {
        self.entries.push((model_turns, tool_calls, repairs, usage));
    }

    /// Take every recorded sub-run's counters. Draining means each entry is
    /// charged exactly once even if a caller polls more than once.
    pub fn drain(&mut self) -> Vec<(u32, u32, u32, Usage)> {
        std::mem::take(&mut self.entries)
    }
}

/// Errors the tool maps to `ToolError` without a panic.
#[derive(Debug, thiserror::Error)]
pub enum SubRunError {
    #[error("delegation depth limit reached")]
    DepthLimit,
    #[error("subagent run was cancelled")]
    Cancelled,
    #[error("subagent model call failed: {0}")]
    ModelFailed(String),
    #[error("subagent reached its model-turn limit")]
    TurnLimit,
    #[error("subagent reached its tool-call limit")]
    ToolCallLimit,
    #[error("subagent reached its repair limit")]
    RepairLimit,
    #[error("subagent ended without a final answer")]
    Incomplete,
}

/// Approval/hook/provider pieces a child `ToolTurnContext` needs that are not
/// part of `RuntimeToolServices`. Public so the engine can fill them from its
/// own `ToolTurnContext` when it builds the spawner.
#[derive(Clone)]
pub struct SubRunApprovalParts {
    pub approval_decision: crate::types::ApprovalDecision,
    pub approval_provider: Option<Arc<dyn crate::types::ToolApprovalProvider>>,
    pub hooks: HookRegistry,
    pub run_mode: crate::types::RunMode,
}

/// Runs one delegation request to completion inside the current tool call.
///
/// Owned by the engine through `ToolContext::with_extension`. Holds everything
/// a child kernel needs that the tool itself does not know: the live model
/// client, a frozen registry snapshot (with `task` already removed), runtime
/// services for child tool contexts, the side-channel into the parent event
/// stream, the parent call identity, and the depth guard.
pub struct SubRunSpawner {
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
    pub fn new(
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
    pub fn child_token_for(&self, parent: &CancellationToken) -> CancellationToken {
        parent.child_token()
    }

    /// Drive one child kernel to completion.
    ///
    /// `parent_token` is the invoking tool context's cancel token; the child
    /// runs under a `child_token()` of it, so a parent cancel cascades but a
    /// future per-subagent stop can target just this child. Every
    /// `KernelItem::Event` is pushed into the side channel as
    /// `SubagentEvent { parent_call_id }`.
    pub async fn run(
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
                crate::types::RunMode::Normal,
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

/// The model-facing `task` tool. Stateless: everything it needs arrives on the
/// `ToolContext` extensions the engine attached for this call.
pub struct DelegateTool;

impl DelegateTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DelegateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: DELEGATE_TOOL_NAME.to_string(),
            description: "Delegate a bounded subtask to a nested agent that runs \
                with the same tools and workspace, then returns its final \
                answer. Use for self-contained work that benefits from an \
                isolated context."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The task the subagent should complete"
                    },
                    "agent": {
                        "type": "string",
                        "description": "Optional agent label for the sub-run"
                    },
                    "max_turns": {
                        "type": "integer",
                        "description": "Optional cap on the subagent's model turns"
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            destructive: false,
            // It spends the parent budget and can run other tools, so it must
            // never run inside a parallel batch.
            parallel_safe: false,
            capability_id: Some("agent.delegate.task".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: prompt".to_string(),
            })?
            .to_string();
        let agent = args
            .get("agent")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let max_turns = args
            .get("max_turns")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok());

        let Some(spawner) = ctx.extension::<SubRunSpawner>() else {
            return Err(ToolError::ExecutionFailed {
                reason: "delegation is not available in this embedding".to_string(),
            });
        };

        match spawner
            .run(
                SubRunRequest {
                    prompt,
                    agent,
                    max_turns,
                },
                &ctx.cancel_token,
            )
            .await
        {
            Ok(outcome) => Ok(tool_output_for_subrun(outcome)),
            Err(SubRunError::DepthLimit) => Err(ToolError::PermissionDenied {
                reason: "delegation depth limit reached".to_string(),
            }),
            Err(SubRunError::Cancelled) => Err(ToolError::ExecutionFailed {
                reason: "subagent run was cancelled".to_string(),
            }),
            Err(error) => Err(ToolError::ExecutionFailed {
                reason: error.to_string(),
            }),
        }
    }
}

/// Pack the child's final answer into the tool result.
///
/// The budget counters are charged separately through `SubUsageLedger`, so the
/// envelope carries only the single text truth — nothing the model never needs
/// to see is duplicated into a structured side-channel.
fn tool_output_for_subrun(outcome: SubRunOutcome) -> ToolOutput {
    let text = outcome
        .output
        .unwrap_or_else(|| "subagent finished without a final answer".to_string());
    ToolOutput::from_envelope(rove_core::ToolOutputEnvelope::text(text))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use rove_core::{CallId, Tool, ToolContext, ToolRegistry};
    use rove_models::{FakeModelClient, FakeTurn};

    use super::{
        DelegateTool, MAX_DELEGATION_DEPTH, SubRunApprovalParts, SubRunRequest, SubRunSpawner,
        SubUsageLedger, SubagentRegistry,
    };
    use crate::environment::local_environment;
    use crate::events::StreamEvent;
    use crate::hooks::HookRegistry;
    use crate::memory::paths::MemoryPaths;
    use crate::tools::echo::EchoTool;
    use crate::tools::runtime_context::RuntimeToolServices;
    use crate::types::{ApprovalDecision, ApprovalPolicy, RunMode};
    use crate::workspace::Workspace;

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

    #[test]
    fn task_schema_requires_prompt() {
        let descriptor = DelegateTool::new().schema();
        assert_eq!(descriptor.name, "task");
        assert!(!descriptor.destructive);
        assert!(!descriptor.parallel_safe);
        assert_eq!(
            descriptor.parameters["required"],
            serde_json::json!(["prompt"])
        );
        assert_eq!(descriptor.parameters["additionalProperties"], false);
    }

    #[tokio::test]
    async fn delegate_tool_fails_cleanly_without_a_spawner() {
        let ctx = ToolContext::new(CallId::new(), CancellationToken::new());
        let error = DelegateTool::new()
            .execute(serde_json::json!({"prompt": "do a thing"}), &ctx)
            .await
            .unwrap_err();
        match error {
            rove_core::ToolError::ExecutionFailed { reason } => {
                assert!(reason.contains("delegation is not available"));
            }
            other => panic!("expected ExecutionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delegate_tool_validates_prompt_presence() {
        let ctx = ToolContext::new(CallId::new(), CancellationToken::new());
        let error = DelegateTool::new()
            .execute(serde_json::json!({"prompt": "   "}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(error, rove_core::ToolError::InvalidArgs { .. }));
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

        let result = spawner
            .run(
                SubRunRequest {
                    prompt: "go deeper".to_string(),
                    agent: None,
                    max_turns: None,
                },
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(super::SubRunError::DepthLimit)));
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

        let outcome = spawner
            .run(
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
                spawner
                    .run(
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
        assert!(matches!(result, Err(super::SubRunError::Cancelled)));
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

        spawner
            .run(
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
