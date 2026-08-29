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
use ulid::Ulid;

use crate::session::{Session, SessionError};

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
    /// Exact Agent snapshot used by this run. It is retained in state only;
    /// events and reports project the content-free identity instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<crate::agents::AgentRuntimeProfile>,
    #[serde(
        default,
        skip_serializing_if = "crate::execution::StepLedgerState::is_empty"
    )]
    pub step_ledger: crate::execution::StepLedgerState,
    #[serde(
        default,
        skip_serializing_if = "crate::execution::ExecutionLifecycleState::is_empty"
    )]
    pub execution_lifecycle: crate::execution::ExecutionLifecycleState,
}

impl TaskState {
    /// The model-visible history a resumed run would start from.
    ///
    /// Three sources can hold it and they are not equivalent, so the order
    /// matters: the canonical session is authoritative for anything written
    /// since it was introduced, `preserved_tail` is the compatibility
    /// projection older writers left behind, and the flat `history` is the
    /// pre-checkpoint format. Whoever needs this history has to agree with the
    /// resume path about the precedence or the two will drift, which is why it
    /// lives on the state rather than at each call site.
    ///
    /// Fails only when a stored canonical session cannot be projected for the
    /// given protocol; callers decide whether that is fatal.
    pub fn replayable_history(&self, protocol: &str) -> Result<Vec<Message>, SessionError> {
        let Some(checkpoint) = self.checkpoint.as_ref() else {
            return Ok(self.history.clone());
        };
        let Some(session) = checkpoint.session.as_ref() else {
            return Ok(checkpoint.preserved_tail.clone());
        };
        let mut session = session.clone();
        session.close_unresolved_tool_calls()?;
        session
            .suffix(crate::session::CHECKPOINT_SESSION_TAIL_ENTRIES)
            .messages_for_provider(protocol)
    }

    /// Whether an empty replayable history is the deliberate result of a
    /// compaction rather than a snapshot that never got written.
    ///
    /// The distinction matters because the two are indistinguishable by size
    /// alone and want opposite handling: a crashed run's empty snapshot should
    /// be refilled from its trace, while a compacted one must stay empty or the
    /// summary ends up sitting next to the very history it replaced.
    ///
    /// This is the exact shape [`Self::continue_from_summary`] leaves behind: a
    /// checkpoint that carries a summary and holds no history in either the
    /// canonical session or the compatibility tail. A run that died before
    /// writing a checkpoint has no checkpoint at all, so it cannot match.
    pub fn history_was_compacted_away(&self) -> bool {
        self.checkpoint.as_ref().is_some_and(|checkpoint| {
            checkpoint.summary.is_some()
                && checkpoint.session.is_none()
                && checkpoint.preserved_tail.is_empty()
        })
    }

    /// Replace the accumulated history with `summary`, so the next run starts
    /// from the summary instead of the messages it stands for.
    ///
    /// All three history sources are cleared. Clearing only one would leave the
    /// summary coexisting with the messages it replaces — the next prompt would
    /// get *larger*, which is the opposite of what a compaction is for.
    ///
    /// The summary is written to `checkpoint.summary` even when there was no
    /// checkpoint before, because that field — not `TaskState::summary` — is the
    /// one a resumed run reads as its compaction summary. `TaskState::summary`
    /// is a different slot that every completed run fills with a truncated
    /// final output, so it cannot be used to carry a compaction forward without
    /// making ordinary resumes look like compacted ones.
    pub fn continue_from_summary(&mut self, summary: String) {
        self.history.clear();
        let checkpoint = self
            .checkpoint
            .get_or_insert_with(|| PromptCheckpoint::carrying_summary(self.step));
        checkpoint.preserved_tail.clear();
        checkpoint.session = None;
        checkpoint.summary = Some(summary.clone());
        checkpoint.compacted_history_messages = 0;
        checkpoint.token_estimate = summary.chars().count().div_ceil(4);
        self.summary = Some(summary);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageDeliveryRecord {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Resumable prompt checkpoint used to rebuild context without replaying the full audit history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptCheckpoint {
    pub summary: Option<String>,
    pub preserved_tail: Vec<Message>,
    /// Canonical session is the durable conversation source for new writers.
    /// `preserved_tail` remains a derived compatibility projection for older
    /// readers and is dual-read when this field is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<crate::agents::AgentRuntimeProfile>,
    #[serde(default)]
    pub step_ledger: crate::execution::StepLedgerCheckpoint,
    #[serde(default)]
    pub execution_lifecycle: crate::execution::ExecutionLifecycleCheckpoint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_deliveries: Vec<MessageDeliveryRecord>,
}

impl PromptCheckpoint {
    /// A checkpoint whose only job is to carry a compaction summary forward.
    ///
    /// Used when a session is compacted before it ever produced a checkpoint of
    /// its own: there is no preserved tail or canonical session to keep, and the
    /// summary is filled in by the caller. Pointers are left unset rather than
    /// guessed, since nothing here knows the memory layout the run used.
    pub(crate) fn carrying_summary(last_step: u32) -> Self {
        Self {
            summary: None,
            preserved_tail: Vec::new(),
            session: None,
            plan: None,
            session_memory_pointer: None,
            durable_memory_pointer: None,
            last_step,
            last_event_seq: None,
            token_estimate: 0,
            compacted_history_messages: 0,
            compaction: PromptCompactionState::default(),
            runtime_identity: None,
            agent_profile: None,
            step_ledger: crate::execution::StepLedgerCheckpoint::default(),
            execution_lifecycle: crate::execution::ExecutionLifecycleCheckpoint::default(),
            message_deliveries: Vec::new(),
        }
    }
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

macro_rules! impl_runtime_id_from_str {
    ($id:ident) => {
        impl std::str::FromStr for $id {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(value)
                    .map(Self)
                    .map_err(|error| error.to_string())
            }
        }
    };
}

impl_runtime_id_from_str!(SessionId);
impl_runtime_id_from_str!(JobId);
impl_runtime_id_from_str!(RunId);

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

/// Execution profile selected by the host before a run starts.
///
/// Review is deliberately a runtime-owned mode rather than a prompt hint. It
/// is carried into every tool invocation and checked again at dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Normal,
    Review,
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

#[cfg(test)]
mod compaction_state_tests {
    use super::*;

    fn state_with_history(history: Vec<Message>) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "goal".to_string(),
            step: 3,
            history,
            summary: None,
            checkpoint: None,
            plan: None,
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        }
    }

    /// A session compacted before it ever wrote a checkpoint still has to carry
    /// its summary forward. The resume path reads `checkpoint.summary`, so
    /// leaving `checkpoint` as `None` would drop the summary on the floor and
    /// the compaction would amount to deleting the history.
    #[test]
    fn compacting_a_checkpointless_session_still_carries_the_summary() {
        let mut state = state_with_history(vec![Message::user("dropped")]);
        assert!(state.checkpoint.is_none());

        state.continue_from_summary("THE SUMMARY".to_string());

        let checkpoint = state
            .checkpoint
            .as_ref()
            .expect("compaction must leave a checkpoint to carry the summary");
        assert_eq!(checkpoint.summary.as_deref(), Some("THE SUMMARY"));
        assert_eq!(checkpoint.last_step, 3, "the step must survive compaction");
        assert!(state.replayable_history("openai").unwrap().is_empty());
    }

    /// The discriminator that keeps Phase 6's trace fallback from refilling a
    /// deliberately emptied history. A compacted state must be recognisable;
    /// an empty one that was never compacted must not be.
    #[test]
    fn only_a_compacted_state_reports_its_history_as_compacted_away() {
        let mut compacted = state_with_history(vec![Message::user("dropped")]);
        compacted.continue_from_summary("THE SUMMARY".to_string());
        assert!(compacted.history_was_compacted_away());

        // A run killed before writing a checkpoint: empty for a different
        // reason, and its history is still recoverable from the trace.
        let crashed = state_with_history(Vec::new());
        assert!(
            !crashed.history_was_compacted_away(),
            "a checkpointless empty snapshot is a crash, not a compaction"
        );
    }
}
