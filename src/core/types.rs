use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::core::workspace::Workspace;

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
    pub plan: Option<TaskPlan>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
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
        name: String,
        args: serde_json::Value,
    },

    /// LLM output could not be parsed into a valid action.
    Malformed { reason: String },
}

/// Result of a successful tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: CallId,
    pub output: String,
}

/// Token usage from a single LLM call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
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
}

/// Tool schema definition exposed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub destructive: bool,
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

/// Context passed through the tool execution boundary.
#[derive(Debug, Clone, Copy)]
pub struct ToolContext<'a> {
    pub workspace: &'a Workspace,
    pub approval_policy: ApprovalPolicy,
}
