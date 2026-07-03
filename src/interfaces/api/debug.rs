use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::docs::DEBUG_TAG;
use super::{ApiError, ApiState};
use crate::memory::durable::{
    MemoryTopicInfo, MemoryType, RecallHit, RecallOptions, list_memory_topics_from_dir_sync,
    read_topic_file_sync, recall_with_scores_from_dir_sync,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryListResponse {
    pub topics: Vec<MemoryTopicResponse>,
    pub total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryTopicResponse {
    pub slug: String,
    pub title: String,
    pub memory_type: String,
    pub scope: String,
    pub source: String,
    pub confidence: f32,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub description: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryTopicContentResponse {
    pub slug: String,
    pub content: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecallTestRequest {
    pub query: String,
    #[serde(default)]
    pub type_filter: Option<String>,
    #[serde(default = "default_recall_limit")]
    pub limit: usize,
}

fn default_recall_limit() -> usize {
    8
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecallTestResponse {
    pub query: String,
    pub hits: Vec<RecallHitResponse>,
    pub total_hits: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecallHitResponse {
    pub slug: String,
    pub title: String,
    pub memory_type: String,
    pub scope: String,
    pub confidence: f32,
    pub score: f64,
    pub snippet: Option<String>,
}

#[utoipa::path(
    get,
    path = "/debug/memory",
    tag = DEBUG_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Durable memory topics", body = MemoryListResponse, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(super) async fn list_memory(
    State(state): State<ApiState>,
) -> Result<Json<MemoryListResponse>, ApiError> {
    let memory_dir = state.inner.config.memory_paths().durable_dir;
    let topics = tokio::task::spawn_blocking(move || list_memory_topics_from_dir_sync(&memory_dir))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;

    let topics: Vec<_> = topics.into_iter().map(memory_topic_response).collect();
    let total = topics.len();
    Ok(Json(MemoryListResponse { topics, total }))
}

#[utoipa::path(
    get,
    path = "/debug/memory/topics/{slug}",
    tag = DEBUG_TAG,
    security(("BearerAuth" = [])),
    params(
        ("slug" = String, Path, description = "Durable memory topic slug")
    ),
    responses(
        (status = 200, description = "Durable memory topic content", body = MemoryTopicContentResponse, content_type = "application/json"),
        (status = 404, description = "Topic not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(super) async fn get_memory_topic(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
) -> Result<Json<MemoryTopicContentResponse>, ApiError> {
    let memory_dir = state.inner.config.memory_paths().durable_dir;
    let requested_slug = slug.clone();
    let content =
        tokio::task::spawn_blocking(move || read_topic_file_sync(&memory_dir, &requested_slug))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?;

    match content {
        Some(content) => Ok(Json(MemoryTopicContentResponse { slug, content })),
        None => Err(ApiError::not_found(format!(
            "memory topic not found: {slug}"
        ))),
    }
}

#[utoipa::path(
    post,
    path = "/debug/memory/recall",
    tag = DEBUG_TAG,
    security(("BearerAuth" = [])),
    request_body = RecallTestRequest,
    responses(
        (status = 200, description = "Scored durable memory recall results", body = RecallTestResponse, content_type = "application/json"),
        (status = 400, description = "Invalid recall request", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(super) async fn test_recall(
    State(state): State<ApiState>,
    Json(req): Json<RecallTestRequest>,
) -> Result<Json<RecallTestResponse>, ApiError> {
    if req.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }

    let type_filter = match req.type_filter.as_deref() {
        Some(raw) => Some(MemoryType::parse(raw).ok_or_else(|| {
            ApiError::bad_request(format!(
                "type_filter must be one of user, feedback, project, reference; got {raw}"
            ))
        })?),
        None => None,
    };

    let memory_dir = state.inner.config.memory_paths().durable_dir;
    let query = req.query.clone();
    let opts = RecallOptions {
        type_filter,
        limit: req.limit.clamp(1, 50),
    };
    let hits = tokio::task::spawn_blocking(move || {
        recall_with_scores_from_dir_sync(&memory_dir, &query, opts)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;

    let hits: Vec<_> = hits.into_iter().map(recall_hit_response).collect();
    let total_hits = hits.len();
    Ok(Json(RecallTestResponse {
        query: req.query,
        hits,
        total_hits,
    }))
}

fn memory_topic_response(topic: MemoryTopicInfo) -> MemoryTopicResponse {
    MemoryTopicResponse {
        slug: topic.slug,
        title: topic.title,
        memory_type: topic.memory_type.as_str().to_string(),
        scope: topic.scope.as_str().to_string(),
        source: topic.source,
        confidence: topic.confidence,
        created_at: topic.created_at,
        updated_at: topic.updated_at,
        description: topic.description,
    }
}

fn recall_hit_response(hit: RecallHit) -> RecallHitResponse {
    RecallHitResponse {
        slug: hit.slug,
        title: hit.title,
        memory_type: hit.memory_type.as_str().to_string(),
        scope: hit.scope.as_str().to_string(),
        confidence: hit.confidence,
        score: hit.score,
        snippet: hit.snippet,
    }
}
