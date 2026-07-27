use std::io::ErrorKind;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rove_runtime::memory::durable::{MemoryScope, MemoryType};
use rove_runtime::memory::management::{
    ManagedMemoryTopicInfo, delete_memory_topic_for_product_sync, is_valid_memory_topic_slug,
    list_memory_topics_for_product_sync, read_memory_topic_for_product_sync,
};

use super::*;
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

#[utoipa::path(
    get,
    path = "/product/memory/topics",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Bounded durable memory topic catalog", body = ProductMemoryTopicsResponse),
        (status = 409, description = "Memory catalog is unsafe or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory catalog read failed", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_memory_topics(
    State(state): State<ApiState>,
) -> Result<Json<ProductMemoryTopicsResponse>, ApiError> {
    let memory_dir = state.inner.config.memory_paths().durable_dir;
    let topics =
        tokio::task::spawn_blocking(move || list_memory_topics_for_product_sync(&memory_dir))
            .await
            .map_err(|_| product_memory_internal())?
            .map_err(map_memory_io_error)?;
    let topics = topics
        .into_iter()
        .filter(|topic| is_valid_memory_topic_slug(&topic.slug))
        .map(product_memory_topic)
        .collect::<Vec<_>>();
    let total = topics.len();
    Ok(Json(ProductMemoryTopicsResponse { topics, total }))
}

#[utoipa::path(
    get,
    path = "/product/memory/topics/{slug}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("slug" = String, Path, description = "Validated durable memory topic slug")),
    responses(
        (status = 200, description = "Bounded durable memory topic content", body = ProductMemoryTopicContentResponse),
        (status = 400, description = "Invalid memory topic slug", body = ApiErrorResponse),
        (status = 404, description = "Memory topic not found", body = ApiErrorResponse),
        (status = 409, description = "Memory topic is unsafe or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic read failed", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
) -> Result<Json<ProductMemoryTopicContentResponse>, ApiError> {
    validate_product_memory_slug(&slug)?;
    let memory_dir = state.inner.config.memory_paths().durable_dir;
    let requested_slug = slug.clone();
    let result = tokio::task::spawn_blocking(move || {
        let topics = list_memory_topics_for_product_sync(&memory_dir)?;
        let topic = topics
            .into_iter()
            .find(|topic| topic.slug == requested_slug);
        let content = read_memory_topic_for_product_sync(&memory_dir, &requested_slug)?;
        Ok::<_, std::io::Error>((topic, content))
    })
    .await
    .map_err(|_| product_memory_internal())?
    .map_err(map_memory_io_error)?;
    let (Some(topic), Some(content)) = result else {
        return Err(ApiError::not_found_with_code(
            ProductErrorCode::ProductMemoryNotFound.as_str(),
            "product memory topic was not found",
        ));
    };
    Ok(Json(ProductMemoryTopicContentResponse {
        topic: product_memory_topic(topic),
        content: content.content,
        truncated: content.truncated,
    }))
}

#[utoipa::path(
    delete,
    path = "/product/memory/topics/{slug}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("slug" = String, Path, description = "Validated durable memory topic slug")),
    responses(
        (status = 204, description = "Memory topic and any stale index entry are absent"),
        (status = 400, description = "Invalid memory topic slug", body = ApiErrorResponse),
        (status = 409, description = "Memory topic or index is unsafe or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic deletion failed", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
) -> Result<StatusCode, ApiError> {
    validate_product_memory_slug(&slug)?;
    let memory_dir = state.inner.config.memory_paths().durable_dir;
    tokio::task::spawn_blocking(move || delete_memory_topic_for_product_sync(&memory_dir, &slug))
        .await
        .map_err(|_| product_memory_internal())?
        .map_err(map_memory_io_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/product/runtime",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Bounded API version, connection, ProductStore, and resume health", body = ProductRuntimeInfo),
        (status = 500, description = "Product resume health read failed", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_runtime_info(
    State(state): State<ApiState>,
) -> Result<Json<ProductRuntimeInfo>, ApiError> {
    let (product_store, resume_health) = match state.inner.product_store.as_ref() {
        Some(store) => (
            ProductStoreStatus::Ready,
            Some(store.get_resume_health().await?),
        ),
        None => (ProductStoreStatus::Unavailable, None),
    };
    Ok(Json(ProductRuntimeInfo {
        api_version: env!("CARGO_PKG_VERSION").to_string(),
        connection: ProductConnectionStatus::Connected,
        product_store,
        resume_health,
    }))
}

fn validate_product_memory_slug(slug: &str) -> Result<(), ApiError> {
    if is_valid_memory_topic_slug(slug) {
        Ok(())
    } else {
        Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductMemoryInvalidSlug.as_str(),
            "invalid product memory topic slug",
        ))
    }
}

fn map_memory_io_error(error: std::io::Error) -> ApiError {
    match error.kind() {
        ErrorKind::InvalidInput => ApiError::bad_request_with_code(
            ProductErrorCode::ProductMemoryInvalidSlug.as_str(),
            "invalid product memory topic slug",
        ),
        ErrorKind::InvalidData | ErrorKind::PermissionDenied => ApiError::conflict_with_code(
            ProductErrorCode::ProductMemoryConflict.as_str(),
            "product memory topic or index is unsafe or corrupt",
        ),
        _ => product_memory_internal(),
    }
}

fn product_memory_internal() -> ApiError {
    ProductStoreError::new(
        ProductErrorCode::ProductStorageFailure,
        "product memory operation failed",
    )
    .into()
}

fn product_memory_topic(topic: ManagedMemoryTopicInfo) -> ProductMemoryTopic {
    let (title, title_truncated) = bounded_metadata(&topic.title);
    let (description, description_truncated) = bounded_metadata(&topic.description);
    let (created_at, created_at_truncated) = bounded_optional_metadata(topic.created_at);
    let (updated_at, updated_at_truncated) = bounded_optional_metadata(topic.updated_at);
    ProductMemoryTopic {
        slug: topic.slug,
        title,
        memory_type: match topic.memory_type {
            MemoryType::User => ProductMemoryType::User,
            MemoryType::Feedback => ProductMemoryType::Feedback,
            MemoryType::Project => ProductMemoryType::Project,
            MemoryType::Reference => ProductMemoryType::Reference,
        },
        scope: match topic.scope {
            MemoryScope::Global => ProductMemoryScope::Global,
            MemoryScope::Project => ProductMemoryScope::Project,
            MemoryScope::Session => ProductMemoryScope::Session,
        },
        confidence: topic.confidence.clamp(0.0, 1.0),
        created_at,
        updated_at,
        description,
        metadata_truncated: topic.metadata_truncated
            || title_truncated
            || description_truncated
            || created_at_truncated
            || updated_at_truncated,
    }
}

fn bounded_optional_metadata(value: Option<String>) -> (Option<String>, bool) {
    match value {
        Some(value) => {
            let (value, truncated) = bounded_metadata(&value);
            (Some(value), truncated)
        }
        None => (None, false),
    }
}

fn bounded_metadata(value: &str) -> (String, bool) {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized_changed = sanitized != value;
    if sanitized.len() <= MAX_PRODUCT_TEXT_BYTES {
        return (sanitized, sanitized_changed);
    }
    let mut end = 0;
    for (index, character) in sanitized.char_indices() {
        let next = index + character.len_utf8();
        if next > MAX_PRODUCT_TEXT_BYTES {
            break;
        }
        end = next;
    }
    (sanitized[..end].to_string(), true)
}
