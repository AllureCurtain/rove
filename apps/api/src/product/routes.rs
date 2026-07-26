//! Coordinator-owned product route surface.
//!
//! Catalog handlers delegate to the API-global product store. Migration stays
//! fail-closed until the coordinator validates browser runtime hints against
//! workspace-owned runtime state.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
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
}

fn foundation_unavailable<T>() -> Result<Json<T>, ApiError> {
    Err(ApiError::not_implemented(
        ProductErrorCode::ProductStoreUnavailable.as_str(),
        "the C0 product store foundation is not wired yet",
    ))
}

fn product_json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    body.map(|Json(value)| value).map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "invalid or unknown field in product request body",
        )
    })
}

#[utoipa::path(
    get,
    path = "/product/workspaces",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Known product workspaces", body = ProductWorkspacesResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 200, description = "Product sessions in one workspace", body = ProductSessionsResponse),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_sessions(
    State(state): State<ApiState>,
    Query(query): Query<ListProductSessionsQuery>,
) -> Result<Json<ProductSessionsResponse>, ApiError> {
    let sessions = state
        .product_store()?
        .list_sessions(&query.workspace_id)
        .await?;
    Ok(Json(ProductSessionsResponse { sessions }))
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "Transcript projector is not wired", body = ApiErrorResponse)
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
    path = "/product/provider-profiles",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Persisted secret-reference-only provider profiles", body = ProductProviderProfilesResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_provider_profiles(
    State(state): State<ApiState>,
) -> Result<Json<ProductProviderProfilesResponse>, ApiError> {
    let provider_profiles = state.product_store()?.list_provider_profiles().await?;
    Ok(Json(ProductProviderProfilesResponse { provider_profiles }))
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_provider_profile(
    State(state): State<ApiState>,
    body: Result<Json<CreateProductProviderProfileRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductProviderProfile>), ApiError> {
    let request = product_json(body)?;
    let profile = state
        .product_store()?
        .create_provider_profile(request)
        .await?;
    Ok((StatusCode::CREATED, Json(profile)))
}

#[utoipa::path(
    put,
    path = "/product/provider-profiles/{profile_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("profile_id" = ProductProviderProfileId, Path, description = "Provider profile id")),
    request_body = UpdateProductProviderProfileRequest,
    responses(
        (status = 200, description = "Provider profile updated", body = ProductProviderProfile),
        (status = 400, description = "Invalid profile or secret-shaped field", body = ApiErrorResponse),
        (status = 404, description = "Provider profile not found", body = ApiErrorResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_provider_profile(
    State(state): State<ApiState>,
    Path(profile_id): Path<ProductProviderProfileId>,
    body: Result<Json<UpdateProductProviderProfileRequest>, JsonRejection>,
) -> Result<Json<ProductProviderProfile>, ApiError> {
    let request = product_json(body)?;
    let profile = state
        .product_store()?
        .update_provider_profile(&profile_id, request)
        .await?;
    Ok(Json(profile))
}

#[utoipa::path(
    delete,
    path = "/product/provider-profiles/{profile_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("profile_id" = ProductProviderProfileId, Path, description = "Provider profile id")),
    responses(
        (status = 204, description = "Provider profile deleted"),
        (status = 404, description = "Provider profile not found", body = ApiErrorResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_provider_profile(
    State(state): State<ApiState>,
    Path(profile_id): Path<ProductProviderProfileId>,
) -> Result<StatusCode, ApiError> {
    state
        .product_store()?
        .delete_provider_profile(&profile_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/product/preferences",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Safe persisted product preferences", body = ProductPreferences),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
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
        (status = 409, description = "Idempotency key was reused with a different sanitized payload", body = ApiErrorResponse),
        (status = 501, description = "ProductStore is not wired", body = ApiErrorResponse)
    )
)]
pub(crate) async fn migrate_m1_browser_state(
    State(_state): State<ApiState>,
    body: Result<Json<M1BrowserMigrationRequest>, JsonRejection>,
) -> Result<Json<M1BrowserMigrationResponse>, ApiError> {
    let _request = product_json(body)?;
    foundation_unavailable()
}
