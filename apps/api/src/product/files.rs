//! Bounded workspace file browsing, text reads, and safe binary delivery.

pub(crate) use rove_product_store::text::truncate_utf8_preserving_markers;
use std::path::{Path, PathBuf};

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

use super::trust::{ensure_workspace_read_allowed, product_workspace_kind};
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::{ProductWorkspaceId, ProductWorkspaceKind};
use rove_product_store::ProductErrorCode;

const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_DIRECTORY_SCAN: usize = 50_000;
pub(crate) const MAX_TEXT_CONTENT_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_DOWNLOAD_RANGE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const IMAGE_HEADER_BYTES: u64 = 1024 * 1024;

/// Longest run-state artifact this route will read, rewrite, and then range.
///
/// The rewrite happens before the range is computed, so the whole file is held
/// in memory and the length headers can describe the rewritten bytes. Past this
/// cap the route refuses with a typed error; it never falls back to streaming
/// the file unredacted, because that fallback is the disclosure this path
/// exists to prevent. The order of magnitude matches the existing download
/// range cap.
pub(crate) const MAX_REDACTED_RUN_STATE_BYTES: u64 = 64 * 1024 * 1024;

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
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
        (status = 409, description = "Project trust was revoked for this workspace", body = ApiErrorResponse)
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
    // Bounded reads honour the project-trust boundary: a revoked root is denied.
    ensure_workspace_read_allowed(&state, &root, product_workspace_kind(&workspace.kind)).await?;
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
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
        (status = 409, description = "Project trust was revoked for this workspace", body = ApiErrorResponse)
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
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
        (status = 409, description = "Project trust was revoked for this workspace", body = ApiErrorResponse)
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
        None,
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
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
        (status = 409, description = "Project trust was revoked for this workspace", body = ApiErrorResponse)
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
    serve_file(
        &full,
        safe_name,
        None,
        FileDisposition::InlineRasterImage,
        None,
    )
    .await
}

async fn resolve_workspace_file(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
    relative: &str,
) -> Result<PathBuf, ApiError> {
    let workspace = state.product_store()?.get_workspace(workspace_id).await?;
    let root = workspace_root(&workspace.kind, &workspace.canonical_root)?;
    // Content, download and preview all resolve through this helper, so one
    // check covers all three; listing has its own copy of the same rule.
    ensure_workspace_read_allowed(state, &root, product_workspace_kind(&workspace.kind)).await?;
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
    // The JSON preview surface for content the requesting principal can also read
    // for themselves: the workspace file route, the artifact *manifest* view (which
    // keeps only the metadata and drops the text), and registered/tool artifacts.
    // A run-state artifact does **not** come through here — `get_artifact_content`
    // sends those to `read_run_state_file_content`, which redacts the whole file
    // before applying the caller's window; windowing first is safe only for bytes
    // the caller could have read from disk anyway.
    //
    // The budget is applied *after* the rewrite and to what is actually emitted.
    // Redaction can grow the text — an 8-byte value becomes a 23-byte marker —
    // so measuring first would let the envelope exceed its documented bound by
    // nearly three times.
    let mut redaction_truncated = false;
    let text = text.map(|text| {
        let redacted = rove_runtime::secrets::registry().redact_text(&text);
        let (kept, cut) = truncate_utf8_preserving_markers(
            &redacted,
            usize::try_from(MAX_TEXT_CONTENT_BYTES).unwrap_or(usize::MAX),
        );
        redaction_truncated = cut;
        kept.to_string()
    });

    Ok(BoundedFileContent {
        mime,
        size,
        truncated: start > 0 || end < size || redaction_truncated,
        text,
        encoding,
        image,
        preview_allowed,
        validation_error,
    })
}

/// The content envelope for a runtime run-state artifact.
///
/// **Redact first, window second.** `report.json`, `task_state.json`, and
/// `trace.jsonl` are the artifacts the byte transport is not allowed to serve
/// raw, and `task_state.json` is raw on disk by necessity, so the window may
/// never be taken before the rewrite: `bytes=N-N` would otherwise return the raw
/// bytes at `N`, a caller walking `N` would reconstruct the file, and a
/// credential split across two requests would defeat the value pass. The whole
/// file is read under the cap, redacted, and only then windowed.
///
/// `size` is therefore the length of the **redacted** text — the byte string the
/// range indexes and the one a caller receives in windows. It can differ from
/// the artifact's on-disk size, which the manifest reports; the download route is
/// in the same representation (`Content-Range: bytes 0-N/<redacted length>`), so
/// both serving paths agree. `truncated` says the window is not that whole text.
/// Workspace file content keeps the other contract
/// ([`read_bounded_file_content`]): there `size` describes the file, because the
/// requesting principal can read those bytes from disk anyway.
///
/// The three artifacts are JSON or NDJSON the runtime wrote, so this envelope is
/// always UTF-8 text: there is nothing to sniff and no image to validate.
pub(crate) async fn read_run_state_file_content(
    path: &Path,
    range: Option<&str>,
) -> Result<BoundedFileContent, ApiError> {
    read_run_state_file_content_with_cap(path, range, MAX_REDACTED_RUN_STATE_BYTES).await
}

/// `max_bytes` is a parameter so the cap is testable without writing a 64 MiB
/// artifact.
async fn read_run_state_file_content_with_cap(
    path: &Path,
    range: Option<&str>,
    max_bytes: u64,
) -> Result<BoundedFileContent, ApiError> {
    let redacted = redacted_run_state_text(path, max_bytes).await?;
    let total = redacted.len() as u64;
    let (start, end) = parse_range(range, total, MAX_TEXT_CONTENT_BYTES)?;
    let (start, end) = clamp_range_to_char_boundaries(&redacted, start, end)?;
    Ok(BoundedFileContent {
        mime: guess_mime(path),
        size: total,
        truncated: start > 0 || end < total as usize,
        text: Some(redacted[start..end].to_string()),
        encoding: Some("utf-8".to_string()),
        image: None,
        preview_allowed: true,
        validation_error: None,
    })
}

/// Serve one file over HTTP with the fixed header set this API promises:
/// `nosniff`, `private, no-store`, a sandbox CSP, and `Accept-Ranges: bytes`.
///
/// `content_type` overrides the type sniffed from the file. It is `Some` only
/// where a durable row already records a *locally verified* type — the
/// attachment store — so a payload whose bytes have no recognizable signature
/// (a stored `txt`) is still served as what validation established it to be,
/// and never as whatever the client claimed.
///
/// **Which callers this covers, and which it does not.** This is the byte
/// transport for content the requesting principal can obtain for themselves:
/// workspace files (`files/download`, `files/preview`, resolved through
/// `resolve_workspace_file`), registered/tool artifacts, which were produced by
/// a tool run in that same workspace, and the attachment store's own rows, whose
/// bytes that principal uploaded. For those, a text rewrite would corrupt a
/// binary download and would break the `Content-Length`/`Content-Range`
/// contract the range requests depend on, while adding no exposure the workspace
/// does not already have.
///
/// It is **not** the serving path for the three runtime run-state artifacts
/// (`report.json`, `task_state.json`, `trace.jsonl`). Those are runtime
/// internals that no workspace read can reach, and `task_state.json` is written
/// raw to disk because resume needs the original values, so their download goes
/// through `serve_run_state_file` instead. A caller that wants redacted bytes
/// for a *text* surface still has the preview envelope
/// (`read_bounded_file_content`), which is assembled for rendering rather than
/// transfer, except for a run-state artifact, whose envelope redacts the whole
/// file first (`read_run_state_file_content`).
pub(crate) async fn serve_file(
    path: &Path,
    safe_name: &str,
    content_type: Option<&str>,
    disposition: FileDisposition,
    range: Option<&str>,
) -> Result<Response<Body>, ApiError> {
    let metadata = require_regular_file(path).await?;
    let size = metadata.len();
    let header_end = size.min(IMAGE_HEADER_BYTES);
    let header = read_file_window(path, 0, header_end).await?;
    let sniffed = content_type
        .map(str::to_string)
        .or_else(|| sniff_mime(&header))
        .unwrap_or_else(|| guess_mime(path));

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

/// Read a run-state artifact and return its whole redacted text.
///
/// Both run-state serving paths — the download stream and the content envelope —
/// go through this, because both have to redact the *whole* file before any
/// caller-chosen window is taken. A window read first would hand out the raw
/// bytes of that window (walking the offset reconstructs the file, and splitting
/// a credential across two requests defeats the value pass entirely).
///
/// The cap bounds both halves: a file past `max_bytes` is refused rather than
/// read, and a redacted result past it is refused rather than buffered further,
/// because redaction can grow text (an 8-byte value becomes a 23-byte marker).
/// A silent fallback to streaming would be exactly the disclosure this path
/// exists to prevent.
async fn redacted_run_state_text(path: &Path, max_bytes: u64) -> Result<String, ApiError> {
    let metadata = require_regular_file(path).await?;
    let size = metadata.len();
    if size > max_bytes {
        return Err(ApiError::payload_too_large_with_code(
            ProductErrorCode::ProductArtifactTooLarge.as_str(),
            format!("run state artifact exceeds the {max_bytes} byte redaction cap"),
        ));
    }
    let bytes = read_file_window(path, 0, size).await?;
    // These three artifacts are JSON or NDJSON the runtime itself wrote. A file
    // that is not text is not one of them, and guessing would mean either
    // streaming it raw or reporting a corrupt artifact as a secret-exposure
    // failure; neither is honest.
    let text = String::from_utf8(bytes)
        .map_err(|_| ApiError::internal("run state artifact is not valid UTF-8"))?;
    let redacted = rove_runtime::secrets::registry().redact_text(&text);
    if redacted.len() as u64 > max_bytes {
        return Err(ApiError::payload_too_large_with_code(
            ProductErrorCode::ProductArtifactTooLarge.as_str(),
            format!("run state artifact exceeds the {max_bytes} byte redaction cap once redacted"),
        ));
    }
    Ok(redacted)
}

/// Serve a runtime run-state artifact with the secret authority applied.
///
/// `report.json`, `task_state.json`, and `trace.jsonl` are runtime internals:
/// they live under the run directory, not in the workspace, so the byte
/// transport's rationale ("the caller can read the same bytes from disk") does
/// not hold for them. `task_state.json` is the sharp case — it is deliberately
/// written raw because resume needs the original values, so this route is the
/// only redaction boundary it has.
///
/// The rewrite happens *before* the range is computed, so `Content-Length` and
/// `Content-Range` describe the bytes actually sent and no length header can
/// lie about the body.
pub(crate) async fn serve_run_state_file(
    path: &Path,
    safe_name: &str,
    range: Option<&str>,
) -> Result<Response<Body>, ApiError> {
    serve_run_state_file_with_cap(path, safe_name, range, MAX_REDACTED_RUN_STATE_BYTES).await
}

/// `max_bytes` is a parameter so the cap is testable without writing a 64 MiB
/// artifact.
async fn serve_run_state_file_with_cap(
    path: &Path,
    safe_name: &str,
    range: Option<&str>,
    max_bytes: u64,
) -> Result<Response<Body>, ApiError> {
    let redacted = redacted_run_state_text(path, max_bytes).await?;
    let total = redacted.len() as u64;
    let (start, end) = parse_range(range, total, MAX_DOWNLOAD_RANGE_BYTES)?;
    let (start, end) = clamp_range_to_char_boundaries(&redacted, start, end)?;
    let body = redacted[start..end].to_string();
    let length = end - start;

    let mut builder = Response::builder().status(if start > 0 || end < total as usize {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    });
    let headers = builder
        .headers_mut()
        .ok_or_else(|| ApiError::internal("response builder unavailable"))?;
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&guess_mime(path))
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(safe_name, FileDisposition::Attachment))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=download")),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length as u64));
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    if start > 0 || end < total as usize {
        let value = format!("bytes {start}-{}/{total}", end - 1);
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&value)
                .map_err(|_| ApiError::internal("invalid content range"))?,
        );
    }
    builder
        .body(Body::from(body))
        .map_err(|error| ApiError::internal(format!("response build failed: {error}")))
}

/// Pull a byte range onto character boundaries.
///
/// `parse_range` works in bytes and a client may name an offset inside a
/// multi-byte character; slicing a `str` there would panic. The range is
/// narrowed instead of widened, so the bytes sent are always a subset of what
/// was asked for, and the `Content-Range` built from it still describes them.
/// A zero-byte representation is the one window that is both empty and in
/// bounds, so it is returned rather than refused.
fn clamp_range_to_char_boundaries(
    text: &str,
    start: u64,
    end: u64,
) -> Result<(usize, usize), ApiError> {
    let mut start = usize::try_from(start)
        .map_err(|_| ApiError::bad_request("requested range is too large"))?;
    let mut end =
        usize::try_from(end).map_err(|_| ApiError::bad_request("requested range is too large"))?;
    start = start.min(text.len());
    end = end.min(text.len());
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if start == 0 && end == 0 {
        // An empty representation has exactly one window, the empty one. Without
        // this the unranged read of a zero-byte run-state artifact answered 400
        // "range does not contain a whole character"; `parse_range` already
        // rejects an explicit out-of-bounds range before the clamp, so this only
        // ever names the empty file.
        return Ok((0, 0));
    }
    if end <= start {
        return Err(ApiError::bad_request(
            "range does not contain a whole character",
        ));
    }
    Ok((start, end))
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

pub(crate) fn header_range(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
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

pub(crate) async fn require_regular_file(path: &Path) -> Result<std::fs::Metadata, ApiError> {
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

pub(crate) fn workspace_root(
    kind: &ProductWorkspaceKind,
    canonical_root: &Path,
) -> Result<PathBuf, ApiError> {
    let _ = kind;
    if !canonical_root.is_absolute() || !canonical_root.exists() {
        return Err(ApiError::not_found("workspace root"));
    }
    canonical_root
        .canonicalize()
        .map_err(|error| ApiError::internal(format!("workspace canonicalize failed: {error}")))
}

pub(crate) fn join_safe(root: &Path, relative: &str) -> Result<PathBuf, ApiError> {
    rove_product_store::attachment_paths::join_safe(root, relative).map_err(ApiError::bad_request)
}

pub(crate) use rove_product_store::attachment_paths::is_secret_filename;

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
        // RFC 9239 / WHATWG-valid script and document types: anything else
        // would be refused under `X-Content-Type-Options: nosniff` when a
        // previewed page loads the file as a script, stylesheet, or document.
        "js" | "mjs" | "cjs" => "text/javascript".to_string(),
        "css" => "text/css".to_string(),
        "html" | "htm" => "text/html".to_string(),
        "rs" | "toml" | "yaml" | "yml" | "ts" | "tsx" | "jsx" | "py" | "sh" | "bash" | "c"
        | "cc" | "cpp" | "h" | "hpp" | "go" | "java" | "kt" | "rb" | "cs" | "swift" => {
            format!("text/{ext}")
        }
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

pub(crate) fn sniff_mime(bytes: &[u8]) -> Option<String> {
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

    /// The preview envelope is the surface a workspace file and a tool artifact
    /// share, so this one assertion covers both routes.
    #[tokio::test]
    async fn text_previews_redact_a_known_credential_and_keep_the_rest() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("notes.txt");
        let canary = "file-preview-credential-canary-7c25d1";
        assert!(rove_runtime::secrets::registry().register_value(canary));
        let raw = format!("keep-preview-head {canary} keep-preview-tail\n");
        tokio::fs::write(&path, &raw).await.unwrap();
        let content = read_bounded_file_content(&path, None).await.unwrap();
        let text = content.text.expect("a text file has a text preview");
        assert!(!text.contains(canary), "the preview leaked: {text}");
        assert!(text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
        assert!(text.contains("keep-preview-head") && text.contains("keep-preview-tail"));
        // The metadata still describes the file, not the rewritten text.
        assert_eq!(content.size, raw.len() as u64);
        assert_eq!(content.encoding.as_deref(), Some("utf-8"));
    }

    /// Run-state artifacts are runtime internals, so unlike a workspace file
    /// their download is rewritten. `task_state.json` is the case that forced
    /// this path: it is raw on disk for resume.
    #[tokio::test]
    async fn run_state_downloads_are_redacted_and_leave_the_file_alone() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("task_state.json");
        let canary = "run-state-download-canary-5a17c3";
        assert!(rove_runtime::secrets::registry().register_value(canary));
        let raw = format!("{{\"history\":\"keep-run-state {canary} keep-run-state-end\"}}");
        tokio::fs::write(&path, &raw).await.unwrap();

        let response = serve_run_state_file(&path, "task_state.json", None)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "application/json",
            "the served type still describes the artifact"
        );
        let declared = response.headers()[CONTENT_LENGTH]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let served = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            declared,
            served.len(),
            "the declared length must be the length actually sent"
        );
        let served = String::from_utf8(served.to_vec()).unwrap();
        assert!(!served.contains(canary), "the download leaked: {served}");
        assert!(served.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
        assert!(served.contains("keep-run-state"));
        // Nothing was written back: resume still reads the raw value.
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), raw);
    }

    #[tokio::test]
    async fn run_state_range_describes_the_redacted_bytes() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("report.json");
        let canary = "run-state-range-canary-2f8e60";
        assert!(rove_runtime::secrets::registry().register_value(canary));
        let raw = format!("{{\"output\":\"head {canary} tail\"}}");
        tokio::fs::write(&path, &raw).await.unwrap();

        let full = serve_run_state_file(&path, "report.json", None)
            .await
            .unwrap();
        let full = axum::body::to_bytes(full.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let total = full.len();

        let ranged = serve_run_state_file(&path, "report.json", Some("bytes=0-11"))
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            ranged.headers()[CONTENT_RANGE].to_str().unwrap(),
            format!("bytes 0-11/{total}")
        );
        assert_eq!(ranged.headers()[CONTENT_LENGTH].to_str().unwrap(), "12");
        let ranged = axum::body::to_bytes(ranged.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(&ranged[..], &full[..12]);
        assert!(!String::from_utf8_lossy(&ranged).contains(canary));
    }

    /// A client can name a byte offset inside a multi-byte character. The range
    /// is narrowed onto character boundaries rather than slicing a `str` and
    /// panicking, and the `Content-Range` it reports stays true to the bytes.
    #[tokio::test]
    async fn run_state_range_never_slices_a_character_in_half() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("report.json");
        // "é" occupies bytes 6..8, so byte 7 is inside a character.
        let raw = "{\"a\":\"é\",\"b\":\"keep\"}";
        tokio::fs::write(&path, raw).await.unwrap();

        // A range that would end inside the character narrows to an empty span
        // and is refused rather than answered with half a character.
        assert!(clamp_range_to_char_boundaries(raw, 7, 8).is_err());
        assert_eq!(clamp_range_to_char_boundaries(raw, 6, 8).unwrap(), (6, 8));

        let response = serve_run_state_file(&path, "report.json", Some("bytes=6-7"))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(String::from_utf8(body.to_vec()).unwrap(), "é");
    }

    /// Past the cap the route refuses. Streaming the file anyway is the
    /// disclosure this path exists to prevent, so the refusal is asserted rather
    /// than a fallback.
    #[tokio::test]
    async fn an_over_cap_run_state_artifact_is_refused_not_streamed() {
        use axum::response::IntoResponse;

        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("task_state.json");
        let canary = "over-cap-run-state-canary-9c4d21";
        assert!(rove_runtime::secrets::registry().register_value(canary));
        tokio::fs::write(&path, format!("{{\"history\":\"{canary}\"}}"))
            .await
            .unwrap();

        let error = serve_run_state_file_with_cap(&path, "task_state.json", None, 8)
            .await
            .expect_err("an artifact past the cap is refused");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["code"],
            ProductErrorCode::ProductArtifactTooLarge.as_str()
        );
        // The refusal never carries the value it refused to serve.
        assert!(!String::from_utf8_lossy(&serde_json::to_vec(&body).unwrap()).contains(canary));

        // The same file inside the cap is served, so the refusal really is the
        // cap and not something else about the fixture.
        let served = serve_run_state_file_with_cap(&path, "task_state.json", None, 1024)
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
    }

    /// Register a value without asserting on the global registry's state, which
    /// parallel tests in this binary share.
    fn state_canary_register(canary: &str) -> bool {
        rove_runtime::secrets::registry().register_value(canary)
    }

    /// The cap covers the bytes served, not only the bytes read: a file small
    /// enough to read can still redact past it, and this route's whole job is to
    /// bound what it hands out.
    #[tokio::test]
    async fn a_run_state_artifact_that_grows_past_the_cap_while_redacting_is_refused() {
        use axum::response::IntoResponse;

        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("trace.jsonl");
        // Two 8-byte credentials, each of which becomes a 23-byte marker: 18 raw
        // bytes cannot fit in 24 once redacted (46 + the separator).
        let canary = "abcdefgh";
        assert!(state_canary_register(canary));
        let raw = format!("{canary} {canary}\n");
        assert!(raw.len() < 24);
        tokio::fs::write(&path, &raw).await.unwrap();

        let error = serve_run_state_file_with_cap(&path, "trace.jsonl", None, 24)
            .await
            .expect_err("the redacted body is past the cap");
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let response = error.into_response();
        let raw = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        // The refusal body is asserted on the bytes that were actually sent: a
        // re-serialization of the parsed value would not be evidence about the
        // response.
        assert!(
            !String::from_utf8_lossy(&raw).contains(canary),
            "the refusal body must not carry the credential"
        );
        let body: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(
            body["code"],
            ProductErrorCode::ProductArtifactTooLarge.as_str()
        );

        // The same file is served once the cap covers the redacted text, so the
        // refusal is the growth and not something else about the fixture.
        let served = serve_run_state_file_with_cap(&path, "trace.jsonl", None, 1024)
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
        let served = axum::body::to_bytes(served.into_body(), 64 * 1024)
            .await
            .unwrap();
        let served = String::from_utf8(served.to_vec()).unwrap();
        assert!(!served.contains(canary), "{served}");
        assert!(served.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
    }

    /// The preview budget applies to the bytes actually emitted. Redaction can
    /// grow text — an 8-byte value becomes a 23-byte marker — so measuring the
    /// file before the rewrite would let the envelope exceed its documented
    /// bound by nearly three times.
    #[tokio::test]
    async fn the_preview_budget_applies_to_the_redacted_text() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("notes.txt");
        let canary = "abcdefgh";
        assert!(state_canary_register(canary));
        // Just over the read window of repeated 8-byte credentials, each of which
        // redacts into a 23-byte marker: the redacted text is ~2.9x the read.
        let repetitions = (MAX_TEXT_CONTENT_BYTES as usize / canary.len()) + 1_000;
        let raw = canary.repeat(repetitions);
        assert!(raw.len() as u64 > MAX_TEXT_CONTENT_BYTES);
        tokio::fs::write(&path, &raw).await.unwrap();

        let content = read_bounded_file_content(&path, None).await.unwrap();
        let text = content.text.expect("a text file has a text preview");
        assert!(
            text.len() as u64 <= MAX_TEXT_CONTENT_BYTES,
            "the emitted preview must respect the documented bound: {}",
            text.len()
        );
        assert!(content.truncated, "the emitted text was cut to fit");
        assert!(!text.contains(canary), "the preview leaked");
        assert!(text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
        // The metadata still describes the file.
        assert_eq!(content.size, raw.len() as u64);
    }

    #[test]
    fn truncation_never_ends_inside_a_marker() {
        let value = "abc[REDACTED:known_secret]def";
        // Inside the marker's name, inside its opening, and inside its tail: all
        // drop the marker whole rather than emitting half of it.
        for budget in [4, 10, 20, 25] {
            let (kept, cut) = truncate_utf8_preserving_markers(value, budget);
            assert_eq!(kept, "abc", "budget {budget}");
            assert!(cut, "budget {budget}");
        }
        // At the closing bracket the marker is whole and stays.
        let (kept, _) = truncate_utf8_preserving_markers(value, 26);
        assert_eq!(kept, "abc[REDACTED:known_secret]");

        // A bracket that does not open a marker is ordinary text.
        let (kept, cut) = truncate_utf8_preserving_markers("a[1,2,3", 5);
        assert_eq!(kept, "a[1,2");
        assert!(cut);
        // Under the budget nothing changes.
        assert_eq!(
            truncate_utf8_preserving_markers(value, value.len()),
            (value, false)
        );
        // A multi-byte character is never split.
        let (kept, _) = truncate_utf8_preserving_markers("aé", 2);
        assert_eq!(kept, "a");
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

    /// The run-state *content* route is the second serving path for the same
    /// three artifacts, and it takes a caller-chosen window. The rewrite has to
    /// cover the whole file before that window, or walking the offset reassembles
    /// the raw artifact and a credential split across two requests survives.
    #[tokio::test]
    async fn a_run_state_content_window_never_carries_the_credential() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("task_state.json");
        let canary = "run-state-window-canary-4f8c17";
        assert!(state_canary_register(canary));
        // The credential sits inside a window a caller would ask for on its own.
        let raw = format!(
            "{{\"history\":\"keep-window-head {canary} keep-window-tail\",\"note\":\"keep-note\"}}"
        );
        tokio::fs::write(&path, &raw).await.unwrap();
        let credential_at = raw.find(canary).expect("the fixture carries the canary");

        let whole = read_run_state_file_content(&path, None).await.unwrap();
        let whole_text = whole.text.clone().expect("run state is always text");
        assert!(
            !whole_text.contains(canary),
            "the unranged read leaked: {whole_text}"
        );
        assert!(whole_text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
        assert_eq!(
            whole.size,
            whole_text.len() as u64,
            "size is the redacted text"
        );
        assert!(!whole.truncated, "the unranged read is the whole text");
        assert_eq!(whole.encoding.as_deref(), Some("utf-8"));
        assert!(whole.mime.contains("json"), "{}", whole.mime);

        // Every window, including one that starts *inside* the credential's raw
        // offset. Each window must be a substring of the redacted text and the
        // walk must cover every byte: no window may hand back a raw byte of the
        // file, and no split across two requests may reassemble the credential.
        let mut covered = vec![false; whole_text.len()];
        let mut starts: Vec<usize> = (0..whole_text.len()).collect();
        starts.push(credential_at.min(whole_text.len() - 1));
        for start in starts {
            if start >= whole_text.len() {
                continue;
            }
            let end = (start + 14).min(whole_text.len() - 1);
            let window = read_run_state_file_content(&path, Some(&format!("bytes={start}-{end}")))
                .await
                .unwrap();
            let text = window.text.expect("a window of run state is text");
            assert!(
                !text.contains(canary),
                "window {start}-{end} leaked a credential: {text}"
            );
            assert_eq!(window.size, whole_text.len() as u64);
            assert!(window.truncated, "a window is not the whole text");
            assert_eq!(&whole_text[start..end + 1], text.as_str());
            for seen in &mut covered[start..=end] {
                *seen = true;
            }
        }
        assert!(
            covered.iter().all(|seen| *seen),
            "the walk covered every byte of the redacted text"
        );
    }

    /// A zero-byte artifact is an empty success on both serving paths: the
    /// unranged window of an empty representation is the empty window, not a 400
    /// about characters.
    #[tokio::test]
    async fn an_empty_run_state_artifact_is_an_empty_success() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("report.json");
        tokio::fs::write(&path, "").await.unwrap();

        let content = read_run_state_file_content(&path, None).await.unwrap();
        assert_eq!(content.size, 0);
        assert_eq!(content.text.as_deref(), Some(""));
        assert!(!content.truncated);
        assert!(content.preview_allowed);

        let response = serve_run_state_file(&path, "report.json", None)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-length"], "0");
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert!(body.is_empty());
    }

    /// The same cap as the download, on the content path: a run-state artifact
    /// whose redacted text exceeds it is refused rather than buffered.
    #[tokio::test]
    async fn the_run_state_content_route_is_capped_like_the_download() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("trace.jsonl");
        let canary = "abcdefgh";
        assert!(state_canary_register(canary));
        let raw = format!("{canary} {canary}\n");
        assert!(raw.len() < 24);
        tokio::fs::write(&path, &raw).await.unwrap();

        let error = read_run_state_file_content_with_cap(&path, None, 24)
            .await
            .expect_err("the redacted text is past the cap");
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let served = read_run_state_file_content_with_cap(&path, None, 1024)
            .await
            .unwrap();
        let text = served.text.expect("text");
        assert!(!text.contains(canary));
        assert!(text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
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
