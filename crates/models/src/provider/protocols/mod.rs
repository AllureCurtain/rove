mod anthropic;
mod ollama;
mod openai_completions;
mod openai_responses;

pub use anthropic::AnthropicMessagesProtocol;
pub use ollama::OllamaChatProtocol;
pub use openai_completions::OpenAiCompletionsProtocol;
pub use openai_responses::OpenAiResponsesProtocol;

use reqwest::StatusCode;

use crate::ModelError;

/// The normalized error for an HTTP failure that is neither authentication,
/// throttling, nor a context-length condition.
///
/// The split matters to every caller that can retry: a 4xx status is the
/// provider definitively rejecting *this* request (unknown model, malformed
/// payload, too large, unprocessable), so the identical request cannot start
/// succeeding and a retry only burns the budget and the backoff before failing
/// the same way. A 5xx, or a status this adapter does not know, stays
/// [`ModelError::RequestFailed`] so a later attempt can clear it.
///
/// Two 4xx statuses are not rejections and stay retryable:
/// `408 Request Timeout` is the server giving up on waiting for us, and
/// `409 Conflict` (and the `425 Too Early` some gateways return while a
/// deployment is rolling) name a concurrent condition that a later attempt can
/// win.
pub(crate) fn classify_http_failure(status: StatusCode, body: &str) -> ModelError {
    let message = format!("HTTP {status}: {body}");
    if is_permanent_rejection(status) {
        ModelError::RequestRejected(message)
    } else {
        ModelError::RequestFailed(message)
    }
}

fn is_permanent_rejection(status: StatusCode) -> bool {
    status.is_client_error()
        && !matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::CONFLICT | StatusCode::TOO_EARLY
        )
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use crate::ModelError;

    use super::classify_http_failure;

    #[test]
    fn client_errors_are_rejections_and_server_errors_are_transient() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::NOT_FOUND,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::GONE,
        ] {
            assert!(
                matches!(
                    classify_http_failure(status, "body"),
                    ModelError::RequestRejected(_)
                ),
                "{status} rejects this request and must not be retried"
            );
        }

        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                matches!(
                    classify_http_failure(status, "body"),
                    ModelError::RequestFailed(_)
                ),
                "{status} is a provider condition a later attempt can clear"
            );
        }

        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::CONFLICT,
            StatusCode::TOO_EARLY,
        ] {
            assert!(
                matches!(
                    classify_http_failure(status, "body"),
                    ModelError::RequestFailed(_)
                ),
                "{status} is 4xx but names a condition a later attempt can win"
            );
        }
    }

    #[test]
    fn the_rejection_message_keeps_the_status_and_body() {
        assert_eq!(
            classify_http_failure(StatusCode::NOT_FOUND, "model not found").to_string(),
            "Provider rejected the request: HTTP 404 Not Found: model not found"
        );
    }
}
