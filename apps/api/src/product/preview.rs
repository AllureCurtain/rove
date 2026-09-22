//! Executable local HTML preview on an isolated loopback origin (plan P5b).
//!
//! Threat model:
//! `docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md`.
//!
//! - Previewed pages run on a dedicated loopback origin (its own ephemeral
//!   port, no product cookies, no BearerAuth middleware), so page scripts
//!   cannot read the product token or call the product API as the user
//!   (A1/A2). The product surface only ever creates or closes sessions.
//! - Every resource resolves through the same `join_safe` /
//!   `is_secret_filename` / regular-file / byte-cap discipline as the
//!   read-only file API (A3/A4/A6/A10).
//! - A session carries a 160-bit random token, a fixed TTL, and is revoked
//!   on close or process exit (A7/A8). The token only ever appears in the
//!   URL returned to the session creator; it is never logged, traced, or
//!   included in error bodies.
//! - The service serves existing workspace files only. It never runs
//!   scripts, builds, or `package scripts`, and never proxies remote
//!   content (A5/A9); `connect-src 'none'` keeps previewed pages from
//!   exfiltrating workspace content.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, Semaphore, SemaphorePermit};
use utoipa::ToSchema;

use super::files::{guess_mime, join_safe, require_regular_file, sniff_mime, workspace_root};
use super::trust::{ensure_workspace_read_allowed, product_workspace_kind};
use super::{ProductErrorCode, ProductWorkspaceId};
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

/// At most this many preview sessions stay open at once (A10).
pub(crate) const MAX_PREVIEW_SESSIONS: usize = 8;
/// A preview session expires this long after creation (A8).
pub(crate) const PREVIEW_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// A single previewed resource may not exceed this size (A10).
pub(crate) const MAX_PREVIEW_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Concurrent preview resource requests across all sessions (A10).
pub(crate) const MAX_PREVIEW_CONCURRENT_REQUESTS: usize = 16;
/// One resource load must finish within this budget (A10).
pub(crate) const PREVIEW_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The preview origin executes project HTML, so scripts and styles from the
/// preview root itself are allowed; everything outbound is not (A5/A12).
const PREVIEW_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self'; connect-src 'none'; form-action 'none'; base-uri 'none'";

/// One open preview session. The 160-bit access token is only ever held as
/// the registry map key, never inside this struct, so `Debug` cannot leak it
/// (A7).
#[derive(Debug)]
pub(crate) struct PreviewSession {
    pub preview_id: String,
    pub workspace_id: ProductWorkspaceId,
    pub root: PathBuf,
    pub entry: String,
    expires_instant: Instant,
}

#[derive(Debug)]
struct PreviewRegistryInner {
    sessions: RwLock<HashMap<String, Arc<PreviewSession>>>,
    bound_addr: RwLock<Option<SocketAddr>>,
    listener_error: RwLock<Option<String>>,
    request_slots: Semaphore,
    session_ttl: Duration,
    max_sessions: usize,
}

/// Process-local preview session registry shared by the product routes
/// (create/close) and the isolated preview router (resource reads).
#[derive(Debug, Clone)]
pub(crate) struct PreviewRegistry {
    inner: Arc<PreviewRegistryInner>,
}

impl Default for PreviewRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewRegistry {
    pub(crate) fn new() -> Self {
        Self::with_limits(PREVIEW_SESSION_TTL, MAX_PREVIEW_SESSIONS)
    }

    fn with_limits(session_ttl: Duration, max_sessions: usize) -> Self {
        Self {
            inner: Arc::new(PreviewRegistryInner {
                sessions: RwLock::new(HashMap::new()),
                bound_addr: RwLock::new(None),
                listener_error: RwLock::new(None),
                request_slots: Semaphore::new(MAX_PREVIEW_CONCURRENT_REQUESTS),
                session_ttl,
                max_sessions,
            }),
        }
    }

    pub(crate) async fn record_listener_bound(&self, addr: SocketAddr) {
        *self.inner.bound_addr.write().await = Some(addr);
    }

    pub(crate) async fn record_listener_unavailable(&self, error: String) {
        *self.inner.listener_error.write().await = Some(error);
    }

    pub(crate) async fn origin_addr(&self) -> Option<SocketAddr> {
        *self.inner.bound_addr.read().await
    }

    fn try_acquire_request_slot(&self) -> Option<SemaphorePermit<'_>> {
        self.inner.request_slots.try_acquire().ok()
    }

    /// Open a session for `entry` under `root`. The returned URL embeds the
    /// session token; callers must keep it out of logs and traces (A7).
    pub(crate) async fn create_session(
        &self,
        workspace_id: ProductWorkspaceId,
        root: PathBuf,
        entry: String,
    ) -> Result<ProductPreviewSession, ApiError> {
        let addr = match self.origin_addr().await {
            Some(addr) => addr,
            None => {
                let detail = self.inner.listener_error.read().await.clone();
                return Err(ApiError::service_unavailable_with_code(
                    ProductErrorCode::ProductPreviewUnavailable.as_str(),
                    match detail {
                        Some(detail) => {
                            format!("preview listener is not available: {detail}")
                        }
                        None => "preview listener is not bound on this host".to_string(),
                    },
                ));
            }
        };

        let mut sessions = self.inner.sessions.write().await;
        let now = Instant::now();
        sessions.retain(|_, session| session.expires_instant > now);
        if sessions.len() >= self.inner.max_sessions {
            return Err(ApiError::too_many_requests_with_code(
                ProductErrorCode::ProductPreviewLimit.as_str(),
                "too many open preview sessions; close one before opening another",
            ));
        }

        let preview_id = ulid::Ulid::new().to_string();
        // Two ULIDs give 160 bits of randomness; the token is the only
        // capability the preview origin checks (A7).
        let token = format!("{}{}", ulid::Ulid::new(), ulid::Ulid::new());
        let created_at = Utc::now();
        let ttl_seconds = i64::try_from(self.inner.session_ttl.as_secs()).unwrap_or(i64::MAX);
        let expires_at = created_at + chrono::Duration::seconds(ttl_seconds);
        let session = Arc::new(PreviewSession {
            preview_id: preview_id.clone(),
            workspace_id: workspace_id.clone(),
            root,
            entry: entry.clone(),
            expires_instant: now + self.inner.session_ttl,
        });
        sessions.insert(token.clone(), session);

        Ok(ProductPreviewSession {
            preview_id,
            workspace_id,
            url: format!("http://{addr}/{token}/{}", percent_encode_path(&entry)),
            entry,
            created_at: created_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        })
    }

    /// Resolve a live session by token, removing it lazily once expired (A8).
    async fn resolve(&self, token: &str) -> Option<Arc<PreviewSession>> {
        let session = self.inner.sessions.read().await.get(token).cloned()?;
        if session.expires_instant <= Instant::now() {
            self.inner.sessions.write().await.remove(token);
            return None;
        }
        Some(session)
    }

    /// Close the session owned by `workspace_id` with id `preview_id`.
    /// Returns `false` when no matching live session exists.
    pub(crate) async fn close_session(
        &self,
        workspace_id: &ProductWorkspaceId,
        preview_id: &str,
    ) -> bool {
        let mut sessions = self.inner.sessions.write().await;
        let token = sessions
            .iter()
            .find(|(_, session)| {
                session.preview_id == preview_id && &session.workspace_id == workspace_id
            })
            .map(|(token, _)| token.clone());
        match token {
            Some(token) => sessions.remove(&token).is_some(),
            None => false,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProductPreviewRequest {
    /// Workspace-relative path of the HTML entry file (e.g. `dist/index.html`).
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductPreviewSession {
    /// Server-owned preview session id; safe to log. Not the access token.
    pub preview_id: String,
    pub workspace_id: ProductWorkspaceId,
    /// Workspace-relative entry path the session was opened for.
    pub entry: String,
    /// Absolute loopback URL that opens the preview, embedding the session
    /// token. Keep it out of logs, traces, and exports.
    pub url: String,
    pub created_at: String,
    pub expires_at: String,
}

#[utoipa::path(
    post,
    path = "/product/workspaces/{workspace_id}/previews",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id")),
    request_body = CreateProductPreviewRequest,
    responses(
        (status = 201, description = "Preview session created on the isolated loopback origin", body = ProductPreviewSession),
        (status = 400, description = "Entry path is invalid, secret-shaped, or not an HTML file", body = ApiErrorResponse),
        (status = 404, description = "Workspace or entry file not found", body = ApiErrorResponse),
        (status = 409, description = "Project trust was revoked for this workspace", body = ApiErrorResponse),
        (status = 429, description = "Too many open preview sessions", body = ApiErrorResponse),
        (status = 500, description = "Workspace read or path resolution failed", body = ApiErrorResponse),
        (status = 503, description = "Preview listener is unavailable on this host", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_preview(
    State(state): State<ApiState>,
    AxumPath(workspace_id): AxumPath<ProductWorkspaceId>,
    body: Result<Json<CreateProductPreviewRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductPreviewSession>), ApiError> {
    let request = super::routes::product_json(body)?;
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let root = workspace_root(&workspace.kind, &workspace.canonical_root)?;
    // Opening an executable preview is a read of workspace content, so it
    // honours the same project-trust boundary as the file API (A11).
    ensure_workspace_read_allowed(&state, &root, product_workspace_kind(&workspace.kind)).await?;

    let entry_path = join_safe(&root, &request.path).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductPreviewInvalidInput.as_str(),
            "preview entry must be a workspace-relative, non-secret path",
        )
    })?;
    require_regular_file(&entry_path).await.map_err(|error| {
        if error.status() == StatusCode::NOT_FOUND {
            ApiError::not_found_with_code(
                ProductErrorCode::ProductPreviewNotFound.as_str(),
                "preview entry file not found",
            )
        } else {
            ApiError::bad_request_with_code(
                ProductErrorCode::ProductPreviewInvalidInput.as_str(),
                "preview entry is not a regular file",
            )
        }
    })?;
    let is_html = entry_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "html" | "htm"));
    if !is_html {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductPreviewInvalidInput.as_str(),
            "preview entry must be an .html or .htm file",
        ));
    }

    let entry = relative_posix_path(&root, &entry_path)?;
    let session = state
        .inner
        .preview
        .create_session(workspace_id, root, entry)
        .await?;
    Ok((StatusCode::CREATED, Json(session)))
}

#[utoipa::path(
    delete,
    path = "/product/workspaces/{workspace_id}/previews/{preview_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id"),
        ("preview_id" = String, Path, description = "Preview session id returned at creation")
    ),
    responses(
        (status = 204, description = "Preview session closed; its token no longer resolves"),
        (status = 404, description = "Workspace or preview session not found", body = ApiErrorResponse),
        (status = 500, description = "ProductStore lookup failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn close_product_preview(
    State(state): State<ApiState>,
    AxumPath((workspace_id, preview_id)): AxumPath<(ProductWorkspaceId, String)>,
) -> Result<StatusCode, ApiError> {
    state.product_store()?.get_workspace(&workspace_id).await?;
    if state
        .inner
        .preview
        .close_session(&workspace_id, &preview_id)
        .await
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found_with_code(
            ProductErrorCode::ProductPreviewNotFound.as_str(),
            "preview session not found or already closed",
        ))
    }
}

/// Router for the isolated preview origin. It intentionally shares nothing
/// with the product router: no BearerAuth middleware, no CORS allowance, no
/// cookies, and no product routes (A1/A2).
pub(crate) fn preview_router(registry: PreviewRegistry) -> Router {
    Router::new()
        .route("/{token}", get(preview_entry_redirect))
        .route("/{token}/{*resource}", get(serve_preview_resource))
        .with_state(registry)
}

async fn preview_entry_redirect(
    State(registry): State<PreviewRegistry>,
    AxumPath(token): AxumPath<String>,
) -> Response<Body> {
    let _permit = match registry.try_acquire_request_slot() {
        Some(permit) => permit,
        None => {
            return preview_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "preview request limit reached",
            );
        }
    };
    match registry.resolve(&token).await {
        Some(session) => redirect_to_entry(&token, &session.entry),
        None => preview_error_response(
            StatusCode::NOT_FOUND,
            "preview session not found or expired",
        ),
    }
}

async fn serve_preview_resource(
    State(registry): State<PreviewRegistry>,
    AxumPath((token, resource)): AxumPath<(String, String)>,
) -> Response<Body> {
    let _permit = match registry.try_acquire_request_slot() {
        Some(permit) => permit,
        None => {
            return preview_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "preview request limit reached",
            );
        }
    };
    match tokio::time::timeout(
        PREVIEW_REQUEST_TIMEOUT,
        load_preview_resource(&registry, &token, &resource),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => preview_error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "preview resource load timed out",
        ),
    }
}

async fn load_preview_resource(
    registry: &PreviewRegistry,
    token: &str,
    resource: &str,
) -> Response<Body> {
    let Some(session) = registry.resolve(token).await else {
        return preview_error_response(
            StatusCode::NOT_FOUND,
            "preview session not found or expired",
        );
    };
    if resource.is_empty() {
        return redirect_to_entry(token, &session.entry);
    }

    // A3/A4/A6: the exact same path discipline as the read-only file API.
    let path = match join_safe(&session.root, resource) {
        Ok(path) => path,
        Err(_) => {
            return preview_error_response(
                StatusCode::BAD_REQUEST,
                "path is not allowed within the preview root",
            );
        }
    };
    let Ok(metadata) = require_regular_file(&path).await else {
        return preview_error_response(StatusCode::NOT_FOUND, "preview resource not found");
    };
    if metadata.len() > MAX_PREVIEW_FILE_BYTES {
        return preview_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "preview resource exceeds the size limit",
        );
    }
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return preview_error_response(StatusCode::NOT_FOUND, "preview resource not found");
        }
    };
    // TOCTOU guard: the file may have grown between metadata and read.
    if bytes.len() as u64 > MAX_PREVIEW_FILE_BYTES {
        return preview_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "preview resource exceeds the size limit",
        );
    }

    let mime = sniff_mime(&bytes).unwrap_or_else(|| guess_mime(&path));
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    apply_preview_security_headers(headers);
    response
}
fn redirect_to_entry(token: &str, entry: &str) -> Response<Body> {
    let location = format!("/{token}/{}", percent_encode_path(entry));
    let mut response = (
        StatusCode::TEMPORARY_REDIRECT,
        [(LOCATION, location)],
        Body::empty(),
    )
        .into_response();
    apply_preview_security_headers(response.headers_mut());
    response
}

/// Short plain-text errors with the same isolation headers as resources; the
/// body never echoes the token, the path, or filesystem details (A7).
fn preview_error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = (status, message).into_response();
    apply_preview_security_headers(response.headers_mut());
    response
}

fn apply_preview_security_headers(headers: &mut axum::http::HeaderMap) {
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PREVIEW_CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

/// Percent-encode one workspace-relative path for use in a URL path,
/// keeping `/` segment separators intact.
fn percent_encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Re-express a canonical file under `root` as a forward-slash relative path
/// so preview URLs are platform-stable.
fn relative_posix_path(root: &std::path::Path, full: &std::path::Path) -> Result<String, ApiError> {
    let relative = full.strip_prefix(root).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductPreviewInvalidInput.as_str(),
            "preview entry must stay inside the workspace root",
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        if let std::path::Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    if parts.is_empty() {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductPreviewInvalidInput.as_str(),
            "preview entry must name a file, not the workspace root",
        ));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_id() -> ProductWorkspaceId {
        ProductWorkspaceId::new()
    }

    async fn bound_registry(session_ttl: Duration, max_sessions: usize) -> PreviewRegistry {
        let registry = PreviewRegistry::with_limits(session_ttl, max_sessions);
        registry
            .record_listener_bound("127.0.0.1:9081".parse().unwrap())
            .await;
        registry
    }

    #[tokio::test]
    async fn create_fails_closed_until_the_listener_is_bound() {
        let registry = PreviewRegistry::new();
        let error = registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code(), "product_preview_unavailable");

        registry
            .record_listener_unavailable("bind failed".to_string())
            .await;
        let error = registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.message().contains("bind failed"));
    }

    #[tokio::test]
    async fn session_cap_is_enforced_with_a_typed_limit_error() {
        let registry = bound_registry(PREVIEW_SESSION_TTL, 2).await;
        for _ in 0..2 {
            registry
                .create_session(
                    workspace_id(),
                    PathBuf::from("/tmp/work"),
                    "index.html".into(),
                )
                .await
                .unwrap();
        }
        let error = registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.code(), "product_preview_limit");
    }

    #[tokio::test]
    async fn expired_sessions_do_not_resolve_and_the_next_create_reclaims_slots() {
        let registry = bound_registry(Duration::ZERO, 1).await;
        let session = registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap();
        let token = session.url.split('/').nth(3).unwrap().to_string();
        assert!(registry.resolve(&token).await.is_none());
        // The lazy sweep on the next create must have reclaimed the slot.
        registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn close_revokes_the_token_and_is_scoped_to_the_workspace() {
        let registry = bound_registry(PREVIEW_SESSION_TTL, MAX_PREVIEW_SESSIONS).await;
        let owner = workspace_id();
        let session = registry
            .create_session(
                owner.clone(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap();
        let token = session.url.split('/').nth(3).unwrap().to_string();
        assert!(registry.resolve(&token).await.is_some());

        let other = ProductWorkspaceId::new();
        assert!(!registry.close_session(&other, &session.preview_id).await);
        assert!(registry.resolve(&token).await.is_some());

        assert!(registry.close_session(&owner, &session.preview_id).await);
        assert!(registry.resolve(&token).await.is_none());
        assert!(!registry.close_session(&owner, &session.preview_id).await);
    }

    #[tokio::test]
    async fn the_url_embeds_the_token_but_debug_and_errors_redact_it() {
        let registry = bound_registry(PREVIEW_SESSION_TTL, MAX_PREVIEW_SESSIONS).await;
        let session = registry
            .create_session(
                workspace_id(),
                PathBuf::from("/tmp/work"),
                "index.html".into(),
            )
            .await
            .unwrap();
        let token = session.url.split('/').nth(3).unwrap().to_string();
        assert_eq!(token.len(), 52);
        let resolved = registry.resolve(&token).await.unwrap();
        // The token is the registry key, never a struct field, so even a
        // `Debug` dump of the session cannot leak it.
        let debug = format!("{resolved:?}");
        assert!(!debug.contains(&token));
    }

    #[test]
    fn percent_encode_path_keeps_segments_and_escapes_bytes() {
        assert_eq!(percent_encode_path("dist/index.html"), "dist/index.html");
        assert_eq!(
            percent_encode_path("my dir/首页.html"),
            "my%20dir/%E9%A6%96%E9%A1%B5.html"
        );
    }
}
