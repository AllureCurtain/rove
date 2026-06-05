use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, ORIGIN, RETRY_AFTER, VARY, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::ApiState;

pub(crate) async fn api_security(
    State(state): State<ApiState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let headers = request.headers().clone();
    let origin = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if let Some(origin) = origin.as_deref()
        && let Some(response) = disallowed_origin_response(&state, origin)
    {
        return response;
    }

    if let Some(response) = ensure_bearer_token(&state, &headers, origin.as_deref()) {
        return response;
    }

    if let Some(response) = ensure_rate_limit(&state, origin.as_deref()).await {
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = origin.as_deref()
        && is_origin_allowed(&state, origin)
    {
        apply_cors_headers(response.headers_mut(), origin);
    }
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("origin"));
    response
}

fn disallowed_origin_response(state: &ApiState, origin: &str) -> Option<Response> {
    if is_origin_allowed(state, origin) {
        return None;
    }

    Some((StatusCode::FORBIDDEN, "origin not allowed").into_response())
}

fn ensure_bearer_token(
    state: &ApiState,
    headers: &axum::http::HeaderMap,
    origin: Option<&str>,
) -> Option<Response> {
    let expected = state.inner.config.api.token_auth.as_deref()?;

    let authorized = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_bearer_token)
        .is_some_and(|provided| provided == expected);
    if authorized {
        return None;
    }

    let mut response = (
        StatusCode::UNAUTHORIZED,
        [(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
        "missing or invalid bearer token",
    )
        .into_response();
    apply_optional_cors_headers(response.headers_mut(), state, origin);
    Some(response)
}

async fn ensure_rate_limit(state: &ApiState, origin: Option<&str>) -> Option<Response> {
    let limit_per_minute = state.inner.config.api.rate_limit_per_minute?;

    let now = Instant::now();
    let mut rate_limit = state.inner.rate_limit.lock().await;
    match rate_limit.window_started_at {
        Some(window_started_at)
            if now.duration_since(window_started_at) >= Duration::from_secs(60) =>
        {
            rate_limit.window_started_at = Some(now);
            rate_limit.requests_in_window = 1;
        }
        Some(_) => {
            rate_limit.requests_in_window = rate_limit.requests_in_window.saturating_add(1);
        }
        None => {
            rate_limit.window_started_at = Some(now);
            rate_limit.requests_in_window = 1;
        }
    }

    if rate_limit.requests_in_window <= limit_per_minute {
        return None;
    }

    let retry_after = rate_limit
        .window_started_at
        .and_then(|started_at| started_at.checked_add(Duration::from_secs(60)))
        .and_then(|expires_at| expires_at.checked_duration_since(now))
        .map(|remaining| remaining.as_secs().max(1).to_string())
        .unwrap_or_else(|| "60".to_string());

    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        [(
            RETRY_AFTER,
            HeaderValue::from_str(&retry_after).unwrap_or(HeaderValue::from_static("60")),
        )],
        "rate limit exceeded",
    )
        .into_response();
    apply_optional_cors_headers(response.headers_mut(), state, origin);
    Some(response)
}

fn apply_optional_cors_headers(
    headers: &mut axum::http::HeaderMap,
    state: &ApiState,
    origin: Option<&str>,
) {
    if let Some(origin) = origin
        && is_origin_allowed(state, origin)
    {
        apply_cors_headers(headers, origin);
    }
}

fn apply_cors_headers(headers: &mut axum::http::HeaderMap, origin: &str) {
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(origin).unwrap_or(HeaderValue::from_static("*")),
    );
    headers.insert(VARY, HeaderValue::from_static("origin"));
}

fn is_origin_allowed(state: &ApiState, origin: &str) -> bool {
    let origins = &state.inner.config.api.cors_origins;
    !origins.is_empty() && origins.iter().any(|allowed| allowed == origin)
}

fn parse_bearer_token(header: &str) -> Option<&str> {
    let value = header.trim();
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
}
