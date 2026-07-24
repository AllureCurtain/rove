use thiserror::Error;

use rove_models::ModelError;

/// Errors from the model/tool Agent harness.
#[derive(Debug, Clone, Error)]
pub enum AgentError {
    #[error("Model error: {0}")]
    Model(#[from] ModelError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("Agent loop ended without a terminal model response")]
    Incomplete,
}

/// Errors from the runtime-neutral tool execution boundary.
#[derive(Debug, Clone, Error, serde::Serialize, serde::Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ToolError {
    #[error("Unknown tool: {name}")]
    UnknownTool { name: String },

    #[error("Invalid arguments: {reason}")]
    InvalidArgs { reason: String },

    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },

    #[error("Hook blocked tool call: {reason}")]
    HookBlocked { reason: String },

    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("Tool execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Tool timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
}

impl ToolError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::UnknownTool { .. } => "unknown_tool",
            Self::InvalidArgs { .. } => "invalid_args",
            Self::InvalidInput { .. } => "invalid_input",
            Self::HookBlocked { .. } => "hook_blocked",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::ExecutionFailed { .. } => "execution_failed",
            Self::Timeout { .. } => "timeout",
        }
    }
}
