use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::product::{ProductErrorCode, ProductStoreError};

const CURRENT_SCHEMA_VERSION: i64 = 1;
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
        apply_migrations(&mut connection)
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

fn apply_migrations(connection: &mut Connection) -> Result<(), ProductStoreError> {
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

    let applied = connection
        .query_row(
            "SELECT version FROM product_schema_migrations WHERE version = ?1",
            params![CURRENT_SCHEMA_VERSION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|_| database_error(true))?;
    if applied.is_some() {
        return Ok(());
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
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
            params![CURRENT_SCHEMA_VERSION, "product_store_v1", now],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
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
