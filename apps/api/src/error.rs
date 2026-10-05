//! `ApiError` and its HTTP response mapping.

use super::*;

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) const fn status(&self) -> StatusCode {
        self.status
    }

    #[cfg(test)]
    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }

    #[cfg(test)]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn agent_engine_assembly(error: &anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<SelectorError>() {
            return Self {
                status: StatusCode::BAD_REQUEST,
                code: error.code(),
                message: error.to_string(),
            };
        }
        if let Some(error) = error.downcast_ref::<AgentActivationError>() {
            return Self {
                status: if matches!(error, AgentActivationError::WorkspaceSourceNotAuthorized) {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::BAD_REQUEST
                },
                code: error.code(),
                message: error.to_string(),
            };
        }
        Self::internal("failed to assemble job engine")
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub(crate) fn bad_request_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    /// `413`: the payload is larger than the route accepts.
    ///
    /// Two uses, one rule. A bounded artifact this route refuses to serve at all,
    /// where the alternative is serving the bytes without the transformation the
    /// route exists to apply; and a request body over the route's ceiling, which
    /// is raised before or during the read and never after the payload has been
    /// stored. A transport limit, so 413 rather than 400.
    pub(crate) fn payload_too_large_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn not_found_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
        }
    }

    pub(crate) fn conflict_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "bad_gateway",
            message: message.into(),
        }
    }

    pub(crate) fn bad_gateway_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn too_many_requests_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn gateway_timeout_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            code,
            message: message.into(),
        }
    }

    /// `410`. The row is intact but the payload it names cannot be served: the
    /// bytes are absent, or they no longer match the recorded length and
    /// digest. The distinction from `404` is deliberate — the client's
    /// reference was valid, so it must not be retried as a new upload.
    pub(crate) fn gone_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GONE,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn service_unavailable_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: err.to_string(),
        }
    }
}

impl From<ProductStoreError> for ApiError {
    fn from(error: ProductStoreError) -> Self {
        let status = match error.code {
            ProductErrorCode::ProductNotFound
            | ProductErrorCode::ProductMemoryNotFound
            | ProductErrorCode::ProductMcpNotFound => StatusCode::NOT_FOUND,
            ProductErrorCode::ProductInvalidInput
            | ProductErrorCode::ProductMemoryInvalidSlug
            | ProductErrorCode::ProductMcpInvalidInput
            | ProductErrorCode::ProjectTrustInvalidInput => StatusCode::BAD_REQUEST,
            ProductErrorCode::ProductStoreUnavailable
            | ProductErrorCode::ProjectTrustUnavailable
            | ProductErrorCode::ReviewUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ProductErrorCode::ProductStorageFailure => StatusCode::INTERNAL_SERVER_ERROR,
            ProductErrorCode::ProductPreviewInvalidInput => StatusCode::BAD_REQUEST,
            ProductErrorCode::ProductPreviewNotFound => StatusCode::NOT_FOUND,
            ProductErrorCode::ProductPreviewUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ProductErrorCode::ProductPreviewLimit => StatusCode::TOO_MANY_REQUESTS,
            ProductErrorCode::ProductArtifactTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ProductErrorCode::ProductAttachmentInvalidInput => StatusCode::BAD_REQUEST,
            ProductErrorCode::ProductAttachmentNotFound => StatusCode::NOT_FOUND,
            ProductErrorCode::ProductAttachmentConflict
            | ProductErrorCode::ProductAttachmentQuota => StatusCode::CONFLICT,
            ProductErrorCode::ProductAttachmentTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ProductErrorCode::ProductAttachmentBusy => StatusCode::TOO_MANY_REQUESTS,
            ProductErrorCode::ProductAttachmentTimeout => StatusCode::GATEWAY_TIMEOUT,
            ProductErrorCode::ProductAttachmentUnavailable => StatusCode::GONE,
            ProductErrorCode::ProductSessionActive
            | ProductErrorCode::ProductSessionWorkspaceMismatch
            | ProductErrorCode::ProductSessionResumeConflict
            | ProductErrorCode::ProductSessionRuntimeStateMissing
            | ProductErrorCode::ProductSessionRuntimeStateCorrupt
            | ProductErrorCode::ProductBindingCorrupt
            | ProductErrorCode::ProductRevisionConflict
            | ProductErrorCode::ProductMemoryConflict
            | ProductErrorCode::ProductMcpConflict
            | ProductErrorCode::ProjectTrustRequired
            | ProductErrorCode::MigrationIdempotencyConflict
            | ProductErrorCode::ProductControlConflict
            | ProductErrorCode::ProductControlRejected
            | ProductErrorCode::ProductForkConflict
            | ProductErrorCode::ProductForkSourceInvalid
            | ProductErrorCode::ProductSessionModelConfigConflict
            | ProductErrorCode::ProviderUnavailableForResume
            | ProductErrorCode::ProviderChangedForResume
            | ProductErrorCode::ReviewTargetUnavailable
            | ProductErrorCode::ReviewConflict
            | ProductErrorCode::ProductEventsExpired => StatusCode::CONFLICT,
            ProductErrorCode::ProductProviderProfileUnavailable => StatusCode::NOT_FOUND,
        };
        let message = if error.code == ProductErrorCode::ProductStorageFailure {
            tracing::warn!("product store operation failed: {error}");
            "product store operation failed".to_string()
        } else {
            error.message
        };
        Self {
            status,
            code: error.code.as_str(),
            message,
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        // The error envelope is the one API surface that can carry text from
        // anywhere — an upstream provider message, a store failure, a rejected
        // input echoed back by a typed rejection. Redacting here covers every
        // error path at once, including the ones that never reach the trace.
        let error = rove_runtime::secrets::registry().redact_text(&self.message);
        (
            self.status,
            Json(ApiErrorResponse {
                code: self.code.to_string(),
                error,
            }),
        )
            .into_response()
    }
}

/// Unwrap a JSON request body without ever echoing what the caller sent.
///
/// Taking `Json<T>` as a handler argument hands the rejection to axum, which
/// answers with its own `text/plain` body built from the serde error — and
/// serde prints the offending value for a type mismatch
/// (`invalid type: string "sk-live-…", expected u32`). That body is produced
/// before any handler code runs, so it never reaches `ApiError::into_response`
/// and no amount of redacting there could reach it: a caller who mistypes one
/// field would get their own credential back in the response, and into whatever
/// captured the exchange.
///
/// Taking `Result<Json<T>, JsonRejection>` instead replaces that body with a
/// fixed message under the caller's own error code. This is the same shape the
/// product routes already used; both now share one implementation so the rule
/// cannot drift between them.
pub(crate) fn json_body<T>(
    body: Result<Json<T>, JsonRejection>,
    code: &'static str,
    message: &'static str,
) -> Result<T, ApiError> {
    body.map(|Json(value)| value)
        .map_err(|_| ApiError::bad_request_with_code(code, message))
}

/// Fixed text for a rejected job/control body. Never derived from the rejection
/// itself: the rejection is the one string that can contain the caller's secret.
pub(crate) const INVALID_JOB_BODY_MESSAGE: &str = "invalid or unknown field in job request body";

pub(crate) const INVALID_PROVIDER_BODY_MESSAGE: &str =
    "invalid or unknown field in provider request body";

pub(crate) const INVALID_APPROVAL_BODY_MESSAGE: &str =
    "invalid or unknown field in approval request body";

pub(crate) const INVALID_INPUT_BODY_MESSAGE: &str =
    "invalid or unknown field in input request body";

pub(crate) const INVALID_BENCH_BODY_MESSAGE: &str =
    "invalid or unknown field in benchmark request body";

pub(crate) const INVALID_RECALL_BODY_MESSAGE: &str =
    "invalid or unknown field in recall request body";
