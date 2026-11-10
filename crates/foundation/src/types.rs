use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
pub use rove_core::{
    Action, CallId, ToolCallAction, ToolCapability, ToolContext, ToolDescriptor, ToolError,
    ToolExecutionMetadata, ToolExecutionStatus, ToolMutation, ToolMutationOperation, ToolResult,
    ToolRiskLevel,
};
pub use rove_models::{Message, ModelToolSchema, Role, ToolCallRef, Usage};
use serde::{Deserialize, Serialize};

/// Identifiers and lifecycle vocabulary that cross the wire.
///
/// These are defined in `rove-protocol` — the workspace leaf crate, which
/// depends on nothing but serde and ulid — and re-exported here so that every
/// `rove_runtime::types::SessionId` import keeps resolving unchanged.
pub use rove_protocol::{
    ApprovalDecision, ApprovalPolicy, JobId, RunId, RunMode, RunStatus, SessionId,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PromptCompactionState {
    pub mode: PromptCompactionMode,
    pub auto_triggered: bool,
    pub degraded: bool,
    pub consecutive_failures: u32,
    pub circuit_open: bool,
    /// When the automatic path may next probe a tripped breaker, as an RFC 3339
    /// timestamp.
    ///
    /// The breaker is a session fact, so its cooldown has to be one too: the
    /// count is cleared only by a successful summary, and a count that survives
    /// a restart without a deadline would either retry a known-broken model on
    /// every turn or — once seeded — never compact again. `None` means no window
    /// is armed, which an older snapshot also decodes to: a missing deadline is
    /// read as expired, so the worst case of a legacy or corrupt value is one
    /// bounded probe, never a session that can no longer compact itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    #[serde(default)]
    pub source_message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Default for PromptCompactionState {
    fn default() -> Self {
        Self {
            mode: PromptCompactionMode::Deterministic,
            auto_triggered: false,
            degraded: false,
            consecutive_failures: 0,
            circuit_open: false,
            next_attempt_after: None,
            model: None,
            prompt_version: None,
            source_message_count: 0,
            last_error: None,
        }
    }
}

impl PromptCompactionState {
    /// Copy the session-scoped breaker facts from `persisted` onto this state.
    ///
    /// The breaker is a session fact: the failure count is cleared only by a
    /// successful summary, and the cooldown it armed is what keeps that count
    /// from becoming a retry on every turn. A run that continues its session
    /// therefore has to carry all of it — the count, the reported breaker state,
    /// the deadline, and the two facts that explain them (`degraded` and
    /// `last_error`) — so a tripped breaker never ships without its cause.
    ///
    /// `mode`, `auto_triggered`, `model`, `prompt_version`, and
    /// `source_message_count` are deliberately *not* copied: they describe the
    /// compaction one run performed, and a run that compacted nothing must not
    /// claim another run's mode.
    pub fn inherit_session_breaker(&mut self, persisted: &PromptCompactionState) {
        self.consecutive_failures = persisted.consecutive_failures;
        self.circuit_open = persisted.circuit_open;
        self.next_attempt_after = persisted.next_attempt_after.clone();
        self.degraded = persisted.degraded;
        self.last_error = persisted.last_error.clone();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptCompactionMode {
    None,
    Deterministic,
    ModelGenerated,
    Automatic,
    Degraded,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct TaskPlan {
    pub goal: String,
    pub steps: Vec<PlanStep>,
    pub current_step: usize,
}

impl TaskPlan {
    pub fn current_step(&self) -> Option<&PlanStep> {
        self.steps.get(self.current_step)
    }

    pub fn mark_current_done(&mut self) {
        if let Some(step) = self.steps.get_mut(self.current_step) {
            step.done = true;
        }
        self.current_step = self
            .steps
            .iter()
            .position(|step| !step.done)
            .unwrap_or(self.steps.len());
    }

    pub fn is_complete(&self) -> bool {
        self.current_step >= self.steps.len() || self.steps.iter().all(|step| step.done)
    }
}

/// Why a run terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    /// LLM produced a final answer.
    Final,
    /// Hit the maximum step count.
    StepLimit,
    /// Hit the token budget.
    TokenLimit,
    /// Hit the time budget.
    TimeLimit,
    /// Unrecoverable error.
    Error,
    /// Cancelled by user or system.
    Cancelled,
}

/// Approval request sent from core to an interface before a destructive tool runs.
#[derive(Debug, Clone)]
pub struct ToolApprovalRequest {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

/// A registered approval request whose interface decision can be awaited
/// after Core publishes the canonical approval event.
pub struct PendingToolApproval {
    response: Pin<Box<dyn Future<Output = ApprovalDecision> + Send + 'static>>,
}

impl PendingToolApproval {
    pub fn new<F>(response: F) -> Self
    where
        F: Future<Output = ApprovalDecision> + Send + 'static,
    {
        Self {
            response: Box::pin(response),
        }
    }

    pub async fn resolve(self) -> ApprovalDecision {
        self.response.await
    }
}

impl std::fmt::Debug for PendingToolApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingToolApproval")
            .finish_non_exhaustive()
    }
}

/// Interface-provided approval channel.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ToolApprovalProvider: Send + Sync {
    /// Legacy one-phase approval API for direct callers.
    async fn decide(&self, request: ToolApprovalRequest) -> ApprovalDecision {
        match self.begin_approval(request).await {
            Ok(pending) => pending.resolve().await,
            Err(_) => ApprovalDecision::Reject,
        }
    }

    /// Registers an approval request without waiting for its decision.
    ///
    /// Existing providers that only implement [`Self::decide`] remain
    /// source-compatible, but must migrate to this method before they can be
    /// used by the Engine's canonical approval lifecycle. The default rejects
    /// requests fail-closed.
    async fn begin_approval(
        &self,
        _request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        Err(ToolError::ExecutionFailed {
            reason:
                "approval provider must implement begin_approval for registered approval events"
                    .to_string(),
        })
    }
}

/// User input request sent from a tool to an interface.
#[derive(Debug, Clone)]
pub struct UserInputRequest {
    pub prompt: String,
}

/// A registered input request whose interface response can be awaited later.
pub struct PendingUserInput {
    response: Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + 'static>>,
}

impl PendingUserInput {
    pub fn new<F>(response: F) -> Self
    where
        F: Future<Output = Result<String, ToolError>> + Send + 'static,
    {
        Self {
            response: Box::pin(response),
        }
    }

    pub async fn resolve(self) -> Result<String, ToolError> {
        self.response.await
    }
}

impl std::fmt::Debug for PendingUserInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingUserInput").finish_non_exhaustive()
    }
}

/// Interface-provided channel for tools that need mid-task user input.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait UserInputProvider: Send + Sync {
    /// Legacy one-phase input API.
    ///
    /// New providers should implement [`Self::begin_input`] so Core can
    /// publish the canonical waiting event only after the request is
    /// answerable. This default keeps direct callers working with two-phase
    /// providers.
    async fn request_input(&self, request: UserInputRequest) -> Result<String, ToolError> {
        crate::user_input::request_input(self, request.prompt).await
    }

    /// Registers an answerable request without waiting for the answer itself.
    ///
    /// Legacy providers that only implement [`Self::request_input`] remain
    /// source-compatible, but must migrate to this method before they can be
    /// used by the Engine's canonical `InputNeeded` lifecycle.
    async fn begin_input(
        &self,
        _input_id: CallId,
        _request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        Err(ToolError::ExecutionFailed {
            reason: "input provider must implement begin_input for registered input events"
                .to_string(),
        })
    }
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}
