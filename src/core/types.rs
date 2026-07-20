use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::memory::paths::MemoryPaths;

/// Unique identifier for a session (user-level, spans multiple jobs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Ulid);

/// Unique identifier for a job (one task submission).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub Ulid);

/// Unique identifier for a single engine run (one main-loop execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub Ulid);

/// Unique identifier for a tool call within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallId(pub Ulid);

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
    pub runtime_identity: Option<crate::core::runtime_identity::RuntimeIdentity>,
    #[serde(
        default,
        skip_serializing_if = "crate::core::execution::StepLedgerState::is_empty"
    )]
    pub step_ledger: crate::core::execution::StepLedgerState,
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
    pub runtime_identity: Option<crate::core::runtime_identity::RuntimeIdentity>,
    #[serde(default)]
    pub step_ledger: crate::core::execution::StepLedgerCheckpoint,
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

impl CallId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for CallId {
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

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls issued by an assistant message. Empty for non-assistant messages
    /// or assistant messages that did not invoke tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRef>,
    /// Identifier from the model's tool-use block that this tool message responds to.
    /// `None` for non-tool messages or for tool results from text-parsed actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A tool call reference recorded on an assistant message so providers can replay
/// the full tool-use exchange on the next request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRef {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRef>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn tool(content: impl Into<String>, tool_call_id: Option<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id,
        }
    }
}

/// Message role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// The action parsed from LLM output.
#[derive(Debug, Clone)]
pub enum Action {
    /// LLM produced a final answer — stop the loop.
    Final { text: String },

    /// LLM wants to call a tool.
    ToolCall {
        call_id: CallId,
        tool_use_id: Option<String>,
        name: String,
        args: serde_json::Value,
    },

    /// LLM wants to call multiple tools in one batch.
    ToolBatch { calls: Vec<ToolCallAction> },

    /// LLM output could not be parsed into a valid action.
    Malformed { reason: String },
}

#[derive(Debug, Clone)]
pub struct ToolCallAction {
    pub call_id: CallId,
    /// The model-assigned tool-use ID (e.g. "call_abc123" for OpenAI, "toolu_xyz" for Anthropic).
    /// Used to correlate tool results back to the model on the next turn.
    pub tool_use_id: Option<String>,
    pub name: String,
    pub args: serde_json::Value,
}

/// Result of a successful tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: CallId,
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<ToolMutation>,
    #[serde(default)]
    pub metadata: ToolExecutionMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMutation {
    pub path: String,
    pub operation: ToolMutationOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolMutationOperation {
    Create,
    Update,
    Delete,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    #[default]
    Ok,
    Error,
    Rejected,
    PartialSuccess,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    #[default]
    Low,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionMetadata {
    pub status: ToolExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_event_type: Option<String>,
    pub risk_level: ToolRiskLevel,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_paths: Vec<String>,
    pub workspace_changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff_summary: Vec<String>,
}

/// Token usage from a single LLM call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
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

/// Tool schema definition exposed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub destructive: bool,
    pub parallel_safe: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<ToolCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCapability {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
        crate::core::tool_input::request_input(self, request.prompt).await
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

/// Context passed through the tool execution boundary.
#[derive(Clone)]
pub struct ToolContext<'a> {
    pub workspace: &'a Workspace,
    pub memory_paths: MemoryPaths,
    pub approval_policy: ApprovalPolicy,
    pub cancel_token: CancellationToken,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
}

impl std::fmt::Debug for ToolContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("workspace", &self.workspace)
            .field("memory_paths", &self.memory_paths)
            .field("approval_policy", &self.approval_policy)
            .field("cancel_token", &self.cancel_token)
            .field("input_provider", &self.input_provider.is_some())
            .finish()
    }
}
