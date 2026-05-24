use thiserror::Error;

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

/// Errors from LLM model interactions.
#[derive(Debug, Clone, Error)]
pub enum ModelError {
    #[error("API request failed: {0}")]
    RequestFailed(String),

    #[error("Stream interrupted: {0}")]
    StreamInterrupted(String),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Context length exceeded: used {used} / max {max}")]
    ContextLengthExceeded { used: u32, max: u32 },
}

impl ModelError {
    pub fn error_code(&self) -> &'static str {
        match self {
            ModelError::RequestFailed(_) => "request_failed",
            ModelError::StreamInterrupted(_) => "stream_interrupted",
            ModelError::RateLimited { .. } => "rate_limited",
            ModelError::AuthFailed => "auth_failed",
            ModelError::ContextLengthExceeded { .. } => "context_length_exceeded",
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ModelError::RequestFailed(_)
                | ModelError::StreamInterrupted(_)
                | ModelError::RateLimited { .. }
        )
    }

    pub fn counts_as_health_failure(&self) -> bool {
        matches!(
            self,
            ModelError::RequestFailed(_)
                | ModelError::StreamInterrupted(_)
                | ModelError::RateLimited { .. }
        )
    }
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

#[cfg(test)]
mod tests {
    use super::ModelError;

    #[test]
    fn model_error_codes_are_stable() {
        assert_eq!(
            ModelError::RequestFailed("network".to_string()).error_code(),
            "request_failed"
        );
        assert_eq!(
            ModelError::StreamInterrupted("closed".to_string()).error_code(),
            "stream_interrupted"
        );
        assert_eq!(
            ModelError::RateLimited {
                retry_after_ms: 500
            }
            .error_code(),
            "rate_limited"
        );
        assert_eq!(ModelError::AuthFailed.error_code(), "auth_failed");
        assert_eq!(
            ModelError::ContextLengthExceeded { used: 10, max: 5 }.error_code(),
            "context_length_exceeded"
        );
    }

    #[test]
    fn model_error_classification_separates_retry_and_health() {
        assert!(ModelError::RequestFailed("network".to_string()).is_retryable());
        assert!(ModelError::StreamInterrupted("closed".to_string()).is_retryable());
        assert!(
            ModelError::RateLimited {
                retry_after_ms: 500
            }
            .is_retryable()
        );
        assert!(!ModelError::AuthFailed.is_retryable());
        assert!(!ModelError::ContextLengthExceeded { used: 10, max: 5 }.is_retryable());

        assert!(ModelError::RequestFailed("network".to_string()).counts_as_health_failure());
        assert!(ModelError::StreamInterrupted("closed".to_string()).counts_as_health_failure());
        assert!(
            ModelError::RateLimited {
                retry_after_ms: 500
            }
            .counts_as_health_failure()
        );
        assert!(!ModelError::AuthFailed.counts_as_health_failure());
        assert!(!ModelError::ContextLengthExceeded { used: 10, max: 5 }.counts_as_health_failure());
    }
}
