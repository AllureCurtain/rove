use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{SecondsFormat, Utc};
use rove_runtime::review::{
    ReviewConclusion, ReviewFinding, ReviewResult, ReviewTargetKind, ReviewTargetSummary,
};
use rove_runtime::types::{JobId, RunId, SessionId};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;

use crate::product::{
    CommitProductRunBinding, CreateProductControlRequest, CreateProductForkRequest,
    CreateProductMessageRequest, CreateProductProviderProfileRequest, CreateProductReviewRecord,
    CreateProductSessionRequest, CreateProductWorkspaceRequest, DEFAULT_PRODUCT_MAX_STEPS,
    M1BrowserMigrationPreflight, M1BrowserMigrationResponse, M1MigrationDisposition,
    M1MigrationIssue, M1MigrationIssueCode, M1PreferencesBaseline, M1ProviderProfileIdMapping,
    M1SessionIdMapping, M1WorkspaceIdMapping, MAX_PRODUCT_FORK_INHERITED_RUNS,
    MAX_PRODUCT_MAX_STEPS, MAX_PRODUCT_MESSAGE_PAGE_LIMIT, MAX_PRODUCT_PROVIDER_PROFILES,
    MAX_PRODUCT_SESSIONS, MAX_PRODUCT_TEXT_BYTES, MAX_PRODUCT_WORKSPACES,
    PreparedM1BrowserMigration, ProductApprovalPreference, ProductControl, ProductControlId,
    ProductControlKind, ProductControlStatus, ProductErrorCode, ProductFollowupTurnClaim,
    ProductFork, ProductForkContext, ProductForkId, ProductForkInheritedRun, ProductMessage,
    ProductMessageDelivery, ProductMessagePage, ProductMessagePageQuery, ProductMessageStatus,
    ProductMigrationReceiptId, ProductPreferences, ProductPricingAvailability,
    ProductProviderCredentialSource, ProductProviderProfile, ProductProviderProfileId,
    ProductProviderSelection, ProductProviderType, ProductReasoningPreference, ProductResumeHealth,
    ProductResumeHealthStatus, ProductReview, ProductReviewFinding, ProductReviewFindingsQuery,
    ProductReviewFindingsResponse, ProductReviewId, ProductReviewStatus, ProductRuntimeBinding,
    ProductSession, ProductSessionContext, ProductSessionCursor, ProductSessionId,
    ProductSessionModelConfig, ProductSessionPage, ProductSessionPageQuery, ProductSessionRecovery,
    ProductSessionRunBinding, ProductSessionRunModelView, ProductSessionStatus, ProductStoreError,
    ProductThemePreference, ProductTurnClaim, ProductTurnClaimId, ProductTurnControlFinish,
    ProductWorkspace, ProductWorkspaceId, ProductWorkspaceKind, RecoverProductSessionOwnership,
    SESSION_RANK_ARCHIVED, SESSION_RANK_LIVE, UpdateProductPreferencesRequest,
    UpdateProductProviderProfileRequest, UpdateProductSessionModelConfigRequest,
    UpdateProductSessionRequest, VerifiedM1SessionRunBinding, VerifiedProductForkBoundary,
    m1_browser_migration_digest,
};

use super::schema::{ProductDatabase, storage_error};
use super::validation::{
    MAX_RUN_BINDINGS_PER_SESSION, ValidatedPreferences, ValidatedProviderProfile,
    ValidatedWorkspace, invalid, normalized_timestamp, profile_id_string, validate_issue_entity,
    validate_migration_envelope, validate_migration_provider, validate_preferences,
    validate_provider_create, validate_provider_selection, validate_provider_update,
    validate_required_text, validate_source_id, validate_title, validate_workspace,
    validate_workspace_request,
};

const MIGRATION_SOURCE_WEB_M1: &str = "web_m1_local_storage";
const MIGRATION_PREPARATION_TTL_SECS: i64 = 24 * 60 * 60;
// Mirrors the bounded runtime steer channel. Keeping persistent pending
// steers within this capacity guarantees a run attaching after an HTTP/API
// race can inject every pending message at its first declared safe point.
const MAX_PENDING_STEERS_PER_SESSION: i64 = 64;

fn validate_control_message(
    content: &str,
    idempotency_key: Option<&str>,
) -> Result<(), ProductStoreError> {
    if content.is_empty() {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "message content must not be empty",
        ));
    }
    if content.len() > 32_768 {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "message content exceeds the 32KiB limit",
        ));
    }
    if let Some(key) = idempotency_key
        && (key.is_empty() || key.len() > 128)
    {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "idempotency_key must be 1..128 characters",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) struct ProductRepository {
    database: ProductDatabase,
}

impl ProductRepository {
    pub(super) fn new(database: ProductDatabase) -> Self {
        Self { database }
    }

    pub(super) fn initialize_and_recover(&self) -> Result<u64, ProductStoreError> {
        self.database.initialize()?;
        self.remove_expired_migration_preparations()?;
        let recovered = self.recover_stale_turn_claims()?;
        self.recover_interrupted_reviews()?;
        Ok(recovered)
    }

    fn recover_interrupted_reviews(&self) -> Result<u64, ProductStoreError> {
        let connection = self.database.connect()?;
        let updated = connection
            .execute(
                "UPDATE product_reviews SET status = 'needs_attention', updated_at = ?1, finalized_at = COALESCE(finalized_at, ?1) WHERE status IN ('queued', 'running')",
                params![now_rfc3339()],
            )
            .map_err(storage_error)?;
        u64::try_from(updated).map_err(storage_error)
    }

    fn remove_expired_migration_preparations(&self) -> Result<u64, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let removed = remove_expired_migration_preparations_at(&transaction, Utc::now())?;
        transaction.commit().map_err(storage_error)?;
        Ok(removed)
    }

    pub(super) fn recover_stale_turn_claims(&self) -> Result<u64, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let claims: Vec<(
            ProductTurnClaimId,
            ProductSessionId,
            Option<ProductControlId>,
        )> = {
            let mut statement = transaction
                .prepare(
                    "SELECT claim_id, product_session_id, followup_control_id FROM product_turn_claims",
                )
                .map_err(storage_error)?;
            statement
                .query_map([], |row| {
                    let claim_id: String = row.get(0)?;
                    let session_id: String = row.get(1)?;
                    let control_id: Option<String> = row.get(2)?;
                    Ok((claim_id, session_id, control_id))
                })
                .map_err(storage_error)?
                .map(|row| {
                    let (claim_id, session_id, control_id) = row.map_err(storage_error)?;
                    Ok((
                        parse_product_id(&claim_id, "turn claim id")?,
                        parse_product_id(&session_id, "product session id")?,
                        control_id
                            .as_deref()
                            .map(|value| parse_product_id(value, "follow-up control id"))
                            .transpose()?,
                    ))
                })
                .collect::<Result<Vec<_>, ProductStoreError>>()?
        };

        let mut affected = 0_u64;
        for (claim_id, session_id, control_id) in claims {
            let safely_requeue = match control_id.as_ref() {
                Some(control_id) => transaction
                    .query_row(
                        r#"
                        SELECT status = 'accepted' AND run_id IS NULL
                        FROM product_session_controls
                        WHERE product_session_id = ?1 AND control_id = ?2
                        "#,
                        params![session_id.to_string(), control_id.to_string()],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()
                    .map_err(storage_error)?
                    .unwrap_or(false),
                None => false,
            };

            if safely_requeue {
                let control_id = control_id.as_ref().expect("checked above");
                transaction
                    .execute(
                        r#"
                        UPDATE product_session_controls
                        SET status = 'pending', run_id = NULL, applied_at = NULL,
                            abandoned_reason = NULL
                        WHERE product_session_id = ?1 AND control_id = ?2
                          AND status = 'accepted' AND run_id IS NULL
                        "#,
                        params![session_id.to_string(), control_id.to_string()],
                    )
                    .map_err(storage_error)?;
                transaction
                    .execute(
                        "UPDATE product_sessions SET status = 'idle', updated_at = ?2 WHERE product_session_id = ?1",
                        params![session_id.to_string(), now_rfc3339()],
                    )
                    .map_err(storage_error)?;
            } else {
                let steers = unapplied_steers_for_session(&transaction, &session_id)?;
                let followups = unapplied_followups_for_session(&transaction, &session_id)?;
                transition_unapplied_steers_in_transaction(
                    &transaction,
                    &session_id,
                    None,
                    "API process stopped before the steer reached a model turn",
                    steers.len(),
                )?;
                transition_unapplied_followups_in_transaction(
                    &transaction,
                    &session_id,
                    "API process stopped during follow-up delivery",
                    followups.len(),
                )?;
                transaction
                    .execute(
                        "UPDATE product_sessions SET status = 'needs_attention', updated_at = ?2 WHERE product_session_id = ?1",
                        params![session_id.to_string(), now_rfc3339()],
                    )
                    .map_err(storage_error)?;
            }
            let deleted = transaction
                .execute(
                    "DELETE FROM product_turn_claims WHERE claim_id = ?1 AND product_session_id = ?2",
                    params![claim_id.to_string(), session_id.to_string()],
                )
                .map_err(storage_error)?;
            affected = affected.saturating_add(u64::try_from(deleted).map_err(storage_error)?);
        }

        transaction.commit().map_err(storage_error)?;
        Ok(affected)
    }

    pub(super) fn list_workspaces(&self) -> Result<Vec<ProductWorkspace>, ProductStoreError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT workspace_id, canonical_root, kind, display_name, pinned,
                       last_opened_at, created_at, updated_at
                FROM product_workspaces
                ORDER BY pinned DESC, last_opened_at DESC,
                         display_name COLLATE NOCASE ASC, workspace_id ASC
                LIMIT ?1
                "#,
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![limit_i64(MAX_PRODUCT_WORKSPACES)?],
                raw_workspace_from_row,
            )
            .map_err(storage_error)?;
        let mut workspaces = Vec::new();
        for row in rows {
            workspaces.push(row.map_err(storage_error)?.into_product()?);
        }
        Ok(workspaces)
    }

    pub(super) fn get_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<ProductWorkspace, ProductStoreError> {
        let connection = self.database.connect()?;
        get_workspace(&connection, workspace_id)
    }

    pub(super) fn create_workspace(
        &self,
        request: CreateProductWorkspaceRequest,
    ) -> Result<ProductWorkspace, ProductStoreError> {
        let now = now_rfc3339();
        let workspace = validate_workspace_request(request, &now)?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;

        if let Some(existing) = find_workspace_by_key(&transaction, &workspace.canonical_key)? {
            if existing.kind != workspace.kind {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductSessionWorkspaceMismatch,
                    "canonical workspace root is already registered with a different kind",
                ));
            }
            transaction
                .execute(
                    r#"
                    UPDATE product_workspaces
                    SET canonical_root = ?2, display_name = ?3, pinned = ?4,
                        last_opened_at = ?5, updated_at = ?6
                    WHERE workspace_id = ?1
                    "#,
                    params![
                        existing.id.to_string(),
                        workspace.canonical_root_text,
                        workspace.display_name,
                        bool_to_i64(workspace.pinned),
                        workspace.last_opened_at,
                        now,
                    ],
                )
                .map_err(storage_error)?;
            let updated = get_workspace(&transaction, &existing.id)?;
            transaction.commit().map_err(storage_error)?;
            return Ok(updated);
        }

        enforce_table_limit(
            &transaction,
            "product_workspaces",
            MAX_PRODUCT_WORKSPACES,
            "workspace limit reached",
        )?;
        let workspace_id = ProductWorkspaceId::new();
        transaction
            .execute(
                r#"
                INSERT INTO product_workspaces(
                    workspace_id, canonical_root, canonical_key, kind, display_name,
                    pinned, last_opened_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                params![
                    workspace_id.to_string(),
                    workspace.canonical_root_text,
                    workspace.canonical_key,
                    workspace_kind_to_db(workspace.kind),
                    workspace.display_name,
                    bool_to_i64(workspace.pinned),
                    workspace.last_opened_at,
                    now,
                ],
            )
            .map_err(storage_error)?;
        let created = get_workspace(&transaction, &workspace_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(created)
    }

    pub(super) fn delete_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        require_workspace(&transaction, workspace_id)?;
        let active_claims: i64 = transaction
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM product_turn_claims AS claims
                INNER JOIN product_sessions AS sessions
                    ON sessions.product_session_id = claims.product_session_id
                WHERE sessions.workspace_id = ?1
                "#,
                params![workspace_id.to_string()],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        if active_claims != 0 {
            return Err(session_active(
                "workspace has a product session with an active turn",
            ));
        }

        transaction
            .execute(
                r#"
                UPDATE product_preferences
                SET active_workspace_id = NULL, active_session_id = NULL,
                    updated_at = ?2, revision = revision + 1
                WHERE singleton = 1 AND (
                    active_workspace_id = ?1 OR active_session_id IN (
                        SELECT product_session_id FROM product_sessions WHERE workspace_id = ?1
                    )
                )
                "#,
                params![workspace_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        let deleted = transaction
            .execute(
                "DELETE FROM product_workspaces WHERE workspace_id = ?1",
                params![workspace_id.to_string()],
            )
            .map_err(storage_error)?;
        if deleted != 1 {
            return Err(not_found("product workspace was not found"));
        }
        transaction.commit().map_err(storage_error)?;
        Ok(())
    }

    /// Read one page of a workspace's sessions.
    ///
    /// Codex alignment Phase 7. The sort key is
    /// `(archived_rank, updated_at DESC, product_session_id ASC)`, which
    /// `idx_product_sessions_workspace_page` covers end to end. A cursor names
    /// the last row already delivered, so resuming is a range scan rather than
    /// an offset that has to count past everything skipped.
    ///
    /// The chain validation that runs per session is why paging matters beyond
    /// response size: it used to run once per session in the workspace on every
    /// request, and now runs at most `limit` times.
    ///
    /// The page is assembled one rank group at a time rather than by one query
    /// spanning both. A keyset predicate that let the rank vary has to be a
    /// three-way disjunction, and SQLite cannot prove such a scan is already
    /// ordered — it materialises the matches and sorts them, at a cost that grows
    /// with the workspace, which is the cost paging exists to remove. Pinning the
    /// rank as an equality keeps the index scan itself ordered. Rank has two
    /// values, so a page costs at most two seeks.
    pub(super) fn list_sessions(
        &self,
        query: &ProductSessionPageQuery,
    ) -> Result<ProductSessionPage, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        require_workspace(&transaction, &query.workspace_id)?;
        // One row past the page: its existence is what distinguishes "the page
        // is full" from "there is more", without a second COUNT query that
        // could disagree with the page under concurrent writes.
        let probe_limit = limit_i64(query.limit.saturating_add(1))?;
        let ranks: &[i64] = if query.include_archived {
            &[SESSION_RANK_LIVE, SESSION_RANK_ARCHIVED]
        } else {
            &[SESSION_RANK_LIVE]
        };
        let mut sessions: Vec<ProductSession> = Vec::new();
        for &rank in ranks {
            // A cursor from a later group means this one is already fully
            // delivered; a cursor from an earlier group leaves this one untouched.
            let resume = match &query.cursor {
                Some(cursor) if cursor.archived_rank > rank => continue,
                Some(cursor) if cursor.archived_rank == rank => Some(cursor),
                _ => None,
            };
            let remaining = probe_limit - sessions.len() as i64;
            if remaining <= 0 {
                break;
            }
            let mut statement = transaction
                .prepare(&rank_page_sql(query, resume.is_some()))
                .map_err(storage_error)?;
            let owned = rank_page_params(query, rank, resume, remaining);
            let bound: Vec<&dyn rusqlite::ToSql> =
                owned.iter().map(|value| value.as_ref()).collect();
            let rows = statement
                .query_map(bound.as_slice(), raw_session_from_row)
                .map_err(storage_error)?;
            for row in rows {
                sessions.push(row.map_err(storage_error)?.into_product()?);
            }
        }
        let next_cursor = if sessions.len() > query.limit {
            sessions.truncate(query.limit);
            sessions.last().map(cursor_for_session)
        } else {
            None
        };
        for session in &sessions {
            validate_binding_integrity(&transaction, session)?;
        }
        transaction.commit().map_err(storage_error)?;
        Ok(ProductSessionPage {
            sessions,
            next_cursor,
        })
    }

    pub(super) fn create_session(
        &self,
        request: CreateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError> {
        let title = validate_title(request.title.as_deref())?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        require_workspace(&transaction, &request.workspace_id)?;
        enforce_table_limit(
            &transaction,
            "product_sessions",
            MAX_PRODUCT_SESSIONS,
            "product session limit reached",
        )?;
        let session_id = ProductSessionId::new();
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
                INSERT INTO product_sessions(
                    product_session_id, workspace_id, title, status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, 'idle', ?4, ?4)
                "#,
                params![
                    session_id.to_string(),
                    request.workspace_id.to_string(),
                    title,
                    now,
                ],
            )
            .map_err(storage_error)?;
        let (profile_id, model, max_steps) = default_session_model_values(&transaction)?;
        insert_session_model_config(
            &transaction,
            SessionModelConfigWrite {
                session_id: &session_id,
                profile_id: profile_id.as_deref(),
                model: &model,
                reasoning: ProductReasoningPreference::Default,
                max_steps,
                revision: 1,
                updated_at: &now,
            },
        )?;
        let session = get_session(&transaction, &session_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(session)
    }

    pub(super) fn update_session(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError> {
        let title = request
            .title
            .as_deref()
            .map(|value| validate_title(Some(value)))
            .transpose()?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let current = get_session(&transaction, session_id)?;
        validate_binding_integrity(&transaction, &current)?;
        if request.archived.is_some() && has_active_claim_for_session(&transaction, session_id)? {
            return Err(session_active(
                "product session cannot be archived while a turn is active",
            ));
        }
        let status = match request.archived {
            Some(true) => ProductSessionStatus::Archived,
            Some(false) if current.status == ProductSessionStatus::Archived => {
                ProductSessionStatus::Idle
            }
            _ => current.status,
        };
        let title = title.unwrap_or(current.title);
        transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET title = ?2, status = ?3, updated_at = ?4
                WHERE product_session_id = ?1
                "#,
                params![
                    session_id.to_string(),
                    title,
                    session_status_to_db(status),
                    now_rfc3339(),
                ],
            )
            .map_err(storage_error)?;
        let updated = get_session(&transaction, session_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn get_session_model_config(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionModelConfig, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_session(&transaction, session_id)?;
        ensure_session_model_config(&transaction, session_id)?;
        let config = get_session_model_config_in_transaction(&transaction, session_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(config)
    }

    pub(super) fn update_session_model_config(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionModelConfigRequest,
    ) -> Result<ProductSessionModelConfig, ProductStoreError> {
        let model = validate_required_text("session model", &request.model)?;
        if request.max_steps == 0 || request.max_steps > MAX_PRODUCT_MAX_STEPS {
            return Err(invalid("session max_steps is outside the supported range"));
        }
        if request
            .expected_revision
            .is_some_and(|revision| revision > i64::MAX as u64)
        {
            return Err(invalid(
                "session model revision is outside the supported range",
            ));
        }
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_session(&transaction, session_id)?;
        let current_revision = transaction
            .query_row(
                "SELECT revision FROM product_session_model_configs WHERE product_session_id = ?1",
                params![session_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|value| {
                u64::try_from(value)
                    .map_err(|_| binding_corrupt("session model revision is invalid"))
            })
            .transpose()?
            .unwrap_or(0);
        if let Some(expected_revision) = request.expected_revision
            && expected_revision != current_revision
        {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductSessionModelConfigConflict,
                format!(
                    "session model config revision mismatch (expected {expected_revision}, current {current_revision})"
                ),
            ));
        }
        let revision = current_revision
            .checked_add(1)
            .ok_or_else(|| invalid("session model revision overflow"))?;
        let now = now_rfc3339();
        let profile_id = request.profile_id.as_ref().map(ToString::to_string);
        insert_or_update_session_model_config(
            &transaction,
            SessionModelConfigWrite {
                session_id,
                profile_id: profile_id.as_deref(),
                model: &model,
                reasoning: request.reasoning,
                max_steps: request.max_steps,
                revision,
                updated_at: &now,
            },
        )?;
        let config = get_session_model_config_in_transaction(&transaction, session_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(config)
    }

    pub(super) fn list_session_run_models(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunModelView>, ProductStoreError> {
        let connection = self.database.connect()?;
        get_session(&connection, session_id)?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT product_session_id, ordinal, runtime_run_id, profile_id,
                       model, reasoning, max_steps,
                       provider_type, wire_protocol, endpoint, catalog_revision,
                       safe_config_digest,
                       context_window,
                       pricing_source, pricing_version, pricing_currency,
                       pricing_availability, per_mtok_prompt, per_mtok_completion,
                       per_mtok_cache_read
                FROM product_session_run_models
                WHERE product_session_id = ?1
                ORDER BY ordinal ASC
                LIMIT 2048
                "#,
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![session_id.to_string()], |row| {
                Ok(RawSessionRunModel {
                    product_session_id: row.get(0)?,
                    ordinal: row.get(1)?,
                    runtime_run_id: row.get(2)?,
                    profile_id: row.get(3)?,
                    model: row.get(4)?,
                    reasoning: row.get(5)?,
                    max_steps: row.get(6)?,
                    provider_type: row.get(7)?,
                    wire_protocol: row.get(8)?,
                    endpoint: row.get(9)?,
                    catalog_revision: row.get(10)?,
                    safe_config_digest: row.get(11)?,
                    context_window: row.get(12)?,
                    pricing_source: row.get(13)?,
                    pricing_version: row.get(14)?,
                    pricing_currency: row.get(15)?,
                    pricing_availability: row.get(16)?,
                    per_mtok_prompt: row.get(17)?,
                    per_mtok_completion: row.get(18)?,
                    per_mtok_cache_read: row.get(19)?,
                })
            })
            .map_err(storage_error)?;
        rows.map(|row| row.map_err(storage_error)?.into_product())
            .collect()
    }

    pub(super) fn delete_session(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_session(&transaction, session_id)?;
        if has_active_claim_for_session(&transaction, session_id)? {
            return Err(session_active(
                "product session cannot be deleted while a turn is active",
            ));
        }
        transaction
            .execute(
                r#"
                UPDATE product_preferences
                SET active_session_id = NULL, updated_at = ?2,
                    revision = revision + 1
                WHERE singleton = 1 AND active_session_id = ?1
                "#,
                params![session_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        let deleted = transaction
            .execute(
                "DELETE FROM product_sessions WHERE product_session_id = ?1",
                params![session_id.to_string()],
            )
            .map_err(storage_error)?;
        if deleted != 1 {
            return Err(not_found("product session was not found"));
        }
        transaction.commit().map_err(storage_error)?;
        Ok(())
    }

    pub(super) fn get_session_context(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionContext, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let session = get_session(&transaction, session_id)?;
        validate_binding_integrity(&transaction, &session)?;
        let workspace = get_workspace(&transaction, &session.workspace_id)?;
        let fork = get_fork_context(&transaction, &session)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ProductSessionContext {
            workspace,
            session,
            fork,
        })
    }

    pub(super) fn create_review(
        &self,
        record: CreateProductReviewRecord,
    ) -> Result<(ProductReview, bool), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        // Validate both foreign-key identities before any idempotency lookup so
        // a stale client cannot create a review row detached from the catalog.
        let session = get_session(&transaction, &record.product_session_id)?;
        if session.workspace_id != record.workspace_id {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductSessionWorkspaceMismatch,
                "review workspace does not match the product session",
            ));
        }
        get_workspace(&transaction, &record.workspace_id)?;

        let idempotency_key = record.idempotency_key.as_deref();
        if let Some(key) = idempotency_key {
            validate_review_idempotency_key(key)?;
            if let Some(existing) = transaction
                .query_row(
                    "SELECT review_id, target_digest FROM product_reviews WHERE product_session_id = ?1 AND idempotency_key = ?2",
                    params![record.product_session_id.to_string(), key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(storage_error)?
            {
                if existing.1 != record.target.digest {
                    return Err(ProductStoreError::new(
                        ProductErrorCode::ReviewConflict,
                        "review idempotency key is already bound to another target",
                    ));
                }
                let existing_id = parse_product_id(&existing.0, "review id")?;
                let review = get_review_in_transaction(&transaction, &existing_id)?;
                transaction.commit().map_err(storage_error)?;
                return Ok((review, true));
            }
        }

        if let Some(existing_id) = transaction
            .query_row(
                "SELECT review_id FROM product_reviews WHERE product_session_id = ?1 AND target_digest = ?2 AND status IN ('queued', 'running') ORDER BY created_at DESC LIMIT 1",
                params![record.product_session_id.to_string(), record.target.digest],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
        {
            let existing_id = parse_product_id(&existing_id, "review id")?;
            let review = get_review_in_transaction(&transaction, &existing_id)?;
            transaction.commit().map_err(storage_error)?;
            return Ok((review, true));
        }

        let target_summary_json = serde_json::to_string(&record.target).map_err(storage_error)?;
        let target_spec_json = serde_json::to_string(&record.target_spec).map_err(storage_error)?;
        let state_root = record
            .state_root
            .to_str()
            .ok_or_else(|| invalid("review state root must be valid UTF-8"))?;
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
                INSERT INTO product_reviews(
                    review_id, product_session_id, workspace_id,
                    target_kind, target_revision, resolved_base, target_digest,
                    target_summary_json, target_spec_json, state_root,
                    status, idempotency_key, captured_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                          'queued', ?11, ?12, ?12, ?12)
                "#,
                params![
                    record.review_id.to_string(),
                    record.product_session_id.to_string(),
                    record.workspace_id.to_string(),
                    review_target_kind_to_db(record.target_spec.kind),
                    record.target_spec.revision,
                    record.target.resolved_base,
                    record.target.digest,
                    target_summary_json,
                    target_spec_json,
                    state_root,
                    idempotency_key,
                    now,
                ],
            )
            .map_err(|error| {
                if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
                    ProductStoreError::new(
                        ProductErrorCode::ReviewConflict,
                        "an active review already exists for this target",
                    )
                } else {
                    storage_error(error)
                }
            })?;
        let review = get_review_in_transaction(&transaction, &record.review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok((review, false))
    }

    pub(super) fn list_reviews(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductReview>, ProductStoreError> {
        let connection = self.database.connect()?;
        get_session(&connection, session_id)?;
        let mut statement = connection
            .prepare(
                "SELECT review_id FROM product_reviews WHERE product_session_id = ?1 ORDER BY created_at DESC, review_id DESC LIMIT 256",
            )
            .map_err(storage_error)?;
        let ids = statement
            .query_map(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        ids.into_iter()
            .map(|id| {
                let id = parse_product_id(&id, "review id")?;
                get_review_in_transaction(&connection, &id)
            })
            .collect()
    }

    pub(super) fn get_review(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let connection = self.database.connect()?;
        get_review_in_transaction(&connection, review_id)
    }

    pub(super) fn bind_review_runtime(
        &self,
        review_id: &ProductReviewId,
        runtime_session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
    ) -> Result<ProductReview, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let current = get_review_in_transaction(&transaction, review_id)?;
        if let (Some(current_session), Some(current_job), Some(current_run)) =
            (current.runtime_session_id, current.job_id, current.run_id)
        {
            if current_session == runtime_session_id
                && current_job == job_id
                && current_run == run_id
            {
                transaction.commit().map_err(storage_error)?;
                return Ok(current);
            }
            return Err(ProductStoreError::new(
                ProductErrorCode::ReviewConflict,
                "review is already bound to a different runtime run",
            ));
        }
        if current.status.is_terminal() {
            return Err(ProductStoreError::new(
                ProductErrorCode::ReviewConflict,
                "review is already terminal",
            ));
        }
        transaction
            .execute(
                "UPDATE product_reviews SET status = 'running', runtime_session_id = ?2, job_id = ?3, run_id = ?4, updated_at = ?5 WHERE review_id = ?1 AND status IN ('queued', 'running')",
                params![
                    review_id.to_string(),
                    runtime_session_id.to_string(),
                    job_id.to_string(),
                    run_id.to_string(),
                    now_rfc3339()
                ],
            )
            .map_err(storage_error)?;
        let updated = get_review_in_transaction(&transaction, review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn finalize_review(
        &self,
        review_id: &ProductReviewId,
        result: ReviewResult,
    ) -> Result<ProductReview, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let current = get_review_in_transaction(&transaction, review_id)?;
        if current.result.is_some() {
            transaction.commit().map_err(storage_error)?;
            return Ok(current);
        }
        let conclusion = result.conclusion.clone();
        let status = review_status_from_conclusion(&conclusion);
        let result_json = serde_json::to_string(&result).map_err(storage_error)?;
        let now = now_rfc3339();
        transaction
            .execute(
                "UPDATE product_reviews SET status = ?2, conclusion = ?3, result_json = ?4, findings_count = ?5, unchecked_count = ?6, warnings_count = ?7, updated_at = ?8, finalized_at = ?8 WHERE review_id = ?1 AND result_json IS NULL",
                params![
                    review_id.to_string(),
                    status.as_str(),
                    review_conclusion_to_db(&conclusion),
                    result_json,
                    limit_i64(result.findings.len())?,
                    limit_i64(result.unchecked.len())?,
                    limit_i64(result.warnings.len())?,
                    now,
                ],
            )
            .map_err(storage_error)?;
        for finding in &result.findings {
            let finding_json = serde_json::to_string(finding).map_err(storage_error)?;
            let sort_key = review_finding_sort_key(finding);
            transaction
                .execute(
                    "INSERT OR IGNORE INTO product_review_findings(review_id, finding_id, sort_key, finding_json, location_status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        review_id.to_string(),
                        finding.finding_id,
                        sort_key,
                        finding_json,
                        format!("{:?}", finding.location_status).to_ascii_lowercase(),
                        now,
                    ],
                )
                .map_err(storage_error)?;
        }
        let updated = get_review_in_transaction(&transaction, review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn cancel_review(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_review_in_transaction(&transaction, review_id)?;
        transaction
            .execute(
                "UPDATE product_reviews SET status = 'cancelled', conclusion = 'cancelled', updated_at = ?2, finalized_at = COALESCE(finalized_at, ?2) WHERE review_id = ?1 AND status IN ('queued', 'running') AND result_json IS NULL",
                params![review_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        let review = get_review_in_transaction(&transaction, review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(review)
    }

    pub(super) fn mark_review_needs_attention(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_review_in_transaction(&transaction, review_id)?;
        transaction
            .execute(
                "UPDATE product_reviews SET status = 'needs_attention', updated_at = ?2 WHERE review_id = ?1 AND status IN ('pass', 'findings', 'partial', 'stale')",
                params![review_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        let review = get_review_in_transaction(&transaction, review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(review)
    }

    pub(super) fn mark_review_unavailable(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_review_in_transaction(&transaction, review_id)?;
        transaction
            .execute(
                "UPDATE product_reviews SET status = 'unavailable', conclusion = 'unavailable', updated_at = ?2, finalized_at = COALESCE(finalized_at, ?2) WHERE review_id = ?1 AND status IN ('queued', 'running') AND result_json IS NULL",
                params![review_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        let review = get_review_in_transaction(&transaction, review_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(review)
    }

    pub(super) fn list_review_findings(
        &self,
        review_id: &ProductReviewId,
        query: ProductReviewFindingsQuery,
    ) -> Result<ProductReviewFindingsResponse, ProductStoreError> {
        let connection = self.database.connect()?;
        get_review_in_transaction(&connection, review_id)?;
        let limit = query.limit.unwrap_or(64).clamp(1, 128);
        let cursor = query.cursor.unwrap_or(0);
        let mut statement = connection
            .prepare(
                "SELECT finding_id, sort_key, finding_json FROM product_review_findings WHERE review_id = ?1 ORDER BY sort_key ASC, finding_id ASC LIMIT ?2 OFFSET ?3",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    review_id.to_string(),
                    limit_i64(limit.saturating_add(1))?,
                    limit_i64(cursor)?
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let has_more = rows.len() > limit;
        let findings = rows
            .into_iter()
            .take(limit)
            .map(|(_id, sort_key, json)| {
                let finding = serde_json::from_str(&json)
                    .map_err(|_| storage_error("persisted review finding is invalid"))?;
                Ok(ProductReviewFinding { finding, sort_key })
            })
            .collect::<Result<Vec<_>, ProductStoreError>>()?;
        Ok(ProductReviewFindingsResponse {
            findings,
            next_cursor: has_more.then_some(cursor.saturating_add(limit)),
        })
    }

    pub(super) fn list_run_bindings(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunBinding>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let session = get_session(&transaction, session_id)?;
        let bindings = list_and_validate_bindings(&transaction, &session)?;
        transaction.commit().map_err(storage_error)?;
        Ok(bindings)
    }

    pub(super) fn create_fork(
        &self,
        request: CreateProductForkRequest,
        boundary: VerifiedProductForkBoundary,
    ) -> Result<(ProductSession, ProductFork, bool), ProductStoreError> {
        let idempotency_key = validate_fork_idempotency_key(&request.idempotency_key)?;
        if request.fork_at_run_id != boundary.source_runtime_run_id {
            return Err(fork_source_invalid(
                "the verified fork boundary does not match the requested runtime run",
            ));
        }
        let digest = fork_request_digest(&request);
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;

        // Check an idempotent replay before loading the parent. This means a
        // network retry still returns the original child even if the user has
        // since removed the parent catalog row.
        if let Some((child, fork)) = replay_fork_if_exists(
            &transaction,
            &boundary.parent_product_session_id,
            &idempotency_key,
            &digest,
        )? {
            transaction.commit().map_err(storage_error)?;
            return Ok((child, fork, true));
        }

        let parent = get_session(&transaction, &boundary.parent_product_session_id)?;
        if parent.workspace_id != boundary.parent_workspace_id {
            return Err(fork_source_invalid(
                "the verified fork workspace does not match the parent session",
            ));
        }
        if has_active_claim_for_session(&transaction, &parent.id)?
            || parent.status == ProductSessionStatus::Running
        {
            return Err(session_active(
                "a product session can only be forked after its active turn reaches a terminal boundary",
            ));
        }
        if parent.status != ProductSessionStatus::Idle {
            return Err(fork_source_invalid(
                "only an idle product session with a final durable run can be forked",
            ));
        }

        ensure_session_model_config(&transaction, &parent.id)?;
        let parent_model_config =
            get_session_model_config_in_transaction(&transaction, &parent.id)?;

        let parent_bindings = list_and_validate_bindings(&transaction, &parent)?;
        let Some(source_index) = parent_bindings.iter().position(|binding| {
            binding.runtime_session_id == boundary.source_runtime_session_id
                && binding.runtime_job_id == boundary.source_runtime_job_id
                && binding.runtime_run_id == boundary.source_runtime_run_id
        }) else {
            return Err(fork_source_invalid(
                "the verified runtime run is not an immutable binding of the parent session",
            ));
        };

        let mut inherited_runs = get_fork_context(&transaction, &parent)?
            .map(|context| context.inherited_runs)
            .unwrap_or_default();
        for binding in parent_bindings.iter().take(source_index + 1) {
            inherited_runs.push(ProductForkInheritedRun {
                ordinal: u64::try_from(inherited_runs.len() + 1).map_err(storage_error)?,
                source_product_session_id: parent.id.clone(),
                runtime_session_id: binding.runtime_session_id,
                runtime_job_id: binding.runtime_job_id,
                runtime_run_id: binding.runtime_run_id,
                through_event_seq: (binding.runtime_run_id == boundary.source_runtime_run_id)
                    .then_some(boundary.fork_at_event_seq),
            });
        }
        if inherited_runs.is_empty() || inherited_runs.len() > MAX_PRODUCT_FORK_INHERITED_RUNS {
            return Err(fork_source_invalid(
                "the fork source has no bounded inherited runtime history",
            ));
        }

        let title = match request.title.as_deref() {
            Some(title) => validate_title(Some(title))?,
            None => default_fork_title(&parent.title),
        };
        enforce_table_limit(
            &transaction,
            "product_sessions",
            MAX_PRODUCT_SESSIONS,
            "product session limit reached",
        )?;
        let child_id = ProductSessionId::new();
        let fork_id = ProductForkId::new();
        let now = now_rfc3339();
        let parent_title = parent.title.clone();
        transaction
            .execute(
                r#"
                INSERT INTO product_sessions(
                    product_session_id, workspace_id, title, status,
                    parent_session_id, fork_point_run_id, fork_point_seq,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, 'idle', ?4, ?5, ?6, ?7, ?7)
                "#,
                params![
                    child_id.to_string(),
                    parent.workspace_id.to_string(),
                    title,
                    parent.id.to_string(),
                    boundary.source_runtime_run_id.to_string(),
                    i64::try_from(boundary.fork_at_event_seq).map_err(|_| {
                        ProductStoreError::new(
                            ProductErrorCode::ProductInvalidInput,
                            "fork terminal event sequence exceeds SQLite integer range",
                        )
                    })?,
                    now,
                ],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                r#"
                INSERT INTO product_session_forks(
                    fork_id, parent_product_session_id, child_product_session_id,
                    parent_workspace_id, parent_title, source_runtime_session_id,
                    source_runtime_job_id, source_runtime_run_id, fork_at_event_seq,
                    idempotency_key, request_digest, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
                params![
                    fork_id.to_string(),
                    parent.id.to_string(),
                    child_id.to_string(),
                    parent.workspace_id.to_string(),
                    parent_title.clone(),
                    boundary.source_runtime_session_id.to_string(),
                    boundary.source_runtime_job_id.to_string(),
                    boundary.source_runtime_run_id.to_string(),
                    i64::try_from(boundary.fork_at_event_seq).map_err(|_| {
                        ProductStoreError::new(
                            ProductErrorCode::ProductInvalidInput,
                            "fork terminal event sequence exceeds SQLite integer range",
                        )
                    })?,
                    idempotency_key,
                    digest,
                    now,
                ],
            )
            .map_err(storage_error)?;
        for inherited in &inherited_runs {
            transaction
                .execute(
                    r#"
                    INSERT INTO product_fork_inherited_runs(
                        fork_id, ordinal, source_product_session_id,
                        runtime_session_id, runtime_job_id, runtime_run_id,
                        through_event_seq
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        fork_id.to_string(),
                        i64::try_from(inherited.ordinal).map_err(storage_error)?,
                        inherited.source_product_session_id.to_string(),
                        inherited.runtime_session_id.to_string(),
                        inherited.runtime_job_id.to_string(),
                        inherited.runtime_run_id.to_string(),
                        inherited
                            .through_event_seq
                            .map(i64::try_from)
                            .transpose()
                            .map_err(storage_error)?,
                    ],
                )
                .map_err(storage_error)?;
        }

        let parent_profile_id = parent_model_config
            .profile_id
            .as_ref()
            .map(ToString::to_string);
        insert_session_model_config(
            &transaction,
            SessionModelConfigWrite {
                session_id: &child_id,
                profile_id: parent_profile_id.as_deref(),
                model: &parent_model_config.model,
                reasoning: parent_model_config.reasoning,
                max_steps: parent_model_config.max_steps,
                revision: 1,
                updated_at: &now,
            },
        )?;

        let child = get_session(&transaction, &child_id)?;
        let fork = ProductFork {
            id: fork_id,
            parent_product_session_id: parent.id,
            child_product_session_id: child_id,
            parent_workspace_id: boundary.parent_workspace_id,
            parent_title,
            source_runtime_session_id: boundary.source_runtime_session_id,
            source_runtime_job_id: boundary.source_runtime_job_id,
            source_runtime_run_id: boundary.source_runtime_run_id,
            fork_at_event_seq: boundary.fork_at_event_seq,
            idempotency_key,
            created_at: now,
        };
        transaction.commit().map_err(storage_error)?;
        Ok((child, fork, false))
    }

    pub(super) fn replay_fork(
        &self,
        parent_session_id: &ProductSessionId,
        request: &CreateProductForkRequest,
    ) -> Result<Option<(ProductSession, ProductFork)>, ProductStoreError> {
        let idempotency_key = validate_fork_idempotency_key(&request.idempotency_key)?;
        let digest = fork_request_digest(request);
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let replay =
            replay_fork_if_exists(&transaction, parent_session_id, &idempotency_key, &digest)?;
        transaction.commit().map_err(storage_error)?;
        Ok(replay)
    }

    pub(super) fn list_forks(
        &self,
        parent_session_id: &ProductSessionId,
    ) -> Result<Vec<ProductFork>, ProductStoreError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT fork_id, parent_product_session_id, child_product_session_id,
                       parent_workspace_id, parent_title, source_runtime_session_id,
                       source_runtime_job_id, source_runtime_run_id, fork_at_event_seq,
                       idempotency_key, request_digest, created_at
                FROM product_session_forks
                WHERE parent_product_session_id = ?1
                ORDER BY created_at ASC, fork_id ASC
                LIMIT ?2
                "#,
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    parent_session_id.to_string(),
                    limit_i64(MAX_PRODUCT_SESSIONS)?
                ],
                raw_fork_from_row,
            )
            .map_err(storage_error)?;
        let mut forks = Vec::new();
        for row in rows {
            forks.push(row.map_err(storage_error)?.into_stored()?.fork);
        }
        if forks.is_empty() && get_session(&connection, parent_session_id).is_err() {
            return Err(not_found("product session was not found"));
        }
        Ok(forks)
    }

    pub(super) fn claim_session_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductTurnClaim, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let mut session = get_session(&transaction, session_id)?;
        if session.status == ProductSessionStatus::Archived {
            return Err(invalid("archived product sessions cannot start a turn"));
        }
        if session.status == ProductSessionStatus::NeedsAttention {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "product session requires runtime recovery before starting a turn",
            ));
        }
        if has_active_claim_for_session(&transaction, session_id)? {
            return Err(session_active("product session already has an active turn"));
        }
        validate_binding_integrity(&transaction, &session)?;
        let workspace = get_workspace(&transaction, &session.workspace_id)?;
        let fork = get_fork_context(&transaction, &session)?;
        ensure_session_model_config(&transaction, session_id)?;
        let model_config = get_session_model_config_in_transaction(&transaction, session_id)?;
        let previous_status = session.status;
        let previous_binding = session.runtime_binding.clone();
        let claim_id = ProductTurnClaimId::new();
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
                INSERT INTO product_turn_claims(claim_id, product_session_id, claimed_at)
                VALUES (?1, ?2, ?3)
                "#,
                params![claim_id.to_string(), session_id.to_string(), now],
            )
            .map_err(storage_error)?;
        let updated = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = 'running', updated_at = ?2
                WHERE product_session_id = ?1
                  AND status NOT IN ('archived', 'needs_attention')
                "#,
                params![session_id.to_string(), now],
            )
            .map_err(storage_error)?;
        if updated != 1 {
            return Err(session_active(
                "product session turn claim was not acquired",
            ));
        }
        session.status = ProductSessionStatus::Running;
        session.updated_at = now;
        transaction.commit().map_err(storage_error)?;
        Ok(ProductTurnClaim {
            claim_id,
            context: ProductSessionContext {
                workspace,
                session,
                fork,
            },
            previous_status,
            previous_binding,
            model_config,
        })
    }

    pub(super) fn commit_run_binding(
        &self,
        binding: CommitProductRunBinding,
    ) -> Result<ProductSessionRunBinding, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let claimed = transaction
            .query_row(
                "SELECT product_session_id, followup_control_id FROM product_turn_claims WHERE claim_id = ?1",
                params![binding.claim_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(storage_error)?
            .map(|(session_id, control_id)| {
                Ok((
                    parse_product_id::<ProductSessionId>(&session_id, "product session id")?,
                    control_id
                        .as_deref()
                        .map(|value| parse_product_id(value, "follow-up control id"))
                        .transpose()?,
                ))
            })
            .transpose()?;
        if claimed.as_ref().map(|(session_id, _)| session_id) != Some(&binding.product_session_id)
            || claimed
                .as_ref()
                .and_then(|(_, control_id)| control_id.as_ref())
                != binding.followup_control_id.as_ref()
        {
            return Err(resume_conflict(
                "product session turn claim is missing or does not match",
            ));
        }

        let session = get_session(&transaction, &binding.product_session_id)?;
        validate_binding_integrity(&transaction, &session)?;
        if let Some(existing) = find_binding_by_runtime_run(&transaction, binding.runtime_run_id)? {
            if binding_matches_commit(&existing, &binding)
                && session.runtime_binding.as_ref().is_some_and(|latest| {
                    latest.ordinal == existing.ordinal
                        && latest.latest_run_id == existing.runtime_run_id
                })
            {
                transaction.commit().map_err(storage_error)?;
                return Ok(existing);
            }
            return Err(resume_conflict(
                "runtime run is already bound to another product turn",
            ));
        }

        let ordinal = next_binding_ordinal(&session)?;
        validate_commit_chain(&session, &binding)?;
        claim_runtime_ownership(
            &transaction,
            &binding.product_session_id,
            binding.runtime_session_id,
            binding.runtime_job_id,
            ProductErrorCode::ProductSessionResumeConflict,
        )?;
        let created = insert_run_binding(
            &transaction,
            NewRunBinding {
                product_session_id: binding.product_session_id.clone(),
                ordinal,
                runtime_session_id: binding.runtime_session_id,
                runtime_job_id: binding.runtime_job_id,
                runtime_run_id: binding.runtime_run_id,
                resumed_from_run_id: binding.resumed_from_run_id,
                migration_receipt_id: None,
            },
        )?;
        update_latest_binding_cas(&transaction, &session, &created)?;
        insert_session_run_model_snapshot(
            &transaction,
            &binding.product_session_id,
            created.ordinal,
            &binding.runtime_run_id,
            &binding.model_config,
            binding.run_model_snapshot.as_ref(),
        )?;
        if let Some(control_id) = &binding.followup_control_id {
            let claimable = transaction
                .query_row(
                    r#"
                    SELECT 1
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND control_id = ?2
                      AND kind = 'followup' AND status = 'accepted' AND run_id = ?3
                    "#,
                    params![
                        binding.product_session_id.to_string(),
                        control_id.to_string(),
                        binding.runtime_run_id.to_string(),
                    ],
                    |_| Ok(()),
                )
                .optional()
                .map_err(storage_error)?;
            if claimable.is_none() {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductControlRejected,
                    "queued follow-up was no longer claimable for the new run",
                ));
            }
            // A binding only proves that preparation reached a durable
            // reservation. The control becomes `applied` after the successor
            // has emitted and persisted `followup_dequeued` at its actual
            // run-start boundary.
        }
        transaction.commit().map_err(storage_error)?;
        Ok(created)
    }

    /// Reinsert the catalog rows one session's ownership records describe.
    ///
    /// Codex alignment Phase 5: the counterpart to the runtime index backfill.
    /// A session the catalog still knows is left completely alone — recovery is
    /// for holes, and a half-merge of on-disk records into a live chain is how
    /// you get a session that reads as corrupt. Everything lands in one
    /// transaction: a session without its workspace, or a binding without its
    /// ownership rows, would fail a foreign key on the next write and be worse
    /// than no recovery at all.
    ///
    /// The chain is renumbered from 1 and relinked as it is inserted, because
    /// every read of `product_session_runs` requires contiguous ordinals whose
    /// `resumed_from_run_id` points at the previous run. A lost record therefore
    /// shifts later ordinals rather than leaving a gap that makes the whole
    /// session unreadable. Runs that would break the chain's runtime identity —
    /// a different runtime session or job than the first run — are dropped,
    /// since the reader rejects those outright.
    pub(super) fn recover_session_ownership(
        &self,
        ownership: &RecoverProductSessionOwnership,
    ) -> Result<ProductSessionRecovery, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let now = now_rfc3339();

        // A session the catalog already holds is authoritative. On-disk records
        // are a snapshot from when the run started and know nothing about
        // renames, archiving, or later turns, so merging them in would undo
        // live state.
        let session_exists = transaction
            .query_row(
                "SELECT 1 FROM product_sessions WHERE product_session_id = ?1",
                params![ownership.product_session_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage_error)?
            .is_some();
        if session_exists {
            return Ok(ProductSessionRecovery::AlreadyPresent);
        }

        // The chain's runtime identity comes from its first run, and the reader
        // requires every later run to share it.
        let Some(first_run) = ownership.runs.iter().min_by_key(|run| run.recorded_ordinal) else {
            return Ok(ProductSessionRecovery::Skipped);
        };
        let chain_session_id = first_run.runtime_session_id;
        let chain_job_id = first_run.runtime_job_id;

        // `runtime_session_id` and `runtime_job_id` are primary keys of their
        // owner tables, so a runtime identity already owned by a different
        // product session cannot be re-owned here. That means the records on
        // disk lost their claim — most likely the session was deleted and its
        // runtime ids reused by a migration — and the whole record is stale.
        if runtime_owner_conflicts(
            &transaction,
            "SELECT product_session_id FROM product_runtime_session_owners WHERE runtime_session_id = ?1",
            &chain_session_id.to_string(),
            &ownership.product_session_id,
        )? || runtime_owner_conflicts(
            &transaction,
            "SELECT product_session_id FROM product_runtime_job_owners WHERE runtime_job_id = ?1",
            &chain_job_id.to_string(),
            &ownership.product_session_id,
        )? {
            return Ok(ProductSessionRecovery::Skipped);
        }

        // The canonical key is unique, so a workspace re-registered under a new
        // id since the run wrote its record must win over the recorded id;
        // inserting the stale id would duplicate the root under two ids.
        let workspace_id = match find_workspace_by_key(&transaction, &ownership.canonical_key)? {
            Some(existing) => existing.id,
            None => {
                transaction
                    .execute(
                        r#"
                        INSERT INTO product_workspaces(
                            workspace_id, canonical_root, canonical_key, kind, display_name,
                            pinned, last_opened_at, created_at, updated_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6, ?7)
                        ON CONFLICT(workspace_id) DO NOTHING
                        "#,
                        params![
                            ownership.workspace_id.to_string(),
                            ownership.canonical_root_text,
                            ownership.canonical_key,
                            workspace_kind_to_db(ownership.workspace_kind),
                            ownership.workspace_display_name,
                            ownership.session_created_at,
                            now,
                        ],
                    )
                    .map_err(storage_error)?;
                ownership.workspace_id.clone()
            }
        };

        transaction
            .execute(
                r#"
                INSERT INTO product_sessions(
                    product_session_id, workspace_id, title, status,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    ownership.product_session_id.to_string(),
                    workspace_id.to_string(),
                    ownership.session_title,
                    session_status_to_db(ownership.status),
                    ownership.session_created_at,
                    now,
                ],
            )
            .map_err(storage_error)?;
        // Without a model config row the session cannot be opened, so a
        // recovered session gets the current defaults rather than a
        // half-usable row.
        let (profile_id, model, max_steps) = default_session_model_values(&transaction)?;
        insert_session_model_config(
            &transaction,
            SessionModelConfigWrite {
                session_id: &ownership.product_session_id,
                profile_id: profile_id.as_deref(),
                model: &model,
                reasoning: ProductReasoningPreference::Default,
                max_steps,
                revision: 1,
                updated_at: &now,
            },
        )?;

        transaction
            .execute(
                r#"
                INSERT INTO product_runtime_session_owners(runtime_session_id, product_session_id)
                VALUES (?1, ?2)
                ON CONFLICT(runtime_session_id) DO NOTHING
                "#,
                params![
                    chain_session_id.to_string(),
                    ownership.product_session_id.to_string(),
                ],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                r#"
                INSERT INTO product_runtime_job_owners(
                    runtime_job_id, runtime_session_id, product_session_id
                ) VALUES (?1, ?2, ?3)
                ON CONFLICT(runtime_job_id) DO NOTHING
                "#,
                params![
                    chain_job_id.to_string(),
                    chain_session_id.to_string(),
                    ownership.product_session_id.to_string(),
                ],
            )
            .map_err(storage_error)?;

        let mut runs = ownership.runs.clone();
        runs.sort_by_key(|run| run.recorded_ordinal);

        let mut ordinal: i64 = 0;
        let mut previous_run_id: Option<RunId> = None;
        for run in &runs {
            // The reader rejects a chain whose runs disagree on their runtime
            // session or job, so a record that disagrees with the first run is
            // not something this session can hold.
            if run.runtime_session_id != chain_session_id || run.runtime_job_id != chain_job_id {
                continue;
            }
            // A run bound to a different session means this record is stale, not
            // that the catalog is wrong. Skip it rather than fight the live row
            // for the `runtime_run_id` unique index.
            let bound_elsewhere = transaction
                .query_row(
                    "SELECT 1 FROM product_session_runs WHERE runtime_run_id = ?1",
                    params![run.runtime_run_id.to_string()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(storage_error)?
                .is_some();
            if bound_elsewhere {
                continue;
            }

            ordinal += 1;
            transaction
                .execute(
                    r#"
                    INSERT INTO product_session_runs(
                        product_session_id, ordinal, runtime_session_id, runtime_job_id,
                        runtime_run_id, resumed_from_run_id, bound_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        ownership.product_session_id.to_string(),
                        ordinal,
                        chain_session_id.to_string(),
                        chain_job_id.to_string(),
                        run.runtime_run_id.to_string(),
                        previous_run_id.map(|id| id.to_string()),
                        run.bound_at,
                    ],
                )
                .map_err(storage_error)?;
            previous_run_id = Some(run.runtime_run_id);
        }

        // A session with no runs cannot be opened and cannot be resumed, so
        // there is nothing to recover — better to leave the hole than to add a
        // row every listing has to explain.
        if ordinal == 0 {
            transaction.rollback().map_err(storage_error)?;
            return Ok(ProductSessionRecovery::Skipped);
        }

        transaction
            .execute(
                r#"
                UPDATE product_sessions SET
                    latest_ordinal = latest.ordinal,
                    runtime_session_id = latest.runtime_session_id,
                    latest_job_id = latest.runtime_job_id,
                    latest_run_id = latest.runtime_run_id,
                    updated_at = ?2
                FROM (
                    SELECT ordinal, runtime_session_id, runtime_job_id, runtime_run_id
                    FROM product_session_runs
                    WHERE product_session_id = ?1
                    ORDER BY ordinal DESC LIMIT 1
                ) AS latest
                WHERE product_session_id = ?1
                "#,
                params![ownership.product_session_id.to_string(), now],
            )
            .map_err(storage_error)?;

        transaction.commit().map_err(storage_error)?;
        Ok(ProductSessionRecovery::Recovered {
            runs: usize::try_from(ordinal).unwrap_or(usize::MAX),
        })
    }

    pub(super) fn finish_session_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        status: ProductSessionStatus,
    ) -> Result<(), ProductStoreError> {
        if status == ProductSessionStatus::Running {
            return Err(invalid("a finished product turn cannot remain running"));
        }
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = transaction
            .query_row(
                "SELECT product_session_id FROM product_turn_claims WHERE claim_id = ?1",
                params![claim_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|value| parse_product_id::<ProductSessionId>(&value, "product session id"))
            .transpose()?
            .ok_or_else(|| {
                resume_conflict("product session turn claim is missing or no longer active")
            })?;
        let deleted = transaction
            .execute(
                "DELETE FROM product_turn_claims WHERE claim_id = ?1 AND product_session_id = ?2",
                params![claim_id.to_string(), session_id.to_string()],
            )
            .map_err(storage_error)?;
        if deleted != 1 {
            return Err(resume_conflict(
                "product session turn claim is missing or no longer active",
            ));
        }
        let updated = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = ?2, updated_at = ?3
                WHERE product_session_id = ?1
                "#,
                params![
                    session_id.to_string(),
                    session_status_to_db(status),
                    now_rfc3339(),
                ],
            )
            .map_err(storage_error)?;
        if updated != 1 {
            return Err(binding_corrupt("turn claim references a missing session"));
        }
        transaction.commit().map_err(storage_error)?;
        Ok(())
    }

    /// Atomically replace a successfully-final turn claim with the oldest
    /// queued follow-up claim. Keeping the old claim until the new one exists
    /// closes the final/enqueue race and means concurrent drain attempts can
    /// never observe the session as independently runnable.
    pub(super) fn finish_session_turn_and_claim_followup(
        &self,
        claim_id: &ProductTurnClaimId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = active_turn_session_id(&transaction, claim_id)?;
        let mut session = get_session(&transaction, &session_id)?;
        if session.status != ProductSessionStatus::Running {
            return Err(resume_conflict(
                "product session turn claim does not own a running session",
            ));
        }
        validate_binding_integrity(&transaction, &session)?;
        let workspace = get_workspace(&transaction, &session.workspace_id)?;
        let fork = get_fork_context(&transaction, &session)?;
        ensure_session_model_config(&transaction, &session_id)?;
        let model_config = get_session_model_config_in_transaction(&transaction, &session_id)?;
        let previous_binding = session.runtime_binding.clone();

        let Some(pending) = pending_followup_for_session(&transaction, &session_id)? else {
            release_turn_claim_with_status(
                &transaction,
                claim_id,
                &session_id,
                ProductSessionStatus::Idle,
            )?;
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        };

        let now = now_rfc3339();
        let accepted = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'accepted', applied_at = NULL
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND kind = 'followup' AND status = 'pending' AND run_id IS NULL
                "#,
                params![session_id.to_string(), pending.id.to_string()],
            )
            .map_err(storage_error)?;
        if accepted != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "queued follow-up changed while the previous turn was finishing",
            ));
        }

        let next_claim_id = ProductTurnClaimId::new();
        let deleted = transaction
            .execute(
                "DELETE FROM product_turn_claims WHERE claim_id = ?1 AND product_session_id = ?2",
                params![claim_id.to_string(), session_id.to_string()],
            )
            .map_err(storage_error)?;
        if deleted != 1 {
            return Err(resume_conflict(
                "product session turn claim is missing or no longer active",
            ));
        }
        transaction
            .execute(
                r#"
                INSERT INTO product_turn_claims(
                    claim_id, product_session_id, claimed_at, followup_control_id
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    next_claim_id.to_string(),
                    session_id.to_string(),
                    now,
                    pending.id.to_string(),
                ],
            )
            .map_err(storage_error)?;
        let updated = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = 'running', updated_at = ?2
                WHERE product_session_id = ?1
                "#,
                params![session_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        if updated != 1 {
            return Err(binding_corrupt(
                "product turn claim references a missing session",
            ));
        }
        session.status = ProductSessionStatus::Running;
        session.updated_at = now;
        let control = get_control_in_transaction(&transaction, &session_id, &pending.id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(Some(ProductFollowupTurnClaim {
            control,
            turn: ProductTurnClaim {
                claim_id: next_claim_id,
                context: ProductSessionContext {
                    workspace,
                    session,
                    fork,
                },
                previous_status: ProductSessionStatus::Idle,
                previous_binding,
                model_config,
            },
        }))
    }

    /// Close steers that made it into the current turn's control channel or
    /// safe-point bookkeeping but never reached a model-turn start. This must
    /// happen while the old turn claim is still active, before the coordinator
    /// makes its terminal runtime artifact visible. A successor follow-up can
    /// therefore never inherit a stale steer as pending work.
    pub(super) fn drop_unapplied_steers_for_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: RunId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = active_turn_session_id(&transaction, claim_id)?;
        let steers = unapplied_steers_for_session(&transaction, &session_id)?;
        transition_unapplied_steers_in_transaction(
            &transaction,
            &session_id,
            Some(run_id),
            reason,
            steers.len(),
        )?;
        transaction.commit().map_err(storage_error)?;
        Ok(steers)
    }

    /// Close an indeterminate or non-final turn and atomically classify every
    /// control that did not reach its terminal lifecycle at that boundary. No
    /// later request can be silently swept into the old run after its claim is
    /// released.
    pub(super) fn finish_session_turn_and_abandon_pending_controls(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: Option<RunId>,
        status: ProductSessionStatus,
        reason: &str,
    ) -> Result<ProductTurnControlFinish, ProductStoreError> {
        if status == ProductSessionStatus::Running {
            return Err(invalid("a completed product turn cannot remain running"));
        }
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = active_turn_session_id(&transaction, claim_id)?;
        let dropped_steers = unapplied_steers_for_session(&transaction, &session_id)?;
        let abandoned_followups = unapplied_followups_for_session(&transaction, &session_id)?;
        transition_unapplied_steers_in_transaction(
            &transaction,
            &session_id,
            run_id,
            reason,
            dropped_steers.len(),
        )?;
        transition_unapplied_followups_in_transaction(
            &transaction,
            &session_id,
            reason,
            abandoned_followups.len(),
        )?;
        release_turn_claim_with_status(&transaction, claim_id, &session_id, status)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ProductTurnControlFinish {
            dropped_steers,
            abandoned_followups,
        })
    }

    pub(super) fn list_provider_profiles(
        &self,
    ) -> Result<Vec<ProductProviderProfile>, ProductStoreError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT profile_id, label, provider_type, api_base, api_key_env,
                       default_model, created_at, updated_at
                FROM product_provider_profiles
                ORDER BY label COLLATE NOCASE ASC, updated_at DESC, profile_id ASC
                LIMIT ?1
                "#,
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![limit_i64(MAX_PRODUCT_PROVIDER_PROFILES)?],
                raw_provider_from_row,
            )
            .map_err(storage_error)?;
        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(row.map_err(storage_error)?.into_product()?);
        }
        Ok(profiles)
    }

    pub(super) fn get_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        let connection = self.database.connect()?;
        get_provider_profile(&connection, profile_id)
    }

    pub(super) fn create_provider_profile(
        &self,
        request: CreateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        let profile = validate_provider_create(request)?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        enforce_table_limit(
            &transaction,
            "product_provider_profiles",
            MAX_PRODUCT_PROVIDER_PROFILES,
            "provider profile limit reached",
        )?;
        let profile_id = ProductProviderProfileId::new();
        let now = now_rfc3339();
        insert_provider_profile(&transaction, &profile_id, &profile, &now, &now)?;
        let created = get_provider_profile(&transaction, &profile_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(created)
    }

    pub(super) fn update_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
        request: UpdateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        let profile = validate_provider_update(request)?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_provider_profile(&transaction, profile_id)?;
        update_provider_profile_row(&transaction, profile_id, &profile, &now_rfc3339())?;
        let updated = get_provider_profile(&transaction, profile_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn delete_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_provider_profile(&transaction, profile_id)?;
        transaction
            .execute(
                r#"
                UPDATE product_preferences
                SET provider_profile_id = NULL, provider_model = NULL,
                    provider_approval = NULL, provider_max_steps = NULL,
                    updated_at = ?2, revision = revision + 1
                WHERE singleton = 1 AND provider_profile_id = ?1
                "#,
                params![profile_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                r#"
                UPDATE product_session_model_configs
                SET profile_id = NULL, revision = revision + 1, updated_at = ?2
                WHERE profile_id = ?1
                "#,
                params![profile_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                "UPDATE product_session_run_models SET profile_id = NULL WHERE profile_id = ?1",
                params![profile_id.to_string()],
            )
            .map_err(storage_error)?;
        let deleted = transaction
            .execute(
                "DELETE FROM product_provider_profiles WHERE profile_id = ?1",
                params![profile_id.to_string()],
            )
            .map_err(storage_error)?;
        if deleted != 1 {
            return Err(not_found("product provider profile was not found"));
        }
        transaction.commit().map_err(storage_error)?;
        Ok(())
    }

    pub(super) fn get_preferences(&self) -> Result<ProductPreferences, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let preferences = get_preferences(&transaction)?;
        transaction.commit().map_err(storage_error)?;
        Ok(preferences)
    }

    pub(super) fn upsert_provider_catalog_identity(
        &self,
        profile_id: &ProductProviderProfileId,
        label: &str,
        provider_type: ProductProviderType,
        catalog_revision: &str,
    ) -> Result<(), ProductStoreError> {
        let label = validate_required_text("provider profile label", label)?;
        if catalog_revision.is_empty()
            || catalog_revision.len() > 128
            || catalog_revision.chars().any(char::is_control)
        {
            return Err(invalid("provider catalog revision is invalid"));
        }
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
                INSERT INTO product_provider_profiles(
                    profile_id, label, provider_type, api_base, api_key_env,
                    default_model, created_at, updated_at
                ) VALUES (?1, ?2, ?3, '', NULL, NULL, ?4, ?4)
                ON CONFLICT(profile_id) DO UPDATE SET
                    label = excluded.label,
                    provider_type = excluded.provider_type,
                    api_base = '', api_key_env = NULL, default_model = NULL,
                    updated_at = excluded.updated_at
                "#,
                params![
                    profile_id.to_string(),
                    label,
                    provider_type_to_db(provider_type),
                    now,
                ],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                r#"
                INSERT INTO product_provider_profile_catalog_mappings(
                    source, source_profile_id, catalog_profile_id, source_digest, migrated_at
                ) VALUES ('user_catalog', ?1, ?1, ?2, ?3)
                ON CONFLICT(source, source_profile_id) DO UPDATE SET
                    catalog_profile_id = excluded.catalog_profile_id,
                    source_digest = excluded.source_digest,
                    migrated_at = excluded.migrated_at
                "#,
                params![profile_id.to_string(), catalog_revision, now],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(())
    }

    pub(super) fn get_resume_health(&self) -> Result<ProductResumeHealth, ProductStoreError> {
        let connection = self.database.connect()?;
        let counts = connection
            .query_row(
                r#"
                SELECT
                    (SELECT COUNT(*) FROM product_workspaces),
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN latest_run_id IS NOT NULL THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'needs_attention' THEN 1 ELSE 0 END), 0)
                FROM product_sessions
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(storage_error)?;
        let needs_attention_session_count = u64::try_from(counts.4).map_err(storage_error)?;
        Ok(ProductResumeHealth {
            status: if needs_attention_session_count == 0 {
                ProductResumeHealthStatus::Healthy
            } else {
                ProductResumeHealthStatus::NeedsAttention
            },
            workspace_count: u64::try_from(counts.0).map_err(storage_error)?,
            session_count: u64::try_from(counts.1).map_err(storage_error)?,
            bound_session_count: u64::try_from(counts.2).map_err(storage_error)?,
            running_session_count: u64::try_from(counts.3).map_err(storage_error)?,
            needs_attention_session_count,
        })
    }

    pub(super) fn update_preferences(
        &self,
        request: UpdateProductPreferencesRequest,
    ) -> Result<ProductPreferences, ProductStoreError> {
        let preferences = validate_preferences(request)?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        validate_preference_references(&transaction, &preferences)?;
        match preferences.expected_revision {
            Some(expected_revision) => {
                if !write_preferences_at_revision(&transaction, &preferences, expected_revision)? {
                    return Err(ProductStoreError::new(
                        ProductErrorCode::ProductRevisionConflict,
                        "product preferences changed since they were read",
                    ));
                }
            }
            None => write_preferences(&transaction, &preferences)?,
        }
        let updated = get_preferences(&transaction)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn apply_m1_browser_migration(
        &self,
        migration: PreparedM1BrowserMigration,
    ) -> Result<M1BrowserMigrationResponse, ProductStoreError> {
        apply_migration(&self.database, migration)
    }

    pub(super) fn preflight_m1_browser_migration(
        &self,
        request: &crate::product::M1BrowserMigrationRequest,
    ) -> Result<M1BrowserMigrationPreflight, ProductStoreError> {
        validate_migration_envelope(request, &[])?;
        let digest = m1_browser_migration_digest(request)
            .map_err(|_| invalid("browser migration request could not be normalized"))?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        remove_expired_migration_preparations_at(&transaction, Utc::now())?;
        if let Some(response) = replay_receipt(
            &transaction,
            request.source_schema_version,
            &request.idempotency_key,
            &digest,
        )? {
            transaction.commit().map_err(storage_error)?;
            return Ok(M1BrowserMigrationPreflight::Replay(response));
        }

        if let Some(preparation) = migration_preparation(
            &transaction,
            request.source_schema_version,
            &request.idempotency_key,
        )? {
            if preparation.request_digest != digest {
                return Err(migration_idempotency_conflict(
                    "migration idempotency key is already preparing a different payload",
                ));
            }
            validate_preparation_baseline(request, preparation.preferences_baseline)?;
            transaction.commit().map_err(storage_error)?;
            return Ok(M1BrowserMigrationPreflight::Prepare(
                preparation.preferences_baseline,
            ));
        }

        enforce_table_limit(
            &transaction,
            "product_migration_preparations",
            MAX_MIGRATION_PREPARATIONS,
            "browser migration preparation limit reached",
        )?;
        let baseline = if migration_requests_preferences(request) {
            M1PreferencesBaseline::Revision(preferences_revision(&transaction)?)
        } else {
            M1PreferencesBaseline::NotRequested
        };
        let (preferences_requested, preferences_revision) = baseline_to_db(baseline)?;
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_preparations(
                    source, source_schema_version, idempotency_key, request_digest,
                    preferences_requested, preferences_revision, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    MIGRATION_SOURCE_WEB_M1,
                    i64::from(request.source_schema_version),
                    request.idempotency_key,
                    digest,
                    preferences_requested,
                    preferences_revision,
                    now_rfc3339(),
                ],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(M1BrowserMigrationPreflight::Prepare(baseline))
    }

    pub(super) fn create_control(
        &self,
        session_id: &ProductSessionId,
        kind: ProductControlKind,
        request: CreateProductControlRequest,
    ) -> Result<(ProductControl, bool), ProductStoreError> {
        let content = request.content.trim();
        validate_control_message(content, request.idempotency_key.as_deref())?;

        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        get_session(&transaction, session_id)?;

        let kind_db = control_kind_to_db(kind);
        let now = now_rfc3339();
        let digest = rove_runtime::context::stable_hash(content);

        if let Some(key) = request.idempotency_key.as_deref()
            && let Some(existing) = transaction
                .query_row(
                    r#"
                    SELECT control_id, product_session_id, kind, idempotency_key, content,
                           status, run_id, seq, created_at, applied_at
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND idempotency_key = ?2
                    "#,
                    params![session_id.to_string(), key],
                    row_to_control,
                )
                .optional()
                .map_err(storage_error)?
        {
            if existing.kind != kind || existing.content != content {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductControlConflict,
                    "idempotency_key already exists with different kind or content",
                ));
            }
            transaction.commit().map_err(storage_error)?;
            return Ok((existing, true));
        }

        if kind == ProductControlKind::Steer {
            let pending_count: i64 = transaction
                .query_row(
                    r#"
                    SELECT COUNT(*) FROM product_session_controls
                    WHERE product_session_id = ?1 AND kind = 'steer' AND status = 'pending'
                    "#,
                    params![session_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if pending_count >= MAX_PENDING_STEERS_PER_SESSION {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductControlRejected,
                    "steer queue is full; retry after the next model turn",
                ));
            }
        }

        let control_id = ProductControlId::new();
        let seq: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM product_session_controls WHERE product_session_id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(storage_error)?;

        transaction
            .execute(
                r#"
                INSERT INTO product_session_controls(
                    control_id, product_session_id, kind, idempotency_key,
                    request_digest, content, status, seq, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8)
                "#,
                params![
                    control_id.to_string(),
                    session_id.to_string(),
                    kind_db,
                    request.idempotency_key,
                    digest,
                    content,
                    seq,
                    now,
                ],
            )
            .map_err(storage_error)?;

        let control = ProductControl {
            id: control_id,
            product_session_id: session_id.clone(),
            kind,
            idempotency_key: request.idempotency_key.clone(),
            content: content.to_string(),
            status: ProductControlStatus::Pending,
            run_id: None,
            seq,
            created_at: now,
            applied_at: None,
        };
        transaction.commit().map_err(storage_error)?;
        Ok((control, false))
    }

    pub(super) fn create_message(
        &self,
        session_id: &ProductSessionId,
        request: CreateProductMessageRequest,
    ) -> Result<(ProductMessage, bool), ProductStoreError> {
        let content = request.content.trim();
        validate_control_message(content, request.idempotency_key.as_deref())?;
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session = get_session(&transaction, session_id)?;
        let digest = rove_runtime::context::stable_hash(content);
        if let Some(key) = request.idempotency_key.as_deref()
            && let Some(existing) = transaction
                .query_row(
                    r#"
                    SELECT control_id, product_session_id, kind, idempotency_key, content,
                           status, run_id, seq, created_at, applied_at
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND idempotency_key = ?2
                    "#,
                    params![session_id.to_string(), key],
                    row_to_control,
                )
                .optional()
                .map_err(storage_error)?
        {
            if existing.content != content {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductControlConflict,
                    "idempotency_key already exists with different message content",
                ));
            }
            let contract_version: i64 = transaction
                .query_row(
                    r#"
                    SELECT message_contract_version
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND control_id = ?2
                    "#,
                    params![session_id.to_string(), existing.id.to_string()],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if contract_version != 1 {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductControlConflict,
                    "idempotency_key belongs to a legacy control, not a product message",
                ));
            }
            let message = get_message_in_transaction(&transaction, session_id, &existing.id)?;
            transaction.commit().map_err(storage_error)?;
            return Ok((message, true));
        }
        let pending_count: i64 = transaction
            .query_row(
                r#"
                SELECT COUNT(*) FROM product_session_controls
                WHERE product_session_id = ?1
                  AND message_contract_version = 1
                  AND status IN ('pending', 'accepted')
                "#,
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        if pending_count >= MAX_PENDING_STEERS_PER_SESSION {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message queue is full",
            ));
        }
        let control_id = ProductControlId::new();
        let seq: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM product_session_controls WHERE product_session_id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        let now = now_rfc3339();
        let (status, reason) = match session.status {
            ProductSessionStatus::Idle | ProductSessionStatus::Running => ("pending", None),
            ProductSessionStatus::Error | ProductSessionStatus::NeedsAttention => (
                "abandoned",
                Some("session requires an explicit recovery decision"),
            ),
            ProductSessionStatus::Archived => {
                return Err(invalid("archived product sessions cannot accept messages"));
            }
        };
        transaction
            .execute(
                r#"
                INSERT INTO product_session_controls(
                    control_id, product_session_id, kind, idempotency_key,
                    request_digest, content, status, seq, abandoned_reason,
                    created_at, message_contract_version, requested_delivery
                ) VALUES (?1, ?2, 'followup', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'successor')
                "#,
                params![
                    control_id.to_string(),
                    session_id.to_string(),
                    request.idempotency_key,
                    digest,
                    content,
                    status,
                    seq,
                    reason,
                    now,
                ],
            )
            .map_err(storage_error)?;
        let message = get_message_in_transaction(&transaction, session_id, &control_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok((message, false))
    }

    pub(super) fn promote_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let existing_message = get_message_in_transaction(&transaction, session_id, message_id)?;
        if existing_message.requested_delivery == ProductMessageDelivery::CurrentRun {
            transaction.commit().map_err(storage_error)?;
            return Ok(existing_message);
        }
        let session = get_session(&transaction, session_id)?;
        if session.status != ProductSessionStatus::Running
            || !has_active_claim_for_session(&transaction, session_id)?
        {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message can only be promoted while its session turn is active",
            ));
        }
        let existing = get_control_in_transaction(&transaction, session_id, message_id)?;
        if existing.kind != ProductControlKind::Followup
            || existing.status != ProductControlStatus::Pending
        {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message is no longer eligible for promotion",
            ));
        }
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET kind = 'steer', message_contract_version = 1,
                    requested_delivery = 'current_run'
                WHERE product_session_id = ?1 AND control_id = ?2 AND status = 'pending'
                "#,
                params![session_id.to_string(), message_id.to_string()],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message promotion lost its compare-and-set race",
            ));
        }
        let updated = get_message_in_transaction(&transaction, session_id, message_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn revoke_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let existing = get_control_in_transaction(&transaction, session_id, message_id)?;
        if existing.status == ProductControlStatus::Revoked {
            let message = get_message_in_transaction(&transaction, session_id, message_id)?;
            transaction.commit().map_err(storage_error)?;
            return Ok(message);
        }
        if matches!(
            existing.status,
            ProductControlStatus::Accepted
                | ProductControlStatus::Applied
                | ProductControlStatus::Dropped
        ) {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message already has a terminal or claimed delivery outcome",
            ));
        }
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'revoked'
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND status IN ('pending', 'abandoned')
                "#,
                params![session_id.to_string(), message_id.to_string()],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "message revoke lost its compare-and-set race",
            ));
        }
        let updated = get_message_in_transaction(&transaction, session_id, message_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn list_messages(
        &self,
        session_id: &ProductSessionId,
        query: ProductMessagePageQuery,
    ) -> Result<ProductMessagePage, ProductStoreError> {
        if query.limit == 0
            || query.limit > MAX_PRODUCT_MESSAGE_PAGE_LIMIT
            || query.after_seq.is_some_and(|sequence| sequence < 0)
            || query.before_seq.is_some_and(|sequence| sequence <= 0)
            || (query.after_seq.is_some() && query.before_seq.is_some())
        {
            return Err(invalid("message page query is invalid"));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        get_session(&transaction, session_id)?;
        let fetch_limit = i64::try_from(query.limit + 1).map_err(storage_error)?;
        let (mut messages, reverse) = if let Some(after_seq) = query.after_seq {
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT control_id, product_session_id, content, status, run_id, seq,
                           created_at, applied_at, abandoned_reason, requested_delivery
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND message_contract_version = 1
                      AND seq > ?2
                    ORDER BY seq ASC
                    LIMIT ?3
                    "#,
                )
                .map_err(storage_error)?;
            let messages = statement
                .query_map(
                    params![session_id.to_string(), after_seq, fetch_limit],
                    row_to_message,
                )
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?;
            (messages, false)
        } else {
            let before_seq = query.before_seq.unwrap_or(i64::MAX);
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT control_id, product_session_id, content, status, run_id, seq,
                           created_at, applied_at, abandoned_reason, requested_delivery
                    FROM product_session_controls
                    WHERE product_session_id = ?1 AND message_contract_version = 1
                      AND seq < ?2
                    ORDER BY seq DESC
                    LIMIT ?3
                    "#,
                )
                .map_err(storage_error)?;
            let messages = statement
                .query_map(
                    params![session_id.to_string(), before_seq, fetch_limit],
                    row_to_message,
                )
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?;
            (messages, true)
        };
        let has_more = messages.len() > query.limit;
        if has_more {
            messages.pop();
        }
        if reverse {
            messages.reverse();
        }
        let page = ProductMessagePage {
            next_after_seq: if !reverse && has_more {
                messages.last().map(|message| message.seq)
            } else {
                None
            },
            next_before_seq: if reverse && has_more {
                messages.first().map(|message| message.seq)
            } else {
                None
            },
            messages,
        };
        transaction.commit().map_err(storage_error)?;
        Ok(page)
    }

    pub(super) fn get_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let message = get_message_in_transaction(&transaction, session_id, message_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(message)
    }

    pub(super) fn list_controls(
        &self,
        session_id: &ProductSessionId,
        filter: Option<ProductControlStatus>,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        get_session(&transaction, session_id)?;
        let (sql, status_db) = match filter {
            Some(s) => (
                "SELECT control_id, product_session_id, kind, idempotency_key, content, status, run_id, seq, created_at, applied_at FROM product_session_controls WHERE product_session_id = ?1 AND status = ?2 ORDER BY seq ASC",
                Some(control_status_to_db(s)),
            ),
            None => (
                "SELECT control_id, product_session_id, kind, idempotency_key, content, status, run_id, seq, created_at, applied_at FROM product_session_controls WHERE product_session_id = ?1 ORDER BY seq ASC",
                None,
            ),
        };
        let mut statement = transaction.prepare(sql).map_err(storage_error)?;
        let rows: Vec<ProductControl> = match status_db {
            Some(s) => statement
                .query_map(params![session_id.to_string(), s], row_to_control)
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?,
            None => statement
                .query_map(params![session_id.to_string()], row_to_control)
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?,
        };
        drop(statement);
        transaction.commit().map_err(storage_error)?;
        Ok(rows)
    }

    pub(super) fn get_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let control = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND control_id = ?2
                "#,
                params![session_id.to_string(), control_id.to_string()],
                row_to_control,
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| not_found("control not found"))?;
        transaction.commit().map_err(storage_error)?;
        Ok(control)
    }

    pub(super) fn transition_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
        from: ProductControlStatus,
        to: ProductControlStatus,
        applied_run_id: Option<&RunId>,
    ) -> Result<ProductControl, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let existing = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND control_id = ?2
                "#,
                params![session_id.to_string(), control_id.to_string()],
                row_to_control,
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| not_found("control not found"))?;
        if existing.status != from {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                format!(
                    "control status is {} (expected {})",
                    control_status_to_db(existing.status),
                    control_status_to_db(from)
                ),
            ));
        }
        let now = now_rfc3339();
        let applied_at = (to == ProductControlStatus::Applied).then(|| now.clone());
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = ?3,
                    run_id = COALESCE(?4, run_id),
                    applied_at = COALESCE(?5, applied_at)
                WHERE product_session_id = ?1 AND control_id = ?2 AND status = ?6
                "#,
                params![
                    session_id.to_string(),
                    control_id.to_string(),
                    control_status_to_db(to),
                    applied_run_id.map(|id| id.to_string()),
                    applied_at,
                    control_status_to_db(from),
                ],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "control status transition failed",
            ));
        }
        let updated = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND control_id = ?2
                "#,
                params![session_id.to_string(), control_id.to_string()],
                row_to_control,
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn confirm_abandoned_followup(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session = get_session(&transaction, session_id)?;
        validate_binding_integrity(&transaction, &session)?;
        if has_active_claim_for_session(&transaction, session_id)? {
            return Err(session_active(
                "an abandoned follow-up cannot be confirmed while a turn is active",
            ));
        }
        let control = get_control_in_transaction(&transaction, session_id, control_id)?;
        if control.kind != ProductControlKind::Followup {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "only follow-up controls can be confirmed",
            ));
        }
        if matches!(
            control.status,
            ProductControlStatus::Pending
                | ProductControlStatus::Accepted
                | ProductControlStatus::Applied
        ) {
            transaction.commit().map_err(storage_error)?;
            return Ok(control);
        }
        if control.status != ProductControlStatus::Abandoned {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "only an abandoned follow-up can be confirmed",
            ));
        }
        if session.status == ProductSessionStatus::Archived {
            return Err(invalid(
                "archived product sessions cannot confirm a follow-up",
            ));
        }
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'pending', run_id = NULL, applied_at = NULL,
                    abandoned_reason = NULL
                WHERE product_session_id = ?1 AND control_id = ?2 AND status = 'abandoned'
                "#,
                params![session_id.to_string(), control_id.to_string()],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "abandoned follow-up confirmation was not acquired",
            ));
        }
        let updated_session = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = 'idle', updated_at = ?2
                WHERE product_session_id = ?1
                  AND status IN ('needs_attention', 'error', 'idle')
                "#,
                params![session_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        if updated_session != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "product session cannot confirm a follow-up from its current state",
            ));
        }
        let updated = get_control_in_transaction(&transaction, session_id, control_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(updated)
    }

    pub(super) fn abandon_pending_controls(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<u64, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'abandoned', abandoned_reason = ?2
                WHERE product_session_id = ?1 AND status = 'pending'
                "#,
                params![session_id.to_string(), reason],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        u64::try_from(changed).map_err(storage_error)
    }

    pub(super) fn list_pending_followups(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut statement = transaction
            .prepare(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND kind = 'followup' AND status = 'pending'
                ORDER BY seq ASC
                "#,
            )
            .map_err(storage_error)?;
        let rows: Vec<ProductControl> = statement
            .query_map(params![session_id.to_string()], row_to_control)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        drop(statement);
        transaction.commit().map_err(storage_error)?;
        Ok(rows)
    }

    /// Atomically pick the lowest-seq pending follow-up and mark it accepted so
    /// a crash between claim and run-start cannot double-start the same control.
    pub(super) fn claim_next_pending_followup(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductControl>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let Some(pending) = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND kind = 'followup' AND status = 'pending'
                ORDER BY seq ASC
                LIMIT 1
                "#,
                params![session_id.to_string()],
                row_to_control,
            )
            .optional()
            .map_err(storage_error)?
        else {
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        };

        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'accepted', applied_at = NULL
                WHERE product_session_id = ?1 AND control_id = ?2 AND status = 'pending'
                "#,
                params![session_id.to_string(), pending.id.to_string()],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        }
        let claimed = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND control_id = ?2
                "#,
                params![session_id.to_string(), pending.id.to_string()],
                row_to_control,
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(Some(claimed))
    }

    pub(super) fn claim_next_followup_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let mut session = get_session(&transaction, session_id)?;
        if session.status != ProductSessionStatus::Idle
            || has_active_claim_for_session(&transaction, session_id)?
        {
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        }
        validate_binding_integrity(&transaction, &session)?;
        let workspace = get_workspace(&transaction, &session.workspace_id)?;
        let fork = get_fork_context(&transaction, &session)?;
        ensure_session_model_config(&transaction, session_id)?;
        let model_config = get_session_model_config_in_transaction(&transaction, session_id)?;
        let previous_binding = session.runtime_binding.clone();
        let Some(pending) = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND kind = 'followup' AND status = 'pending'
                ORDER BY seq ASC
                LIMIT 1
                "#,
                params![session_id.to_string()],
                row_to_control,
            )
            .optional()
            .map_err(storage_error)?
        else {
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        };

        let claim_id = ProductTurnClaimId::new();
        let accepted = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'accepted', applied_at = NULL
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND kind = 'followup' AND status = 'pending' AND run_id IS NULL
                "#,
                params![session_id.to_string(), pending.id.to_string()],
            )
            .map_err(storage_error)?;
        if accepted != 1 {
            transaction.commit().map_err(storage_error)?;
            return Ok(None);
        }
        transaction
            .execute(
                r#"
                INSERT INTO product_turn_claims(
                    claim_id, product_session_id, claimed_at, followup_control_id
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    claim_id.to_string(),
                    session_id.to_string(),
                    now_rfc3339(),
                    pending.id.to_string(),
                ],
            )
            .map_err(storage_error)?;
        let session_updated = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = 'running', updated_at = ?2
                WHERE product_session_id = ?1 AND status = 'idle'
                "#,
                params![session_id.to_string(), now_rfc3339()],
            )
            .map_err(storage_error)?;
        if session_updated != 1 {
            return Err(session_active(
                "product session follow-up turn claim was not acquired",
            ));
        }
        session.status = ProductSessionStatus::Running;
        session.updated_at = now_rfc3339();
        let control = transaction
            .query_row(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND control_id = ?2
                "#,
                params![session_id.to_string(), pending.id.to_string()],
                row_to_control,
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(Some(ProductFollowupTurnClaim {
            control,
            turn: ProductTurnClaim {
                claim_id,
                context: ProductSessionContext {
                    workspace,
                    session,
                    fork,
                },
                previous_status: ProductSessionStatus::Idle,
                previous_binding,
                model_config,
            },
        }))
    }

    pub(super) fn requeue_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = followup_claim_session_id(&transaction, claim_id, control_id)?;
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'pending', run_id = NULL, applied_at = NULL,
                    abandoned_reason = NULL
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND status = 'accepted' AND run_id IS NULL
                "#,
                params![session_id.to_string(), control_id.to_string()],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "queued follow-up cannot be requeued after runtime preparation",
            ));
        }
        release_followup_claim_with_status(
            &transaction,
            claim_id,
            &session_id,
            ProductSessionStatus::Idle,
        )?;
        transaction.commit().map_err(storage_error)
    }

    pub(super) fn reserve_followup_run(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        run_id: RunId,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = followup_claim_session_id(&transaction, claim_id, control_id)?;
        let changed = transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET run_id = ?3
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND status = 'accepted' AND run_id IS NULL
                "#,
                params![
                    session_id.to_string(),
                    control_id.to_string(),
                    run_id.to_string(),
                ],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "queued follow-up run reservation was not acquired",
            ));
        }
        transaction.commit().map_err(storage_error)
    }

    pub(super) fn abandon_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        reason: &str,
    ) -> Result<(), ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let session_id = followup_claim_session_id(&transaction, claim_id, control_id)?;
        transaction
            .execute(
                r#"
                UPDATE product_session_controls
                SET status = 'abandoned', abandoned_reason = ?3
                WHERE product_session_id = ?1 AND control_id = ?2
                  AND status IN ('pending', 'accepted', 'applied')
                "#,
                params![session_id.to_string(), control_id.to_string(), reason],
            )
            .map_err(storage_error)?;
        release_followup_claim_with_status(
            &transaction,
            claim_id,
            &session_id,
            ProductSessionStatus::NeedsAttention,
        )?;
        transaction.commit().map_err(storage_error)
    }

    pub(super) fn list_idle_sessions_with_pending_followups(
        &self,
    ) -> Result<Vec<ProductSessionId>, ProductStoreError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT session.product_session_id
                FROM product_sessions AS session
                WHERE session.status = 'idle'
                  AND NOT EXISTS (
                      SELECT 1 FROM product_turn_claims AS claim
                      WHERE claim.product_session_id = session.product_session_id
                  )
                  AND EXISTS (
                      SELECT 1 FROM product_session_controls AS control
                      WHERE control.product_session_id = session.product_session_id
                        AND control.kind = 'followup' AND control.status = 'pending'
                  )
                ORDER BY session.updated_at ASC, session.product_session_id ASC
                "#,
            )
            .map_err(storage_error)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?
            .map(|row| {
                row.map_err(storage_error)
                    .and_then(|value| parse_product_id(&value, "product session id"))
            })
            .collect()
    }

    pub(super) fn drop_pending_steers(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        transition_pending_controls(
            &self.database,
            session_id,
            ProductControlKind::Steer,
            ProductControlStatus::Dropped,
            reason,
        )
    }

    pub(super) fn abandon_pending_followups(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        transition_pending_controls(
            &self.database,
            session_id,
            ProductControlKind::Followup,
            ProductControlStatus::Abandoned,
            reason,
        )
    }
}

fn followup_claim_session_id(
    transaction: &Transaction<'_>,
    claim_id: &ProductTurnClaimId,
    control_id: &ProductControlId,
) -> Result<ProductSessionId, ProductStoreError> {
    transaction
        .query_row(
            r#"
            SELECT product_session_id
            FROM product_turn_claims
            WHERE claim_id = ?1 AND followup_control_id = ?2
            "#,
            params![claim_id.to_string(), control_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|value| parse_product_id(&value, "product session id"))
        .transpose()?
        .ok_or_else(|| {
            ProductStoreError::new(
                ProductErrorCode::ProductControlRejected,
                "follow-up turn claim is missing or no longer active",
            )
        })
}

fn active_turn_session_id(
    transaction: &Transaction<'_>,
    claim_id: &ProductTurnClaimId,
) -> Result<ProductSessionId, ProductStoreError> {
    transaction
        .query_row(
            "SELECT product_session_id FROM product_turn_claims WHERE claim_id = ?1",
            params![claim_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|value| parse_product_id(&value, "product session id"))
        .transpose()?
        .ok_or_else(|| resume_conflict("product session turn claim is missing or no longer active"))
}

fn pending_followup_for_session(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
) -> Result<Option<ProductControl>, ProductStoreError> {
    transaction
        .query_row(
            r#"
            SELECT control_id, product_session_id, kind, idempotency_key, content,
                   status, run_id, seq, created_at, applied_at
            FROM product_session_controls
            WHERE product_session_id = ?1 AND kind = 'followup' AND status = 'pending'
            ORDER BY seq ASC
            LIMIT 1
            "#,
            params![session_id.to_string()],
            row_to_control,
        )
        .optional()
        .map_err(storage_error)
}

fn get_control_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
    control_id: &ProductControlId,
) -> Result<ProductControl, ProductStoreError> {
    transaction
        .query_row(
            r#"
            SELECT control_id, product_session_id, kind, idempotency_key, content,
                   status, run_id, seq, created_at, applied_at
            FROM product_session_controls
            WHERE product_session_id = ?1 AND control_id = ?2
            "#,
            params![session_id.to_string(), control_id.to_string()],
            row_to_control,
        )
        .map_err(storage_error)
}

/// Follow-ups become historical only after their successor emits a durable
/// run-start fact. A queued or claimed-but-not-started follow-up is still
/// recoverable work and must be classified if its owning turn ends.
fn unapplied_followups_for_session(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
) -> Result<Vec<ProductControl>, ProductStoreError> {
    let mut statement = transaction
        .prepare(
            r#"
            SELECT control_id, product_session_id, kind, idempotency_key, content,
                   status, run_id, seq, created_at, applied_at
            FROM product_session_controls
            WHERE product_session_id = ?1 AND kind = 'followup'
              AND status IN ('pending', 'accepted')
            ORDER BY seq ASC
            "#,
        )
        .map_err(storage_error)?;
    statement
        .query_map(params![session_id.to_string()], row_to_control)
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

/// A steer is only terminal once it was applied to a model turn, dropped, or
/// revoked. `accepted` means it crossed a safe point but may still be lost to
/// cancellation or a budget/error boundary; it must therefore be closed with
/// the same conservative rule as `pending`.
fn unapplied_steers_for_session(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
) -> Result<Vec<ProductControl>, ProductStoreError> {
    let mut statement = transaction
        .prepare(
            r#"
            SELECT control_id, product_session_id, kind, idempotency_key, content,
                   status, run_id, seq, created_at, applied_at
            FROM product_session_controls
            WHERE product_session_id = ?1 AND kind = 'steer'
              AND status IN ('pending', 'accepted')
            ORDER BY seq ASC
            "#,
        )
        .map_err(storage_error)?;
    statement
        .query_map(params![session_id.to_string()], row_to_control)
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

fn transition_unapplied_steers_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
    run_id: Option<RunId>,
    reason: &str,
    expected_count: usize,
) -> Result<(), ProductStoreError> {
    if expected_count == 0 {
        return Ok(());
    }
    let changed = transaction
        .execute(
            r#"
            UPDATE product_session_controls
            SET status = 'dropped',
                run_id = COALESCE(run_id, ?2),
                abandoned_reason = ?3
            WHERE product_session_id = ?1 AND kind = 'steer'
              AND status IN ('pending', 'accepted')
            "#,
            params![
                session_id.to_string(),
                run_id.map(|value| value.to_string()),
                reason
            ],
        )
        .map_err(storage_error)?;
    if changed != expected_count {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductControlRejected,
            "unapplied steers changed while closing a run",
        ));
    }
    Ok(())
}

fn transition_unapplied_followups_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
    reason: &str,
    expected_count: usize,
) -> Result<(), ProductStoreError> {
    if expected_count == 0 {
        return Ok(());
    }
    let changed = transaction
        .execute(
            r#"
            UPDATE product_session_controls
            SET status = 'abandoned', abandoned_reason = ?2
            WHERE product_session_id = ?1 AND kind = 'followup'
              AND status IN ('pending', 'accepted')
            "#,
            params![session_id.to_string(), reason],
        )
        .map_err(storage_error)?;
    if changed != expected_count {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductControlRejected,
            "unapplied follow-ups changed while closing a run",
        ));
    }
    Ok(())
}

fn release_turn_claim_with_status(
    transaction: &Transaction<'_>,
    claim_id: &ProductTurnClaimId,
    session_id: &ProductSessionId,
    status: ProductSessionStatus,
) -> Result<(), ProductStoreError> {
    let deleted = transaction
        .execute(
            "DELETE FROM product_turn_claims WHERE claim_id = ?1 AND product_session_id = ?2",
            params![claim_id.to_string(), session_id.to_string()],
        )
        .map_err(storage_error)?;
    if deleted != 1 {
        return Err(resume_conflict(
            "product session turn claim is missing or no longer active",
        ));
    }
    let updated = transaction
        .execute(
            r#"
            UPDATE product_sessions
            SET status = ?2, updated_at = ?3
            WHERE product_session_id = ?1
            "#,
            params![
                session_id.to_string(),
                session_status_to_db(status),
                now_rfc3339(),
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(binding_corrupt("turn claim references a missing session"));
    }
    Ok(())
}

fn release_followup_claim_with_status(
    transaction: &Transaction<'_>,
    claim_id: &ProductTurnClaimId,
    session_id: &ProductSessionId,
    status: ProductSessionStatus,
) -> Result<(), ProductStoreError> {
    let deleted = transaction
        .execute(
            r#"
            DELETE FROM product_turn_claims
            WHERE claim_id = ?1 AND product_session_id = ?2
            "#,
            params![claim_id.to_string(), session_id.to_string()],
        )
        .map_err(storage_error)?;
    if deleted != 1 {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductControlRejected,
            "follow-up turn claim is missing or no longer active",
        ));
    }
    let updated = transaction
        .execute(
            r#"
            UPDATE product_sessions
            SET status = ?2, updated_at = ?3
            WHERE product_session_id = ?1
            "#,
            params![
                session_id.to_string(),
                session_status_to_db(status),
                now_rfc3339(),
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(binding_corrupt(
            "follow-up turn claim references a missing session",
        ));
    }
    Ok(())
}

fn transition_pending_controls(
    database: &ProductDatabase,
    session_id: &ProductSessionId,
    kind: ProductControlKind,
    target: ProductControlStatus,
    reason: &str,
) -> Result<Vec<ProductControl>, ProductStoreError> {
    let mut connection = database.connect()?;
    let transaction = immediate_transaction(&mut connection)?;
    let controls: Vec<ProductControl> = {
        let mut statement = transaction
            .prepare(
                r#"
                SELECT control_id, product_session_id, kind, idempotency_key, content,
                       status, run_id, seq, created_at, applied_at
                FROM product_session_controls
                WHERE product_session_id = ?1 AND kind = ?2 AND status = 'pending'
                ORDER BY seq ASC
                "#,
            )
            .map_err(storage_error)?;
        statement
            .query_map(
                params![session_id.to_string(), control_kind_to_db(kind)],
                row_to_control,
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?
    };
    if controls.is_empty() {
        transaction.commit().map_err(storage_error)?;
        return Ok(controls);
    }
    let changed = transaction
        .execute(
            r#"
            UPDATE product_session_controls
            SET status = ?3, abandoned_reason = ?4
            WHERE product_session_id = ?1 AND kind = ?2 AND status = 'pending'
            "#,
            params![
                session_id.to_string(),
                control_kind_to_db(kind),
                control_status_to_db(target),
                reason,
            ],
        )
        .map_err(storage_error)?;
    if changed != controls.len() {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductControlRejected,
            "pending controls changed while closing a run",
        ));
    }
    transaction.commit().map_err(storage_error)?;
    Ok(controls)
}

pub(super) fn remove_expired_migration_preparations_at(
    connection: &Connection,
    now: chrono::DateTime<Utc>,
) -> Result<u64, ProductStoreError> {
    let cutoff = (now - chrono::Duration::seconds(MIGRATION_PREPARATION_TTL_SECS))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let removed = connection
        .execute(
            r#"
            DELETE FROM product_migration_preparations
            WHERE rowid IN (
                SELECT rowid
                FROM product_migration_preparations
                WHERE julianday(created_at) IS NULL
                   OR julianday(created_at) <= julianday(?1)
                ORDER BY created_at ASC, rowid ASC
                LIMIT ?2
            )
            "#,
            params![cutoff, limit_i64(MAX_MIGRATION_PREPARATIONS)?],
        )
        .map_err(storage_error)?;
    u64::try_from(removed).map_err(storage_error)
}

#[derive(Debug)]
struct RawWorkspace {
    id: String,
    canonical_root: String,
    kind: String,
    display_name: String,
    pinned: i64,
    last_opened_at: String,
    created_at: String,
    updated_at: String,
}

impl RawWorkspace {
    fn into_product(self) -> Result<ProductWorkspace, ProductStoreError> {
        Ok(ProductWorkspace {
            id: parse_product_id(&self.id, "workspace id")?,
            canonical_root: PathBuf::from(self.canonical_root),
            kind: workspace_kind_from_db(&self.kind)?,
            display_name: self.display_name,
            pinned: bool_from_i64(self.pinned)?,
            last_opened_at: self.last_opened_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug)]
struct RawSession {
    id: String,
    workspace_id: String,
    title: String,
    status: String,
    latest_ordinal: Option<i64>,
    runtime_session_id: Option<String>,
    latest_job_id: Option<String>,
    latest_run_id: Option<String>,
    parent_session_id: Option<String>,
    fork_point_run_id: Option<String>,
    fork_point_seq: Option<i64>,
    created_at: String,
    updated_at: String,
}

impl RawSession {
    fn into_product(self) -> Result<ProductSession, ProductStoreError> {
        let runtime_binding = match (
            self.latest_ordinal,
            self.runtime_session_id,
            self.latest_job_id,
            self.latest_run_id,
        ) {
            (None, None, None, None) => None,
            (Some(ordinal), Some(session_id), Some(job_id), Some(run_id)) if ordinal >= 1 => {
                Some(ProductRuntimeBinding {
                    ordinal: u64::try_from(ordinal)
                        .map_err(|_| binding_corrupt("latest binding ordinal is invalid"))?,
                    runtime_session_id: parse_runtime_id(&session_id, "runtime session id")?,
                    latest_job_id: parse_runtime_id(&job_id, "runtime job id")?,
                    latest_run_id: parse_runtime_id(&run_id, "runtime run id")?,
                })
            }
            _ => {
                return Err(binding_corrupt(
                    "latest product runtime binding is incomplete",
                ));
            }
        };
        let (parent_session_id, fork_point_run_id, fork_point_seq) = match (
            self.parent_session_id,
            self.fork_point_run_id,
            self.fork_point_seq,
        ) {
            (None, None, None) => (None, None, None),
            (Some(parent), Some(run_id), Some(seq)) if seq >= 1 => (
                Some(parse_product_id(&parent, "parent product session id")?),
                Some(parse_runtime_id(&run_id, "fork point runtime run id")?),
                Some(
                    u64::try_from(seq)
                        .map_err(|_| binding_corrupt("fork point event seq is invalid"))?,
                ),
            ),
            _ => {
                return Err(binding_corrupt(
                    "product session fork provenance is incomplete",
                ));
            }
        };
        Ok(ProductSession {
            id: parse_product_id(&self.id, "product session id")?,
            workspace_id: parse_product_id(&self.workspace_id, "workspace id")?,
            title: self.title,
            status: session_status_from_db(&self.status)?,
            runtime_binding,
            parent_session_id,
            fork_point_run_id,
            fork_point_seq,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug)]
struct RawBinding {
    product_session_id: String,
    ordinal: i64,
    runtime_session_id: String,
    runtime_job_id: String,
    runtime_run_id: String,
    resumed_from_run_id: Option<String>,
    bound_at: String,
}

#[derive(Debug)]
struct RawFork {
    id: String,
    parent_session_id: String,
    child_session_id: String,
    parent_workspace_id: String,
    parent_title: String,
    source_runtime_session_id: String,
    source_runtime_job_id: String,
    source_runtime_run_id: String,
    fork_at_event_seq: i64,
    idempotency_key: String,
    request_digest: String,
    created_at: String,
}

#[derive(Debug)]
struct StoredFork {
    fork: ProductFork,
    request_digest: String,
}

impl RawFork {
    fn into_stored(self) -> Result<StoredFork, ProductStoreError> {
        if self.fork_at_event_seq < 1 {
            return Err(binding_corrupt("fork terminal event sequence is invalid"));
        }
        let request_digest = self.request_digest;
        Ok(StoredFork {
            fork: ProductFork {
                id: parse_product_id(&self.id, "fork id")?,
                parent_product_session_id: parse_product_id(
                    &self.parent_session_id,
                    "fork parent product session id",
                )?,
                child_product_session_id: parse_product_id(
                    &self.child_session_id,
                    "fork child product session id",
                )?,
                parent_workspace_id: parse_product_id(
                    &self.parent_workspace_id,
                    "fork parent workspace id",
                )?,
                parent_title: self.parent_title,
                source_runtime_session_id: parse_runtime_id(
                    &self.source_runtime_session_id,
                    "fork source runtime session id",
                )?,
                source_runtime_job_id: parse_runtime_id(
                    &self.source_runtime_job_id,
                    "fork source runtime job id",
                )?,
                source_runtime_run_id: parse_runtime_id(
                    &self.source_runtime_run_id,
                    "fork source runtime run id",
                )?,
                fork_at_event_seq: u64::try_from(self.fork_at_event_seq)
                    .map_err(|_| binding_corrupt("fork terminal event sequence is invalid"))?,
                idempotency_key: self.idempotency_key,
                created_at: self.created_at,
            },
            request_digest,
        })
    }
}

#[derive(Debug)]
struct RawForkInheritedRun {
    ordinal: i64,
    source_product_session_id: String,
    runtime_session_id: String,
    runtime_job_id: String,
    runtime_run_id: String,
    through_event_seq: Option<i64>,
}

impl RawForkInheritedRun {
    fn into_product(self) -> Result<ProductForkInheritedRun, ProductStoreError> {
        if self.ordinal < 1 {
            return Err(binding_corrupt("fork inherited run ordinal is invalid"));
        }
        let through_event_seq = match self.through_event_seq {
            None => None,
            Some(seq) if seq >= 1 => Some(
                u64::try_from(seq)
                    .map_err(|_| binding_corrupt("fork inherited event sequence is invalid"))?,
            ),
            Some(_) => {
                return Err(binding_corrupt("fork inherited event sequence is invalid"));
            }
        };
        Ok(ProductForkInheritedRun {
            ordinal: u64::try_from(self.ordinal)
                .map_err(|_| binding_corrupt("fork inherited run ordinal is invalid"))?,
            source_product_session_id: parse_product_id(
                &self.source_product_session_id,
                "fork inherited source product session id",
            )?,
            runtime_session_id: parse_runtime_id(
                &self.runtime_session_id,
                "fork inherited runtime session id",
            )?,
            runtime_job_id: parse_runtime_id(
                &self.runtime_job_id,
                "fork inherited runtime job id",
            )?,
            runtime_run_id: parse_runtime_id(
                &self.runtime_run_id,
                "fork inherited runtime run id",
            )?,
            through_event_seq,
        })
    }
}

impl RawBinding {
    fn into_product(self) -> Result<ProductSessionRunBinding, ProductStoreError> {
        if self.ordinal < 1 {
            return Err(binding_corrupt("product run binding ordinal is invalid"));
        }
        Ok(ProductSessionRunBinding {
            product_session_id: parse_product_id(&self.product_session_id, "product session id")?,
            ordinal: u64::try_from(self.ordinal)
                .map_err(|_| binding_corrupt("product run binding ordinal is invalid"))?,
            runtime_session_id: parse_runtime_id(&self.runtime_session_id, "runtime session id")?,
            runtime_job_id: parse_runtime_id(&self.runtime_job_id, "runtime job id")?,
            runtime_run_id: parse_runtime_id(&self.runtime_run_id, "runtime run id")?,
            resumed_from_run_id: self
                .resumed_from_run_id
                .map(|value| parse_runtime_id(&value, "resumed run id"))
                .transpose()?,
            bound_at: self.bound_at,
        })
    }
}

#[derive(Debug)]
struct RawProviderProfile {
    id: String,
    label: String,
    provider_type: String,
    api_base: String,
    api_key_env: Option<String>,
    default_model: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug)]
struct RawSessionModelConfig {
    product_session_id: String,
    profile_id: Option<String>,
    model: String,
    reasoning: String,
    max_steps: i64,
    revision: i64,
    updated_at: String,
}

impl RawSessionModelConfig {
    fn into_product(self) -> Result<ProductSessionModelConfig, ProductStoreError> {
        if self.max_steps < 1 || self.max_steps > i64::from(MAX_PRODUCT_MAX_STEPS) {
            return Err(binding_corrupt("persisted session max_steps is invalid"));
        }
        if self.revision < 1 {
            return Err(binding_corrupt(
                "persisted session model revision is invalid",
            ));
        }
        Ok(ProductSessionModelConfig {
            product_session_id: parse_product_id(&self.product_session_id, "product session id")?,
            profile_id: self
                .profile_id
                .as_deref()
                .map(|value| parse_product_id(value, "provider profile id"))
                .transpose()?,
            model: self.model,
            reasoning: ProductReasoningPreference::from_str(&self.reasoning)?,
            max_steps: u32::try_from(self.max_steps)
                .map_err(|_| binding_corrupt("persisted session max_steps is invalid"))?,
            revision: u64::try_from(self.revision)
                .map_err(|_| binding_corrupt("persisted session model revision is invalid"))?,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug)]
struct RawSessionRunModel {
    product_session_id: String,
    ordinal: i64,
    runtime_run_id: String,
    profile_id: Option<String>,
    model: String,
    reasoning: String,
    max_steps: i64,
    provider_type: Option<String>,
    wire_protocol: Option<String>,
    endpoint: Option<String>,
    catalog_revision: Option<String>,
    safe_config_digest: Option<String>,
    context_window: Option<i64>,
    pricing_source: Option<String>,
    pricing_version: Option<String>,
    pricing_currency: Option<String>,
    pricing_availability: Option<String>,
    per_mtok_prompt: Option<f64>,
    per_mtok_completion: Option<f64>,
    per_mtok_cache_read: Option<f64>,
}

impl RawSessionRunModel {
    fn into_product(self) -> Result<ProductSessionRunModelView, ProductStoreError> {
        if self.ordinal < 1
            || self.max_steps < 1
            || self.max_steps > i64::from(MAX_PRODUCT_MAX_STEPS)
        {
            return Err(binding_corrupt("persisted run model snapshot is invalid"));
        }
        let pricing_availability = self
            .pricing_availability
            .as_deref()
            .map(|value| {
                ProductPricingAvailability::parse(value)
                    .ok_or_else(|| binding_corrupt("persisted run pricing availability is invalid"))
            })
            .transpose()?;
        Ok(ProductSessionRunModelView {
            product_session_id: parse_product_id(&self.product_session_id, "product session id")?,
            ordinal: u64::try_from(self.ordinal)
                .map_err(|_| binding_corrupt("run model ordinal is invalid"))?,
            runtime_run_id: parse_runtime_id(&self.runtime_run_id, "runtime run id")?,
            profile_id: self
                .profile_id
                .as_deref()
                .map(|value| parse_product_id(value, "provider profile id"))
                .transpose()?,
            model: self.model,
            reasoning: ProductReasoningPreference::from_str(&self.reasoning)?,
            max_steps: u32::try_from(self.max_steps)
                .map_err(|_| binding_corrupt("run model max_steps is invalid"))?,
            provider_type: self.provider_type,
            wire_protocol: self.wire_protocol,
            endpoint: self.endpoint,
            catalog_revision: self.catalog_revision,
            safe_config_digest: self.safe_config_digest,
            context_window: self
                .context_window
                .map(|value| {
                    u64::try_from(value)
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| binding_corrupt("persisted context window is invalid"))
                })
                .transpose()?,
            pricing_source: self.pricing_source,
            pricing_version: self.pricing_version,
            pricing_currency: self.pricing_currency,
            pricing_availability,
            per_mtok_prompt: self.per_mtok_prompt,
            per_mtok_completion: self.per_mtok_completion,
            per_mtok_cache_read: self.per_mtok_cache_read,
        })
    }
}

impl RawProviderProfile {
    fn into_product(self) -> Result<ProductProviderProfile, ProductStoreError> {
        let credential_source = self
            .api_key_env
            .as_ref()
            .map(|name| ProductProviderCredentialSource::Env { name: name.clone() })
            .unwrap_or(ProductProviderCredentialSource::None);
        Ok(ProductProviderProfile {
            id: parse_product_id(&self.id, "provider profile id")?,
            label: self.label,
            provider_type: provider_type_from_db(&self.provider_type)?,
            api_base: self.api_base,
            api_key_env: self.api_key_env,
            credential_source,
            default_model: self.default_model,
            created_at: self.created_at,
            updated_at: self.updated_at,
            catalog_revision: "legacy-product-store".to_string(),
        })
    }
}

fn raw_workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawWorkspace> {
    Ok(RawWorkspace {
        id: row.get(0)?,
        canonical_root: row.get(1)?,
        kind: row.get(2)?,
        display_name: row.get(3)?,
        pinned: row.get(4)?,
        last_opened_at: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn raw_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSession> {
    Ok(RawSession {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        latest_ordinal: row.get(4)?,
        runtime_session_id: row.get(5)?,
        latest_job_id: row.get(6)?,
        latest_run_id: row.get(7)?,
        parent_session_id: row.get(8)?,
        fork_point_run_id: row.get(9)?,
        fork_point_seq: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn raw_binding_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawBinding> {
    Ok(RawBinding {
        product_session_id: row.get(0)?,
        ordinal: row.get(1)?,
        runtime_session_id: row.get(2)?,
        runtime_job_id: row.get(3)?,
        runtime_run_id: row.get(4)?,
        resumed_from_run_id: row.get(5)?,
        bound_at: row.get(6)?,
    })
}

fn raw_fork_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawFork> {
    Ok(RawFork {
        id: row.get(0)?,
        parent_session_id: row.get(1)?,
        child_session_id: row.get(2)?,
        parent_workspace_id: row.get(3)?,
        parent_title: row.get(4)?,
        source_runtime_session_id: row.get(5)?,
        source_runtime_job_id: row.get(6)?,
        source_runtime_run_id: row.get(7)?,
        fork_at_event_seq: row.get(8)?,
        idempotency_key: row.get(9)?,
        request_digest: row.get(10)?,
        created_at: row.get(11)?,
    })
}

fn raw_fork_inherited_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RawForkInheritedRun> {
    Ok(RawForkInheritedRun {
        ordinal: row.get(0)?,
        source_product_session_id: row.get(1)?,
        runtime_session_id: row.get(2)?,
        runtime_job_id: row.get(3)?,
        runtime_run_id: row.get(4)?,
        through_event_seq: row.get(5)?,
    })
}

fn raw_provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawProviderProfile> {
    Ok(RawProviderProfile {
        id: row.get(0)?,
        label: row.get(1)?,
        provider_type: row.get(2)?,
        api_base: row.get(3)?,
        api_key_env: row.get(4)?,
        default_model: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn raw_session_model_config_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RawSessionModelConfig> {
    Ok(RawSessionModelConfig {
        product_session_id: row.get(0)?,
        profile_id: row.get(1)?,
        model: row.get(2)?,
        reasoning: row.get(3)?,
        max_steps: row.get(4)?,
        revision: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn get_workspace(
    connection: &Connection,
    workspace_id: &ProductWorkspaceId,
) -> Result<ProductWorkspace, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT workspace_id, canonical_root, kind, display_name, pinned,
                   last_opened_at, created_at, updated_at
            FROM product_workspaces WHERE workspace_id = ?1
            "#,
            params![workspace_id.to_string()],
            raw_workspace_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| not_found("product workspace was not found"))?
        .into_product()
}

fn require_workspace(
    connection: &Connection,
    workspace_id: &ProductWorkspaceId,
) -> Result<(), ProductStoreError> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM product_workspaces WHERE workspace_id = ?1)",
            params![workspace_id.to_string()],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if exists {
        Ok(())
    } else {
        Err(not_found("product workspace was not found"))
    }
}

fn find_workspace_by_key(
    connection: &Connection,
    canonical_key: &str,
) -> Result<Option<ProductWorkspace>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT workspace_id, canonical_root, kind, display_name, pinned,
                   last_opened_at, created_at, updated_at
            FROM product_workspaces WHERE canonical_key = ?1
            "#,
            params![canonical_key],
            raw_workspace_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .map(RawWorkspace::into_product)
        .transpose()
}

fn get_session(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<ProductSession, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT product_session_id, workspace_id, title, status, latest_ordinal,
                   runtime_session_id, latest_job_id, latest_run_id,
                   parent_session_id, fork_point_run_id, fork_point_seq,
                   created_at, updated_at
            FROM product_sessions WHERE product_session_id = ?1
            "#,
            params![session_id.to_string()],
            raw_session_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| not_found("product session was not found"))?
        .into_product()
}

fn default_session_model_values(
    connection: &Connection,
) -> Result<(Option<String>, String, u32), ProductStoreError> {
    let row = connection
        .query_row(
            "SELECT provider_profile_id, provider_model, provider_max_steps FROM product_preferences WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .map_err(storage_error)?;
    let max_steps = row
        .2
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| (1..=MAX_PRODUCT_MAX_STEPS).contains(value))
        .unwrap_or(DEFAULT_PRODUCT_MAX_STEPS);
    Ok((
        row.0,
        row.1
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| "fake".to_string()),
        max_steps,
    ))
}

fn ensure_session_model_config(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<(), ProductStoreError> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM product_session_model_configs WHERE product_session_id = ?1)",
            params![session_id.to_string()],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if exists {
        return Ok(());
    }
    let (profile_id, model, max_steps) = default_session_model_values(connection)?;
    let updated_at = now_rfc3339();
    insert_session_model_config(
        connection,
        SessionModelConfigWrite {
            session_id,
            profile_id: profile_id.as_deref(),
            model: &model,
            reasoning: ProductReasoningPreference::Default,
            max_steps,
            revision: 1,
            updated_at: &updated_at,
        },
    )
}

struct SessionModelConfigWrite<'a> {
    session_id: &'a ProductSessionId,
    profile_id: Option<&'a str>,
    model: &'a str,
    reasoning: ProductReasoningPreference,
    max_steps: u32,
    revision: u64,
    updated_at: &'a str,
}

fn insert_session_model_config(
    connection: &Connection,
    write: SessionModelConfigWrite<'_>,
) -> Result<(), ProductStoreError> {
    if !(1..=MAX_PRODUCT_MAX_STEPS).contains(&write.max_steps) {
        return Err(binding_corrupt("persisted session max_steps is invalid"));
    }
    connection
        .execute(
            r#"
            INSERT INTO product_session_model_configs(
                product_session_id, profile_id, model, reasoning, max_steps,
                revision, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                write.session_id.to_string(),
                write.profile_id,
                write.model,
                write.reasoning.as_str(),
                i64::from(write.max_steps),
                i64::try_from(write.revision)
                    .map_err(|_| binding_corrupt("session model revision is invalid"))?,
                write.updated_at,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn insert_or_update_session_model_config(
    connection: &Connection,
    write: SessionModelConfigWrite<'_>,
) -> Result<(), ProductStoreError> {
    connection
        .execute(
            r#"
            INSERT INTO product_session_model_configs(
                product_session_id, profile_id, model, reasoning, max_steps,
                revision, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(product_session_id) DO UPDATE SET
                profile_id = excluded.profile_id,
                model = excluded.model,
                reasoning = excluded.reasoning,
                max_steps = excluded.max_steps,
                revision = excluded.revision,
                updated_at = excluded.updated_at
            "#,
            params![
                write.session_id.to_string(),
                write.profile_id,
                write.model,
                write.reasoning.as_str(),
                i64::from(write.max_steps),
                i64::try_from(write.revision)
                    .map_err(|_| binding_corrupt("session model revision is invalid"))?,
                write.updated_at,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn get_session_model_config_in_transaction(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<ProductSessionModelConfig, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT product_session_id, profile_id, model, reasoning, max_steps,
                   revision, updated_at
            FROM product_session_model_configs
            WHERE product_session_id = ?1
            "#,
            params![session_id.to_string()],
            raw_session_model_config_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| binding_corrupt("product session model config is missing"))?
        .into_product()
}

fn find_fork_by_parent_and_key(
    connection: &Connection,
    parent_session_id: &ProductSessionId,
    idempotency_key: &str,
) -> Result<Option<StoredFork>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT fork_id, parent_product_session_id, child_product_session_id,
                   parent_workspace_id, parent_title, source_runtime_session_id,
                   source_runtime_job_id, source_runtime_run_id, fork_at_event_seq,
                   idempotency_key, request_digest, created_at
            FROM product_session_forks
            WHERE parent_product_session_id = ?1 AND idempotency_key = ?2
            "#,
            params![parent_session_id.to_string(), idempotency_key],
            raw_fork_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .map(RawFork::into_stored)
        .transpose()
}

fn replay_fork_if_exists(
    connection: &Connection,
    parent_session_id: &ProductSessionId,
    idempotency_key: &str,
    request_digest: &str,
) -> Result<Option<(ProductSession, ProductFork)>, ProductStoreError> {
    let Some(existing) =
        find_fork_by_parent_and_key(connection, parent_session_id, idempotency_key)?
    else {
        return Ok(None);
    };
    if existing.request_digest != request_digest {
        return Err(fork_conflict(
            "idempotency_key already exists for a different fork request",
        ));
    }
    let child = get_session(connection, &existing.fork.child_product_session_id).map_err(|error| {
        if error.code == ProductErrorCode::ProductNotFound {
            fork_conflict(
                "the idempotent fork child was deleted and cannot be recreated with the same key",
            )
        } else {
            error
        }
    })?;
    Ok(Some((child, existing.fork)))
}

fn find_fork_by_child(
    connection: &Connection,
    child_session_id: &ProductSessionId,
) -> Result<Option<StoredFork>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT fork_id, parent_product_session_id, child_product_session_id,
                   parent_workspace_id, parent_title, source_runtime_session_id,
                   source_runtime_job_id, source_runtime_run_id, fork_at_event_seq,
                   idempotency_key, request_digest, created_at
            FROM product_session_forks
            WHERE child_product_session_id = ?1
            "#,
            params![child_session_id.to_string()],
            raw_fork_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .map(RawFork::into_stored)
        .transpose()
}

fn list_fork_inherited_runs(
    connection: &Connection,
    fork_id: &ProductForkId,
) -> Result<Vec<ProductForkInheritedRun>, ProductStoreError> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT ordinal, source_product_session_id, runtime_session_id,
                   runtime_job_id, runtime_run_id, through_event_seq
            FROM product_fork_inherited_runs
            WHERE fork_id = ?1
            ORDER BY ordinal ASC
            LIMIT ?2
            "#,
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![
                fork_id.to_string(),
                i64::try_from(MAX_PRODUCT_FORK_INHERITED_RUNS + 1).map_err(storage_error)?
            ],
            raw_fork_inherited_run_from_row,
        )
        .map_err(storage_error)?;
    let mut inherited = Vec::new();
    for row in rows {
        inherited.push(row.map_err(storage_error)?.into_product()?);
    }
    if inherited.is_empty() || inherited.len() > MAX_PRODUCT_FORK_INHERITED_RUNS {
        return Err(binding_corrupt(
            "fork inherited runtime history is absent or exceeds its limit",
        ));
    }
    for (index, inherited_run) in inherited.iter().enumerate() {
        let expected = u64::try_from(index + 1).map_err(storage_error)?;
        if inherited_run.ordinal != expected {
            return Err(binding_corrupt(
                "fork inherited runtime history is not contiguous",
            ));
        }
    }
    Ok(inherited)
}

fn get_fork_context(
    connection: &Connection,
    session: &ProductSession,
) -> Result<Option<ProductForkContext>, ProductStoreError> {
    let stored = find_fork_by_child(connection, &session.id)?;
    let Some(parent_session_id) = session.parent_session_id.as_ref() else {
        if stored.is_some() {
            return Err(binding_corrupt(
                "product session has fork provenance without a parent session pointer",
            ));
        }
        return Ok(None);
    };
    let (Some(fork_point_run_id), Some(fork_point_seq), Some(stored)) =
        (session.fork_point_run_id, session.fork_point_seq, stored)
    else {
        return Err(binding_corrupt(
            "product session fork provenance is incomplete",
        ));
    };
    let fork = stored.fork;
    if fork.parent_product_session_id != *parent_session_id
        || fork.child_product_session_id != session.id
        || fork.parent_workspace_id != session.workspace_id
        || fork.source_runtime_run_id != fork_point_run_id
        || fork.fork_at_event_seq != fork_point_seq
    {
        return Err(binding_corrupt(
            "product session fork provenance does not match its immutable fork record",
        ));
    }
    let inherited_runs = list_fork_inherited_runs(connection, &fork.id)?;
    let Some(boundary) = inherited_runs.last() else {
        return Err(binding_corrupt("fork inherited runtime history is absent"));
    };
    if boundary.source_product_session_id != fork.parent_product_session_id
        || boundary.runtime_session_id != fork.source_runtime_session_id
        || boundary.runtime_job_id != fork.source_runtime_job_id
        || boundary.runtime_run_id != fork.source_runtime_run_id
        || boundary.through_event_seq != Some(fork.fork_at_event_seq)
    {
        return Err(binding_corrupt(
            "fork inherited runtime history does not end at its stored boundary",
        ));
    }
    Ok(Some(ProductForkContext {
        fork,
        inherited_runs,
    }))
}

/// The listing's rank for a session: live sessions sort before archived ones.
///
/// This mirrors the `CASE` expression in the SQL and in
/// `idx_product_sessions_workspace_page`. All three must agree, or a cursor
/// minted from a row would not find that row again.
fn archived_rank(status: ProductSessionStatus) -> i64 {
    match status {
        ProductSessionStatus::Archived => SESSION_RANK_ARCHIVED,
        _ => SESSION_RANK_LIVE,
    }
}

/// Mint the cursor that resumes immediately after `session`.
fn cursor_for_session(session: &ProductSession) -> ProductSessionCursor {
    ProductSessionCursor::after(
        archived_rank(session.status),
        &session.updated_at,
        session.id.clone(),
    )
}

/// Escape a user-supplied search term for use inside a `LIKE` pattern.
///
/// Without this, a title search for `100%` would match every title, and `_`
/// would match any character — the user would get results they did not ask for
/// and could not explain. The escape character is declared by `ESCAPE` in the
/// SQL, and the backslash itself must be escaped first or escaping the other
/// two would be reversible by the input.
fn like_pattern(term: &str) -> String {
    let mut pattern = String::with_capacity(term.len() + 2);
    pattern.push('%');
    for character in term.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

/// Build the query for one rank group of the page.
///
/// The rank is bound as an equality and is therefore *absent* from `ORDER BY`:
/// it is constant across every row the query can return. That is what lets
/// `idx_product_sessions_workspace_page` satisfy the ordering by scan position
/// alone, with no sorting step. Writing `ORDER BY rank, updated_at DESC, id`
/// here instead — the more obvious form — reintroduces the temp B-tree, because
/// SQLite matches `ORDER BY` against the index prefix textually and the leading
/// `CASE` expression is not something it will simplify away.
///
/// Within a group the keyset predicate is a plain two-term disjunction: an
/// *earlier* timestamp, or the same timestamp and a *greater* id. The middle
/// sort term is `DESC`, so "after" is `<`.
pub(super) fn rank_page_sql(query: &ProductSessionPageQuery, resuming: bool) -> String {
    const RANK: &str = "CASE WHEN status = 'archived' THEN 1 ELSE 0 END";
    let mut sql = format!(
        r#"
        SELECT product_session_id, workspace_id, title, status, latest_ordinal,
               runtime_session_id, latest_job_id, latest_run_id,
               parent_session_id, fork_point_run_id, fork_point_seq,
               created_at, updated_at
        FROM product_sessions
        WHERE workspace_id = ?1 AND {RANK} = ?2
        "#
    );
    let mut next_index = 3;
    if resuming {
        let updated_at = next_index;
        let session_id = next_index + 1;
        sql.push_str(&format!(
            " AND (updated_at < ?{updated_at}
                   OR (updated_at = ?{updated_at}
                       AND product_session_id > ?{session_id}))\n"
        ));
        next_index += 2;
    }
    if query.search.is_some() {
        sql.push_str(&format!(" AND title LIKE ?{next_index} ESCAPE '\\'\n"));
        next_index += 1;
    }
    sql.push_str(&format!(
        " ORDER BY updated_at DESC, product_session_id ASC
          LIMIT ?{next_index}"
    ));
    sql
}

/// Bind the parameters in the same order [`rank_page_sql`] numbered them.
fn rank_page_params(
    query: &ProductSessionPageQuery,
    rank: i64,
    resume: Option<&ProductSessionCursor>,
    limit: i64,
) -> Vec<Box<dyn rusqlite::ToSql>> {
    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(query.workspace_id.to_string()), Box::new(rank)];
    if let Some(cursor) = resume {
        params.push(Box::new(cursor.updated_at.clone()));
        params.push(Box::new(cursor.session_id.to_string()));
    }
    if let Some(term) = &query.search {
        params.push(Box::new(like_pattern(term)));
    }
    params.push(Box::new(limit));
    params
}

fn list_and_validate_bindings(
    connection: &Connection,
    session: &ProductSession,
) -> Result<Vec<ProductSessionRunBinding>, ProductStoreError> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT product_session_id, ordinal, runtime_session_id, runtime_job_id,
                   runtime_run_id, resumed_from_run_id, bound_at
            FROM product_session_runs
            WHERE product_session_id = ?1
            ORDER BY ordinal ASC
            LIMIT ?2
            "#,
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![
                session.id.to_string(),
                i64::try_from(MAX_RUN_BINDINGS_PER_SESSION + 1).map_err(storage_error)?
            ],
            raw_binding_from_row,
        )
        .map_err(storage_error)?;
    let mut bindings = Vec::new();
    for row in rows {
        bindings.push(row.map_err(storage_error)?.into_product()?);
    }
    if bindings.len() > usize::try_from(MAX_RUN_BINDINGS_PER_SESSION).map_err(storage_error)? {
        return Err(binding_corrupt(
            "product session run binding limit exceeded",
        ));
    }
    for (index, binding) in bindings.iter().enumerate() {
        let expected = u64::try_from(index).map_err(storage_error)? + 1;
        if binding.product_session_id != session.id || binding.ordinal != expected {
            return Err(binding_corrupt(
                "product session run bindings are not contiguous",
            ));
        }
        if index == 0 {
            if binding.resumed_from_run_id.is_some() {
                return Err(binding_corrupt(
                    "first product session run binding cannot resume another run",
                ));
            }
        } else {
            let previous = &bindings[index - 1];
            if binding.runtime_session_id != previous.runtime_session_id
                || binding.runtime_job_id != previous.runtime_job_id
                || binding.resumed_from_run_id != Some(previous.runtime_run_id)
            {
                return Err(binding_corrupt(
                    "product session run binding chain is inconsistent",
                ));
            }
        }
    }
    match (bindings.last(), session.runtime_binding.as_ref()) {
        (None, None) => {}
        (Some(latest), Some(snapshot))
            if latest.ordinal == snapshot.ordinal
                && latest.runtime_session_id == snapshot.runtime_session_id
                && latest.runtime_job_id == snapshot.latest_job_id
                && latest.runtime_run_id == snapshot.latest_run_id => {}
        _ => {
            return Err(binding_corrupt(
                "product session latest binding does not match its immutable run ledger",
            ));
        }
    }
    Ok(bindings)
}

fn validate_binding_integrity(
    connection: &Connection,
    session: &ProductSession,
) -> Result<(), ProductStoreError> {
    list_and_validate_bindings(connection, session).map(|_| ())
}

fn has_active_claim_for_session(
    connection: &Connection,
    session_id: &ProductSessionId,
) -> Result<bool, ProductStoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM product_turn_claims WHERE product_session_id = ?1)",
            params![session_id.to_string()],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn find_binding_by_runtime_run(
    connection: &Connection,
    run_id: RunId,
) -> Result<Option<ProductSessionRunBinding>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT product_session_id, ordinal, runtime_session_id, runtime_job_id,
                   runtime_run_id, resumed_from_run_id, bound_at
            FROM product_session_runs WHERE runtime_run_id = ?1
            "#,
            params![run_id.to_string()],
            raw_binding_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .map(RawBinding::into_product)
        .transpose()
}

fn binding_matches_commit(
    existing: &ProductSessionRunBinding,
    requested: &CommitProductRunBinding,
) -> bool {
    existing.product_session_id == requested.product_session_id
        && existing.runtime_session_id == requested.runtime_session_id
        && existing.runtime_job_id == requested.runtime_job_id
        && existing.runtime_run_id == requested.runtime_run_id
        && existing.resumed_from_run_id == requested.resumed_from_run_id
}

fn insert_session_run_model_snapshot(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
    ordinal: u64,
    runtime_run_id: &RunId,
    config: &ProductSessionModelConfig,
    snapshot: Option<&rove_runtime::runtime_identity::RunModelSnapshot>,
) -> Result<(), ProductStoreError> {
    if config.product_session_id != *session_id {
        return Err(resume_conflict(
            "run model snapshot does not belong to the claimed product session",
        ));
    }
    if let Some(snapshot) = snapshot {
        let programmatic_fake = config.profile_id.is_none()
            && snapshot.profile_id == "programmatic-fake"
            && snapshot.provider_type == "fake"
            && snapshot.wire_protocol == "fake"
            && snapshot.endpoint.is_empty()
            && snapshot.catalog_revision == "programmatic";
        let profile_matches = config
            .profile_id
            .as_ref()
            .map(ToString::to_string)
            .is_some_and(|profile_id| profile_id == snapshot.profile_id)
            || programmatic_fake;
        if snapshot.model != config.model
            || snapshot.reasoning != config.reasoning.as_str()
            || !profile_matches
        {
            return Err(resume_conflict(
                "run model snapshot does not match the claimed session selection",
            ));
        }
    }
    let pricing = crate::pricing::PricingSnapshot::bundled_for_model(&config.model);
    let inserted = transaction
        .execute(
            r#"
            INSERT OR IGNORE INTO product_session_run_models(
                product_session_id, ordinal, runtime_run_id, profile_id,
                model, reasoning, max_steps, started_at,
                provider_type, wire_protocol, endpoint, catalog_revision,
                safe_config_digest,
                context_window,
                pricing_source, pricing_version, pricing_currency,
                pricing_availability, per_mtok_prompt, per_mtok_completion,
                per_mtok_cache_read
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
            "#,
            params![
                session_id.to_string(),
                i64::try_from(ordinal).map_err(storage_error)?,
                runtime_run_id.to_string(),
                config.profile_id.as_ref().map(ToString::to_string),
                config.model,
                config.reasoning.as_str(),
                i64::from(config.max_steps),
                now_rfc3339(),
                snapshot.map(|value| value.provider_type.as_str()),
                snapshot.map(|value| value.wire_protocol.as_str()),
                snapshot.map(|value| value.endpoint.as_str()),
                snapshot.map(|value| value.catalog_revision.as_str()),
                snapshot.map(|value| value.safe_config_digest.as_str()),
                crate::pricing::bundled_context_window(&config.model)
                    .and_then(|value| i64::try_from(value).ok()),
                pricing.source,
                pricing.version,
                pricing.currency,
                pricing.availability.as_str(),
                pricing.per_mtok_prompt,
                pricing.per_mtok_completion,
                pricing.per_mtok_cache_read,
            ],
        )
        .map_err(storage_error)?;
    if inserted == 0 {
        let existing = transaction
            .query_row(
                r#"
                SELECT product_session_id, ordinal, runtime_run_id, profile_id,
                       model, reasoning, max_steps,
                       provider_type, wire_protocol, endpoint, catalog_revision,
                       safe_config_digest,
                       context_window,
                       pricing_source, pricing_version, pricing_currency,
                       pricing_availability, per_mtok_prompt, per_mtok_completion,
                       per_mtok_cache_read
                FROM product_session_run_models
                WHERE runtime_run_id = ?1
                "#,
                params![runtime_run_id.to_string()],
                |row| {
                    Ok(RawSessionRunModel {
                        product_session_id: row.get(0)?,
                        ordinal: row.get(1)?,
                        runtime_run_id: row.get(2)?,
                        profile_id: row.get(3)?,
                        model: row.get(4)?,
                        reasoning: row.get(5)?,
                        max_steps: row.get(6)?,
                        provider_type: row.get(7)?,
                        wire_protocol: row.get(8)?,
                        endpoint: row.get(9)?,
                        catalog_revision: row.get(10)?,
                        safe_config_digest: row.get(11)?,
                        context_window: row.get(12)?,
                        pricing_source: row.get(13)?,
                        pricing_version: row.get(14)?,
                        pricing_currency: row.get(15)?,
                        pricing_availability: row.get(16)?,
                        per_mtok_prompt: row.get(17)?,
                        per_mtok_completion: row.get(18)?,
                        per_mtok_cache_read: row.get(19)?,
                    })
                },
            )
            .map_err(storage_error)?
            .into_product()?;
        let same_identity = existing.product_session_id == *session_id
            && existing.ordinal == ordinal
            && existing.runtime_run_id == *runtime_run_id
            && existing.profile_id == config.profile_id
            && existing.model == config.model
            && existing.reasoning == config.reasoning
            && existing.max_steps == config.max_steps
            && existing.provider_type == snapshot.map(|value| value.provider_type.clone())
            && existing.wire_protocol == snapshot.map(|value| value.wire_protocol.clone())
            && existing.endpoint == snapshot.map(|value| value.endpoint.clone())
            && existing.catalog_revision == snapshot.map(|value| value.catalog_revision.clone())
            && existing.safe_config_digest
                == snapshot.map(|value| value.safe_config_digest.clone());
        if !same_identity {
            return Err(resume_conflict(
                "runtime run already has a different model snapshot",
            ));
        }
    }
    Ok(())
}

fn next_binding_ordinal(session: &ProductSession) -> Result<u64, ProductStoreError> {
    let ordinal = match &session.runtime_binding {
        Some(binding) => binding
            .ordinal
            .checked_add(1)
            .ok_or_else(|| binding_corrupt("product run binding ordinal overflowed"))?,
        None => 1,
    };
    if ordinal > MAX_RUN_BINDINGS_PER_SESSION {
        return Err(invalid("product session run binding limit reached"));
    }
    Ok(ordinal)
}

fn validate_commit_chain(
    session: &ProductSession,
    binding: &CommitProductRunBinding,
) -> Result<(), ProductStoreError> {
    match &session.runtime_binding {
        None if binding.resumed_from_run_id.is_none() => Ok(()),
        None => Err(resume_conflict(
            "first product session turn cannot claim a resumed runtime run",
        )),
        Some(previous)
            if previous.runtime_session_id == binding.runtime_session_id
                && previous.latest_job_id == binding.runtime_job_id
                && binding.resumed_from_run_id == Some(previous.latest_run_id)
                && binding.runtime_run_id != previous.latest_run_id =>
        {
            Ok(())
        }
        Some(_) => Err(resume_conflict(
            "runtime identity does not continue the product session's exact latest run",
        )),
    }
}

fn claim_runtime_ownership(
    transaction: &Transaction<'_>,
    product_session_id: &ProductSessionId,
    runtime_session_id: SessionId,
    runtime_job_id: JobId,
    conflict_code: ProductErrorCode,
) -> Result<(), ProductStoreError> {
    transaction
        .execute(
            r#"
            INSERT OR IGNORE INTO product_runtime_session_owners(
                runtime_session_id, product_session_id
            ) VALUES (?1, ?2)
            "#,
            params![
                runtime_session_id.to_string(),
                product_session_id.to_string()
            ],
        )
        .map_err(storage_error)?;
    let session_owner: String = transaction
        .query_row(
            "SELECT product_session_id FROM product_runtime_session_owners WHERE runtime_session_id = ?1",
            params![runtime_session_id.to_string()],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if session_owner != product_session_id.to_string() {
        return Err(ProductStoreError::new(
            conflict_code,
            "runtime session is already owned by another product session",
        ));
    }

    transaction
        .execute(
            r#"
            INSERT OR IGNORE INTO product_runtime_job_owners(
                runtime_job_id, runtime_session_id, product_session_id
            ) VALUES (?1, ?2, ?3)
            "#,
            params![
                runtime_job_id.to_string(),
                runtime_session_id.to_string(),
                product_session_id.to_string(),
            ],
        )
        .map_err(storage_error)?;
    let (job_session, job_owner): (String, String) = transaction
        .query_row(
            r#"
            SELECT runtime_session_id, product_session_id
            FROM product_runtime_job_owners WHERE runtime_job_id = ?1
            "#,
            params![runtime_job_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(storage_error)?;
    if job_session != runtime_session_id.to_string() || job_owner != product_session_id.to_string()
    {
        return Err(ProductStoreError::new(
            conflict_code,
            "runtime job is already owned by another product session",
        ));
    }
    Ok(())
}

struct NewRunBinding {
    product_session_id: ProductSessionId,
    ordinal: u64,
    runtime_session_id: SessionId,
    runtime_job_id: JobId,
    runtime_run_id: RunId,
    resumed_from_run_id: Option<RunId>,
    migration_receipt_id: Option<ProductMigrationReceiptId>,
}

fn insert_run_binding(
    transaction: &Transaction<'_>,
    binding: NewRunBinding,
) -> Result<ProductSessionRunBinding, ProductStoreError> {
    let bound_at = now_rfc3339();
    transaction
        .execute(
            r#"
            INSERT INTO product_session_runs(
                product_session_id, ordinal, runtime_session_id, runtime_job_id,
                runtime_run_id, resumed_from_run_id, bound_at, migration_receipt_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                binding.product_session_id.to_string(),
                i64::try_from(binding.ordinal).map_err(storage_error)?,
                binding.runtime_session_id.to_string(),
                binding.runtime_job_id.to_string(),
                binding.runtime_run_id.to_string(),
                binding.resumed_from_run_id.map(|id| id.to_string()),
                bound_at,
                binding
                    .migration_receipt_id
                    .as_ref()
                    .map(ToString::to_string),
            ],
        )
        .map_err(storage_error)?;
    Ok(ProductSessionRunBinding {
        product_session_id: binding.product_session_id,
        ordinal: binding.ordinal,
        runtime_session_id: binding.runtime_session_id,
        runtime_job_id: binding.runtime_job_id,
        runtime_run_id: binding.runtime_run_id,
        resumed_from_run_id: binding.resumed_from_run_id,
        bound_at,
    })
}

fn update_latest_binding_cas(
    transaction: &Transaction<'_>,
    previous: &ProductSession,
    binding: &ProductSessionRunBinding,
) -> Result<(), ProductStoreError> {
    let expected_ordinal = previous
        .runtime_binding
        .as_ref()
        .map(|value| i64::try_from(value.ordinal).map_err(storage_error))
        .transpose()?;
    let updated = transaction
        .execute(
            r#"
            UPDATE product_sessions
            SET latest_ordinal = ?2, runtime_session_id = ?3, latest_job_id = ?4,
                latest_run_id = ?5, updated_at = ?6
            WHERE product_session_id = ?1
              AND (latest_ordinal IS ?7 OR latest_ordinal = ?7)
            "#,
            params![
                previous.id.to_string(),
                i64::try_from(binding.ordinal).map_err(storage_error)?,
                binding.runtime_session_id.to_string(),
                binding.runtime_job_id.to_string(),
                binding.runtime_run_id.to_string(),
                now_rfc3339(),
                expected_ordinal,
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(resume_conflict(
            "product session latest binding changed during commit",
        ));
    }
    Ok(())
}

fn get_provider_profile(
    connection: &Connection,
    profile_id: &ProductProviderProfileId,
) -> Result<ProductProviderProfile, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT profile_id, label, provider_type, api_base, api_key_env,
                   default_model, created_at, updated_at
            FROM product_provider_profiles WHERE profile_id = ?1
            "#,
            params![profile_id.to_string()],
            raw_provider_from_row,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| not_found("product provider profile was not found"))?
        .into_product()
}

fn insert_provider_profile(
    transaction: &Transaction<'_>,
    profile_id: &ProductProviderProfileId,
    profile: &ValidatedProviderProfile,
    created_at: &str,
    updated_at: &str,
) -> Result<(), ProductStoreError> {
    transaction
        .execute(
            r#"
            INSERT INTO product_provider_profiles(
                profile_id, label, provider_type, api_base, api_key_env,
                default_model, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                profile_id.to_string(),
                profile.label,
                provider_type_to_db(profile.provider_type),
                profile.api_base,
                profile.api_key_env,
                profile.default_model,
                created_at,
                updated_at,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn update_provider_profile_row(
    transaction: &Transaction<'_>,
    profile_id: &ProductProviderProfileId,
    profile: &ValidatedProviderProfile,
    updated_at: &str,
) -> Result<(), ProductStoreError> {
    let updated = transaction
        .execute(
            r#"
            UPDATE product_provider_profiles
            SET label = ?2, provider_type = ?3, api_base = ?4, api_key_env = ?5,
                default_model = ?6, updated_at = ?7
            WHERE profile_id = ?1
            "#,
            params![
                profile_id.to_string(),
                profile.label,
                provider_type_to_db(profile.provider_type),
                profile.api_base,
                profile.api_key_env,
                profile.default_model,
                updated_at,
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(not_found("product provider profile was not found"));
    }
    Ok(())
}

fn get_preferences(connection: &Connection) -> Result<ProductPreferences, ProductStoreError> {
    let raw = connection
        .query_row(
            r#"
            SELECT schema_version, revision, theme, default_approval_policy,
                   active_workspace_id, active_session_id,
                   provider_profile_id, provider_model, provider_approval,
                   provider_max_steps
            FROM product_preferences WHERE singleton = 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| storage_error("product preferences row is missing"))?;
    let schema_version = u32::try_from(raw.0).map_err(storage_error)?;
    if schema_version != 1 {
        return Err(storage_error(
            "persisted product preferences schema is unsupported",
        ));
    }
    let revision = u64::try_from(raw.1).map_err(storage_error)?;
    let active_workspace_id = raw
        .4
        .map(|value| parse_product_id(&value, "active workspace id"))
        .transpose()?;
    let active_session_id = raw
        .5
        .map(|value| parse_product_id(&value, "active session id"))
        .transpose()?;
    let provider_selection = match (raw.6, raw.7, raw.8, raw.9) {
        (None, None, None, None) => None,
        (profile_id, Some(model), Some(approval), Some(max_steps)) => Some(
            validate_provider_selection(ProductProviderSelection {
                profile_id: profile_id
                    .map(|value| parse_product_id(&value, "provider profile id"))
                    .transpose()?,
                model,
                approval: approval_from_db(&approval)?,
                max_steps: u32::try_from(max_steps).map_err(storage_error)?,
            })
            .map_err(|_| storage_error("persisted provider selection is invalid"))?,
        ),
        _ => return Err(storage_error("product preferences are corrupt")),
    };
    let preferences = ProductPreferences {
        schema_version,
        revision,
        theme: theme_from_db(&raw.2)?,
        default_approval_policy: approval_from_db(&raw.3)?,
        active_workspace_id,
        active_session_id,
        provider_selection,
    };
    if let Some(active_workspace_id) = &preferences.active_workspace_id {
        require_workspace(connection, active_workspace_id)
            .map_err(|_| storage_error("persisted active workspace reference is invalid"))?;
    }
    if let Some(active_session_id) = &preferences.active_session_id {
        let session = get_session(connection, active_session_id)
            .map_err(|_| storage_error("persisted active session reference is invalid"))?;
        if preferences.active_workspace_id.as_ref() != Some(&session.workspace_id) {
            return Err(storage_error(
                "persisted active session does not belong to the active workspace",
            ));
        }
    }
    if let Some(profile_id) = preferences
        .provider_selection
        .as_ref()
        .and_then(|selection| selection.profile_id.as_ref())
    {
        get_provider_profile(connection, profile_id)
            .map_err(|_| storage_error("persisted provider profile reference is invalid"))?;
    }
    Ok(preferences)
}

fn preferences_revision(connection: &Connection) -> Result<u64, ProductStoreError> {
    let revision = connection
        .query_row(
            "SELECT revision FROM product_preferences WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| storage_error("product preferences row is missing"))?;
    u64::try_from(revision).map_err(storage_error)
}

fn validate_preference_references(
    connection: &Connection,
    preferences: &ValidatedPreferences,
) -> Result<(), ProductStoreError> {
    if let Some(workspace_id) = &preferences.active_workspace_id {
        require_workspace(connection, workspace_id)?;
    }
    if let Some(session_id) = &preferences.active_session_id {
        let session = get_session(connection, session_id)?;
        if Some(&session.workspace_id) != preferences.active_workspace_id.as_ref() {
            return Err(invalid(
                "active session does not belong to the active workspace",
            ));
        }
    }
    if let Some(profile_id) = preferences
        .provider_selection
        .as_ref()
        .and_then(|selection| selection.profile_id.as_ref())
    {
        get_provider_profile(connection, profile_id)?;
    }
    Ok(())
}

fn write_preferences(
    transaction: &Transaction<'_>,
    preferences: &ValidatedPreferences,
) -> Result<(), ProductStoreError> {
    let (profile_id, model, approval, max_steps) = match &preferences.provider_selection {
        Some(selection) => (
            profile_id_string(selection.profile_id.as_ref()),
            Some(selection.model.clone()),
            Some(approval_to_db(selection.approval)),
            Some(i64::from(selection.max_steps)),
        ),
        None => (None, None, None, None),
    };
    let updated = transaction
        .execute(
            r#"
            UPDATE product_preferences
            SET schema_version = ?1, theme = ?2, active_workspace_id = ?3,
                active_session_id = ?4, provider_profile_id = ?5, provider_model = ?6,
                provider_approval = ?7, provider_max_steps = ?8,
                default_approval_policy = COALESCE(?9, default_approval_policy),
                updated_at = ?10,
                revision = revision + 1
            WHERE singleton = 1
            "#,
            params![
                i64::from(preferences.schema_version),
                theme_to_db(preferences.theme),
                preferences
                    .active_workspace_id
                    .as_ref()
                    .map(ToString::to_string),
                preferences
                    .active_session_id
                    .as_ref()
                    .map(ToString::to_string),
                profile_id,
                model,
                approval,
                max_steps,
                preferences.default_approval_policy.map(approval_to_db),
                now_rfc3339(),
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(storage_error("product preferences row is missing"));
    }
    Ok(())
}

fn write_preferences_at_revision(
    transaction: &Transaction<'_>,
    preferences: &ValidatedPreferences,
    expected_revision: u64,
) -> Result<bool, ProductStoreError> {
    let (profile_id, model, approval, max_steps) = match &preferences.provider_selection {
        Some(selection) => (
            profile_id_string(selection.profile_id.as_ref()),
            Some(selection.model.clone()),
            Some(approval_to_db(selection.approval)),
            Some(i64::from(selection.max_steps)),
        ),
        None => (None, None, None, None),
    };
    let updated = transaction
        .execute(
            r#"
            UPDATE product_preferences
            SET schema_version = ?1, theme = ?2, active_workspace_id = ?3,
                active_session_id = ?4, provider_profile_id = ?5, provider_model = ?6,
                provider_approval = ?7, provider_max_steps = ?8,
                default_approval_policy = COALESCE(?9, default_approval_policy),
                updated_at = ?10,
                revision = revision + 1
            WHERE singleton = 1 AND revision = ?11
            "#,
            params![
                i64::from(preferences.schema_version),
                theme_to_db(preferences.theme),
                preferences
                    .active_workspace_id
                    .as_ref()
                    .map(ToString::to_string),
                preferences
                    .active_session_id
                    .as_ref()
                    .map(ToString::to_string),
                profile_id,
                model,
                approval,
                max_steps,
                preferences.default_approval_policy.map(approval_to_db),
                now_rfc3339(),
                i64::try_from(expected_revision).map_err(storage_error)?,
            ],
        )
        .map_err(storage_error)?;
    match updated {
        1 => Ok(true),
        0 if preferences_row_exists(transaction)? => Ok(false),
        0 => Err(storage_error("product preferences row is missing")),
        _ => Err(storage_error("product preferences singleton is corrupt")),
    }
}

fn preferences_row_exists(connection: &Connection) -> Result<bool, ProductStoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM product_preferences WHERE singleton = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

const MAX_MIGRATION_PREPARATIONS: usize = 4_096;
const MAX_MIGRATION_RECEIPTS: usize = 4_096;
const MAX_MIGRATION_ISSUES: usize = 4_096;
const MAX_MIGRATION_BINDINGS: usize = 10_000;

#[derive(Debug)]
struct DurableMigrationPreparation {
    request_digest: String,
    preferences_baseline: M1PreferencesBaseline,
}

#[derive(Debug)]
struct PreparedWorkspaceImport {
    source_id: String,
    workspace: Option<ValidatedWorkspace>,
}

#[derive(Debug)]
struct PreparedSessionImport {
    source_id: String,
    source_workspace_id: String,
    title: String,
    created_at: String,
    updated_at: String,
    has_runtime_hint: bool,
    invalid_runtime_hint: bool,
    legacy_has_durable_turn: bool,
}

#[derive(Debug)]
struct PreparedProviderImport {
    source_id: String,
    profile: ValidatedProviderProfile,
    updated_at: String,
}

#[derive(Debug)]
struct PreparedMigrationData {
    workspaces: Vec<PreparedWorkspaceImport>,
    sessions: Vec<PreparedSessionImport>,
    profiles: Vec<PreparedProviderImport>,
    verified_bindings: HashMap<String, Vec<VerifiedM1SessionRunBinding>>,
    issues: Vec<M1MigrationIssue>,
}

fn apply_migration(
    database: &ProductDatabase,
    migration: PreparedM1BrowserMigration,
) -> Result<M1BrowserMigrationResponse, ProductStoreError> {
    validate_migration_envelope(&migration.request, &migration.issues)?;
    if migration.issues.len() > MAX_MIGRATION_ISSUES
        || migration.verified_run_bindings.len() > MAX_MIGRATION_BINDINGS
    {
        return Err(invalid("prepared browser migration exceeds its limits"));
    }
    let digest = m1_browser_migration_digest(&migration.request)
        .map_err(|_| invalid("browser migration request could not be normalized"))?;
    let source_schema_version = migration.request.source_schema_version;
    let idempotency_key = migration.request.idempotency_key.clone();

    {
        let connection = database.connect()?;
        if let Some(response) = replay_receipt(
            &connection,
            source_schema_version,
            &idempotency_key,
            &digest,
        )? {
            return Ok(response);
        }
    }

    let prepared = prepare_migration_data(&migration)?;
    let mut connection = database.connect()?;
    let transaction = immediate_transaction(&mut connection)?;
    if let Some(response) = replay_receipt(
        &transaction,
        source_schema_version,
        &idempotency_key,
        &digest,
    )? {
        transaction.commit().map_err(storage_error)?;
        return Ok(response);
    }
    validate_durable_migration_preparation(&transaction, &migration, &digest)?;
    enforce_table_limit(
        &transaction,
        "product_migration_receipts",
        MAX_MIGRATION_RECEIPTS,
        "browser migration receipt limit reached",
    )?;

    let receipt_id = ProductMigrationReceiptId::new();
    let applied_at = now_rfc3339();
    transaction
        .execute(
            r#"
            INSERT INTO product_migration_receipts(
                receipt_id, source, source_schema_version, idempotency_key,
                request_digest, response_json, applied_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, '{}', ?6)
            "#,
            params![
                receipt_id.to_string(),
                MIGRATION_SOURCE_WEB_M1,
                i64::from(source_schema_version),
                idempotency_key,
                digest,
                applied_at,
            ],
        )
        .map_err(storage_error)?;

    let mut issues = prepared.issues;
    let mut workspace_ids = HashMap::new();
    let mut workspace_mappings = Vec::new();
    for imported in prepared.workspaces {
        let Some(workspace) = imported.workspace else {
            push_issue_unique(
                &mut issues,
                M1MigrationIssue {
                    code: M1MigrationIssueCode::InvalidWorkspace,
                    entity: "workspace".to_string(),
                    source_id: Some(imported.source_id),
                },
            )?;
            continue;
        };
        let workspace_id =
            migrate_workspace(&transaction, &imported.source_id, &workspace, &applied_at)?;
        workspace_ids.insert(imported.source_id.clone(), workspace_id.clone());
        workspace_mappings.push(M1WorkspaceIdMapping {
            source_id: imported.source_id,
            workspace_id,
        });
    }

    let mut session_ids = HashMap::new();
    let mut session_imports = HashMap::new();
    let mut session_mappings = Vec::new();
    for imported in prepared.sessions {
        let source_id = imported.source_id.clone();
        let Some(workspace_id) = workspace_ids.get(&imported.source_workspace_id) else {
            push_issue_unique(
                &mut issues,
                M1MigrationIssue {
                    code: M1MigrationIssueCode::MissingWorkspace,
                    entity: "session".to_string(),
                    source_id: Some(imported.source_id),
                },
            )?;
            continue;
        };
        let session_id = migrate_session(&transaction, &imported, workspace_id, &applied_at)?;
        session_ids.insert(source_id.clone(), session_id.clone());
        session_imports.insert(source_id.clone(), imported);
        session_mappings.push(M1SessionIdMapping {
            source_id,
            product_session_id: session_id,
        });
    }

    let mut profile_ids = HashMap::new();
    let mut profile_mappings = Vec::new();
    for imported in prepared.profiles {
        let profile_id = migrate_provider_profile(&transaction, &imported, &applied_at)?;
        profile_ids.insert(imported.source_id.clone(), profile_id.clone());
        profile_mappings.push(M1ProviderProfileIdMapping {
            source_id: imported.source_id,
            provider_profile_id: profile_id,
        });
    }

    for (source_session_id, bindings) in prepared.verified_bindings {
        let Some(product_session_id) = session_ids.get(&source_session_id) else {
            continue;
        };
        apply_verified_bindings(&transaction, product_session_id, &bindings, &receipt_id)?;
    }

    for (source_session_id, imported) in &session_imports {
        let has_verified = migration
            .verified_run_bindings
            .iter()
            .any(|binding| binding.source_session_id.as_str() == source_session_id.as_str());
        let mut has_runtime_issue = issues.iter().any(|issue| {
            issue.source_id.as_deref() == Some(source_session_id.as_str())
                && matches!(
                    issue.code,
                    M1MigrationIssueCode::InvalidRuntimeHint
                        | M1MigrationIssueCode::AmbiguousRuntimeBinding
                        | M1MigrationIssueCode::RuntimeBindingNotFound
                )
        });
        if (imported.has_runtime_hint || imported.legacy_has_durable_turn)
            && !has_verified
            && !has_runtime_issue
        {
            let code = if imported.invalid_runtime_hint {
                M1MigrationIssueCode::InvalidRuntimeHint
            } else {
                M1MigrationIssueCode::RuntimeBindingNotFound
            };
            push_issue_unique(
                &mut issues,
                M1MigrationIssue {
                    code,
                    entity: "session_runtime_binding".to_string(),
                    source_id: Some(source_session_id.clone()),
                },
            )?;
            has_runtime_issue = true;
        }
        if has_runtime_issue {
            mark_session_needs_attention(
                &transaction,
                session_ids
                    .get(source_session_id)
                    .ok_or_else(|| binding_corrupt("migration session mapping is missing"))?,
            )?;
        }
    }

    if let M1PreferencesBaseline::Revision(expected_revision) = migration.preferences_baseline {
        let current_preferences = get_preferences(&transaction)?;
        let preferences = migration_preferences(
            &migration,
            &workspace_ids,
            &session_ids,
            &profile_ids,
            &transaction,
            current_preferences,
            &mut issues,
        )?;
        validate_preference_references(&transaction, &preferences)?;
        if !write_preferences_at_revision(&transaction, &preferences, expected_revision)? {
            push_issue_unique(
                &mut issues,
                M1MigrationIssue {
                    code: M1MigrationIssueCode::PreferenceWriteConflict,
                    entity: "preferences".to_string(),
                    source_id: None,
                },
            )?;
        }
    }

    if issues.len() > MAX_MIGRATION_ISSUES {
        return Err(invalid("browser migration issue limit reached"));
    }
    ensure_migration_ack_completeness(
        migration
            .request
            .workspaces
            .iter()
            .map(|item| item.source_id.as_str()),
        workspace_mappings
            .iter()
            .map(|item| item.source_id.as_str()),
        "workspace",
        &issues,
    )?;
    ensure_migration_ack_completeness(
        migration
            .request
            .sessions
            .iter()
            .map(|item| item.source_id.as_str()),
        session_mappings.iter().map(|item| item.source_id.as_str()),
        "session",
        &issues,
    )?;
    ensure_migration_ack_completeness(
        migration
            .request
            .provider_profiles
            .iter()
            .map(|item| item.source_id.as_str()),
        profile_mappings.iter().map(|item| item.source_id.as_str()),
        "provider_profile",
        &issues,
    )?;
    persist_receipt_mappings(
        &transaction,
        &receipt_id,
        &workspace_mappings,
        &session_mappings,
        &profile_mappings,
        &issues,
    )?;

    let response = M1BrowserMigrationResponse {
        source_schema_version,
        idempotency_key,
        receipt_id: receipt_id.clone(),
        disposition: M1MigrationDisposition::Applied,
        workspace_mappings,
        session_mappings,
        provider_profile_mappings: profile_mappings,
        issues,
        applied_at,
    };
    let response_json = serde_json::to_string(&response).map_err(storage_error)?;
    let updated = transaction
        .execute(
            "UPDATE product_migration_receipts SET response_json = ?2 WHERE receipt_id = ?1",
            params![receipt_id.to_string(), response_json],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(storage_error("migration receipt disappeared before commit"));
    }
    let deleted = transaction
        .execute(
            r#"
            DELETE FROM product_migration_preparations
            WHERE source = ?1 AND source_schema_version = ?2 AND idempotency_key = ?3
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                i64::from(source_schema_version),
                migration.request.idempotency_key,
            ],
        )
        .map_err(storage_error)?;
    if deleted != 1 {
        return Err(storage_error(
            "migration preparation disappeared before commit",
        ));
    }
    transaction.commit().map_err(storage_error)?;
    Ok(response)
}

fn replay_receipt(
    connection: &Connection,
    source_schema_version: u32,
    idempotency_key: &str,
    digest: &str,
) -> Result<Option<M1BrowserMigrationResponse>, ProductStoreError> {
    let receipt = connection
        .query_row(
            r#"
            SELECT request_digest, response_json
            FROM product_migration_receipts
            WHERE source = ?1 AND source_schema_version = ?2 AND idempotency_key = ?3
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                i64::from(source_schema_version),
                idempotency_key,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((stored_digest, response_json)) = receipt else {
        return Ok(None);
    };
    if stored_digest != digest {
        return Err(ProductStoreError::new(
            ProductErrorCode::MigrationIdempotencyConflict,
            "migration idempotency key was already used for a different payload",
        ));
    }
    let mut response = serde_json::from_str::<M1BrowserMigrationResponse>(&response_json)
        .map_err(storage_error)?;
    response.disposition = M1MigrationDisposition::AlreadyApplied;
    Ok(Some(response))
}

fn migration_preparation(
    connection: &Connection,
    source_schema_version: u32,
    idempotency_key: &str,
) -> Result<Option<DurableMigrationPreparation>, ProductStoreError> {
    let preparation = connection
        .query_row(
            r#"
            SELECT request_digest, preferences_requested, preferences_revision
            FROM product_migration_preparations
            WHERE source = ?1 AND source_schema_version = ?2 AND idempotency_key = ?3
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                i64::from(source_schema_version),
                idempotency_key,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((request_digest, preferences_requested, preferences_revision)) = preparation else {
        return Ok(None);
    };
    let preferences_baseline = baseline_from_db(preferences_requested, preferences_revision)?;
    Ok(Some(DurableMigrationPreparation {
        request_digest,
        preferences_baseline,
    }))
}

fn validate_durable_migration_preparation(
    transaction: &Transaction<'_>,
    migration: &PreparedM1BrowserMigration,
    digest: &str,
) -> Result<(), ProductStoreError> {
    validate_preparation_baseline(&migration.request, migration.preferences_baseline)?;
    let preparation = migration_preparation(
        transaction,
        migration.request.source_schema_version,
        &migration.request.idempotency_key,
    )?
    .ok_or_else(|| storage_error("prepared browser migration preflight is missing"))?;
    if preparation.request_digest != digest {
        return Err(migration_idempotency_conflict(
            "migration idempotency key is already preparing a different payload",
        ));
    }
    if preparation.preferences_baseline != migration.preferences_baseline {
        return Err(storage_error(
            "prepared browser migration preferences baseline does not match durable preflight",
        ));
    }
    Ok(())
}

fn migration_idempotency_conflict(message: &'static str) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::MigrationIdempotencyConflict, message)
}

fn prepare_migration_data(
    migration: &PreparedM1BrowserMigration,
) -> Result<PreparedMigrationData, ProductStoreError> {
    let mut issues = migration.issues.clone();
    let mut workspaces = Vec::with_capacity(migration.request.workspaces.len());
    for imported in &migration.request.workspaces {
        let workspace = validate_workspace(
            &imported.root,
            imported.kind,
            Some(&imported.display_name),
            imported.pinned,
            &imported.last_opened_at,
        )
        .ok();
        workspaces.push(PreparedWorkspaceImport {
            source_id: validate_source_id("workspace source_id", &imported.source_id)?,
            workspace,
        });
    }

    let mut sessions = Vec::with_capacity(migration.request.sessions.len());
    for imported in &migration.request.sessions {
        let has_job = imported.legacy_active_job_id.is_some();
        let has_run = imported.legacy_active_run_id.is_some();
        let has_resumed = imported.legacy_resumed_from_run_id.is_some();
        sessions.push(PreparedSessionImport {
            source_id: validate_source_id("session source_id", &imported.source_id)?,
            source_workspace_id: validate_source_id(
                "session source_workspace_id",
                &imported.source_workspace_id,
            )?,
            title: validate_title(Some(&imported.title))?,
            created_at: normalized_timestamp("session created_at", &imported.created_at)?,
            updated_at: normalized_timestamp("session updated_at", &imported.updated_at)?,
            has_runtime_hint: has_job || has_run || has_resumed,
            invalid_runtime_hint: has_job != has_run || (has_resumed && !has_run),
            legacy_has_durable_turn: imported.legacy_has_durable_turn,
        });
    }

    let mut profiles = Vec::with_capacity(migration.request.provider_profiles.len());
    for imported in &migration.request.provider_profiles {
        profiles.push(PreparedProviderImport {
            source_id: validate_source_id("provider profile source_id", &imported.source_id)?,
            profile: validate_migration_provider(
                &imported.label,
                imported.provider_type,
                &imported.api_base,
                imported.api_key_env.as_deref(),
                imported.default_model.as_deref(),
            )?,
            updated_at: normalized_timestamp("provider profile updated_at", &imported.updated_at)?,
        });
    }

    let request_session_ids = sessions
        .iter()
        .map(|session| session.source_id.as_str())
        .collect::<HashSet<_>>();
    let workspace_seals = workspaces
        .iter()
        .filter_map(|imported| {
            imported.workspace.as_ref().map(|workspace| {
                (
                    imported.source_id.as_str(),
                    (
                        PathBuf::from(&workspace.canonical_root_text),
                        workspace.kind,
                    ),
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let session_workspace_sources = sessions
        .iter()
        .map(|session| {
            (
                session.source_id.as_str(),
                session.source_workspace_id.as_str(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut verified_bindings: HashMap<String, Vec<VerifiedM1SessionRunBinding>> = HashMap::new();
    let mut runtime_runs = HashSet::new();
    for binding in &migration.verified_run_bindings {
        let source_session_id =
            validate_source_id("verified source session id", &binding.source_session_id)?;
        if !request_session_ids.contains(source_session_id.as_str()) {
            return Err(invalid(
                "verified runtime binding references an unknown source session",
            ));
        }
        let source_workspace_id = session_workspace_sources
            .get(source_session_id.as_str())
            .ok_or_else(|| binding_corrupt("verified migration session has no workspace"))?;
        let (canonical_root, kind) = workspace_seals
            .get(*source_workspace_id)
            .ok_or_else(|| binding_corrupt("verified migration workspace is no longer valid"))?;
        if canonical_root != &binding.verified_workspace_root
            || kind != &binding.verified_workspace_kind
        {
            return Err(binding_corrupt(
                "verified migration workspace changed before apply",
            ));
        }
        if !runtime_runs.insert(binding.runtime_run_id) {
            return Err(binding_corrupt(
                "prepared migration repeats a runtime run binding",
            ));
        }
        verified_bindings
            .entry(source_session_id)
            .or_default()
            .push(binding.clone());
    }
    for bindings in verified_bindings.values_mut() {
        bindings.sort_by_key(|binding| binding.ordinal);
        validate_verified_binding_chain(bindings)?;
    }

    for issue in &mut issues {
        issue.entity = validate_issue_entity(&issue.entity)?;
        issue.source_id = issue
            .source_id
            .as_deref()
            .map(|value| validate_source_id("migration issue source_id", value))
            .transpose()?;
    }
    Ok(PreparedMigrationData {
        workspaces,
        sessions,
        profiles,
        verified_bindings,
        issues,
    })
}

fn validate_verified_binding_chain(
    bindings: &[VerifiedM1SessionRunBinding],
) -> Result<(), ProductStoreError> {
    let Some(first) = bindings.first() else {
        return Ok(());
    };
    if bindings.len() > usize::try_from(MAX_RUN_BINDINGS_PER_SESSION).map_err(storage_error)? {
        return Err(invalid("migration run binding limit reached"));
    }
    if first.resumed_from_run_id.is_some() {
        return Err(binding_corrupt(
            "first verified migration binding cannot resume another run",
        ));
    }
    for (index, binding) in bindings.iter().enumerate() {
        let expected = u64::try_from(index).map_err(storage_error)? + 1;
        if binding.ordinal != expected {
            return Err(binding_corrupt(
                "verified migration run ordinals must be contiguous and start at one",
            ));
        }
        if binding.runtime_session_id != first.runtime_session_id
            || binding.runtime_job_id != first.runtime_job_id
        {
            return Err(binding_corrupt(
                "verified migration bindings do not share one runtime session and job",
            ));
        }
        if index > 0 && binding.resumed_from_run_id != Some(bindings[index - 1].runtime_run_id) {
            return Err(binding_corrupt(
                "verified migration run chain is not contiguous",
            ));
        }
    }
    Ok(())
}

fn migrate_workspace(
    transaction: &Transaction<'_>,
    source_id: &str,
    workspace: &ValidatedWorkspace,
    now: &str,
) -> Result<ProductWorkspaceId, ProductStoreError> {
    if let Some(mapped_id) = source_workspace_mapping(transaction, source_id)? {
        let (stored_key, stored_kind): (String, String) = transaction
            .query_row(
                "SELECT canonical_key, kind FROM product_workspaces WHERE workspace_id = ?1",
                params![mapped_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| binding_corrupt("workspace source mapping is dangling"))?;
        if stored_key != workspace.canonical_key {
            return Err(ProductStoreError::new(
                ProductErrorCode::MigrationIdempotencyConflict,
                "workspace source_id resolves to a different canonical root",
            ));
        }
        if workspace_kind_from_db(&stored_kind)? != workspace.kind {
            return Err(ProductStoreError::new(
                ProductErrorCode::MigrationIdempotencyConflict,
                "workspace source_id resolves to a different workspace kind",
            ));
        }
        update_migrated_workspace(transaction, &mapped_id, workspace, now)?;
        touch_workspace_source(transaction, source_id, &mapped_id, now)?;
        return Ok(mapped_id);
    }

    let workspace_id =
        if let Some(existing) = find_workspace_by_key(transaction, &workspace.canonical_key)? {
            if existing.kind != workspace.kind {
                return Err(ProductStoreError::new(
                    ProductErrorCode::MigrationIdempotencyConflict,
                    "canonical workspace root is already registered with a different kind",
                ));
            }
            update_migrated_workspace(transaction, &existing.id, workspace, now)?;
            existing.id
        } else {
            enforce_table_limit(
                transaction,
                "product_workspaces",
                MAX_PRODUCT_WORKSPACES,
                "workspace limit reached",
            )?;
            let workspace_id = ProductWorkspaceId::new();
            transaction
                .execute(
                    r#"
                INSERT INTO product_workspaces(
                    workspace_id, canonical_root, canonical_key, kind, display_name,
                    pinned, last_opened_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                    params![
                        workspace_id.to_string(),
                        workspace.canonical_root_text,
                        workspace.canonical_key,
                        workspace_kind_to_db(workspace.kind),
                        workspace.display_name,
                        bool_to_i64(workspace.pinned),
                        workspace.last_opened_at,
                        now,
                    ],
                )
                .map_err(storage_error)?;
            workspace_id
        };
    enforce_source_mapping_limit(
        transaction,
        "product_migration_workspace_sources",
        MAX_PRODUCT_WORKSPACES.saturating_mul(4),
    )?;
    transaction
        .execute(
            r#"
            INSERT INTO product_migration_workspace_sources(
                source, source_id, workspace_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?4)
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                source_id,
                workspace_id.to_string(),
                now,
            ],
        )
        .map_err(storage_error)?;
    Ok(workspace_id)
}

fn update_migrated_workspace(
    transaction: &Transaction<'_>,
    workspace_id: &ProductWorkspaceId,
    workspace: &ValidatedWorkspace,
    now: &str,
) -> Result<(), ProductStoreError> {
    transaction
        .execute(
            r#"
            UPDATE product_workspaces
            SET canonical_root = ?2,
                display_name = CASE WHEN last_opened_at <= ?5 THEN ?3 ELSE display_name END,
                pinned = MAX(pinned, ?4), last_opened_at = MAX(last_opened_at, ?5),
                updated_at = ?6
            WHERE workspace_id = ?1
            "#,
            params![
                workspace_id.to_string(),
                workspace.canonical_root_text,
                workspace.display_name,
                bool_to_i64(workspace.pinned),
                workspace.last_opened_at,
                now,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn source_workspace_mapping(
    connection: &Connection,
    source_id: &str,
) -> Result<Option<ProductWorkspaceId>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT workspace_id FROM product_migration_workspace_sources
            WHERE source = ?1 AND source_id = ?2
            "#,
            params![MIGRATION_SOURCE_WEB_M1, source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|value| parse_product_id(&value, "workspace id"))
        .transpose()
}

fn touch_workspace_source(
    transaction: &Transaction<'_>,
    source_id: &str,
    workspace_id: &ProductWorkspaceId,
    now: &str,
) -> Result<(), ProductStoreError> {
    let updated = transaction
        .execute(
            r#"
            UPDATE product_migration_workspace_sources
            SET updated_at = ?4
            WHERE source = ?1 AND source_id = ?2 AND workspace_id = ?3
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                source_id,
                workspace_id.to_string(),
                now,
            ],
        )
        .map_err(storage_error)?;
    if updated != 1 {
        return Err(binding_corrupt(
            "workspace source mapping changed unexpectedly",
        ));
    }
    Ok(())
}

fn migrate_session(
    transaction: &Transaction<'_>,
    imported: &PreparedSessionImport,
    workspace_id: &ProductWorkspaceId,
    now: &str,
) -> Result<ProductSessionId, ProductStoreError> {
    if let Some(session_id) = source_session_mapping(transaction, &imported.source_id)? {
        let session = get_session(transaction, &session_id)?;
        if session.workspace_id != *workspace_id {
            return Err(ProductStoreError::new(
                ProductErrorCode::MigrationIdempotencyConflict,
                "session source_id resolves to a different workspace",
            ));
        }
        if has_active_claim_for_session(transaction, &session_id)? {
            return Err(session_active(
                "a source-mapped product session has an active turn",
            ));
        }
        transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET title = CASE WHEN updated_at <= ?3 THEN ?2 ELSE title END,
                    updated_at = MAX(updated_at, ?3)
                WHERE product_session_id = ?1
                "#,
                params![session_id.to_string(), imported.title, imported.updated_at],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                r#"
                UPDATE product_migration_session_sources SET updated_at = ?4
                WHERE source = ?1 AND source_id = ?2 AND product_session_id = ?3
                "#,
                params![
                    MIGRATION_SOURCE_WEB_M1,
                    imported.source_id,
                    session_id.to_string(),
                    now,
                ],
            )
            .map_err(storage_error)?;
        return Ok(session_id);
    }

    enforce_table_limit(
        transaction,
        "product_sessions",
        MAX_PRODUCT_SESSIONS,
        "product session limit reached",
    )?;
    let session_id = ProductSessionId::new();
    transaction
        .execute(
            r#"
            INSERT INTO product_sessions(
                product_session_id, workspace_id, title, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'idle', ?4, ?5)
            "#,
            params![
                session_id.to_string(),
                workspace_id.to_string(),
                imported.title,
                imported.created_at,
                imported.updated_at,
            ],
        )
        .map_err(storage_error)?;
    enforce_source_mapping_limit(
        transaction,
        "product_migration_session_sources",
        MAX_PRODUCT_SESSIONS.saturating_mul(4),
    )?;
    transaction
        .execute(
            r#"
            INSERT INTO product_migration_session_sources(
                source, source_id, product_session_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?4)
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                imported.source_id,
                session_id.to_string(),
                now,
            ],
        )
        .map_err(storage_error)?;
    Ok(session_id)
}

fn source_session_mapping(
    connection: &Connection,
    source_id: &str,
) -> Result<Option<ProductSessionId>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT product_session_id FROM product_migration_session_sources
            WHERE source = ?1 AND source_id = ?2
            "#,
            params![MIGRATION_SOURCE_WEB_M1, source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|value| parse_product_id(&value, "product session id"))
        .transpose()
}

fn migrate_provider_profile(
    transaction: &Transaction<'_>,
    imported: &PreparedProviderImport,
    now: &str,
) -> Result<ProductProviderProfileId, ProductStoreError> {
    if let Some(profile_id) = source_profile_mapping(transaction, &imported.source_id)? {
        let existing = get_provider_profile(transaction, &profile_id)?;
        if existing.updated_at.as_str() <= imported.updated_at.as_str() {
            update_provider_profile_row(
                transaction,
                &profile_id,
                &imported.profile,
                &imported.updated_at,
            )?;
        }
        transaction
            .execute(
                r#"
                UPDATE product_migration_profile_sources SET updated_at = ?4
                WHERE source = ?1 AND source_id = ?2 AND profile_id = ?3
                "#,
                params![
                    MIGRATION_SOURCE_WEB_M1,
                    imported.source_id,
                    profile_id.to_string(),
                    now,
                ],
            )
            .map_err(storage_error)?;
        return Ok(profile_id);
    }

    enforce_table_limit(
        transaction,
        "product_provider_profiles",
        MAX_PRODUCT_PROVIDER_PROFILES,
        "provider profile limit reached",
    )?;
    let profile_id = ProductProviderProfileId::new();
    insert_provider_profile(
        transaction,
        &profile_id,
        &imported.profile,
        &imported.updated_at,
        &imported.updated_at,
    )?;
    enforce_source_mapping_limit(
        transaction,
        "product_migration_profile_sources",
        MAX_PRODUCT_PROVIDER_PROFILES.saturating_mul(4),
    )?;
    transaction
        .execute(
            r#"
            INSERT INTO product_migration_profile_sources(
                source, source_id, profile_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?4)
            "#,
            params![
                MIGRATION_SOURCE_WEB_M1,
                imported.source_id,
                profile_id.to_string(),
                now,
            ],
        )
        .map_err(storage_error)?;
    Ok(profile_id)
}

fn source_profile_mapping(
    connection: &Connection,
    source_id: &str,
) -> Result<Option<ProductProviderProfileId>, ProductStoreError> {
    connection
        .query_row(
            r#"
            SELECT profile_id FROM product_migration_profile_sources
            WHERE source = ?1 AND source_id = ?2
            "#,
            params![MIGRATION_SOURCE_WEB_M1, source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|value| parse_product_id(&value, "provider profile id"))
        .transpose()
}

fn apply_verified_bindings(
    transaction: &Transaction<'_>,
    product_session_id: &ProductSessionId,
    bindings: &[VerifiedM1SessionRunBinding],
    receipt_id: &ProductMigrationReceiptId,
) -> Result<(), ProductStoreError> {
    let mut session = get_session(transaction, product_session_id)?;
    let existing = list_and_validate_bindings(transaction, &session)?;
    for verified in bindings {
        if let Some(current) = existing
            .iter()
            .find(|item| item.ordinal == verified.ordinal)
        {
            if current.runtime_session_id != verified.runtime_session_id
                || current.runtime_job_id != verified.runtime_job_id
                || current.runtime_run_id != verified.runtime_run_id
                || current.resumed_from_run_id != verified.resumed_from_run_id
            {
                return Err(binding_corrupt(
                    "verified migration binding conflicts with an immutable product run binding",
                ));
            }
            continue;
        }
        let expected = next_binding_ordinal(&session)?;
        if verified.ordinal != expected {
            return Err(binding_corrupt(
                "verified migration binding would create an ordinal gap",
            ));
        }
        if let Some(other) = find_binding_by_runtime_run(transaction, verified.runtime_run_id)?
            && (other.product_session_id != *product_session_id
                || other.ordinal != verified.ordinal)
        {
            return Err(binding_corrupt(
                "verified runtime run is already bound to another product session",
            ));
        }
        claim_runtime_ownership(
            transaction,
            product_session_id,
            verified.runtime_session_id,
            verified.runtime_job_id,
            ProductErrorCode::ProductBindingCorrupt,
        )?;
        let inserted = insert_run_binding(
            transaction,
            NewRunBinding {
                product_session_id: product_session_id.clone(),
                ordinal: verified.ordinal,
                runtime_session_id: verified.runtime_session_id,
                runtime_job_id: verified.runtime_job_id,
                runtime_run_id: verified.runtime_run_id,
                resumed_from_run_id: verified.resumed_from_run_id,
                migration_receipt_id: Some(receipt_id.clone()),
            },
        )?;
        update_latest_binding_cas(transaction, &session, &inserted)?;
        session.runtime_binding = Some(ProductRuntimeBinding {
            ordinal: inserted.ordinal,
            runtime_session_id: inserted.runtime_session_id,
            latest_job_id: inserted.runtime_job_id,
            latest_run_id: inserted.runtime_run_id,
        });
    }
    validate_binding_integrity(transaction, &get_session(transaction, product_session_id)?)
}

fn mark_session_needs_attention(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
) -> Result<(), ProductStoreError> {
    transaction
        .execute(
            r#"
            UPDATE product_sessions
            SET status = 'needs_attention', updated_at = ?2
            WHERE product_session_id = ?1 AND status != 'archived'
            "#,
            params![session_id.to_string(), now_rfc3339()],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn migration_preferences(
    migration: &PreparedM1BrowserMigration,
    workspace_ids: &HashMap<String, ProductWorkspaceId>,
    session_ids: &HashMap<String, ProductSessionId>,
    profile_ids: &HashMap<String, ProductProviderProfileId>,
    transaction: &Transaction<'_>,
    existing: ProductPreferences,
    issues: &mut Vec<M1MigrationIssue>,
) -> Result<ValidatedPreferences, ProductStoreError> {
    let imported = &migration.request.safe_preferences;
    let mut active_workspace_id = existing.active_workspace_id;
    let mut active_session_id = existing.active_session_id;
    let mut imported_workspace_valid = None;

    if let Some(source_id) = imported.source_active_workspace_id.as_ref() {
        if let Some(workspace_id) = workspace_ids.get(source_id).cloned() {
            if active_workspace_id.as_ref() != Some(&workspace_id) {
                active_session_id = None;
            }
            active_workspace_id = Some(workspace_id);
            imported_workspace_valid = Some(true);
        } else {
            imported_workspace_valid = Some(false);
            push_issue_unique(
                issues,
                M1MigrationIssue {
                    code: M1MigrationIssueCode::InvalidPreferenceReference,
                    entity: "active_workspace".to_string(),
                    source_id: Some(source_id.clone()),
                },
            )?;
        }
    }

    if let Some(source_id) = imported.source_active_session_id.as_ref() {
        let mapped_session = session_ids.get(source_id).cloned();
        let valid_session = match (mapped_session, imported_workspace_valid) {
            (Some(session_id), Some(true)) => {
                let session = get_session(transaction, &session_id)?;
                (Some(&session.workspace_id) == active_workspace_id.as_ref()).then_some(session)
            }
            (Some(session_id), None) => Some(get_session(transaction, &session_id)?),
            _ => None,
        };
        if let Some(session) = valid_session {
            active_workspace_id = Some(session.workspace_id);
            active_session_id = Some(session.id);
        } else {
            push_issue_unique(
                issues,
                M1MigrationIssue {
                    code: M1MigrationIssueCode::InvalidPreferenceReference,
                    entity: "active_session".to_string(),
                    source_id: Some(source_id.clone()),
                },
            )?;
        }
    }

    let provider_selection = match imported.provider_selection.as_ref() {
        None => existing.provider_selection,
        Some(selection) => {
            let profile_id = selection
                .source_profile_id
                .as_ref()
                .and_then(|source_id| profile_ids.get(source_id))
                .cloned();
            if let Some(source_id) = selection.source_profile_id.as_ref()
                && profile_id.is_none()
            {
                push_issue_unique(
                    issues,
                    M1MigrationIssue {
                        code: M1MigrationIssueCode::InvalidPreferenceReference,
                        entity: "provider_selection".to_string(),
                        source_id: Some(source_id.clone()),
                    },
                )?;
                existing.provider_selection
            } else {
                Some(validate_provider_selection(ProductProviderSelection {
                    profile_id,
                    model: selection.model.clone(),
                    approval: selection.approval,
                    max_steps: selection.max_steps,
                })?)
            }
        }
    };

    Ok(ValidatedPreferences {
        schema_version: 1,
        expected_revision: None,
        theme: imported.theme.unwrap_or(existing.theme),
        default_approval_policy: Some(existing.default_approval_policy),
        active_workspace_id,
        active_session_id,
        provider_selection,
    })
}

fn migration_requests_preferences(request: &crate::product::M1BrowserMigrationRequest) -> bool {
    let preferences = &request.safe_preferences;
    preferences.theme.is_some()
        || preferences.source_active_workspace_id.is_some()
        || preferences.source_active_session_id.is_some()
        || preferences.provider_selection.is_some()
}

fn validate_preparation_baseline(
    request: &crate::product::M1BrowserMigrationRequest,
    baseline: M1PreferencesBaseline,
) -> Result<(), ProductStoreError> {
    let consistent = matches!(
        (migration_requests_preferences(request), baseline),
        (false, M1PreferencesBaseline::NotRequested) | (true, M1PreferencesBaseline::Revision(_))
    );
    if consistent {
        Ok(())
    } else {
        Err(storage_error(
            "prepared browser migration preferences baseline is inconsistent",
        ))
    }
}

fn baseline_to_db(
    baseline: M1PreferencesBaseline,
) -> Result<(i64, Option<i64>), ProductStoreError> {
    match baseline {
        M1PreferencesBaseline::NotRequested => Ok((0, None)),
        M1PreferencesBaseline::Revision(revision) => {
            Ok((1, Some(i64::try_from(revision).map_err(storage_error)?)))
        }
    }
}

fn baseline_from_db(
    preferences_requested: i64,
    preferences_revision: Option<i64>,
) -> Result<M1PreferencesBaseline, ProductStoreError> {
    match (preferences_requested, preferences_revision) {
        (0, None) => Ok(M1PreferencesBaseline::NotRequested),
        (1, Some(revision)) => Ok(M1PreferencesBaseline::Revision(
            u64::try_from(revision).map_err(storage_error)?,
        )),
        _ => Err(storage_error(
            "persisted browser migration preferences baseline is invalid",
        )),
    }
}

fn persist_receipt_mappings(
    transaction: &Transaction<'_>,
    receipt_id: &ProductMigrationReceiptId,
    workspaces: &[M1WorkspaceIdMapping],
    sessions: &[M1SessionIdMapping],
    profiles: &[M1ProviderProfileIdMapping],
    issues: &[M1MigrationIssue],
) -> Result<(), ProductStoreError> {
    for (ordinal, mapping) in workspaces.iter().enumerate() {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_receipt_workspace_mappings(
                    receipt_id, ordinal, source_id, workspace_id
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    receipt_id.to_string(),
                    i64::try_from(ordinal).map_err(storage_error)?,
                    mapping.source_id,
                    mapping.workspace_id.to_string(),
                ],
            )
            .map_err(storage_error)?;
    }
    for (ordinal, mapping) in sessions.iter().enumerate() {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_receipt_session_mappings(
                    receipt_id, ordinal, source_id, product_session_id
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    receipt_id.to_string(),
                    i64::try_from(ordinal).map_err(storage_error)?,
                    mapping.source_id,
                    mapping.product_session_id.to_string(),
                ],
            )
            .map_err(storage_error)?;
    }
    for (ordinal, mapping) in profiles.iter().enumerate() {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_receipt_profile_mappings(
                    receipt_id, ordinal, source_id, profile_id
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    receipt_id.to_string(),
                    i64::try_from(ordinal).map_err(storage_error)?,
                    mapping.source_id,
                    mapping.provider_profile_id.to_string(),
                ],
            )
            .map_err(storage_error)?;
    }
    for (ordinal, issue) in issues.iter().enumerate() {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_receipt_issues(
                    receipt_id, ordinal, code, entity, source_id
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    receipt_id.to_string(),
                    i64::try_from(ordinal).map_err(storage_error)?,
                    migration_issue_code_to_db(issue.code),
                    issue.entity,
                    issue.source_id,
                ],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn push_issue_unique(
    issues: &mut Vec<M1MigrationIssue>,
    mut issue: M1MigrationIssue,
) -> Result<(), ProductStoreError> {
    issue.entity = validate_issue_entity(&issue.entity)?;
    issue.source_id = issue
        .source_id
        .as_deref()
        .map(|value| validate_source_id("migration issue source_id", value))
        .transpose()?;
    let duplicate = issues.iter().any(|existing| {
        existing.code == issue.code
            && existing.entity == issue.entity
            && existing.source_id == issue.source_id
    });
    if !duplicate {
        if issues.len() >= MAX_MIGRATION_ISSUES {
            return Err(invalid("browser migration issue limit reached"));
        }
        issues.push(issue);
    }
    Ok(())
}

fn ensure_migration_ack_completeness<'a>(
    input_source_ids: impl Iterator<Item = &'a str>,
    mapped_source_ids: impl Iterator<Item = &'a str>,
    entity: &'static str,
    issues: &[M1MigrationIssue],
) -> Result<(), ProductStoreError> {
    let mapped = mapped_source_ids.collect::<HashSet<_>>();
    for source_id in input_source_ids {
        let covered_by_issue = issues
            .iter()
            .any(|issue| issue.entity == entity && issue.source_id.as_deref() == Some(source_id));
        if !mapped.contains(source_id) && !covered_by_issue {
            return Err(storage_error(
                "browser migration acknowledgement is incomplete",
            ));
        }
    }
    Ok(())
}

fn enforce_source_mapping_limit(
    connection: &Connection,
    table: &str,
    maximum: usize,
) -> Result<(), ProductStoreError> {
    enforce_table_limit(
        connection,
        table,
        maximum,
        "browser migration source mapping limit reached",
    )
}

fn enforce_table_limit(
    connection: &Connection,
    table: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), ProductStoreError> {
    let sql = match table {
        "product_workspaces" => "SELECT COUNT(*) FROM product_workspaces",
        "product_sessions" => "SELECT COUNT(*) FROM product_sessions",
        "product_provider_profiles" => "SELECT COUNT(*) FROM product_provider_profiles",
        "product_migration_preparations" => "SELECT COUNT(*) FROM product_migration_preparations",
        "product_migration_receipts" => "SELECT COUNT(*) FROM product_migration_receipts",
        "product_migration_workspace_sources" => {
            "SELECT COUNT(*) FROM product_migration_workspace_sources"
        }
        "product_migration_session_sources" => {
            "SELECT COUNT(*) FROM product_migration_session_sources"
        }
        "product_migration_profile_sources" => {
            "SELECT COUNT(*) FROM product_migration_profile_sources"
        }
        _ => return Err(storage_error("unknown bounded product table")),
    };
    let count: i64 = connection
        .query_row(sql, [], |row| row.get(0))
        .map_err(storage_error)?;
    let maximum = i64::try_from(maximum).map_err(storage_error)?;
    if count >= maximum {
        return Err(invalid(message));
    }
    Ok(())
}

fn immediate_transaction(
    connection: &mut Connection,
) -> Result<Transaction<'_>, ProductStoreError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)
}

fn parse_product_id<T>(value: &str, field: &'static str) -> Result<T, ProductStoreError>
where
    T: FromStr<Err = String>,
{
    value
        .parse::<T>()
        .map_err(|_| binding_corrupt(format!("persisted {field} is invalid")))
}

fn parse_runtime_id<T>(value: &str, field: &'static str) -> Result<T, ProductStoreError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| binding_corrupt(format!("persisted {field} is invalid")))
}

fn get_review_in_transaction(
    connection: &Connection,
    review_id: &ProductReviewId,
) -> Result<ProductReview, ProductStoreError> {
    let row = connection
        .query_row(
            r#"
            SELECT review_id, product_session_id, workspace_id,
                   target_summary_json, status, conclusion,
                   runtime_session_id, job_id, run_id, result_json,
                   findings_count, unchecked_count, warnings_count,
                   created_at, updated_at, captured_at, finalized_at
            FROM product_reviews WHERE review_id = ?1
            "#,
            params![review_id.to_string()],
            |row| {
                Ok(RawReview {
                    id: row.get(0)?,
                    product_session_id: row.get(1)?,
                    workspace_id: row.get(2)?,
                    target_summary_json: row.get(3)?,
                    status: row.get(4)?,
                    conclusion: row.get(5)?,
                    runtime_session_id: row.get(6)?,
                    job_id: row.get(7)?,
                    run_id: row.get(8)?,
                    result_json: row.get(9)?,
                    findings_count: row.get(10)?,
                    unchecked_count: row.get(11)?,
                    warnings_count: row.get(12)?,
                    created_at: row.get(13)?,
                    updated_at: row.get(14)?,
                    captured_at: row.get(15)?,
                    finalized_at: row.get(16)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| not_found("review was not found"))?;
    row.into_product()
}

#[derive(Debug)]
struct RawReview {
    id: String,
    product_session_id: String,
    workspace_id: String,
    target_summary_json: String,
    status: String,
    conclusion: Option<String>,
    runtime_session_id: Option<String>,
    job_id: Option<String>,
    run_id: Option<String>,
    result_json: Option<String>,
    findings_count: i64,
    unchecked_count: i64,
    warnings_count: i64,
    created_at: String,
    updated_at: String,
    captured_at: String,
    finalized_at: Option<String>,
}

impl RawReview {
    fn into_product(self) -> Result<ProductReview, ProductStoreError> {
        let target: ReviewTargetSummary = serde_json::from_str(&self.target_summary_json)
            .map_err(|_| binding_corrupt("persisted review target summary is invalid"))?;
        let result = self
            .result_json
            .as_deref()
            .map(serde_json::from_str::<ReviewResult>)
            .transpose()
            .map_err(|_| binding_corrupt("persisted review result is invalid"))?;
        let parse_count = |value: i64, field: &'static str| {
            usize::try_from(value)
                .map_err(|_| binding_corrupt(format!("persisted {field} is invalid")))
        };
        Ok(ProductReview {
            id: parse_product_id(&self.id, "review id")?,
            product_session_id: parse_product_id(&self.product_session_id, "review session id")?,
            workspace_id: parse_product_id(&self.workspace_id, "review workspace id")?,
            target,
            status: review_status_from_db(&self.status)?,
            conclusion: self
                .conclusion
                .as_deref()
                .map(review_conclusion_from_db)
                .transpose()?,
            runtime_session_id: self
                .runtime_session_id
                .as_deref()
                .map(|value| parse_runtime_id(value, "review runtime session id"))
                .transpose()?,
            job_id: self
                .job_id
                .as_deref()
                .map(|value| parse_runtime_id(value, "review job id"))
                .transpose()?,
            run_id: self
                .run_id
                .as_deref()
                .map(|value| parse_runtime_id(value, "review run id"))
                .transpose()?,
            result,
            findings_count: parse_count(self.findings_count, "review findings count")?,
            unchecked_count: parse_count(self.unchecked_count, "review unchecked count")?,
            warnings_count: parse_count(self.warnings_count, "review warnings count")?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            captured_at: self.captured_at,
            finalized_at: self.finalized_at,
        })
    }
}

fn validate_review_idempotency_key(value: &str) -> Result<(), ProductStoreError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(invalid("review idempotency_key must be 1..128 characters"));
    }
    Ok(())
}

fn review_target_kind_to_db(kind: ReviewTargetKind) -> &'static str {
    match kind {
        ReviewTargetKind::Uncommitted => "uncommitted",
        ReviewTargetKind::Base => "base",
        ReviewTargetKind::Commit => "commit",
    }
}

fn review_status_from_db(value: &str) -> Result<ProductReviewStatus, ProductStoreError> {
    match value {
        "queued" => Ok(ProductReviewStatus::Queued),
        "running" => Ok(ProductReviewStatus::Running),
        "pass" => Ok(ProductReviewStatus::Pass),
        "findings" => Ok(ProductReviewStatus::Findings),
        "partial" => Ok(ProductReviewStatus::Partial),
        "stale" => Ok(ProductReviewStatus::Stale),
        "needs_attention" => Ok(ProductReviewStatus::NeedsAttention),
        "unavailable" => Ok(ProductReviewStatus::Unavailable),
        "cancelled" => Ok(ProductReviewStatus::Cancelled),
        "error" => Ok(ProductReviewStatus::Error),
        _ => Err(binding_corrupt("persisted review status is invalid")),
    }
}

fn review_status_from_conclusion(conclusion: &ReviewConclusion) -> ProductReviewStatus {
    match conclusion {
        ReviewConclusion::Pass => ProductReviewStatus::Pass,
        ReviewConclusion::Findings => ProductReviewStatus::Findings,
        ReviewConclusion::Partial => ProductReviewStatus::Partial,
        ReviewConclusion::Stale => ProductReviewStatus::Stale,
        ReviewConclusion::Unavailable => ProductReviewStatus::Unavailable,
        ReviewConclusion::Cancelled => ProductReviewStatus::Cancelled,
        ReviewConclusion::Error => ProductReviewStatus::Error,
    }
}

fn review_conclusion_to_db(conclusion: &ReviewConclusion) -> &'static str {
    match conclusion {
        ReviewConclusion::Pass => "pass",
        ReviewConclusion::Findings => "findings",
        ReviewConclusion::Partial => "partial",
        ReviewConclusion::Stale => "stale",
        ReviewConclusion::Unavailable => "unavailable",
        ReviewConclusion::Cancelled => "cancelled",
        ReviewConclusion::Error => "error",
    }
}

fn review_conclusion_from_db(value: &str) -> Result<ReviewConclusion, ProductStoreError> {
    match value {
        "pass" => Ok(ReviewConclusion::Pass),
        "findings" => Ok(ReviewConclusion::Findings),
        "partial" => Ok(ReviewConclusion::Partial),
        "stale" => Ok(ReviewConclusion::Stale),
        "unavailable" => Ok(ReviewConclusion::Unavailable),
        "cancelled" => Ok(ReviewConclusion::Cancelled),
        "error" => Ok(ReviewConclusion::Error),
        _ => Err(binding_corrupt("persisted review conclusion is invalid")),
    }
}

fn review_finding_sort_key(finding: &ReviewFinding) -> String {
    let severity = match finding.severity {
        rove_runtime::review::ReviewSeverity::Critical => 0,
        rove_runtime::review::ReviewSeverity::High => 1,
        rove_runtime::review::ReviewSeverity::Medium => 2,
        rove_runtime::review::ReviewSeverity::Low => 3,
        rove_runtime::review::ReviewSeverity::Info => 4,
    };
    format!(
        "{severity:02}:{:08}:{}:{}",
        finding.location.start_line, finding.path, finding.finding_id
    )
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn bool_from_i64(value: i64) -> Result<bool, ProductStoreError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(storage_error("persisted product boolean is invalid")),
    }
}

/// Whether a runtime id is already owned by a product session other than `expected`.
///
/// The owner tables key on the runtime id alone, so an id held by another
/// session cannot be re-owned. Recovery uses this to tell "not yet recorded"
/// apart from "someone else's", since only the first is recoverable.
fn runtime_owner_conflicts(
    transaction: &Transaction<'_>,
    query: &str,
    runtime_id: &str,
    expected: &ProductSessionId,
) -> Result<bool, ProductStoreError> {
    let owner = transaction
        .query_row(query, params![runtime_id], |row| row.get::<_, String>(0))
        .optional()
        .map_err(storage_error)?;
    Ok(owner.is_some_and(|owner| owner != expected.to_string()))
}

fn workspace_kind_to_db(kind: ProductWorkspaceKind) -> &'static str {
    match kind {
        ProductWorkspaceKind::Folder => "folder",
        ProductWorkspaceKind::Repo => "repo",
    }
}

fn workspace_kind_from_db(value: &str) -> Result<ProductWorkspaceKind, ProductStoreError> {
    match value {
        "folder" => Ok(ProductWorkspaceKind::Folder),
        "repo" => Ok(ProductWorkspaceKind::Repo),
        _ => Err(storage_error("persisted workspace kind is invalid")),
    }
}

fn session_status_to_db(status: ProductSessionStatus) -> &'static str {
    match status {
        ProductSessionStatus::Idle => "idle",
        ProductSessionStatus::Running => "running",
        ProductSessionStatus::Error => "error",
        ProductSessionStatus::NeedsAttention => "needs_attention",
        ProductSessionStatus::Archived => "archived",
    }
}

fn session_status_from_db(value: &str) -> Result<ProductSessionStatus, ProductStoreError> {
    match value {
        "idle" => Ok(ProductSessionStatus::Idle),
        "running" => Ok(ProductSessionStatus::Running),
        "error" => Ok(ProductSessionStatus::Error),
        "needs_attention" => Ok(ProductSessionStatus::NeedsAttention),
        "archived" => Ok(ProductSessionStatus::Archived),
        _ => Err(storage_error("persisted product session status is invalid")),
    }
}

fn control_kind_to_db(kind: ProductControlKind) -> &'static str {
    match kind {
        ProductControlKind::Steer => "steer",
        ProductControlKind::Followup => "followup",
    }
}

fn control_kind_from_db(value: &str) -> Result<ProductControlKind, ProductStoreError> {
    match value {
        "steer" => Ok(ProductControlKind::Steer),
        "followup" => Ok(ProductControlKind::Followup),
        _ => Err(storage_error("persisted product control kind is invalid")),
    }
}

fn control_status_to_db(status: ProductControlStatus) -> &'static str {
    match status {
        ProductControlStatus::Pending => "pending",
        ProductControlStatus::Accepted => "accepted",
        ProductControlStatus::Applied => "applied",
        ProductControlStatus::Dropped => "dropped",
        ProductControlStatus::Abandoned => "abandoned",
        ProductControlStatus::Revoked => "revoked",
    }
}

fn control_status_from_db(value: &str) -> Result<ProductControlStatus, ProductStoreError> {
    match value {
        "pending" => Ok(ProductControlStatus::Pending),
        "accepted" => Ok(ProductControlStatus::Accepted),
        "applied" => Ok(ProductControlStatus::Applied),
        "dropped" => Ok(ProductControlStatus::Dropped),
        "abandoned" => Ok(ProductControlStatus::Abandoned),
        "revoked" => Ok(ProductControlStatus::Revoked),
        _ => Err(storage_error("persisted product control status is invalid")),
    }
}

fn row_to_control(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProductControl> {
    let control_id: String = row.get(0)?;
    let product_session_id: String = row.get(1)?;
    let kind: String = row.get(2)?;
    let idempotency_key: Option<String> = row.get(3)?;
    let content: String = row.get(4)?;
    let status: String = row.get(5)?;
    let run_id: Option<String> = row.get(6)?;
    let seq: i64 = row.get(7)?;
    let created_at: String = row.get(8)?;
    let applied_at: Option<String> = row.get(9)?;

    // Map storage failures into rusqlite::Error so query_row/query_map work.
    let mapped = (|| {
        Ok::<ProductControl, ProductStoreError>(ProductControl {
            id: parse_product_id(&control_id, "control id")?,
            product_session_id: parse_product_id(&product_session_id, "product session id")?,
            kind: control_kind_from_db(&kind)?,
            idempotency_key,
            content,
            status: control_status_from_db(&status)?,
            run_id: run_id
                .as_deref()
                .map(|value| parse_runtime_id(value, "control run id"))
                .transpose()?,
            seq,
            created_at,
            applied_at,
        })
    })();
    mapped.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.message)),
        )
    })
}

fn get_message_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &ProductSessionId,
    message_id: &ProductControlId,
) -> Result<ProductMessage, ProductStoreError> {
    transaction
        .query_row(
            r#"
            SELECT control_id, product_session_id, content, status, run_id, seq,
                   created_at, applied_at, abandoned_reason, requested_delivery
            FROM product_session_controls
            WHERE product_session_id = ?1 AND control_id = ?2
              AND message_contract_version = 1
            "#,
            params![session_id.to_string(), message_id.to_string()],
            row_to_message,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| not_found("message not found"))
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProductMessage> {
    let id: String = row.get(0)?;
    let product_session_id: String = row.get(1)?;
    let content: String = row.get(2)?;
    let status: String = row.get(3)?;
    let run_id: Option<String> = row.get(4)?;
    let seq: i64 = row.get(5)?;
    let created_at: String = row.get(6)?;
    let applied_at: Option<String> = row.get(7)?;
    let persisted_reason: Option<String> = row.get(8)?;
    let requested_delivery: String = row.get(9)?;
    let mapped = (|| {
        let stored_status = control_status_from_db(&status)?;
        let requested_delivery = match requested_delivery.as_str() {
            "successor" => ProductMessageDelivery::Successor,
            "current_run" => ProductMessageDelivery::CurrentRun,
            _ => {
                return Err(ProductStoreError::new(
                    ProductErrorCode::ProductStorageFailure,
                    "persisted product message delivery is invalid",
                ));
            }
        };
        let run_id = run_id
            .as_deref()
            .map(|value| parse_runtime_id(value, "message run id"))
            .transpose()?;
        let status = match stored_status {
            ProductControlStatus::Pending => match requested_delivery {
                ProductMessageDelivery::Successor => ProductMessageStatus::Queued,
                ProductMessageDelivery::CurrentRun => ProductMessageStatus::InterventionRequested,
            },
            ProductControlStatus::Accepted => match requested_delivery {
                ProductMessageDelivery::Successor => ProductMessageStatus::ClaimedSuccessor,
                ProductMessageDelivery::CurrentRun => ProductMessageStatus::InterventionRequested,
            },
            ProductControlStatus::Applied => match requested_delivery {
                ProductMessageDelivery::Successor => ProductMessageStatus::ClaimedSuccessor,
                ProductMessageDelivery::CurrentRun => ProductMessageStatus::AppliedCurrentRun,
            },
            ProductControlStatus::Dropped | ProductControlStatus::Abandoned => {
                ProductMessageStatus::NeedsAttention
            }
            ProductControlStatus::Revoked => ProductMessageStatus::Revoked,
        };
        Ok::<_, ProductStoreError>(ProductMessage {
            id: parse_product_id(&id, "message id")?,
            product_session_id: parse_product_id(&product_session_id, "product session id")?,
            content,
            requested_delivery,
            actual_delivery: matches!(
                status,
                ProductMessageStatus::AppliedCurrentRun | ProductMessageStatus::ClaimedSuccessor
            )
            .then_some(requested_delivery),
            status,
            seq,
            run_id: (requested_delivery == ProductMessageDelivery::CurrentRun)
                .then_some(run_id)
                .flatten(),
            successor_run_id: (requested_delivery == ProductMessageDelivery::Successor)
                .then_some(run_id)
                .flatten(),
            created_at,
            applied_at,
            reason: match status {
                ProductMessageStatus::NeedsAttention => persisted_reason.or_else(|| {
                    Some("message delivery requires an explicit recovery decision".to_string())
                }),
                _ => persisted_reason,
            },
        })
    })();
    mapped.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.message)),
        )
    })
}

fn provider_type_to_db(provider_type: ProductProviderType) -> &'static str {
    match provider_type {
        ProductProviderType::Openai => "openai",
        ProductProviderType::OpenaiResponses => "openai-responses",
        ProductProviderType::Anthropic => "anthropic",
        ProductProviderType::Ollama => "ollama",
        ProductProviderType::Fake => "fake",
    }
}

fn provider_type_from_db(value: &str) -> Result<ProductProviderType, ProductStoreError> {
    match value {
        "openai" => Ok(ProductProviderType::Openai),
        "openai-responses" => Ok(ProductProviderType::OpenaiResponses),
        "anthropic" => Ok(ProductProviderType::Anthropic),
        "ollama" => Ok(ProductProviderType::Ollama),
        "fake" => Ok(ProductProviderType::Fake),
        _ => Err(storage_error("persisted provider type is invalid")),
    }
}

fn theme_to_db(theme: ProductThemePreference) -> &'static str {
    match theme {
        ProductThemePreference::Light => "light",
        ProductThemePreference::Dark => "dark",
        ProductThemePreference::System => "system",
    }
}

fn theme_from_db(value: &str) -> Result<ProductThemePreference, ProductStoreError> {
    match value {
        "light" => Ok(ProductThemePreference::Light),
        "dark" => Ok(ProductThemePreference::Dark),
        "system" => Ok(ProductThemePreference::System),
        _ => Err(storage_error("persisted product theme is invalid")),
    }
}

fn approval_to_db(approval: ProductApprovalPreference) -> &'static str {
    match approval {
        ProductApprovalPreference::Ask => "ask",
        ProductApprovalPreference::Auto => "auto",
        ProductApprovalPreference::Never => "never",
    }
}

fn approval_from_db(value: &str) -> Result<ProductApprovalPreference, ProductStoreError> {
    match value {
        "ask" => Ok(ProductApprovalPreference::Ask),
        "auto" => Ok(ProductApprovalPreference::Auto),
        "never" => Ok(ProductApprovalPreference::Never),
        _ => Err(storage_error("persisted approval preference is invalid")),
    }
}

fn migration_issue_code_to_db(code: M1MigrationIssueCode) -> &'static str {
    match code {
        M1MigrationIssueCode::InvalidWorkspace => "invalid_workspace",
        M1MigrationIssueCode::MissingWorkspace => "missing_workspace",
        M1MigrationIssueCode::InvalidRuntimeHint => "invalid_runtime_hint",
        M1MigrationIssueCode::AmbiguousRuntimeBinding => "ambiguous_runtime_binding",
        M1MigrationIssueCode::RuntimeBindingNotFound => "runtime_binding_not_found",
        M1MigrationIssueCode::InvalidPreferenceReference => "invalid_preference_reference",
        M1MigrationIssueCode::PreferenceWriteConflict => "preference_write_conflict",
    }
}

fn limit_i64(value: usize) -> Result<i64, ProductStoreError> {
    i64::try_from(value).map_err(storage_error)
}

pub(super) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn not_found(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductNotFound, message)
}

fn session_active(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductSessionActive, message)
}

fn resume_conflict(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductSessionResumeConflict, message)
}

fn binding_corrupt(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductBindingCorrupt, message)
}

fn fork_conflict(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductForkConflict, message)
}

fn fork_source_invalid(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductForkSourceInvalid, message)
}

fn validate_fork_idempotency_key(value: &str) -> Result<String, ProductStoreError> {
    if value.is_empty() || value.len() > 128 {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "fork idempotency_key must be 1..128 characters",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "fork idempotency_key must not contain control characters",
        ));
    }
    Ok(value.to_string())
}

fn fork_request_digest(request: &CreateProductForkRequest) -> String {
    // Length-delimited fields avoid conflating distinct title/run combinations
    // while remaining deterministic across retried JSON requests.
    let title = request.title.as_deref().unwrap_or("");
    rove_runtime::context::stable_hash(&format!(
        "run:{}:{}:title:{}:{}",
        request.fork_at_run_id,
        request.fork_at_run_id.to_string().len(),
        title.len(),
        title
    ))
}

fn default_fork_title(parent_title: &str) -> String {
    const PREFIX: &str = "Fork of ";
    let available = MAX_PRODUCT_TEXT_BYTES.saturating_sub(PREFIX.len());
    if parent_title.len() <= available {
        return format!("{PREFIX}{parent_title}");
    }
    let mut end = available;
    while end > 0 && !parent_title.is_char_boundary(end) {
        end -= 1;
    }
    format!("{PREFIX}{}", &parent_title[..end])
}
