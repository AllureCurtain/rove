use std::io::ErrorKind;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rove_runtime::memory::durable::{MemoryScope, MemoryType};
use rove_runtime::memory::management::{
    ManagedMemorySource, ManagedMemoryTopicInfo, ManagedMemoryTopicWrite, MemoryTopicDeleteOutcome,
    create_memory_topic_for_product_sync, delete_memory_topic_for_product_sync,
    is_valid_memory_topic_slug, list_memory_topics_for_product_sync,
    read_memory_topic_for_product_sync, update_memory_topic_for_product_sync,
};
use serde::Deserialize;
use utoipa::IntoParams;

use super::*;
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProductMemoryWorkspaceQuery {
    /// Server-owned product workspace identity. Client paths are never accepted.
    pub workspace_id: ProductWorkspaceId,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListProductMemoryTopicsQuery {
    /// Server-owned product workspace identity. Client paths are never accepted.
    pub workspace_id: ProductWorkspaceId,
    /// Case-insensitive match against topic slug, title, or description.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub memory_type: Option<ProductMemoryType>,
    #[serde(default)]
    pub scope: Option<ProductMemoryScope>,
    #[serde(default)]
    pub source: Option<ProductMemorySource>,
}

#[utoipa::path(
    get,
    path = "/product/memory/topics",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ListProductMemoryTopicsQuery),
    responses(
        (status = 200, description = "Bounded durable memory topic catalog", body = ProductMemoryTopicsResponse),
        (status = 400, description = "Missing or invalid product workspace id", body = ApiErrorResponse),
        (status = 404, description = "Product workspace not found", body = ApiErrorResponse),
        (status = 409, description = "Memory path or catalog is outside the selected workspace, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory catalog read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_memory_topics(
    State(state): State<ApiState>,
    query: Result<Query<ListProductMemoryTopicsQuery>, QueryRejection>,
) -> Result<Json<ProductMemoryTopicsResponse>, ApiError> {
    let Query(query) = query.map_err(|_| invalid_product_memory_query())?;
    let search = product_memory_search_query(query.q)?;
    let memory_dir = product_memory_dir(&state, &query.workspace_id).await?;
    let topics =
        tokio::task::spawn_blocking(move || list_memory_topics_for_product_sync(&memory_dir))
            .await
            .map_err(|_| product_memory_internal())?
            .map_err(map_memory_io_error)?;
    let topics = topics
        .into_iter()
        .filter(|topic| is_valid_memory_topic_slug(&topic.slug))
        .map(product_memory_topic)
        .filter(|topic| {
            query
                .memory_type
                .is_none_or(|memory_type| topic.memory_type == memory_type)
                && query.scope.is_none_or(|scope| topic.scope == scope)
                && query.source.is_none_or(|source| topic.source == source)
                && search.as_ref().is_none_or(|search| {
                    topic.slug.to_lowercase().contains(search)
                        || topic.title.to_lowercase().contains(search)
                        || topic.description.to_lowercase().contains(search)
                })
        })
        .collect::<Vec<_>>();
    let total = topics.len();
    Ok(Json(ProductMemoryTopicsResponse { topics, total }))
}

#[utoipa::path(
    post,
    path = "/product/memory/topics",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ProductMemoryWorkspaceQuery),
    request_body = CreateProductMemoryTopicRequest,
    responses(
        (status = 201, description = "Durable memory topic created", body = ProductMemoryTopicContentResponse),
        (status = 400, description = "Invalid workspace query or memory topic body", body = ApiErrorResponse),
        (status = 404, description = "Product workspace not found", body = ApiErrorResponse),
        (status = 409, description = "Memory topic already exists or memory storage is unsafe or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic creation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_memory_topic(
    State(state): State<ApiState>,
    query: Result<Query<ProductMemoryWorkspaceQuery>, QueryRejection>,
    body: Result<Json<CreateProductMemoryTopicRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductMemoryTopicContentResponse>), ApiError> {
    let Query(query) = product_memory_query(query)?;
    let request = super::routes::product_json(body)?;
    validate_product_memory_slug(&request.slug)?;
    let memory_dir = product_memory_dir(&state, &query.workspace_id).await?;
    let content = request.content.clone();
    let topic = product_memory_write(
        request.slug,
        request.title,
        request.memory_type,
        request.scope,
        request.confidence,
        request.description,
        request.content,
    );
    let topic = tokio::task::spawn_blocking(move || {
        create_memory_topic_for_product_sync(&memory_dir, topic)
    })
    .await
    .map_err(|_| product_memory_internal())?
    .map_err(map_memory_io_error)?;
    Ok((
        StatusCode::CREATED,
        Json(ProductMemoryTopicContentResponse {
            topic: product_memory_topic(topic),
            content,
            truncated: false,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/product/memory/topics/{slug}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMemoryWorkspaceQuery,
        ("slug" = String, Path, description = "Validated durable memory topic slug")
    ),
    responses(
        (status = 200, description = "Bounded durable memory topic content", body = ProductMemoryTopicContentResponse),
        (status = 400, description = "Missing/invalid product workspace id or memory topic slug", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or memory topic not found", body = ApiErrorResponse),
        (status = 409, description = "Memory path or topic is outside the selected workspace, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    query: Result<Query<ProductMemoryWorkspaceQuery>, QueryRejection>,
) -> Result<Json<ProductMemoryTopicContentResponse>, ApiError> {
    validate_product_memory_slug(&slug)?;
    let Query(query) = product_memory_query(query)?;
    let memory_dir = product_memory_dir(&state, &query.workspace_id).await?;
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
    put,
    path = "/product/memory/topics/{slug}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMemoryWorkspaceQuery,
        ("slug" = String, Path, description = "Validated durable memory topic slug")
    ),
    request_body = UpdateProductMemoryTopicRequest,
    responses(
        (status = 200, description = "Durable memory topic updated", body = ProductMemoryTopicContentResponse),
        (status = 400, description = "Invalid workspace query, slug, or memory topic body", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or memory topic not found", body = ApiErrorResponse),
        (status = 409, description = "Memory topic revision conflict or memory storage is unsafe or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic update failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    query: Result<Query<ProductMemoryWorkspaceQuery>, QueryRejection>,
    body: Result<Json<UpdateProductMemoryTopicRequest>, JsonRejection>,
) -> Result<Json<ProductMemoryTopicContentResponse>, ApiError> {
    validate_product_memory_slug(&slug)?;
    let Query(query) = product_memory_query(query)?;
    let request = super::routes::product_json(body)?;
    let memory_dir = product_memory_dir(&state, &query.workspace_id).await?;
    let expected_updated_at = request.expected_updated_at.clone();
    let content = request.content.clone();
    let topic = product_memory_write(
        slug,
        request.title,
        request.memory_type,
        request.scope,
        request.confidence,
        request.description,
        request.content,
    );
    let topic = tokio::task::spawn_blocking(move || {
        update_memory_topic_for_product_sync(&memory_dir, topic, expected_updated_at.as_deref())
    })
    .await
    .map_err(|_| product_memory_internal())?
    .map_err(map_memory_io_error)?;
    Ok(Json(ProductMemoryTopicContentResponse {
        topic: product_memory_topic(topic),
        content,
        truncated: false,
    }))
}

#[utoipa::path(
    delete,
    path = "/product/memory/topics/{slug}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMemoryWorkspaceQuery,
        ("slug" = String, Path, description = "Validated durable memory topic slug")
    ),
    responses(
        (status = 204, description = "Memory topic deleted from the selected product workspace"),
        (status = 400, description = "Missing/invalid product workspace id or memory topic slug", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or memory topic not found", body = ApiErrorResponse),
        (status = 409, description = "Memory path, topic, or index is outside the selected workspace, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "Memory topic deletion failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    query: Result<Query<ProductMemoryWorkspaceQuery>, QueryRejection>,
) -> Result<StatusCode, ApiError> {
    validate_product_memory_slug(&slug)?;
    let Query(query) = product_memory_query(query)?;
    let memory_dir = product_memory_dir(&state, &query.workspace_id).await?;
    let outcome: MemoryTopicDeleteOutcome = tokio::task::spawn_blocking(move || {
        delete_memory_topic_for_product_sync(&memory_dir, &slug)
    })
    .await
    .map_err(|_| product_memory_internal())?
    .map_err(map_memory_io_error)?;
    if !outcome.topic_deleted {
        return Err(product_memory_not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

fn product_memory_query(
    query: Result<Query<ProductMemoryWorkspaceQuery>, QueryRejection>,
) -> Result<Query<ProductMemoryWorkspaceQuery>, ApiError> {
    query.map_err(|_| invalid_product_memory_query())
}

fn invalid_product_memory_query() -> ApiError {
    ApiError::bad_request_with_code(
        ProductErrorCode::ProductInvalidInput.as_str(),
        "a valid product workspace id and bounded memory filters are required",
    )
}

fn product_memory_search_query(query: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(query) = query else {
        return Ok(None);
    };
    if query.len() > MAX_PRODUCT_TEXT_BYTES || query.chars().any(char::is_control) {
        return Err(invalid_product_memory_query());
    }
    let query = query.trim();
    if query.is_empty() {
        return Ok(None);
    }
    Ok(Some(query.to_lowercase()))
}

async fn product_memory_dir(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
) -> Result<std::path::PathBuf, ApiError> {
    let product_workspace = state.product_store()?.get_workspace(workspace_id).await?;
    let workspace = crate::open_product_workspace(&product_workspace)?;
    let (_, config) = crate::rebased_workspace_config(state, workspace)?;
    config
        .workspace_bounded_durable_memory_dir()
        .map_err(|error| {
            tracing::warn!(
                product_workspace_id = %workspace_id,
                "rejected unbounded product memory path: {error}"
            );
            ApiError::conflict_with_code(
                ProductErrorCode::ProductMemoryConflict.as_str(),
                "product memory is not bounded by the selected workspace",
            )
        })
}

fn product_memory_not_found() -> ApiError {
    ApiError::not_found_with_code(
        ProductErrorCode::ProductMemoryNotFound.as_str(),
        "product memory topic was not found in the selected workspace",
    )
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
            ProductErrorCode::ProductInvalidInput.as_str(),
            "invalid product memory topic input",
        ),
        ErrorKind::NotFound => product_memory_not_found(),
        ErrorKind::AlreadyExists | ErrorKind::InvalidData | ErrorKind::PermissionDenied => {
            ApiError::conflict_with_code(
                ProductErrorCode::ProductMemoryConflict.as_str(),
                "product memory topic changed, already exists, or its storage is unsafe",
            )
        }
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
        layer: ProductMemoryLayer::Durable,
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
        source: match topic.source {
            ManagedMemorySource::ProductSettings => ProductMemorySource::ProductSettings,
            ManagedMemorySource::LlmTool => ProductMemorySource::LlmTool,
            ManagedMemorySource::Other => ProductMemorySource::Other,
            ManagedMemorySource::Unknown => ProductMemorySource::Unknown,
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

fn product_memory_write(
    slug: String,
    title: String,
    memory_type: ProductMemoryType,
    scope: ProductMemoryScope,
    confidence: f32,
    description: String,
    content: String,
) -> ManagedMemoryTopicWrite {
    ManagedMemoryTopicWrite {
        slug,
        title,
        memory_type: match memory_type {
            ProductMemoryType::User => MemoryType::User,
            ProductMemoryType::Feedback => MemoryType::Feedback,
            ProductMemoryType::Project => MemoryType::Project,
            ProductMemoryType::Reference => MemoryType::Reference,
        },
        scope: match scope {
            ProductMemoryScope::Global => MemoryScope::Global,
            ProductMemoryScope::Project => MemoryScope::Project,
            ProductMemoryScope::Session => MemoryScope::Session,
        },
        confidence,
        description,
        content,
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
