use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use rove_core::{CallId, ToolError};
use rove_models::Message;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Unique identifier for a session (user-level, spans multiple jobs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Ulid);

/// Unique identifier for a job (one task submission).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub Ulid);

/// Unique identifier for a single engine run (one main-loop execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub Ulid);

/// Request to run a single engine loop.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub user_message: String,
    pub resume_state: Option<TaskState>,
}

/// Persisted task snapshot used for resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub schema_version: u32,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub goal: String,
    pub step: u32,
    pub history: Vec<Message>,
    pub summary: Option<String>,
    #[serde(default)]
    pub checkpoint: Option<PromptCheckpoint>,
    #[serde(default)]
    pub plan: Option<TaskPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_identity: Option<crate::runtime_identity::RuntimeIdentity>,
    #[serde(
        default,
        skip_serializing_if = "crate::execution::StepLedgerState::is_empty"
    )]
    pub step_ledger: crate::execution::StepLedgerState,
}

/// Resumable prompt checkpoint used to rebuild context without replaying the full audit history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptCheckpoint {
    pub summary: Option<String>,
    pub preserved_tail: Vec<Message>,
    pub plan: Option<TaskPlan>,
    pub session_memory_pointer: Option<String>,
    pub durable_memory_pointer: Option<String>,
    pub last_step: u32,
    pub last_event_seq: Option<u64>,
    pub token_estimate: usize,
    pub compacted_history_messages: usize,
    #[serde(default)]
    pub compaction: PromptCompactionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_identity: Option<crate::runtime_identity::RuntimeIdentity>,
    #[serde(default)]
    pub step_ledger: crate::execution::StepLedgerCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptCompactionState {
    pub mode: PromptCompactionMode,
    pub auto_triggered: bool,
    pub degraded: bool,
    pub consecutive_failures: u32,
    pub circuit_open: bool,
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
            model: None,
            prompt_version: None,
            source_message_count: 0,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCompactionMode {
    None,
    Deterministic,
    ModelGenerated,
    Automatic,
    Degraded,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl SessionId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl JobId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl RunId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why a run terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Current status of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Init,
    Running,
    Done,
    Error,
    Cancelled,
    Interrupted,
}

/// Tool approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Ask,
    Auto,
    Never,
}

/// A concrete approval decision supplied by an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
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
#[async_trait]
pub trait UserInputProvider: Send + Sync {
    /// Legacy one-phase input API.
    ///
    /// New providers should implement [`Self::begin_input`] so Core can
    /// publish the canonical waiting event only after the request is
    /// answerable. This default keeps direct callers working with two-phase
    /// providers.
    async fn request_input(&self, request: UserInputRequest) -> Result<String, ToolError> {
        crate::tool_input::request_input(self, request.prompt).await
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
