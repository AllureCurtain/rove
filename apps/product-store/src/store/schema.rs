use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{ProductErrorCode, ProductStoreError};

const CURRENT_SCHEMA_VERSION: i64 = 22;
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

/// Make the session listing order seekable.
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

/// `product_sessions.last_outcome` records how the most recent finished turn
/// ended, which the status column cannot express: a successful turn, a
/// cancelled turn, and a failed turn all release the session back to a
/// non-running status. Existing rows stay NULL, and NULL means "no turn has
/// finished yet" — back-filling a guess would turn an unknown into a claim.
const MIGRATION_016_COLUMNS: [(&str, &str); 2] = [
    (
        "last_outcome",
        "TEXT CHECK(last_outcome IS NULL OR last_outcome IN ('success', 'failed', 'cancelled'))",
    ),
    ("last_outcome_at", "TEXT"),
];

/// `product_session_controls.queue_order` is the explicit successor-queue
/// position. `seq` stays the append-only ledger order that paging and the
/// transcript projection rely on, so a reorder must not rewrite it. A row with
/// `queue_order IS NULL` predates migration 017 and keeps its creation order
/// through the `COALESCE(queue_order, seq)` sort key, which means the upgrade
/// needs no back-fill and no queue can be silently reshuffled by it.
const MIGRATION_017_COLUMNS: [(&str, &str); 1] = [("queue_order", "INTEGER")];

/// The product-level directory event log behind `GET /product/events`.
///
/// It is a rolling signal, not an audit log: rows are trimmed to the newest
/// `MAX_PRODUCT_EVENTS_RETAINED` entries as they are appended, and `id` doubles
/// as the SSE `id:` field so a reconnect can resume. Every column is a bounded,
/// secret-free summary: message content, tool arguments, error details, and
/// secrets are never stored here. The primary key already serves the only read
/// pattern (`WHERE id > ? ORDER BY id`), so no second index is created.
const MIGRATION_018: &str = r#"
CREATE TABLE IF NOT EXISTS product_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    session_id TEXT NULL,
    workspace_id TEXT NULL,
    summary TEXT NULL,
    created_at TEXT NOT NULL
);
"#;

/// Edit-and-resend forks record the parent message the child's seed stops
/// before. `truncate_after_message_seq` is the client-facing ledger sequence
/// (and part of the fork request digest), while `truncate_after_message_id`
/// names the same row so the child's first turn can still be cut after the
/// parent's message ledger has been deleted along with its catalog row.
///
/// `CURRENT_SCHEMA_VERSION` skips 18 on purpose: that migration belongs to the
/// open R5 successor-queue work, and a fresh database ending at 19 is a
/// complete schema because every step is guarded by `migration_is_applied`.
const MIGRATION_019_COLUMNS: [(&str, &str); 2] = [
    (
        "truncate_after_message_seq",
        "INTEGER CHECK(truncate_after_message_seq IS NULL OR truncate_after_message_seq >= 1)",
    ),
    ("truncate_after_message_id", "TEXT"),
];

/// Migration 020: the single-session message-search index.
///
/// `product_messages_fts` is an **external content** FTS5 table over
/// `product_session_controls.content`, tokenized with `trigram`. The tokenizer
/// was probed at runtime against the same `rusqlite` `bundled` build the store
/// links (SQLite 3.46.0, `ENABLE_FTS5` present, trigram table creation and
/// substring `MATCH` both work); the probe and its limits are recorded in the
/// design record's R7 implementation record.
///
/// External content means the index stores only tokens and reads the body back
/// from the ledger, so a message body exists once. That only stays true if
/// every write to the ledger is mirrored into the index, which is what the
/// three triggers do: they run inside the statement's own transaction, so a
/// message and its index entry commit or roll back together. The probe
/// confirmed the delete trigger also fires for a foreign-key
/// `ON DELETE CASCADE` from `product_sessions`/`product_workspaces`, so the
/// index cannot outlive the ledger through the cascade paths either.
///
/// Legacy `steer` control rows are indexed too — the index has no way to tell
/// them apart — and the search query filters them out by
/// `message_contract_version = 1`. That keeps the write path trigger-only
/// (nothing for a future writer to remember) at the cost of a few tokens that
/// no query returns.
///
/// Hidden risk worth recording: this table's `rowid` is
/// `product_session_controls.rowid`, and `VACUUM` is free to renumber the
/// rowids of a table without an `INTEGER PRIMARY KEY`. Nothing in the API
/// vacuums the product store today, and a future one would have to re-run
/// `rebuild` afterwards.
const MIGRATION_020: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS product_messages_fts USING fts5(
    content,
    content = 'product_session_controls',
    content_rowid = 'rowid',
    tokenize = 'trigram'
);

CREATE TRIGGER IF NOT EXISTS product_session_controls_fts_insert
AFTER INSERT ON product_session_controls BEGIN
    INSERT INTO product_messages_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS product_session_controls_fts_delete
AFTER DELETE ON product_session_controls BEGIN
    INSERT INTO product_messages_fts(product_messages_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS product_session_controls_fts_update
AFTER UPDATE OF content ON product_session_controls BEGIN
    INSERT INTO product_messages_fts(product_messages_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
    INSERT INTO product_messages_fts(rowid, content) VALUES (new.rowid, new.content);
END;

INSERT INTO product_messages_fts(product_messages_fts) VALUES('rebuild');
"#;

/// Migration 021: the durable metadata record for session-scoped attachments.
///
/// SQLite is authoritative for metadata; the payload bytes live at
/// `<data_root>/attachments/<product_session_id>/<attachment_id>` and are *not*
/// content-addressed (design section 3.3): two sessions must not share a
/// deletion, and a client must not be able to probe another session's store by
/// comparing returned ids. The filesystem therefore holds bytes only, and this
/// table is the only thing that can resolve an id to a payload.
///
/// `content_type` is the **locally verified** type — never the client's
/// `Content-Type` claim, which is read for comparison and then discarded.
/// `byte_length` and `sha256` are computed while streaming the body, so a read
/// can re-verify them and report `missing`/`corrupt` instead of streaming a
/// truncated body as success.
///
/// `status` is the row lifecycle: `staged` (written but not referenced by a
/// message), `referenced` (durable session content, no TTL), `expired`
/// (deliberately reclaimed by the cleanup job — a visible conflict, not a
/// silent 404). `display_name` is a bounded, sanitized display hint; it is
/// never a filesystem name and no code path may derive a path from it.
/// `scan_flags` is a JSON array of secret-free warning codes.
///
/// The `(product_session_id, status)` index backs the per-session quota
/// accounting, which runs inside the same transaction that publishes a row.
const MIGRATION_021: &str = r#"
CREATE TABLE IF NOT EXISTS product_attachments (
    attachment_id TEXT PRIMARY KEY,
    product_session_id TEXT NOT NULL,
    content_type TEXT NOT NULL,
    byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
    sha256 TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('staged', 'referenced', 'expired')),
    display_name TEXT,
    created_at TEXT NOT NULL,
    referenced_at TEXT,
    expires_at TEXT,
    scan_flags TEXT NOT NULL DEFAULT '[]',
    FOREIGN KEY(product_session_id) REFERENCES product_sessions(product_session_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_product_attachments_session_status
    ON product_attachments(product_session_id, status);
"#;

/// `product_session_controls.message_attachments` is the ordered JSON array of
/// the attachment references one message carries, `[{"attachment_id": …,
/// "name": …}, …]`.
///
/// The list lives on the message row rather than in a join table for one
/// reason: the message *is* the set. Order is part of the contract (the
/// injected blocks keep the client's order), an empty or absent array is the
/// pre-022 state and reads as "no attachments", and the reference is written in
/// the same transaction as the row that names it, which is what makes "a
/// referenced attachment has a durable message that names it" atomic.
///
/// Nothing here is authoritative about an attachment's type, size, or digest:
/// those stay on `product_attachments`, so a reference cannot drift from the row
/// it points at.
const MIGRATION_022_COLUMNS: [(&str, &str); 1] = [("message_attachments", "TEXT")];

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

    /// The product database file.
    ///
    /// The attachment payload root is defined as a sibling of this file, so a
    /// read that has to state an attachment's availability needs the same path
    /// the store was opened with — not a copy that a workspace move could stale.
    pub(super) fn path(&self) -> &Path {
        self.path.as_path()
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
    apply_migration_016(connection)?;
    apply_migration_017(connection)?;
    apply_migration_018(connection)?;
    apply_migration_019(connection)?;
    apply_migration_020(connection)?;
    apply_migration_021(connection)?;
    apply_migration_022(connection)?;
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

fn apply_migration_016(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 16)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // Guarded for the same reason as migrations 007 and 015: a historical
    // compatibility fixture can claim a version without containing every table
    // that version implies, and an added column is pure additive state.
    if table_exists(&transaction, "product_sessions")? {
        for (column, declaration) in MIGRATION_016_COLUMNS {
            if !table_has_column(&transaction, "product_sessions", column)? {
                transaction
                    .execute_batch(&format!(
                        "ALTER TABLE product_sessions ADD COLUMN {column} {declaration};"
                    ))
                    .map_err(|_| database_error(true))?;
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                16,
                "product_session_last_outcome",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_017(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 17)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // Guarded for the same reason as migrations 007, 015, and 016: a historical
    // compatibility fixture can claim a version without containing every table
    // that version implies, and an added column is pure additive state.
    if table_exists(&transaction, "product_session_controls")? {
        for (column, declaration) in MIGRATION_017_COLUMNS {
            if !table_has_column(&transaction, "product_session_controls", column)? {
                transaction
                    .execute_batch(&format!(
                        "ALTER TABLE product_session_controls ADD COLUMN {column} {declaration};"
                    ))
                    .map_err(|_| database_error(true))?;
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                17,
                "product_message_queue_order",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_018(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 18)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // The statement is `IF NOT EXISTS`, so a compatibility fixture that already
    // carries the table upgrades by recording the version only.
    transaction
        .execute_batch(MIGRATION_018)
        .map_err(|_| database_error(true))?;
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![18, "product_events_log", super::repository::now_rfc3339()],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

fn apply_migration_019(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 19)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    // Guarded for the same reason as migrations 007 and 015-017: a historical
    // compatibility fixture can claim a version without containing every table
    // that version implies, and an added column is pure additive state.
    if table_exists(&transaction, "product_session_forks")? {
        for (column, declaration) in MIGRATION_019_COLUMNS {
            if !table_has_column(&transaction, "product_session_forks", column)? {
                transaction
                    .execute_batch(&format!(
                        "ALTER TABLE product_session_forks ADD COLUMN {column} {declaration};"
                    ))
                    .map_err(|_| database_error(true))?;
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                19,
                "product_fork_message_truncation",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

/// Build the single-session message-search index.
///
/// Guarded like migrations 007 and 015-019: a historical compatibility fixture
/// can claim a version without containing every table that version implies, and
/// an FTS5 external-content table whose content table is missing cannot be
/// created at all (`no such table`), which would fail store startup. The guard
/// therefore keeps one incomplete fixture from making the whole store
/// unavailable. It is not a search fallback: a store without
/// `product_session_controls` cannot answer a search on either query path, and
/// the index is created whenever that table exists.
fn apply_migration_020(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 20)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    if table_exists(&transaction, "product_session_controls")? {
        transaction
            .execute_batch(MIGRATION_020)
            .map_err(|_| database_error(true))?;
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                20,
                "product_message_search_index",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

/// Create the durable attachment metadata record.
///
/// Guarded like migrations 007 and 015-020: a historical compatibility fixture
/// can claim a version without containing every table that version implies, and
/// a table whose foreign key names a missing `product_sessions` would make the
/// store unusable for every later read. Attachments are additive state, so a
/// fixture without sessions simply does not get the table.
///
/// The version is recorded even when that guard skipped the DDL, which is the
/// same convention 007 and 015-020 follow. Recording it only on a real create
/// would leave a fixture's ledger at 20 forever, so every later open would
/// retry the same skip and the store would never report reaching
/// `CURRENT_SCHEMA_VERSION`. The divergence this accepts is real, is the reason
/// the guard is documented at all, and is asserted by
/// `a_fixture_without_sessions_records_the_version_without_the_table`:
/// `migration_is_applied(21)` can be true while `product_attachments` does not
/// exist. Only a fixture that lacks `product_sessions` can be in that state —
/// every store that has the table gets the attachment table on its next open.
fn apply_migration_021(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 21)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    if table_exists(&transaction, "product_sessions")? {
        transaction
            .execute_batch(MIGRATION_021)
            .map_err(|_| database_error(true))?;
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                21,
                "product_attachment_records",
                super::repository::now_rfc3339()
            ],
        )
        .map_err(|_| database_error(true))?;
    transaction.commit().map_err(|_| database_error(true))?;
    Ok(())
}

/// Attach the message-level attachment list to the message row.
///
/// Additive and in place: a v21 database keeps every session, message, and
/// attachment row and gains one nullable column, so an existing message reads
/// back with an empty attachment set and serialises exactly as it did before.
/// The `table_has_column` check makes a replay a no-op rather than an error,
/// which is the convention migrations 007 and 015-021 follow; the
/// `table_exists` guard keeps a compatibility fixture that claims a version
/// without the table from failing the whole open.
fn apply_migration_022(connection: &mut Connection) -> Result<(), ProductStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| database_error(true))?;
    if migration_is_applied(&transaction, 22)? {
        transaction.commit().map_err(|_| database_error(true))?;
        return Ok(());
    }
    if table_exists(&transaction, "product_session_controls")? {
        for (column, declaration) in MIGRATION_022_COLUMNS {
            if !table_has_column(&transaction, "product_session_controls", column)? {
                transaction
                    .execute_batch(&format!(
                        "ALTER TABLE product_session_controls ADD COLUMN {column} {declaration};"
                    ))
                    .map_err(|_| database_error(true))?;
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO product_schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![
                22,
                "product_message_attachments",
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
        // Migration 016 is additive, so a fresh store must record it and have
        // both columns even though no session row exists yet.
        assert!(migration_is_applied(connection, 16).unwrap());
        for column in ["last_outcome", "last_outcome_at"] {
            assert!(table_has_column(connection, "product_sessions", column).unwrap());
        }
        // Migration 017 is additive in the same way, and a store that recorded
        // the version without the column would sort every queue by `seq` while
        // claiming to support reordering.
        assert!(migration_is_applied(connection, 17).unwrap());
        assert!(table_has_column(connection, "product_session_controls", "queue_order").unwrap());
        // Migration 018 creates the directory event log, so a fresh store must
        // record it and own the table even though no event was appended yet.
        assert!(migration_is_applied(connection, 18).unwrap());
        assert!(table_exists(connection, "product_events").unwrap());
        // Migration 019 is the edit-and-resend fork truncation. It is additive
        // too, and a store that recorded the version without the columns would
        // accept `truncate_after_message_seq` and then fork the whole prefix.
        assert!(migration_is_applied(connection, 19).unwrap());
        for column in ["truncate_after_message_seq", "truncate_after_message_id"] {
            assert!(table_has_column(connection, "product_session_forks", column).unwrap());
        }
        // Migration 020 is the message-search index. Recording the version
        // without the virtual table and its triggers would leave every search
        // silently empty, or leave the index drifting from the ledger.
        assert!(migration_is_applied(connection, 20).unwrap());
        assert!(table_exists(connection, "product_messages_fts").unwrap());
        for trigger in [
            "product_session_controls_fts_insert",
            "product_session_controls_fts_delete",
            "product_session_controls_fts_update",
        ] {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                        params![trigger],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1,
                "the message-search index is missing trigger {trigger}"
            );
        }
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

    #[test]
    fn a_v15_store_gains_the_session_outcome_columns_without_backfilling() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        insert_v15_session(&connection);
        assert!(!table_has_column(&connection, "product_sessions", "last_outcome").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 16).unwrap());
        assert!(table_has_column(&connection, "product_sessions", "last_outcome").unwrap());
        assert!(table_has_column(&connection, "product_sessions", "last_outcome_at").unwrap());
        // An existing session keeps an unknown outcome: back-filling a guess
        // would turn "no turn has finished here" into a claim about the past.
        assert_eq!(
            connection
                .query_row(
                    "SELECT last_outcome, last_outcome_at FROM product_sessions WHERE product_session_id = 'session-1'",
                    [],
                    |row| Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    )),
                )
                .unwrap(),
            (None, None)
        );
    }

    #[test]
    fn a_store_with_one_outcome_column_already_present_still_upgrades() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        // The guard exists because a compatibility fixture can carry part of a
        // version. Adding the missing column must not fail on the present one.
        connection
            .execute_batch("ALTER TABLE product_sessions ADD COLUMN last_outcome TEXT;")
            .unwrap();

        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 16).unwrap());
        assert!(table_has_column(&connection, "product_sessions", "last_outcome").unwrap());
        assert!(table_has_column(&connection, "product_sessions", "last_outcome_at").unwrap());
    }

    #[test]
    fn session_outcomes_are_constrained_and_the_upgrade_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op rather than an attempt
        // to add the same columns twice.
        apply_migrations_isolated(&mut connection).unwrap();
        insert_v15_session(&connection);

        for outcome in ["success", "failed", "cancelled"] {
            connection
                .execute(
                    "UPDATE product_sessions SET last_outcome = ?1, last_outcome_at = '2026-09-26T00:00:00Z' WHERE product_session_id = 'session-1'",
                    params![outcome],
                )
                .unwrap_or_else(|error| panic!("{outcome} must be an accepted outcome: {error}"));
        }
        connection
            .execute(
                "UPDATE product_sessions SET last_outcome = NULL, last_outcome_at = NULL WHERE product_session_id = 'session-1'",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "UPDATE product_sessions SET last_outcome = 'partial' WHERE product_session_id = 'session-1'",
                    [],
                )
                .is_err(),
            "an outcome outside the three-value contract must be rejected"
        );
    }

    #[test]
    fn a_v16_store_gains_the_queue_order_column_without_reordering_its_queue() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        insert_v15_session(&connection);
        insert_v16_message(&connection);
        assert!(!table_has_column(&connection, "product_session_controls", "queue_order").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 17).unwrap());
        assert!(table_has_column(&connection, "product_session_controls", "queue_order").unwrap());
        // The existing row keeps an unknown position, which the read path turns
        // back into its creation order. Back-filling a number would freeze
        // today's order into the database and make the upgrade a reorder.
        assert_eq!(
            connection
                .query_row(
                    "SELECT queue_order FROM product_session_controls WHERE control_id = 'message-1'",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_queue_order_column_already_present_still_upgrades_and_the_pass_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        insert_v15_session(&connection);
        insert_v16_message(&connection);
        // A compatibility fixture can carry part of a version, so the guard must
        // add nothing that is already there and must not fail on it.
        connection
            .execute_batch("ALTER TABLE product_session_controls ADD COLUMN queue_order INTEGER;")
            .unwrap();

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op rather than an attempt
        // to add the same column twice.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 17).unwrap());
        assert!(table_has_column(&connection, "product_session_controls", "queue_order").unwrap());
        connection
            .execute(
                "UPDATE product_session_controls SET queue_order = 3 WHERE control_id = 'message-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT queue_order FROM product_session_controls WHERE control_id = 'message-1'",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap(),
            Some(3),
            "an explicit position must survive a repeat migration pass"
        );
    }

    #[test]
    fn a_v17_store_gains_the_directory_event_log_without_synthesizing_events() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        apply_migration_017(&mut connection).unwrap();
        insert_v15_session(&connection);
        assert!(!table_exists(&connection, "product_events").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass must not attempt to create the table twice.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 18).unwrap());
        assert!(table_exists(&connection, "product_events").unwrap());
        // The upgrade creates an empty log. Emitting a synthetic "state
        // recovered" fact for every existing session would make the directory
        // stream report changes that never happened.
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM product_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "an upgrade must not invent directory facts"
        );
    }

    /// A v17 store predating the edit-and-resend fork truncation gains both
    /// columns, keeps its fork rows intact, and treats the columns as absent
    /// for every existing fork: an old fork keeps the whole parent prefix.
    #[test]
    fn a_v17_store_gains_the_fork_truncation_columns_without_truncating_old_forks() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        apply_migration_017(&mut connection).unwrap();
        insert_v15_session(&connection);
        connection
            .execute_batch(
                r#"
                INSERT INTO product_session_forks(
                    fork_id, parent_product_session_id, child_product_session_id,
                    parent_workspace_id, parent_title, source_runtime_session_id,
                    source_runtime_job_id, source_runtime_run_id, fork_at_event_seq,
                    idempotency_key, request_digest, created_at
                ) VALUES (
                    'fork-1', 'session-1', 'session-2', 'workspace-1', 'Parent',
                    'runtime-session-1', 'runtime-job-1', 'runtime-run-1', 4,
                    'key-1', 'digest-1', '2026-09-26T00:00:00Z'
                );
                "#,
            )
            .unwrap();
        assert!(
            !table_has_column(
                &connection,
                "product_session_forks",
                "truncate_after_message_seq"
            )
            .unwrap()
        );

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op rather than an attempt
        // to add the same columns twice.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 19).unwrap());
        for column in ["truncate_after_message_seq", "truncate_after_message_id"] {
            assert!(table_has_column(&connection, "product_session_forks", column).unwrap());
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT truncate_after_message_seq, truncate_after_message_id
                     FROM product_session_forks WHERE fork_id = 'fork-1'",
                    [],
                    |row| Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?
                    )),
                )
                .unwrap(),
            (None, None),
            "an existing fork keeps the whole parent prefix"
        );
    }

    /// A v19 store predating single-session message search gains the index,
    /// indexes the messages it already holds, and reaches a no-op on a second
    /// pass. The index is then maintained by the triggers alone: a message
    /// inserted afterwards is searchable without any application-side write to
    /// the index table, and deleting it removes the index entry again.
    #[test]
    fn a_v19_store_gains_the_message_search_index_and_backfills_existing_messages() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        apply_migration_017(&mut connection).unwrap();
        apply_migration_019(&mut connection).unwrap();
        insert_v15_session(&connection);
        insert_v16_message(&connection);
        assert!(!table_exists(&connection, "product_messages_fts").unwrap());

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op rather than a second
        // rebuild of the whole ledger.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 20).unwrap());
        assert!(table_exists(&connection, "product_messages_fts").unwrap());
        assert_eq!(
            fts_match_count(&connection, "queued"),
            1,
            "a message written before the migration must be searchable after it"
        );

        connection
            .execute_batch(
                "INSERT INTO product_session_controls(
                     control_id, product_session_id, kind, content, status, seq,
                     created_at, message_contract_version, requested_delivery
                 ) VALUES (
                     'message-2', 'session-1', 'followup', 'searchable immediately',
                     'pending', 2, '2026-09-26T00:00:01Z', 1, 'successor'
                 );",
            )
            .unwrap();
        assert_eq!(
            fts_match_count(&connection, "searchable immediately"),
            1,
            "the insert trigger must index a newly written message"
        );

        connection
            .execute_batch("DELETE FROM product_session_controls WHERE control_id = 'message-2';")
            .unwrap();
        assert_eq!(
            fts_match_count(&connection, "searchable immediately"),
            0,
            "the delete trigger must remove the index entry with its message"
        );
        assert_eq!(
            fts_match_count(&connection, "queued"),
            1,
            "deleting one message must not disturb another one's index entry"
        );
    }

    /// A fresh store creates the attachment record, records version 21, and
    /// gives the table the exact columns and constraint set migration 021
    /// declares.
    #[test]
    fn a_fresh_store_creates_the_attachment_record() {
        let mut connection = Connection::open_in_memory().unwrap();

        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 21).unwrap());
        assert!(table_exists(&connection, "product_attachments").unwrap());
        for column in [
            "attachment_id",
            "product_session_id",
            "content_type",
            "byte_length",
            "sha256",
            "status",
            "display_name",
            "created_at",
            "referenced_at",
            "expires_at",
            "scan_flags",
        ] {
            assert!(
                table_has_column(&connection, "product_attachments", column).unwrap(),
                "{column} must exist on product_attachments"
            );
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT name FROM product_schema_migrations WHERE version = 21",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "product_attachment_records"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema
                     WHERE type = 'index' AND name = 'idx_product_attachments_session_status'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );

        // The status vocabulary is exactly the three lifecycle states the
        // endpoint contract names, so a typo cannot become a fourth state.
        connection
            .execute_batch(
                "INSERT INTO product_workspaces(
                     workspace_id, canonical_root, canonical_key, kind, display_name,
                     pinned, last_opened_at, created_at, updated_at
                 ) VALUES (
                     'ws-1', 'C:/ws', 'key-1', 'folder', 'ws',
                     0, '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                 );
                 INSERT INTO product_sessions(
                     product_session_id, workspace_id, title, status, created_at, updated_at
                 ) VALUES (
                     'session-1', 'ws-1', 's', 'idle',
                     '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                 );
                 INSERT INTO product_attachments(
                     attachment_id, product_session_id, content_type, byte_length,
                     sha256, status, created_at
                 ) VALUES (
                     'att-1', 'session-1', 'image/png', 12,
                     'abc', 'staged', '2026-09-26T00:00:00Z'
                 );",
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT scan_flags FROM product_attachments WHERE attachment_id = 'att-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "[]",
            "scan_flags must default to an empty, secret-free array"
        );
        assert!(
            connection
                .execute_batch(
                    "INSERT INTO product_attachments(
                         attachment_id, product_session_id, content_type, byte_length,
                         sha256, status, created_at
                     ) VALUES (
                         'att-2', 'session-1', 'image/png', 12,
                         'abc', 'quarantined', '2026-09-26T00:00:00Z'
                     );",
                )
                .is_err(),
            "an unknown status must be refused by the table CHECK"
        );
    }

    /// A v20 store gains the attachment record, keeps the state it already
    /// held, and reaches a no-op on a second pass rather than re-running the
    /// migration.
    #[test]
    fn a_v20_store_gains_the_attachment_record_without_rewriting_existing_state() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        apply_migration_012(&mut connection).unwrap();
        apply_migration_013(&mut connection).unwrap();
        apply_migration_014(&mut connection).unwrap();
        apply_migration_015(&mut connection).unwrap();
        apply_migration_016(&mut connection).unwrap();
        apply_migration_017(&mut connection).unwrap();
        apply_migration_019(&mut connection).unwrap();
        apply_migration_020(&mut connection).unwrap();
        insert_v15_session(&connection);
        insert_v16_message(&connection);

        assert!(!table_exists(&connection, "product_attachments").unwrap());
        assert_eq!(
            connection
                .query_row(
                    "SELECT MAX(version) FROM product_schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            20
        );

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op, not a second create.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(table_exists(&connection, "product_attachments").unwrap());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM product_schema_migrations WHERE version = 21",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "an idempotent re-run must not record the migration twice"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT content FROM product_session_controls WHERE control_id = 'message-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "queued",
            "the upgrade must not rewrite existing message state"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT title FROM product_sessions WHERE product_session_id = 'session-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "existing",
            "the upgrade must not rewrite existing session state"
        );
    }

    /// Deleting a session removes its attachment rows through the foreign-key
    /// cascade, so a deleted session cannot leave resolvable metadata behind.
    #[test]
    fn a_fixture_without_sessions_records_the_version_without_the_table() {
        // The accepted divergence, made discoverable rather than left implicit:
        // a compatibility fixture whose ledger claims 20 but which has no
        // `product_sessions` cannot hold a foreign key to it, so the guard skips
        // the create — and the version is still recorded, as migrations 007 and
        // 015-020 record theirs.
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE product_schema_migrations(
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 WITH RECURSIVE claimed(version) AS (
                     SELECT 1 UNION ALL SELECT version + 1 FROM claimed WHERE version < 20
                 )
                 INSERT INTO product_schema_migrations(version, name, applied_at)
                     SELECT version, 'fixture', '2026-09-26T00:00:00Z' FROM claimed;",
            )
            .unwrap();

        apply_migration_021(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 21).unwrap());
        assert!(
            !table_exists(&connection, "product_attachments").unwrap(),
            "the guard skipped the create and the ledger still advanced"
        );

        // The skip is permanent, so a later pass does not revisit the table even
        // once a sessions table exists.
        connection
            .execute_batch(
                "CREATE TABLE product_sessions(
                     product_session_id TEXT PRIMARY KEY,
                     workspace_id TEXT NOT NULL,
                     title TEXT NOT NULL,
                     status TEXT NOT NULL,
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
        apply_migration_021(&mut connection).unwrap();
        assert!(
            !table_exists(&connection, "product_attachments").unwrap(),
            "a recorded version is not re-run, so the divergence is stable"
        );
    }

    /// A v20 store with sessions gains the table, which is the state every
    /// non-fixture store reaches.
    #[test]
    fn a_store_with_sessions_gets_the_attachment_table_on_its_next_open() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        for migration in [
            apply_migration_012,
            apply_migration_013,
            apply_migration_014,
            apply_migration_015,
            apply_migration_016,
            apply_migration_017,
            apply_migration_019,
            apply_migration_020,
        ] {
            migration(&mut connection).unwrap();
        }
        assert!(table_exists(&connection, "product_sessions").unwrap());
        assert!(!table_exists(&connection, "product_attachments").unwrap());

        apply_migration_021(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 21).unwrap());
        assert!(table_exists(&connection, "product_attachments").unwrap());
    }

    /// A v21 store gains the message-attachment column in place: a message that
    /// predates the column keeps its row and reads as attachment-free, and a
    /// compatibility fixture that already carries the column is not altered by a
    /// second pass.
    #[test]
    fn a_v21_store_gains_the_message_attachment_column_in_place() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        for migration in [
            apply_migration_012,
            apply_migration_013,
            apply_migration_014,
            apply_migration_015,
            apply_migration_016,
            apply_migration_017,
            apply_migration_019,
            apply_migration_020,
            apply_migration_021,
        ] {
            migration(&mut connection).unwrap();
        }
        insert_v15_session(&connection);
        insert_v16_message(&connection);
        assert!(
            !table_has_column(
                &connection,
                "product_session_controls",
                "message_attachments"
            )
            .unwrap(),
            "the fixture must predate the column for the upgrade to be the thing under test"
        );

        apply_migrations_isolated(&mut connection).unwrap();
        // A second pass over a current store is a no-op rather than an attempt
        // to add the same column twice.
        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 22).unwrap());
        assert!(
            table_has_column(
                &connection,
                "product_session_controls",
                "message_attachments"
            )
            .unwrap()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT message_attachments FROM product_session_controls
                     WHERE control_id = 'message-1'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap(),
            None,
            "a message written before the column existed stays readable and attachment-free"
        );
    }

    /// A v21 store whose controls table already carries the column (an
    /// interrupted or hand-built fixture) still reaches 22 without failing.
    #[test]
    fn a_message_attachment_column_already_present_still_upgrades() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_v11(&mut connection);
        for migration in [
            apply_migration_012,
            apply_migration_013,
            apply_migration_014,
            apply_migration_015,
            apply_migration_016,
            apply_migration_017,
            apply_migration_019,
            apply_migration_020,
            apply_migration_021,
        ] {
            migration(&mut connection).unwrap();
        }
        connection
            .execute_batch(
                "ALTER TABLE product_session_controls ADD COLUMN message_attachments TEXT;",
            )
            .unwrap();

        apply_migrations_isolated(&mut connection).unwrap();

        assert!(migration_is_applied(&connection, 22).unwrap());
        assert!(
            table_has_column(
                &connection,
                "product_session_controls",
                "message_attachments"
            )
            .unwrap()
        );
    }

    /// Deleting a session removes its attachment rows through the foreign-key
    /// cascade, so a deleted session cannot leave resolvable metadata behind.
    #[test]
    fn deleting_a_session_cascades_its_attachment_rows() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations_isolated(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO product_workspaces(
                     workspace_id, canonical_root, canonical_key, kind, display_name,
                     pinned, last_opened_at, created_at, updated_at
                 ) VALUES (
                     'ws-1', 'C:/ws', 'key-1', 'folder', 'ws',
                     0, '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                 );
                 INSERT INTO product_sessions(
                     product_session_id, workspace_id, title, status, created_at, updated_at
                 ) VALUES (
                     'session-1', 'ws-1', 's', 'idle',
                     '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                 );
                 INSERT INTO product_attachments(
                     attachment_id, product_session_id, content_type, byte_length,
                     sha256, status, created_at
                 ) VALUES (
                     'att-1', 'session-1', 'image/png', 12,
                     'abc', 'staged', '2026-09-26T00:00:00Z'
                 );",
            )
            .unwrap();

        connection
            .execute_batch("DELETE FROM product_sessions WHERE product_session_id = 'session-1';")
            .unwrap();

        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM product_attachments", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    /// The newest recorded version is this build's version, so a store this
    /// build opens records the message-attachment contract rather than stopping
    /// short of it.
    #[test]
    fn the_attachment_record_is_the_newest_recorded_version() {
        let mut connection = Connection::open_in_memory().unwrap();

        apply_migrations_isolated(&mut connection).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT MAX(version) FROM product_schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    /// Count index entries matching a phrase, with the same quoting the search
    /// query uses so the assertion is about the shipped behaviour.
    fn fts_match_count(connection: &Connection, term: &str) -> i64 {
        connection
            .query_row(
                "SELECT COUNT(*) FROM product_messages_fts WHERE product_messages_fts MATCH ?1",
                params![format!("\"{}\"", term.replace('"', "\"\""))],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// A minimal v16 successor message, used to prove migration 017 leaves the
    /// existing queue untouched.
    fn insert_v16_message(connection: &Connection) {
        connection
            .execute_batch(
                r#"
                INSERT INTO product_session_controls(
                    control_id, product_session_id, kind, content, status, seq,
                    created_at, message_contract_version, requested_delivery
                ) VALUES (
                    'message-1', 'session-1', 'followup', 'queued', 'pending', 1,
                    '2026-09-26T00:00:00Z', 1, 'successor'
                );
                "#,
            )
            .unwrap();
    }

    /// A minimal v15 session row, used to prove additive migrations do not
    /// rewrite existing state.
    fn insert_v15_session(connection: &Connection) {
        connection
            .execute_batch(
                r#"
                INSERT INTO product_workspaces(
                    workspace_id, canonical_root, canonical_key, kind, display_name,
                    pinned, last_opened_at, created_at, updated_at
                ) VALUES (
                    'ws-1', 'C:/ws', 'key-1', 'folder', 'ws',
                    0, '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                );
                INSERT INTO product_sessions(
                    product_session_id, workspace_id, title, status, created_at, updated_at
                ) VALUES (
                    'session-1', 'ws-1', 'existing', 'idle',
                    '2026-09-26T00:00:00Z', '2026-09-26T00:00:00Z'
                );
                "#,
            )
            .unwrap();
    }
}
