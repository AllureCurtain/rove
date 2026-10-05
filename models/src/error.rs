use thiserror::Error;

/// Errors from LLM model interactions.
#[derive(Debug, Clone, Error)]
pub enum ModelError {
    #[error("Invalid provider configuration: {0}")]
    InvalidConfiguration(String),

    /// The call failed in a way a later attempt can clear: a transport
    /// failure, a 5xx, a connection reset, a stream that ended early.
    #[error("API request failed: {0}")]
    RequestFailed(String),

    /// The provider definitively rejected this request.
    ///
    /// This is the client-error half of a failed call: an unknown model, a
    /// malformed or oversized payload, an unprocessable entity. The identical
    /// request cannot start succeeding on its own, so a caller must not spend
    /// retry attempts — and the backoff between them — on it. Only a failure
    /// the provider can clear later is retryable.
    #[error("Provider rejected the request: {0}")]
    RequestRejected(String),

    #[error("Stream interrupted: {0}")]
    StreamInterrupted(String),

    /// The provider asked the caller to slow down.
    ///
    /// `retry_after_ms` is the provider's own instruction, and it stays `None`
    /// when the response carried no usable `Retry-After` header. A synthesized
    /// default must not take its place: it would masquerade as a provider
    /// instruction and remove the wait from the caller's own backoff curve.
    #[error("Rate limited{}", retry_after_suffix(.retry_after_ms))]
    RateLimited { retry_after_ms: Option<u64> },

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Context length exceeded: used {used} / max {max}")]
    ContextLengthExceeded { used: u32, max: u32 },
}

/// Renders the optional provider wait hint for [`ModelError::RateLimited`].
fn retry_after_suffix(retry_after_ms: &Option<u64>) -> String {
    match retry_after_ms {
        Some(ms) => format!(", retry after {ms}ms"),
        None => String::new(),
    }
}

impl ModelError {
    pub fn error_code(&self) -> &'static str {
        match self {
            ModelError::InvalidConfiguration(_) => "invalid_configuration",
            ModelError::RequestFailed(_) => "request_failed",
            ModelError::RequestRejected(_) => "request_rejected",
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

    /// Whether this failure says something about the *target's* health.
    ///
    /// A permanent rejection counts here for the same reason it did while it
    /// was reported as `RequestFailed`: it says the target cannot serve this
    /// request (an unknown model, for example), which is a fact the routing
    /// health store can act on. Retryability, not health, is what separates the
    /// two variants.
    pub fn counts_as_health_failure(&self) -> bool {
        matches!(
            self,
            ModelError::RequestFailed(_)
                | ModelError::RequestRejected(_)
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
            ModelError::RequestRejected("HTTP 404 Not Found".to_string()).error_code(),
            "request_rejected"
        );
        assert_eq!(
            ModelError::StreamInterrupted("closed".to_string()).error_code(),
            "stream_interrupted"
        );
        assert_eq!(
            ModelError::RateLimited {
                retry_after_ms: Some(500)
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
                retry_after_ms: Some(500)
            }
            .is_retryable()
        );
        assert!(!ModelError::AuthFailed.is_retryable());
        assert!(!ModelError::InvalidConfiguration("bad".to_string()).is_retryable());
        assert!(!ModelError::ContextLengthExceeded { used: 10, max: 5 }.is_retryable());
        assert!(
            !ModelError::RequestRejected("HTTP 404 Not Found".to_string()).is_retryable(),
            "a request the provider definitively rejected cannot succeed on a retry"
        );

        assert!(ModelError::RequestFailed("network".to_string()).counts_as_health_failure());
        assert!(ModelError::StreamInterrupted("closed".to_string()).counts_as_health_failure());
        assert!(
            ModelError::RateLimited {
                retry_after_ms: None
            }
            .counts_as_health_failure()
        );
        assert!(
            ModelError::RequestRejected("HTTP 404 Not Found".to_string())
                .counts_as_health_failure(),
            "a target-specific rejection is still a target health fact"
        );
        assert!(!ModelError::AuthFailed.counts_as_health_failure());
        assert!(!ModelError::InvalidConfiguration("bad".to_string()).counts_as_health_failure());
        assert!(!ModelError::ContextLengthExceeded { used: 10, max: 5 }.counts_as_health_failure());
    }

    #[test]
    fn rate_limit_display_reports_only_a_real_provider_hint() {
        assert_eq!(
            ModelError::RateLimited {
                retry_after_ms: Some(1_500)
            }
            .to_string(),
            "Rate limited, retry after 1500ms"
        );
        assert_eq!(
            ModelError::RateLimited {
                retry_after_ms: None
            }
            .to_string(),
            "Rate limited",
            "a header-less 429 carries no provider instruction to display"
        );
    }
}
