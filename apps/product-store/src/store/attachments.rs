//! The durable attachment row layer.
//!
//! SQLite is authoritative for attachment metadata; the payload bytes live at
//! `<data_root>/attachments/<product_session_id>/<attachment_id>` and are not
//! content-addressed. This module owns identity, the staged TTL, the
//! per-session quotas, and the session-scoped read. It owns no filesystem
//! behaviour: publishing bytes happens before a row is created, and the caller
//! removes them if this layer refuses.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    CreateStagedAttachmentRequest, MAX_REFERENCED_ATTACHMENT_BYTES_PER_SESSION,
    MAX_REFERENCED_ATTACHMENTS_PER_SESSION, MAX_STAGED_ATTACHMENT_BYTES_PER_SESSION,
    MAX_STAGED_ATTACHMENTS_PER_SESSION, PRODUCT_ATTACHMENT_STAGED_TTL_SECONDS,
    ProductAttachmentAvailability, ProductAttachmentId, ProductAttachmentRecord,
    ProductAttachmentStatus, ProductErrorCode, ProductMessageAttachmentRef, ProductSessionId,
    ProductSessionStatus, ProductStoreError,
};
use rove_runtime::conversation::{
    MAX_ATTACHMENT_DISPLAY_NAME_BYTES, MAX_MESSAGE_ATTACHMENTS, MessageAttachmentRef,
};

use super::repository::{get_session, immediate_transaction, now_rfc3339};
use super::schema::storage_error;

const ATTACHMENT_COLUMNS: &str = "attachment_id, product_session_id, content_type, byte_length,
     sha256, status, display_name, created_at, referenced_at, expires_at, scan_flags";

impl super::repository::ProductRepository {
    /// Resolve the session an upload will belong to, without touching disk.
    pub(super) fn resolve_attachment_session(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<(), ProductStoreError> {
        let connection = self.database.connect()?;
        require_attachment_session(&connection, session_id)
    }

    /// Publish a staged attachment row inside one `IMMEDIATE` transaction that
    /// also evaluates the quotas, so the last free slot cannot be granted twice.
    pub(super) fn create_staged_attachment(
        &self,
        session_id: &ProductSessionId,
        request: CreateStagedAttachmentRequest,
    ) -> Result<ProductAttachmentRecord, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        // Re-resolved inside the transaction: the session may have been
        // archived (or deleted) between the pre-flight check and this write.
        require_attachment_session(&transaction, session_id)?;

        let byte_length = i64::try_from(request.byte_length).map_err(storage_error)?;
        let budget = session_attachment_budget(&transaction, session_id)?;
        budget.admits_staged(byte_length)?;

        let attachment_id = request.attachment_id.clone();
        let created_at = now_rfc3339();
        let expires_at = rfc3339_in_seconds(PRODUCT_ATTACHMENT_STAGED_TTL_SECONDS);
        let scan_flags = serde_json::to_string(&request.scan_flags).map_err(storage_error)?;
        transaction
            .execute(
                r#"
                INSERT INTO product_attachments(
                    attachment_id, product_session_id, content_type, byte_length,
                    sha256, status, display_name, created_at, referenced_at,
                    expires_at, scan_flags
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'staged', ?6, ?7, NULL, ?8, ?9)
                "#,
                params![
                    attachment_id.as_str(),
                    session_id.to_string(),
                    request.content_type,
                    byte_length,
                    request.sha256,
                    request.display_name,
                    created_at,
                    expires_at,
                    scan_flags,
                ],
            )
            .map_err(storage_error)?;
        let record = attachment_in_transaction(&transaction, session_id, &attachment_id)?
            .ok_or_else(|| storage_error("published attachment row disappeared"))?;
        transaction.commit().map_err(storage_error)?;
        Ok(record)
    }

    /// Read one attachment, always scoped to its session.
    pub(super) fn attachment_for_session(
        &self,
        session_id: &ProductSessionId,
        attachment_id: &ProductAttachmentId,
    ) -> Result<Option<ProductAttachmentRecord>, ProductStoreError> {
        let connection = self.database.connect()?;
        attachment_in_transaction(&connection, session_id, attachment_id)
    }

    /// Move staged attachments whose TTL has passed to `expired`.
    ///
    /// Only the rows this call actually moved are returned, and they are the
    /// only payloads the cleanup job may unlink. A row a concurrent send
    /// promoted to `referenced` between the read and the update is not in the
    /// result — the `status = 'staged'` predicate and the returned change set
    /// agree because both are decided by the same `UPDATE`.
    pub(super) fn expire_staged_attachments(
        &self,
        limit: usize,
    ) -> Result<Vec<ProductAttachmentRecord>, ProductStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now_rfc3339_value = now_rfc3339();
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let limit = i64::try_from(limit).map_err(storage_error)?;
        // Selected first, then moved by id, rather than one `UPDATE … RETURNING`:
        // the row set and the change set have to be the same set, and an
        // `IMMEDIATE` transaction is what makes the two statements one decision.
        let candidates = transaction
            .prepare(&format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM product_attachments
                 WHERE status = 'staged' AND expires_at IS NOT NULL AND expires_at <= ?1
                 ORDER BY expires_at ASC, attachment_id ASC
                 LIMIT ?2"
            ))
            .map_err(storage_error)?
            .query_map(params![now_rfc3339_value, limit], raw_attachment_from_row)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let mut expired = Vec::with_capacity(candidates.len());
        for raw in candidates {
            let record = raw.into_product()?;
            let updated = transaction
                .execute(
                    "UPDATE product_attachments SET status = 'expired'
                     WHERE product_session_id = ?1 AND attachment_id = ?2 AND status = 'staged'",
                    params![
                        record.product_session_id.to_string(),
                        record.attachment_id.as_str()
                    ],
                )
                .map_err(storage_error)?;
            if updated == 1 {
                expired.push(record);
            }
        }
        transaction.commit().map_err(storage_error)?;
        Ok(expired)
    }

    /// The `(attachment id, status)` pairs of one session, bounded by `limit`.
    pub(super) fn attachment_statuses_for_session(
        &self,
        session_id: &ProductSessionId,
        limit: usize,
    ) -> Result<Vec<(ProductAttachmentId, ProductAttachmentStatus)>, ProductStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.database.connect()?;
        let limit = i64::try_from(limit).map_err(storage_error)?;
        let rows = connection
            .prepare(
                r#"
                SELECT attachment_id, status FROM product_attachments
                WHERE product_session_id = ?1
                ORDER BY attachment_id ASC
                LIMIT ?2
                "#,
            )
            .map_err(storage_error)?
            .query_map(params![session_id.to_string(), limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let mut pairs = Vec::with_capacity(rows.len());
        for (id, status) in rows {
            pairs.push((
                id.parse()
                    .map_err(|_| corrupt("persisted attachment id is invalid"))?,
                ProductAttachmentStatus::from_db(&status)?,
            ));
        }
        Ok(pairs)
    }
}

/// One attachment reference exactly as it is persisted on its message row.
///
/// Deliberately minimal: the verified type, length, and digest are *not* copied
/// here. A copy would be a second authority that a cleanup or a re-upload could
/// contradict, and the transcript would then report a type the payload no longer
/// has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedMessageAttachment {
    pub(super) attachment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
}

/// Serialize a message's attachment list for its row.
///
/// An empty list is stored as SQL `NULL`, not `'[]'`, so a message written
/// before migration 022 and a message written with no attachments are the same
/// durable state and read back the same way.
pub(super) fn encode_message_attachments(
    attachments: &[PersistedMessageAttachment],
) -> Result<Option<String>, ProductStoreError> {
    if attachments.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(attachments)
        .map(Some)
        .map_err(storage_error)
}

/// Read a message row's attachment list.
pub(super) fn decode_message_attachments(
    value: Option<&str>,
) -> Result<Vec<PersistedMessageAttachment>, ProductStoreError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(value).map_err(|_| corrupt("persisted message attachments are invalid"))
}

/// Turn a message's attachment references into durable references.
///
/// Runs inside the caller's `IMMEDIATE` transaction, before the message row
/// exists, so a refusal leaves neither a message nor a promoted attachment and a
/// success leaves a message whose every reference is already `referenced`.
///
/// Refusals are typed and distinct, because they are different conditions for
/// the caller: an id the session does not hold is `product_attachment_not_found`
/// (404, and indistinguishable from another session's id), an expired row is
/// `product_attachment_conflict` (409, "upload it again"), and a reference that
/// would cross either referenced ceiling is `product_attachment_quota` (409).
pub(super) fn reference_message_attachments(
    connection: &Connection,
    session_id: &ProductSessionId,
    attachments: &[MessageAttachmentRef],
) -> Result<Vec<PersistedMessageAttachment>, ProductStoreError> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    if attachments.len() > MAX_MESSAGE_ATTACHMENTS {
        return Err(invalid_attachment("too many attachments on one message"));
    }
    for attachment in attachments {
        if attachment.name.as_ref().is_some_and(|name| {
            name.len() > MAX_ATTACHMENT_DISPLAY_NAME_BYTES
                || name.chars().any(|character| character.is_control())
        }) {
            return Err(invalid_attachment("attachment display name is invalid"));
        }
    }
    let mut seen: Vec<&str> = attachments
        .iter()
        .map(|attachment| attachment.attachment_id.as_str())
        .collect();
    seen.sort_unstable();
    if seen.windows(2).any(|pair| pair[0] == pair[1]) {
        // Refused rather than de-duplicated: a set whose persisted length
        // differs from the number of blocks the runtime injects would make the
        // one-block-per-reference rule unstatable.
        return Err(invalid_attachment(
            "an attachment is listed twice on one message",
        ));
    }
    let mut persisted = Vec::with_capacity(attachments.len());
    let mut promoted_count = 0_i64;
    let mut promoted_bytes = 0_i64;
    let mut staged = Vec::new();
    for attachment in attachments {
        let attachment_id: ProductAttachmentId = attachment
            .attachment_id
            .parse()
            .map_err(|_| invalid_attachment("attachment id is invalid"))?;
        let record = attachment_in_transaction(connection, session_id, &attachment_id)?
            .ok_or_else(|| {
                ProductStoreError::new(
                    ProductErrorCode::ProductAttachmentNotFound,
                    "attachment does not belong to this session",
                )
            })?;
        match record.status {
            ProductAttachmentStatus::Expired => {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductAttachmentConflict,
                    "attachment has expired and must be uploaded again",
                ));
            }
            ProductAttachmentStatus::Staged => {
                promoted_count += 1;
                promoted_bytes = promoted_bytes
                    .saturating_add(i64::try_from(record.byte_length).map_err(storage_error)?);
                staged.push(attachment_id);
            }
            ProductAttachmentStatus::Referenced => {}
        }
        persisted.push(PersistedMessageAttachment {
            attachment_id: attachment.attachment_id.clone(),
            name: attachment.name.clone(),
        });
    }
    let budget = session_attachment_budget(connection, session_id)?;
    budget.admits_referenced(promoted_count, promoted_bytes)?;
    let referenced_at = now_rfc3339();
    for attachment_id in staged {
        let updated = connection
            .execute(
                r#"
                UPDATE product_attachments
                SET status = 'referenced', referenced_at = ?1, expires_at = NULL
                WHERE product_session_id = ?2 AND attachment_id = ?3
                  AND status = 'staged'
                "#,
                params![
                    referenced_at,
                    session_id.to_string(),
                    attachment_id.as_str()
                ],
            )
            .map_err(storage_error)?;
        if updated != 1 {
            // The row changed under this transaction. Every writer that could
            // do that takes the same `IMMEDIATE` lock, so this is corruption or
            // a conflicting schema, not a race to paper over.
            return Err(corrupt(
                "attachment changed status while a message referenced it",
            ));
        }
    }
    Ok(persisted)
}

/// Project one durable attachment row into the client-facing reference.
///
/// Availability is derived from the row plus a metadata-only look at the
/// payload: an `expired` row is gone by decision, and a payload whose length no
/// longer matches its row is `corrupt`. The digest is not recomputed here — a
/// transcript read must not hash every payload — so the paths that actually read
/// bytes (download and model injection) are the ones that compare it.
pub(super) fn attachment_ref_from_record(
    root: &Path,
    record: &ProductAttachmentRecord,
    name: Option<String>,
) -> ProductMessageAttachmentRef {
    ProductMessageAttachmentRef {
        attachment_id: record.attachment_id.clone(),
        content_type: record.content_type.clone(),
        size: record.byte_length,
        sha256: record.sha256.clone(),
        name: name.or_else(|| record.display_name.clone()),
        availability: payload_availability(root, record),
        degradation: None,
    }
}

/// Metadata-only availability of one attachment's payload.
fn payload_availability(
    root: &Path,
    record: &ProductAttachmentRecord,
) -> ProductAttachmentAvailability {
    if record.status == ProductAttachmentStatus::Expired {
        return ProductAttachmentAvailability::Expired;
    }
    let Some(path) = crate::attachment_paths::payload_path_for(root, record) else {
        return ProductAttachmentAvailability::Corrupt;
    };
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() == record.byte_length => {
            ProductAttachmentAvailability::Available
        }
        Ok(metadata) if metadata.file_type().is_file() => ProductAttachmentAvailability::Corrupt,
        Ok(_) => ProductAttachmentAvailability::Corrupt,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ProductAttachmentAvailability::Missing
        }
        Err(_) => ProductAttachmentAvailability::Corrupt,
    }
}

fn invalid_attachment(message: &'static str) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductAttachmentInvalidInput, message)
}

/// The locator a message uses to name a batch of the session's attachment rows.
///
/// Built from the persisted list, so a resolve step only ever asks for the rows
/// the message actually names — the number of statements is bounded by the
/// message's attachment count, never by how many expired rows the session has
/// accumulated.
pub(super) fn attachment_id_placeholders(count: usize, first: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", first + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve one message's persisted references against the durable rows.
///
/// One bounded statement per message, asking only for the ids that message
/// names. A reference whose row has vanished is projected as
/// `availability: missing` with no type rather than dropped: the message did
/// name it, and a transcript that silently lost an attachment would misreport
/// what the user sent.
pub(super) fn resolve_message_attachments(
    connection: &Connection,
    root: &Path,
    session_id: &ProductSessionId,
    persisted: &[PersistedMessageAttachment],
) -> Result<Vec<ProductMessageAttachmentRef>, ProductStoreError> {
    if persisted.is_empty() {
        return Ok(Vec::new());
    }
    let mut binds: Vec<String> = Vec::with_capacity(persisted.len() + 1);
    binds.push(session_id.to_string());
    binds.extend(
        persisted
            .iter()
            .map(|attachment| attachment.attachment_id.clone()),
    );
    let sql = format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM product_attachments
         WHERE product_session_id = ?1 AND attachment_id IN ({})",
        attachment_id_placeholders(persisted.len(), 2)
    );
    let rows = connection
        .prepare(&sql)
        .map_err(storage_error)?
        .query_map(
            rusqlite::params_from_iter(binds.iter()),
            raw_attachment_from_row,
        )
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    let mut records = std::collections::HashMap::with_capacity(rows.len());
    for raw in rows {
        let record = raw.into_product()?;
        records.insert(record.attachment_id.as_str().to_string(), record);
    }
    let mut resolved = Vec::with_capacity(persisted.len());
    for attachment in persisted {
        let attachment_id: ProductAttachmentId = attachment
            .attachment_id
            .parse()
            .map_err(|_| corrupt("persisted message attachment id is invalid"))?;
        resolved.push(match records.get(attachment.attachment_id.as_str()) {
            Some(record) => attachment_ref_from_record(root, record, attachment.name.clone()),
            None => ProductMessageAttachmentRef {
                attachment_id,
                content_type: String::new(),
                size: 0,
                sha256: String::new(),
                name: attachment.name.clone(),
                availability: ProductAttachmentAvailability::Missing,
                degradation: None,
            },
        });
    }
    Ok(resolved)
}

/// The session must exist and must not be archived. Unknown is the existing
/// `product_not_found` (404); archived is an attachment conflict (409),
/// matching how an archived session refuses a new message.
fn require_attachment_session(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<(), ProductStoreError> {
    let session = get_session(connection, session_id)?;
    if session.status == ProductSessionStatus::Archived {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductAttachmentConflict,
            "archived product sessions cannot accept attachments",
        ));
    }
    Ok(())
}

/// Everything the session spends on attachments right now, split by lifecycle
/// status. Staged and referenced are accumulated separately, so a staged
/// attachment cannot starve a referenced one and neither budget can be charged
/// to the other. This is the accounting the reference transition (a later PR)
/// and the publish path share, so the two cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionAttachmentBudget {
    staged_count: i64,
    staged_bytes: i64,
    referenced_count: i64,
    referenced_bytes: i64,
}

impl SessionAttachmentBudget {
    /// Refuse when this publish would exceed either staged budget. Equality is
    /// allowed: the limit is a ceiling, not an exclusive bound.
    fn admits_staged(&self, byte_length: i64) -> Result<(), ProductStoreError> {
        if self.staged_count + 1 > MAX_STAGED_ATTACHMENTS_PER_SESSION {
            return Err(quota_error(
                "staged attachment count quota for this session is exhausted",
            ));
        }
        if self.staged_bytes.saturating_add(byte_length) > MAX_STAGED_ATTACHMENT_BYTES_PER_SESSION {
            return Err(quota_error(
                "staged attachment byte quota for this session would be exceeded",
            ));
        }
        Ok(())
    }

    /// Refuse when the *referenced* budgets are already violated.
    ///
    /// This is the consistency guard on every read/write of the account. The
    /// reachable edge — a reference that would cross the ceiling — is
    /// [`Self::admits_referenced`], which is what a send evaluates.
    fn referenced_limits(&self) -> (i64, i64) {
        (
            MAX_REFERENCED_ATTACHMENTS_PER_SESSION,
            MAX_REFERENCED_ATTACHMENT_BYTES_PER_SESSION,
        )
    }

    fn within_referenced_limits(&self) -> bool {
        let (count_limit, byte_limit) = self.referenced_limits();
        self.referenced_count <= count_limit && self.referenced_bytes <= byte_limit
    }

    /// Refuse a reference that would push the session past either referenced
    /// ceiling.
    ///
    /// Only *newly* referenced rows are charged: an attachment a second message
    /// also cites is already counted, and charging it twice would make the
    /// budget depend on how many messages mention a file rather than on how much
    /// the session holds. Equality is allowed; the limit is a ceiling.
    fn admits_referenced(
        &self,
        added_count: i64,
        added_bytes: i64,
    ) -> Result<(), ProductStoreError> {
        let (count_limit, byte_limit) = self.referenced_limits();
        if self.referenced_count.saturating_add(added_count) > count_limit {
            return Err(quota_error(
                "referenced attachment count quota for this session would be exceeded",
            ));
        }
        if self.referenced_bytes.saturating_add(added_bytes) > byte_limit {
            return Err(quota_error(
                "referenced attachment byte quota for this session would be exceeded",
            ));
        }
        Ok(())
    }
}

/// One pass over the session's rows. A single statement keeps the four totals
/// consistent with each other inside the surrounding transaction.
fn session_attachment_budget(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<SessionAttachmentBudget, ProductStoreError> {
    let budget = connection
        .query_row(
            r#"
            SELECT
                COALESCE(SUM(CASE WHEN status = 'staged' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'staged' THEN byte_length ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'referenced' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'referenced' THEN byte_length ELSE 0 END), 0)
            FROM product_attachments WHERE product_session_id = ?1
            "#,
            params![session_id.to_string()],
            |row| {
                Ok(SessionAttachmentBudget {
                    staged_count: row.get(0)?,
                    staged_bytes: row.get(1)?,
                    referenced_count: row.get(2)?,
                    referenced_bytes: row.get(3)?,
                })
            },
        )
        .map_err(storage_error)?;
    if !budget.within_referenced_limits() {
        // A typed refusal rather than a storage failure: the session is over a
        // published ceiling, which is the caller's condition to see. No publish
        // path can reach this today — nothing can reference an attachment yet —
        // so it is reachable only once the reference transition exists, and it
        // is reported as the same quota code that transition will use.
        return Err(quota_error(
            "persisted attachment referenced usage exceeds its session quota",
        ));
    }
    Ok(budget)
}

fn attachment_in_transaction(
    connection: &Connection,
    session_id: &ProductSessionId,
    attachment_id: &ProductAttachmentId,
) -> Result<Option<ProductAttachmentRecord>, ProductStoreError> {
    let raw = connection
        .query_row(
            &format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM product_attachments
                 WHERE product_session_id = ?1 AND attachment_id = ?2"
            ),
            params![session_id.to_string(), attachment_id.as_str()],
            raw_attachment_from_row,
        )
        .optional()
        .map_err(storage_error)?;
    raw.map(RawAttachment::into_product).transpose()
}

/// The row exactly as SQLite stores it. Validation and id parsing happen in
/// [`RawAttachment::into_product`], so a corrupt row is a typed store error
/// rather than a silent default.
struct RawAttachment {
    attachment_id: String,
    product_session_id: String,
    content_type: String,
    byte_length: i64,
    sha256: String,
    status: String,
    display_name: Option<String>,
    created_at: String,
    referenced_at: Option<String>,
    expires_at: Option<String>,
    scan_flags: String,
}

impl RawAttachment {
    fn into_product(self) -> Result<ProductAttachmentRecord, ProductStoreError> {
        Ok(ProductAttachmentRecord {
            attachment_id: self
                .attachment_id
                .parse()
                .map_err(|_| corrupt("persisted attachment id is invalid"))?,
            product_session_id: self
                .product_session_id
                .parse()
                .map_err(|_| corrupt("persisted attachment session id is invalid"))?,
            content_type: self.content_type,
            byte_length: u64::try_from(self.byte_length)
                .map_err(|_| corrupt("persisted attachment length is invalid"))?,
            sha256: self.sha256,
            status: ProductAttachmentStatus::from_db(&self.status)?,
            display_name: self.display_name,
            created_at: self.created_at,
            referenced_at: self.referenced_at,
            expires_at: self.expires_at,
            scan_flags: serde_json::from_str(&self.scan_flags)
                .map_err(|_| corrupt("persisted attachment scan flags are invalid"))?,
        })
    }
}

fn raw_attachment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAttachment> {
    Ok(RawAttachment {
        attachment_id: row.get(0)?,
        product_session_id: row.get(1)?,
        content_type: row.get(2)?,
        byte_length: row.get(3)?,
        sha256: row.get(4)?,
        status: row.get(5)?,
        display_name: row.get(6)?,
        created_at: row.get(7)?,
        referenced_at: row.get(8)?,
        expires_at: row.get(9)?,
        scan_flags: row.get(10)?,
    })
}

fn corrupt(message: &'static str) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductBindingCorrupt, message)
}

fn quota_error(message: &'static str) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductAttachmentQuota, message)
}

/// `now + seconds`, in the same format [`now_rfc3339`] writes.
fn rfc3339_in_seconds(seconds: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(seconds))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::{CreateProductMessageRequest, ProductMessageAttachmentRequest};

    const SESSION: &str = "01J8Z0M6Q3W9F2V7B4K1N5T8XA";
    const OTHER_SESSION: &str = "01J8Z0M6Q3W9F2V7B4K1N5T8XB";
    const WORKSPACE: &str = "01J8Z0M6Q3W9F2V7B4K1N5T8XC";

    struct Fixture {
        _temp: TempDir,
        repository: super::super::repository::ProductRepository,
    }

    impl Fixture {
        /// A migrated store holding two live sessions, one archived session, and
        /// the workspace they belong to.
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let path: PathBuf = temp.path().join("product.sqlite");
            let database = super::super::schema::ProductDatabase::new(path, 5_000).unwrap();
            database.initialize().unwrap();
            let repository = super::super::repository::ProductRepository::new(database);
            let connection = repository.database.connect().unwrap();
            connection
                .execute_batch(&format!(
                    r#"
                    INSERT INTO product_workspaces(
                        workspace_id, canonical_root, canonical_key, kind, display_name,
                        pinned, last_opened_at, created_at, updated_at
                    ) VALUES (
                        '{WORKSPACE}', 'C:/ws', 'key-1', 'folder', 'ws',
                        0, '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                    );
                    INSERT INTO product_sessions(
                        product_session_id, workspace_id, title, status, created_at, updated_at
                    ) VALUES
                        ('{SESSION}', '{WORKSPACE}', 's', 'idle',
                         '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'),
                        ('{OTHER_SESSION}', '{WORKSPACE}', 's', 'idle',
                         '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'),
                        ('01J8Z0M6Q3W9F2V7B4K1N5T8XD', '{WORKSPACE}', 's', 'archived',
                         '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z');
                    "#
                ))
                .unwrap();
            Self {
                _temp: temp,
                repository,
            }
        }

        fn session(&self) -> ProductSessionId {
            SESSION.parse().unwrap()
        }

        fn other_session(&self) -> ProductSessionId {
            OTHER_SESSION.parse().unwrap()
        }

        fn archived_session(&self) -> ProductSessionId {
            "01J8Z0M6Q3W9F2V7B4K1N5T8XD".parse().unwrap()
        }

        fn unknown_session(&self) -> ProductSessionId {
            "01J8Z0M6Q3W9F2V7B4K1N5T8XZ".parse().unwrap()
        }

        /// Insert a row directly, so a quota edge can be reached without
        /// writing that many payloads.
        fn insert_row(&self, session: &ProductSessionId, status: &str, bytes: i64) {
            let id = ProductAttachmentId::new();
            let connection = self.repository.database.connect().unwrap();
            connection
                .execute(
                    r#"
                    INSERT INTO product_attachments(
                        attachment_id, product_session_id, content_type, byte_length,
                        sha256, status, display_name, created_at, referenced_at,
                        expires_at, scan_flags
                    ) VALUES (?1, ?2, 'image/png', ?3, 'digest', ?4, NULL,
                              '2026-09-26T00:00:00Z', NULL, NULL, '[]')
                    "#,
                    params![id.as_str(), session.to_string(), bytes, status],
                )
                .unwrap();
        }

        fn staged_request(&self, bytes: u64) -> CreateStagedAttachmentRequest {
            CreateStagedAttachmentRequest {
                attachment_id: ProductAttachmentId::new(),
                content_type: "image/png".to_string(),
                byte_length: bytes,
                sha256: "9f2c".to_string(),
                display_name: Some("screenshot.png".to_string()),
                scan_flags: vec![],
            }
        }
    }

    #[test]
    fn a_staged_attachment_is_published_with_a_server_identity_and_a_ttl() {
        let fixture = Fixture::new();

        let record = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(2_048))
            .unwrap();

        assert_eq!(record.product_session_id, fixture.session());
        assert_eq!(record.status, ProductAttachmentStatus::Staged);
        assert_eq!(record.byte_length, 2_048);
        assert_eq!(record.content_type, "image/png");
        assert_eq!(record.display_name.as_deref(), Some("screenshot.png"));
        assert!(record.referenced_at.is_none());
        assert!(record.expires_at.is_some());
        assert!(record.scan_flags.is_empty());
        // A fresh ULID, never derived from the display name.
        assert_eq!(record.attachment_id.as_str().len(), 26);

        let read_back = fixture
            .repository
            .attachment_for_session(&fixture.session(), &record.attachment_id)
            .unwrap()
            .expect("the published row must be readable");
        assert_eq!(read_back, record);
    }

    #[test]
    fn an_unknown_session_cannot_resolve_and_cannot_publish() {
        let fixture = Fixture::new();

        let error = fixture
            .repository
            .resolve_attachment_session(&fixture.unknown_session())
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductNotFound);

        let error = fixture
            .repository
            .create_staged_attachment(&fixture.unknown_session(), fixture.staged_request(1))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductNotFound);
    }

    #[test]
    fn an_archived_session_refuses_an_attachment_as_a_conflict() {
        let fixture = Fixture::new();

        let error = fixture
            .repository
            .resolve_attachment_session(&fixture.archived_session())
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentConflict);

        let error = fixture
            .repository
            .create_staged_attachment(&fixture.archived_session(), fixture.staged_request(1))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentConflict);
    }

    #[test]
    fn a_read_in_another_session_finds_nothing() {
        let fixture = Fixture::new();
        let record = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(16))
            .unwrap();

        // The id exists, but not for this session: the pair is the scope, so a
        // cross-session probe is indistinguishable from an absent id.
        assert!(
            fixture
                .repository
                .attachment_for_session(&fixture.other_session(), &record.attachment_id)
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .repository
                .attachment_for_session(&fixture.session(), &ProductAttachmentId::new())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_staged_count_quota_refuses_the_slot_after_the_limit() {
        let fixture = Fixture::new();
        for _ in 0..MAX_STAGED_ATTACHMENTS_PER_SESSION {
            fixture.insert_row(&fixture.session(), "staged", 1);
        }

        let error = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(1))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentQuota);

        // One row below the limit still fits.
        let fixture = Fixture::new();
        for _ in 0..MAX_STAGED_ATTACHMENTS_PER_SESSION - 1 {
            fixture.insert_row(&fixture.session(), "staged", 1);
        }
        assert!(
            fixture
                .repository
                .create_staged_attachment(&fixture.session(), fixture.staged_request(1))
                .is_ok()
        );
    }

    #[test]
    fn the_staged_byte_quota_refuses_only_what_would_exceed_it() {
        let fixture = Fixture::new();
        fixture.insert_row(
            &fixture.session(),
            "staged",
            MAX_STAGED_ATTACHMENT_BYTES_PER_SESSION - 10,
        );

        // Exactly the remaining budget is allowed; one byte more is not.
        let error = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(11))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentQuota);
        assert!(
            fixture
                .repository
                .create_staged_attachment(&fixture.session(), fixture.staged_request(10))
                .is_ok()
        );
    }

    #[test]
    fn a_full_referenced_budget_does_not_consume_the_staged_one() {
        let fixture = Fixture::new();
        let per_row =
            MAX_REFERENCED_ATTACHMENT_BYTES_PER_SESSION / MAX_REFERENCED_ATTACHMENTS_PER_SESSION;
        for _ in 0..MAX_REFERENCED_ATTACHMENTS_PER_SESSION {
            fixture.insert_row(&fixture.session(), "referenced", per_row);
        }

        // The referenced budget is at its ceiling and the staged budget is
        // untouched, so a staged attachment still fits.
        assert!(
            fixture
                .repository
                .create_staged_attachment(&fixture.session(), fixture.staged_request(1_048_576))
                .is_ok()
        );
    }

    /// Referenced usage over its ceiling is a typed quota refusal, not a
    /// storage failure.
    ///
    /// No publish path can create a `referenced` row today, so this state is
    /// reachable only once a message can reference an attachment; the point of
    /// the test is that the account reports it as the caller's condition
    /// (`product_attachment_quota`) rather than as an internal error.
    #[test]
    fn persisted_referenced_usage_over_its_ceiling_is_a_typed_quota_refusal() {
        let fixture = Fixture::new();

        // One byte over the referenced byte ceiling.
        fixture.insert_row(
            &fixture.session(),
            "referenced",
            MAX_REFERENCED_ATTACHMENT_BYTES_PER_SESSION + 1,
        );
        let error = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(1))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentQuota);

        // One row over the referenced count ceiling.
        for _ in 0..=MAX_REFERENCED_ATTACHMENTS_PER_SESSION {
            fixture.insert_row(&fixture.other_session(), "referenced", 1);
        }
        let error = fixture
            .repository
            .create_staged_attachment(&fixture.other_session(), fixture.staged_request(1))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentQuota);
    }

    #[test]
    fn expired_rows_do_not_consume_the_staged_budget() {
        let fixture = Fixture::new();
        for _ in 0..MAX_STAGED_ATTACHMENTS_PER_SESSION + 4 {
            fixture.insert_row(&fixture.session(), "expired", 1_048_576);
        }

        assert!(
            fixture
                .repository
                .create_staged_attachment(&fixture.session(), fixture.staged_request(1))
                .is_ok(),
            "a reclaimed row must not keep holding a staged slot"
        );
    }

    #[test]
    fn every_session_accounts_for_its_own_budget() {
        let fixture = Fixture::new();
        for _ in 0..MAX_STAGED_ATTACHMENTS_PER_SESSION {
            fixture.insert_row(&fixture.session(), "staged", 1);
        }

        assert!(
            fixture
                .repository
                .create_staged_attachment(&fixture.other_session(), fixture.staged_request(1))
                .is_ok(),
            "one session's quota must not be charged to another"
        );
    }

    #[test]
    fn scan_flags_round_trip_as_a_secret_free_code_list() {
        let fixture = Fixture::new();
        let mut request = fixture.staged_request(8);
        request.scan_flags = vec![
            "secret_shaped_name".to_string(),
            "possible_secret_content".to_string(),
        ];

        let record = fixture
            .repository
            .create_staged_attachment(&fixture.session(), request)
            .unwrap();

        assert_eq!(
            record.scan_flags,
            vec![
                "secret_shaped_name".to_string(),
                "possible_secret_content".to_string()
            ]
        );
        let read_back = fixture
            .repository
            .attachment_for_session(&fixture.session(), &record.attachment_id)
            .unwrap()
            .unwrap();
        assert_eq!(read_back.scan_flags, record.scan_flags);
    }

    #[test]
    fn a_corrupt_row_is_reported_rather_than_defaulted() {
        let fixture = Fixture::new();
        let record = fixture
            .repository
            .create_staged_attachment(&fixture.session(), fixture.staged_request(4))
            .unwrap();
        let connection = fixture.repository.database.connect().unwrap();
        connection
            .execute(
                "UPDATE product_attachments SET scan_flags = 'not json' WHERE attachment_id = ?1",
                params![record.attachment_id.as_str()],
            )
            .unwrap();

        let error = fixture
            .repository
            .attachment_for_session(&fixture.session(), &record.attachment_id)
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductBindingCorrupt);
    }

    /// A message request referencing `attachment_ids`, in order.
    fn message_request(
        attachment_ids: &[&ProductAttachmentId],
        idempotency_key: Option<&str>,
    ) -> CreateProductMessageRequest {
        CreateProductMessageRequest {
            content: "please read the attachment".to_string(),
            idempotency_key: idempotency_key.map(str::to_string),
            attachments: attachment_ids
                .iter()
                .map(|id| ProductMessageAttachmentRequest {
                    attachment_id: (*id).clone(),
                    name: None,
                })
                .collect(),
        }
    }

    impl Fixture {
        fn staged(&self, bytes: u64) -> ProductAttachmentRecord {
            self.repository
                .create_staged_attachment(&self.session(), self.staged_request(bytes))
                .unwrap()
        }

        /// Force a staged row past its TTL without waiting for real time.
        fn expire_now(&self, id: &ProductAttachmentId) {
            let connection = self.repository.database.connect().unwrap();
            connection
                .execute(
                    "UPDATE product_attachments
                     SET expires_at = '2000-01-01T00:00:00.000Z'
                     WHERE attachment_id = ?1",
                    params![id.as_str()],
                )
                .unwrap();
        }

        fn status_of(&self, id: &ProductAttachmentId) -> (String, Option<String>) {
            let connection = self.repository.database.connect().unwrap();
            connection
                .query_row(
                    "SELECT status, expires_at FROM product_attachments WHERE attachment_id = ?1",
                    params![id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        }
    }

    #[test]
    fn a_referenced_attachment_is_inbound_and_its_ttl_is_cleared() {
        let fixture = Fixture::new();
        let staged = fixture.staged(2_048);
        assert_eq!(fixture.status_of(&staged.attachment_id).0, "staged");

        let (message, replayed) = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&staged.attachment_id], None),
            )
            .unwrap();

        assert!(!replayed);
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(
            message.attachments[0].attachment_id, staged.attachment_id,
            "the reference names the id the client sent"
        );
        assert_eq!(message.attachments[0].content_type, "image/png");
        assert_eq!(message.attachments[0].size, 2_048);
        assert_eq!(
            message.attachments[0].name.as_deref(),
            Some("screenshot.png")
        );

        // The promotion is durable, and the TTL that would have reclaimed a
        // staged payload no longer applies: a referenced attachment is kept
        // regardless of age.
        let (status, expires_at) = fixture.status_of(&staged.attachment_id);
        assert_eq!(status, "referenced");
        assert!(expires_at.is_none(), "a referenced row keeps no TTL");
        let record = fixture
            .repository
            .attachment_for_session(&fixture.session(), &staged.attachment_id)
            .unwrap()
            .unwrap();
        assert!(record.referenced_at.is_some());
    }

    #[test]
    fn a_message_referencing_an_expired_attachment_is_refused() {
        let fixture = Fixture::new();
        let staged = fixture.staged(16);
        fixture.expire_now(&staged.attachment_id);
        // The cleanup job would have run by now; expire the row the way it does.
        let expired = fixture.repository.expire_staged_attachments(8).unwrap();
        assert_eq!(expired.len(), 1);

        let error = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&staged.attachment_id], None),
            )
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentConflict);
        // Nothing was written for the refused message.
        assert_eq!(fixture.status_of(&staged.attachment_id).0, "expired");
    }

    #[test]
    fn a_reference_to_an_unknown_or_foreign_attachment_is_not_found() {
        let fixture = Fixture::new();
        let unknown: ProductAttachmentId = ProductAttachmentId::new();
        let error = fixture
            .repository
            .create_message(&fixture.session(), message_request(&[&unknown], None))
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentNotFound);

        let foreign = fixture
            .repository
            .create_staged_attachment(&fixture.other_session(), fixture.staged_request(16))
            .unwrap();
        let error = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&foreign.attachment_id], None),
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            ProductErrorCode::ProductAttachmentNotFound,
            "an id from another session must not be referenceable"
        );
        assert_eq!(
            fixture.status_of(&foreign.attachment_id).0,
            "staged",
            "a refused reference leaves the other session's row untouched"
        );
    }

    #[test]
    fn a_duplicate_reference_in_one_message_is_refused() {
        let fixture = Fixture::new();
        let staged = fixture.staged(16);
        let error = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&staged.attachment_id, &staged.attachment_id], None),
            )
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductAttachmentInvalidInput);
        assert_eq!(
            fixture.status_of(&staged.attachment_id).0,
            "staged",
            "a refused message must not partially promote"
        );
    }

    #[test]
    fn a_reference_over_the_referenced_byte_budget_is_a_typed_refusal() {
        let fixture = Fixture::new();
        // Spend the referenced ceiling with a row written directly, then ask for
        // one byte more through the real path.
        fixture.insert_row(
            &fixture.session(),
            "referenced",
            MAX_REFERENCED_ATTACHMENT_BYTES_PER_SESSION,
        );
        let staged = fixture.staged(1);

        let error = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&staged.attachment_id], None),
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            ProductErrorCode::ProductAttachmentQuota,
            "an over-budget reference is a typed, visible refusal"
        );
        assert_eq!(fixture.status_of(&staged.attachment_id).0, "staged");
    }

    #[test]
    fn the_idempotency_digest_covers_the_attachment_set() {
        let fixture = Fixture::new();
        let first = fixture.staged(16);
        let second = fixture.staged(16);

        let (message, replayed) = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&first.attachment_id], Some("key-1")),
            )
            .unwrap();
        assert!(!replayed);

        // The same key and the same content but a different attachment set is a
        // different request, not a replay.
        let error = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&second.attachment_id], Some("key-1")),
            )
            .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductControlConflict);
        assert_eq!(
            fixture.status_of(&second.attachment_id).0,
            "staged",
            "the refused retry must not promote its own attachment"
        );

        // Repeating the original request is still idempotent, and the replay
        // carries the attachment set it was stored with.
        let (same, replayed) = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&first.attachment_id], Some("key-1")),
            )
            .unwrap();
        assert!(replayed);
        assert_eq!(same.id, message.id);
        assert_eq!(same.attachments.len(), 1);
        assert_eq!(same.attachments[0].attachment_id, first.attachment_id);
    }

    #[test]
    fn the_attachment_order_does_not_change_the_request_identity() {
        let fixture = Fixture::new();
        let first = fixture.staged(16);
        let second = fixture.staged(16);

        let (message, _) = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(
                    &[&first.attachment_id, &second.attachment_id],
                    Some("key-2"),
                ),
            )
            .unwrap();
        let (same, replayed) = fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(
                    &[&second.attachment_id, &first.attachment_id],
                    Some("key-2"),
                ),
            )
            .unwrap();
        assert!(replayed, "a reordered attachment set is the same request");
        assert_eq!(same.id, message.id);
        assert_eq!(
            same.attachments
                .iter()
                .map(|a| a.attachment_id.clone())
                .collect::<Vec<_>>(),
            vec![first.attachment_id.clone(), second.attachment_id.clone()],
            "the stored order is the order the client sent"
        );
    }

    #[test]
    fn an_attachment_free_message_keeps_its_legacy_digest_and_projection() {
        let fixture = Fixture::new();
        let (message, _) = fixture
            .repository
            .create_message(
                &fixture.session(),
                CreateProductMessageRequest {
                    content: "no attachment".to_string(),
                    idempotency_key: Some("key-3".to_string()),
                    attachments: Vec::new(),
                },
            )
            .unwrap();
        assert!(message.attachments.is_empty());
        // The digest is the content-only digest a pre-attachment server wrote,
        // so a message stored before this column existed still replays.
        let connection = fixture.repository.database.connect().unwrap();
        let digest: String = connection
            .query_row(
                "SELECT request_digest FROM product_session_controls WHERE control_id = ?1",
                params![message.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(digest, rove_runtime::context::stable_hash("no attachment"));
    }

    #[test]
    fn expiring_staged_rows_reports_only_what_it_moved_and_respects_its_bound() {
        let fixture = Fixture::new();
        let first = fixture.staged(16);
        let second = fixture.staged(16);
        let referenced = fixture.staged(16);
        fixture.expire_now(&first.attachment_id);
        fixture.expire_now(&second.attachment_id);
        fixture.expire_now(&referenced.attachment_id);
        // Promote one of them first: a referenced row is never a candidate,
        // whatever its `expires_at` says.
        fixture
            .repository
            .create_message(
                &fixture.session(),
                message_request(&[&referenced.attachment_id], None),
            )
            .unwrap();

        let moved = fixture.repository.expire_staged_attachments(1).unwrap();
        assert_eq!(moved.len(), 1, "the run is bounded by the caller's limit");
        assert_eq!(
            fixture.status_of(&referenced.attachment_id).0,
            "referenced",
            "a referenced row keeps its status and its payload"
        );
        let moved = fixture.repository.expire_staged_attachments(8).unwrap();
        assert_eq!(moved.len(), 1, "the second run moves what is left");
        assert_eq!(
            fixture.status_of(&first.attachment_id).0,
            "expired",
            "both rows end up expired, one run each"
        );
        assert_eq!(fixture.status_of(&second.attachment_id).0, "expired");
        assert!(
            fixture
                .repository
                .expire_staged_attachments(8)
                .unwrap()
                .is_empty(),
            "a third run has nothing left to move"
        );

        // The bounded scan is ordered, so a small limit makes progress rather
        // than starving on the same row.
        assert_eq!(
            fixture
                .repository
                .attachment_statuses_for_session(&fixture.session(), 8)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn attachment_statuses_are_session_scoped_and_bounded() {
        let fixture = Fixture::new();
        let mine = fixture.staged(16);
        let theirs = fixture
            .repository
            .create_staged_attachment(&fixture.other_session(), fixture.staged_request(16))
            .unwrap();

        let statuses = fixture
            .repository
            .attachment_statuses_for_session(&fixture.session(), 8)
            .unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].0, mine.attachment_id);
        assert!(
            statuses.iter().all(|(id, _)| *id != theirs.attachment_id),
            "another session's rows are never in this session's set"
        );

        let bounded = fixture
            .repository
            .attachment_statuses_for_session(&fixture.session(), 0)
            .unwrap();
        assert!(bounded.is_empty(), "a zero bound examines nothing");
    }
}
