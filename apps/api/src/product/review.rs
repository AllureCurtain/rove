//! Product-facing hard read-only Review routes.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use rove_runtime::review::{capture_target, resolve_external_state_root};
use rove_runtime::workspace::Workspace;

use super::*;
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/reviews",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    request_body = CreateProductReviewRequest,
    responses(
        (status = 201, description = "Hard read-only Review started", body = ProductReview),
        (status = 200, description = "Idempotent active Review replay", body = ProductReview),
        (status = 400, description = "Invalid Review request", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Review target unavailable or conflicting", body = ApiErrorResponse),
        (status = 500, description = "Review or ProductStore failure", body = ApiErrorResponse),
        (status = 503, description = "Review or ProductStore unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_review(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
    body: Result<Json<CreateProductReviewRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductReview>), ApiError> {
    let request = super::routes::product_json(body)?;
    let store = state.product_store()?;
    let context = store.get_session_context(&session_id).await?;
    if context.workspace.kind != ProductWorkspaceKind::Repo {
        return Err(ProductStoreError::new(
            ProductErrorCode::ReviewTargetUnavailable,
            "Review requires a Git repository workspace",
        )
        .into());
    }
    let workspace = Workspace::open_repo(&context.workspace.canonical_root).map_err(|error| {
        tracing::warn!(workspace_id = %context.workspace.id, "review workspace unavailable: {error}");
        ApiError::from(ProductStoreError::new(
            ProductErrorCode::ReviewTargetUnavailable,
            "Review target repository is unavailable",
        ))
    })?;
    if workspace.root != context.workspace.canonical_root {
        return Err(ProductStoreError::new(
            ProductErrorCode::ReviewTargetUnavailable,
            "Review target no longer matches the catalog workspace",
        )
        .into());
    }
    let spec = request.target.clone();
    let workspace_for_capture = workspace.clone();
    let snapshot = tokio::task::spawn_blocking(move || capture_target(&workspace_for_capture, spec))
        .await
        .map_err(|_| ApiError::internal("Review target capture did not complete"))?
        .map_err(|error| {
            tracing::warn!(workspace_id = %context.workspace.id, "review target capture failed: {error}");
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ReviewTargetUnavailable,
                "Review target could not be captured",
            ))
        })?;
    let max_steps = request.max_steps.unwrap_or(DEFAULT_PRODUCT_MAX_STEPS);
    if max_steps == 0 || max_steps > MAX_PRODUCT_MAX_STEPS {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "Review max_steps is outside the supported range",
        ));
    }
    let review_id = ProductReviewId::new();
    let requested_state_root = std::env::temp_dir()
        .join("rove-review-state")
        .join(review_id.to_string());
    let state_root =
        resolve_external_state_root(&workspace, Some(&requested_state_root)).map_err(|_| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ReviewUnavailable,
                "Review state root is unavailable",
            ))
        })?;
    let record = CreateProductReviewRecord {
        review_id: review_id.clone(),
        product_session_id: session_id.clone(),
        workspace_id: context.workspace.id.clone(),
        target: snapshot.summary(),
        target_spec: request.target,
        state_root: state_root.clone(),
        idempotency_key: request.idempotency_key,
    };
    let (review, already_exists) = store.create_review(record).await?;
    if already_exists {
        return Ok((StatusCode::OK, Json(review)));
    }
    let model_config = store.get_session_model_config(&session_id).await?;
    match crate::start_product_review_runtime(
        state,
        review.clone(),
        context.workspace,
        model_config,
        Arc::new(snapshot),
        state_root,
        max_steps,
    )
    .await
    {
        Ok(review) => Ok((StatusCode::CREATED, Json(review))),
        Err(error) => {
            if let Err(store_error) = store.mark_review_unavailable(&review.id).await {
                tracing::warn!(review_id = %review.id, "failed to mark Review unavailable: {store_error}");
            }
            Err(error)
        }
    }
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/reviews",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 200, description = "Bounded Review history", body = ProductReviewsResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "ProductStore failure", body = ApiErrorResponse),
        (status = 503, description = "ProductStore unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_reviews(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductReviewsResponse>, ApiError> {
    let reviews = state.product_store()?.list_reviews(&session_id).await?;
    Ok(Json(ProductReviewsResponse { reviews }))
}

#[utoipa::path(
    get,
    path = "/product/reviews/{review_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("review_id" = ProductReviewId, Path, description = "Review id")),
    responses(
        (status = 200, description = "Review status and sanitized result", body = ProductReview),
        (status = 404, description = "Review not found", body = ApiErrorResponse),
        (status = 500, description = "ProductStore failure", body = ApiErrorResponse),
        (status = 503, description = "ProductStore unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_review(
    State(state): State<ApiState>,
    Path(review_id): Path<ProductReviewId>,
) -> Result<Json<ProductReview>, ApiError> {
    Ok(Json(
        crate::get_product_review_with_stale_check(&state, &review_id).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/product/reviews/{review_id}/findings",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("review_id" = ProductReviewId, Path, description = "Review id"),
        ("limit" = Option<usize>, Query, description = "Page size, 1..128"),
        ("cursor" = Option<usize>, Query, description = "Stable offset cursor")
    ),
    responses(
        (status = 200, description = "Stable finding page", body = ProductReviewFindingsResponse),
        (status = 404, description = "Review not found", body = ApiErrorResponse),
        (status = 500, description = "ProductStore failure", body = ApiErrorResponse),
        (status = 503, description = "ProductStore unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_review_findings(
    State(state): State<ApiState>,
    Path(review_id): Path<ProductReviewId>,
    Query(query): Query<ProductReviewFindingsQuery>,
) -> Result<Json<ProductReviewFindingsResponse>, ApiError> {
    Ok(Json(
        state
            .product_store()?
            .list_review_findings(&review_id, query)
            .await?,
    ))
}

#[utoipa::path(
    post,
    path = "/product/reviews/{review_id}/cancel",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("review_id" = ProductReviewId, Path, description = "Review id")),
    responses(
        (status = 200, description = "Review after idempotent cancellation", body = ProductReview),
        (status = 404, description = "Review not found", body = ApiErrorResponse),
        (status = 500, description = "ProductStore failure", body = ApiErrorResponse),
        (status = 503, description = "ProductStore unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn cancel_product_review(
    State(state): State<ApiState>,
    Path(review_id): Path<ProductReviewId>,
) -> Result<Json<ProductReview>, ApiError> {
    Ok(Json(
        crate::cancel_product_review_runtime(&state, &review_id).await?,
    ))
}
