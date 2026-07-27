use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;

use rove_runtime::types::{JobId, RunId, SessionId};

use crate::product::{
    CommitProductRunBinding, CreateProductProviderProfileRequest, CreateProductSessionRequest,
    CreateProductWorkspaceRequest, M1BrowserMigrationPreflight, M1BrowserMigrationResponse,
    M1MigrationDisposition, M1MigrationIssue, M1MigrationIssueCode, M1PreferencesBaseline,
    M1ProviderProfileIdMapping, M1SessionIdMapping, M1WorkspaceIdMapping,
    MAX_PRODUCT_PROVIDER_PROFILES, MAX_PRODUCT_SESSIONS, MAX_PRODUCT_WORKSPACES,
    PreparedM1BrowserMigration, ProductApprovalPreference, ProductErrorCode,
    ProductMigrationReceiptId, ProductPreferences, ProductProviderProfile,
    ProductProviderProfileId, ProductProviderSelection, ProductProviderType, ProductResumeHealth,
    ProductResumeHealthStatus, ProductRuntimeBinding, ProductSession, ProductSessionContext,
    ProductSessionId, ProductSessionRunBinding, ProductSessionStatus, ProductStoreError,
    ProductThemePreference, ProductTurnClaim, ProductTurnClaimId, ProductWorkspace,
    ProductWorkspaceId, ProductWorkspaceKind, UpdateProductPreferencesRequest,
    UpdateProductProviderProfileRequest, UpdateProductSessionRequest, VerifiedM1SessionRunBinding,
    m1_browser_migration_digest,
};

use super::schema::{ProductDatabase, storage_error};
use super::validation::{
    MAX_RUN_BINDINGS_PER_SESSION, ValidatedPreferences, ValidatedProviderProfile,
    ValidatedWorkspace, invalid, normalized_timestamp, profile_id_string, validate_issue_entity,
    validate_migration_envelope, validate_migration_provider, validate_preferences,
    validate_provider_create, validate_provider_selection, validate_provider_update,
    validate_source_id, validate_title, validate_workspace, validate_workspace_request,
};

const MIGRATION_SOURCE_WEB_M1: &str = "web_m1_local_storage";
const MIGRATION_PREPARATION_TTL_SECS: i64 = 24 * 60 * 60;

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
        self.recover_stale_turn_claims()
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
        let affected = transaction
            .execute(
                r#"
                UPDATE product_sessions
                SET status = 'needs_attention', updated_at = ?1
                WHERE product_session_id IN (
                    SELECT product_session_id FROM product_turn_claims
                )
                "#,
                params![now_rfc3339()],
            )
            .map_err(storage_error)?;
        transaction
            .execute("DELETE FROM product_turn_claims", [])
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        u64::try_from(affected).map_err(storage_error)
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

    pub(super) fn list_sessions(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<Vec<ProductSession>, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        require_workspace(&transaction, workspace_id)?;
        let sessions = {
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT product_session_id, workspace_id, title, status, latest_ordinal,
                           runtime_session_id, latest_job_id, latest_run_id, created_at, updated_at
                    FROM product_sessions
                    WHERE workspace_id = ?1
                    ORDER BY CASE WHEN status = 'archived' THEN 1 ELSE 0 END ASC,
                             updated_at DESC, created_at DESC, product_session_id ASC
                    LIMIT ?2
                    "#,
                )
                .map_err(storage_error)?;
            let rows = statement
                .query_map(
                    params![workspace_id.to_string(), limit_i64(MAX_PRODUCT_SESSIONS)?],
                    raw_session_from_row,
                )
                .map_err(storage_error)?;
            let mut sessions = Vec::new();
            for row in rows {
                sessions.push(row.map_err(storage_error)?.into_product()?);
            }
            sessions
        };
        for session in &sessions {
            validate_binding_integrity(&transaction, session)?;
        }
        transaction.commit().map_err(storage_error)?;
        Ok(sessions)
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
        transaction.commit().map_err(storage_error)?;
        Ok(ProductSessionContext { workspace, session })
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
            context: ProductSessionContext { workspace, session },
            previous_status,
            previous_binding,
        })
    }

    pub(super) fn commit_run_binding(
        &self,
        binding: CommitProductRunBinding,
    ) -> Result<ProductSessionRunBinding, ProductStoreError> {
        let mut connection = self.database.connect()?;
        let transaction = immediate_transaction(&mut connection)?;
        let claimed_session_id = transaction
            .query_row(
                "SELECT product_session_id FROM product_turn_claims WHERE claim_id = ?1",
                params![binding.claim_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|value| parse_product_id::<ProductSessionId>(&value, "product session id"))
            .transpose()?;
        if claimed_session_id.as_ref() != Some(&binding.product_session_id) {
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
        transaction.commit().map_err(storage_error)?;
        Ok(created)
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
        Ok(ProductSession {
            id: parse_product_id(&self.id, "product session id")?,
            workspace_id: parse_product_id(&self.workspace_id, "workspace id")?,
            title: self.title,
            status: session_status_from_db(&self.status)?,
            runtime_binding,
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

impl RawProviderProfile {
    fn into_product(self) -> Result<ProductProviderProfile, ProductStoreError> {
        Ok(ProductProviderProfile {
            id: parse_product_id(&self.id, "provider profile id")?,
            label: self.label,
            provider_type: provider_type_from_db(&self.provider_type)?,
            api_base: self.api_base,
            api_key_env: self.api_key_env,
            default_model: self.default_model,
            created_at: self.created_at,
            updated_at: self.updated_at,
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
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
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
                   runtime_session_id, latest_job_id, latest_run_id, created_at, updated_at
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
