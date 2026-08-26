//! Coordinator-owned product route surface.
//!
//! Catalog handlers delegate to the API-global product store. Migration stays
//! fail-closed until the coordinator validates browser runtime hints against
//! workspace-owned runtime state.

use std::future::Future;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;

use super::*;
use crate::docs;
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

pub(super) fn product_json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    body.map(|Json(value)| value).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "invalid or unknown field in product request body",
        )
    })
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
        (status = 400, description = "Invalid fork request", body = ApiErrorResponse),
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
    let boundary =
        crate::verify_product_fork_boundary(&state, &session_id, request.fork_at_run_id).await?;
    let (session, fork, already_exists) = store.create_fork(request, boundary).await?;
    let status = if already_exists {
        StatusCode::OK
    } else {
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
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/transcript",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 200, description = "Canonical-event transcript projection", body = ProductTranscriptResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 500, description = "Product transcript projection failed", body = ApiErrorResponse),
        (status = 503, description = "Product transcript projector is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_session_transcript(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductTranscriptResponse>, ApiError> {
    let transcript = state
        .product_transcript_reader()?
        .read_transcript(&session_id)
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
                context_window: crate::pricing::bundled_context_window(&id)
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
    drop(lifecycle);
    Ok((
        if already_exists {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(message),
    ))
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
    Ok(Json(ProductMessagesResponse {
        messages: page.messages,
        next_after_seq: page.next_after_seq,
        next_before_seq: page.next_before_seq,
    }))
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/messages/{message_id}/promote",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path), ("message_id" = String, Path)),
    responses((status = 200, description = "Intervention requested", body = ProductMessage))
)]
pub(crate) async fn promote_product_session_message(
    State(state): State<ApiState>,
    Path((session_id, message_id)): Path<(ProductSessionId, ProductControlId)>,
) -> Result<Json<ProductMessage>, ApiError> {
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
    if let Ok(existing) = store.get_message(&session_id, &message_id).await
        && existing.requested_delivery == ProductMessageDelivery::CurrentRun
    {
        return Ok(Json(existing));
    }
    let service = super::message_adapter::service(store.clone());
    let _promoted = service
        .promote(session_id.as_str(), message_id.as_str())
        .await
        .map_err(super::message_adapter::map_domain_error)?;
    let message = store.get_message(&session_id, &message_id).await?;
    let handle = record.control.lock().await.clone();
    let accepted = handle.is_some_and(|handle| {
        handle.try_send_steer(rove_runtime::engine::SteerMessage::for_message(
            message.id.as_str(),
            message.content.clone(),
        ))
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
        return Ok(Json(store.get_message(&session_id, &message_id).await?));
    }
    Ok(Json(message))
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
    let controls = store.list_controls(&session_id, filter).await?;
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
    try_start_idle_followup(&state, &session_id).await;
    Ok(Json(control))
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
}
