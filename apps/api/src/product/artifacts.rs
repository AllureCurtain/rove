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
}
