//! Fork-A subagent delegation: a `task` tool that runs a nested
//! [`run_agent_kernel`] synchronously inside the parent run.
//!
//! The parent model sees a single `task` tool result. The child kernel's own
//! [`StreamEvent`]s travel to the parent event stream already wrapped in
//! [`StreamEvent::SubagentEvent`], so every consumer sees one flat stream and
//! model-history projection drops the wrapped events automatically.
//!
//! The spawn surface is a `ToolContext` extension the engine attaches at run
//! time (it needs the live model client and the event sink, which do not exist
//! when the tool registry is built). The engine's driver half lives in
//! `rove-runtime::engine::subrun`; this module holds the request/outcome data,
//! the `SubRunSpawn` trait the tool downcasts to, and the `task` tool itself.
//! An embedding that never attaches a spawner gets a clean
//! `delegation is not available in this embedding` error rather than a panic.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use rove_core::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};
use rove_models::Usage;

use crate::hooks::HookRegistry;

/// Model-facing name of the delegation tool. Exported so the engine can filter
/// it out of a child's own registry snapshot (a delegated run may not delegate).
pub const DELEGATE_TOOL_NAME: &str = "task";

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
    pub approval_decision: rove_foundation::types::ApprovalDecision,
    pub approval_provider: Option<Arc<dyn rove_foundation::types::ToolApprovalProvider>>,
    pub hooks: HookRegistry,
    pub run_mode: rove_foundation::types::RunMode,
}

/// The spawn surface `DelegateTool` reads off its `ToolContext`.
///
/// The engine's `SubRunSpawner` (in `rove-runtime`) implements this trait and
/// is attached as `Arc<dyn SubRunSpawn>` so this crate never names the
/// concrete driver — the trait object keeps the dependency arrow pointing
/// `rove-runtime -> rove-tools` instead of the reverse.
pub trait SubRunSpawn: Send + Sync {
    /// Drive one child kernel to completion under a child token of
    /// `parent_token`.
    fn run<'a>(
        &'a self,
        request: SubRunRequest,
        parent_token: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<SubRunOutcome, SubRunError>>;

    /// The token a per-subagent stop can cancel for just this child.
    fn child_token_for(&self, parent: &CancellationToken) -> CancellationToken;
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

        let Some(spawner) = ctx.extension::<Arc<dyn SubRunSpawn>>() else {
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
    use tokio_util::sync::CancellationToken;

    use rove_core::{CallId, Tool, ToolContext};

    use super::DelegateTool;

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
}
