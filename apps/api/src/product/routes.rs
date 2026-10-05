//! Coordinator-owned product route surface.
//!
//! Catalog handlers delegate to the API-global product store. Migration stays
//! fail-closed until the coordinator validates browser runtime hints against
//! workspace-owned runtime state.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use utoipa::IntoParams;

use super::*;
use crate::docs;
use rove_app_bootstrap::model_supports_images;

use crate::{ApiError, ApiErrorResponse, ApiState};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListProductSessionsQuery {
    pub workspace_id: ProductWorkspaceId,
    /// Opaque token from a previous response's `next_cursor`. Omit for page one.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Case-insensitive substring match on the session title.
    #[serde(default)]
    pub q: Option<String>,
    /// Archived sessions are included by default, sorted after live ones.
    ///
    /// The default preserves the pre-pagination response, which returned them:
    /// hiding them server-side would have made every existing client's list
    /// quietly shorter. Clients that never show archived sessions can now say so
    /// and stop paying to transfer them.
    #[serde(default)]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeleteProviderProfileQuery {
    pub expected_revision: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListProductMessagesQuery {
    #[serde(default)]
    pub after_seq: Option<i64>,
    #[serde(default)]
    pub before_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Unwrap a product JSON body through the shared rejection-discarding rule.
///
/// The product message/code pair is the only difference from the job routes:
/// the reason the rejection text is dropped lives in `crate::json_body`.
pub(super) fn product_json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    crate::json_body(
        body,
        ProductErrorCode::ProductInvalidInput.as_str(),
        "invalid or unknown field in product request body",
    )
}

async fn complete_after_bounded_migration_preparation<P, T, Prepare, Apply, ApplyFuture>(
    deadline: Duration,
    prepare: Prepare,
    apply: Apply,
) -> Result<T, ApiError>
where
    Prepare: Future<Output = Result<P, ApiError>>,
    Apply: FnOnce(P) -> ApplyFuture,
    ApplyFuture: Future<Output = Result<T, ApiError>>,
{
    let prepared = tokio::time::timeout(deadline, prepare)
        .await
        .map_err(|_| {
            ApiError::gateway_timeout_with_code(
                ProductErrorCode::ProductStorageFailure.as_str(),
                "browser migration exceeded its preparation deadline before commit",
            )
        })??;
    apply(prepared).await
}

enum M1MigrationPreparation {
    Replay(M1BrowserMigrationResponse),
    Apply(super::migration::GuardedM1BrowserMigration),
}

#[utoipa::path(
    get,
    path = "/product/workspaces",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Known product workspaces", body = ProductWorkspacesResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_workspaces(
    State(state): State<ApiState>,
) -> Result<Json<ProductWorkspacesResponse>, ApiError> {
    let workspaces = state.product_store()?.list_workspaces().await?;
    Ok(Json(ProductWorkspacesResponse { workspaces }))
}

#[utoipa::path(
    post,
    path = "/product/workspaces",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = CreateProductWorkspaceRequest,
    responses(
        (status = 201, description = "Product workspace created", body = ProductWorkspace),
        (status = 400, description = "Invalid workspace", body = ApiErrorResponse),
        (status = 409, description = "Workspace conflicts with an existing entry", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_workspace(
    State(state): State<ApiState>,
    body: Result<Json<CreateProductWorkspaceRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductWorkspace>), ApiError> {
    let request = product_json(body)?;
    let workspace = state.product_store()?.create_workspace(request).await?;
    state.notify_product_events();
    Ok((StatusCode::CREATED, Json(workspace)))
}

#[utoipa::path(
    delete,
    path = "/product/workspaces/{workspace_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id")),
    responses(
        (status = 204, description = "Catalog entry deleted; workspace files are untouched"),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 409, description = "Workspace has a session with an active turn", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_workspace(
    State(state): State<ApiState>,
    Path(workspace_id): Path<ProductWorkspaceId>,
) -> Result<StatusCode, ApiError> {
    state
        .product_store()?
        .delete_workspace(&workspace_id)
        .await?;
    state.notify_product_events();
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/product/sessions",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ListProductSessionsQuery),
    responses(
        (status = 200, description = "One page of a workspace's product sessions", body = ProductSessionsResponse),
        (status = 400, description = "Page limit, cursor, or search term is invalid", body = ApiErrorResponse),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_sessions(
    State(state): State<ApiState>,
    Query(query): Query<ListProductSessionsQuery>,
) -> Result<Json<ProductSessionsResponse>, ApiError> {
    let page = state
        .product_store()?
        .list_sessions(session_page_query(query)?)
        .await?;
    Ok(Json(ProductSessionsResponse {
        sessions: page.sessions,
        next_cursor: page.next_cursor.map(|cursor| cursor.encode()),
    }))
}

/// Validate and resolve a listing request.
///
/// Every rejection is deliberate. A limit of zero or a broken cursor would
/// otherwise return an empty page, which a client cannot distinguish from
/// having reached the end — it would stop paging and silently lose rows.
fn session_page_query(
    query: ListProductSessionsQuery,
) -> Result<ProductSessionPageQuery, ApiError> {
    let invalid = || {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "session page query is invalid",
        )
    };
    let limit = query.limit.unwrap_or(DEFAULT_PRODUCT_SESSION_PAGE_LIMIT);
    if limit == 0 || limit > MAX_PRODUCT_SESSION_PAGE_LIMIT {
        return Err(invalid());
    }
    let cursor = match query.cursor.as_deref() {
        Some(encoded) => Some(ProductSessionCursor::decode(encoded).map_err(|_| invalid())?),
        None => None,
    };
    // A term of only whitespace is treated as no filter rather than as a search
    // for a space, which would match nearly every title.
    let search = match query.q.as_deref().map(str::trim) {
        Some("") => None,
        Some(term) if term.len() > MAX_PRODUCT_SESSION_QUERY_BYTES => return Err(invalid()),
        Some(term) => Some(term.to_string()),
        None => None,
    };
    Ok(ProductSessionPageQuery {
        workspace_id: query.workspace_id,
        cursor,
        limit,
        search,
        include_archived: query.include_archived.unwrap_or(true),
    })
}

#[utoipa::path(
    post,
    path = "/product/sessions",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = CreateProductSessionRequest,
    responses(
        (status = 201, description = "Server-owned product session created", body = ProductSession),
        (status = 400, description = "Invalid session", body = ApiErrorResponse),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_session(
    State(state): State<ApiState>,
    body: Result<Json<CreateProductSessionRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductSession>), ApiError> {
    let request = product_json(body)?;
    let session = state.product_store()?.create_session(request).await?;
    state.notify_product_events();
    Ok((StatusCode::CREATED, Json(session)))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/forks",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Parent product session id")),
    request_body = CreateProductForkRequest,
    responses(
        (status = 201, description = "Child session forked from an exact final runtime boundary", body = ProductForkResponse),
        (status = 200, description = "Idempotent replay of the same fork", body = ProductForkResponse),
        (status = 400, description = "Invalid fork request, including a truncation target that is not a user message sequence", body = ApiErrorResponse),
        (status = 404, description = "Parent product session not found", body = ApiErrorResponse),
        (status = 409, description = "Source is active, incomplete, corrupt, or conflicts with an idempotency key", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_session_fork(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<CreateProductForkRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductForkResponse>), ApiError> {
    let request = product_json(body)?;
    let store = state.product_store()?;
    if let Some((session, fork)) = store.replay_fork(&session_id, &request).await? {
        return Ok((StatusCode::OK, Json(ProductForkResponse { fork, session })));
    }
    let boundary = crate::verify_product_fork_boundary(
        &state,
        &session_id,
        request.fork_at_run_id,
        request.truncate_after_message_seq,
    )
    .await?;
    let (session, fork, already_exists) = store.create_fork(request, boundary).await?;
    let status = if already_exists {
        StatusCode::OK
    } else {
        // A replayed fork is not a new fact, so it does not wake the stream.
        state.notify_product_events();
        StatusCode::CREATED
    };
    Ok((status, Json(ProductForkResponse { fork, session })))
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/forks",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Parent product session id, including deleted-parent provenance")),
    responses(
        (status = 200, description = "Direct immutable forks from this parent", body = ProductForksResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_session_forks(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductForksResponse>, ApiError> {
    let forks = state.product_store()?.list_forks(&session_id).await?;
    Ok(Json(ProductForksResponse { forks }))
}

#[utoipa::path(
    patch,
    path = "/product/sessions/{session_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    request_body = UpdateProductSessionRequest,
    responses(
        (status = 200, description = "Product session updated", body = ProductSession),
        (status = 400, description = "Invalid update", body = ApiErrorResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 409, description = "Session has an active turn", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_session(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<UpdateProductSessionRequest>, JsonRejection>,
) -> Result<Json<ProductSession>, ApiError> {
    let request = product_json(body)?;
    let session = state
        .product_store()?
        .update_session(&session_id, request)
        .await?;
    state.notify_product_events();
    Ok(Json(session))
}

#[utoipa::path(
    delete,
    path = "/product/sessions/{session_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 204, description = "Product session metadata deleted; runtime artifacts are untouched"),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 409, description = "Session has an active turn", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_session(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<StatusCode, ApiError> {
    state.product_store()?.delete_session(&session_id).await?;
    // Rows first, then bytes. The delete cascades every attachment row for this
    // session, so once it returns nothing the store still names lives in this
    // directory. Removing it is best effort: a residue that cannot be deleted
    // (a file another process holds open) is reclaimed by the next cleanup run,
    // and it is never reachable, because the rows that would have authorised the
    // download are already gone.
    if state
        .attachment_storage()
        .remove_session_dir(&session_id)
        .await
        .is_err()
    {
        tracing::warn!(
            product_session_id = %session_id,
            "deleted product session left attachment payloads behind; the cleanup run will reclaim them"
        );
    }
    state.notify_product_events();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct GetProductTranscriptQuery {
    /// Cursor from a previous page's `next_before_ordinal`. Returns only runs
    /// older than that ordinal, newest first.
    #[serde(default)]
    pub before_ordinal: Option<u64>,
    /// Maximum runs in this page. Omit for the bounded legacy window.
    #[serde(default)]
    pub limit_runs: Option<usize>,
    /// Newest canonical-event contract version this client can decode, as
    /// defined by `STREAM_EVENT_KINDS` in `runtime/src/foundation/events.rs`.
    /// Omit to keep the legacy contract: every durable row the server can decode
    /// is delivered. When a version is declared, a durable row whose kind
    /// entered the contract after it is withheld and reported as an
    /// `unknown_event_type` partial reason instead of failing the response.
    /// Decimal digits only: a leading `+` is malformed, and a value of `0` or one
    /// longer than ten digits is refused as `product_invalid_input`.
    #[serde(default)]
    pub event_contract: Option<String>,
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/transcript",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        GetProductTranscriptQuery
    ),
    responses(
        (status = 200, description = "Canonical-event transcript projection. A run segment reports `unknown_event_type` with the withheld `expected_seq`/`observed_seq` range when a declared `event_contract` cannot decode a durable row; delivered events always keep their own durable `seq`.", body = ProductTranscriptResponse),
        (status = 400, description = "Invalid transcript page query or event contract", body = ApiErrorResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 500, description = "Product transcript projection failed", body = ApiErrorResponse),
        (status = 503, description = "Product transcript projector is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_session_transcript(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    Query(query): Query<GetProductTranscriptQuery>,
) -> Result<Json<ProductTranscriptResponse>, ApiError> {
    if query.before_ordinal == Some(0)
        || query
            .limit_runs
            .is_some_and(|limit| limit == 0 || limit > MAX_TRANSCRIPT_PAGE_RUNS)
    {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "transcript page query is invalid",
        ));
    }
    let event_contract = query
        .event_contract
        .as_deref()
        .map(|raw| {
            ProductTranscriptEventContract::parse(raw).ok_or_else(|| {
                ApiError::bad_request_with_code(
                    ProductErrorCode::ProductInvalidInput.as_str(),
                    "event_contract must be a positive integer canonical-event contract version",
                )
            })
        })
        .transpose()?;
    let transcript = state
        .product_transcript_reader()?
        .read_transcript(
            &session_id,
            ProductTranscriptQuery {
                before_ordinal: query.before_ordinal,
                limit_runs: query.limit_runs,
                event_contract,
            },
        )
        .await?;
    Ok(Json(transcript))
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/model-config",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 200, description = "Session-scoped model configuration", body = ProductSessionModelConfig),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_session_model_config(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductSessionModelConfig>, ApiError> {
    Ok(Json(
        state
            .product_store()?
            .get_session_model_config(&session_id)
            .await?,
    ))
}

#[utoipa::path(
    put,
    path = "/product/sessions/{session_id}/model-config",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    request_body = UpdateProductSessionModelConfigRequest,
    responses(
        (status = 200, description = "Session-scoped model configuration updated", body = ProductSessionModelConfig),
        (status = 400, description = "Invalid model configuration", body = ApiErrorResponse),
        (status = 409, description = "Session model revision conflict", body = ApiErrorResponse),
        (status = 404, description = "Product session or provider profile not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_session_model_config(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<UpdateProductSessionModelConfigRequest>, JsonRejection>,
) -> Result<Json<ProductSessionModelConfig>, ApiError> {
    let request = product_json(body)?;
    if let Some(profile_id) = request.profile_id.as_ref() {
        let catalog = state.provider_catalog().await?;
        let profile = super::provider_catalog::get(&catalog, profile_id)?;
        state
            .product_store()?
            .upsert_provider_catalog_identity(
                &profile.id,
                &profile.label,
                profile.provider_type,
                &profile.catalog_revision,
            )
            .await?;
    }
    Ok(Json(
        state
            .product_store()?
            .update_session_model_config(&session_id, request)
            .await?,
    ))
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/run-models",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 200, description = "Immutable model snapshots for product runs", body = ProductSessionRunModelsResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_session_run_models(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductSessionRunModelsResponse>, ApiError> {
    let runs = state
        .product_store()?
        .list_session_run_models(&session_id)
        .await?;
    Ok(Json(ProductSessionRunModelsResponse { runs }))
}

#[utoipa::path(
    get,
    path = "/product/provider-profiles",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Persisted secret-reference-only provider profiles", body = ProductProviderProfilesResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_provider_profiles(
    State(state): State<ApiState>,
) -> Result<Json<ProductProviderProfilesResponse>, ApiError> {
    let catalog = state.provider_catalog().await?;
    let provider_profiles = super::provider_catalog::list(&catalog)?;
    Ok(Json(ProductProviderProfilesResponse {
        catalog_revision: catalog.revision().to_string(),
        provider_profiles,
    }))
}

#[utoipa::path(
    post,
    path = "/product/provider-profiles",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = CreateProductProviderProfileRequest,
    responses(
        (status = 201, description = "Provider profile created", body = ProductProviderProfile),
        (status = 400, description = "Invalid profile or secret-shaped field", body = ApiErrorResponse),
        (status = 409, description = "Provider catalog revision conflict", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_provider_profile(
    State(state): State<ApiState>,
    body: Result<Json<CreateProductProviderProfileRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductProviderProfile>), ApiError> {
    let request = product_json(body)?;
    let service = state.provider_catalog_service();
    let profile =
        tokio::task::spawn_blocking(move || super::provider_catalog::create(&service, request))
            .await
            .map_err(|_| ApiError::internal("provider catalog operation did not complete"))??;
    state
        .product_store()?
        .upsert_provider_catalog_identity(
            &profile.id,
            &profile.label,
            profile.provider_type,
            &profile.catalog_revision,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(profile)))
}

#[utoipa::path(
    put,
    path = "/product/provider-profiles/{profile_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("profile_id" = ProductProviderProfileId, Path, description = "Provider profile id")
    ),
    request_body = UpdateProductProviderProfileRequest,
    responses(
        (status = 200, description = "Provider profile updated", body = ProductProviderProfile),
        (status = 400, description = "Invalid profile or secret-shaped field", body = ApiErrorResponse),
        (status = 409, description = "Provider catalog revision conflict", body = ApiErrorResponse),
        (status = 404, description = "Provider profile not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_provider_profile(
    State(state): State<ApiState>,
    Path(profile_id): Path<ProductProviderProfileId>,
    body: Result<Json<UpdateProductProviderProfileRequest>, JsonRejection>,
) -> Result<Json<ProductProviderProfile>, ApiError> {
    let request = product_json(body)?;
    let service = state.provider_catalog_service();
    let profile = tokio::task::spawn_blocking(move || {
        super::provider_catalog::update(&service, &profile_id, request)
    })
    .await
    .map_err(|_| ApiError::internal("provider catalog operation did not complete"))??;
    state
        .product_store()?
        .upsert_provider_catalog_identity(
            &profile.id,
            &profile.label,
            profile.provider_type,
            &profile.catalog_revision,
        )
        .await?;
    Ok(Json(profile))
}

#[utoipa::path(
    delete,
    path = "/product/provider-profiles/{profile_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("profile_id" = ProductProviderProfileId, Path, description = "Provider profile id"),
        DeleteProviderProfileQuery,
    ),
    responses(
        (status = 204, description = "Provider profile deleted"),
        (status = 404, description = "Provider profile not found", body = ApiErrorResponse),
        (status = 409, description = "Provider catalog revision conflict", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_provider_profile(
    State(state): State<ApiState>,
    Path(profile_id): Path<ProductProviderProfileId>,
    Query(query): Query<DeleteProviderProfileQuery>,
) -> Result<StatusCode, ApiError> {
    let service = state.provider_catalog_service();
    tokio::task::spawn_blocking(move || {
        super::provider_catalog::delete(&service, &profile_id, query.expected_revision.as_deref())
    })
    .await
    .map_err(|_| ApiError::internal("provider catalog operation did not complete"))??;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/product/provider-profiles/{profile_id}/models",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("profile_id" = ProductProviderProfileId, Path, description = "Provider profile id")),
    responses(
        (status = 200, description = "Models reported by the configured provider", body = ProductProviderModelsResponse),
        (status = 400, description = "Invalid provider profile or missing key environment variable", body = ApiErrorResponse),
        (status = 404, description = "Provider profile not found", body = ApiErrorResponse),
        (status = 429, description = "Provider model inventory was rate limited", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 502, description = "Provider model inventory failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
        (status = 504, description = "Provider model inventory timed out", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_provider_models(
    State(state): State<ApiState>,
    Path(profile_id): Path<ProductProviderProfileId>,
) -> Result<Json<ProductProviderModelsResponse>, ApiError> {
    let service = state.provider_catalog_service();
    let inventory_profile_id = profile_id.clone();
    let (provider, default_model, provider_type, headers) =
        tokio::task::spawn_blocking(move || {
            let catalog = service
                .load()
                .map_err(super::provider_catalog::catalog_error)?;
            super::provider_catalog::inventory_request(
                &catalog,
                &inventory_profile_id,
                &service.paths().root,
            )
        })
        .await
        .map_err(|_| ApiError::internal("provider catalog operation did not complete"))??;
    let normalized = crate::provider::normalize_provider_profile(&provider)?;
    let key_present = !headers.is_empty();
    let inventory =
        crate::provider::provider_inventory_with_headers(&normalized, headers, key_present, None)
            .await?;
    let supports_reasoning = provider_type == ProductProviderType::OpenaiResponses;
    let supported_reasoning = if supports_reasoning {
        vec![
            ProductReasoningPreference::Low,
            ProductReasoningPreference::Medium,
            ProductReasoningPreference::High,
        ]
    } else {
        Vec::new()
    };
    let reasoning_unavailable_reason = (!supports_reasoning).then(|| {
        "Reasoning controls are only available for OpenAI Responses profiles.".to_string()
    });
    Ok(Json(ProductProviderModelsResponse {
        profile_id,
        default_model,
        models: inventory
            .models
            .into_iter()
            .map(|id| ProductModelDescriptor {
                context_window: rove_product_store::pricing::bundled_context_window(&id)
                    .and_then(|value| u32::try_from(value).ok()),
                id,
                supports_reasoning,
                supported_reasoning: supported_reasoning.clone(),
                reasoning_unavailable_reason: reasoning_unavailable_reason.clone(),
            })
            .collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/product/preferences",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Safe persisted product preferences", body = ProductPreferences),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_preferences(
    State(state): State<ApiState>,
) -> Result<Json<ProductPreferences>, ApiError> {
    let preferences = state.product_store()?.get_preferences().await?;
    Ok(Json(preferences))
}

#[utoipa::path(
    put,
    path = "/product/preferences",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = UpdateProductPreferencesRequest,
    responses(
        (status = 200, description = "Safe product preferences updated", body = ProductPreferences),
        (status = 400, description = "Invalid preference", body = ApiErrorResponse),
        (status = 409, description = "Preference revision conflict", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_preferences(
    State(state): State<ApiState>,
    body: Result<Json<UpdateProductPreferencesRequest>, JsonRejection>,
) -> Result<Json<ProductPreferences>, ApiError> {
    let request = product_json(body)?;
    let preferences = state.product_store()?.update_preferences(request).await?;
    state.notify_product_events();
    Ok(Json(preferences))
}

#[utoipa::path(
    post,
    path = "/product/migrations/m1-browser",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = M1BrowserMigrationRequest,
    responses(
        (status = 200, description = "Migration applied or idempotently replayed", body = M1BrowserMigrationResponse),
        (status = 400, description = "Invalid, unknown, or secret-shaped migration field", body = ApiErrorResponse),
        (status = 409, description = "Idempotency key or active product session conflict", body = ApiErrorResponse),
        (status = 504, description = "Migration exceeded its bounded pre-commit preparation deadline", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn migrate_m1_browser_state(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<M1BrowserMigrationResponse>, ApiError> {
    let store = state.product_store()?;
    let preparation_store = store.clone();
    let preparation_state = state.clone();
    let allow_external_paths = state.inner.config.state.allow_external_paths;
    let supervisors = state.inner.supervisors.clone();

    complete_after_bounded_migration_preparation(
        crate::PRODUCT_MIGRATION_PREPARATION_DEADLINE,
        async move {
            let request =
                product_json(Json::<M1BrowserMigrationRequest>::from_request(request, &()).await)?;
            let preferences_baseline = match preparation_store
                .preflight_m1_browser_migration(&request)
                .await?
            {
                M1BrowserMigrationPreflight::Replay(receipt) => {
                    return Ok(M1MigrationPreparation::Replay(receipt));
                }
                M1BrowserMigrationPreflight::Prepare(baseline) => baseline,
            };
            let config_for_state = preparation_state.inner.config.clone();
            let migration = super::migration::prepare_m1_browser_migration_with_state_resolver(
                request,
                preferences_baseline,
                allow_external_paths,
                move |root| config_for_state.state_dir_for_workspace_discovery(root),
                |workspace| preparation_state.product_state_store_for_workspace(workspace),
            )
            .await?;
            Ok(M1MigrationPreparation::Apply(migration))
        },
        move |prepared| async move {
            match prepared {
                M1MigrationPreparation::Replay(receipt) => Ok(Json(receipt)),
                M1MigrationPreparation::Apply(guarded) => {
                    let handle = supervisors.spawn(async move {
                        let runtime_guards = guarded.runtime_guards;
                        let result = store.apply_m1_browser_migration(guarded.migration).await;
                        drop(runtime_guards);
                        result
                    });
                    let response = handle.await.map_err(|_| {
                        ApiError::from(ProductStoreError::new(
                            ProductErrorCode::ProductStorageFailure,
                            "browser migration commit supervisor did not complete",
                        ))
                    })??;
                    Ok(Json(response))
                }
            }
        },
    )
    .await
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListControlsQuery {
    #[serde(default)]
    pub status: Option<ProductControlStatusFilter>,
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/steers",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path, description = "Product session ULID")),
    request_body = CreateProductControlRequest,
    responses(
        (status = 201, description = "Steer accepted", body = ProductControl),
        (status = 200, description = "Idempotent replay", body = ProductControl),
        (status = 400, description = "Invalid input", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Idempotency or control-state conflict", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn create_product_session_steer(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<CreateProductControlRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductControl>), ApiError> {
    create_control(state, session_id, ProductControlKind::Steer, body).await
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/followups",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path, description = "Product session ULID")),
    request_body = CreateProductControlRequest,
    responses(
        (status = 201, description = "Follow-up queued", body = ProductControl),
        (status = 200, description = "Idempotent replay", body = ProductControl),
        (status = 400, description = "Invalid input", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Idempotency or control-state conflict", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn create_product_session_followup(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<CreateProductControlRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductControl>), ApiError> {
    create_control(state, session_id, ProductControlKind::Followup, body).await
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/messages",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path, description = "Product session ULID")),
    request_body = CreateProductMessageRequest,
    responses(
        (status = 201, description = "Message durably accepted", body = ProductMessage),
        (status = 200, description = "Idempotent replay", body = ProductMessage),
        (status = 400, description = "Invalid input", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Idempotency conflict", body = ApiErrorResponse),
    )
)]
pub(crate) async fn create_product_session_message(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<CreateProductMessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductMessage>), ApiError> {
    let request = product_json(body)?;
    let store = state.product_store()?;
    let live_candidate = live_product_job(&state, &session_id).await;
    let lifecycle = match &live_candidate {
        Some(record) => Some(record.control_lifecycle_lock.lock().await),
        None => None,
    };
    let live_is_active = if let Some(record) = live_candidate.as_ref() {
        let status = record.status.lock().await;
        !crate::is_terminal(&status)
    } else {
        false
    };
    let live = live_candidate.as_ref().filter(|_| live_is_active);
    let service = super::message_adapter::service(store.clone());
    let content = request.content.clone();
    let attachments: Vec<rove_runtime::conversation::MessageAttachmentRef> = request
        .attachments
        .iter()
        .map(
            |attachment| rove_runtime::conversation::MessageAttachmentRef {
                attachment_id: attachment.attachment_id.as_str().to_string(),
                name: attachment.name.clone(),
            },
        )
        .collect();
    let mutation = service
        .send(
            session_id.as_str(),
            rove_runtime::conversation::SendMessageCommand {
                content,
                idempotency_key: request.idempotency_key.clone(),
                session_state: match live {
                    Some(_) => rove_runtime::conversation::SessionDeliveryState::Active,
                    None => rove_runtime::conversation::SessionDeliveryState::Idle,
                },
                target_run_id: live.map(|record| record.run_id),
                attachments,
            },
        )
        .await
        .map_err(super::message_adapter::map_domain_error)?;
    let already_exists = mutation.replayed;
    let message = store
        .get_message(
            &session_id,
            &mutation
                .message
                .id
                .parse()
                .map_err(|_| ApiError::bad_request("invalid message id"))?,
        )
        .await?;
    if !already_exists && message.status == ProductMessageStatus::Queued {
        if let Some(record) = live {
            crate::queue_or_publish_product_control_event(
                record,
                rove_runtime::events::StreamEvent::MessageQueued {
                    id: message.id.to_string(),
                    content: message.content.clone(),
                },
            )
            .await;
        } else {
            try_start_idle_followup(&state, &session_id).await;
        }
    }
    if !already_exists {
        state.notify_product_events();
    }
    drop(lifecycle);
    let mut message = message;
    message.redact_for_response();
    Ok((
        if already_exists {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(message),
    ))
}

/// Stamp the session-visible degradation fact onto every image reference the
/// page carries when the session's selected model cannot accept image input.
///
/// The store projects only durable facts; whether an image reached the model
/// depends on the session's resolved Provider config, which the store does not
/// know. The resolution mirrors the run path: the session's profile becomes
/// the active one, and the wire protocol's declared capability answers. A
/// reference that was degraded carries the reason so a transcript client can
/// say "this image was not sent to the model" without guessing.
async fn stamp_message_degradations(
    state: &ApiState,
    session_id: &ProductSessionId,
    messages: &mut [ProductMessage],
) {
    let images_supported = match state.product_store() {
        Ok(store) => match store.get_session_model_config(session_id).await {
            Ok(model_config) => {
                let mut config = state.inner.config.clone();
                if let Some(profile_id) = model_config.profile_id.as_ref() {
                    let id = profile_id.as_str().to_string();
                    if config.provider.profiles.contains_key(&id) {
                        config.provider.active = Some(id);
                    }
                }
                config.provider.model = model_config.model.clone();
                model_supports_images(&config, &model_config.model)
            }
            Err(_) => true,
        },
        Err(_) => true,
    };
    if images_supported {
        return;
    }
    for message in messages {
        for reference in &mut message.attachments {
            if reference.availability == ProductAttachmentAvailability::Available
                && super::attachments::is_raster_image_type(&reference.content_type)
            {
                reference.degradation =
                    Some("not sent: the selected model does not accept image input".to_string());
            }
        }
    }
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/messages",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path),
        ListProductMessagesQuery
    ),
    responses((status = 200, description = "Unified messages", body = ProductMessagesResponse))
)]
pub(crate) async fn list_product_session_messages(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    Query(query): Query<ListProductMessagesQuery>,
) -> Result<Json<ProductMessagesResponse>, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_PRODUCT_MESSAGE_PAGE_LIMIT);
    if query.after_seq.is_some_and(|sequence| sequence < 0)
        || query.before_seq.is_some_and(|sequence| sequence <= 0)
        || (query.after_seq.is_some() && query.before_seq.is_some())
        || limit == 0
        || limit > MAX_PRODUCT_MESSAGE_PAGE_LIMIT
    {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "message page query is invalid",
        ));
    }
    let page = state
        .product_store()?
        .list_messages(
            &session_id,
            ProductMessagePageQuery {
                after_seq: query.after_seq,
                before_seq: query.before_seq,
                limit,
            },
        )
        .await?;
    // A message body is stored verbatim and redacted when it is answered, like
    // every other text surface.
    let mut messages: Vec<_> = page
        .messages
        .into_iter()
        .map(|mut message| {
            message.redact_for_response();
            message
        })
        .collect();
    stamp_message_degradations(&state, &session_id, &mut messages).await;
    Ok(Json(ProductMessagesResponse {
        messages,
        next_after_seq: page.next_after_seq,
        next_before_seq: page.next_before_seq,
    }))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchProductMessagesQuery {
    /// Substring to find in this session's messages. **Required**: an absent,
    /// blank, or oversized term is a typed 400 rather than an empty page. At
    /// most 128 bytes, and no control characters.
    pub q: String,
    /// Opaque token from a previous response's `next_cursor`. Omit for page one.
    /// A token minted for a different session or term is refused.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size, 1..100. Defaults to 32.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Resolve the query extractor, mapping its rejection into the typed envelope.
///
/// `q` is a required, non-optional field, so a request that omits it, gives
/// `limit=abc`, or repeats `q` never reaches the handler: axum's `Query`
/// extractor refuses it first with a plain-text 400 that carries no `code`.
/// The documented envelope says every malformed search request is a typed
/// `product_invalid_input`, and a client parsing that envelope would throw on
/// the plain-text body. The product MCP routes already translate their
/// extractor rejections the same way.
fn search_query(
    query: Result<Query<SearchProductMessagesQuery>, QueryRejection>,
) -> Result<SearchProductMessagesQuery, ApiError> {
    query.map(|Query(query)| query).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "message search query is invalid",
        )
    })
}

/// Validate and resolve one message-search request.
///
/// Every rejection is deliberate, and each one is a distinct mistake: a missing
/// or blank term is not a search, an oversized term is refused before it
/// reaches the index rather than silently truncated, a broken cursor must not
/// be answered with page one, and a limit of zero or beyond the page cap must
/// not be answered with an empty list that a client would read as "no hits".
fn message_search_query(
    query: SearchProductMessagesQuery,
) -> Result<ProductMessageSearchQuery, ApiError> {
    let invalid = || {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "message search query is invalid",
        )
    };
    let cursor = match query.cursor.as_deref() {
        Some(encoded) => Some(ProductMessageSearchCursor::decode(encoded).map_err(|_| invalid())?),
        None => None,
    };
    let limit = query.limit.unwrap_or(DEFAULT_PRODUCT_MESSAGE_SEARCH_LIMIT);
    if limit == 0 || limit > MAX_PRODUCT_MESSAGE_SEARCH_LIMIT {
        return Err(invalid());
    }
    // A term of only whitespace is treated as a missing term rather than as a
    // search for a space, which would match nearly every message.
    if query.q.trim().is_empty() {
        return Err(invalid());
    }
    if query.q.len() > MAX_PRODUCT_MESSAGE_SEARCH_QUERY_BYTES
        || query.q.chars().any(is_unrepresentable_query_character)
    {
        return Err(invalid());
    }
    Ok(ProductMessageSearchQuery {
        term: query.q,
        cursor,
        limit,
    })
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchProductQuery {
    /// Substring to find. **Required**: an absent, blank, or oversized term is a
    /// typed 400 rather than an empty page. At most 128 bytes, and no control
    /// characters.
    pub q: String,
    /// **Required** corpus and location, as `<kind>:<id>`:
    ///
    /// - `workspace:<workspace_id>` — messages of every session in the
    ///   workspace, oldest hit first per session;
    /// - `session:<session_id>` — messages of one session;
    /// - `trace:<session_id>` — records of that session's `trace.jsonl` runs,
    ///   in run order.
    ///
    /// An unknown kind, an unparsable id, or a workspace/session that is not in
    /// the catalog is a typed failure, never an empty page.
    pub scope: String,
    /// Opaque token from a previous response's `next_cursor`. Omit for page one.
    /// A token minted for a different scope or term is refused.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size, 1..100. Defaults to 32.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Resolve the query extractor, mapping its rejection into the typed envelope.
///
/// `q` and `scope` are required fields, so a request that omits one, gives
/// `limit=abc`, or repeats a parameter never reaches the handler: axum's
/// `Query` extractor refuses it first with a plain-text 400 that carries no
/// `code`. A client parsing the documented envelope would throw on that body.
fn search_product_query(
    query: Result<Query<SearchProductQuery>, QueryRejection>,
) -> Result<SearchProductQuery, ApiError> {
    query.map(|Query(query)| query).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "product search query is invalid",
        )
    })
}

/// Validate and resolve one unified search request.
///
/// Every rejection matches the store's own validation, and both run: the route
/// answers with the typed envelope before any store work, and the store refuses
/// the same inputs because it is also a public trait surface.
fn product_search_query(query: SearchProductQuery) -> Result<ProductSearchQuery, ApiError> {
    let invalid = |message: &str| {
        ApiError::bad_request_with_code(ProductErrorCode::ProductInvalidInput.as_str(), message)
    };
    let cursor = match query.cursor.as_deref() {
        Some(encoded) => Some(
            ProductSearchCursor::decode(encoded)
                .map_err(|_| invalid("search cursor is invalid"))?,
        ),
        None => None,
    };
    let limit = query.limit.unwrap_or(DEFAULT_PRODUCT_SEARCH_LIMIT);
    if limit == 0 || limit > MAX_PRODUCT_SEARCH_LIMIT {
        return Err(invalid("product search page limit is invalid"));
    }
    // A term of only whitespace is treated as a missing term rather than as a
    // search for a space, which would match nearly every record.
    if query.q.trim().is_empty() {
        return Err(invalid("product search query is invalid"));
    }
    if query.q.len() > MAX_PRODUCT_MESSAGE_SEARCH_QUERY_BYTES
        || query.q.chars().any(is_unrepresentable_query_character)
    {
        return Err(invalid("product search query is invalid"));
    }
    Ok(ProductSearchQuery {
        term: query.q,
        cursor,
        limit,
    })
}

#[utoipa::path(
    get,
    path = "/product/search",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(SearchProductQuery),
    responses(
        (status = 200, description = "One page of bounded, redacted excerpts from the messages or the persisted run traces the scope names", body = ProductSearchResponse),
        (status = 400, description = "Missing, oversized, or malformed query, scope, or cursor", body = ApiErrorResponse),
        (status = 404, description = "The workspace or session the scope names is not in the catalog", body = ApiErrorResponse),
        (status = 409, description = "The cursor cannot be served: the run history it was paging through is gone. Restart the search without a cursor", body = ApiErrorResponse),
        (status = 500, description = "A trace could not be read, or one record exceeds the bounded read window", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn search_product(
    State(state): State<ApiState>,
    query: Result<Query<SearchProductQuery>, QueryRejection>,
) -> Result<Json<ProductSearchResponse>, ApiError> {
    let raw = search_product_query(query)?;
    let scope = ProductSearchScope::parse(&raw.scope)?;
    let query = product_search_query(raw)?;
    let digest = scope.cursor_digest(&query.term);
    let page = match scope.message_scope() {
        Some(message_scope) => {
            state
                .product_store()?
                .search_scoped_messages(&message_scope, query)
                .await?
        }
        None => {
            // The trace scope names a session, so the catalog read that proves
            // it exists is also the read that resolves the workspace whose
            // runtime state holds the traces. A session that is not in the
            // catalog fails here with a typed not-found, before any file is
            // opened.
            //
            // A request path must not panic, so the impossible case — a scope
            // with no message corpus and no session — is a typed refusal
            // rather than an `expect`, even though `ProductSearchScope` has no
            // such variant today.
            let Some(session_id) = scope.session_id().cloned() else {
                return Err(ApiError::bad_request_with_code(
                    ProductErrorCode::ProductInvalidInput.as_str(),
                    "product search scope is invalid",
                ));
            };
            let store = state.product_store()?;
            let context = store.get_session_context(&session_id).await?;
            let state_store =
                state.product_state_store_for_product_workspace(&context.workspace)?;
            trace_search::search_session_trace(
                store.as_ref(),
                &state_store,
                &session_id,
                &query,
                &digest,
            )
            .await?
        }
    };
    Ok(Json(ProductSearchResponse {
        scope: scope.canonical(),
        hits: page.hits,
        next_cursor: page.next_cursor.map(|cursor| cursor.encode()),
    }))
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/search",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        SearchProductMessagesQuery
    ),
    responses(
        (status = 200, description = "One page of bounded message excerpts", body = ProductMessagesSearchResponse),
        (status = 400, description = "Missing, oversized, or malformed search query", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn search_product_session_messages(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    query: Result<Query<SearchProductMessagesQuery>, QueryRejection>,
) -> Result<Json<ProductMessagesSearchResponse>, ApiError> {
    let query = message_search_query(search_query(query)?)?;
    let page = state
        .product_store()?
        .search_messages(&session_id, query)
        .await?;
    Ok(Json(ProductMessagesSearchResponse {
        hits: page.hits,
        next_cursor: page.next_cursor.map(|cursor| cursor.encode()),
    }))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/messages/{message_id}/promote",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path), ("message_id" = String, Path)),
    request_body = Option<PromoteProductMessageRequest>,
    responses((status = 200, description = "Intervention requested", body = ProductMessage))
)]
pub(crate) async fn promote_product_session_message(
    State(state): State<ApiState>,
    Path((session_id, message_id)): Path<(ProductSessionId, ProductControlId)>,
    body: Option<Json<PromoteProductMessageRequest>>,
) -> Result<Json<ProductMessage>, ApiError> {
    let delivery = body
        .map(|Json(request)| request.delivery())
        .unwrap_or(ProductMessageDelivery::CurrentRun);
    let live = live_product_job(&state, &session_id).await;
    let Some(record) = live else {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductControlRejected.as_str(),
            "message can only be promoted while its session turn is active",
        ));
    };
    let _lifecycle = record.control_lifecycle_lock.lock().await;
    let is_terminal = {
        let status = record.status.lock().await;
        crate::is_terminal(&status)
    };
    if is_terminal {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductControlRejected.as_str(),
            "message can only be promoted while its session turn is active",
        ));
    }
    let store = state.product_store()?;
    // A repeated `current_run` promotion is a no-op replay; a repeated
    // `successor` promotion still has to move the message back to the head,
    // because it may have been reordered behind another one in the meantime.
    if delivery == ProductMessageDelivery::CurrentRun
        && let Ok(existing) = store.get_message(&session_id, &message_id).await
        && existing.requested_delivery == delivery
    {
        let mut existing = existing;
        existing.redact_for_response();
        return Ok(Json(existing));
    }
    match delivery {
        // `current_run` is the shared runtime message contract's promotion: the
        // message becomes a steer for the live run.
        ProductMessageDelivery::CurrentRun => {
            let service = super::message_adapter::service(store.clone());
            service
                .promote(session_id.as_str(), message_id.as_str())
                .await
                .map_err(super::message_adapter::map_domain_error)?;
            state.notify_product_events();
        }
        // `successor` only moves the message to the head of its queue; the live
        // run must not be interrupted, so nothing is steered here. The terminal
        // boundary drains the queue through the existing claimed-successor path.
        ProductMessageDelivery::Successor => {
            store
                .promote_message(&session_id, &message_id, delivery)
                .await?;
            state.notify_product_events();
            let mut message = store.get_message(&session_id, &message_id).await?;
            message.redact_for_response();
            return Ok(Json(message));
        }
    }
    let message = store.get_message(&session_id, &message_id).await?;
    // Resolved before the handle is taken: the send is synchronous once the
    // bounded channel accepts it, and the runtime must never be handed a path.
    // A message promoted into the live run therefore keeps its attachment
    // blocks, exactly as it would have if it had been launched as a follow-up.
    let attachments = super::attachments::resolve_message_attachments(
        &state.attachment_storage(),
        store.as_ref(),
        &session_id,
        &message.attachments,
    )
    .await;
    // The live run's resolved Provider config decides whether its model
    // accepts image input; the same mapping shapes the text and the blocks.
    let images_supported = record
        .product_model_config
        .as_ref()
        .map(|model| rove_app_bootstrap::model_supports_images(&record.config, &model.model))
        .unwrap_or(false);
    let (mapped, blocks) =
        super::attachments::map_attachments_for_model(attachments, images_supported);
    let handle = record.control.lock().await.clone();
    let accepted = handle.is_some_and(|handle| {
        handle.try_send_steer(
            rove_runtime::engine::SteerMessage::for_message(
                message.id.as_str(),
                message.content.clone(),
            )
            .with_attachments(mapped)
            .with_content_blocks(blocks),
        )
    });
    if !accepted {
        let _ = store
            .transition_control(
                &session_id,
                &message_id,
                ProductControlStatus::Pending,
                ProductControlStatus::Abandoned,
                Some(&record.run_id),
            )
            .await;
        let mut message = store.get_message(&session_id, &message_id).await?;
        message.redact_for_response();
        return Ok(Json(message));
    }
    let mut message = message;
    message.redact_for_response();
    Ok(Json(message))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/messages/reorder",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path)),
    request_body = ReorderProductMessagesRequest,
    responses(
        (status = 200, description = "Reordered queue", body = ProductQueueResponse),
        (status = 400, description = "Invalid reorder list", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Reorder list no longer matches the queue", body = ApiErrorResponse),
    )
)]
pub(crate) async fn reorder_product_session_messages(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    Json(request): Json<ReorderProductMessagesRequest>,
) -> Result<Json<ProductQueueResponse>, ApiError> {
    if request.ordered_ids.len() > MAX_PENDING_MESSAGES_PER_SESSION as usize {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "reorder list is larger than the bounded message queue",
        ));
    }
    // Reordering is a pure queue operation, so it holds the same lifecycle lock
    // a promote does: that serializes it against a concurrent promote/revoke
    // inside this process, and the store's exact-coverage check covers the
    // cross-process case.
    let live = live_product_job(&state, &session_id).await;
    let _lifecycle = match &live {
        Some(record) => Some(record.control_lifecycle_lock.lock().await),
        None => None,
    };
    let store = state.product_store()?;
    let messages = store
        .reorder_messages(&session_id, &request.ordered_ids)
        .await?
        .into_iter()
        .map(|mut message| {
            message.redact_for_response();
            message
        })
        .collect();
    Ok(Json(ProductQueueResponse { messages }))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/messages/{message_id}/revoke",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path), ("message_id" = String, Path)),
    responses((status = 200, description = "Message revoked", body = ProductMessage))
)]
pub(crate) async fn revoke_product_session_message(
    State(state): State<ApiState>,
    Path((session_id, message_id)): Path<(ProductSessionId, ProductControlId)>,
) -> Result<Json<ProductMessage>, ApiError> {
    let live_candidate = live_product_job(&state, &session_id).await;
    let lifecycle = match &live_candidate {
        Some(record) => Some(record.control_lifecycle_lock.lock().await),
        None => None,
    };
    let live_is_active = if let Some(record) = live_candidate.as_ref() {
        let status = record.status.lock().await;
        !crate::is_terminal(&status)
    } else {
        false
    };
    let live = live_candidate.as_ref().filter(|_| live_is_active);
    let store = state.product_store()?;
    let service = super::message_adapter::service(store.clone());
    let _revoked = service
        .revoke(session_id.as_str(), message_id.as_str())
        .await
        .map_err(super::message_adapter::map_domain_error)?;
    let message = store.get_message(&session_id, &message_id).await?;
    state.notify_product_events();
    if let Some(record) = live {
        crate::queue_or_publish_product_control_event(
            record,
            rove_runtime::events::StreamEvent::MessageRevoked {
                id: message.id.to_string(),
            },
        )
        .await;
    }
    drop(lifecycle);
    let mut message = message;
    message.redact_for_response();
    Ok(Json(message))
}

async fn create_control(
    state: ApiState,
    session_id: ProductSessionId,
    kind: ProductControlKind,
    body: Result<Json<CreateProductControlRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductControl>), ApiError> {
    let request = product_json(body)?;
    let store = state.product_store()?;
    // A live run owns the final safe point. Hold its lifecycle lock across
    // persistence and delivery so a just-finished run cannot leave a steer
    // stranded between terminal cleanup and the next turn claim.
    let live = live_product_job(&state, &session_id).await;
    let lifecycle = match &live {
        Some(record) => Some(record.control_lifecycle_lock.lock().await),
        None => None,
    };
    let (mut control, already_exists) = store.create_control(&session_id, kind, request).await?;
    let status = if already_exists {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    if !already_exists {
        state.notify_product_events();
        match kind {
            ProductControlKind::Steer => {
                control =
                    deliver_steer_to_live_job(&state, &session_id, &control, live.as_ref()).await?;
            }
            ProductControlKind::Followup => {
                if let Some(record) = live.as_ref() {
                    crate::queue_or_publish_product_control_event(
                        record,
                        rove_runtime::events::StreamEvent::FollowupQueued {
                            id: control.id.to_string(),
                            content: control.content.clone(),
                        },
                    )
                    .await;
                }
                // Durable queue only; supervisor drains after Final.
                // If the session is already idle, kick the drain immediately so
                // the client does not need a second send().
                try_start_idle_followup(&state, &session_id).await;
            }
        }
    }
    drop(lifecycle);

    control.redact_for_response();
    Ok((status, Json(control)))
}

async fn deliver_steer_to_live_job(
    state: &ApiState,
    session_id: &ProductSessionId,
    control: &ProductControl,
    known_live: Option<&std::sync::Arc<crate::JobRecord>>,
) -> Result<ProductControl, ApiError> {
    let record = match known_live {
        Some(record) => Some(std::sync::Arc::clone(record)),
        None => live_product_job(state, session_id).await,
    };
    let Some(record) = record else {
        // A session marked running can still be between product-turn claim and
        // supervisor registration. The start path will replay this pending
        // row under the lifecycle lock once its runtime handle exists.
        let session = state
            .product_store()?
            .get_session_context(session_id)
            .await?;
        if session.session.status == ProductSessionStatus::Running {
            tracing::debug!(control_id = %control.id, "steer submitted while a live run was attaching");
            return Ok(control.clone());
        }
        // There is no safe point left for an idle or terminal session. Commit
        // a durable outcome now so a repeated idempotency key returns this
        // exact dropped fact and never targets a later run.
        let dropped = state
            .product_store()?
            .transition_control(
                session_id,
                &control.id,
                ProductControlStatus::Pending,
                ProductControlStatus::Dropped,
                None,
            )
            .await?;
        tracing::debug!(
            control_id = %control.id,
            "steer submitted after the product session reached a terminal state"
        );
        return Ok(dropped);
    };
    let handle_guard = record.control.lock().await;
    let Some(handle) = handle_guard.as_ref() else {
        drop(handle_guard);
        // The supervisor installs the control handle under the same lifecycle
        // lock as this route, then replays pending controls. Keep this row
        // pending so the original idempotency key has one durable outcome.
        tracing::debug!(
            control_id = %control.id,
            "steer persisted before the runtime control handle was installed"
        );
        return Ok(control.clone());
    };
    let msg =
        rove_runtime::engine::SteerMessage::with_id(control.id.as_str(), control.content.clone());
    if !handle.try_send_steer(msg) {
        drop(handle_guard);
        // A closed or full bounded channel did not accept this message. Mark
        // only this still-pending row as dropped; accepted/applied rows remain
        // immutable facts. The idempotency replay therefore cannot deliver it
        // later to a different run.
        let dropped = state
            .product_store()?
            .transition_control(
                session_id,
                &control.id,
                ProductControlStatus::Pending,
                ProductControlStatus::Dropped,
                Some(&record.run_id),
            )
            .await?;
        tracing::debug!(
            control_id = %control.id,
            "runtime steer channel did not accept the control"
        );
        return Ok(dropped);
    }
    Ok(control.clone())
}

async fn live_product_job(
    state: &ApiState,
    session_id: &ProductSessionId,
) -> Option<std::sync::Arc<crate::JobRecord>> {
    let candidates: Vec<std::sync::Arc<crate::JobRecord>> = {
        let jobs = state.inner.jobs.read().await;
        jobs.values()
            .filter(|r| r.product_session_id.as_ref() == Some(session_id))
            .cloned()
            .collect()
    };
    for record in candidates {
        let is_terminal = {
            let status = record.status.lock().await;
            crate::is_terminal(&status)
        };
        if !is_terminal {
            return Some(record);
        }
    }
    None
}

/// When a follow-up is enqueued against an idle session, ask the supervisor
/// path to claim+start it. Best-effort: failures leave the control pending.
async fn try_start_idle_followup(state: &ApiState, session_id: &ProductSessionId) {
    if live_product_job(state, session_id).await.is_some() {
        return;
    }
    let Ok(store) = state.product_store() else {
        return;
    };
    let Ok(context) = store.get_session_context(session_id).await else {
        return;
    };
    if context.session.status != ProductSessionStatus::Idle {
        return;
    }
    crate::schedule_followup_drain(state, session_id.clone());
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/controls",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path, description = "Product session ULID"),
        ListControlsQuery,
    ),
    responses(
        (status = 200, description = "Controls for the session", body = ProductControlsResponse),
        (status = 400, description = "Invalid control status filter", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn list_product_session_controls(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    Query(query): Query<ListControlsQuery>,
) -> Result<Json<ProductControlsResponse>, ApiError> {
    let store = state.product_store()?;
    let filter = match query.status {
        None | Some(ProductControlStatusFilter::All) => None,
        Some(ProductControlStatusFilter::Pending) => Some(ProductControlStatus::Pending),
        Some(ProductControlStatusFilter::Accepted) => Some(ProductControlStatus::Accepted),
        Some(ProductControlStatusFilter::Applied) => Some(ProductControlStatus::Applied),
        Some(ProductControlStatusFilter::Dropped) => Some(ProductControlStatus::Dropped),
        Some(ProductControlStatusFilter::Abandoned) => Some(ProductControlStatus::Abandoned),
        Some(ProductControlStatusFilter::Revoked) => Some(ProductControlStatus::Revoked),
    };
    let controls = store
        .list_controls(&session_id, filter)
        .await?
        .into_iter()
        .map(|mut control| {
            control.redact_for_response();
            control
        })
        .collect();
    Ok(Json(ProductControlsResponse { controls }))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/controls/{control_id}/revoke",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path),
        ("control_id" = String, Path),
    ),
    responses(
        (status = 200, description = "Control revoked", body = ProductControl),
        (status = 400, description = "Invalid control identifier", body = ApiErrorResponse),
        (status = 404, description = "Product session or control not found", body = ApiErrorResponse),
        (status = 409, description = "Control already terminal", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn revoke_product_session_control(
    State(state): State<ApiState>,
    Path((session_id, control_id)): Path<(ProductSessionId, ProductControlId)>,
) -> Result<Json<ProductControl>, ApiError> {
    let store = state.product_store()?;
    let current = store.get_control(&session_id, &control_id).await?;
    let from = match (current.kind, current.status) {
        (_, ProductControlStatus::Pending) => ProductControlStatus::Pending,
        (ProductControlKind::Followup, ProductControlStatus::Abandoned) => {
            ProductControlStatus::Abandoned
        }
        _ => {
            return Err(ApiError::conflict_with_code(
                ProductErrorCode::ProductControlRejected.as_str(),
                "only pending controls or abandoned follow-ups can be revoked",
            ));
        }
    };
    let updated = store
        .transition_control(
            &session_id,
            &control_id,
            from,
            ProductControlStatus::Revoked,
            None,
        )
        .await?;
    state.notify_product_events();
    let mut updated = updated;
    updated.redact_for_response();
    Ok(Json(updated))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/controls/{control_id}/confirm",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path),
        ("control_id" = String, Path),
    ),
    responses(
        (status = 200, description = "Abandoned follow-up confirmed for a new server-owned turn", body = ProductControl),
        (status = 400, description = "Invalid control identifier", body = ApiErrorResponse),
        (status = 404, description = "Product session or control not found", body = ApiErrorResponse),
        (status = 409, description = "Control cannot be confirmed in its current state", body = ApiErrorResponse),
        (status = 500, description = "Product store operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn confirm_product_session_followup(
    State(state): State<ApiState>,
    Path((session_id, control_id)): Path<(ProductSessionId, ProductControlId)>,
) -> Result<Json<ProductControl>, ApiError> {
    let store = state.product_store()?;
    let control = store
        .confirm_abandoned_followup(&session_id, &control_id)
        .await?;
    state.notify_product_events();
    try_start_idle_followup(&state, &session_id).await;
    let mut control = control;
    control.redact_for_response();
    Ok(Json(control))
}

/// Query parameters for `GET /product/events`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProductEventsQuery {
    /// Replay only events whose `seq` is greater than this value. When omitted,
    /// a `Last-Event-ID` header is used instead; with neither, the stream
    /// follows from the newest retained event.
    #[serde(default)]
    pub after: Option<i64>,
}

/// Reject an unparsable cursor query with the same typed error the header path
/// and the range checks use, instead of axum's default plain-text rejection.
fn product_events_query(
    query: Result<Query<ProductEventsQuery>, QueryRejection>,
) -> Result<Query<ProductEventsQuery>, ApiError> {
    query.map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "after must be a valid event sequence number",
        )
    })
}

/// The `Last-Event-ID` header value for the product stream.
///
/// The job stream's parser accepts `u64`; product `seq` is an `i64` column, so
/// this mirrors its behavior (absent = no cursor, unparsable = 400) on the
/// product `seq` domain. Range checking stays with the query cursor so both
/// cursor sources answer with the same typed error.
fn parse_last_product_event_id(headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    let Some(raw) = headers.get("last-event-id") else {
        return Ok(None);
    };
    let invalid = || {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "Last-Event-ID must be a valid integer",
        )
    };
    let raw = raw.to_str().map_err(|_| invalid())?;
    let value = raw.parse::<i64>().map_err(|_| invalid())?;
    Ok(Some(value))
}

/// One SSE frame for a product directory event.
///
/// The body reuses the job stream's versioned envelope — protocol version
/// first, then the event's own fields — so a client that already parses
/// `/jobs/{job_id}/events` can parse this stream with the same decoder. `id:`
/// is the event's durable `seq`, which is what makes `Last-Event-ID` resume
/// exact.
fn product_event_frame(event: &ProductEvent) -> Result<Event, serde_json::Error> {
    Ok(Event::default()
        .id(event.seq.to_string())
        .event(event.kind.as_str())
        .data(product_event_data(event)?))
}

/// The frame body: the versioned envelope, redacted.
///
/// The product directory stream is a second SSE surface and needs the same pass
/// the job stream applies. Today the summary builders that fill these events
/// carry no free text, so this is defense in depth rather than a fix for a live
/// leak — but a redaction rule that holds on one SSE surface and rests on a
/// convention on the other is the kind of gap a later event payload closes
/// silently. Kept separate from the frame so the bytes that reach the wire are
/// what a test can assert on.
fn product_event_data(event: &ProductEvent) -> Result<String, serde_json::Error> {
    Ok(
        rove_runtime::secrets::registry().redact_json_text(&serde_json::to_string(
            &rove_protocol::Versioned::now(event),
        )?),
    )
}

/// How long a stream waits before re-reading the log on its own.
///
/// Notifications are best-effort: a mutation performed by another process (or a
/// nudge that arrived before this stream subscribed) only has the durable log
/// to rely on, so the stream must not depend on being woken.
const PRODUCT_EVENT_POLL_INTERVAL: Duration = Duration::from_secs(1);

struct ProductEventStream {
    store: Arc<dyn ProductStore>,
    pending: VecDeque<ProductEvent>,
    /// Last `seq` delivered to the client, or the cursor the stream started
    /// from. It moves to the end of a scanned page once that page holds nothing
    /// left to emit, which includes rows this build cannot decode.
    cursor: i64,
    notify: tokio::sync::broadcast::Receiver<()>,
    poll: tokio::time::Interval,
    shutdown: CancellationToken,
}

/// Replay the retained log, then follow it live until the client disconnects or
/// the server shuts down.
///
/// The durable log — not the notification channel — is the stream's source of
/// truth, so a missed wake-up costs one poll interval and never an event.
fn product_event_stream(
    store: Arc<dyn ProductStore>,
    initial: ProductEventPage,
    after: i64,
    notify: tokio::sync::broadcast::Receiver<()>,
    shutdown: CancellationToken,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let state = ProductEventStream {
        store,
        pending: VecDeque::from(initial.events),
        cursor: after,
        notify,
        poll: tokio::time::interval_at(
            tokio::time::Instant::now() + PRODUCT_EVENT_POLL_INTERVAL,
            PRODUCT_EVENT_POLL_INTERVAL,
        ),
        shutdown,
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                match product_event_frame(&event) {
                    Ok(frame) => {
                        state.cursor = event.seq;
                        return Some((Ok(frame), state));
                    }
                    Err(error) => {
                        // Dropping one frame would silently punch a hole in the
                        // sequence the client resumes from, so end the stream
                        // and let the client reconnect with its cursor.
                        tracing::error!(%error, "product event frame could not be serialized");
                        return None;
                    }
                }
            }
            match state
                .store
                .list_product_events(state.cursor, MAX_PRODUCT_EVENT_PAGE)
                .await
            {
                Ok(page) => {
                    // The cursor only advances on an emitted frame, so a page
                    // this build cannot decode must move it explicitly: skipping
                    // the row is right, waiting forever in front of it is not.
                    // `scanned` covers rows written by a newer build, so a run of
                    // them costs one skipped sequence number and the next
                    // recognized fact is still delivered.
                    let scanned = page.last_scanned_seq.unwrap_or(state.cursor);
                    if !page.events.is_empty() {
                        state.pending = VecDeque::from(page.events);
                        continue;
                    }
                    if scanned > state.cursor {
                        state.cursor = scanned;
                        continue;
                    }
                    tokio::select! {
                        received = state.notify.recv() => {
                            // A lag still means "something was committed": fall
                            // through and re-read. A closed channel means no
                            // notification can ever arrive, so waking on the
                            // poll alone is the only honest behaviour left.
                            if matches!(
                                received,
                                Err(tokio::sync::broadcast::error::RecvError::Closed)
                            ) {
                                return None;
                            }
                        }
                        _ = state.poll.tick() => {}
                        _ = state.shutdown.cancelled() => return None,
                    }
                }
                Err(error) => {
                    // The stream cannot report a store failure in-band without
                    // inventing a frame kind the client would have to special
                    // case. Ending it makes the client reconnect with its
                    // cursor, which either succeeds or surfaces the failure.
                    tracing::warn!(%error, "product event stream ended after a store read failed");
                    return None;
                }
            }
        }
    })
}

#[utoipa::path(
    get,
    path = "/product/events",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ProductEventsQuery),
    responses(
        (status = 200, description = "Server-Sent Events stream of product directory facts. Each frame carries the event `seq` in the SSE `id:` field, the dotted kind in `event:`, and a `data:` body of `{\"v\": PROTOCOL_VERSION, \"type\": kind, \"seq\", \"session_id\", \"workspace_id\", \"summary\", \"created_at\"}`. `summary` is a JSON-encoded string of status/outcome fields; message content, tool arguments, error details, and secrets are never included. Without a cursor the stream follows from the newest retained event.", body = ProductEvent, content_type = "text/event-stream"),
        (status = 400, description = "Unparsable, negative, or otherwise invalid cursor or Last-Event-ID header", body = ApiErrorResponse, content_type = "application/json"),
        (status = 409, description = "The cursor cannot be served by this log: it either predates the retained event window or is ahead of the newest retained fact (a reset or different store). Refetch the catalog and reconnect without a cursor", body = ApiErrorResponse, content_type = "application/json"),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse, content_type = "application/json"),
    )
)]
pub(crate) async fn product_events(
    State(state): State<ApiState>,
    query: Result<Query<ProductEventsQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let Query(query) = product_events_query(query)?;
    let store = state.product_store()?;
    let cursor = match query.after {
        Some(after) => Some(after),
        None => parse_last_product_event_id(&headers)?,
    };
    if cursor.is_some_and(|after| after < 0) {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "event cursor must not be negative",
        ));
    }
    // The log head answers two questions at once: where a cursorless subscriber
    // starts (it is not resuming anything, so it follows from now rather than
    // replaying history it never asked for), and whether an explicit cursor can
    // be served at all.
    let latest = store.latest_product_event_seq().await?;
    if cursor.is_some_and(|after| after > latest) {
        // A cursor past the newest retained fact can never be satisfied, because
        // every later append lands behind it. That happens when the log was
        // reset or replaced under a client that kept its cursor; silently
        // following from there would leave the client waiting forever.
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductEventsExpired.as_str(),
            "event cursor is ahead of the retained product event log; refetch the catalog and \
             reconnect without a cursor",
        ));
    }
    let after = cursor.unwrap_or(latest);

    let initial = store
        .list_product_events(after, MAX_PRODUCT_EVENT_PAGE)
        .await?;
    // Retention trims the oldest rows, so an explicit cursor below the oldest
    // retained `seq` cannot be served: the client would silently miss committed
    // facts. Telling it to resynchronize is the only honest answer. The check
    // uses the oldest row the page *scanned*, not the oldest row it could decode:
    // a row written by a newer build is still a fact this client has to catch up
    // on, and treating it as absent would report a gap that retention never made.
    if cursor.is_some()
        && initial
            .first_scanned_seq
            .is_some_and(|oldest| oldest > after.saturating_add(1))
    {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductEventsExpired.as_str(),
            "event cursor is older than the retained product event window",
        ));
    }

    let stream = product_event_stream(
        store,
        initial,
        after,
        state.subscribe_product_events(),
        state.shutdown_token(),
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[tokio::test]
    async fn preparation_deadline_does_not_cancel_apply() {
        let value = complete_after_bounded_migration_preparation(
            Duration::from_millis(1),
            async { Ok::<_, ApiError>(7) },
            |value| async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok::<_, ApiError>(value)
            },
        )
        .await
        .unwrap();

        assert_eq!(value, 7);
    }

    #[tokio::test]
    async fn preparation_timeout_never_starts_apply() {
        let apply_started = Arc::new(AtomicBool::new(false));
        let apply_observer = apply_started.clone();
        let result: Result<(), ApiError> = complete_after_bounded_migration_preparation(
            Duration::from_millis(1),
            std::future::pending::<Result<(), ApiError>>(),
            move |_| async move {
                apply_observer.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        let error = result.expect_err("preparation must time out");
        assert_eq!(error.status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(error.code, ProductErrorCode::ProductStorageFailure.as_str());
        assert!(!apply_started.load(Ordering::SeqCst));
    }

    /// The directory stream is the second SSE surface. Its summaries carry no
    /// free text today, which is exactly why this is asserted rather than
    /// assumed: the redaction has to hold on the frame body itself, not on a
    /// convention about what the payload happens to contain.
    #[test]
    fn product_event_frames_never_carry_a_known_credential() {
        let canary = "product-frame-credential-canary-9a02c7";
        assert!(rove_runtime::secrets::registry().register_value(canary));
        let event = ProductEvent {
            seq: 7,
            kind: ProductEventKind::SessionUpdated,
            session_id: None,
            workspace_id: None,
            summary: Some(format!("keep-frame-summary carrying {canary}")),
            created_at: "2026-09-27T00:00:00Z".to_string(),
        };
        let data = product_event_data(&event).unwrap();
        assert!(!data.contains(canary), "the product frame leaked: {data}");
        assert!(
            data.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
            "the frame must show the removal: {data}"
        );
        assert!(data.contains("keep-frame-summary"), "{data}");
        // The frame itself still builds from the redacted body.
        assert!(product_event_frame(&event).is_ok());
    }
}
