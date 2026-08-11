//! Session-scoped artifact manifest, content, preview, and download routes.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use utoipa::{IntoParams, ToSchema};

use rove_core::ToolArtifactRef;
use rove_runtime::state::tool_artifacts::{
    ARTIFACTS_DIR as TOOL_ARTIFACTS_DIR, is_valid_artifact_id,
};
use rove_runtime::types::RunId;

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::files::{
    FileDisposition, ProductImageMetadata, guess_mime, is_secret_filename,
    read_bounded_file_content, serve_file,
};
use super::{ProductSessionId, ProductSessionRunBinding};

const MAX_ARTIFACTS_PER_RUN: usize = 512;
const MAX_ARTIFACT_HASH_BYTES: u64 = 512 * 1024 * 1024;
/// Artifact metadata is a small fixed record; anything larger is not trusted to
/// be parsed, so a corrupted or hostile file cannot drive allocation here.
const MAX_TOOL_ARTIFACT_METADATA_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ArtifactsQuery {
    #[serde(default)]
    pub include_system: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductArtifactSourceKind {
    Report,
    TaskState,
    Trace,
    Registered,
    /// A durable Tool Artifact retained by the canonical artifact store.
    ///
    /// Distinct from `Registered` because the payload is content-addressed and
    /// carries store-validated metadata, so its MIME type and name come from
    /// that metadata rather than from a file name on disk.
    ToolArtifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductArtifactAvailability {
    Available,
    Cleaned,
    Invalid,
    TooLarge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductArtifactPreviewKind {
    Text,
    RasterImage,
    DownloadOnly,
    Unavailable,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductArtifactView {
    pub artifact_id: String,
    pub safe_name: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub source_run_id: String,
    pub source_kind: ProductArtifactSourceKind,
    pub availability: ProductArtifactAvailability,
    pub preview_kind: ProductArtifactPreviewKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ProductImageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductArtifactsResponse {
    pub session_id: ProductSessionId,
    pub artifacts: Vec<ProductArtifactView>,
    pub partial_reasons: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductArtifactContentEnvelope {
    pub artifact_id: String,
    pub safe_name: String,
    pub mime: String,
    pub size: u64,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ProductImageMetadata>,
    pub preview_allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
}

struct ResolvedArtifact {
    path: PathBuf,
    safe_name: String,
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/artifacts",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        ArtifactsQuery
    ),
    responses(
        (status = 200, description = "Per-session artifact manifest", body = ProductArtifactsResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store or runtime state operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_session_artifacts(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    Query(query): Query<ArtifactsQuery>,
) -> Result<Json<ProductArtifactsResponse>, ApiError> {
    Ok(Json(
        load_session_artifacts(&state, &session_id, query.include_system.unwrap_or(true)).await?,
    ))
}

pub(crate) async fn load_session_artifacts(
    state: &ApiState,
    session_id: &ProductSessionId,
    include_system: bool,
) -> Result<ProductArtifactsResponse, ApiError> {
    let store = state.product_store()?;
    let context = store.get_session_context(session_id).await?;
    let bindings = store.list_run_bindings(session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;

    let mut artifacts = Vec::new();
    let mut partial_reasons = Vec::new();
    for binding in &bindings {
        let run_dir = state_store.run_store.run_dir(&binding.runtime_run_id);
        if include_system {
            for (name, mime, kind) in system_artifacts() {
                let view = describe_artifact(
                    session_id,
                    binding.runtime_run_id,
                    &run_dir.join(name),
                    name,
                    mime,
                    kind,
                    true,
                    &mut partial_reasons,
                )
                .await;
                artifacts.push(view);
            }
        }
        append_registered_artifacts(
            session_id,
            binding,
            &run_dir,
            &mut artifacts,
            &mut partial_reasons,
        )
        .await?;
        append_tool_artifacts(
            session_id,
            binding,
            &run_dir,
            &mut artifacts,
            &mut partial_reasons,
        )
        .await?;
    }

    Ok(ProductArtifactsResponse {
        session_id: session_id.clone(),
        artifacts,
        partial_reasons,
    })
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/artifacts/{artifact_id}/content",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        ("artifact_id" = String, Path, description = "Opaque artifact id")
    ),
    responses(
        (status = 200, description = "Bounded artifact content", body = ProductArtifactContentEnvelope),
        (status = 400, description = "Invalid artifact id", body = ApiErrorResponse),
        (status = 404, description = "Artifact unavailable or cleaned", body = ApiErrorResponse),
        (status = 500, description = "Runtime artifact operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_artifact_content(
    State(state): State<ApiState>,
    AxumPath((session_id, artifact_id)): AxumPath<(ProductSessionId, String)>,
    headers: HeaderMap,
) -> Result<Json<ProductArtifactContentEnvelope>, ApiError> {
    let resolved = resolve_artifact(&state, &session_id, &artifact_id).await?;
    let range = header_range(&headers)?;
    let content = read_bounded_file_content(&resolved.path, range).await?;
    Ok(Json(ProductArtifactContentEnvelope {
        artifact_id,
        safe_name: resolved.safe_name,
        mime: content.mime,
        size: content.size,
        truncated: content.truncated,
        text: content.text,
        encoding: content.encoding,
        image: content.image,
        preview_allowed: content.preview_allowed,
        validation_error: content.validation_error,
    }))
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/artifacts/{artifact_id}/download",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        ("artifact_id" = String, Path, description = "Opaque artifact id")
    ),
    responses(
        (status = 200, description = "Safe artifact attachment stream"),
        (status = 206, description = "Safe ranged artifact attachment stream"),
        (status = 404, description = "Artifact unavailable or cleaned", body = ApiErrorResponse),
        (status = 500, description = "Runtime artifact operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn download_artifact(
    State(state): State<ApiState>,
    AxumPath((session_id, artifact_id)): AxumPath<(ProductSessionId, String)>,
    headers: HeaderMap,
) -> Result<Response<Body>, ApiError> {
    let resolved = resolve_artifact(&state, &session_id, &artifact_id).await?;
    serve_file(
        &resolved.path,
        &resolved.safe_name,
        FileDisposition::Attachment,
        header_range(&headers)?,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/artifacts/{artifact_id}/preview",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        ("artifact_id" = String, Path, description = "Opaque artifact id")
    ),
    responses(
        (status = 200, description = "Validated raster artifact preview"),
        (status = 400, description = "Invalid or unsafe preview", body = ApiErrorResponse),
        (status = 404, description = "Artifact unavailable or cleaned", body = ApiErrorResponse),
        (status = 500, description = "Runtime artifact operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn preview_artifact(
    State(state): State<ApiState>,
    AxumPath((session_id, artifact_id)): AxumPath<(ProductSessionId, String)>,
) -> Result<Response<Body>, ApiError> {
    let resolved = resolve_artifact(&state, &session_id, &artifact_id).await?;
    serve_file(
        &resolved.path,
        &resolved.safe_name,
        FileDisposition::InlineRasterImage,
        None,
    )
    .await
}

async fn append_registered_artifacts(
    session_id: &ProductSessionId,
    binding: &ProductSessionRunBinding,
    run_dir: &Path,
    artifacts: &mut Vec<ProductArtifactView>,
    partial_reasons: &mut Vec<String>,
) -> Result<(), ApiError> {
    let art_dir = run_dir.join("artifacts");
    if !art_dir.is_dir() {
        return Ok(());
    }
    let canonical_dir = art_dir
        .canonicalize()
        .map_err(|error| ApiError::internal(format!("artifact directory unavailable: {error}")))?;
    let mut rd = tokio::fs::read_dir(&art_dir)
        .await
        .map_err(|error| ApiError::internal(format!("artifact directory unreadable: {error}")))?;
    let mut count = 0usize;
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|error| ApiError::internal(format!("artifact directory read failed: {error}")))?
    {
        if count >= MAX_ARTIFACTS_PER_RUN {
            partial_reasons.push(format!(
                "run {}: artifact manifest capped at {MAX_ARTIFACTS_PER_RUN} entries",
                binding.runtime_run_id
            ));
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_safe_artifact_name(&name) || is_secret_filename(&name) {
            partial_reasons.push(format!(
                "run {}: skipped unsafe artifact name",
                binding.runtime_run_id
            ));
            continue;
        }
        let path = entry.path();
        let Ok(canonical) = path.canonicalize() else {
            partial_reasons.push(format!(
                "run {}: artifact {name} is unavailable",
                binding.runtime_run_id
            ));
            continue;
        };
        if !canonical.starts_with(&canonical_dir) {
            partial_reasons.push(format!(
                "run {}: skipped artifact symlink escape",
                binding.runtime_run_id
            ));
            continue;
        }
        let view = describe_artifact(
            session_id,
            binding.runtime_run_id,
            &canonical,
            &name,
            &guess_mime(&canonical),
            ProductArtifactSourceKind::Registered,
            false,
            partial_reasons,
        )
        .await;
        artifacts.push(view);
        count += 1;
    }
    Ok(())
}

/// Adds this run's durable Tool Artifacts to the manifest.
///
/// The store's metadata is the authority for MIME type and size; nothing is
/// sniffed from the payload and no remote-supplied name reaches the response.
/// An artifact whose payload has been expired is reported as `Cleaned` so the
/// UI can still show that the tool produced it.
async fn append_tool_artifacts(
    session_id: &ProductSessionId,
    binding: &ProductSessionRunBinding,
    run_dir: &Path,
    artifacts: &mut Vec<ProductArtifactView>,
    partial_reasons: &mut Vec<String>,
) -> Result<(), ApiError> {
    let root = run_dir.join(TOOL_ARTIFACTS_DIR);
    if !root.is_dir() {
        return Ok(());
    }
    let mut rd = tokio::fs::read_dir(&root).await.map_err(|error| {
        ApiError::internal(format!("tool artifact directory unreadable: {error}"))
    })?;
    let mut count = 0usize;
    while let Some(entry) = rd.next_entry().await.map_err(|error| {
        ApiError::internal(format!("tool artifact directory read failed: {error}"))
    })? {
        if count >= MAX_ARTIFACTS_PER_RUN {
            partial_reasons.push(format!(
                "run {}: tool artifact manifest capped at {MAX_ARTIFACTS_PER_RUN} entries",
                binding.runtime_run_id
            ));
            break;
        }
        let raw_id = entry.file_name().to_string_lossy().into_owned();
        // The store owns this identifier shape. Anything else in the directory
        // is not a Tool Artifact and must not be served as one.
        if !is_valid_artifact_id(&raw_id) {
            partial_reasons.push(format!(
                "run {}: skipped a tool artifact directory with an invalid id",
                binding.runtime_run_id
            ));
            continue;
        }
        count += 1;
        match read_tool_artifact_metadata(&root, &raw_id).await {
            Ok(metadata) => {
                artifacts.push(
                    describe_tool_artifact(
                        session_id,
                        binding.runtime_run_id,
                        &root,
                        &raw_id,
                        &metadata,
                    )
                    .await,
                );
            }
            Err(reason) => partial_reasons.push(format!(
                "run {}: tool artifact {raw_id} metadata unusable ({reason})",
                binding.runtime_run_id
            )),
        }
    }
    Ok(())
}

/// Reads and parses one artifact's committed metadata.
async fn read_tool_artifact_metadata(root: &Path, raw_id: &str) -> Result<ToolArtifactRef, String> {
    let path = root.join(raw_id).join("metadata.json");
    let raw = tokio::fs::read(&path)
        .await
        .map_err(|error| error.to_string())?;
    if raw.len() > MAX_TOOL_ARTIFACT_METADATA_BYTES {
        return Err("metadata exceeds the supported size".to_string());
    }
    serde_json::from_slice::<ToolArtifactRef>(&raw).map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn describe_artifact(
    session_id: &ProductSessionId,
    run_id: RunId,
    path: &Path,
    name: &str,
    fallback_mime: &str,
    source_kind: ProductArtifactSourceKind,
    expected: bool,
    partial_reasons: &mut Vec<String>,
) -> ProductArtifactView {
    let artifact_id = artifact_id(session_id, run_id, source_kind, name);
    let canonical_root = path.parent().and_then(|parent| parent.canonicalize().ok());
    let canonical_path = path.canonicalize();
    let path = match (canonical_root, canonical_path) {
        (Some(root), Ok(path)) if path.starts_with(&root) => path,
        (_, Err(error)) if error.kind() == std::io::ErrorKind::NotFound && expected => {
            return unavailable_view(
                artifact_id,
                name,
                fallback_mime,
                run_id,
                source_kind,
                ProductArtifactAvailability::Cleaned,
            );
        }
        _ => {
            partial_reasons.push(format!(
                "run {run_id}: artifact {name} escapes its controlled directory"
            ));
            return unavailable_view(
                artifact_id,
                name,
                fallback_mime,
                run_id,
                source_kind,
                ProductArtifactAvailability::Invalid,
            );
        }
    };
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            partial_reasons.push(format!(
                "run {run_id}: artifact {name} is not a regular file"
            ));
            return unavailable_view(
                artifact_id,
                name,
                fallback_mime,
                run_id,
                source_kind,
                ProductArtifactAvailability::Invalid,
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && expected => {
            return unavailable_view(
                artifact_id,
                name,
                fallback_mime,
                run_id,
                source_kind,
                ProductArtifactAvailability::Cleaned,
            );
        }
        Err(error) => {
            partial_reasons.push(format!(
                "run {run_id}: artifact {name} unavailable ({error})"
            ));
            return unavailable_view(
                artifact_id,
                name,
                fallback_mime,
                run_id,
                source_kind,
                ProductArtifactAvailability::Invalid,
            );
        }
    };
    let size = metadata.len();
    let content = read_bounded_file_content(&path, None).await.ok();
    let mime = content
        .as_ref()
        .map(|content| content.mime.clone())
        .unwrap_or_else(|| fallback_mime.to_string());
    let image = content.as_ref().and_then(|content| content.image.clone());
    let validation_error = content
        .as_ref()
        .and_then(|content| content.validation_error.clone());
    let preview_kind = if validation_error.is_some() {
        ProductArtifactPreviewKind::Unavailable
    } else if image.is_some() {
        ProductArtifactPreviewKind::RasterImage
    } else if content
        .as_ref()
        .is_some_and(|content| content.text.is_some())
    {
        ProductArtifactPreviewKind::Text
    } else {
        ProductArtifactPreviewKind::DownloadOnly
    };
    let (availability, sha256) = if size > MAX_ARTIFACT_HASH_BYTES {
        partial_reasons.push(format!(
            "run {run_id}: artifact {name} exceeds the 512 MiB hashing limit"
        ));
        (ProductArtifactAvailability::TooLarge, None)
    } else {
        match sha256_file(&path).await {
            Ok(hash) => (ProductArtifactAvailability::Available, Some(hash)),
            Err(error) => {
                partial_reasons.push(format!(
                    "run {run_id}: artifact {name} hash failed ({error})"
                ));
                (ProductArtifactAvailability::Invalid, None)
            }
        }
    };
    ProductArtifactView {
        artifact_id,
        safe_name: name.to_string(),
        mime,
        size: Some(size),
        sha256,
        source_run_id: run_id.to_string(),
        source_kind,
        availability,
        preview_kind,
        image,
        validation_error,
    }
}

/// Projects one durable Tool Artifact into the product manifest view.
///
/// Size and hash come from the store's metadata rather than from a fresh read:
/// the store hashed the bytes as it streamed them, and re-deriving here would
/// let a payload that changed under us appear self-consistent.
async fn describe_tool_artifact(
    session_id: &ProductSessionId,
    run_id: RunId,
    root: &Path,
    raw_id: &str,
    metadata: &ToolArtifactRef,
) -> ProductArtifactView {
    let public_id = artifact_id(
        session_id,
        run_id,
        ProductArtifactSourceKind::ToolArtifact,
        raw_id,
    );
    let mime = metadata
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let safe_name = tool_artifact_safe_name(raw_id, metadata.mime_type.as_deref());
    let payload = root.join(raw_id).join("payload");
    let available = tokio::fs::metadata(&payload)
        .await
        .is_ok_and(|metadata| metadata.is_file());
    if !available {
        return ProductArtifactView {
            artifact_id: public_id,
            safe_name,
            mime,
            size: Some(metadata.byte_length),
            sha256: Some(metadata.sha256.clone()),
            source_run_id: run_id.to_string(),
            source_kind: ProductArtifactSourceKind::ToolArtifact,
            availability: ProductArtifactAvailability::Cleaned,
            preview_kind: ProductArtifactPreviewKind::Unavailable,
            image: None,
            validation_error: metadata.validation_detail.clone(),
        };
    }
    // Inline preview is offered only for a raster image whose declared type the
    // store validated. Active content is download-only regardless of what the
    // producing server claimed, so a hostile payload cannot execute in the app.
    let preview_kind = match metadata.mime_type.as_deref() {
        Some(mime) if rove_core::mime_type_is_active_content(mime) => {
            ProductArtifactPreviewKind::DownloadOnly
        }
        Some("image/png" | "image/jpeg" | "image/gif" | "image/webp") => {
            ProductArtifactPreviewKind::RasterImage
        }
        Some(mime) if mime.starts_with("text/") => ProductArtifactPreviewKind::Text,
        _ => ProductArtifactPreviewKind::DownloadOnly,
    };
    ProductArtifactView {
        artifact_id: public_id,
        safe_name,
        mime,
        size: Some(metadata.byte_length),
        sha256: Some(metadata.sha256.clone()),
        source_run_id: run_id.to_string(),
        source_kind: ProductArtifactSourceKind::ToolArtifact,
        availability: ProductArtifactAvailability::Available,
        preview_kind,
        image: None,
        validation_error: metadata.validation_detail.clone(),
    }
}

/// Builds a download name from the opaque ID and the validated MIME type.
///
/// A remote server's filename or URI is never used here. The name a browser
/// writes to disk is derived only from values this process controls, so a
/// crafted `original_uri` cannot steer a download path or extension.
fn tool_artifact_safe_name(raw_id: &str, mime: Option<&str>) -> String {
    let extension = match mime {
        Some("text/plain") => "txt",
        Some("text/markdown") => "md",
        Some("application/json") => "json",
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some("audio/wav" | "audio/x-wav") => "wav",
        Some("audio/mpeg") => "mp3",
        Some("application/pdf") => "pdf",
        _ => "bin",
    };
    format!("{raw_id}.{extension}")
}

fn unavailable_view(
    artifact_id: String,
    name: &str,
    mime: &str,
    run_id: RunId,
    source_kind: ProductArtifactSourceKind,
    availability: ProductArtifactAvailability,
) -> ProductArtifactView {
    ProductArtifactView {
        artifact_id,
        safe_name: name.to_string(),
        mime: mime.to_string(),
        size: None,
        sha256: None,
        source_run_id: run_id.to_string(),
        source_kind,
        availability,
        preview_kind: ProductArtifactPreviewKind::Unavailable,
        image: None,
        validation_error: None,
    }
}

async fn resolve_artifact(
    state: &ApiState,
    session_id: &ProductSessionId,
    requested_id: &str,
) -> Result<ResolvedArtifact, ApiError> {
    if requested_id.len() != 64 || !requested_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("artifact id is invalid"));
    }
    let store = state.product_store()?;
    let context = store.get_session_context(session_id).await?;
    let bindings = store.list_run_bindings(session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;
    for binding in bindings {
        let run_dir = state_store.run_store.run_dir(&binding.runtime_run_id);
        for (name, _, source_kind) in system_artifacts() {
            if artifact_id(session_id, binding.runtime_run_id, source_kind, name) == requested_id {
                let path = run_dir.join(name);
                let path = require_bound_artifact_file(&path, &run_dir)?;
                return Ok(ResolvedArtifact {
                    path,
                    safe_name: name.to_string(),
                });
            }
        }
        // Resolved before the registered-artifact scan: that scan skips to the
        // next binding when `artifacts/` is absent, which is exactly the shape
        // of a run that produced only durable Tool Artifacts.
        if let Some(resolved) =
            resolve_tool_artifact(session_id, binding.runtime_run_id, &run_dir, requested_id)
                .await?
        {
            return Ok(resolved);
        }
        let art_dir = run_dir.join("artifacts");
        if !art_dir.is_dir() {
            continue;
        }
        let canonical_dir = art_dir
            .canonicalize()
            .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
        let mut rd = tokio::fs::read_dir(&art_dir)
            .await
            .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
        let mut count = 0usize;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?
        {
            if count >= MAX_ARTIFACTS_PER_RUN {
                break;
            }
            count += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_safe_artifact_name(&name) || is_secret_filename(&name) {
                continue;
            }
            if artifact_id(
                session_id,
                binding.runtime_run_id,
                ProductArtifactSourceKind::Registered,
                &name,
            ) != requested_id
            {
                continue;
            }
            let canonical = entry
                .path()
                .canonicalize()
                .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
            if !canonical.starts_with(&canonical_dir) {
                return Err(ApiError::bad_request("artifact path escapes its run"));
            }
            let canonical = require_bound_artifact_file(&canonical, &canonical_dir)?;
            return Ok(ResolvedArtifact {
                path: canonical,
                safe_name: name,
            });
        }
    }
    Err(ApiError::not_found(
        "artifact does not belong to this session or was cleaned",
    ))
}

/// Resolves a requested public ID to a durable Tool Artifact payload.
///
/// The requested ID is never used to build a path. Each candidate directory is
/// enumerated, its store-owned ID validated, and its public ID recomputed from
/// the session, run, and that validated ID; only an exact match resolves. So a
/// caller cannot reach an artifact belonging to another session, nor traverse
/// out of the run, even with a crafted identifier.
async fn resolve_tool_artifact(
    session_id: &ProductSessionId,
    run_id: RunId,
    run_dir: &Path,
    requested_id: &str,
) -> Result<Option<ResolvedArtifact>, ApiError> {
    let root = run_dir.join(TOOL_ARTIFACTS_DIR);
    if !root.is_dir() {
        return Ok(None);
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
    let mut rd = tokio::fs::read_dir(&root)
        .await
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
    let mut count = 0usize;
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?
    {
        if count >= MAX_ARTIFACTS_PER_RUN {
            break;
        }
        let raw_id = entry.file_name().to_string_lossy().into_owned();
        if !is_valid_artifact_id(&raw_id) {
            continue;
        }
        count += 1;
        if artifact_id(
            session_id,
            run_id,
            ProductArtifactSourceKind::ToolArtifact,
            &raw_id,
        ) != requested_id
        {
            continue;
        }
        let metadata = read_tool_artifact_metadata(&root, &raw_id)
            .await
            .map_err(|_| ApiError::not_found("artifact metadata is unavailable"))?;
        let payload = require_bound_artifact_file(&root.join(&raw_id).join("payload"), &root)?;
        if !payload.starts_with(&canonical_root) {
            return Err(ApiError::bad_request("artifact path escapes its run"));
        }
        return Ok(Some(ResolvedArtifact {
            path: payload,
            safe_name: tool_artifact_safe_name(&raw_id, metadata.mime_type.as_deref()),
        }));
    }
    Ok(None)
}

fn require_bound_artifact_file(path: &Path, bound_root: &Path) -> Result<PathBuf, ApiError> {
    let canonical_root = bound_root
        .canonicalize()
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
    let canonical = path
        .canonicalize()
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
    if !canonical.starts_with(canonical_root) {
        return Err(ApiError::bad_request("artifact path escapes its run"));
    }
    let metadata = std::fs::metadata(&canonical)
        .map_err(|_| ApiError::not_found("artifact was cleaned or is unavailable"))?;
    if !metadata.is_file() {
        return Err(ApiError::bad_request("artifact is not a regular file"));
    }
    Ok(canonical)
}

fn artifact_id(
    session_id: &ProductSessionId,
    run_id: RunId,
    source_kind: ProductArtifactSourceKind,
    name: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"rove-product-artifact-v1\0");
    hasher.update(session_id.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(run_id.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(format!("{source_kind:?}").as_bytes());
    hasher.update(b"\0");
    hasher.update(name.as_bytes());
    hex_digest(hasher.finalize().as_slice())
}

async fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize().as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn header_range(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    headers
        .get(axum::http::header::RANGE)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::bad_request("range header is not valid ASCII"))
        })
        .transpose()
}

fn system_artifacts() -> [(&'static str, &'static str, ProductArtifactSourceKind); 3] {
    [
        (
            "report.json",
            "application/json",
            ProductArtifactSourceKind::Report,
        ),
        (
            "task_state.json",
            "application/json",
            ProductArtifactSourceKind::TaskState,
        ),
        (
            "trace.jsonl",
            "application/x-ndjson",
            ProductArtifactSourceKind::Trace,
        ),
    ]
}

fn is_safe_artifact_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 255
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return false;
    }
    name.chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_shaped_or_secret_artifact_names() {
        assert!(!is_safe_artifact_name("../x"));
        assert!(!is_safe_artifact_name("a/b"));
        assert!(!is_safe_artifact_name("a\\b"));
        assert!(is_safe_artifact_name("report.json"));
        assert!(is_safe_artifact_name("结果-1.json"));
        assert!(is_secret_filename("private.pem"));
    }

    #[test]
    fn artifact_ids_are_opaque_stable_and_session_scoped() {
        let run_id = RunId::new();
        let first_session = ProductSessionId::new();
        let second_session = ProductSessionId::new();
        let first = artifact_id(
            &first_session,
            run_id,
            ProductArtifactSourceKind::Registered,
            "output.txt",
        );
        assert_eq!(first.len(), 64);
        assert_eq!(
            first,
            artifact_id(
                &first_session,
                run_id,
                ProductArtifactSourceKind::Registered,
                "output.txt"
            )
        );
        assert_ne!(
            first,
            artifact_id(
                &second_session,
                run_id,
                ProductArtifactSourceKind::Registered,
                "output.txt"
            )
        );
        assert!(!first.contains("output"));
        assert!(!first.contains(&run_id.to_string()));
    }

    #[tokio::test]
    async fn sha256_is_streamed_and_exact() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("artifact.bin");
        tokio::fs::write(&path, b"abc").await.unwrap();
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A download name must be derivable only from values this process owns.
    #[test]
    fn tool_artifact_download_names_ignore_remote_influence() {
        let id = "art_0123456789abcdef0123456789abcdef";
        assert_eq!(
            tool_artifact_safe_name(id, Some("image/png")),
            format!("{id}.png")
        );
        // An unmodelled or hostile type falls back to an inert extension
        // instead of being trusted to name the file.
        assert_eq!(
            tool_artifact_safe_name(id, Some("text/html")),
            format!("{id}.bin")
        );
        assert_eq!(
            tool_artifact_safe_name(id, Some("../../evil.sh")),
            format!("{id}.bin")
        );
        assert_eq!(tool_artifact_safe_name(id, None), format!("{id}.bin"));
    }

    fn tool_artifact_ref(mime: Option<&str>) -> ToolArtifactRef {
        ToolArtifactRef {
            artifact_id: rove_core::ArtifactId::new(
                "art_0123456789abcdef0123456789abcdef".to_string(),
            ),
            kind: rove_core::ToolArtifactKind::Image,
            mime_type: mime.map(str::to_string),
            byte_length: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string(),
            storage_ref: "tool_artifacts/art_0123456789abcdef0123456789abcdef/payload".to_string(),
            source: rove_core::ToolArtifactSource {
                run_id: "run_x".to_string(),
                call_id: "call_x".to_string(),
                server_config_id: Some("srv".to_string()),
                server_identity_hash: Some("hash".to_string()),
                session_hash: None,
                remote_tool_name: Some("render".to_string()),
                block_ordinal: 0,
                captured_at: "2026-08-09T00:00:00Z".to_string(),
            },
            original_uri: None,
            audience: None,
            priority: None,
            last_modified: None,
            sensitivity: rove_core::Sensitivity::Normal,
            trust: rove_core::ArtifactTrust::Untrusted,
            validation: rove_core::ArtifactValidation::Validated,
            validation_detail: None,
        }
    }

    async fn write_tool_artifact(
        root: &Path,
        raw_id: &str,
        metadata: &ToolArtifactRef,
        payload: Option<&[u8]>,
    ) {
        let dir = root.join(raw_id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("metadata.json"),
            serde_json::to_vec(metadata).unwrap(),
        )
        .await
        .unwrap();
        if let Some(bytes) = payload {
            tokio::fs::write(dir.join("payload"), bytes).await.unwrap();
        }
    }

    /// The manifest reports store metadata, and an expired payload stays
    /// visible as evidence rather than vanishing.
    #[tokio::test]
    async fn tool_artifact_manifest_uses_store_metadata_and_survives_cleanup() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join(TOOL_ARTIFACTS_DIR);
        let raw_id = "art_0123456789abcdef0123456789abcdef";
        let metadata = tool_artifact_ref(Some("image/png"));
        write_tool_artifact(&root, raw_id, &metadata, Some(b"abc")).await;

        let session_id = ProductSessionId::new();
        let run_id = RunId::new();
        let view = describe_tool_artifact(&session_id, run_id, &root, raw_id, &metadata).await;
        assert_eq!(view.source_kind, ProductArtifactSourceKind::ToolArtifact);
        assert_eq!(view.availability, ProductArtifactAvailability::Available);
        assert_eq!(view.preview_kind, ProductArtifactPreviewKind::RasterImage);
        assert_eq!(view.size, Some(3));
        assert_eq!(view.sha256.as_deref(), Some(metadata.sha256.as_str()));
        // The public id is opaque: it does not leak the store id.
        assert_eq!(view.artifact_id.len(), 64);
        assert!(!view.artifact_id.contains(raw_id));

        tokio::fs::remove_file(root.join(raw_id).join("payload"))
            .await
            .unwrap();
        let cleaned = describe_tool_artifact(&session_id, run_id, &root, raw_id, &metadata).await;
        assert_eq!(cleaned.availability, ProductArtifactAvailability::Cleaned);
        assert_eq!(cleaned.size, Some(3), "cleanup keeps the recorded facts");
    }

    /// Active content is never offered for inline preview, whatever the
    /// producing server declared.
    #[tokio::test]
    async fn active_content_tool_artifacts_are_download_only() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join(TOOL_ARTIFACTS_DIR);
        let raw_id = "art_0123456789abcdef0123456789abcdef";
        for mime in ["text/html", "image/svg+xml", "application/pdf"] {
            let metadata = tool_artifact_ref(Some(mime));
            write_tool_artifact(&root, raw_id, &metadata, Some(b"abc")).await;
            let view = describe_tool_artifact(
                &ProductSessionId::new(),
                RunId::new(),
                &root,
                raw_id,
                &metadata,
            )
            .await;
            assert_eq!(
                view.preview_kind,
                ProductArtifactPreviewKind::DownloadOnly,
                "{mime} must not be previewable inline"
            );
        }
    }

    /// A directory whose name is not a valid store id is not a Tool Artifact
    /// and must never be enumerated or served.
    #[tokio::test]
    async fn traversal_and_invalid_ids_are_not_enumerated() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join(TOOL_ARTIFACTS_DIR);
        let metadata = tool_artifact_ref(Some("text/plain"));
        for bad in [
            "not_an_artifact",
            "art_short",
            "art_0123456789ABCDEF0123456789abcdef",
        ] {
            write_tool_artifact(&root, bad, &metadata, Some(b"abc")).await;
        }

        let session_id = ProductSessionId::new();
        let run_id = RunId::new();
        let binding_run_dir = temp.path();
        let mut artifacts = Vec::new();
        let mut reasons = Vec::new();
        let binding = ProductSessionRunBinding {
            product_session_id: session_id.clone(),
            ordinal: 0,
            runtime_session_id: rove_runtime::types::SessionId::new(),
            runtime_job_id: rove_runtime::types::JobId::new(),
            runtime_run_id: run_id,
            resumed_from_run_id: None,
            bound_at: "2026-08-09T00:00:00Z".to_string(),
        };
        append_tool_artifacts(
            &session_id,
            &binding,
            binding_run_dir,
            &mut artifacts,
            &mut reasons,
        )
        .await
        .unwrap();
        assert!(artifacts.is_empty(), "invalid ids must not be enumerated");
        assert_eq!(reasons.len(), 3);

        // And an unknown id resolves to nothing rather than a path guess.
        assert!(
            resolve_tool_artifact(&session_id, run_id, binding_run_dir, &"0".repeat(64))
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Oversized or unparsable metadata is refused rather than parsed.
    #[tokio::test]
    async fn oversized_tool_artifact_metadata_is_refused() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join(TOOL_ARTIFACTS_DIR);
        let raw_id = "art_0123456789abcdef0123456789abcdef";
        let dir = root.join(raw_id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("metadata.json"),
            vec![b'a'; MAX_TOOL_ARTIFACT_METADATA_BYTES + 1],
        )
        .await
        .unwrap();
        let error = read_tool_artifact_metadata(&root, raw_id)
            .await
            .unwrap_err();
        assert!(error.contains("exceeds the supported size"), "{error}");
    }
}
