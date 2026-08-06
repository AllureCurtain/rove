//! Bounded workspace file browsing, text reads, and safe binary delivery.

use std::path::{Component, Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, RANGE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use utoipa::{IntoParams, ToSchema};

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::{ProductWorkspaceId, ProductWorkspaceKind};

const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_DIRECTORY_SCAN: usize = 50_000;
pub(crate) const MAX_TEXT_CONTENT_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_DOWNLOAD_RANGE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const IMAGE_HEADER_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListFilesQuery {
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductFileEntry {
    pub path: String,
    pub kind: ProductFileKind,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductFileKind {
    File,
    Directory,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductFilesResponse {
    pub workspace_id: ProductWorkspaceId,
    pub prefix: String,
    pub entries: Vec<ProductFileEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub truncated: bool,
    #[serde(default)]
    pub scan_limit_reached: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileContentQuery {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct ProductImageMetadata {
    pub width: u32,
    pub height: u32,
    pub format: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductFileContentEnvelope {
    pub path: String,
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

#[derive(Debug)]
pub(crate) struct BoundedFileContent {
    pub mime: String,
    pub size: u64,
    pub truncated: bool,
    pub text: Option<String>,
    pub encoding: Option<String>,
    pub image: Option<ProductImageMetadata>,
    pub preview_allowed: bool,
    pub validation_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FileDisposition {
    Attachment,
    InlineRasterImage,
}

#[utoipa::path(
    get,
    path = "/product/workspaces/{workspace_id}/files",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id"),
        ListFilesQuery
    ),
    responses(
        (status = 200, description = "Bounded directory listing", body = ProductFilesResponse),
        (status = 400, description = "Invalid path or query", body = ApiErrorResponse),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 500, description = "Product store or filesystem operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_workspace_files(
    State(state): State<ApiState>,
    AxumPath(workspace_id): AxumPath<ProductWorkspaceId>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<ProductFilesResponse>, ApiError> {
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let root = workspace_root(&workspace.kind, &workspace.canonical_root)?;
    let prefix = query.prefix.unwrap_or_default();
    let list_dir = join_safe(&root, &prefix)?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let skip = query
        .cursor
        .as_deref()
        .map(|cursor| {
            cursor
                .parse::<usize>()
                .map_err(|_| ApiError::bad_request("invalid cursor"))
        })
        .transpose()?
        .unwrap_or(0);
    if skip > MAX_DIRECTORY_SCAN {
        return Err(ApiError::bad_request("cursor exceeds directory scan limit"));
    }

    if !list_dir.is_dir() {
        return Ok(Json(ProductFilesResponse {
            workspace_id,
            prefix,
            entries: Vec::new(),
            next_cursor: None,
            truncated: false,
            scan_limit_reached: false,
        }));
    }

    let (mut read, scan_limit_reached) =
        collect_entries(&root, &list_dir, &prefix, MAX_DIRECTORY_SCAN)?;
    read.sort_by(|left, right| left.path.cmp(&right.path));
    let total = read.len();
    let entries: Vec<_> = read.into_iter().skip(skip).take(limit).collect();
    let page_end = skip.saturating_add(entries.len());
    let has_more_scanned = page_end < total;
    let next_cursor = has_more_scanned.then(|| page_end.to_string());
    Ok(Json(ProductFilesResponse {
        workspace_id,
        prefix,
        entries,
        next_cursor,
        truncated: has_more_scanned || scan_limit_reached,
        scan_limit_reached,
    }))
}

#[utoipa::path(
    get,
    path = "/product/workspaces/{workspace_id}/files/content",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id"),
        FileContentQuery
    ),
    responses(
        (status = 200, description = "File content metadata or bounded text", body = ProductFileContentEnvelope),
        (status = 400, description = "Invalid path or range", body = ApiErrorResponse),
        (status = 404, description = "Workspace or file not found", body = ApiErrorResponse),
        (status = 500, description = "Product store or filesystem operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_workspace_file_content(
    State(state): State<ApiState>,
    AxumPath(workspace_id): AxumPath<ProductWorkspaceId>,
    Query(query): Query<FileContentQuery>,
    headers: HeaderMap,
) -> Result<Json<ProductFileContentEnvelope>, ApiError> {
    let full = resolve_workspace_file(&state, &workspace_id, &query.path).await?;
    let range = header_range(&headers)?;
    let content = read_bounded_file_content(&full, range).await?;
    Ok(Json(ProductFileContentEnvelope {
        path: query.path,
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
    path = "/product/workspaces/{workspace_id}/files/download",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id"),
        FileContentQuery
    ),
    responses(
        (status = 200, description = "Safe attachment stream"),
        (status = 206, description = "Safe ranged attachment stream"),
        (status = 400, description = "Invalid path, range, or oversized request", body = ApiErrorResponse),
        (status = 404, description = "Workspace or file not found", body = ApiErrorResponse),
        (status = 500, description = "Filesystem operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn download_workspace_file(
    State(state): State<ApiState>,
    AxumPath(workspace_id): AxumPath<ProductWorkspaceId>,
    Query(query): Query<FileContentQuery>,
    headers: HeaderMap,
) -> Result<Response<Body>, ApiError> {
    let full = resolve_workspace_file(&state, &workspace_id, &query.path).await?;
    let safe_name = full
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    serve_file(
        &full,
        safe_name,
        FileDisposition::Attachment,
        header_range(&headers)?,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/product/workspaces/{workspace_id}/files/preview",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("workspace_id" = ProductWorkspaceId, Path, description = "Product workspace id"),
        FileContentQuery
    ),
    responses(
        (status = 200, description = "Validated raster image preview"),
        (status = 400, description = "Invalid or unsafe preview", body = ApiErrorResponse),
        (status = 404, description = "Workspace or file not found", body = ApiErrorResponse),
        (status = 500, description = "Filesystem operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn preview_workspace_file(
    State(state): State<ApiState>,
    AxumPath(workspace_id): AxumPath<ProductWorkspaceId>,
    Query(query): Query<FileContentQuery>,
) -> Result<Response<Body>, ApiError> {
    let full = resolve_workspace_file(&state, &workspace_id, &query.path).await?;
    let safe_name = full
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("preview");
    serve_file(&full, safe_name, FileDisposition::InlineRasterImage, None).await
}

async fn resolve_workspace_file(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
    relative: &str,
) -> Result<PathBuf, ApiError> {
    let workspace = state.product_store()?.get_workspace(workspace_id).await?;
    let root = workspace_root(&workspace.kind, &workspace.canonical_root)?;
    let full = join_safe(&root, relative)?;
    require_regular_file(&full).await?;
    Ok(full)
}

pub(crate) async fn read_bounded_file_content(
    path: &Path,
    range: Option<&str>,
) -> Result<BoundedFileContent, ApiError> {
    let metadata = require_regular_file(path).await?;
    let size = metadata.len();
    let (start, end) = parse_range(range, size, MAX_TEXT_CONTENT_BYTES)?;
    let bytes = read_file_window(path, start, end).await?;
    let sniffed = sniff_mime(&bytes);
    let extension_mime = guess_mime(path);
    let extension_is_raster = is_raster_mime(&extension_mime);
    let sniffed_is_raster = sniffed.as_deref().is_some_and(is_raster_mime);
    let (image, validation_error) = if extension_is_raster && !sniffed_is_raster {
        (
            None,
            Some("file extension and raster image signature do not match".to_string()),
        )
    } else if sniffed_is_raster {
        match validate_raster_image(&bytes, size) {
            Ok(image) => (Some(image), None),
            Err(_) => (
                None,
                Some("raster image failed format, size, or pixel validation".to_string()),
            ),
        }
    } else {
        (None, None)
    };

    let extension_is_text = is_text_mime(&extension_mime);
    let valid_text = std::str::from_utf8(&bytes)
        .ok()
        .filter(|_| !bytes.contains(&0));
    let (mime, text, encoding) = if extension_is_raster {
        (
            sniffed.unwrap_or(extension_mime),
            None,
            Some("binary".to_string()),
        )
    } else if extension_is_text {
        match valid_text {
            Some(text) => (
                extension_mime,
                Some(text.to_string()),
                Some("utf-8".to_string()),
            ),
            None => (
                sniffed.unwrap_or_else(|| "application/octet-stream".to_string()),
                None,
                Some("binary".to_string()),
            ),
        }
    } else if let Some(mime) = sniffed {
        (mime, None, Some("binary".to_string()))
    } else if let Some(text) = valid_text {
        (
            "text/plain".to_string(),
            Some(text.to_string()),
            Some("utf-8".to_string()),
        )
    } else {
        (extension_mime, None, Some("binary".to_string()))
    };
    let preview_allowed = text.is_some() || image.is_some();

    Ok(BoundedFileContent {
        mime,
        size,
        truncated: start > 0 || end < size,
        text,
        encoding,
        image,
        preview_allowed,
        validation_error,
    })
}

pub(crate) async fn serve_file(
    path: &Path,
    safe_name: &str,
    disposition: FileDisposition,
    range: Option<&str>,
) -> Result<Response<Body>, ApiError> {
    let metadata = require_regular_file(path).await?;
    let size = metadata.len();
    let header_end = size.min(IMAGE_HEADER_BYTES);
    let header = read_file_window(path, 0, header_end).await?;
    let sniffed = sniff_mime(&header).unwrap_or_else(|| guess_mime(path));

    if matches!(disposition, FileDisposition::InlineRasterImage) {
        validate_raster_image(&header, size)?;
    }

    let (start, end) = match disposition {
        FileDisposition::InlineRasterImage => (0, size),
        FileDisposition::Attachment => parse_range(range, size, MAX_DOWNLOAD_RANGE_BYTES)?,
    };
    let length = end.saturating_sub(start);
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| map_file_open_error(error, "file unavailable"))?;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|error| ApiError::internal(format!("seek failed: {error}")))?;
    let stream = ReaderStream::new(file.take(length));
    let mut builder = Response::builder().status(if start > 0 || end < size {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    });
    let headers = builder
        .headers_mut()
        .ok_or_else(|| ApiError::internal("response builder unavailable"))?;
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&sniffed)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(safe_name, disposition))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=download")),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    if start > 0 || end < size {
        let value = format!("bytes {start}-{}/{size}", end.saturating_sub(1));
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&value)
                .map_err(|_| ApiError::internal("invalid content range"))?,
        );
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|error| ApiError::internal(format!("response build failed: {error}")))
}

/// Collect one directory's safe entries, stopping after `scan_limit` entries.
///
/// `scan_limit` is a parameter rather than a direct `MAX_DIRECTORY_SCAN` read so
/// the bound is testable without materializing 50,000 files.
fn collect_entries(
    root: &Path,
    list_dir: &Path,
    prefix: &str,
    scan_limit: usize,
) -> Result<(Vec<ProductFileEntry>, bool), ApiError> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(list_dir)
        .map_err(|error| map_file_open_error(error, "directory unavailable"))?;
    let mut scan_limit_reached = false;
    for (scanned, entry_result) in rd.enumerate() {
        if scanned >= scan_limit {
            scan_limit_reached = true;
            break;
        }
        let Ok(entry) = entry_result else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_secret_filename(&name) {
            continue;
        }
        let full = entry.path();
        let Ok(canonical) = full.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(root) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&full) else {
            continue;
        };
        let kind = if metadata.is_dir() {
            ProductFileKind::Directory
        } else if metadata.is_file() {
            ProductFileKind::File
        } else {
            continue;
        };
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{}/{name}", prefix.trim_end_matches('/'))
        };
        out.push(ProductFileEntry {
            path: rel,
            kind,
            size: if kind == ProductFileKind::File {
                metadata.len()
            } else {
                0
            },
            modified: metadata
                .modified()
                .ok()
                .map(|stamp| chrono::DateTime::<chrono::Utc>::from(stamp).to_rfc3339()),
        });
    }
    Ok((out, scan_limit_reached))
}

fn header_range(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    headers
        .get(RANGE)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::bad_request("range header is not valid ASCII"))
        })
        .transpose()
}

fn parse_range(range: Option<&str>, size: u64, max_bytes: u64) -> Result<(u64, u64), ApiError> {
    let Some(range) = range else {
        if size > max_bytes {
            return Ok((0, max_bytes));
        }
        return Ok((0, size));
    };
    let Some(spec) = range.strip_prefix("bytes=") else {
        return Err(ApiError::bad_request("range must start with bytes="));
    };
    if spec.contains(',') {
        return Err(ApiError::bad_request("multiple ranges are not supported"));
    }
    let (start_s, end_s) = spec
        .split_once('-')
        .ok_or_else(|| ApiError::bad_request("malformed range"))?;
    if start_s.is_empty() {
        return Err(ApiError::bad_request("suffix ranges are not supported"));
    }
    let start: u64 = start_s
        .parse()
        .map_err(|_| ApiError::bad_request("invalid range start"))?;
    let parsed_end: u64 = if end_s.is_empty() {
        size.saturating_sub(1)
    } else {
        end_s
            .parse()
            .map_err(|_| ApiError::bad_request("invalid range end"))?
    };
    let end = parsed_end.saturating_add(1).min(size);
    if start >= size || start >= end {
        return Err(ApiError::bad_request("range out of bounds"));
    }
    if end - start > max_bytes {
        return Err(ApiError::bad_request(format!(
            "range exceeds {} byte cap",
            max_bytes
        )));
    }
    Ok((start, end))
}

async fn read_file_window(path: &Path, start: u64, end: u64) -> Result<Vec<u8>, ApiError> {
    let length = end.saturating_sub(start);
    let capacity = usize::try_from(length)
        .map_err(|_| ApiError::bad_request("requested range is too large"))?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| map_file_open_error(error, "file unavailable"))?;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|error| ApiError::internal(format!("seek failed: {error}")))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(length)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| ApiError::internal(format!("read failed: {error}")))?;
    Ok(bytes)
}

async fn require_regular_file(path: &Path) -> Result<std::fs::Metadata, ApiError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| map_file_open_error(error, "file not found"))?;
    if !metadata.is_file() {
        return Err(ApiError::bad_request("path is not a regular file"));
    }
    Ok(metadata)
}

fn map_file_open_error(error: std::io::Error, fallback: &str) -> ApiError {
    match error.kind() {
        std::io::ErrorKind::NotFound => ApiError::not_found(fallback),
        std::io::ErrorKind::PermissionDenied => ApiError::bad_request("file access denied"),
        _ => ApiError::internal(format!("filesystem operation failed: {error}")),
    }
}

fn workspace_root(kind: &ProductWorkspaceKind, canonical_root: &Path) -> Result<PathBuf, ApiError> {
    let _ = kind;
    if !canonical_root.is_absolute() || !canonical_root.exists() {
        return Err(ApiError::not_found("workspace root"));
    }
    canonical_root
        .canonicalize()
        .map_err(|error| ApiError::internal(format!("workspace canonicalize failed: {error}")))
}

fn join_safe(root: &Path, relative: &str) -> Result<PathBuf, ApiError> {
    if relative.is_empty() {
        return Ok(root.to_path_buf());
    }
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Err(ApiError::bad_request("path must be workspace-relative"));
    }
    let mut out = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let name = part.to_string_lossy();
                if is_secret_filename(&name) {
                    return Err(ApiError::bad_request("hidden or secret-shaped path"));
                }
                out.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ApiError::bad_request("path traversal blocked"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ApiError::bad_request("absolute paths are not allowed"));
            }
        }
    }
    if let Ok(canonical) = out.canonicalize() {
        if !canonical.starts_with(root) {
            return Err(ApiError::bad_request("symlink escapes workspace"));
        }
        Ok(canonical)
    } else {
        Ok(out)
    }
}

pub(crate) fn is_secret_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.starts_with("id_rsa")
        || lower.starts_with("id_ed25519")
        || lower == ".npmrc"
        || lower == ".netrc"
        || lower == ".dockercfg"
        || lower == "credentials.json"
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/x-ndjson"
        || mime == "image/svg+xml"
}

fn is_raster_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

pub(crate) fn guess_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" => "text/plain".to_string(),
        "md" => "text/markdown".to_string(),
        "json" => "application/json".to_string(),
        "jsonl" | "ndjson" => "application/x-ndjson".to_string(),
        "xml" => "application/xml".to_string(),
        "rs" | "toml" | "yaml" | "yml" | "js" | "ts" | "tsx" | "jsx" | "css" | "html" | "htm"
        | "py" | "sh" | "bash" | "c" | "cc" | "cpp" | "h" | "hpp" | "go" | "java" | "kt" | "rb"
        | "cs" | "swift" => format!("text/{ext}"),
        "svg" => "image/svg+xml".to_string(),
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "pdf" => "application/pdf".to_string(),
        "zip" => "application/zip".to_string(),
        "wasm" => "application/wasm".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn sniff_mime(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png".to_string())
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg".to_string())
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif".to_string())
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp".to_string())
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf".to_string())
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some("application/zip".to_string())
    } else if bytes.starts_with(b"\0asm") {
        Some("application/wasm".to_string())
    } else {
        None
    }
}

pub(crate) fn validate_raster_image(
    header: &[u8],
    file_size: u64,
) -> Result<ProductImageMetadata, ApiError> {
    if file_size == 0 || file_size > MAX_IMAGE_BYTES {
        return Err(ApiError::bad_request(
            "image exceeds the 16 MiB preview limit",
        ));
    }
    let (format, width, height) = if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        if header.len() < 24 || &header[12..16] != b"IHDR" {
            return Err(ApiError::bad_request("invalid PNG header"));
        }
        (
            "png",
            u32::from_be_bytes(header[16..20].try_into().expect("PNG width slice")),
            u32::from_be_bytes(header[20..24].try_into().expect("PNG height slice")),
        )
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        if header.len() < 10 {
            return Err(ApiError::bad_request("invalid GIF header"));
        }
        (
            "gif",
            u16::from_le_bytes([header[6], header[7]]) as u32,
            u16::from_le_bytes([header[8], header[9]]) as u32,
        )
    } else if header.starts_with(&[0xff, 0xd8, 0xff]) {
        let (width, height) = jpeg_dimensions(header)
            .ok_or_else(|| ApiError::bad_request("invalid or unsupported JPEG header"))?;
        ("jpeg", width, height)
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        let (width, height) = webp_dimensions(header)
            .ok_or_else(|| ApiError::bad_request("invalid or unsupported WebP header"))?;
        ("webp", width, height)
    } else {
        return Err(ApiError::bad_request(
            "only validated PNG, JPEG, GIF, and WebP images may be previewed",
        ));
    };
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(ApiError::bad_request(
            "image dimensions exceed preview limits",
        ));
    }
    Ok(ProductImageMetadata {
        width,
        height,
        format: format.to_string(),
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut index = 2usize;
    while index + 4 <= bytes.len() {
        while index < bytes.len() && bytes[index] != 0xff {
            index += 1;
        }
        while index < bytes.len() && bytes[index] == 0xff {
            index += 1;
        }
        let marker = *bytes.get(index)?;
        index += 1;
        if matches!(marker, 0xd8 | 0xd9) {
            continue;
        }
        if marker == 0xda {
            return None;
        }
        let length = u16::from_be_bytes([*bytes.get(index)?, *bytes.get(index + 1)?]) as usize;
        if length < 2 || index.checked_add(length)? > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if length < 7 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            return Some((width, height));
        }
        index += length;
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let chunk = bytes.get(12..16)?;
    if chunk == b"VP8X" {
        let data = bytes.get(24..30)?;
        let width = 1 + u32::from(data[0]) + (u32::from(data[1]) << 8) + (u32::from(data[2]) << 16);
        let height =
            1 + u32::from(data[3]) + (u32::from(data[4]) << 8) + (u32::from(data[5]) << 16);
        Some((width, height))
    } else if chunk == b"VP8L" {
        let data = bytes.get(21..25)?;
        if bytes.get(20).copied()? != 0x2f {
            return None;
        }
        let bits = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
    } else if chunk == b"VP8 " {
        let data = bytes.get(26..30)?;
        Some((
            u16::from_le_bytes([data[0], data[1]]) as u32 & 0x3fff,
            u16::from_le_bytes([data[2], data[3]]) as u32 & 0x3fff,
        ))
    } else {
        None
    }
}

fn content_disposition(name: &str, disposition: FileDisposition) -> String {
    let kind = match disposition {
        FileDisposition::Attachment => "attachment",
        FileDisposition::InlineRasterImage => "inline",
    };
    let ascii_name: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(160)
        .collect();
    format!(
        "{kind}; filename=\"{}\"",
        if ascii_name.is_empty() {
            "download"
        } else {
            &ascii_name
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_detected() {
        for bad in [
            ".env",
            ".env.local",
            "id_rsa",
            "server.pem",
            "service.key",
            "credentials.json",
        ] {
            assert!(is_secret_filename(bad), "expected secret: {bad}");
        }
        for ok in ["main.rs", "README.md", "data.json", "app.tsx"] {
            assert!(!is_secret_filename(ok), "expected ok: {ok}");
        }
    }

    #[test]
    fn join_safe_blocks_traversal_absolute_and_secret_components() {
        let root = PathBuf::from("/tmp/work");
        assert!(join_safe(&root, "../etc/passwd").is_err());
        assert!(join_safe(&root, "/etc/passwd").is_err());
        assert!(join_safe(&root, "a/b/../../outside").is_err());
        assert!(join_safe(&root, "src/main.rs").is_ok());
        assert!(join_safe(&root, "nested/.env.local").is_err());
    }

    #[test]
    fn range_parsing_caps_size_and_rejects_ambiguous_forms() {
        assert_eq!(parse_range(None, 500, 1_000).unwrap(), (0, 500));
        assert_eq!(parse_range(None, 2_000, 1_000).unwrap(), (0, 1_000));
        assert_eq!(
            parse_range(Some("bytes=0-99"), 1_000, 1_000).unwrap(),
            (0, 100)
        );
        assert!(parse_range(Some("bytes=0-2000"), 5_000, 1_000).is_err());
        assert!(parse_range(Some("bytes=100-999"), 50, 1_000).is_err());
        assert!(parse_range(Some("bytes=-10"), 50, 1_000).is_err());
        assert!(parse_range(Some("bytes=0-1,3-4"), 50, 1_000).is_err());
    }

    #[test]
    fn validates_png_dimensions_and_rejects_pixel_bombs() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        let image = validate_raster_image(&png, png.len() as u64).unwrap();
        assert_eq!((image.width, image.height), (640, 480));

        png[16..20].copy_from_slice(&16_384u32.to_be_bytes());
        png[20..24].copy_from_slice(&16_384u32.to_be_bytes());
        assert!(validate_raster_image(&png, png.len() as u64).is_err());
    }

    #[tokio::test]
    async fn invalid_utf8_is_binary_even_with_text_extension() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("bad.txt");
        tokio::fs::write(&path, [0xff, 0xfe, 0x00, 0x61])
            .await
            .unwrap();
        let content = read_bounded_file_content(&path, None).await.unwrap();
        assert_eq!(content.encoding.as_deref(), Some("binary"));
        assert!(content.text.is_none());
        assert_eq!(content.mime, "application/octet-stream");
    }

    #[tokio::test]
    async fn bounded_read_does_not_load_the_rest_of_a_large_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("large.txt");
        let bytes = vec![b'a'; (MAX_TEXT_CONTENT_BYTES + 64) as usize];
        tokio::fs::write(&path, bytes).await.unwrap();
        let content = read_bounded_file_content(&path, None).await.unwrap();
        assert_eq!(content.text.as_ref().map(String::len), Some(1024 * 1024));
        assert!(content.truncated);
    }

    #[test]
    fn directory_scan_stops_at_the_limit_and_reports_it() {
        let temp = tempfile::TempDir::new().unwrap();
        // The route canonicalizes the workspace root before scanning, so the test
        // must too: containment compares canonical forms.
        let root = &temp.path().canonicalize().unwrap();
        for index in 0..12 {
            std::fs::write(root.join(format!("file{index:03}.txt")), b"x").unwrap();
        }

        // Under the limit: every entry is returned and nothing is flagged.
        let (entries, scan_limit_reached) = collect_entries(root, root, "", 12).unwrap();
        assert_eq!(entries.len(), 12);
        assert!(!scan_limit_reached);

        // At the limit: exactly `scan_limit` entries are collected and the caller
        // is told the scan was cut short.
        let (entries, scan_limit_reached) = collect_entries(root, root, "", 5).unwrap();
        assert_eq!(entries.len(), 5);
        assert!(scan_limit_reached);

        // A zero limit must collect nothing rather than scanning the directory.
        let (entries, scan_limit_reached) = collect_entries(root, root, "", 0).unwrap();
        assert!(entries.is_empty());
        assert!(scan_limit_reached);
    }

    #[test]
    fn directory_scan_limit_does_not_count_skipped_secret_entries_as_results() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = &temp.path().canonicalize().unwrap();
        // Secrets are consumed by the scan but never returned, so a limit that
        // spans them yields fewer results without under-reporting the cut.
        std::fs::write(root.join(".env"), b"SECRET=1").unwrap();
        std::fs::write(root.join("id_rsa"), b"key").unwrap();
        std::fs::write(root.join("keep.txt"), b"x").unwrap();

        let (entries, scan_limit_reached) = collect_entries(root, root, "", 3).unwrap();
        assert!(!scan_limit_reached);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "keep.txt");
    }
}
