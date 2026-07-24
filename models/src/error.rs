use thiserror::Error;

/// Errors from LLM model interactions.
#[derive(Debug, Clone, Error)]
pub enum ModelError {
    #[error("Invalid provider configuration: {0}")]
    InvalidConfiguration(String),

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
            ModelError::InvalidConfiguration(_) => "invalid_configuration",
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

#[cfg(test)]
mod tests {
    use super::ModelError;

    #[test]
    fn model_error_codes_are_stable() {
        assert_eq!(
            ModelError::InvalidConfiguration("bad endpoint".to_string()).error_code(),
            "invalid_configuration"
        );
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
        assert!(!ModelError::InvalidConfiguration("bad".to_string()).is_retryable());
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
        assert!(!ModelError::InvalidConfiguration("bad".to_string()).counts_as_health_failure());
        assert!(!ModelError::ContextLengthExceeded { used: 10, max: 5 }.counts_as_health_failure());
    }
}
