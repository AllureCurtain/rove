use thiserror::Error;

pub use rove_models::ModelError;

/// Top-level application errors (used at binary/interface boundary).
#[derive(Debug, Error)]
pub enum RoveError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Model error: {0}")]
    Model(#[from] ModelError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("State error: {0}")]
    State(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Errors from tool execution pipeline.
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
            ToolError::UnknownTool { .. } => "unknown_tool",
            ToolError::InvalidArgs { .. } => "invalid_args",
            ToolError::InvalidInput { .. } => "invalid_input",
            ToolError::HookBlocked { .. } => "hook_blocked",
            ToolError::PermissionDenied { .. } => "permission_denied",
            ToolError::ExecutionFailed { .. } => "execution_failed",
            ToolError::Timeout { .. } => "timeout",
        }
    }
}
