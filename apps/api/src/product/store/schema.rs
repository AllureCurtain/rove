use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::product::{ProductErrorCode, ProductStoreError};

const CURRENT_SCHEMA_VERSION: i64 = 15;
const MAX_BUSY_TIMEOUT_MS: u64 = 120_000;

const MIGRATION_001: &str = r#"
CREATE TABLE product_workspaces (
    workspace_id TEXT PRIMARY KEY,
    canonical_root TEXT NOT NULL,
    canonical_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('folder', 'repo')),
    display_name TEXT NOT NULL,
    pinned INTEGER NOT NULL CHECK(pinned IN (0, 1)),
    last_opened_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE product_sessions (
    product_session_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK(
        status IN ('idle', 'running', 'error', 'needs_attention', 'archived')
    ),
    latest_ordinal INTEGER,
    runtime_session_id TEXT,
    latest_job_id TEXT,
    latest_run_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(workspace_id) REFERENCES product_workspaces(workspace_id)
        ON DELETE CASCADE,
    CHECK(
        (latest_ordinal IS NULL
            AND runtime_session_id IS NULL
            AND latest_job_id IS NULL
            AND latest_run_id IS NULL)
        OR
        (latest_ordinal >= 1
            AND runtime_session_id IS NOT NULL
            AND latest_job_id IS NOT NULL
            AND latest_run_id IS NOT NULL)
    )
);

CREATE TABLE product_provider_profiles (
    profile_id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK(
        provider_type IN ('openai', 'openai-responses', 'anthropic', 'ollama', 'fake')
    ),
    api_base TEXT NOT NULL,
    api_key_env TEXT,
    default_model TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE product_migration_receipts (
    receipt_id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    source_schema_version INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    response_json TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    UNIQUE(source, source_schema_version, idempotency_key)
);

CREATE TABLE product_session_runs (
    product_session_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
    runtime_session_id TEXT NOT NULL,
    runtime_job_id TEXT NOT NULL,
    runtime_run_id TEXT NOT NULL UNIQUE,
    resumed_from_run_id TEXT,
    bound_at TEXT NOT NULL,
    migration_receipt_id TEXT,
    PRIMARY KEY(product_session_id, ordinal),
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(migration_receipt_id) REFERENCES product_migration_receipts(receipt_id)
        ON DELETE SET NULL,
    FOREIGN KEY(runtime_session_id, product_session_id)
        REFERENCES product_runtime_session_owners(runtime_session_id, product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(runtime_job_id, runtime_session_id, product_session_id)
        REFERENCES product_runtime_job_owners(
            runtime_job_id, runtime_session_id, product_session_id
        ) ON DELETE CASCADE
);

CREATE TABLE product_runtime_session_owners (
    runtime_session_id TEXT PRIMARY KEY,
    product_session_id TEXT NOT NULL,
    UNIQUE(runtime_session_id, product_session_id),
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE TABLE product_runtime_job_owners (
    runtime_job_id TEXT PRIMARY KEY,
    runtime_session_id TEXT NOT NULL,
    product_session_id TEXT NOT NULL,
    UNIQUE(runtime_job_id, runtime_session_id, product_session_id),
    FOREIGN KEY(runtime_session_id, product_session_id)
        REFERENCES product_runtime_session_owners(runtime_session_id, product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE TABLE product_turn_claims (
    claim_id TEXT PRIMARY KEY,
    product_session_id TEXT NOT NULL UNIQUE,
    claimed_at TEXT NOT NULL,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE TABLE product_preferences (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    schema_version INTEGER NOT NULL,
    theme TEXT NOT NULL CHECK(theme IN ('light', 'dark', 'system')),
    active_workspace_id TEXT,
    active_session_id TEXT,
    provider_profile_id TEXT,
    provider_model TEXT,
    provider_approval TEXT CHECK(
        provider_approval IS NULL OR provider_approval IN ('ask', 'auto', 'never')
    ),
    provider_max_steps INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(active_workspace_id) REFERENCES product_workspaces(workspace_id)
        ON DELETE SET NULL,
    FOREIGN KEY(active_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE SET NULL,
    FOREIGN KEY(provider_profile_id) REFERENCES product_provider_profiles(profile_id)
        ON DELETE SET NULL,
    CHECK(
        (provider_model IS NULL
            AND provider_approval IS NULL
            AND provider_max_steps IS NULL
            AND provider_profile_id IS NULL)
        OR
        (provider_model IS NOT NULL
            AND provider_approval IS NOT NULL
            AND provider_max_steps >= 1)
    )
);

CREATE TABLE product_migration_workspace_sources (
    source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(source, source_id),
    FOREIGN KEY(workspace_id) REFERENCES product_workspaces(workspace_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_session_sources (
    source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    product_session_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(source, source_id),
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_profile_sources (
    source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(source, source_id),
    FOREIGN KEY(profile_id) REFERENCES product_provider_profiles(profile_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_receipt_workspace_mappings (
    receipt_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    source_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    PRIMARY KEY(receipt_id, ordinal),
    FOREIGN KEY(receipt_id) REFERENCES product_migration_receipts(receipt_id)
        ON DELETE CASCADE,
    FOREIGN KEY(workspace_id) REFERENCES product_workspaces(workspace_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_receipt_session_mappings (
    receipt_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    source_id TEXT NOT NULL,
    product_session_id TEXT NOT NULL,
    PRIMARY KEY(receipt_id, ordinal),
    FOREIGN KEY(receipt_id) REFERENCES product_migration_receipts(receipt_id)
        ON DELETE CASCADE,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_receipt_profile_mappings (
    receipt_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    source_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    PRIMARY KEY(receipt_id, ordinal),
    FOREIGN KEY(receipt_id) REFERENCES product_migration_receipts(receipt_id)
        ON DELETE CASCADE,
    FOREIGN KEY(profile_id) REFERENCES product_provider_profiles(profile_id)
        ON DELETE CASCADE
);

CREATE TABLE product_migration_receipt_issues (
    receipt_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    code TEXT NOT NULL,
    entity TEXT NOT NULL,
    source_id TEXT,
    PRIMARY KEY(receipt_id, ordinal),
    FOREIGN KEY(receipt_id) REFERENCES product_migration_receipts(receipt_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_product_workspaces_list
    ON product_workspaces(pinned DESC, last_opened_at DESC, workspace_id ASC);
CREATE INDEX idx_product_sessions_workspace_list
    ON product_sessions(workspace_id, updated_at DESC, product_session_id ASC);
CREATE INDEX idx_product_session_runs_order
    ON product_session_runs(product_session_id, ordinal ASC);
CREATE INDEX idx_product_provider_profiles_list
    ON product_provider_profiles(label COLLATE NOCASE, profile_id ASC);
"#;

const MIGRATION_014: &str = r#"
CREATE TABLE IF NOT EXISTS product_reviews (
    review_id TEXT PRIMARY KEY,
    product_session_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK(target_kind IN ('uncommitted', 'base', 'commit')),
    target_revision TEXT,
    resolved_base TEXT,
    target_digest TEXT NOT NULL,
    target_summary_json TEXT NOT NULL,
    target_spec_json TEXT NOT NULL,
    state_root TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'queued', 'running', 'pass', 'findings', 'partial', 'stale',
        'needs_attention', 'unavailable', 'cancelled', 'error'
    )),
    conclusion TEXT,
    runtime_session_id TEXT,
    job_id TEXT,
    run_id TEXT,
    result_json TEXT,
    idempotency_key TEXT,
    findings_count INTEGER NOT NULL DEFAULT 0 CHECK(findings_count >= 0),
    unchecked_count INTEGER NOT NULL DEFAULT 0 CHECK(unchecked_count >= 0),
    warnings_count INTEGER NOT NULL DEFAULT 0 CHECK(warnings_count >= 0),
    captured_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finalized_at TEXT,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(workspace_id) REFERENCES product_workspaces(workspace_id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_product_reviews_idempotency
    ON product_reviews(product_session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_product_reviews_session_created
    ON product_reviews(product_session_id, created_at DESC, review_id DESC);
CREATE INDEX IF NOT EXISTS idx_product_reviews_active_digest
    ON product_reviews(product_session_id, target_digest, status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_product_reviews_one_active_target
    ON product_reviews(product_session_id, target_digest)
    WHERE status IN ('queued', 'running');

CREATE TABLE IF NOT EXISTS product_review_findings (
    review_id TEXT NOT NULL,
    finding_id TEXT NOT NULL,
    sort_key TEXT NOT NULL,
    finding_json TEXT NOT NULL,
    location_status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(review_id, finding_id),
    FOREIGN KEY(review_id) REFERENCES product_reviews(review_id)
        ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_product_review_findings_order
    ON product_review_findings(review_id, sort_key, finding_id);
"#;

/// Codex alignment Phase 7: make the session listing order seekable.
///
/// The listing has always sorted live sessions before archived ones, and
/// `idx_product_sessions_workspace_list` could not serve that leading term, so
/// SQLite sorted the whole workspace on every request. That was tolerable only
/// because the listing also stopped at `MAX_PRODUCT_SESSIONS` and silently
/// dropped the tail.
///
/// Indexing the `CASE` expression itself lets one index cover the full sort
/// key, which turns "the page after this row" into a range scan and keeps the
/// archived-last grouping the UI already relies on. Dropping the grouping would
/// have been the easier way to get a keyset order; it would also have silently
/// reshuffled every client's list.
const MIGRATION_015: &str = r#"
CREATE INDEX IF NOT EXISTS idx_product_sessions_workspace_page
    ON product_sessions(
        workspace_id,
        CASE WHEN status = 'archived' THEN 1 ELSE 0 END ASC,
        updated_at DESC,
        product_session_id ASC
    );
"#;

const MIGRATION_002: &str = r#"
ALTER TABLE product_preferences
ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
CHECK(typeof(revision) = 'integer' AND revision >= 0);
"#;

const MIGRATION_003: &str = r#"
CREATE TABLE product_migration_preparations (
    source TEXT NOT NULL,
    source_schema_version INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    preferences_requested INTEGER NOT NULL CHECK(preferences_requested IN (0, 1)),
    preferences_revision INTEGER,
    created_at TEXT NOT NULL,
    PRIMARY KEY(source, source_schema_version, idempotency_key),
    CHECK(
        (preferences_requested = 0 AND preferences_revision IS NULL)
        OR
        (preferences_requested = 1
            AND typeof(preferences_revision) = 'integer'
            AND preferences_revision >= 0)
    )
);
"#;

const MIGRATION_004: &str = r#"
ALTER TABLE product_preferences
ADD COLUMN default_approval_policy TEXT NOT NULL DEFAULT 'ask'
CHECK(default_approval_policy IN ('ask', 'auto', 'never'));
"#;

const MIGRATION_005: &str = r#"
CREATE TABLE product_session_controls (
    control_id TEXT PRIMARY KEY,
    product_session_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('steer', 'followup')),
    idempotency_key TEXT,
    request_digest TEXT,
    content TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'accepted', 'applied', 'dropped', 'abandoned', 'revoked'
    )),
    run_id TEXT,
    seq INTEGER NOT NULL,
    abandoned_reason TEXT,
    created_at TEXT NOT NULL,
    applied_at TEXT,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_product_session_controls_idempotency
    ON product_session_controls(product_session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX idx_product_session_controls_session_status
    ON product_session_controls(product_session_id, status, created_at ASC);
CREATE INDEX idx_product_session_controls_session_seq
    ON product_session_controls(product_session_id, seq ASC);
"#;

// A follow-up's `run_id` is written in the same ProductStore transaction as
// the corresponding run binding. This turns the formerly ambiguous
// `accepted` state into a recoverable delivery record: an accepted row with
// no run id never crossed the runtime-start boundary and can be requeued;
// one with a run id was durably bound and must not be started again.
const MIGRATION_006_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_product_session_controls_followup_recovery
    ON product_session_controls(product_session_id, kind, status, run_id, seq);
"#;

// Fork provenance intentionally has no foreign keys to product_sessions. A
// removed parent catalog row must not erase a child's immutable source
// boundary or read-only runtime-run references. Runtime artifacts remain under
// the workspace StateStore and are validated when read.
const MIGRATION_007: &str = r#"
CREATE TABLE IF NOT EXISTS product_session_forks (
    fork_id TEXT PRIMARY KEY,
    parent_product_session_id TEXT NOT NULL,
    child_product_session_id TEXT NOT NULL UNIQUE,
    parent_workspace_id TEXT NOT NULL,
    parent_title TEXT NOT NULL,
    source_runtime_session_id TEXT NOT NULL,
    source_runtime_job_id TEXT NOT NULL,
    source_runtime_run_id TEXT NOT NULL,
    fork_at_event_seq INTEGER NOT NULL CHECK(fork_at_event_seq >= 1),
    idempotency_key TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(parent_product_session_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS product_fork_inherited_runs (
    fork_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
    source_product_session_id TEXT NOT NULL,
    runtime_session_id TEXT NOT NULL,
    runtime_job_id TEXT NOT NULL,
    runtime_run_id TEXT NOT NULL,
    through_event_seq INTEGER CHECK(through_event_seq IS NULL OR through_event_seq >= 1),
    PRIMARY KEY(fork_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_product_session_forks_parent
    ON product_session_forks(parent_product_session_id, created_at ASC, fork_id ASC);
CREATE INDEX IF NOT EXISTS idx_product_session_forks_child
    ON product_session_forks(child_product_session_id);
CREATE INDEX IF NOT EXISTS idx_product_fork_inherited_runs_source
    ON product_fork_inherited_runs(runtime_run_id);
"#;

const MIGRATION_008: &str = r#"
CREATE TABLE IF NOT EXISTS product_session_model_configs (
    product_session_id TEXT PRIMARY KEY,
    profile_id TEXT,
    model TEXT NOT NULL,
    reasoning TEXT NOT NULL CHECK(reasoning IN ('default', 'low', 'medium', 'high')),
    max_steps INTEGER NOT NULL CHECK(max_steps >= 1 AND max_steps <= 256),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    updated_at TEXT NOT NULL,
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(profile_id) REFERENCES product_provider_profiles(profile_id)
        ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS product_session_run_models (
    product_session_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
    runtime_run_id TEXT NOT NULL UNIQUE,
    profile_id TEXT,
    model TEXT NOT NULL,
    reasoning TEXT NOT NULL CHECK(reasoning IN ('default', 'low', 'medium', 'high')),
    max_steps INTEGER NOT NULL CHECK(max_steps >= 1 AND max_steps <= 256),
    started_at TEXT NOT NULL,
    PRIMARY KEY(product_session_id, ordinal),
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE,
    FOREIGN KEY(profile_id) REFERENCES product_provider_profiles(profile_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_product_session_run_models_session
    ON product_session_run_models(product_session_id, ordinal ASC);
"#;

const MIGRATION_009: &str = r#"
ALTER TABLE product_session_run_models ADD COLUMN pricing_source TEXT;
ALTER TABLE product_session_run_models ADD COLUMN pricing_version TEXT;
ALTER TABLE product_session_run_models ADD COLUMN pricing_currency TEXT;
ALTER TABLE product_session_run_models ADD COLUMN pricing_availability TEXT
    CHECK(
        pricing_availability IS NULL
        OR pricing_availability IN ('priced', 'local_zero', 'unpriced')
    );
ALTER TABLE product_session_run_models ADD COLUMN per_mtok_prompt REAL;
ALTER TABLE product_session_run_models ADD COLUMN per_mtok_completion REAL;
ALTER TABLE product_session_run_models ADD COLUMN per_mtok_cache_read REAL;
"#;

const MIGRATION_010: &str = r#"
ALTER TABLE product_session_run_models ADD COLUMN context_window INTEGER
    CHECK(context_window IS NULL OR context_window > 0);
"#;

const MIGRATION_011: &str = r#"
CREATE TABLE IF NOT EXISTS project_trust_records (
    canonical_root TEXT NOT NULL,
    workspace_kind TEXT NOT NULL CHECK(workspace_kind IN ('folder', 'repo', 'task')),
    identity_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('unknown', 'restricted', 'trusted', 'revoked')),
    capability_digests_json TEXT NOT NULL,
    granted_at TEXT,
    revoked_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(canonical_root, workspace_kind)
);
CREATE INDEX IF NOT EXISTS idx_project_trust_state
    ON project_trust_records(state, updated_at DESC);
"#;

const MIGRATION_012_PROVIDER_CATALOG: &str = r#"
CREATE TABLE IF NOT EXISTS product_provider_profile_catalog_mappings (
    source TEXT NOT NULL,
    source_profile_id TEXT NOT NULL,
    catalog_profile_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    migrated_at TEXT NOT NULL,
    PRIMARY KEY(source, source_profile_id)
);
"#;

const MIGRATION_012_LEGACY_PROVIDER_MAPPINGS: &str = r#"
INSERT OR IGNORE INTO product_provider_profile_catalog_mappings(
    source, source_profile_id, catalog_profile_id, source_digest, migrated_at
)
SELECT 'product_store_v11', profile_id, profile_id,
       'legacy-definition-pending-import', updated_at
FROM product_provider_profiles;
"#;

#[derive(Debug, Clone)]
pub(super) struct ProductDatabase {
    path: Arc<PathBuf>,
    busy_timeout_ms: u64,
}

impl ProductDatabase {
    pub(super) fn new(path: PathBuf, busy_timeout_ms: u64) -> Result<Self, ProductStoreError> {
        if path.as_os_str().is_empty()
            || path.to_string_lossy().len() > super::validation::MAX_PATH_BYTES
            || busy_timeout_ms == 0
            || busy_timeout_ms > MAX_BUSY_TIMEOUT_MS
        {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductStoreUnavailable,
                "product store configuration is invalid",
            ));
        }
        Ok(Self {
            path: Arc::new(path),
            busy_timeout_ms,
        })
    }

    pub(super) fn initialize(&self) -> Result<(), ProductStoreError> {
        let mut connection = self.open_connection(true)?;
        apply_migrations(&mut connection, self.path.as_ref())
    }

    pub(super) fn connect(&self) -> Result<Connection, ProductStoreError> {
        self.open_connection(false)
    }

    fn open_connection(&self, startup: bool) -> Result<Connection, ProductStoreError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|_| database_error(startup))?;
        }
        if self.path.is_dir() {
            return Err(database_error(startup));
        }

        let connection =
            Connection::open(self.path.as_ref()).map_err(|_| database_error(startup))?;
        connection
            .busy_timeout(Duration::from_millis(self.busy_timeout_ms))
            .map_err(|_| database_error(startup))?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|_| database_error(startup))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|_| database_error(startup))?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|_| database_error(startup))?;
        Ok(connection)
    }
}

/// Bring the product store up to [`CURRENT_SCHEMA_VERSION`].
///
/// Each migration already commits inside an `IMMEDIATE` transaction, which
/// makes any single step atomic. What that does not cover is the *sequence*:
/// two processes starting together could interleave steps, so a peer could
/// observe a schema that is half-way between two versions. The cross-process
/// barrier closes that window, and is taken only when work is actually pending
/// so the already-current startup path does no locking.
fn apply_migrations(
    connection: &mut Connection,
    database_path: &Path,
) -> Result<(), ProductStoreError> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS product_schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );
            "#,
        )
        .map_err(|_| database_error(true))?;

    if product_schema_is_current(connection)? {
        return Ok(());
    }

    let _barrier = rove_runtime::state::migration_lock::acquire_migration_lock(database_path)
        .map_err(|error| {
            ProductStoreError::new(
                ProductErrorCode::ProductStoreUnavailable,
                match error {
                    rove_runtime::state::migration_lock::MigrationLockError::Timeout { .. } => {
                        "another process is migrating the product store"
                    }
                    rove_runtime::state::migration_lock::MigrationLockError::Io { .. } => {
                        "product store migration lock is unavailable"
                    }
                },
            )
        })?;
    // Double-checked locking: a peer may have finished while this process waited.
    if product_schema_is_current(connection)? {
        return Ok(());
    }

    apply_migration_001(connection)?;
    apply_migration_002(connection)?;
    apply_migration_003(connection)?;
    apply_migration_004(connection)?;
    apply_migration_005(connection)?;
    apply_migration_006(connection)?;
    apply_migration_007(connection)?;
    apply_migration_008(connection)?;
    apply_migration_009(connection)?;
    apply_migration_010(connection)?;
    apply_migration_011(connection)?;
    apply_migration_012(connection)?;
    apply_migration_013(connection)?;
    apply_migration_014(connection)?;
    apply_migration_015(connection)?;
    Ok(())
}

/// True when the recorded version is already current. A version newer than this
/// build is refused rather than ignored.
fn product_schema_is_current(connection: &Connection) -> Result<bool, ProductStoreError> {
    let newest: Option<i64> = connection
        .query_row(
            "SELECT MAX(version) FROM product_schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(|_| database_error(true))?;
    if newest.is_some_and(|version| version > CURRENT_SCHEMA_VERSION) {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductStoreUnavailable,
            "product store schema is newer than this API",
        ));
    }
    Ok(newest == Some(CURRENT_SCHEMA_VERSION))
}

fn migration_is_applied(connection: &Connection, version: i64) -> Result<bool, ProductStoreError> {
    connection
        .query_row(
            "SELECT version FROM product_schema_migrations WHERE version = ?1",
            params![version],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|version| version.is_some())
        .map_err(|_| database_error(true))
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, ProductStoreError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(|_| database_error(true))
}

fn table_has_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, ProductStoreError> {
    connection
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2 LIMIT 1",
            params![table, column],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(|_| database_error(true))
}

fn apply_migration_001(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 1)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_001)
        .map_err(|_| database_error(true))?;
    let now = super::repository::now_rfc3339();
    transaction
        .execute(
            "INSERT INTO product_preferences(singleton, schema_version, theme, created_at, updated_at) VALUES (1, 1, 'system', ?1, ?1)",
            params![now],
        )
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![1, "product_store_v1", now],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_002(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 2)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_002)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                2,
                "product_preferences_revision",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_003(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 3)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_003)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                3,
                "product_migration_preparations",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_004(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 4)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_004)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                4,
                "product_default_approval_policy",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_005(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 5)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_005)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                5,
                "product_session_controls",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_006(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 6)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // The v1-preferences compatibility fixture intentionally predates the
    // session/claim tables. A real v1 product store has the table, but avoid
    // making a preferences-only legacy database impossible to open solely
    // because this additive delivery column has no parent table yet.
    if table_exists(&transaction, "product_turn_claims")?
        && !table_has_column(&transaction, "product_turn_claims", "followup_control_id")?
    {
        transaction
            .execute_batch("ALTER TABLE product_turn_claims ADD COLUMN followup_control_id TEXT;")
            .map_err(|_| database_error(true))?;
    }
    transaction
        .execute_batch(MIGRATION_006_INDEX)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                6,
                "product_followup_delivery_recovery",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_007(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 7)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // The compatibility fixture can contain only the preferences table. A
    // normal v6 ProductStore always contains product_sessions, but preserve the
    // existing additive-migration convention for partial historical fixtures.
    if table_exists(&transaction, "product_sessions")? {
        if !table_has_column(&transaction, "product_sessions", "parent_session_id")? {
            transaction
                .execute_batch("ALTER TABLE product_sessions ADD COLUMN parent_session_id TEXT;")
                .map_err(|_| database_error(true))?;
        }
        if !table_has_column(&transaction, "product_sessions", "fork_point_run_id")? {
            transaction
                .execute_batch("ALTER TABLE product_sessions ADD COLUMN fork_point_run_id TEXT;")
                .map_err(|_| database_error(true))?;
        }
        if !table_has_column(&transaction, "product_sessions", "fork_point_seq")? {
            transaction
                .execute_batch("ALTER TABLE product_sessions ADD COLUMN fork_point_seq INTEGER;")
                .map_err(|_| database_error(true))?;
        }
    }
    transaction
        .execute_batch(MIGRATION_007)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![7, "product_session_forks", super::repository::now_rfc3339()],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_008(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 8)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_008)
        .map_err(|_| database_error(true))?;
    // Existing sessions inherit the last global product preference exactly
    // once. New sessions and forks write their own row in their creation
    // transaction, so later global edits do not rewrite session defaults.
    if table_exists(&transaction, "product_sessions")? {
        transaction
            .execute(
                r#"
                INSERT OR IGNORE INTO product_session_model_configs(
                    product_session_id, profile_id, model, reasoning, max_steps,
                    revision, updated_at
                )
                SELECT sessions.product_session_id,
                       preferences.provider_profile_id,
                       COALESCE(preferences.provider_model, 'fake'),
                       'default',
                       COALESCE(preferences.provider_max_steps, 8),
                       1,
                       sessions.updated_at
                FROM product_sessions AS sessions
                CROSS JOIN product_preferences AS preferences
                WHERE preferences.singleton = 1
                "#,
                [],
            )
            .map_err(|_| database_error(true))?;
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                8,
                "product_session_model_config",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_009(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 9)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_009)
        .map_err(|_| database_error(true))?;
    // Existing run model rows keep their historical model identity. Pricing
    // columns stay NULL so cost stays unavailable until a new run captures a
    // real snapshot; we never invent retroactive rates for old runs.
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                9,
                "product_session_run_pricing_snapshot",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_010(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 10)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_010)
        .map_err(|_| database_error(true))?;
    // Existing rows remain NULL. Inferring a current hard limit for an old run
    // would violate the historical snapshot contract.
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                10,
                "product_session_run_context_snapshot",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_011(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 11)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_011)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                11,
                "project_trust_records",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_012(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 12)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    reconcile_productization_schema(&transaction)?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                12,
                "parallel_productization_workstreams",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_013(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 13)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    reconcile_productization_schema(&transaction)?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                13,
                "productization_integration_reconciliation",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_014(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 14)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    transaction
        .execute_batch(MIGRATION_014)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                14,
                "read_only_review_workflow",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_015(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 15)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // Guarded for the same reason as migration 007: the historical compatibility
    // fixtures can claim a version without containing every table that version
    // implies. Indexing a table that is not there would fail the whole upgrade,
    // and an index is pure derived state — a store that reaches this point
    // without the table has nothing to index yet.
    if table_exists(&transaction, "product_sessions")? {
        transaction
            .execute_batch(MIGRATION_015)
            .map_err(|_| database_error(true))?;
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                15,
                "session_listing_pagination",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn reconcile_productization_schema(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), ProductStoreError> {
    transaction
        .execute_batch(MIGRATION_012_PROVIDER_CATALOG)
        .map_err(|_| database_error(true))?;
    if table_exists(transaction, "product_provider_profiles")? {
        transaction
            .execute_batch(MIGRATION_012_LEGACY_PROVIDER_MAPPINGS)
            .map_err(|_| database_error(true))?;
    }
    if table_exists(transaction, "product_session_run_models")? {
        for (column, declaration) in [
            ("provider_type", "TEXT"),
            ("wire_protocol", "TEXT"),
            ("endpoint", "TEXT"),
            ("catalog_revision", "TEXT"),
            ("safe_config_digest", "TEXT"),
        ] {
            if !table_has_column(transaction, "product_session_run_models", column)? {
                transaction
                    .execute_batch(&format!(
                        "ALTER TABLE product_session_run_models ADD COLUMN {column} {declaration};"
                    ))
                    .map_err(|_| database_error(true))?;
            }
        }
    }
    if table_exists(transaction, "product_session_controls")? {
        if !table_has_column(
            transaction,
            "product_session_controls",
            "message_contract_version",
        )? {
            transaction
                .execute_batch(
                    "ALTER TABLE product_session_controls ADD COLUMN message_contract_version INTEGER NOT NULL DEFAULT 0 CHECK(message_contract_version IN (0, 1));",
                )
                .map_err(|_| database_error(true))?;
        }
        if !table_has_column(
            transaction,
            "product_session_controls",
            "requested_delivery",
        )? {
            transaction
                .execute_batch(
                    "ALTER TABLE product_session_controls ADD COLUMN requested_delivery TEXT CHECK(requested_delivery IS NULL OR requested_delivery IN ('successor', 'current_run'));",
                )
                .map_err(|_| database_error(true))?;
        }
        transaction
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_product_session_messages_delivery ON product_session_controls(product_session_id, message_contract_version, requested_delivery, status, seq);",
            )
            .map_err(|_| database_error(true))?;
    }
    Ok(())
}

fn database_error(startup: bool) -> ProductStoreError {
    if startup {
        ProductStoreError::new(
            ProductErrorCode::ProductStoreUnavailable,
            "product store is not available",
        )
    } else {
        ProductStoreError::new(
            ProductErrorCode::ProductStorageFailure,
            "product store operation failed",
        )
    }
}

pub(super) fn storage_error(_: impl std::fmt::Display) -> ProductStoreError {
    database_error(false)
}

pub(super) fn path_to_utf8(path: &Path) -> Result<&str, ProductStoreError> {
    path.to_str().ok_or_else(|| {
        ProductStoreError::new(
            ProductErrorCode::ProductInvalidInput,
            "workspace root must be valid UTF-8",
        )
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// Run the migration sequence against a connection whose barrier lives in a
    /// throwaway directory.
    ///
    /// The barrier is derived from the database path, and an in-memory database
    /// has none. Giving each call its own directory keeps these tests mutually
    /// independent, which is what an in-memory database was chosen for.
    fn apply_migrations_isolated(connection: &mut Connection) -> Result<(), ProductStoreError> {
        let temp = TempDir::new().unwrap();
        apply_migrations(connection, &temp.path().join("product.sqlite"))
    }

    #[test]
    fn schema_v1_preferences_upgrade_preserves_values_and_starts_revision_at_zero() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("product.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE product_schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                CREATE TABLE product_preferences (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    schema_version INTEGER NOT NULL,
                    theme TEXT NOT NULL,
                    active_workspace_id TEXT,
                    active_session_id TEXT,
                    provider_profile_id TEXT,
                    provider_model TEXT,
                    provider_approval TEXT,
                    provider_max_steps INTEGER,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO product_schema_migrations(version, name, applied_at)
                VALUES (1, 'product_store_v1', '2026-07-26T00:00:00Z');
                INSERT INTO product_preferences(
                    singleton, schema_version, theme, provider_model,
                    provider_approval, provider_max_steps, created_at, updated_at
                ) VALUES (
                    1, 1, 'dark', 'fake', 'never', 12,
                    '2026-07-26T00:00:00Z', '2026-07-26T00:00:00Z'
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let database = ProductDatabase::new(path, 5_000).unwrap();
        database.initialize().unwrap();
        let connection = database.connect().unwrap();
        let row = connection
            .query_row(
                "SELECT theme, provider_model, provider_approval, provider_max_steps, revision, default_approval_policy FROM product_preferences WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            row,
            (
                "dark".to_string(),
                "fake".to_string(),
                "never".to_string(),
                12,
                0,
                "ask".to_string()
            )
        );
        assert!(migration_is_applied(&connection, 2).unwrap());
        assert!(migration_is_applied(&connection, 3).unwrap());
        assert!(migration_is_applied(&connection, 4).unwrap());
        assert!(migration_is_applied(&connection, 5).unwrap());
        assert!(migration_is_applied(&connection, 6).unwrap());
        assert!(migration_is_applied(&connection, 11).unwrap());
        assert!(migration_is_applied(&connection, 12).unwrap());
        assert!(migration_is_applied(&connection, 13).unwrap());
        let preparations_table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'product_migration_preparations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preparations_table, 1);
        let controls_table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'product_session_controls'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(controls_table, 1);
        assert!(table_exists(&connection, "project_trust_records").unwrap());
        assert!(table_exists(&connection, "product_provider_profile_catalog_mappings").unwrap());
    }

    #[test]
    fn a_schema_newer_than_this_build_is_rejected_without_rollback() {
        let mut connection = Connection::open_in_memory().unwrap();
        // Derived from the constant rather than written out, so adding a migration
        // does not turn this test into an assertion that the *current* version is
        // rejected — which is how it would fail, silently testing nothing.
        let future = CURRENT_SCHEMA_VERSION + 1;
        connection
            .execute_batch(
                r#"
                CREATE TABLE product_schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                "#,
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO product_schema_migrations(version, name, applied_at)
                 VALUES (?1, 'future_schema', '2026-08-14T00:00:00Z')",
                params![future],
            )
            .unwrap();

        let error = apply_migrations_isolated(&mut connection).unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductStoreUnavailable);
        assert!(error.message.contains("newer than this API"));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM product_schema_migrations WHERE version = ?1",
                    params![future],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn fresh_database_reaches_v14_with_both_productization_contracts() {
        let mut connection = Connection::open_in_memory().unwrap();

        apply_migrations_isolated(&mut connection).unwrap();

        assert_integrated_v14(&connection);
    }

    #[test]
    fn integrated_v13_upgrades_to_v14_without_rewriting_existing_state() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        connection
            .execute(
                "UPDATE product_preferences SET theme = 'dark', revision = 7 WHERE singleton = 1",
                [],
            )
            .unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT MAX(version) FROM product_schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            13
        );
        assert!(!table_exists(&connection, "product_reviews").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();

        assert_integrated_v14(&connection);
        assert_eq!(
            connection
                .query_row(
                    "SELECT theme, revision FROM product_preferences WHERE singleton = 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            ("dark".to_string(), 7)
        );
    }

    #[test]
    fn provider_only_v12_upgrades_to_integrated_v14() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        connection
            .execute_batch(MIGRATION_012_PROVIDER_CATALOG)
            .unwrap();
        connection
            .execute_batch(MIGRATION_012_LEGACY_PROVIDER_MAPPINGS)
            .unwrap();
        for (column, declaration) in [
            ("provider_type", "TEXT"),
            ("wire_protocol", "TEXT"),
            ("endpoint", "TEXT"),
            ("catalog_revision", "TEXT"),
            ("safe_config_digest", "TEXT"),
        ] {
            connection
                .execute_batch(&format!(
                    "ALTER TABLE product_session_run_models ADD COLUMN {column} {declaration};"
                ))
                .unwrap();
        }
        record_parallel_v12(&connection, "provider_catalog_mapping");
        assert!(
            !table_has_column(
                &connection,
                "product_session_controls",
                "message_contract_version"
            )
            .unwrap()
        );

        apply_migrations_isolated(&mut connection).unwrap();

        assert_integrated_v14(&connection);
    }

    #[test]
    fn conversation_only_v12_upgrades_to_integrated_v14() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        connection
            .execute_batch(
                r#"
                ALTER TABLE product_session_controls
                ADD COLUMN message_contract_version INTEGER NOT NULL DEFAULT 0
                CHECK(message_contract_version IN (0, 1));
                ALTER TABLE product_session_controls
                ADD COLUMN requested_delivery TEXT
                CHECK(requested_delivery IS NULL OR requested_delivery IN ('successor', 'current_run'));
                CREATE INDEX IF NOT EXISTS idx_product_session_messages_delivery
                    ON product_session_controls(
                        product_session_id, message_contract_version,
                        requested_delivery, status, seq
                    );
                "#,
            )
            .unwrap();
        record_parallel_v12(&connection, "unified_product_message_lifecycle");
        assert!(!table_exists(&connection, "product_provider_profile_catalog_mappings").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();

        assert_integrated_v14(&connection);
    }

    fn initialize_v11(connection: &mut Connection) {
        connection
            .execute_batch(
                r#"
                CREATE TABLE product_schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                "#,
            )
            .unwrap();
        apply_migration_001(connection).unwrap();
        apply_migration_002(connection).unwrap();
        apply_migration_003(connection).unwrap();
        apply_migration_004(connection).unwrap();
        apply_migration_005(connection).unwrap();
        apply_migration_006(connection).unwrap();
        apply_migration_007(connection).unwrap();
        apply_migration_008(connection).unwrap();
        apply_migration_009(connection).unwrap();
        apply_migration_010(connection).unwrap();
        apply_migration_011(connection).unwrap();
    }

    fn record_parallel_v12(connection: &Connection, name: &str) {
        connection
            .execute(
                "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (12, ?1, '2026-08-12T00:00:00Z')",
                params![name],
            )
            .unwrap();
    }

    fn assert_integrated_v14(connection: &Connection) {
        assert!(migration_is_applied(connection, 13).unwrap());
        assert!(migration_is_applied(connection, 14).unwrap());
        assert!(table_exists(connection, "product_provider_profile_catalog_mappings").unwrap());
        assert!(table_exists(connection, "product_reviews").unwrap());
        assert!(table_exists(connection, "product_review_findings").unwrap());
        for column in [
            "provider_type",
            "wire_protocol",
            "endpoint",
            "catalog_revision",
            "safe_config_digest",
        ] {
            assert!(table_has_column(connection, "product_session_run_models", column).unwrap());
        }
        assert!(
            table_has_column(
                connection,
                "product_session_controls",
                "message_contract_version"
            )
            .unwrap()
        );
        assert!(
            table_has_column(connection, "product_session_controls", "requested_delivery").unwrap()
        );
        // Migration 015 exists only for its index, so a fresh database that
        // records the version without creating it would be a silent regression
        // that only the query-plan test would notice.
        assert!(migration_is_applied(connection, 15).unwrap());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema
                     WHERE type = 'index' AND name = 'idx_product_sessions_workspace_page'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "the session paging index is missing"
        );
    }

    #[test]
    fn failed_v11_migration_rolls_back_its_schema_record() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE product_schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                CREATE TABLE project_trust_records (
                    canonical_root TEXT PRIMARY KEY
                );
                "#,
            )
            .unwrap();

        let error = apply_migration_011(&mut connection).unwrap_err();

        assert_eq!(error.code, ProductErrorCode::ProductStoreUnavailable);
        assert!(!migration_is_applied(&connection, 11).unwrap());
        assert!(!table_has_column(&connection, "project_trust_records", "state").unwrap());
        let index_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name = 'idx_project_trust_state'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 0);
    }
}
