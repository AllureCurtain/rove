//! Session-scoped attachment payloads on disk.
//!
//! The layout is `<data_root>/attachments/<product_session_id>/<attachment_id>`
//! with a transient `<attachment_id>.part` during a write:
//!
//! - the root is a sibling of `product.sqlite` under the same pinned user-data
//!   root, so it is never inside a workspace;
//! - the filename is exactly the server-generated ULID. No extension is ever
//!   appended, so no code path can recover a MIME type from a filesystem name;
//! - SQLite is authoritative for metadata. This module owns bytes only and
//!   never decides an identity, a type, or a quota.
//!
//! Write order is file first, row second: a crash can therefore only leave an
//! orphan payload that nothing can resolve, never a row naming a payload that
//! does not exist. The reverse case — a row whose payload is gone — is reported
//! as `missing`/`corrupt` rather than served as a short body.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header::CONTENT_TYPE};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use utoipa::{IntoParams, ToSchema};

use crate::ApiError;
use crate::ApiErrorResponse;
use crate::ApiState;
use crate::docs;
#[cfg(test)]
use rove_product_store::attachment_paths::ATTACHMENTS_DIR;
pub(crate) use rove_product_store::attachment_paths::attachments_root;

use super::ProductAttachmentRecord;
use super::artifacts::hex_digest;
use super::files::{
    FileDisposition, header_range, is_secret_filename, serve_file, sniff_mime,
    validate_raster_image,
};
use super::secret_patterns::contains_secret_pattern;
use rove_product_store::{
    CreateStagedAttachmentRequest, MAX_PRODUCT_ATTACHMENT_DISPLAY_NAME_BYTES,
    MAX_PRODUCT_ATTACHMENT_DOCUMENT_BYTES, MAX_PRODUCT_ATTACHMENT_RASTER_BYTES,
    PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS, ProductAttachmentAvailability, ProductAttachmentId,
    ProductAttachmentStatus, ProductErrorCode, ProductMessageAttachmentRef, ProductSessionId,
    ProductStore,
};
use rove_runtime::conversation::{
    MAX_INLINE_ATTACHMENT_TEXT_BYTES, MessageAttachment, MessageAttachmentContent,
    MessageAttachmentOmission,
};

/// The transient suffix a payload is written under before it is published by a
/// rename. A `.part` file is never readable through the API.
const PART_SUFFIX: &str = ".part";

/// One write chunk. Bounds peak memory beyond the already-collected body and
/// keeps the hash fed in step with the bytes that actually reach the disk.
const WRITE_CHUNK_BYTES: usize = 64 * 1024;

/// How many directory entries one stale-partial scan examines.
///
/// The scan reclaims space, so it must not become an unbounded walk of a
/// directory a failing writer keeps growing. Stopping at the cap is safe
/// because the next upload in the same session scans again.
const MAX_STALE_PART_SCAN: usize = 64;

/// What a successful write produced, as the durable row must record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenAttachment {
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

/// The outcome of resolving a durable row to bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedAttachment {
    pub(crate) availability: ProductAttachmentAvailability,
    /// Present only when the payload verified.
    pub(crate) path: Option<PathBuf>,
}

impl VerifiedAttachment {
    fn unavailable(availability: ProductAttachmentAvailability) -> Self {
        Self {
            availability,
            path: None,
        }
    }
}

/// The attachment payload tree for one data root.
#[derive(Debug, Clone)]
pub(crate) struct AttachmentStorage {
    root: PathBuf,
}

impl AttachmentStorage {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Write `bytes` to `<attachment_id>.part`, hashing while writing, then
    /// publish them with a single rename.
    ///
    /// The partial file is created exclusively, so an existing entry at that
    /// path — including a symbolic link — is refused rather than followed or
    /// overwritten. Any failure removes the partial file, so a refused upload
    /// leaves no byte behind.
    ///
    /// Before writing, the session directory is scanned once for partial files
    /// an earlier interrupted upload left behind (see
    /// [`Self::reclaim_stale_parts`]).
    ///
    /// The caller must hold an [`UploadCleanup`] for the same id: this method's
    /// own cleanup covers a failure it observes, but not a caller that is
    /// dropped mid-write.
    pub(crate) async fn write_payload(
        &self,
        session_id: &ProductSessionId,
        attachment_id: &ProductAttachmentId,
        bytes: &[u8],
    ) -> Result<WrittenAttachment, ApiError> {
        let directory = self.writable_session_dir(session_id).await?;
        self.reclaim_stale_parts(
            session_id,
            Duration::from_secs(PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS),
            MAX_STALE_PART_SCAN,
        )
        .await?;
        let payload = payload_path_in(&directory, attachment_id.as_str())?;
        let part = part_path(&payload)?;
        let written = match write_part(&part, bytes).await {
            Ok(written) => written,
            Err(error) => {
                remove_file_if_present(&part).await;
                return Err(error);
            }
        };
        if let Err(error) = tokio::fs::rename(&part, &payload).await {
            remove_file_if_present(&part).await;
            return Err(ApiError::internal(format!(
                "attachment publish failed: {error}"
            )));
        }
        Ok(written)
    }

    /// Arm the cleanup for one upload before its first byte is written.
    ///
    /// The deadline, a dropped request future, and a panic all stop the future
    /// they run in without executing the cleanup *inside* it, so the removal has
    /// to be a `Drop` side effect of something the request owns. Both paths this
    /// id could have created — the transient `.part` and the published payload —
    /// are named here, and the id is generated per request, so no other request
    /// can own them.
    ///
    /// The guard must be disarmed once a durable row names the payload; see
    /// [`UploadCleanup::disarm`].
    pub(crate) async fn guard_upload(
        &self,
        session_id: &ProductSessionId,
        attachment_id: &ProductAttachmentId,
    ) -> Result<UploadCleanup, ApiError> {
        let directory = self.writable_session_dir(session_id).await?;
        let payload = payload_path_in(&directory, attachment_id.as_str())?;
        let part = part_path(&payload)?;
        Ok(UploadCleanup {
            payload,
            part,
            armed: true,
        })
    }

    /// Remove the transient partial files an earlier upload in this session left
    /// behind, returning how many were removed.
    ///
    /// Only `.part` entries are candidates, only those at least `stale_after`
    /// old are removed, and at most `max_entries` directory entries are
    /// examined. A live concurrent upload in the same session is therefore safe:
    /// its partial file is younger than the upload deadline, because the request
    /// holding it is itself bounded by that deadline.
    ///
    /// A published payload is deliberately never a candidate. Only a durable row
    /// can say whether one is still referenced, and deciding that is the store's
    /// reclamation step rather than the byte layer's.
    pub(crate) async fn reclaim_stale_parts(
        &self,
        session_id: &ProductSessionId,
        stale_after: Duration,
        max_entries: usize,
    ) -> Result<usize, ApiError> {
        let Some(directory) = self.read_session_dir(session_id).await? else {
            return Ok(0);
        };
        reclaim_stale_parts_in(&directory, stale_after, max_entries).await
    }

    /// Remove a payload and its partial file.
    ///
    /// The production removal paths are [`UploadCleanup`], which runs on every
    /// exit including a cancelled request, and the cleanup job, which removes the
    /// payload of a row it has just moved to `expired`. The storage tests use it
    /// directly to assert absence after an explicit removal and that a second
    /// removal of an absent payload is still a success.
    ///
    /// Absence is success: the caller's intent is that no payload remains.
    pub(crate) async fn remove_payload(
        &self,
        session_id: &ProductSessionId,
        attachment_id: &ProductAttachmentId,
    ) -> Result<(), ApiError> {
        self.remove_entry_by_name(session_id, attachment_id.as_str())
            .await
    }

    /// Remove one published entry by name.
    ///
    /// The cleanup job reaches a payload this way when the only thing it knows
    /// is the directory entry: a name that matches no durable row is an orphan,
    /// and an orphan has no id to parse. `payload_path_in` still refuses anything
    /// that is not a single safe path component, so a stray entry can never
    /// redirect the removal out of the session directory.
    pub(crate) async fn remove_entry_by_name(
        &self,
        session_id: &ProductSessionId,
        name: &str,
    ) -> Result<(), ApiError> {
        let Some(directory) = self.read_session_dir(session_id).await? else {
            return Ok(());
        };
        let payload = payload_path_in(&directory, name)?;
        remove_file_if_present(&payload).await;
        if let Ok(part) = part_path(&payload) {
            remove_file_if_present(&part).await;
        }
        Ok(())
    }

    /// Remove a whole session's payload directory.
    ///
    /// The caller removes this only after the session's rows are gone, so the
    /// directory holds nothing the store still names. Absence is success, and a
    /// residue that cannot be removed — a file another process holds open, say —
    /// is left for the next cleanup run rather than failing the delete: the
    /// session's durable state is already gone, and the orphan scan treats a
    /// payload with no row as reclaimable.
    pub(crate) async fn remove_session_dir(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<(), ApiError> {
        let Some(directory) = self.read_session_dir(session_id).await? else {
            return Ok(());
        };
        match tokio::fs::remove_dir_all(&directory).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ApiError::internal(format!(
                "attachment session directory could not be removed: {error}"
            ))),
        }
    }

    /// The session directories that hold bytes on disk, bounded.
    ///
    /// The scan's set comes from the filesystem rather than from the store,
    /// because the residue this has to reclaim is exactly the case where the
    /// rows are gone and the directory is not: the store can no longer name
    /// those sessions at all. A directory whose name is not a session id is
    /// skipped — this code did not create it and does not own it.
    pub(crate) async fn payload_session_ids(
        &self,
        max_entries: usize,
    ) -> Result<Vec<ProductSessionId>, ApiError> {
        if max_entries == 0 {
            return Ok(Vec::new());
        }
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "attachment root could not be listed: {error}"
                )));
            }
        };
        let mut examined = 0;
        let mut sessions = Vec::new();
        while examined < max_entries {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    return Err(ApiError::internal(format!(
                        "attachment root could not be read: {error}"
                    )));
                }
            };
            examined += 1;
            if !entry
                .file_type()
                .await
                .map(|kind| kind.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            if let Ok(id) = entry
                .file_name()
                .to_string_lossy()
                .parse::<ProductSessionId>()
            {
                sessions.push(id);
            }
        }
        Ok(sessions)
    }

    /// The published entry names inside one session directory that are old
    /// enough to be judged, bounded.
    ///
    /// Names only, and only names that have been untouched for `stale_after`.
    /// The age rule is what makes reclamation safe: a payload is published a
    /// moment before its row is committed, and a `.part` file is written before
    /// either, so a young entry may belong to an upload that is still in flight.
    /// `.part` entries are excluded here and reclaimed by
    /// [`Self::reclaim_stale_parts`], which applies the same age rule.
    ///
    /// The caller decides what each name means by matching it against the
    /// session's durable rows; a name that matches no row is the only thing this
    /// can safely call an orphan.
    pub(crate) async fn stale_orphan_candidates(
        &self,
        session_id: &ProductSessionId,
        stale_after: Duration,
        max_entries: usize,
    ) -> Result<Vec<String>, ApiError> {
        let Some(directory) = self.read_session_dir(session_id).await? else {
            return Ok(Vec::new());
        };
        let mut entries = tokio::fs::read_dir(&directory).await.map_err(|error| {
            ApiError::internal(format!("attachment directory could not be listed: {error}"))
        })?;
        let now = std::time::SystemTime::now();
        let mut examined = 0;
        let mut names = Vec::new();
        while examined < max_entries {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    return Err(ApiError::internal(format!(
                        "attachment directory could not be read: {error}"
                    )));
                }
            };
            examined += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(PART_SUFFIX) {
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if !metadata.is_file() {
                continue;
            }
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if now.duration_since(modified).unwrap_or_default() < stale_after {
                continue;
            }
            names.push(name);
        }
        Ok(names)
    }

    /// Read a payload for model injection, bounded before the read.
    ///
    /// The recorded length is checked first, so a payload larger than `max_bytes`
    /// is never opened: an oversized attachment cannot turn a message send into a
    /// multi-megabyte read. A payload that is absent, oversized, not a regular
    /// file, too short, or whose digest no longer matches is `None`, which the
    /// caller renders as a labelled omission rather than as content. The digest
    /// is the same one the download path checks, so bytes the model sees are
    /// always bytes the row describes.
    pub(crate) async fn read_payload_within(
        &self,
        record: &ProductAttachmentRecord,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        if record.byte_length > max_bytes {
            return Ok(None);
        }
        let Some(directory) = self.read_session_dir(&record.product_session_id).await? else {
            return Ok(None);
        };
        let Ok(payload) = payload_path_in(&directory, record.attachment_id.as_str()) else {
            return Ok(None);
        };
        let metadata = match tokio::fs::symlink_metadata(&payload).await {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };
        if !metadata.file_type().is_file() || metadata.len() != record.byte_length {
            return Ok(None);
        }
        let Ok(bytes) = tokio::fs::read(&payload).await else {
            return Ok(None);
        };
        if bytes.len() as u64 != record.byte_length || sha256_bytes(&bytes) != record.sha256 {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Resolve a durable row to bytes.
    ///
    /// The payload must exist, must be a regular file, and must match the
    /// recorded length and digest. Anything else is `missing` or `corrupt`, so
    /// a truncated or replaced body is never streamed as success. An `expired`
    /// row never touches the disk: the row already says why the bytes are gone.
    pub(crate) async fn verify_payload(
        &self,
        record: &ProductAttachmentRecord,
    ) -> Result<VerifiedAttachment, ApiError> {
        if record.status == ProductAttachmentStatus::Expired {
            return Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Expired,
            ));
        }
        let Some(directory) = self.read_session_dir(&record.product_session_id).await? else {
            return Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Missing,
            ));
        };
        // `payload_path_in` refuses a path that escapes the session directory,
        // so a link planted at the payload path resolves to `missing` instead
        // of reaching outside the tree.
        let Ok(payload) = payload_path_in(&directory, record.attachment_id.as_str()) else {
            return Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Missing,
            ));
        };
        let metadata = match tokio::fs::symlink_metadata(&payload).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(VerifiedAttachment::unavailable(
                    ProductAttachmentAvailability::Missing,
                ));
            }
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "attachment payload could not be inspected: {error}"
                )));
            }
        };
        // `symlink_metadata` does not follow the final component, so a symbolic
        // link is reported as a link rather than as the file it points at, and
        // is never read as a payload.
        if !metadata.file_type().is_file() {
            return Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Missing,
            ));
        }
        if metadata.len() != record.byte_length {
            return Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Corrupt,
            ));
        }
        match sha256_file(&payload).await {
            Ok(digest) if digest == record.sha256 => Ok(VerifiedAttachment {
                availability: ProductAttachmentAvailability::Available,
                path: Some(payload),
            }),
            Ok(_) => Ok(VerifiedAttachment::unavailable(
                ProductAttachmentAvailability::Corrupt,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(
                VerifiedAttachment::unavailable(ProductAttachmentAvailability::Missing),
            ),
            Err(error) => Err(ApiError::internal(format!(
                "attachment payload could not be read: {error}"
            ))),
        }
    }

    /// The canonical session directory, created on first write.
    async fn writable_session_dir(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<PathBuf, ApiError> {
        let root = self.ensure_root().await?;
        let directory = session_dir_in(&root, session_id.as_str())?;
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| {
                ApiError::service_unavailable_with_code(
                    "product_store_unavailable",
                    format!("attachment directory could not be created: {error}"),
                )
            })?;
        restrict_directory_permissions(&directory);
        canonical_contained(&root, &directory)
            .await?
            .ok_or_else(|| ApiError::internal("attachment directory disappeared after creation"))
    }

    /// The canonical session directory if it already exists. A read never
    /// creates directories, and a missing root is "no payload".
    async fn read_session_dir(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<PathBuf>, ApiError> {
        let root = match tokio::fs::canonicalize(&self.root).await {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "attachment root could not be read: {error}"
                )));
            }
        };
        let directory = session_dir_in(&root, session_id.as_str())?;
        canonical_contained(&root, &directory).await
    }

    /// Create the root if needed and return it canonicalised, so every later
    /// containment check compares canonical paths.
    async fn ensure_root(&self) -> Result<PathBuf, ApiError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|error| {
                ApiError::service_unavailable_with_code(
                    "product_store_unavailable",
                    format!("attachment root could not be created: {error}"),
                )
            })?;
        restrict_directory_permissions(&self.root);
        tokio::fs::canonicalize(&self.root).await.map_err(|error| {
            ApiError::service_unavailable_with_code(
                "product_store_unavailable",
                format!("attachment root could not be resolved: {error}"),
            )
        })
    }
}

/// Removes both artefacts one upload may create unless it is disarmed.
///
/// The removal lives in `Drop` because no earlier hook exists: `timeout`, a
/// dropped connection, and a panic all stop the upload future without running
/// the cleanup inside it, and only a value the request owns can still act. The
/// two removals are synchronous and bounded (one unlink each), which is why
/// they are acceptable on a runtime worker, and they are best effort: a failure
/// is reported by kind and name only, and
/// [`AttachmentStorage::reclaim_stale_parts`] reclaims a partial file on the next
/// upload in the same session.
#[derive(Debug)]
pub(crate) struct UploadCleanup {
    payload: PathBuf,
    part: PathBuf,
    armed: bool,
}

impl UploadCleanup {
    /// Keep the bytes because a durable row now names them.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UploadCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for path in [&self.part, &self.payload] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(
                    file = %path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("attachment"),
                    kind = ?error.kind(),
                    "failed to remove an aborted attachment write"
                ),
            }
        }
    }
}

fn session_dir_in(root: &Path, session_id: &str) -> Result<PathBuf, ApiError> {
    rove_product_store::attachment_paths::session_dir_in(root, session_id)
        .map_err(ApiError::bad_request)
}

fn payload_path_in(session_dir: &Path, attachment_id: &str) -> Result<PathBuf, ApiError> {
    rove_product_store::attachment_paths::payload_path_in(session_dir, attachment_id)
        .map_err(ApiError::bad_request)
}

fn part_path(payload: &Path) -> Result<PathBuf, ApiError> {
    let name = payload
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApiError::internal("attachment payload path has no name"))?;
    Ok(payload.with_file_name(format!("{name}{PART_SUFFIX}")))
}

/// Canonicalise `candidate` and require that it is still inside `root`. A
/// missing path is `None`, which the read path turns into `missing`.
async fn canonical_contained(root: &Path, candidate: &Path) -> Result<Option<PathBuf>, ApiError> {
    let canonical = match tokio::fs::canonicalize(candidate).await {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ApiError::internal(format!(
                "attachment path could not be resolved: {error}"
            )));
        }
    };
    if !canonical.starts_with(root) {
        return Err(ApiError::bad_request(
            "attachment path escapes the attachment root",
        ));
    }
    Ok(Some(canonical))
}

async fn write_part(part: &Path, bytes: &[u8]) -> Result<WrittenAttachment, ApiError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "attachment partial file could not be created: {error}"
            ))
        })?;
    restrict_file_permissions(part);
    let mut hasher = Sha256::new();
    for chunk in bytes.chunks(WRITE_CHUNK_BYTES) {
        file.write_all(chunk)
            .await
            .map_err(|error| ApiError::internal(format!("attachment write failed: {error}")))?;
        hasher.update(chunk);
    }
    file.flush()
        .await
        .map_err(|error| ApiError::internal(format!("attachment flush failed: {error}")))?;
    file.sync_all()
        .await
        .map_err(|error| ApiError::internal(format!("attachment sync failed: {error}")))?;
    drop(file);
    Ok(WrittenAttachment {
        byte_length: bytes.len() as u64,
        sha256: hex_digest(hasher.finalize().as_slice()),
    })
}

async fn remove_file_if_present(path: &Path) {
    // Best effort by design: the durable row is the recovery point, so a file
    // that cannot be removed is retried by the next cleanup pass rather than
    // turning a refusal into a second failure.
    let _ = tokio::fs::remove_file(path).await;
}

/// The bounded scan behind [`AttachmentStorage::reclaim_stale_parts`].
///
/// `stale_after` and `max_entries` are parameters rather than direct reads of
/// their production bounds so both can be exercised without waiting a minute or
/// planting hundreds of files.
async fn reclaim_stale_parts_in(
    directory: &Path,
    stale_after: Duration,
    max_entries: usize,
) -> Result<usize, ApiError> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(ApiError::internal(format!(
                "attachment directory could not be scanned: {error}"
            )));
        }
    };
    let now = std::time::SystemTime::now();
    let mut examined = 0usize;
    let mut removed = 0usize;
    while examined < max_entries {
        let Some(entry) = entries.next_entry().await.map_err(|error| {
            ApiError::internal(format!("attachment directory could not be read: {error}"))
        })?
        else {
            break;
        };
        examined += 1;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(PART_SUFFIX) {
            continue;
        }
        // An entry whose metadata cannot be read is skipped rather than
        // removed: the scan may only ever delete a file it positively
        // identified as an old, regular partial file. `DirEntry::metadata`
        // does not follow a link, so a link planted at a `.part` name is
        // skipped as well.
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() < stale_after {
            continue;
        }
        remove_file_if_present(&entry.path()).await;
        removed += 1;
    }
    if removed > 0 {
        tracing::debug!(removed, "reclaimed stale attachment partial files");
    }
    Ok(removed)
}

async fn sha256_file(path: &Path) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; WRITE_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize().as_slice()))
}

/// The digest of bytes already in memory.
fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_slice())
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) {}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) {}

/// The extension allow-list, lowercased. It is the first of the two required
/// checks: an extension this list does not name is refused before any byte is
/// sniffed.
const ALLOWED_ATTACHMENT_EXTENSIONS: [&str; 8] =
    ["png", "jpg", "jpeg", "webp", "gif", "pdf", "txt", "md"];

/// Warning code for a secret-shaped display name. A warning, never a refusal:
/// the user asked to send their own file, and silently rejecting or rewriting
/// it would be worse than telling them.
pub(crate) const ATTACHMENT_WARNING_SECRET_NAME: &str = "secret_shaped_name";
/// Warning code for content that matches a known credential pattern. The bytes
/// are stored unmodified.
pub(crate) const ATTACHMENT_WARNING_SECRET_CONTENT: &str = "possible_secret_content";
/// Warning code for a client `Content-Type` claim that disagrees with the
/// locally verified type.
const ATTACHMENT_WARNING_CONTENT_TYPE_MISMATCH: &str = "content_type_claim_mismatch";

/// The raw-body display hint. `name` is optional and is never a path.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AttachmentUploadQuery {
    #[serde(default)]
    pub name: Option<String>,
}

/// The `201` body. Every field is server-verified: `content_type`, `size`, and
/// `sha256` come from the bytes this server hashed, and `warnings` is the
/// stored `scan_flags`.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ProductAttachmentUploadResponse {
    pub attachment_id: ProductAttachmentId,
    pub product_session_id: ProductSessionId,
    /// The locally verified type. The client's claim is never echoed.
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: ProductAttachmentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Always present, and always an array, so a client can distinguish "no
    /// warnings" from "this server did not tell me".
    pub warnings: Vec<String>,
}

impl ProductAttachmentUploadResponse {
    fn from_record(record: ProductAttachmentRecord) -> Self {
        Self {
            attachment_id: record.attachment_id,
            product_session_id: record.product_session_id,
            content_type: record.content_type,
            size: record.byte_length,
            sha256: record.sha256,
            name: record.display_name,
            status: record.status,
            expires_at: record.expires_at,
            warnings: record.scan_flags,
        }
    }
}

/// What validation established about one upload, before anything is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedAttachment {
    /// The locally verified type.
    pub(crate) content_type: String,
    /// Secret-free warning codes, in the order the checks produced them.
    pub(crate) warnings: Vec<String>,
}

/// Bound the process-wide upload slots and the whole upload deadline.
///
/// It is a route layer rather than handler code because the body is collected
/// by the extractor *before* the handler runs: taking the permit and starting
/// the clock here bounds the read as well as the write, which is where the
/// memory and the file handle actually go.
pub(crate) async fn guard_attachment_upload(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let _permit = state.try_acquire_attachment_upload()?;
    match tokio::time::timeout(
        std::time::Duration::from_secs(PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS),
        next.run(request),
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(_) => Err(ApiError::gateway_timeout_with_code(
            ProductErrorCode::ProductAttachmentTimeout.as_str(),
            "attachment upload exceeded its 60 second deadline",
        )),
    }
}

/// Upload one attachment as a raw body.
///
/// The order is fixed and the first failure wins: session, body size, display
/// name, extension allow-list, magic-byte sniffing, MIME allow-list and
/// mismatch, raster bounds, secret warnings, then publication. Nothing is
/// written to disk before every check has passed, and a refusal after the write
/// removes the bytes again.
#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/attachments",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path, description = "Product session ULID"),
        ("name" = Option<String>, Query, description = "Optional display name hint, at most 255 bytes. It is never a path.")
    ),
    request_body(content = String, description = "Raw attachment bytes. Not multipart: the display name travels in the query string.", content_type = "application/octet-stream"),
    responses(
        (status = 201, description = "Attachment stored as a staged row", body = ProductAttachmentUploadResponse),
        (status = 400, description = "Invalid input: name, extension, sniffed type, or extension/sniff mismatch", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "Archived session or exhausted per-session quota", body = ApiErrorResponse),
        (status = 413, description = "Body exceeds the 20 MiB document or 16 MiB raster limit", body = ApiErrorResponse),
        (status = 429, description = "Upload slots are exhausted, or the rate limit was hit", body = ApiErrorResponse),
        (status = 500, description = "The data root could not be written", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable or the data root is not writable", body = ApiErrorResponse),
        (status = 504, description = "The upload exceeded its 60 second deadline", body = ApiErrorResponse)
    )
)]
pub(crate) async fn upload_product_session_attachment(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    Query(query): Query<AttachmentUploadQuery>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<ProductAttachmentUploadResponse>), ApiError> {
    // 1. Session and authorization. Resolved before any path is derived.
    let store = state.product_store()?;
    store.resolve_attachment_session(&session_id).await?;

    // 2. Body size. The route limit already refused an oversized
    //    `Content-Length`; this states the same ceiling for the read body. The
    //    empty body is refused by validation itself, where the type checks can
    //    see it.
    let bytes = match body {
        Ok(bytes) => bytes,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return Err(attachment_too_large());
        }
        Err(_) => {
            return Err(invalid_upload(
                "attachment body could not be read as a raw byte sequence",
            ));
        }
    };
    if bytes.len() as u64 > MAX_PRODUCT_ATTACHMENT_DOCUMENT_BYTES {
        return Err(attachment_too_large());
    }

    // 3. Display name, then 4-8: extension, sniffing, MIME, raster bounds, and
    //    the two warn-only secret checks.
    let display_name = validate_display_name(query.name)?;
    let validated = validate_attachment_upload(
        display_name.as_deref(),
        client_content_type_claim(&headers),
        &bytes,
    )?;

    // 9. Publication. Bytes first so a crash can only leave an orphan payload
    //    that no row can resolve; the row is written inside one transaction
    //    that also decides the quota.
    //
    //    The cleanup guard is armed *before* the first byte and disarmed only
    //    once a durable row names the payload, so every other exit — a refusal,
    //    the route deadline, a dropped connection, a panic — removes what this
    //    id created. That is what makes the deadline a cleanup path rather than
    //    only a refusal.
    let storage = state.attachment_storage();
    let attachment_id = ProductAttachmentId::new();
    let mut cleanup = storage.guard_upload(&session_id, &attachment_id).await?;
    let written = storage
        .write_payload(&session_id, &attachment_id, &bytes)
        .await?;
    let record = store
        .create_staged_attachment(
            &session_id,
            CreateStagedAttachmentRequest {
                attachment_id: attachment_id.clone(),
                content_type: validated.content_type.clone(),
                byte_length: written.byte_length,
                sha256: written.sha256.clone(),
                display_name: display_name.clone(),
                scan_flags: validated.warnings.clone(),
            },
        )
        .await
        // A refusal here — a quota, an archived session, a vanished session —
        // must not leave the bytes it just wrote. The armed guard removes them
        // as this error unwinds, and a removal that fails is reclaimed by the
        // next upload in the session.
        .map_err(ApiError::from)?;
    cleanup.disarm();
    Ok((
        StatusCode::CREATED,
        Json(ProductAttachmentUploadResponse::from_record(record)),
    ))
}

/// `400 product_attachment_invalid_input`, the single code every validation
/// failure shares.
fn invalid_upload(message: impl Into<String>) -> ApiError {
    ApiError::bad_request_with_code(
        ProductErrorCode::ProductAttachmentInvalidInput.as_str(),
        message,
    )
}

fn attachment_too_large() -> ApiError {
    ApiError::payload_too_large_with_code(
        ProductErrorCode::ProductAttachmentTooLarge.as_str(),
        "attachment exceeds the 20 MiB document or 16 MiB raster limit",
    )
}

/// The two verified text types read as one family to a client.
///
/// A browser that opens a `.md` and labels the upload `text/plain` is not
/// contradicting the server: the server verified markdown, stores markdown, and
/// serves markdown, so there is nothing to warn about. Any other disagreement —
/// `image/jpeg` for a PNG, `text/plain` for a PDF — still warns.
const EQUIVALENT_TEXT_CLAIMS: [&str; 2] = ["text/plain", "text/markdown"];

/// Whether a client `Content-Type` claim disagrees with the verified type.
///
/// The comparison is on the media type: parameters are not part of it, and case
/// never is, so `IMAGE/PNG` and `image/png; charset=binary` both agree with a
/// verified `image/png`.
fn claim_disagrees_with(claim: &str, verified: &str) -> bool {
    let claim = claim.split(';').next().unwrap_or(claim).trim();
    if claim.eq_ignore_ascii_case(verified) {
        return false;
    }
    let both_text = EQUIVALENT_TEXT_CLAIMS
        .iter()
        .any(|text| claim.eq_ignore_ascii_case(text))
        && EQUIVALENT_TEXT_CLAIMS
            .iter()
            .any(|text| verified.eq_ignore_ascii_case(text));
    !both_text
}

/// The `Content-Type` request header is a *claim*. It is read only to compare
/// with the verified type and is never stored, echoed, or used to build a path.
///
/// A claim of `application/octet-stream` — the raw-body default, and what the
/// endpoint's own documentation shows — carries no type information and is
/// treated as no claim at all.
fn client_content_type_claim(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(CONTENT_TYPE)?.to_str().ok()?;
    let essence = value.split(';').next().unwrap_or("").trim();
    if essence.is_empty() || essence.eq_ignore_ascii_case("application/octet-stream") {
        return None;
    }
    Some(essence)
}

/// Bound the optional display hint: at most 255 decoded bytes, no control
/// character, no path separator, and no `..` component.
///
/// An empty hint is no hint rather than an error. A secret-shaped name is *not*
/// refused here — it becomes a warning after validation.
fn validate_display_name(name: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(name) = name else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    if name.len() > MAX_PRODUCT_ATTACHMENT_DISPLAY_NAME_BYTES {
        return Err(invalid_upload("attachment display name exceeds 255 bytes"));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid_upload(
            "attachment display name contains a control character",
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(invalid_upload(
            "attachment display name contains a path separator",
        ));
    }
    if Path::new(&name)
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(invalid_upload(
            "attachment display name contains a parent-directory component",
        ));
    }
    Ok(Some(name))
}

/// The ordered type validation: extension allow-list, magic-byte sniffing,
/// MIME allow-list and extension/sniff agreement, raster bounds, then the two
/// warn-only secret checks.
///
/// The returned `content_type` is always the locally verified type. Archives
/// and WebAssembly are refused as positive classifications rather than as
/// unknown bytes, which is what makes "we refused an archive" a statement this
/// code can actually support.
pub(crate) fn validate_attachment_upload(
    display_name: Option<&str>,
    client_content_type: Option<&str>,
    bytes: &[u8],
) -> Result<ValidatedAttachment, ApiError> {
    // 2. An empty body is a body-size failure, and no type check below can see
    //    it: `txt`/`md` have no signature, so an empty text body would
    //    otherwise be stored as a zero-byte attachment.
    if bytes.is_empty() {
        return Err(invalid_upload("attachment body is empty"));
    }

    // 4. Extension allow-list. A missing or unknown extension is refused before
    //    any byte is inspected.
    let extension = display_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| ALLOWED_ATTACHMENT_EXTENSIONS.contains(&extension.as_str()))
        .ok_or_else(|| {
            invalid_upload(
                "attachment name must carry one of the allowed extensions: png jpg jpeg webp gif pdf txt md",
            )
        })?;

    // The raster ceiling is a body bound specialized by type, so it is applied
    // as soon as the type is known and before the sniff comparison: an
    // oversized raster is a 413, not a 400 that hides why.
    let is_raster = matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif");
    if is_raster && bytes.len() as u64 > MAX_PRODUCT_ATTACHMENT_RASTER_BYTES {
        return Err(attachment_too_large());
    }

    // 5. Magic-byte sniffing.
    let sniffed = sniff_mime(bytes);

    // 6. MIME allow-list and extension/sniff agreement.
    let content_type = match extension.as_str() {
        "png" => expect_raster(sniffed.as_deref(), "image/png", bytes, "png")?,
        "jpg" | "jpeg" => expect_raster(sniffed.as_deref(), "image/jpeg", bytes, "jpeg")?,
        "webp" => expect_raster(sniffed.as_deref(), "image/webp", bytes, "webp")?,
        "gif" => expect_raster(sniffed.as_deref(), "image/gif", bytes, "gif")?,
        "pdf" => {
            refuse_refused_signature(sniffed.as_deref())?;
            if sniffed.as_deref() != Some("application/pdf") {
                return Err(invalid_upload(
                    "attachment extension and signature disagree: pdf expects a %PDF- header",
                ));
            }
            "application/pdf".to_string()
        }
        // A text extension claims bytes with no signature. Any *positively
        // classified* signature is therefore a disagreement, and the refusal
        // names what the bytes are. Without this check a `PK\x03\x04` prefix
        // followed by ASCII-only bytes — legal UTF-8 with no NUL — would be
        // accepted and stored as `text/plain`.
        "txt" => {
            refuse_non_text_signature(sniffed.as_deref())?;
            require_utf8_text(bytes)?;
            "text/plain".to_string()
        }
        "md" => {
            refuse_non_text_signature(sniffed.as_deref())?;
            require_utf8_text(bytes)?;
            "text/markdown".to_string()
        }
        _ => return Err(invalid_upload("attachment extension is not allowed")),
    };

    // 8. Secret checks, warn-only, on the display name and on text content. The
    //    bytes are stored unmodified.
    let mut warnings = Vec::new();
    if display_name.is_some_and(is_secret_filename) {
        warnings.push(ATTACHMENT_WARNING_SECRET_NAME.to_string());
    }
    if content_type.starts_with("text/")
        && std::str::from_utf8(bytes)
            .ok()
            .is_some_and(contains_secret_pattern)
    {
        warnings.push(ATTACHMENT_WARNING_SECRET_CONTENT.to_string());
    }
    if client_content_type.is_some_and(|claim| claim_disagrees_with(claim, &content_type)) {
        warnings.push(ATTACHMENT_WARNING_CONTENT_TYPE_MISMATCH.to_string());
    }

    Ok(ValidatedAttachment {
        content_type,
        warnings,
    })
}

/// Raster extensions must sniff as their own raster type, and must then pass
/// the existing header validation and pixel caps.
fn expect_raster(
    sniffed: Option<&str>,
    expected: &str,
    bytes: &[u8],
    label: &str,
) -> Result<String, ApiError> {
    refuse_refused_signature(sniffed)?;
    if sniffed != Some(expected) {
        return Err(invalid_upload(format!(
            "attachment extension and signature disagree: {label} does not start with a {expected} signature"
        )));
    }
    // 7. Dimension and pixel bounds, reusing the audited raster validator.
    validate_raster_image(bytes, bytes.len() as u64)
        .map_err(|_| invalid_upload("raster image failed format, size, or pixel validation"))?;
    Ok(expected.to_string())
}

/// Archives and WebAssembly are refused as identified types, not tolerated as
/// unknown ones. Both are parsed from their magic bytes, so this refusal is
/// based on a positive classification (design section 8.3).
fn refuse_refused_signature(sniffed: Option<&str>) -> Result<(), ApiError> {
    match sniffed {
        Some("application/zip") => Err(invalid_upload(
            "archive attachments are not accepted in this batch",
        )),
        Some("application/wasm") => Err(invalid_upload(
            "executable attachments are not accepted in this batch",
        )),
        _ => Ok(()),
    }
}

/// A signature that contradicts a text extension.
///
/// `txt` and `md` are the only extensions that assert "these bytes carry no
/// signature", so *any* positive classification disagrees with them — an
/// archive, an executable, a raster, or a PDF. The refusal is deliberately
/// driven by [`sniff_mime`]'s answer rather than by the byte values: the
/// alternative check ([`require_utf8_text`]) is about encoding, and an archive
/// whose body happens to be ASCII is still an archive.
fn refuse_non_text_signature(sniffed: Option<&str>) -> Result<(), ApiError> {
    match sniffed {
        None => Ok(()),
        Some("application/zip") => Err(invalid_upload(
            "archive attachments are not accepted in this batch",
        )),
        Some("application/wasm") => Err(invalid_upload(
            "executable attachments are not accepted in this batch",
        )),
        Some(verified) => Err(invalid_upload(format!(
            "attachment extension and signature disagree: a text attachment cannot carry a {verified} signature"
        ))),
    }
}

/// `txt`/`md` have no signature, so they are accepted only as valid UTF-8
/// without a NUL byte — the same test the file API already applies.
fn require_utf8_text(bytes: &[u8]) -> Result<(), ApiError> {
    if std::str::from_utf8(bytes).is_err() || bytes.contains(&0) {
        return Err(invalid_upload(
            "text attachments must be valid UTF-8 without a NUL byte",
        ));
    }
    Ok(())
}

/// Serve one stored attachment by the `(session, attachment)` pair.
///
/// The pair is the whole lookup: an attachment that belongs to another session
/// is indistinguishable from one that does not exist, which is what keeps this
/// endpoint from confirming another session's identifiers.
///
/// There is deliberately no list endpoint in this batch, so a client cannot
/// enumerate a session it does not own: the upload response is the only way an
/// attachment id becomes known.
///
/// Verification and service are two opens of the same path, so a payload
/// swapped in between is a real, narrow window rather than an absent one. What
/// bounds it is the pin: the bytes sent are read from the same path whose length
/// and digest were just verified, so a swap can only ever yield bytes this
/// build did not verify and would refuse — a different file that happens to
/// match the recorded length *and* digest is byte-identical by construction. The
/// residual is a provenance question (which file the successful open named), not
/// a content-exposure one, and closing it would mean re-implementing the audited
/// range and disposition path around an already-open handle instead of reusing
/// `serve_file`.
#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/attachments/{attachment_id}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = String, Path, description = "Product session ULID"),
        ("attachment_id" = String, Path, description = "Attachment ULID")
    ),
    responses(
        (status = 200, description = "The stored bytes with the locally verified content type and a fixed header set"),
        (status = 206, description = "The requested byte range"),
        (status = 400, description = "Invalid or unsatisfiable range", body = ApiErrorResponse),
        (status = 404, description = "Unknown attachment, or one owned by another session", body = ApiErrorResponse),
        (status = 409, description = "The staged attachment expired before it was referenced", body = ApiErrorResponse),
        (status = 410, description = "The row is intact but its payload is absent or corrupt", body = ApiErrorResponse),
        (status = 500, description = "The payload could not be read", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_session_attachment(
    State(state): State<ApiState>,
    AxumPath((session_id, attachment_id)): AxumPath<(ProductSessionId, ProductAttachmentId)>,
    headers: HeaderMap,
) -> Result<Response<Body>, ApiError> {
    let store = state.product_store()?;
    // An id that belongs to another session is indistinguishable from an absent
    // one, and both are `404` with the same code and message.
    let record = store
        .attachment_for_session(&session_id, &attachment_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found_with_code(
                ProductErrorCode::ProductAttachmentNotFound.as_str(),
                "attachment not found",
            )
        })?;

    // Re-verify the payload against the durable metadata before the first byte
    // is sent: a length or digest mismatch is a `410`, never a silently
    // truncated or substituted download.
    let storage = state.attachment_storage();
    let verified = storage.verify_payload(&record).await?;
    let path = match verified.availability {
        ProductAttachmentAvailability::Available => verified.path.ok_or_else(|| {
            ApiError::internal("an available attachment must resolve a payload path")
        })?,
        ProductAttachmentAvailability::Expired => {
            return Err(ApiError::conflict_with_code(
                ProductErrorCode::ProductAttachmentConflict.as_str(),
                "attachment expired before it was referenced",
            ));
        }
        ProductAttachmentAvailability::Missing | ProductAttachmentAvailability::Corrupt => {
            // The row existed a moment ago. A session or attachment deleted
            // since then has no row at all, and "there is no such attachment" is
            // the same `404` the first lookup gives rather than a `410` that
            // tells the client the bytes are gone for good. Removing a payload
            // and removing its row are separate acts, so an intact row with
            // absent bytes stays a `410`.
            let row_still_present = store
                .attachment_for_session(&session_id, &attachment_id)
                .await?
                .is_some();
            return Err(unavailable_attachment_error(row_still_present));
        }
    };

    let range = header_range(&headers)?;
    // A ranged request is a download, not a preview: `InlineRasterImage` serves
    // the whole validated image and ignores `Range`, so a range on an image is
    // served under the attachment disposition, where ranges are honoured.
    let inline = is_inline_servable(&record.content_type) && range.is_none();
    let safe_name = record
        .display_name
        .clone()
        .unwrap_or_else(|| record.attachment_id.as_str().to_string());
    serve_file(
        &path,
        &safe_name,
        Some(&record.content_type),
        if inline {
            FileDisposition::InlineRasterImage
        } else {
            FileDisposition::Attachment
        },
        range,
    )
    .await
}

/// The refusal for a payload that did not verify.
///
/// `row_still_present` is the answer to a second pair lookup, taken after the
/// payload check failed: a row that vanished between the two lookups is the same
/// `404` as an attachment whose id was never known, while an intact row whose
/// bytes are absent or replaced is a `410` — the client's reference is real and
/// the bytes are not.
fn unavailable_attachment_error(row_still_present: bool) -> ApiError {
    if row_still_present {
        ApiError::gone_with_code(
            ProductErrorCode::ProductAttachmentUnavailable.as_str(),
            "attachment payload is unavailable",
        )
    } else {
        ApiError::not_found_with_code(
            ProductErrorCode::ProductAttachmentNotFound.as_str(),
            "attachment not found",
        )
    }
}

/// The types a browser may render in place. Everything else is an attachment
/// download, and `application/pdf` and text stay out of the inline set: their
/// content is active in a way a raster image is not.
fn is_inline_servable(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// How many staged rows one cleanup run may expire.
pub(crate) const MAX_ATTACHMENT_CLEANUP_ROWS_PER_RUN: usize = 256;

/// How many sessions one cleanup run may examine for orphan payloads.
pub(crate) const MAX_ATTACHMENT_CLEANUP_SESSIONS_PER_RUN: usize = 64;

/// How many directory entries one cleanup run may examine inside one session.
///
/// The bound is on entries *examined*, not on orphans removed, so a directory a
/// failing writer keeps growing cannot turn one run into an unbounded walk. The
/// next run continues from the same place, because the walk reads the directory
/// order the filesystem reports rather than a cursor this code owns.
pub(crate) const MAX_ATTACHMENT_CLEANUP_ENTRIES_PER_SESSION: usize = 256;

/// Delay before the first cleanup run.
///
/// A start must not wait on reclamation, and the staged TTL is 24 hours, so a
/// minute of grace costs nothing and keeps the boot path free of a filesystem
/// walk.
pub(crate) const ATTACHMENT_CLEANUP_FIRST_DELAY_SECONDS: u64 = 60;

/// Period between cleanup runs.
pub(crate) const ATTACHMENT_CLEANUP_INTERVAL_SECONDS: u64 = 900;

/// The bounds one cleanup run applies.
///
/// A struct rather than three constants read inline, so a test can drive the
/// same code with tiny bounds instead of creating hundreds of rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AttachmentCleanupLimits {
    pub(crate) rows: usize,
    pub(crate) sessions: usize,
    pub(crate) entries_per_session: usize,
}

impl Default for AttachmentCleanupLimits {
    fn default() -> Self {
        Self {
            rows: MAX_ATTACHMENT_CLEANUP_ROWS_PER_RUN,
            sessions: MAX_ATTACHMENT_CLEANUP_SESSIONS_PER_RUN,
            entries_per_session: MAX_ATTACHMENT_CLEANUP_ENTRIES_PER_SESSION,
        }
    }
}

/// What one cleanup run did.
///
/// Counts only. It is the whole of what a run reports, in the log and to a test,
/// because an attachment's display name, its bytes, and its path must not appear
/// in a log line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AttachmentCleanupOutcome {
    /// Staged rows this run moved to `expired`.
    pub(crate) expired_rows: usize,
    /// Payloads removed because their row had just expired.
    pub(crate) removed_expired_payloads: usize,
    /// Payloads removed because no row named them.
    pub(crate) removed_orphan_payloads: usize,
    /// Interrupted-upload partial files reclaimed.
    pub(crate) reclaimed_partials: usize,
    /// Operations that failed. A failure is counted and logged, never fatal:
    /// one unremovable file must not stop the rest of the run.
    pub(crate) failures: usize,
}

/// Reclaim what an unreferenced attachment costs: expire what the TTL has
/// passed, remove the bytes that just expired, and remove payloads no row names.
///
/// Order is the invariant. A row is moved to `expired` *before* its payload is
/// touched, and the reference transition only accepts a row that is `staged` or
/// already `referenced`, so a message can never be written against a payload
/// this function is about to remove:
///
/// * a message that references the row first leaves `staged`, which removes it
///   from this function's candidate set entirely;
/// * a row this function expires cannot afterwards become referenced, because
///   the compare-and-set requires `status = 'staged'`;
/// * a projection that reads the row in between sees `expired`, which is
///   reported as an omission, not as bytes.
///
/// The same rule covers the interleaving with a session delete: the rows go
/// first, so a payload this function still sees is one whose row is gone or
/// expired, and both are reclaimable.
pub(crate) async fn run_attachment_cleanup(
    store: &dyn ProductStore,
    storage: &AttachmentStorage,
    limits: AttachmentCleanupLimits,
) -> AttachmentCleanupOutcome {
    let mut outcome = AttachmentCleanupOutcome::default();
    match store.expire_staged_attachments(limits.rows).await {
        Ok(expired) => {
            outcome.expired_rows = expired.len();
            for record in expired {
                match storage
                    .remove_payload(&record.product_session_id, &record.attachment_id)
                    .await
                {
                    Ok(()) => outcome.removed_expired_payloads += 1,
                    Err(_) => {
                        outcome.failures += 1;
                        // The id is a server-generated ULID: it identifies the
                        // row without naming the file the user uploaded. The
                        // failure's own text can carry a filesystem path, so it
                        // is not logged.
                        tracing::warn!(
                            attachment_id = %record.attachment_id,
                            "attachment cleanup could not remove an expired payload"
                        );
                    }
                }
            }
        }
        Err(error) => {
            outcome.failures += 1;
            tracing::warn!("attachment cleanup could not expire staged rows: {error}");
        }
    }
    // The scan's set comes from the payload root, not from the store: the case
    // this has to cover is precisely the one where the rows are gone (a deleted
    // session whose directory could not be removed) and the store can no longer
    // name it. Each directory is then judged against the rows that do exist.
    let sessions = match storage.payload_session_ids(limits.sessions).await {
        Ok(sessions) => sessions,
        Err(_) => {
            outcome.failures += 1;
            tracing::warn!("attachment cleanup could not list the payload root");
            return outcome;
        }
    };
    for session_id in sessions {
        outcome.reclaimed_partials +=
            reclaim_session_partials(storage, &session_id, &mut outcome).await;
        let statuses = match store
            .attachment_statuses_for_session(&session_id, limits.entries_per_session)
            .await
        {
            Ok(statuses) => statuses,
            Err(error) => {
                outcome.failures += 1;
                tracing::warn!(
                    "attachment cleanup could not read a session's attachment rows: {error}"
                );
                continue;
            }
        };
        let names = match storage
            .stale_orphan_candidates(
                &session_id,
                Duration::from_secs(PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS),
                limits.entries_per_session,
            )
            .await
        {
            Ok(names) => names,
            Err(_) => {
                outcome.failures += 1;
                // The failure's own text can name a path, so only the count moves.
                tracing::warn!("attachment cleanup could not list a session payload directory");
                continue;
            }
        };
        for name in names {
            let reclaimable = match statuses
                .iter()
                .find(|(id, _)| id.as_str() == name)
                .map(|(_, status)| *status)
            {
                // No row names this entry: an upload that failed after its
                // publish, or a payload whose row a session delete already
                // removed.
                None => true,
                // The row is gone by decision and its payload was supposed to
                // follow. This is the residue of a removal that failed, and the
                // row still cannot be referenced again.
                Some(ProductAttachmentStatus::Expired) => true,
                // Staged and referenced rows are never candidates: their
                // payload is either in use or waiting to be referenced.
                Some(ProductAttachmentStatus::Staged | ProductAttachmentStatus::Referenced) => {
                    false
                }
            };
            if !reclaimable {
                continue;
            }
            match storage.remove_entry_by_name(&session_id, &name).await {
                Ok(()) => outcome.removed_orphan_payloads += 1,
                Err(_) => {
                    // No name and no path in the log line, deliberately: an
                    // orphan entry is a filename this code did not generate.
                    outcome.failures += 1;
                }
            }
        }
    }
    outcome
}

async fn reclaim_session_partials(
    storage: &AttachmentStorage,
    session_id: &ProductSessionId,
    outcome: &mut AttachmentCleanupOutcome,
) -> usize {
    match storage
        .reclaim_stale_parts(
            session_id,
            Duration::from_secs(PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS),
            MAX_ATTACHMENT_CLEANUP_ENTRIES_PER_SESSION,
        )
        .await
    {
        Ok(removed) => removed,
        Err(_) => {
            outcome.failures += 1;
            tracing::warn!("attachment cleanup could not reclaim partial uploads");
            0
        }
    }
}

/// The verified types this build inlines into a model request as text.
///
/// Exactly the two types the upload path stores after validating that the bytes
/// are UTF-8. Nothing is inferred from an extension or a client claim, so an
/// unverified type can never be decoded as text.
fn is_inline_text_type(content_type: &str) -> bool {
    matches!(content_type, "text/plain" | "text/markdown")
}

/// Resolve a message's attachment references into path-free content the runtime
/// may render.
///
/// Every reference produces exactly one [`MessageAttachment`], so the number of
/// injected blocks equals the number of references. Nothing here fails the send:
/// a payload that is absent, replaced, oversized, or not readable becomes a
/// labelled omission, because a message the user sent must not be refused — or
/// silently sent without its attachment — because a file changed on disk.
///
/// The runtime never sees a path: the bytes are read here, under this surface's
/// own root, and the value handed across the seam carries identity, verified
/// metadata, and content only.
pub(crate) async fn resolve_message_attachments(
    storage: &AttachmentStorage,
    store: &dyn ProductStore,
    session_id: &ProductSessionId,
    references: &[ProductMessageAttachmentRef],
) -> Vec<MessageAttachment> {
    let mut resolved = Vec::with_capacity(references.len());
    for reference in references {
        resolved.push(resolve_message_attachment(storage, store, session_id, reference).await);
    }
    resolved
}

async fn resolve_message_attachment(
    storage: &AttachmentStorage,
    store: &dyn ProductStore,
    session_id: &ProductSessionId,
    reference: &ProductMessageAttachmentRef,
) -> MessageAttachment {
    let mut attachment = MessageAttachment {
        attachment_id: reference.attachment_id.as_str().to_string(),
        content_type: reference.content_type.clone(),
        byte_length: reference.size,
        sha256: reference.sha256.clone(),
        display_name: reference.name.clone(),
        content: MessageAttachmentContent::Reference,
    };
    if reference.availability != ProductAttachmentAvailability::Available {
        attachment.content =
            MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable);
        return attachment;
    }
    if is_raster_image_type(&reference.content_type) {
        return resolve_image_attachment(storage, store, session_id, reference, attachment).await;
    }
    if !is_inline_text_type(&reference.content_type) {
        // Everything else (PDFs today) is carried as a reference. This build
        // projects no binary payload for it, and the injected block says so
        // instead of inventing content.
        return attachment;
    }
    if reference.size > MAX_INLINE_ATTACHMENT_TEXT_BYTES as u64 {
        attachment.content = MessageAttachmentContent::Omitted(MessageAttachmentOmission::TooLarge);
        return attachment;
    }
    let record = match store
        .attachment_for_session(session_id, &reference.attachment_id)
        .await
    {
        Ok(Some(record)) => record,
        Ok(None) | Err(_) => {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable);
            return attachment;
        }
    };
    match storage
        .read_payload_within(&record, MAX_INLINE_ATTACHMENT_TEXT_BYTES as u64)
        .await
    {
        Ok(Some(bytes)) => {
            attachment.content = match String::from_utf8(bytes) {
                Ok(text) => MessageAttachmentContent::Text(text),
                // The row recorded a text type but the bytes are not UTF-8 now,
                // so the payload no longer matches what was stored.
                Err(_) => MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable),
            };
        }
        Ok(None) | Err(_) => {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable);
        }
    }
    attachment
}

/// Whether the locally verified type is a raster image the model layer can
/// project as an image content block.
pub(crate) fn is_raster_image_type(content_type: &str) -> bool {
    content_type == "image/png"
        || content_type == "image/jpeg"
        || content_type == "image/webp"
        || content_type == "image/gif"
}

/// Read one referenced raster image and encode it for the model layer.
///
/// The bytes are read through the same bounded, digest-verified path as text:
/// the payload must still hash to the recorded `sha256`, and the base64
/// encoding must fit the model protocol's content bound for one block. An
/// oversized image is a labelled [`MessageAttachmentOmission::ImageTooLarge`],
/// never a truncated body and never a payload that would fail the whole turn
/// at dispatch.
async fn resolve_image_attachment(
    storage: &AttachmentStorage,
    store: &dyn ProductStore,
    session_id: &ProductSessionId,
    reference: &ProductMessageAttachmentRef,
    mut attachment: MessageAttachment,
) -> MessageAttachment {
    // The serialized block adds the MIME type, the enum tag, and JSON
    // structure on top of the base64 payload; the slack keeps that overhead
    // inside the protocol's per-request content bound.
    const IMAGE_BLOCK_SLACK_BYTES: usize = 256;
    let max_base64_bytes = rove_models::MAX_CONTENT_BYTES.saturating_sub(IMAGE_BLOCK_SLACK_BYTES);
    let record = match store
        .attachment_for_session(session_id, &reference.attachment_id)
        .await
    {
        Ok(Some(record)) => record,
        Ok(None) | Err(_) => {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable);
            return attachment;
        }
    };
    // Base64 inflates by 4/3; a payload whose encoding cannot fit the model
    // protocol's content bound is refused before the bytes are ever pulled.
    if base64_len(reference.size as usize) > max_base64_bytes {
        attachment.content =
            MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImageTooLarge);
        return attachment;
    }
    // The bounded read verifies the recorded digest, so the bytes the model
    // sees are always bytes the row describes.
    match storage.read_payload_within(&record, reference.size).await {
        Ok(Some(bytes)) => {
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            if data_base64.len() > max_base64_bytes {
                attachment.content =
                    MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImageTooLarge);
                return attachment;
            }
            attachment.content = MessageAttachmentContent::Image {
                mime_type: reference.content_type.clone(),
                data_base64,
            };
        }
        Ok(None) | Err(_) => {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::Unavailable);
        }
    }
    attachment
}

fn base64_len(raw_bytes: usize) -> usize {
    raw_bytes.saturating_add(2) / 3 * 4
}

/// Map one resolved attachment set for a model with the given image support,
/// and extract the image blocks the model request should carry.
///
/// Without image support every image becomes the labelled
/// [`MessageAttachmentOmission::ImagesUnsupported`] line. With support, an
/// image projects a block only while the set still fits the model protocol's
/// per-request content bound; the first image that no longer fits — and every
/// one after it — becomes the labelled [`MessageAttachmentOmission::ImageTooLarge`]
/// line, so the reference count stays equal to the visible block count and no
/// image degrades silently.
pub(crate) fn map_attachments_for_model(
    mut attachments: Vec<MessageAttachment>,
    images_supported: bool,
) -> (Vec<MessageAttachment>, Vec<rove_models::ContentBlock>) {
    if !images_supported {
        for attachment in &mut attachments {
            if matches!(attachment.content, MessageAttachmentContent::Image { .. }) {
                attachment.content =
                    MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImagesUnsupported);
            }
        }
    }
    let mut remaining = rove_models::MAX_CONTENT_BYTES;
    let mut blocks = Vec::new();
    for attachment in &mut attachments {
        let MessageAttachmentContent::Image {
            mime_type,
            data_base64,
        } = &attachment.content
        else {
            continue;
        };
        let candidate = rove_models::ContentBlock::Image {
            mime_type: mime_type.clone(),
            data: data_base64.clone(),
        };
        let Ok(block_bytes) = serde_json::to_vec(&candidate) else {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImageTooLarge);
            continue;
        };
        if block_bytes.len() > remaining {
            attachment.content =
                MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImageTooLarge);
            continue;
        }
        remaining -= block_bytes.len();
        blocks.push(candidate);
    }
    (attachments, blocks)
}

#[cfg(test)]
mod tests {
    use rove_runtime::conversation::{
        MessageAttachment, MessageAttachmentContent, MessageAttachmentOmission,
        compose_user_message,
    };

    fn resolved_image(data_base64: &str) -> MessageAttachment {
        MessageAttachment {
            attachment_id: "01J8Z0M6Q3W9F2V7B4K1N5T8XI1".to_string(),
            content_type: "image/png".to_string(),
            byte_length: 96,
            sha256: "0".repeat(64),
            display_name: Some("screenshot.png".to_string()),
            content: MessageAttachmentContent::Image {
                mime_type: "image/png".to_string(),
                data_base64: data_base64.to_string(),
            },
        }
    }

    #[test]
    fn a_capable_model_receives_the_image_and_an_incapable_one_gets_the_placeholder() {
        let (mapped, blocks) = map_attachments_for_model(vec![resolved_image("aW1hZ2U=")], true);
        assert_eq!(blocks.len(), 1);
        let text = compose_user_message("look", &mapped);
        assert!(text.contains("[image sent to the model as an image content block]"));

        let (mapped, blocks) = map_attachments_for_model(vec![resolved_image("aW1hZ2U=")], false);
        assert!(blocks.is_empty());
        let text = compose_user_message("look", &mapped);
        assert!(
            text.contains(
                "[attachment not sent: screenshot.png (image/png, 96 B) — the selected model does not accept image input]"
            ),
            "{text}"
        );
        // The degraded set never projects bytes by another route.
        assert!(!text.contains("aW1hZ2U="));
    }

    #[test]
    fn the_content_budget_degrades_later_images_rather_than_failing_the_turn() {
        let big = "x".repeat(rove_models::MAX_CONTENT_BYTES / 4);
        let attachments = vec![
            resolved_image(&big),
            resolved_image(&big),
            resolved_image(&big),
            resolved_image(&big),
        ];
        let (mapped, blocks) = map_attachments_for_model(attachments, true);
        assert_eq!(
            blocks.len(),
            3,
            "three quarter-bound images fit; the fourth does not"
        );
        let degraded = mapped
            .iter()
            .filter(|attachment| {
                matches!(
                    attachment.content,
                    MessageAttachmentContent::Omitted(MessageAttachmentOmission::ImageTooLarge)
                )
            })
            .count();
        assert_eq!(degraded, 1, "the first image past the budget is labelled");
        let text = compose_user_message("look", &mapped);
        assert!(text.contains("exceeds the"));
    }

    #[test]
    fn attachment_content_grants_no_capability_or_path_in_the_request() {
        // The attachment surface resolves plain data and never a filesystem
        // path; the injected block is text inside a user message and carries
        // no capability, approval, or permission of any kind.
        let (mapped, _blocks) = map_attachments_for_model(vec![resolved_image("aW1hZ2U=")], true);
        let text = compose_user_message("look", &mapped);
        for forbidden in ["\\", "attachments/", ".rove", "payload"] {
            assert!(
                !text.contains(forbidden),
                "the composed request must not name a path fragment: {forbidden}"
            );
        }
        // The attachment's content is data inside a labelled block: it can
        // never add a tool, an approval, or a permission to the request.
        assert!(!text.contains("tool"));
        assert!(!text.contains("approve"));
    }

    use tempfile::TempDir;

    use super::*;
    use crate::product::ProductAttachmentRecord;

    const SESSION_A: &str = "01J8Z0M6Q3W9F2V7B4K1N5T8XA";
    const SESSION_B: &str = "01J8Z0M6Q3W9F2V7B4K1N5T8XB";

    /// Every warning code the upload response can carry, in a fixed order.
    ///
    /// The codes are a **contract**: they are stable, lowercase, snake-case
    /// identifiers, they name a condition rather than quoting the payload, and a
    /// client maps each one to a localized string. Nothing else may be added to
    /// a `warnings` array — in particular no matched text, no display name, and
    /// no path, because a warning travels to the browser and into screenshots.
    /// The three codes below are the complete set, which is what makes
    /// `attachment_warning_codes_are_stable_and_secret_free` a real assertion
    /// rather than a spot check. It lives in the test module because the codes
    /// are emitted individually at their scan sites; the array itself is the
    /// test's pin on that emission being exhaustive.
    const ATTACHMENT_WARNING_CODES: [&str; 3] = [
        ATTACHMENT_WARNING_SECRET_NAME,
        ATTACHMENT_WARNING_SECRET_CONTENT,
        ATTACHMENT_WARNING_CONTENT_TYPE_MISMATCH,
    ];

    fn session(value: &str) -> ProductSessionId {
        value.parse().unwrap()
    }

    fn storage() -> (TempDir, AttachmentStorage) {
        let temp = TempDir::new().unwrap();
        let storage = AttachmentStorage::new(storage_root(&temp));
        (temp, storage)
    }

    /// The tree `storage()` writes under, named without the storage type so a
    /// test can assert what a read did *not* create.
    fn storage_root(temp: &TempDir) -> std::path::PathBuf {
        temp.path().join("data").join(ATTACHMENTS_DIR)
    }

    /// How many entries a session directory holds, so a cleanup assertion is
    /// "nothing is left" rather than "this one name is gone".
    fn directory_entries(directory: &Path) -> usize {
        std::fs::read_dir(directory)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    fn record(
        session_id: &ProductSessionId,
        attachment_id: &ProductAttachmentId,
        byte_length: u64,
        sha256: &str,
    ) -> ProductAttachmentRecord {
        ProductAttachmentRecord {
            attachment_id: attachment_id.clone(),
            product_session_id: session_id.clone(),
            content_type: "image/png".to_string(),
            byte_length,
            sha256: sha256.to_string(),
            status: ProductAttachmentStatus::Staged,
            display_name: None,
            created_at: "2026-09-26T00:00:00Z".to_string(),
            referenced_at: None,
            expires_at: None,
            scan_flags: vec![],
        }
    }

    /// The root is a sibling of the product database, never inside a workspace.
    #[test]
    fn the_root_is_a_sibling_of_the_product_database() {
        let root = attachments_root(Path::new("C:/data/rove/product.sqlite"));
        assert_eq!(root, Path::new("C:/data/rove").join(ATTACHMENTS_DIR));
        // A workspace root never appears in the derived path.
        assert!(!root.to_string_lossy().contains("workspaces"));
        // A relative database path keeps the attachments beside it.
        assert_eq!(
            attachments_root(Path::new("product.sqlite")),
            Path::new(ATTACHMENTS_DIR)
        );
    }

    /// A minimal PNG whose IHDR declares 2×3 pixels, so the real raster
    /// validator is exercised rather than bypassed.
    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes
    }

    fn gif_bytes() -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes
    }

    fn pdf_bytes() -> Vec<u8> {
        b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF\n".to_vec()
    }

    #[test]
    fn each_allowed_extension_resolves_its_locally_verified_type() {
        for (name, bytes, expected) in [
            ("shot.png", png_bytes(), "image/png"),
            ("anim.gif", gif_bytes(), "image/gif"),
            ("doc.pdf", pdf_bytes(), "application/pdf"),
            ("notes.txt", b"hello there".to_vec(), "text/plain"),
            ("readme.md", b"# title".to_vec(), "text/markdown"),
        ] {
            let validated = validate_attachment_upload(Some(name), None, &bytes)
                .unwrap_or_else(|error| panic!("{name} should validate: {}", error.status()));
            assert_eq!(validated.content_type, expected, "{name}");
            assert!(validated.warnings.is_empty(), "{name}");
        }
        // The extension is matched case-insensitively.
        assert!(validate_attachment_upload(Some("SHOT.PNG"), None, &png_bytes()).is_ok());
    }

    #[test]
    fn a_missing_or_unknown_extension_is_refused_before_any_byte_is_read() {
        for name in [
            "noext",
            "script.sh",
            "vector.svg",
            "page.html",
            "archive.tar",
        ] {
            let error = validate_attachment_upload(Some(name), None, &png_bytes()).unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                error.code(),
                ProductErrorCode::ProductAttachmentInvalidInput.as_str(),
                "{name}"
            );
        }
        // A name is required: without one there is no extension to allow.
        assert!(validate_attachment_upload(None, None, &png_bytes()).is_err());
    }

    #[test]
    fn an_extension_and_signature_mismatch_is_refused_in_both_directions() {
        // A png name over GIF bytes, and a pdf name over PNG bytes.
        assert!(
            validate_attachment_upload(Some("shot.png"), None, &gif_bytes()).is_err(),
            "png bytes must be PNG"
        );
        assert!(
            validate_attachment_upload(Some("doc.pdf"), None, &png_bytes()).is_err(),
            "pdf bytes must start with %PDF-"
        );
        // A raster name over bytes with no signature at all.
        assert!(validate_attachment_upload(Some("shot.webp"), None, b"not an image").is_err());
        // A pdf name over a zip signature is a refusal, not a mismatch.
        let zip = b"PK\x03\x04\x14\x00\x00\x00".to_vec();
        assert!(validate_attachment_upload(Some("doc.pdf"), None, &zip).is_err());
    }

    #[test]
    fn archives_and_executables_are_refused_as_positive_classifications() {
        // Signature bytes that are legal UTF-8 with no NUL byte, so the refusal
        // can only come from the signature classification and not from the
        // encoding check. This is what makes the test non-vacuous: were
        // `refuse_non_text_signature` removed from the text arms, every one of
        // these would be accepted and stored as `text/plain`.
        let zip = b"PK\x03\x04abcdef".to_vec();
        let zip_stored = b"PK\x03\x04\x14\x00\x00\x00".to_vec();
        let wasm = b"\0asm\x01\x00\x00\x00".to_vec();
        let gif = b"GIF89aABCDEFGH".to_vec();
        let pdf = b"%PDF-1.7\nplain ascii trailer".to_vec();
        let png = b"\x89PNG\r\n\x1a\nplain ascii trailer".to_vec();

        for name in ["notes.txt", "readme.md", "doc.pdf", "shot.png"] {
            let error = validate_attachment_upload(Some(name), None, &zip).unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
            let error = validate_attachment_upload(Some(name), None, &wasm).unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
        }
        assert_eq!(
            validate_attachment_upload(Some("notes.txt"), None, &zip)
                .unwrap_err()
                .code(),
            ProductErrorCode::ProductAttachmentInvalidInput.as_str()
        );

        // The NUL-bearing variants stay refused as well, now by the signature
        // rule rather than by the encoding rule alone.
        for bytes in [&zip_stored, &wasm] {
            for name in ["notes.txt", "readme.md"] {
                let error = validate_attachment_upload(Some(name), None, bytes).unwrap_err();
                assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
            }
        }

        // A text extension cannot carry a raster or PDF signature either: the
        // bytes are legal UTF-8, so only the classification refuses them.
        for bytes in [&gif, &pdf, &png] {
            for name in ["notes.txt", "readme.md"] {
                let error = validate_attachment_upload(Some(name), None, bytes).unwrap_err();
                assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
                assert_eq!(
                    error.code(),
                    ProductErrorCode::ProductAttachmentInvalidInput.as_str(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn a_text_extension_without_a_signature_is_accepted() {
        // The positive control for the refusals above: the same shape of bytes
        // with no signature at all is accepted, so the test above is measuring
        // the signature rule and not a blanket refusal of text attachments.
        for name in ["notes.txt", "readme.md"] {
            let accepted =
                validate_attachment_upload(Some(name), None, b"PK notes, not an archive")
                    .expect("plain text without a signature is accepted");
            assert!(accepted.content_type.starts_with("text/"), "{name}");
        }
        // `PK` alone is not the archive signature: the four-byte local file
        // header is, and the probe below must not be refused by a prefix match.
        assert!(
            validate_attachment_upload(Some("notes.txt"), None, b"PK\x03abc").is_ok(),
            "only the four-byte PK\\x03\\x04 header classifies as an archive"
        );
    }

    #[test]
    fn a_text_attachment_must_be_valid_utf8_without_a_nul_byte() {
        assert!(validate_attachment_upload(Some("notes.txt"), None, b"plain ascii").is_ok());
        // A NUL byte is refused even though the bytes are otherwise valid UTF-8.
        let error = validate_attachment_upload(Some("notes.txt"), None, b"a\0b").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        // Invalid UTF-8 is refused too.
        assert!(validate_attachment_upload(Some("notes.md"), None, &[0xff, 0xfe]).is_err());
    }

    #[test]
    fn an_oversized_raster_is_a_413_while_a_document_keeps_its_20_mib_ceiling() {
        let mut oversized = png_bytes();
        oversized.resize(MAX_PRODUCT_ATTACHMENT_RASTER_BYTES as usize + 1, 0);
        let error = validate_attachment_upload(Some("shot.png"), None, &oversized).unwrap_err();
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            error.code(),
            ProductErrorCode::ProductAttachmentTooLarge.as_str()
        );

        // A document of the same size is still inside its own ceiling, so the
        // refusal is about the raster limit, not about 16 MiB in general.
        let mut document = pdf_bytes();
        document.resize(MAX_PRODUCT_ATTACHMENT_RASTER_BYTES as usize + 1, b' ');
        assert_eq!(
            validate_attachment_upload(Some("doc.pdf"), None, &document)
                .expect("a document below 20 MiB is accepted")
                .content_type,
            "application/pdf"
        );
    }

    #[test]
    fn a_raster_over_the_pixel_cap_is_refused() {
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.extend_from_slice(&[0, 0, 0, 13]);
        huge.extend_from_slice(b"IHDR");
        huge.extend_from_slice(&20_000u32.to_be_bytes());
        huge.extend_from_slice(&20_000u32.to_be_bytes());

        let error = validate_attachment_upload(Some("shot.png"), None, &huge).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_secret_shaped_name_and_secret_shaped_content_are_warnings_not_refusals() {
        // Synthetic shapes only: no fixture here is a real credential.
        let named = validate_attachment_upload(Some(".env.txt"), None, b"harmless").unwrap();
        assert_eq!(
            named.warnings,
            vec![ATTACHMENT_WARNING_SECRET_NAME.to_string()]
        );

        let content =
            validate_attachment_upload(Some("notes.txt"), None, b"token=SYNTHETIC-CANARY-0001")
                .unwrap();
        assert_eq!(
            content.warnings,
            vec![ATTACHMENT_WARNING_SECRET_CONTENT.to_string()]
        );

        let mut both = validate_attachment_upload(
            Some("id_rsa.txt"),
            None,
            b"Authorization: Bearer SYNTHETIC-CANARY-0002",
        )
        .unwrap();
        both.warnings.sort();
        assert_eq!(
            both.warnings,
            vec![
                ATTACHMENT_WARNING_SECRET_CONTENT.to_string(),
                ATTACHMENT_WARNING_SECRET_NAME.to_string(),
            ]
        );

        // A binary payload is not scanned as text, so it cannot warn — and it
        // is never rewritten.
        let binary = validate_attachment_upload(Some("shot.png"), None, &png_bytes()).unwrap();
        assert!(binary.warnings.is_empty());
    }

    /// The warnings contract, pinned as a whole rather than spot-checked.
    ///
    /// Three properties together: the set of codes this build can emit is
    /// exactly the documented set; every code is a stable lowercase snake-case
    /// identifier; and no code carries any part of the upload — not the display
    /// name, not a byte of content, not a path. The last one is what makes the
    /// `warnings` array safe to put in an API response and in a screenshot.
    #[test]
    fn attachment_warning_codes_are_stable_and_secret_free() {
        let mut emitted = Vec::new();
        /// One `(reason, display name, claimed content type, raw bytes)` upload
        /// case for the warnings-contract table below.
        type UploadCase<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a [u8]);
        let cases: Vec<UploadCase<'_>> = vec![
            ("plain", Some("notes.txt"), None, b"harmless"),
            ("secret name", Some(".env.txt"), None, b"harmless"),
            (
                "secret content",
                Some("notes.txt"),
                None,
                b"token=SYNTHETIC-CANARY-0001",
            ),
            (
                "both",
                Some("id_rsa.txt"),
                None,
                b"Authorization: Bearer SYNTHETIC-CANARY-0002",
            ),
            (
                "claim mismatch",
                Some("notes.txt"),
                Some("application/pdf"),
                b"Authorization: Bearer SYNTHETIC-CANARY-0003",
            ),
        ];
        for (reason, name, claim, bytes) in cases {
            let accepted = validate_attachment_upload(name, claim, bytes)
                .unwrap_or_else(|_| panic!("{reason} must be accepted"));
            emitted.extend(accepted.warnings);
        }
        // Every code seen is in the published table, and every entry of the
        // table was actually produced by some accepted upload: the array is the
        // complete set, not an upper bound.
        for code in &emitted {
            assert!(
                ATTACHMENT_WARNING_CODES.contains(&code.as_str()),
                "{code} is not in the documented set"
            );
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{code} is not a stable snake-case identifier"
            );
            for forbidden in ["notes.txt", ".env.txt", "id_rsa.txt", "shot.png", "/", "\\"] {
                assert!(
                    !code.contains(forbidden),
                    "{code} carries {forbidden} into a client-visible warning"
                );
            }
        }
        for documented in ATTACHMENT_WARNING_CODES {
            assert!(
                emitted.iter().any(|code| code == documented),
                "{documented} is documented but unreachable"
            );
        }
    }

    #[test]
    fn a_content_type_claim_is_only_ever_a_claim() {
        let matching = validate_attachment_upload(Some("notes.txt"), Some("text/plain"), b"hi")
            .expect("a matching claim warns about nothing");
        assert!(matching.warnings.is_empty());

        let mismatched =
            validate_attachment_upload(Some("notes.txt"), Some("application/pdf"), b"hi").unwrap();
        assert_eq!(
            mismatched.warnings,
            vec![ATTACHMENT_WARNING_CONTENT_TYPE_MISMATCH.to_string()]
        );
        // The verified type is the local one regardless of the claim.
        assert_eq!(mismatched.content_type, "text/plain");
    }

    /// The claim comparison is on the media type, so a difference in case, a
    /// parameter, or the `text/plain`/`text/markdown` pair is not a warning —
    /// and a genuine disagreement still is.
    #[test]
    fn a_content_type_claim_is_compared_by_media_type() {
        let png = png_bytes();

        // Case, and a parameter, do not change the type.
        for claim in ["IMAGE/PNG", "image/png; charset=binary", " image/png "] {
            let accepted = validate_attachment_upload(Some("shot.png"), Some(claim), &png)
                .unwrap_or_else(|_| panic!("{claim} must be accepted"));
            assert!(
                accepted.warnings.is_empty(),
                "{claim} names the verified type and must not warn"
            );
            assert_eq!(accepted.content_type, "image/png");
        }

        // The text family is one type to a client: a `.md` served as
        // `text/markdown` is not contradicted by a `text/plain` claim, in
        // either direction.
        for (name, claim, verified) in [
            ("notes.md", "text/plain", "text/markdown"),
            ("notes.md", "text/markdown", "text/markdown"),
            ("notes.txt", "text/plain", "text/plain"),
            ("notes.txt", "TEXT/PLAIN", "text/plain"),
            ("notes.txt", "text/markdown", "text/plain"),
        ] {
            let accepted = validate_attachment_upload(Some(name), Some(claim), b"plain ascii")
                .unwrap_or_else(|_| panic!("{claim} on {name} must be accepted"));
            assert!(
                accepted.warnings.is_empty(),
                "{claim} on {name} must not warn"
            );
            assert_eq!(accepted.content_type, verified, "{name}");
        }

        // A disagreement that is not that pair still warns, in both directions.
        for (name, claim, body) in [
            ("shot.png", "image/jpeg", png.clone()),
            ("shot.png", "text/plain", png.clone()),
            ("notes.txt", "application/pdf", b"plain ascii".to_vec()),
            ("report.pdf", "text/plain", pdf_bytes()),
        ] {
            let warned = validate_attachment_upload(Some(name), Some(claim), &body)
                .unwrap_or_else(|_| panic!("{claim} on {name} must be accepted"));
            assert_eq!(
                warned.warnings,
                vec![ATTACHMENT_WARNING_CONTENT_TYPE_MISMATCH.to_string()],
                "{claim} on {name} disagrees with the verified type"
            );
        }
    }

    #[test]
    fn the_display_name_is_bounded_and_never_a_path() {
        assert_eq!(validate_display_name(None).unwrap(), None);
        assert_eq!(validate_display_name(Some(String::new())).unwrap(), None);
        assert_eq!(
            validate_display_name(Some("screenshot.png".to_string())).unwrap(),
            Some("screenshot.png".to_string())
        );

        for bad in [
            "line\nbreak.png",
            "tab\tname.png",
            "dir/name.png",
            "dir\\name.png",
            "..",
        ] {
            assert!(
                validate_display_name(Some(bad.to_string())).is_err(),
                "{bad:?} must be refused"
            );
        }

        let too_long = format!(
            "{}.png",
            "a".repeat(MAX_PRODUCT_ATTACHMENT_DISPLAY_NAME_BYTES)
        );
        assert!(validate_display_name(Some(too_long)).is_err());
        let at_limit = format!("{}.png", "a".repeat(250));
        assert!(validate_display_name(Some(at_limit)).is_ok());
    }

    #[test]
    fn a_raw_body_default_content_type_carries_no_claim() {
        let mut headers = HeaderMap::new();
        assert_eq!(client_content_type_claim(&headers), None);
        headers.insert(CONTENT_TYPE, "application/octet-stream".parse().unwrap());
        assert_eq!(client_content_type_claim(&headers), None);
        headers.insert(CONTENT_TYPE, "image/png; charset=binary".parse().unwrap());
        assert_eq!(client_content_type_claim(&headers), Some("image/png"));
        headers.insert(CONTENT_TYPE, "".parse().unwrap());
        assert_eq!(client_content_type_claim(&headers), None);
    }

    /// The empty payload is refused by the extension/type checks rather than
    /// stored as a zero-byte attachment.
    #[test]
    fn an_empty_body_is_refused() {
        for name in ["shot.png", "doc.pdf", "notes.txt"] {
            let error = validate_attachment_upload(Some(name), None, b"").unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
        }
    }

    /// The upload route is registered with a body limit of the document
    /// ceiling plus one byte, so an oversized `Content-Length` can be refused
    /// before the read.
    #[test]
    fn the_route_body_limit_is_one_byte_above_the_document_ceiling() {
        assert_eq!(
            rove_product_store::MAX_PRODUCT_ATTACHMENT_UPLOAD_BODY_BYTES as u64,
            MAX_PRODUCT_ATTACHMENT_DOCUMENT_BYTES + 1
        );
    }

    /// `join_safe`'s negatives are reused unchanged for the attachment path.
    #[test]
    fn crafted_identifiers_cannot_add_a_directory_level() {
        let dir = Path::new("C:/data/attachments/session");
        for bad in [
            "../escape",
            "..",
            "a/b",
            "nested/id",
            "/absolute",
            "C:\\windows\\system32",
            "",
            ".",
            ".env",
            "server.pem",
        ] {
            assert!(
                payload_path_in(dir, bad).is_err(),
                "expected {bad:?} to be refused"
            );
        }
        let ok = payload_path_in(dir, "01J8Z0M6Q3W9F2V7B4K1N5T8XR").unwrap();
        assert_eq!(ok, dir.join("01J8Z0M6Q3W9F2V7B4K1N5T8XR"));
        assert_eq!(
            part_path(&ok).unwrap(),
            dir.join("01J8Z0M6Q3W9F2V7B4K1N5T8XR.part")
        );
        // The stored name is exactly the id: no extension is appended.
        assert_eq!(ok.extension(), None);
    }

    /// A name this platform would reinterpret is refused by the helper that
    /// builds both the session directory and the payload path.
    ///
    /// This is the join defence's own non-skipping negative: the symbolic-link
    /// escapes need a privilege on Windows, so they self-skip here. It is also
    /// read by CI's Linux runner, which is why the two lists below are split by
    /// what actually holds on each platform rather than by severity — a bare UNC
    /// or `\\.\` prefix is Windows path syntax, and on Unix those bytes are one
    /// ordinary file name that `join_safe` still keeps inside the session
    /// directory.
    ///
    /// Platform coverage, case by case:
    /// - refused everywhere (this list runs in CI's Linux job and on Windows):
    ///   `\\?\C:\…` (its `:` is an alternate data stream), `C:`,
    ///   `payload:hidden`, the reserved device stems with and without an
    ///   extension, a trailing dot, space, or dot-space, `..` and `../escape`,
    ///   a two-component `a/b`, an absolute `/absolute`, and the empty string;
    /// - refused on Windows only, asserted under `cfg(windows)`: the bare
    ///   `\\server\share`, `\\?\UNC\server\share`, and `\\.\NUL` prefixes,
    ///   which `Path` reports as a prefix instead of one `Normal` component.
    #[test]
    fn a_component_this_platform_would_reinterpret_is_refused() {
        let dir = Path::new("C:/data/attachments/session");
        // Every entry has to be refused by both helpers on both platforms; CI
        // caught an earlier version of this list keeping the colon-free Windows
        // prefixes here, where Unix reads them as a single legal file name.
        for bad in [
            // `\\?\C:\windows\system32`: a verbatim-disk prefix on Windows, a
            // colon-bearing single component on Unix. Refused on both.
            "\\\\?\\C:\\windows\\system32",
            "C:",
            "payload:hidden",
            "CON",
            "con",
            "NUL.txt",
            "COM1",
            "LPT9.log",
            "AUX",
            "payload.",
            "payload ",
            "payload. ",
            "..",
            "../escape",
            "a/b",
            "/absolute",
            "",
        ] {
            assert!(
                payload_path_in(dir, bad).is_err(),
                "expected payload id {bad:?} to be refused on this platform"
            );
            assert!(
                session_dir_in(dir, bad).is_err(),
                "expected session id {bad:?} to be refused on this platform"
            );
        }

        // A name Windows would reinterpret but Unix would not: asserted only
        // where `Path` reports the prefix, so a Linux run does not claim a
        // refusal that its filesystem does not need.
        #[cfg(windows)]
        for bad in [
            "\\\\?\\UNC\\server\\share",
            "\\\\server\\share",
            "\\\\.\\NUL",
        ] {
            assert!(
                payload_path_in(dir, bad).is_err(),
                "expected Windows prefix {bad:?} to be refused"
            );
            assert!(
                session_dir_in(dir, bad).is_err(),
                "expected Windows prefix {bad:?} to be refused"
            );
        }

        // The positive control: a ULID passes both helpers unchanged.
        let ulid = "01J8Z0M6Q3W9F2V7B4K1N5T8XR";
        assert!(payload_path_in(dir, ulid).is_ok());
        assert_eq!(session_dir_in(dir, ulid).unwrap(), dir.join(ulid));
    }

    #[test]
    fn a_vanished_row_is_a_404_while_an_intact_row_without_bytes_is_a_410() {
        let intact = unavailable_attachment_error(true);
        assert_eq!(intact.status(), StatusCode::GONE);
        assert_eq!(
            intact.code(),
            ProductErrorCode::ProductAttachmentUnavailable.as_str()
        );

        let vanished = unavailable_attachment_error(false);
        assert_eq!(vanished.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            vanished.code(),
            ProductErrorCode::ProductAttachmentNotFound.as_str()
        );
    }

    #[tokio::test]
    async fn a_payload_round_trips_with_the_written_length_and_digest() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let bytes = b"\x89PNG\r\n\x1a\n-a-png-body".to_vec();

        let written = storage
            .write_payload(&session_id, &attachment_id, &bytes)
            .await
            .unwrap();
        assert_eq!(written.byte_length, bytes.len() as u64);

        // The bytes are at the derived path, with no extension, and the
        // transient partial file is gone.
        let payload = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A)
            .join(attachment_id.as_str());
        assert_eq!(std::fs::read(&payload).unwrap(), bytes);
        assert!(!part_path(&payload).unwrap().exists());

        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Available
        );
        let verified_path = verified.path.expect("a verified payload has a path");
        // The name is exactly the id: no extension is appended anywhere.
        assert_eq!(
            verified_path.file_name().and_then(|name| name.to_str()),
            Some(attachment_id.as_str())
        );
        assert_eq!(
            std::fs::read(&verified_path).unwrap(),
            bytes,
            "the served path must hold the written bytes"
        );
    }

    #[tokio::test]
    async fn a_missing_payload_is_reported_rather_than_created() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();

        let verified = storage
            .verify_payload(&record(&session_id, &attachment_id, 4, "00"))
            .await
            .unwrap();

        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Missing
        );
        assert!(verified.path.is_none());
        // A read never creates the tree it looked in.
        assert!(!storage_root(&temp).exists());
    }

    #[tokio::test]
    async fn a_dangling_payload_is_missing_and_a_truncated_one_is_corrupt() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let bytes = b"twelve bytes".to_vec();
        let written = storage
            .write_payload(&session_id, &attachment_id, &bytes)
            .await
            .unwrap();
        let payload = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A)
            .join(attachment_id.as_str());

        // Deleted outright: the row outlives its bytes.
        std::fs::remove_file(&payload).unwrap();
        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Missing
        );

        // Present but replaced with different bytes of the right length.
        std::fs::write(&payload, b"other  bytes").unwrap();
        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Corrupt
        );

        // Present and byte-identical, but shorter than the record claims.
        std::fs::write(&payload, &bytes).unwrap();
        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length + 1,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Corrupt
        );
    }

    #[tokio::test]
    async fn an_expired_row_never_touches_the_disk() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let mut expired = record(&session_id, &attachment_id, 4, "00");
        expired.status = ProductAttachmentStatus::Expired;

        let verified = storage.verify_payload(&expired).await.unwrap();

        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Expired
        );
        assert!(!storage_root(&temp).exists());
    }

    #[tokio::test]
    async fn a_session_directory_that_escapes_the_root_is_refused() {
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let root = temp.path().join("data").join(ATTACHMENTS_DIR);
        std::fs::create_dir_all(&root).unwrap();
        // A same-named directory that is really a link out of the tree.
        if !try_symlink_dir(&outside, &root.join(SESSION_A)) {
            return;
        }
        let storage = AttachmentStorage::new(root);

        let error = storage
            .write_payload(&session(SESSION_A), &ProductAttachmentId::new(), b"x")
            .await
            .unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_symlinked_payload_is_never_read_as_bytes() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let bytes = b"payload bytes".to_vec();
        let attachment_id = ProductAttachmentId::new();
        let written = storage
            .write_payload(&session_id, &attachment_id, &bytes)
            .await
            .unwrap();
        let payload = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A)
            .join(attachment_id.as_str());
        // Replace the payload with a link to a file outside the attachment
        // root that happens to have the same bytes and length.
        let secret = temp.path().join("outside-secret.bin");
        std::fs::write(&secret, &bytes).unwrap();
        std::fs::remove_file(&payload).unwrap();
        if !try_symlink_file(&secret, &payload) {
            return;
        }

        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();

        assert_ne!(
            verified.availability,
            ProductAttachmentAvailability::Available,
            "a symbolic link must never be served as a payload"
        );
        assert!(verified.path.is_none());
    }

    #[tokio::test]
    async fn one_session_cannot_read_another_sessions_directory() {
        let (temp, storage) = storage();
        let bytes = b"session-a bytes".to_vec();
        let attachment_id = ProductAttachmentId::new();
        let written = storage
            .write_payload(&session(SESSION_A), &attachment_id, &bytes)
            .await
            .unwrap();
        let payload = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A)
            .join(attachment_id.as_str());

        // The same id under another session names nothing.
        let verified = storage
            .verify_payload(&record(
                &session(SESSION_B),
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Missing
        );
        // The other session's read did not create a directory for itself.
        assert!(
            !temp
                .path()
                .join("data")
                .join(ATTACHMENTS_DIR)
                .join(SESSION_B)
                .exists()
        );
        assert!(payload.exists());
    }

    #[tokio::test]
    async fn removing_a_payload_leaves_nothing_behind_and_tolerates_absence() {
        let (_temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        storage
            .write_payload(&session_id, &attachment_id, b"bytes")
            .await
            .unwrap();

        storage
            .remove_payload(&session_id, &attachment_id)
            .await
            .unwrap();
        let verified = storage
            .verify_payload(&record(&session_id, &attachment_id, 5, "00"))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Missing
        );
        // Removing something that is not there is not an error.
        storage
            .remove_payload(&session_id, &attachment_id)
            .await
            .unwrap();
        storage
            .remove_payload(&session(SESSION_B), &attachment_id)
            .await
            .unwrap();
    }

    /// A request that stops before its row is committed — the route deadline, a
    /// dropped connection, a panic — leaves no byte behind, because the guard
    /// removes what the id created when it is dropped.
    #[tokio::test]
    async fn an_abandoned_upload_removes_the_bytes_it_had_already_written() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let directory = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A);

        let cleanup = storage
            .guard_upload(&session_id, &attachment_id)
            .await
            .unwrap();
        storage
            .write_payload(&session_id, &attachment_id, b"abandoned bytes")
            .await
            .unwrap();
        let payload = directory.join(attachment_id.as_str());
        assert!(payload.exists());

        // Dropping the guard is what a cancelled request does.
        drop(cleanup);
        assert!(
            !payload.exists(),
            "an abandoned upload must not leave its payload"
        );
        assert_eq!(
            directory_entries(&directory),
            0,
            "the session tree is clean"
        );
    }

    /// A partial file left by a write that could not be cleaned up is removed by
    /// the next upload in the same session, and a published payload is not.
    #[tokio::test]
    async fn a_stale_partial_file_is_reclaimed_without_touching_a_payload() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let directory = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A);
        storage
            .write_payload(&session_id, &attachment_id, b"published")
            .await
            .unwrap();
        std::fs::write(directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XR.part"), b"stale").unwrap();
        std::fs::write(directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XS.part"), b"stale").unwrap();

        // A zero threshold makes every existing partial file stale, so the
        // production age rule is exercised without waiting a minute.
        let removed = storage
            .reclaim_stale_parts(&session_id, Duration::ZERO, MAX_STALE_PART_SCAN)
            .await
            .unwrap();
        assert_eq!(removed, 2);
        assert!(!directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XR.part").exists());
        assert!(!directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XS.part").exists());
        assert!(
            directory.join(attachment_id.as_str()).exists(),
            "a published payload is not a partial file and is never reclaimed here"
        );

        // A partial file that is younger than the threshold survives, so a live
        // concurrent write in the same session is not deleted under it.
        std::fs::write(directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XT.part"), b"live").unwrap();
        let removed = storage
            .reclaim_stale_parts(&session_id, Duration::from_secs(3_600), MAX_STALE_PART_SCAN)
            .await
            .unwrap();
        assert_eq!(removed, 0);
        assert!(directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XT.part").exists());
    }

    /// The scan is bounded, so a directory that keeps growing cannot turn one
    /// upload into an unbounded walk.
    ///
    /// The cap is asserted as a *bound*, not as an exact count: `read_dir` order
    /// is unspecified and differs between Windows and the Linux CI runner, so a
    /// scan that happens to meet the published payload first reclaims fewer
    /// entries than one that meets two partials. What must hold on every
    /// filesystem is that no scan examines more than the cap, that a partial the
    /// cap deferred is reclaimed by a later scan, and that the published payload
    /// is never one of them.
    #[tokio::test]
    async fn the_stale_partial_scan_examines_at_most_its_cap() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let directory = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        storage
            .write_payload(&session_id, &attachment_id, b"published")
            .await
            .unwrap();
        let parts: Vec<String> = (0..4)
            .map(|index| format!("01J8Z0M6Q3W9F2V7B4K1N5T8{index}.part"))
            .collect();
        for part in &parts {
            std::fs::write(directory.join(part), b"x").unwrap();
        }

        // Six entries (four partials, one payload, one lock-free directory
        // entry) against a two-entry cap, so one scan cannot finish the job.
        let mut removed_total = 0;
        let mut scans = 0;
        while scans < 8 {
            let removed = storage
                .reclaim_stale_parts(&session_id, Duration::ZERO, 2)
                .await
                .unwrap();
            assert!(
                removed <= 2,
                "one scan reclaimed {removed} entries with a cap of 2"
            );
            scans += 1;
            removed_total += removed;
            if removed == 0 {
                break;
            }
        }

        assert_eq!(
            removed_total, 4,
            "the cap delays reclamation instead of losing it"
        );
        assert!(
            scans < 8,
            "reclamation stalled: {removed_total} reclaimed over {scans} scans"
        );
        for part in &parts {
            assert!(
                !directory.join(part).exists(),
                "{part} survived the scans that were meant to reclaim it"
            );
        }
        assert!(
            directory.join(attachment_id.as_str()).exists(),
            "the published payload was reclaimed as if it were a partial file"
        );
    }

    /// The positive control for the guard: a committed row keeps its bytes.
    #[tokio::test]
    async fn a_disarmed_guard_keeps_the_payload_it_was_armed_for() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let mut cleanup = storage
            .guard_upload(&session_id, &attachment_id)
            .await
            .unwrap();
        let written = storage
            .write_payload(&session_id, &attachment_id, b"committed bytes")
            .await
            .unwrap();
        cleanup.disarm();

        let verified = storage
            .verify_payload(&record(
                &session_id,
                &attachment_id,
                written.byte_length,
                &written.sha256,
            ))
            .await
            .unwrap();
        assert_eq!(
            verified.availability,
            ProductAttachmentAvailability::Available
        );
        let payload = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A)
            .join(attachment_id.as_str());
        assert!(payload.exists(), "a disarmed guard removes nothing");
    }

    /// The guard names the transient file too, so a write interrupted before its
    /// rename leaves nothing.
    #[tokio::test]
    async fn an_abandoned_upload_removes_a_partial_file_that_never_published() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let attachment_id = ProductAttachmentId::new();
        let cleanup = storage
            .guard_upload(&session_id, &attachment_id)
            .await
            .unwrap();
        let directory = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A);
        let part = directory.join(format!("{}{}", attachment_id.as_str(), PART_SUFFIX));
        std::fs::write(&part, b"half written").unwrap();
        assert!(part.exists());

        drop(cleanup);
        assert!(!part.exists(), "the transient file is removed on drop");
        assert_eq!(directory_entries(&directory), 0);
    }

    /// The upload path itself reclaims: a partial file older than the upload
    /// deadline is gone after the next upload in the same session, without any
    /// caller asking for a scan. This pins the production wiring, not just the
    /// scan helper.
    #[tokio::test]
    async fn the_upload_path_reclaims_what_an_interrupted_upload_left() {
        let (temp, storage) = storage();
        let session_id = session(SESSION_A);
        let directory = temp
            .path()
            .join("data")
            .join(ATTACHMENTS_DIR)
            .join(SESSION_A);
        storage
            .write_payload(&session_id, &ProductAttachmentId::new(), b"first")
            .await
            .unwrap();

        // Older than `PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS`, so the real
        // production threshold — not a test override — decides.
        let orphan = directory.join("01J8Z0M6Q3W9F2V7B4K1N5T8XO.part");
        std::fs::write(&orphan, b"interrupted").unwrap();
        let long_ago = std::time::SystemTime::now()
            .checked_sub(Duration::from_secs(
                PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS + 60,
            ))
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&orphan)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        let next = ProductAttachmentId::new();
        storage
            .write_payload(&session_id, &next, b"second")
            .await
            .unwrap();

        assert!(
            !orphan.exists(),
            "the next upload in the session reclaims a stale partial file"
        );
        assert!(directory.join(next.as_str()).exists());
    }

    /// One product database and its payload root together, so a cleanup run is
    /// exercised against the rows and the bytes that actually belong to each
    /// other rather than against a mock of either.
    struct CleanupFixture {
        temp: TempDir,
        store: std::sync::Arc<dyn crate::product::ProductStore>,
        storage: AttachmentStorage,
        session_id: ProductSessionId,
    }

    impl CleanupFixture {
        async fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let storage = AttachmentStorage::new(storage_root(&temp));
            let store = crate::product::store::open_product_store(
                temp.path().join("product.sqlite"),
                5_000,
            )
            .unwrap();
            let workspace_root = temp.path().join("workspace");
            std::fs::create_dir_all(&workspace_root).unwrap();
            let workspace = store
                .create_workspace(crate::product::CreateProductWorkspaceRequest {
                    root: std::fs::canonicalize(&workspace_root).unwrap(),
                    kind: crate::product::ProductWorkspaceKind::Folder,
                    display_name: None,
                    pinned: false,
                })
                .await
                .unwrap();
            let session = store
                .create_session(crate::product::CreateProductSessionRequest {
                    workspace_id: workspace.id,
                    title: Some("cleanup".to_string()),
                })
                .await
                .unwrap();
            Self {
                temp,
                store,
                storage,
                session_id: session.id,
            }
        }

        /// Publish a real payload and its row together: the state a completed
        /// upload leaves behind.
        async fn staged(&self, bytes: &[u8]) -> ProductAttachmentId {
            let id = ProductAttachmentId::new();
            self.storage
                .write_payload(&self.session_id, &id, bytes)
                .await
                .unwrap();
            self.store
                .create_staged_attachment(
                    &self.session_id,
                    CreateStagedAttachmentRequest {
                        attachment_id: id.clone(),
                        content_type: "text/plain".to_string(),
                        byte_length: bytes.len() as u64,
                        sha256: sha256_bytes(bytes),
                        display_name: None,
                        scan_flags: Vec::new(),
                    },
                )
                .await
                .unwrap();
            id
        }

        /// Move a row's TTL into the past, so the next run is the one that
        /// reclaims it. Direct SQL is the point: a test must not wait 24 hours.
        fn move_ttl_into_the_past(&self, id: &ProductAttachmentId) {
            let connection =
                rusqlite::Connection::open(self.temp.path().join("product.sqlite")).unwrap();
            connection
                .execute(
                    "UPDATE product_attachments
                     SET expires_at = '2000-01-01T00:00:00.000Z'
                     WHERE attachment_id = ?1",
                    rusqlite::params![id.as_str()],
                )
                .unwrap();
        }

        fn status_of(&self, id: &ProductAttachmentId) -> String {
            let connection =
                rusqlite::Connection::open(self.temp.path().join("product.sqlite")).unwrap();
            connection
                .query_row(
                    "SELECT status FROM product_attachments WHERE attachment_id = ?1",
                    rusqlite::params![id.as_str()],
                    |row| row.get(0),
                )
                .unwrap()
        }

        /// Reference an attachment from a durable message: the transition that
        /// makes a payload permanent.
        async fn reference(&self, id: &ProductAttachmentId) {
            self.store
                .create_message(
                    &self.session_id,
                    crate::product::CreateProductMessageRequest {
                        content: "keep this".to_string(),
                        idempotency_key: None,
                        attachments: vec![crate::product::ProductMessageAttachmentRequest {
                            attachment_id: id.clone(),
                            name: None,
                        }],
                    },
                )
                .await
                .unwrap();
        }

        fn payload(&self, id: &ProductAttachmentId) -> std::path::PathBuf {
            storage_root(&self.temp)
                .join(self.session_id.as_str())
                .join(id.as_str())
        }

        /// Age one directory entry past the upload deadline, which is what makes
        /// reclamation safe rather than racy.
        fn age_past_the_upload_deadline(&self, path: &Path) {
            let long_ago = std::time::SystemTime::now()
                - Duration::from_secs(2 * PRODUCT_ATTACHMENT_UPLOAD_DEADLINE_SECONDS);
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(long_ago).unwrap();
        }
    }

    fn tight_limits(rows: usize, sessions: usize, entries: usize) -> AttachmentCleanupLimits {
        AttachmentCleanupLimits {
            rows,
            sessions,
            entries_per_session: entries,
        }
    }

    #[tokio::test]
    async fn a_cleanup_run_removes_an_expired_payload_and_keeps_a_referenced_one() {
        let fixture = CleanupFixture::new().await;
        let expiring = fixture.staged(b"reclaim me").await;
        let kept = fixture.staged(b"keep me").await;
        fixture.reference(&kept).await;
        fixture.move_ttl_into_the_past(&expiring);
        // The referenced row keeps a TTL in the past only if the promotion did
        // not clear it; clearing it is exactly what this exercises.
        fixture.move_ttl_into_the_past(&kept);

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            AttachmentCleanupLimits::default(),
        )
        .await;

        assert_eq!(
            outcome.expired_rows, 1,
            "only the staged row is a candidate"
        );
        assert_eq!(outcome.removed_expired_payloads, 1);
        assert_eq!(outcome.failures, 0);
        assert_eq!(fixture.status_of(&expiring), "expired");
        assert!(
            !fixture.payload(&expiring).exists(),
            "an expired payload is gone, so no message can be served from it"
        );
        assert_eq!(fixture.status_of(&kept), "referenced");
        assert!(
            fixture.payload(&kept).exists(),
            "a referenced attachment is kept regardless of age"
        );
    }

    #[tokio::test]
    async fn a_cleanup_run_is_bounded_and_still_makes_progress() {
        let fixture = CleanupFixture::new().await;
        let mut ids = Vec::new();
        for byte in [b"a".as_slice(), b"b", b"c"] {
            let id = fixture.staged(byte).await;
            fixture.move_ttl_into_the_past(&id);
            ids.push(id);
        }

        // One row per run: three bounded runs reclaim three rows, and no run does
        // more work than the bound it was given.
        for expected_run in 1..=3 {
            let outcome = run_attachment_cleanup(
                fixture.store.as_ref(),
                &fixture.storage,
                tight_limits(1, 4, 4),
            )
            .await;
            assert_eq!(
                outcome.expired_rows, 1,
                "run {expected_run} reclaimed one row, not the whole directory"
            );
        }
        for id in &ids {
            assert_eq!(fixture.status_of(id), "expired");
            assert!(!fixture.payload(id).exists());
        }
        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            tight_limits(4, 4, 4),
        )
        .await;
        assert_eq!(outcome.expired_rows, 0, "nothing is left to reclaim");
    }

    #[tokio::test]
    async fn an_aged_orphan_is_reclaimed_and_a_row_backed_payload_is_not() {
        let fixture = CleanupFixture::new().await;
        let staged = fixture.staged(b"still waiting").await;
        let referenced = fixture.staged(b"in use").await;
        fixture.reference(&referenced).await;

        // An interrupted upload: the payload was published but its row was never
        // committed, which is the only shape that has no owner.
        let orphan = fixture.staged(b"orphan").await;
        let connection =
            rusqlite::Connection::open(fixture.temp.path().join("product.sqlite")).unwrap();
        connection
            .execute(
                "DELETE FROM product_attachments WHERE attachment_id = ?1",
                rusqlite::params![orphan.as_str()],
            )
            .unwrap();
        fixture.age_past_the_upload_deadline(&fixture.payload(&orphan));

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            AttachmentCleanupLimits::default(),
        )
        .await;

        assert_eq!(outcome.removed_orphan_payloads, 1);
        assert_eq!(outcome.failures, 0);
        assert!(!fixture.payload(&orphan).exists());
        assert!(
            fixture.payload(&staged).exists(),
            "a staged payload is never reclaimed by the orphan scan, whatever its age"
        );
        assert!(
            fixture.payload(&referenced).exists(),
            "a referenced payload is never a candidate"
        );
        assert_eq!(fixture.status_of(&staged), "staged");
    }

    #[tokio::test]
    async fn a_young_unreferenced_entry_survives_because_an_upload_may_still_own_it() {
        let fixture = CleanupFixture::new().await;
        // The same shape as the previous test, but published a moment ago: this
        // is the window between a publish and its row commit, and removing it
        // would delete the bytes of an upload that is about to succeed.
        let orphan = fixture.staged(b"just published").await;

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            AttachmentCleanupLimits::default(),
        )
        .await;

        assert_eq!(outcome.removed_orphan_payloads, 0);
        assert!(fixture.payload(&orphan).exists());
    }

    #[tokio::test]
    async fn a_cleanup_run_reclaims_a_stale_partial_and_removes_a_deleted_sessions_payloads() {
        let fixture = CleanupFixture::new().await;
        let id = fixture.staged(b"payload").await;
        // A partial file left by an upload that died before it published.
        let partial = fixture
            .payload(&id)
            .with_file_name(format!("{}{PART_SUFFIX}", ProductAttachmentId::new()));
        std::fs::write(&partial, b"half a file").unwrap();
        fixture.age_past_the_upload_deadline(&partial);

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            AttachmentCleanupLimits::default(),
        )
        .await;
        assert_eq!(outcome.reclaimed_partials, 1);
        assert!(!partial.exists());
        assert!(fixture.payload(&id).exists());

        // A session delete cascades the rows and then the bytes; what the delete
        // leaves behind is exactly what the orphan scan reclaims.
        fixture
            .store
            .delete_session(&fixture.session_id)
            .await
            .unwrap();
        let session_directory = storage_root(&fixture.temp).join(fixture.session_id.as_str());
        assert_eq!(directory_entries(&session_directory), 1);
        fixture.age_past_the_upload_deadline(&fixture.payload(&id));

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            AttachmentCleanupLimits::default(),
        )
        .await;
        assert_eq!(outcome.removed_orphan_payloads, 1);
        assert_eq!(
            directory_entries(&session_directory),
            0,
            "the residue of a deleted session is reclaimed"
        );
    }

    #[tokio::test]
    async fn a_cleanup_run_with_a_zero_bound_does_nothing() {
        let fixture = CleanupFixture::new().await;
        let id = fixture.staged(b"payload").await;
        fixture.move_ttl_into_the_past(&id);

        let outcome = run_attachment_cleanup(
            fixture.store.as_ref(),
            &fixture.storage,
            tight_limits(0, 0, 0),
        )
        .await;

        assert_eq!(outcome, AttachmentCleanupOutcome::default());
        assert_eq!(fixture.status_of(&id), "staged");
        assert!(fixture.payload(&id).exists());
    }

    #[cfg(unix)]
    fn try_symlink_dir(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn try_symlink_dir(target: &Path, link: &Path) -> bool {
        // Windows needs a privilege or developer mode for directory links; the
        // test skips rather than failing when it cannot create one.
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(unix)]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}
