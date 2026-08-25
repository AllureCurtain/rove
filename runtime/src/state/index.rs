use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::events::StreamEvent;
use crate::types::{JobId, RunId, SessionId, TaskState};
use rove_core::CallId;

pub const CURRENT_SCHEMA_VERSION: i64 = 3;
const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;
const MAX_SNAPSHOT_EVENTS: usize = 2_000;
const MAX_SNAPSHOT_EVENT_JSON_BYTES: usize = 1_048_576;
const MAX_SNAPSHOT_EVENT_JSON_TOTAL_BYTES: usize = 16 * 1_048_576;
const RUNS_BY_JOB_INDEX: &str = "idx_runs_job_started";
const MAX_INSPECTION_RUNTIME_ID_BYTES: i64 = 64;
const MAX_INSPECTION_STATUS_BYTES: i64 = 64;
const MAX_INSPECTION_PATH_BYTES: i64 = 64 * 1_024;
const MAX_EXTERNAL_COMMIT_JOB_RUNS: usize = 256;

const MIGRATION_001: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    status TEXT NOT NULL,
    run_id TEXT,
    message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    ttl_expires_at TEXT,
    FOREIGN KEY(session_id) REFERENCES sessions(session_id)
);

CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    status TEXT NOT NULL,
    run_dir TEXT NOT NULL,
    trace_path TEXT NOT NULL,
    task_state_path TEXT,
    report_path TEXT,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    last_event_seq INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(session_id) REFERENCES sessions(session_id),
    FOREIGN KEY(job_id) REFERENCES jobs(job_id)
);

CREATE TABLE IF NOT EXISTS task_states (
    run_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    path TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    goal TEXT NOT NULL,
    step INTEGER NOT NULL,
    summary TEXT,
    modified_at INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES runs(run_id),
    FOREIGN KEY(session_id) REFERENCES sessions(session_id),
    FOREIGN KEY(job_id) REFERENCES jobs(job_id)
);

CREATE TABLE IF NOT EXISTS reports (
    run_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    termination_reason TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS events (
    run_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    event_name TEXT NOT NULL,
    event_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, seq),
    FOREIGN KEY(run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS event_offsets (
    run_id TEXT PRIMARY KEY,
    last_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS pending_approvals (
    call_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    name TEXT NOT NULL,
    args_json TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(job_id) REFERENCES jobs(job_id),
    FOREIGN KEY(run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS pending_inputs (
    input_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    prompt TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(job_id) REFERENCES jobs(job_id),
    FOREIGN KEY(run_id) REFERENCES runs(run_id)
);

CREATE INDEX IF NOT EXISTS idx_task_states_session_modified
    ON task_states(session_id, modified_at DESC, path DESC);
CREATE INDEX IF NOT EXISTS idx_task_states_modified
    ON task_states(modified_at DESC, path DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_session_updated
    ON jobs(session_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_events_run_seq
    ON events(run_id, seq);
"#;

const MIGRATION_002: &str = r#"
CREATE INDEX IF NOT EXISTS idx_runs_job_started
    ON runs(job_id, started_at ASC, run_id ASC);
"#;

const MIGRATION_003: &str = r#"
CREATE TABLE IF NOT EXISTS conversation_messages (
    message_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    idempotency_key TEXT,
    content TEXT NOT NULL,
    requested_delivery TEXT NOT NULL CHECK(requested_delivery IN ('successor', 'current_run')),
    actual_delivery TEXT CHECK(actual_delivery IS NULL OR actual_delivery IN ('successor', 'current_run')),
    status TEXT NOT NULL CHECK(status IN (
        'queued', 'intervention_requested', 'applied_current_run',
        'claimed_successor', 'needs_attention', 'revoked'
    )),
    sequence INTEGER NOT NULL CHECK(sequence >= 1),
    target_run_id TEXT,
    reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(session_id, sequence),
    UNIQUE(session_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_conversation_messages_delivery
    ON conversation_messages(session_id, status, sequence);
"#;

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "runtime_state_index", MIGRATION_001),
    (2, "runs_by_job_index", MIGRATION_002),
    (3, "conversation_messages", MIGRATION_003),
];

#[derive(Debug, Clone)]
pub struct StateIndex {
    db_path: Arc<PathBuf>,
    state_dir: Arc<PathBuf>,
    busy_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStateIndexRecord {
    pub run_id: RunId,
    pub path: PathBuf,
}

/// A short-lived, atomic reservation made immediately before a TUI resume.
///
/// The previous status is retained so a failed run start can release the
/// reservation without overwriting a newer run that may have claimed the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeJobClaim {
    pub job_id: JobId,
    pub previous_status: String,
    pub previous_run_id: Option<RunId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobIndexRecord {
    pub job_id: JobId,
    pub session_id: SessionId,
    pub status: String,
    pub run_id: Option<RunId>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIndexRecord {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub status: String,
    pub run_dir: PathBuf,
    pub trace_path: PathBuf,
    pub task_state_path: Option<PathBuf>,
    pub report_path: Option<PathBuf>,
    pub last_event_seq: u64,
}

/// One bounded, logically read-only view of an exact job/run pair.
///
/// This snapshot is intended for callers that must inspect existing runtime
/// state without creating a missing database, applying migrations, or changing
/// SQLite schema or journal settings. SQLite may still coordinate a WAL reader
/// through an existing `-shm` sidecar; the main database and WAL are never
/// opened for writes. `job_runs_truncated` is set when more matching runs exist
/// than the requested bounded prefix can represent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRunInspectionSnapshot {
    pub job: Option<JobRunInspectionJob>,
    pub run: Option<JobRunInspectionRun>,
    pub job_run_ids: Vec<RunId>,
    pub job_runs_truncated: bool,
    pub task_state_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRunInspectionJob {
    pub job_id: JobId,
    pub session_id: SessionId,
    pub status: String,
    pub run_id: Option<RunId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRunInspectionRun {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub status: String,
    pub run_dir: PathBuf,
    pub task_state_path: Option<PathBuf>,
}

/// A short-lived SQLite write reservation for exact terminal job/run pairs.
///
/// The guard does not mutate runtime rows. Holding the `BEGIN IMMEDIATE`
/// transaction prevents a concurrent run start or resume from changing the
/// validated identities while another local database commits their mapping.
/// Dropping the guard rolls the reservation back, including on process unwind.
pub struct ExternalJobRunCommitGuard {
    connection: Option<Connection>,
}

impl Drop for ExternalJobRunCommitGuard {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.execute_batch("ROLLBACK");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportIndexRecord {
    pub run_id: RunId,
    pub path: PathBuf,
    pub status: String,
    pub termination_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupResult {
    pub job_count: usize,
    pub run_count: usize,
    pub task_state_count: usize,
}

#[derive(Debug, Clone)]
struct ExpiredJobRecord {
    job_id: JobId,
    run_id: Option<RunId>,
    run_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventIndexRecord {
    pub run_id: RunId,
    pub seq: u64,
    pub event_name: String,
    pub event_json: String,
}

/// One bounded, internally consistent read of a run and its indexed events.
///
/// `high_water_seq` and `events` are read in the same SQLite transaction so a
/// transcript projection does not report a false gap while a live run appends.
/// Payload text is materialized only after a byte-bounded prefix is selected.
/// The embedded `run.last_event_seq` remains available to detect a stale run
/// counter left by older interrupted writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEventSnapshot {
    pub run: RunIndexRecord,
    pub high_water_seq: u64,
    pub events: Vec<EventIndexRecord>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventSnapshotMetadata {
    run_id: RunId,
    seq: u64,
    event_name_storage_class: String,
    event_json_storage_class: String,
    event_json_bytes: usize,
}

impl StateIndex {
    pub fn new(state_dir: &Path) -> Self {
        Self::with_path(
            state_dir,
            state_dir.join("state.sqlite"),
            DEFAULT_BUSY_TIMEOUT_MS,
        )
    }

    pub fn with_path(state_dir: &Path, db_path: PathBuf, busy_timeout_ms: u64) -> Self {
        Self {
            db_path: Arc::new(db_path),
            state_dir: Arc::new(state_dir.to_path_buf()),
            busy_timeout_ms,
        }
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn busy_timeout_ms(&self) -> u64 {
        self.busy_timeout_ms
    }

    pub fn initialize(&self) -> std::io::Result<()> {
        let _ = self.connect()?;
        Ok(())
    }

    /// Rebase artifact paths stored in the index after a state directory has
    /// been copied to a new location. Runtime indexes historically stored
    /// absolute paths, so copying the SQLite file alone would leave resume,
    /// report, and run inspection pointed at the legacy directory.
    ///
    /// Only paths below `from_state_dir` are changed. External or malformed
    /// paths are left untouched for the normal boundary validators to reject;
    /// the operation never turns an untrusted database value into a new path.
    pub fn rebase_artifact_paths(
        &self,
        from_state_dir: &Path,
        to_state_dir: &Path,
    ) -> std::io::Result<usize> {
        if !from_state_dir.is_absolute() || !to_state_dir.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "state index rebase roots must be absolute",
            ));
        }

        let mut connection = self.connect_existing_write_guard()?;
        let runtime_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('runs', 'task_states', 'reports')",
                [],
                |row| row.get(0),
            )
            .map_err(io_other)?;
        if runtime_tables != 3 {
            return Ok(0);
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(io_other)?;
        let mut changed = 0usize;

        let run_rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT run_id, run_dir, trace_path, task_state_path, report_path FROM runs",
                )
                .map_err(io_other)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(io_other)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(io_other)?
        };
        for (run_id, run_dir, trace_path, task_state_path, report_path) in run_rows {
            let rebased_run_dir = rebase_index_path(&run_dir, from_state_dir, to_state_dir);
            let rebased_trace_path = rebase_index_path(&trace_path, from_state_dir, to_state_dir);
            let rebased_task_state_path = task_state_path
                .as_deref()
                .and_then(|path| rebase_index_path(path, from_state_dir, to_state_dir));
            let rebased_report_path = report_path
                .as_deref()
                .and_then(|path| rebase_index_path(path, from_state_dir, to_state_dir));
            if rebased_run_dir.is_none()
                && rebased_trace_path.is_none()
                && rebased_task_state_path.is_none()
                && rebased_report_path.is_none()
            {
                continue;
            }
            transaction
                .execute(
                    "UPDATE runs SET run_dir = ?2, trace_path = ?3, task_state_path = ?4, report_path = ?5 WHERE run_id = ?1",
                    params![
                        run_id,
                        rebased_run_dir.unwrap_or(run_dir),
                        rebased_trace_path.unwrap_or(trace_path),
                        rebased_task_state_path.or(task_state_path),
                        rebased_report_path.or(report_path),
                    ],
                )
                .map_err(io_other)?;
            changed += 1;
        }

        let task_state_rows = {
            let mut statement = transaction
                .prepare("SELECT run_id, path FROM task_states")
                .map_err(io_other)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(io_other)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(io_other)?
        };
        for (run_id, path) in task_state_rows {
            let Some(rebased) = rebase_index_path(&path, from_state_dir, to_state_dir) else {
                continue;
            };
            transaction
                .execute(
                    "UPDATE task_states SET path = ?2 WHERE run_id = ?1",
                    params![run_id, rebased],
                )
                .map_err(io_other)?;
            changed += 1;
        }

        let report_rows = {
            let mut statement = transaction
                .prepare("SELECT run_id, path FROM reports")
                .map_err(io_other)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(io_other)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(io_other)?
        };
        for (run_id, path) in report_rows {
            let Some(rebased) = rebase_index_path(&path, from_state_dir, to_state_dir) else {
                continue;
            };
            transaction
                .execute(
                    "UPDATE reports SET path = ?2 WHERE run_id = ?1",
                    params![run_id, rebased],
                )
                .map_err(io_other)?;
            changed += 1;
        }

        transaction.commit().map_err(io_other)?;
        Ok(changed)
    }

    pub fn record_run_started(
        &self,
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        run_dir: &Path,
        trace_path: &Path,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        upsert_session(&conn, session_id, &now)?;
        conn.execute(
            r#"
            INSERT INTO jobs(job_id, session_id, status, run_id, created_at, updated_at)
            VALUES (?1, ?2, 'running', ?3, ?4, ?4)
            ON CONFLICT(job_id) DO UPDATE SET
                session_id = excluded.session_id,
                status = excluded.status,
                run_id = excluded.run_id,
                updated_at = excluded.updated_at
            "#,
            params![
                job_id.to_string(),
                session_id.to_string(),
                run_id.to_string(),
                now
            ],
        )
        .map_err(io_other)?;
        conn.execute(
            r#"
            INSERT INTO runs(
                run_id, session_id, job_id, status, run_dir, trace_path, started_at, updated_at
            )
            VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?6)
            ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                job_id = excluded.job_id,
                status = excluded.status,
                run_dir = excluded.run_dir,
                trace_path = excluded.trace_path,
                updated_at = excluded.updated_at
            "#,
            params![
                run_id.to_string(),
                session_id.to_string(),
                job_id.to_string(),
                run_dir.to_string_lossy().as_ref(),
                trace_path.to_string_lossy().as_ref(),
                now,
            ],
        )
        .map_err(io_other)?;
        Ok(())
    }

    pub async fn record_task_state_async(
        &self,
        state: TaskState,
        path: PathBuf,
        modified: SystemTime,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.record_task_state(&state, &path, modified))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn record_task_state(
        &self,
        state: &TaskState,
        path: &Path,
        modified: SystemTime,
    ) -> std::io::Result<()> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(io_other)?;
        let now = now_rfc3339();
        let modified_millis = system_time_millis(modified);
        upsert_session(&tx, state.session_id, &now)?;

        // Artifact repair may replay old snapshots concurrently with live reads.
        // Advance the mutable job pointer only when this snapshot is not older.
        let current_run_id = tx
            .query_row(
                "SELECT run_id FROM jobs WHERE job_id = ?1",
                params![state.job_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(io_other)?
            .flatten();
        let incoming_run_id = state.run_id.to_string();
        let current_modified = current_run_id
            .as_deref()
            .map(|run_id| {
                tx.query_row(
                    "SELECT modified_at FROM task_states WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(io_other)
            })
            .transpose()?
            .flatten();
        let advances_job = match (current_run_id.as_deref(), current_modified) {
            (None, _) => true,
            (Some(current), None) => current == incoming_run_id,
            (Some(current), Some(current_modified)) if current == incoming_run_id => {
                modified_millis >= current_modified
            }
            (Some(current), Some(current_modified)) => {
                modified_millis > current_modified
                    || (modified_millis == current_modified && incoming_run_id.as_str() > current)
            }
        };

        tx.execute(
            r#"
            INSERT INTO jobs(job_id, session_id, status, run_id, message, created_at, updated_at)
            VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?5)
            ON CONFLICT(job_id) DO UPDATE SET
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                message = COALESCE(excluded.message, jobs.message),
                updated_at = excluded.updated_at
            WHERE ?6
            "#,
            params![
                state.job_id.to_string(),
                state.session_id.to_string(),
                state.run_id.to_string(),
                state.goal,
                now,
                advances_job,
            ],
        )
        .map_err(io_other)?;
        let run_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.state_dir.join("runs").join(state.run_id.to_string()));
        let trace_path = run_dir.join("trace.jsonl");
        tx.execute(
            r#"
            INSERT INTO runs(
                run_id, session_id, job_id, status, run_dir, trace_path, task_state_path,
                started_at, updated_at
            )
            VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                job_id = excluded.job_id,
                task_state_path = excluded.task_state_path,
                updated_at = excluded.updated_at
            "#,
            params![
                state.run_id.to_string(),
                state.session_id.to_string(),
                state.job_id.to_string(),
                run_dir.to_string_lossy().as_ref(),
                trace_path.to_string_lossy().as_ref(),
                path.to_string_lossy().as_ref(),
                now,
            ],
        )
        .map_err(io_other)?;
        tx.execute(
            r#"
            INSERT INTO task_states(
                run_id, session_id, job_id, path, schema_version, goal, step, summary, modified_at,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                job_id = excluded.job_id,
                path = excluded.path,
                schema_version = excluded.schema_version,
                goal = excluded.goal,
                step = excluded.step,
                summary = excluded.summary,
                modified_at = excluded.modified_at,
                updated_at = excluded.updated_at
            "#,
            params![
                state.run_id.to_string(),
                state.session_id.to_string(),
                state.job_id.to_string(),
                path.to_string_lossy().as_ref(),
                state.schema_version,
                state.goal,
                state.step,
                state.summary,
                modified_millis,
                now,
            ],
        )
        .map_err(io_other)?;
        tx.commit().map_err(io_other)?;
        Ok(())
    }

    pub async fn list_task_state_records_async(
        &self,
        session_id: Option<SessionId>,
    ) -> std::io::Result<Vec<TaskStateIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.list_task_state_records(session_id))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn list_task_state_records(
        &self,
        session_id: Option<SessionId>,
    ) -> std::io::Result<Vec<TaskStateIndexRecord>> {
        let conn = self.connect()?;
        let mut records = Vec::new();
        if let Some(session_id) = session_id {
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT run_id, path
                    FROM task_states
                    WHERE session_id = ?1
                    ORDER BY modified_at DESC, path DESC
                    "#,
                )
                .map_err(io_other)?;
            let rows = stmt
                .query_map(params![session_id.to_string()], task_state_record_from_row)
                .map_err(io_other)?;
            for row in rows {
                records.push(row.map_err(io_other)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT run_id, path
                    FROM task_states
                    ORDER BY modified_at DESC, path DESC
                    "#,
                )
                .map_err(io_other)?;
            let rows = stmt
                .query_map([], task_state_record_from_row)
                .map_err(io_other)?;
            for row in rows {
                records.push(row.map_err(io_other)?);
            }
        }
        Ok(records)
    }

    /// Return only snapshots whose owning job is no longer active.
    ///
    /// This query is intentionally bounded at the index layer. Callers that
    /// present a picker must not deserialize every historical artifact before
    /// applying a UI-level limit.
    pub async fn list_resumable_task_state_records_async(
        &self,
        limit: usize,
    ) -> std::io::Result<Vec<TaskStateIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.list_resumable_task_state_records(limit))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn list_resumable_task_state_records(
        &self,
        limit: usize,
    ) -> std::io::Result<Vec<TaskStateIndexRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect()?;
        let limit = limit.min(200) as i64;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT task_states.run_id, task_states.path
                FROM task_states
                INNER JOIN jobs
                    ON jobs.job_id = task_states.job_id
                   AND jobs.session_id = task_states.session_id
                INNER JOIN runs
                    ON runs.run_id = task_states.run_id
                   AND runs.job_id = task_states.job_id
                   AND runs.session_id = task_states.session_id
                WHERE jobs.status IN ('done', 'error', 'cancelled', 'interrupted')
                  AND runs.status IN ('done', 'error', 'cancelled', 'interrupted')
                  AND jobs.run_id = task_states.run_id
                ORDER BY task_states.modified_at DESC, task_states.path DESC
                LIMIT ?1
                "#,
            )
            .map_err(io_other)?;
        let rows = stmt
            .query_map(params![limit], task_state_record_from_row)
            .map_err(io_other)?;
        rows.map(|row| row.map_err(io_other)).collect()
    }

    /// Atomically reserve a terminal job for a resume attempt.
    pub async fn claim_job_for_resume_async(
        &self,
        job_id: JobId,
        expected_run_id: RunId,
    ) -> std::io::Result<Option<ResumeJobClaim>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.claim_job_for_resume(job_id, expected_run_id))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn claim_job_for_resume(
        &self,
        job_id: JobId,
        expected_run_id: RunId,
    ) -> std::io::Result<Option<ResumeJobClaim>> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(io_other)?;
        let current = tx
            .query_row(
                r#"
                SELECT jobs.status, jobs.run_id, runs.status
                FROM jobs
                LEFT JOIN runs
                  ON runs.run_id = jobs.run_id
                 AND runs.job_id = jobs.job_id
                WHERE jobs.job_id = ?1
                "#,
                params![job_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?
                            .map(|value| parse_run_id_at(1, value))
                            .transpose()?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(io_other)?;
        let Some((previous_status, previous_run_id, run_status)) = current else {
            tx.commit().map_err(io_other)?;
            return Ok(None);
        };
        if !matches!(
            previous_status.as_str(),
            "done" | "error" | "cancelled" | "interrupted"
        ) || !run_status.is_some_and(|status| {
            matches!(
                status.as_str(),
                "done" | "error" | "cancelled" | "interrupted"
            )
        }) || previous_run_id != Some(expected_run_id)
        {
            tx.commit().map_err(io_other)?;
            return Ok(None);
        }
        let updated = tx
            .execute(
                r#"
                UPDATE jobs
                SET status = 'running', updated_at = ?2
                WHERE job_id = ?1
                  AND status = ?3
                  AND run_id = ?4
                  AND EXISTS (
                      SELECT 1
                      FROM runs
                      WHERE runs.run_id = ?4
                        AND runs.job_id = ?1
                        AND runs.status IN ('done', 'error', 'cancelled', 'interrupted')
                  )
                "#,
                params![
                    job_id.to_string(),
                    now_rfc3339(),
                    previous_status,
                    expected_run_id.to_string(),
                ],
            )
            .map_err(io_other)?;
        if updated != 1 {
            tx.commit().map_err(io_other)?;
            return Ok(None);
        }
        tx.commit().map_err(io_other)?;
        Ok(Some(ResumeJobClaim {
            job_id,
            previous_status,
            previous_run_id,
        }))
    }

    /// Release a claim only if no newer run has taken ownership of the job.
    pub async fn release_job_resume_claim_async(
        &self,
        claim: ResumeJobClaim,
    ) -> std::io::Result<bool> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.release_job_resume_claim(&claim))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn release_job_resume_claim(&self, claim: &ResumeJobClaim) -> std::io::Result<bool> {
        let conn = self.connect()?;
        let updated = conn
            .execute(
                r#"
                UPDATE jobs
                SET status = ?2, updated_at = ?3
                WHERE job_id = ?1
                  AND status = 'running'
                  AND (run_id IS ?4 OR run_id = ?4)
                "#,
                params![
                    claim.job_id.to_string(),
                    claim.previous_status,
                    now_rfc3339(),
                    claim.previous_run_id.map(|id| id.to_string()),
                ],
            )
            .map_err(io_other)?;
        Ok(updated == 1)
    }

    pub async fn list_run_records_async(
        &self,
        limit: usize,
    ) -> std::io::Result<Vec<RunIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.list_run_records(limit))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn list_run_records(&self, limit: usize) -> std::io::Result<Vec<RunIndexRecord>> {
        let conn = self.connect()?;
        let limit = limit.clamp(1, 200) as i64;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT run_id, session_id, job_id, status, run_dir, trace_path,
                       task_state_path, report_path, last_event_seq
                FROM runs
                ORDER BY updated_at DESC, started_at DESC, run_id DESC
                LIMIT ?1
                "#,
            )
            .map_err(io_other)?;
        let rows = stmt
            .query_map(params![limit], run_record_from_row)
            .map_err(io_other)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(io_other)?);
        }
        Ok(records)
    }

    pub async fn run_records_for_job_async(
        &self,
        job_id: JobId,
        limit: usize,
    ) -> std::io::Result<Vec<RunIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.run_records_for_job(job_id, limit))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn run_records_for_job(
        &self,
        job_id: JobId,
        limit: usize,
    ) -> std::io::Result<Vec<RunIndexRecord>> {
        const MAX_JOB_RUNS: usize = 2_000;

        let conn = self.connect()?;
        let limit = limit.clamp(1, MAX_JOB_RUNS) as i64;
        let mut statement = conn
            .prepare(
                r#"
                SELECT run_id, session_id, job_id, status, run_dir, trace_path,
                       task_state_path, report_path, last_event_seq
                FROM runs
                WHERE job_id = ?1
                ORDER BY started_at ASC, run_id ASC
                LIMIT ?2
                "#,
            )
            .map_err(io_other)?;
        let rows = statement
            .query_map(params![job_id.to_string(), limit], run_record_from_row)
            .map_err(io_other)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(io_other)?);
        }
        Ok(records)
    }

    pub async fn inspect_job_run_read_only_async(
        &self,
        job_id: JobId,
        run_id: RunId,
        run_limit: usize,
    ) -> std::io::Result<JobRunInspectionSnapshot> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || {
            index.inspect_job_run_read_only(job_id, run_id, run_limit)
        })
        .await
        .map_err(std::io::Error::other)?
    }

    /// Reserve one runtime database while an external local store commits
    /// mappings for the supplied exact job/run pairs.
    ///
    /// Eligibility is evaluated after the write reservation is acquired and
    /// preserves input order. A pair is eligible only when it is still the
    /// job's latest terminal run, the run is terminal, both identities share a
    /// session, and an indexed bounded lookup finds no other run for the job.
    pub async fn guard_job_runs_for_external_commit_async(
        &self,
        expected: Vec<(JobId, RunId)>,
    ) -> std::io::Result<(Vec<bool>, ExternalJobRunCommitGuard)> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.guard_job_runs_for_external_commit(&expected))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn guard_job_runs_for_external_commit(
        &self,
        expected: &[(JobId, RunId)],
    ) -> std::io::Result<(Vec<bool>, ExternalJobRunCommitGuard)> {
        if expected.is_empty() || expected.len() > MAX_EXTERNAL_COMMIT_JOB_RUNS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "external commit guard requires a bounded non-empty job/run set",
            ));
        }

        let connection = self.connect_existing_write_guard()?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(io_other)?;
        ensure_current_schema_for_read_only_inspection(&connection)?;
        ensure_runs_by_job_index(&connection)?;
        let eligibility = expected
            .iter()
            .map(|(job_id, run_id)| {
                connection
                    .query_row(
                        r#"
                        SELECT EXISTS (
                            SELECT 1
                            FROM jobs
                            JOIN runs
                              ON runs.run_id = jobs.run_id
                             AND runs.job_id = jobs.job_id
                             AND runs.session_id = jobs.session_id
                            WHERE jobs.job_id = ?1
                              AND jobs.run_id = ?2
                              AND jobs.status IN ('done', 'error', 'cancelled', 'interrupted')
                              AND runs.status IN ('done', 'error', 'cancelled', 'interrupted')
                              AND NOT EXISTS (
                                  SELECT 1
                                  FROM runs AS job_runs INDEXED BY idx_runs_job_started
                                  WHERE job_runs.job_id = jobs.job_id
                                    AND job_runs.run_id <> jobs.run_id
                                  LIMIT 1
                              )
                        )
                        "#,
                        params![job_id.to_string(), run_id.to_string()],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(io_other)
            })
            .collect::<std::io::Result<Vec<_>>>()?;

        Ok((
            eligibility,
            ExternalJobRunCommitGuard {
                connection: Some(connection),
            },
        ))
    }

    pub fn inspect_job_run_read_only(
        &self,
        job_id: JobId,
        run_id: RunId,
        run_limit: usize,
    ) -> std::io::Result<JobRunInspectionSnapshot> {
        const MAX_INSPECTION_JOB_RUNS: usize = 2_000;

        let mut connection = self.connect_read_only()?;
        let transaction = connection.transaction().map_err(io_other)?;
        ensure_current_schema_for_read_only_inspection(&transaction)?;
        ensure_runs_by_job_index(&transaction)?;
        let job_metadata = transaction
            .query_row(
                r#"
                SELECT typeof(job_id), length(CAST(job_id AS BLOB)),
                       typeof(session_id), length(CAST(session_id AS BLOB)),
                       typeof(status), length(CAST(status AS BLOB)),
                       typeof(run_id), length(CAST(run_id AS BLOB))
                FROM jobs
                WHERE job_id = ?1
                "#,
                params![job_id.to_string()],
                |row| inspection_metadata_from_row(row, 4),
            )
            .optional()
            .map_err(io_other)?;
        if let Some(metadata) = job_metadata.as_ref() {
            validate_inspection_metadata(
                metadata,
                &[
                    ("jobs.job_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false),
                    ("jobs.session_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false),
                    ("jobs.status", MAX_INSPECTION_STATUS_BYTES, false),
                    ("jobs.run_id", MAX_INSPECTION_RUNTIME_ID_BYTES, true),
                ],
            )?;
        }
        let job = if job_metadata.is_some() {
            transaction
                .query_row(
                    r#"
                    SELECT job_id, session_id, status, run_id
                    FROM jobs
                    WHERE job_id = ?1
                    "#,
                    params![job_id.to_string()],
                    inspection_job_from_row,
                )
                .optional()
                .map_err(io_other)?
        } else {
            None
        };

        let run_metadata = transaction
            .query_row(
                r#"
                SELECT typeof(run_id), length(CAST(run_id AS BLOB)),
                       typeof(session_id), length(CAST(session_id AS BLOB)),
                       typeof(job_id), length(CAST(job_id AS BLOB)),
                       typeof(status), length(CAST(status AS BLOB)),
                       typeof(run_dir), length(CAST(run_dir AS BLOB)),
                       typeof(task_state_path), length(CAST(task_state_path AS BLOB))
                FROM runs
                WHERE run_id = ?1
                "#,
                params![run_id.to_string()],
                |row| inspection_metadata_from_row(row, 6),
            )
            .optional()
            .map_err(io_other)?;
        if let Some(metadata) = run_metadata.as_ref() {
            validate_inspection_metadata(
                metadata,
                &[
                    ("runs.run_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false),
                    ("runs.session_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false),
                    ("runs.job_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false),
                    ("runs.status", MAX_INSPECTION_STATUS_BYTES, false),
                    ("runs.run_dir", MAX_INSPECTION_PATH_BYTES, false),
                    ("runs.task_state_path", MAX_INSPECTION_PATH_BYTES, true),
                ],
            )?;
        }
        let run = if run_metadata.is_some() {
            transaction
                .query_row(
                    r#"
                    SELECT run_id, session_id, job_id, status, run_dir, task_state_path
                    FROM runs
                    WHERE run_id = ?1
                    "#,
                    params![run_id.to_string()],
                    inspection_run_from_row,
                )
                .optional()
                .map_err(io_other)?
        } else {
            None
        };

        let task_state_path_metadata = transaction
            .query_row(
                r#"
                SELECT typeof(path), length(CAST(path AS BLOB))
                FROM task_states
                WHERE run_id = ?1
                "#,
                params![run_id.to_string()],
                |row| inspection_metadata_from_row(row, 1),
            )
            .optional()
            .map_err(io_other)?;
        if let Some(metadata) = task_state_path_metadata.as_ref() {
            validate_inspection_metadata(
                metadata,
                &[("task_states.path", MAX_INSPECTION_PATH_BYTES, false)],
            )?;
        }
        let task_state_path = if task_state_path_metadata.is_some() {
            transaction
                .query_row(
                    "SELECT path FROM task_states WHERE run_id = ?1",
                    params![run_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map(|path| path.map(PathBuf::from))
                .map_err(io_other)?
        } else {
            None
        };

        let bounded_limit = run_limit.clamp(1, MAX_INSPECTION_JOB_RUNS);
        let query_limit = i64::try_from(bounded_limit.saturating_add(1))
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let job_run_metadata = {
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT typeof(run_id), length(CAST(run_id AS BLOB))
                    FROM runs INDEXED BY idx_runs_job_started
                    WHERE job_id = ?1
                    ORDER BY started_at ASC, run_id ASC
                    LIMIT ?2
                    "#,
                )
                .map_err(io_other)?;
            let rows = statement
                .query_map(params![job_id.to_string(), query_limit], |row| {
                    inspection_metadata_from_row(row, 1)
                })
                .map_err(io_other)?;
            let mut records = Vec::with_capacity(bounded_limit.saturating_add(1));
            for row in rows {
                records.push(row.map_err(io_other)?);
            }
            records
        };
        for metadata in &job_run_metadata {
            validate_inspection_metadata(
                metadata,
                &[("runs.run_id", MAX_INSPECTION_RUNTIME_ID_BYTES, false)],
            )?;
        }
        let mut job_run_ids = {
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT run_id
                    FROM runs INDEXED BY idx_runs_job_started
                    WHERE job_id = ?1
                    ORDER BY started_at ASC, run_id ASC
                    LIMIT ?2
                    "#,
                )
                .map_err(io_other)?;
            let rows = statement
                .query_map(params![job_id.to_string(), query_limit], |row| {
                    run_id_from_row(row, 0)
                })
                .map_err(io_other)?;
            let mut records = Vec::with_capacity(bounded_limit.saturating_add(1));
            for row in rows {
                records.push(row.map_err(io_other)?);
            }
            records
        };
        let job_runs_truncated = job_run_ids.len() > bounded_limit;
        job_run_ids.truncate(bounded_limit);
        transaction.commit().map_err(io_other)?;
        Ok(JobRunInspectionSnapshot {
            job,
            run,
            job_run_ids,
            job_runs_truncated,
            task_state_path,
        })
    }

    pub async fn task_state_path_async(&self, run_id: RunId) -> std::io::Result<Option<PathBuf>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.task_state_path(run_id))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn task_state_path(&self, run_id: RunId) -> std::io::Result<Option<PathBuf>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT path FROM task_states WHERE run_id = ?1",
            params![run_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|path| path.map(PathBuf::from))
        .map_err(io_other)
    }

    pub fn job_record(&self, job_id: JobId) -> std::io::Result<Option<JobIndexRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT job_id, session_id, status, run_id, message
            FROM jobs
            WHERE job_id = ?1
            "#,
            params![job_id.to_string()],
            job_record_from_row,
        )
        .optional()
        .map_err(io_other)
    }

    pub async fn job_record_async(&self, job_id: JobId) -> std::io::Result<Option<JobIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.job_record(job_id))
            .await
            .map_err(std::io::Error::other)?
    }

    pub async fn event_records_async(
        &self,
        run_id: RunId,
    ) -> std::io::Result<Vec<EventIndexRecord>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.event_records(run_id))
            .await
            .map_err(std::io::Error::other)?
    }

    pub async fn run_event_snapshot_async(
        &self,
        run_id: RunId,
        after_seq: u64,
        limit: usize,
    ) -> std::io::Result<Option<RunEventSnapshot>> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.run_event_snapshot(run_id, after_seq, limit))
            .await
            .map_err(std::io::Error::other)?
    }

    pub async fn mark_running_jobs_interrupted_async(&self) -> std::io::Result<usize> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.mark_running_jobs_interrupted())
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn set_job_ttl(
        &self,
        job_id: JobId,
        ttl_expires_at: Option<String>,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        conn.execute(
            r#"
            UPDATE jobs
            SET ttl_expires_at = ?2, updated_at = ?3
            WHERE job_id = ?1
            "#,
            params![job_id.to_string(), ttl_expires_at, now],
        )
        .map_err(io_other)?;
        Ok(())
    }

    pub async fn record_pending_approval_async(
        &self,
        call_id: CallId,
        job_id: JobId,
        run_id: RunId,
        name: String,
        args_json: String,
        reason: String,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || {
            index.record_pending_approval(call_id, job_id, run_id, &name, &args_json, &reason)
        })
        .await
        .map_err(std::io::Error::other)?
    }

    pub fn record_pending_approval(
        &self,
        call_id: CallId,
        job_id: JobId,
        run_id: RunId,
        name: &str,
        args_json: &str,
        reason: &str,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        conn.execute(
            r#"
            INSERT INTO pending_approvals(
                call_id, job_id, run_id, name, args_json, reason, status, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?7)
            ON CONFLICT(call_id) DO UPDATE SET
                job_id = excluded.job_id,
                run_id = excluded.run_id,
                name = excluded.name,
                args_json = excluded.args_json,
                reason = excluded.reason,
                status = 'pending',
                updated_at = excluded.updated_at
            "#,
            params![
                call_id.to_string(),
                job_id.to_string(),
                run_id.to_string(),
                name,
                args_json,
                reason,
                now,
            ],
        )
        .map_err(io_other)?;
        Ok(())
    }

    pub async fn record_pending_input_async(
        &self,
        input_id: CallId,
        job_id: JobId,
        run_id: RunId,
        prompt: String,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || {
            index.record_pending_input(input_id, job_id, run_id, &prompt)
        })
        .await
        .map_err(std::io::Error::other)?
    }

    pub fn record_pending_input(
        &self,
        input_id: CallId,
        job_id: JobId,
        run_id: RunId,
        prompt: &str,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        conn.execute(
            r#"
            INSERT INTO pending_inputs(
                input_id, job_id, run_id, prompt, status, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5)
            ON CONFLICT(input_id) DO UPDATE SET
                job_id = excluded.job_id,
                run_id = excluded.run_id,
                prompt = excluded.prompt,
                status = 'pending',
                updated_at = excluded.updated_at
            "#,
            params![
                input_id.to_string(),
                job_id.to_string(),
                run_id.to_string(),
                prompt,
                now,
            ],
        )
        .map_err(io_other)?;
        Ok(())
    }

    pub async fn mark_pending_approval_status_async(
        &self,
        call_id: CallId,
        status: String,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.mark_pending_approval_status(call_id, &status))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn mark_pending_approval_status(
        &self,
        call_id: CallId,
        status: &str,
    ) -> std::io::Result<()> {
        self.mark_pending_status("pending_approvals", "call_id", call_id, status)
    }

    pub async fn mark_pending_input_status_async(
        &self,
        input_id: CallId,
        status: String,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.mark_pending_input_status(input_id, &status))
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn mark_pending_input_status(&self, input_id: CallId, status: &str) -> std::io::Result<()> {
        self.mark_pending_status("pending_inputs", "input_id", input_id, status)
    }

    pub fn pending_approval_status(&self, call_id: CallId) -> std::io::Result<Option<String>> {
        self.pending_status("pending_approvals", "call_id", call_id)
    }

    pub fn pending_input_status(&self, input_id: CallId) -> std::io::Result<Option<String>> {
        self.pending_status("pending_inputs", "input_id", input_id)
    }

    pub async fn cleanup_expired_async(&self) -> std::io::Result<CleanupResult> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || index.cleanup_expired())
            .await
            .map_err(std::io::Error::other)?
    }

    pub fn cleanup_expired(&self) -> std::io::Result<CleanupResult> {
        let mut conn = self.connect()?;
        let now = Utc::now().with_timezone(&FixedOffset::east_opt(0).expect("zero offset"));
        let expired_jobs = {
            let tx = conn.transaction().map_err(io_other)?;
            let records = select_expired_jobs(&tx, now)?;
            delete_expired_jobs(&tx, &records)?;
            tx.commit().map_err(io_other)?;
            records
        };

        for record in &expired_jobs {
            if let Some(run_dir) = record.run_dir.as_ref() {
                remove_run_dir_if_safe(&self.state_dir, run_dir)?;
            }
        }

        Ok(CleanupResult {
            job_count: expired_jobs.len(),
            run_count: expired_jobs
                .iter()
                .filter(|record| record.run_id.is_some())
                .count(),
            task_state_count: expired_jobs
                .iter()
                .filter(|record| record.run_id.is_some())
                .count(),
        })
    }

    pub fn mark_running_jobs_interrupted(&self) -> std::io::Result<usize> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        let jobs = conn
            .execute(
                r#"
                UPDATE jobs
                SET status = 'interrupted', updated_at = ?1
                WHERE status IN ('init', 'running')
                "#,
                params![now],
            )
            .map_err(io_other)?;
        conn.execute(
            r#"
            UPDATE runs
            SET status = 'interrupted', completed_at = ?1, updated_at = ?1
            WHERE status IN ('init', 'running')
            "#,
            params![now],
        )
        .map_err(io_other)?;
        conn.execute(
            r#"
            UPDATE pending_approvals
            SET status = 'interrupted', updated_at = ?1
            WHERE status = 'pending'
                AND job_id IN (SELECT job_id FROM jobs WHERE status = 'interrupted')
            "#,
            params![now],
        )
        .map_err(io_other)?;
        conn.execute(
            r#"
            UPDATE pending_inputs
            SET status = 'interrupted', updated_at = ?1
            WHERE status = 'pending'
                AND job_id IN (SELECT job_id FROM jobs WHERE status = 'interrupted')
            "#,
            params![now],
        )
        .map_err(io_other)?;
        Ok(jobs)
    }

    pub fn run_record(&self, run_id: RunId) -> std::io::Result<Option<RunIndexRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT run_id, session_id, job_id, status, run_dir, trace_path, task_state_path,
                   report_path, last_event_seq
            FROM runs
            WHERE run_id = ?1
            "#,
            params![run_id.to_string()],
            run_record_from_row,
        )
        .optional()
        .map_err(io_other)
    }

    pub fn report_record(&self, run_id: RunId) -> std::io::Result<Option<ReportIndexRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT run_id, path, status, termination_reason
            FROM reports
            WHERE run_id = ?1
            "#,
            params![run_id.to_string()],
            report_record_from_row,
        )
        .optional()
        .map_err(io_other)
    }

    pub fn event_records(&self, run_id: RunId) -> std::io::Result<Vec<EventIndexRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT run_id, seq, event_name, event_json
                FROM events
                WHERE run_id = ?1
                ORDER BY seq ASC
                "#,
            )
            .map_err(io_other)?;
        let rows = stmt
            .query_map(params![run_id.to_string()], event_record_from_row)
            .map_err(io_other)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(io_other)?);
        }
        Ok(records)
    }

    pub fn run_event_snapshot(
        &self,
        run_id: RunId,
        after_seq: u64,
        limit: usize,
    ) -> std::io::Result<Option<RunEventSnapshot>> {
        let mut conn = self.connect()?;
        let transaction = conn.transaction().map_err(io_other)?;
        let run = transaction
            .query_row(
                r#"
                SELECT run_id, session_id, job_id, status, run_dir, trace_path,
                       task_state_path, report_path, last_event_seq
                FROM runs
                WHERE run_id = ?1
                "#,
                params![run_id.to_string()],
                run_record_from_row,
            )
            .optional()
            .map_err(io_other)?;
        let Some(run) = run else {
            transaction.commit().map_err(io_other)?;
            return Ok(None);
        };

        let indexed_high_water_sql: Option<i64> = transaction
            .query_row(
                "SELECT MAX(seq) FROM events WHERE run_id = ?1",
                params![run_id.to_string()],
                |row| row.get(0),
            )
            .map_err(io_other)?;
        let indexed_high_water = indexed_high_water_sql
            .map(|value| {
                u64::try_from(value).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "event sequence high-water mark is negative",
                    )
                })
            })
            .transpose()?
            .unwrap_or_default();
        let high_water_seq = run.last_event_seq.max(indexed_high_water);
        let bounded_limit = limit.clamp(1, MAX_SNAPSHOT_EVENTS);
        let query_limit = i64::try_from(bounded_limit + 1)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let after_seq_sql = i64::try_from(after_seq)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let high_water_sql = i64::try_from(high_water_seq)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let metadata = {
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT run_id, seq, typeof(event_name), typeof(event_json),
                           length(CAST(event_json AS BLOB))
                    FROM events
                    WHERE run_id = ?1 AND seq > ?2 AND seq <= ?3
                    ORDER BY seq ASC
                    LIMIT ?4
                    "#,
                )
                .map_err(io_other)?;
            let rows = statement
                .query_map(
                    params![
                        run_id.to_string(),
                        after_seq_sql,
                        high_water_sql,
                        query_limit
                    ],
                    event_snapshot_metadata_from_row,
                )
                .map_err(io_other)?;
            let mut metadata = Vec::new();
            for row in rows {
                metadata.push(row.map_err(io_other)?);
            }
            metadata
        };
        if metadata.iter().any(|record| {
            record.run_id != run_id
                || record.event_name_storage_class != "text"
                || record.event_json_storage_class != "text"
        }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "event snapshot contains an invalid identity or column storage class",
            ));
        }

        let prefix_len = bounded_snapshot_prefix_len(&metadata, bounded_limit);
        let has_more = metadata.len() > prefix_len;
        let records = if prefix_len == 0 {
            Vec::new()
        } else {
            let payload_limit = i64::try_from(prefix_len)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
            let mut statement = transaction
                .prepare(
                    r#"
                    SELECT run_id, seq, event_name, event_json
                    FROM events
                    WHERE run_id = ?1 AND seq > ?2 AND seq <= ?3
                    ORDER BY seq ASC
                    LIMIT ?4
                    "#,
                )
                .map_err(io_other)?;
            let rows = statement
                .query_map(
                    params![
                        run_id.to_string(),
                        after_seq_sql,
                        high_water_sql,
                        payload_limit
                    ],
                    event_record_from_row,
                )
                .map_err(io_other)?;
            let mut records = Vec::with_capacity(prefix_len);
            for row in rows {
                records.push(row.map_err(io_other)?);
            }
            records
        };
        if records.len() != prefix_len
            || records.iter().zip(metadata.iter()).any(|(record, size)| {
                record.run_id != size.run_id
                    || record.seq != size.seq
                    || record.event_json.len() != size.event_json_bytes
            })
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "event snapshot payload changed during its read transaction",
            ));
        }
        transaction.commit().map_err(io_other)?;

        Ok(Some(RunEventSnapshot {
            run,
            high_water_seq,
            events: records,
            has_more,
        }))
    }

    pub fn last_event_seq(&self, run_id: RunId) -> std::io::Result<u64> {
        let conn = self.connect()?;
        let seq: Option<i64> = conn
            .query_row(
                "SELECT last_seq FROM event_offsets WHERE run_id = ?1",
                params![run_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(io_other)?;
        Ok(seq.unwrap_or_default().max(0) as u64)
    }

    pub fn append_event(
        &self,
        run_id: RunId,
        seq: u64,
        event: &StreamEvent,
        event_json: &str,
    ) -> std::io::Result<()> {
        let mut conn = self.connect()?;
        let transaction = conn.transaction().map_err(io_other)?;
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
            INSERT OR IGNORE INTO events(run_id, seq, event_name, event_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
                params![
                    run_id.to_string(),
                    seq as i64,
                    event.event_name(),
                    event_json,
                    now,
                ],
            )
            .map_err(io_other)?;
        transaction
            .execute(
                r#"
            INSERT INTO event_offsets(run_id, last_seq, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(run_id) DO UPDATE SET
                last_seq = MAX(last_seq, excluded.last_seq),
                updated_at = excluded.updated_at
            "#,
                params![run_id.to_string(), seq as i64, now],
            )
            .map_err(io_other)?;
        transaction
            .execute(
                "UPDATE runs SET last_event_seq = MAX(last_event_seq, ?2), updated_at = ?3 WHERE run_id = ?1",
                params![run_id.to_string(), seq as i64, now],
            )
            .map_err(io_other)?;
        transaction.commit().map_err(io_other)?;
        Ok(())
    }

    /// Advance the durable sequence high-water mark without inserting an
    /// event row. Trace history lines (Phase 2) consume the run's monotonic
    /// sequence space but never project into SSE/transcript replays, so only
    /// `event_offsets`/`runs.last_event_seq` must move forward to keep a
    /// restarted writer from reusing a written sequence number.
    pub fn advance_event_seq(&self, run_id: RunId, seq: u64) -> std::io::Result<()> {
        let mut conn = self.connect()?;
        let transaction = conn.transaction().map_err(io_other)?;
        let now = now_rfc3339();
        transaction
            .execute(
                r#"
            INSERT INTO event_offsets(run_id, last_seq, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(run_id) DO UPDATE SET
                last_seq = MAX(last_seq, excluded.last_seq),
                updated_at = excluded.updated_at
            "#,
                params![run_id.to_string(), seq as i64, now],
            )
            .map_err(io_other)?;
        transaction
            .execute(
                "UPDATE runs SET last_event_seq = MAX(last_event_seq, ?2), updated_at = ?3 WHERE run_id = ?1",
                params![run_id.to_string(), seq as i64, now],
            )
            .map_err(io_other)?;
        transaction.commit().map_err(io_other)?;
        Ok(())
    }

    pub async fn record_report_async(
        &self,
        run_id: RunId,
        path: PathBuf,
        status: String,
        termination_reason: String,
    ) -> std::io::Result<()> {
        let index = self.clone();
        tokio::task::spawn_blocking(move || {
            index.record_report(run_id, &path, &status, &termination_reason)
        })
        .await
        .map_err(std::io::Error::other)?
    }

    pub fn record_report(
        &self,
        run_id: RunId,
        path: &Path,
        status: &str,
        termination_reason: &str,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        let run_status = match status {
            "success" => "done",
            "incomplete" => "done",
            "cancelled" => "cancelled",
            "error" => "error",
            _ => "interrupted",
        };
        conn.execute(
            r#"
            INSERT INTO reports(run_id, path, status, termination_reason, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(run_id) DO UPDATE SET
                path = excluded.path,
                status = excluded.status,
                termination_reason = excluded.termination_reason,
                updated_at = excluded.updated_at
            "#,
            params![
                run_id.to_string(),
                path.to_string_lossy().as_ref(),
                status,
                termination_reason,
                now,
            ],
        )
        .map_err(io_other)?;
        conn.execute(
            r#"
            UPDATE runs
            SET report_path = ?2, status = ?3, completed_at = ?4, updated_at = ?4
            WHERE run_id = ?1
            "#,
            params![
                run_id.to_string(),
                path.to_string_lossy().as_ref(),
                run_status,
                now
            ],
        )
        .map_err(io_other)?;
        conn.execute(
            r#"
            UPDATE jobs
            SET status = ?2, updated_at = ?3
            WHERE run_id = ?1
            "#,
            params![run_id.to_string(), run_status, now],
        )
        .map_err(io_other)?;
        Ok(())
    }

    fn connect(&self) -> std::io::Result<Connection> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(self.db_path.as_ref()).map_err(io_other)?;
        conn.busy_timeout(Duration::from_millis(self.busy_timeout_ms))
            .map_err(io_other)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(io_other)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(io_other)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(io_other)?;
        apply_migrations(&conn)?;
        Ok(conn)
    }

    fn connect_read_only(&self) -> std::io::Result<Connection> {
        let metadata = match std::fs::metadata(self.db_path.as_ref()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("state index not found at {}", self.db_path.display()),
                ));
            }
            Err(error) => return Err(error),
        };
        if !metadata.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "state index is not a regular file at {}",
                    self.db_path.display()
                ),
            ));
        }
        let connection = Connection::open_with_flags(
            self.db_path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(io_other)?;
        connection
            .busy_timeout(Duration::from_millis(self.busy_timeout_ms))
            .map_err(io_other)?;
        connection
            .pragma_update(None, "query_only", "ON")
            .map_err(io_other)?;
        Ok(connection)
    }

    fn connect_existing_write_guard(&self) -> std::io::Result<Connection> {
        let metadata = match std::fs::symlink_metadata(self.db_path.as_ref()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("state index not found at {}", self.db_path.display()),
                ));
            }
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "state index is not a regular file at {}",
                    self.db_path.display()
                ),
            ));
        }
        let connection = Connection::open_with_flags(
            self.db_path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(io_other)?;
        connection
            .busy_timeout(Duration::from_millis(self.busy_timeout_ms))
            .map_err(io_other)?;
        Ok(connection)
    }

    fn mark_pending_status(
        &self,
        table: &str,
        id_column: &str,
        id: CallId,
        status: &str,
    ) -> std::io::Result<()> {
        let conn = self.connect()?;
        let now = now_rfc3339();
        let sql = format!("UPDATE {table} SET status = ?2, updated_at = ?3 WHERE {id_column} = ?1");
        let updated = conn
            .execute(&sql, params![id.to_string(), status, now])
            .map_err(io_other)?;
        if updated == 1 {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("pending interaction {id} was not found in {table}"),
            ))
        }
    }

    fn pending_status(
        &self,
        table: &str,
        id_column: &str,
        id: CallId,
    ) -> std::io::Result<Option<String>> {
        let conn = self.connect()?;
        let sql = format!("SELECT status FROM {table} WHERE {id_column} = ?1");
        conn.query_row(&sql, params![id.to_string()], |row| row.get(0))
            .optional()
            .map_err(io_other)
    }
}

fn apply_migrations(conn: &Connection) -> std::io::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(io_other)?;
    let newest: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map_err(io_other)?;
    if newest.is_some_and(|version| version > CURRENT_SCHEMA_VERSION) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state index schema is newer than this runtime",
        ));
    }
    for (version, name, sql) in MIGRATIONS {
        let applied: Option<i64> = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = ?1",
                params![version],
                |row| row.get(0),
            )
            .optional()
            .map_err(io_other)?;
        if applied.is_some() {
            continue;
        }
        conn.execute_batch(sql).map_err(io_other)?;
        conn.execute(
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![version, name, now_rfc3339()],
        )
        .map_err(io_other)?;
    }
    Ok(())
}

fn ensure_runs_by_job_index(connection: &Connection) -> std::io::Result<()> {
    let mut index_list = connection
        .prepare("PRAGMA index_list('runs')")
        .map_err(io_other)?;
    let rows = index_list
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(io_other)?;
    let index_shape = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_other)?
        .into_iter()
        .find(|(name, _, _, _)| name == RUNS_BY_JOB_INDEX);
    if index_shape
        .as_ref()
        .map(|(_, unique, origin, partial)| (*unique, origin.as_str(), *partial))
        != Some((0, "c", 0))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state index has an unsafe bounded runs-by-job lookup index",
        ));
    }

    let pragma = format!("PRAGMA index_xinfo('{RUNS_BY_JOB_INDEX}')");
    let mut statement = connection.prepare(&pragma).map_err(io_other)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(io_other)?;
    let columns = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_other)?
        .into_iter()
        .filter_map(|(name, descending, collation, key)| {
            (key == 1).then_some((name, descending, collation))
        })
        .collect::<Vec<_>>();
    if columns
        != [
            (Some("job_id".to_string()), 0, "BINARY".to_string()),
            (Some("started_at".to_string()), 0, "BINARY".to_string()),
            (Some("run_id".to_string()), 0, "BINARY".to_string()),
        ]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state index has an invalid bounded runs-by-job lookup index shape",
        ));
    }
    Ok(())
}

fn ensure_current_schema_for_read_only_inspection(connection: &Connection) -> std::io::Result<()> {
    let mut statement = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version ASC")
        .map_err(io_other)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(io_other)?;
    let applied = rows.collect::<Result<Vec<_>, _>>().map_err(io_other)?;
    let is_current = applied.len() == MIGRATIONS.len()
        && applied.iter().zip(MIGRATIONS.iter()).all(
            |((actual_version, actual_name), (version, name, _))| {
                *actual_version == *version && actual_name.as_str() == *name
            },
        );
    if !is_current {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state index schema is not the exact version required for read-only inspection",
        ));
    }
    Ok(())
}

fn select_expired_jobs(
    conn: &Connection,
    now: DateTime<FixedOffset>,
) -> std::io::Result<Vec<ExpiredJobRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT jobs.job_id, jobs.run_id, runs.run_dir, jobs.ttl_expires_at
            FROM jobs
            LEFT JOIN runs ON jobs.run_id = runs.run_id
            WHERE jobs.ttl_expires_at IS NOT NULL
            ORDER BY jobs.updated_at ASC, jobs.job_id ASC
            "#,
        )
        .map_err(io_other)?;
    let rows = stmt
        .query_map([], |row| {
            let ttl_expires_at: String = row.get(3)?;
            Ok((
                job_id_from_row(row, 0)?,
                row.get::<_, Option<String>>(1)?
                    .map(|value| parse_run_id_at(1, value))
                    .transpose()?,
                row.get::<_, Option<String>>(2)?.map(PathBuf::from),
                ttl_expires_at,
            ))
        })
        .map_err(io_other)?;

    let mut expired = Vec::new();
    for row in rows {
        let (job_id, run_id, run_dir, ttl_expires_at) = row.map_err(io_other)?;
        let ttl = DateTime::parse_from_rfc3339(&ttl_expires_at)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        if ttl <= now {
            expired.push(ExpiredJobRecord {
                job_id,
                run_id,
                run_dir,
            });
        }
    }
    Ok(expired)
}

fn delete_expired_jobs(conn: &Connection, records: &[ExpiredJobRecord]) -> std::io::Result<()> {
    for record in records {
        if let Some(run_id) = record.run_id {
            conn.execute(
                "DELETE FROM pending_approvals WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM pending_inputs WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM events WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM event_offsets WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM reports WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM task_states WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
            conn.execute(
                "DELETE FROM runs WHERE run_id = ?1",
                params![run_id.to_string()],
            )
            .map_err(io_other)?;
        }
        conn.execute(
            "DELETE FROM jobs WHERE job_id = ?1",
            params![record.job_id.to_string()],
        )
        .map_err(io_other)?;
    }
    Ok(())
}

fn remove_run_dir_if_safe(state_dir: &Path, run_dir: &Path) -> std::io::Result<()> {
    let runs_root = state_dir.join("runs");
    if !run_dir.starts_with(&runs_root) {
        return Ok(());
    }
    if run_dir.exists() {
        std::fs::remove_dir_all(run_dir)?;
    }
    Ok(())
}

fn upsert_session(conn: &Connection, session_id: SessionId, now: &str) -> std::io::Result<()> {
    conn.execute(
        r#"
        INSERT INTO sessions(session_id, created_at, updated_at)
        VALUES (?1, ?2, ?2)
        ON CONFLICT(session_id) DO UPDATE SET updated_at = excluded.updated_at
        "#,
        params![session_id.to_string(), now],
    )
    .map_err(io_other)?;
    Ok(())
}

fn task_state_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskStateIndexRecord> {
    Ok(TaskStateIndexRecord {
        run_id: run_id_from_row(row, 0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
    })
}

#[derive(Debug)]
struct InspectionValueMetadata {
    storage_class: String,
    byte_len: Option<i64>,
}

fn inspection_metadata_from_row(
    row: &rusqlite::Row<'_>,
    value_count: usize,
) -> rusqlite::Result<Vec<InspectionValueMetadata>> {
    (0..value_count)
        .map(|index| {
            Ok(InspectionValueMetadata {
                storage_class: row.get(index * 2)?,
                byte_len: row.get(index * 2 + 1)?,
            })
        })
        .collect()
}

fn validate_inspection_metadata(
    metadata: &[InspectionValueMetadata],
    fields: &[(&'static str, i64, bool)],
) -> std::io::Result<()> {
    if metadata.len() != fields.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state index inspection metadata shape is invalid",
        ));
    }
    for (value, (field, maximum, nullable)) in metadata.iter().zip(fields) {
        let valid = match (value.storage_class.as_str(), value.byte_len) {
            ("null", None) => *nullable,
            ("text", Some(length)) => length > 0 && length <= *maximum,
            _ => false,
        };
        if !valid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("state index field {field} has an invalid type or length"),
            ));
        }
    }
    Ok(())
}

fn inspection_job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRunInspectionJob> {
    let run_id = row
        .get::<_, Option<String>>(3)?
        .map(|value| parse_run_id_at(3, value))
        .transpose()?;
    Ok(JobRunInspectionJob {
        job_id: job_id_from_row(row, 0)?,
        session_id: session_id_from_row(row, 1)?,
        status: row.get(2)?,
        run_id,
    })
}

fn inspection_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRunInspectionRun> {
    Ok(JobRunInspectionRun {
        run_id: run_id_from_row(row, 0)?,
        session_id: session_id_from_row(row, 1)?,
        job_id: job_id_from_row(row, 2)?,
        status: row.get(3)?,
        run_dir: PathBuf::from(row.get::<_, String>(4)?),
        task_state_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
    })
}

fn job_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobIndexRecord> {
    let run_id = row
        .get::<_, Option<String>>(3)?
        .map(|value| parse_run_id_at(3, value))
        .transpose()?;
    Ok(JobIndexRecord {
        job_id: job_id_from_row(row, 0)?,
        session_id: session_id_from_row(row, 1)?,
        status: row.get(2)?,
        run_id,
        message: row.get(4)?,
    })
}

fn run_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunIndexRecord> {
    Ok(RunIndexRecord {
        run_id: run_id_from_row(row, 0)?,
        session_id: session_id_from_row(row, 1)?,
        job_id: job_id_from_row(row, 2)?,
        status: row.get(3)?,
        run_dir: PathBuf::from(row.get::<_, String>(4)?),
        trace_path: PathBuf::from(row.get::<_, String>(5)?),
        task_state_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
        report_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
        last_event_seq: nonnegative_u64_from_row(row, 8)?,
    })
}

fn report_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportIndexRecord> {
    Ok(ReportIndexRecord {
        run_id: run_id_from_row(row, 0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        status: row.get(2)?,
        termination_reason: row.get(3)?,
    })
}

fn event_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventIndexRecord> {
    Ok(EventIndexRecord {
        run_id: run_id_from_row(row, 0)?,
        seq: nonnegative_u64_from_row(row, 1)?,
        event_name: row.get(2)?,
        event_json: row.get(3)?,
    })
}

fn event_snapshot_metadata_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<EventSnapshotMetadata> {
    let event_json_bytes = row.get::<_, i64>(4)?;
    Ok(EventSnapshotMetadata {
        run_id: run_id_from_row(row, 0)?,
        seq: nonnegative_u64_from_row(row, 1)?,
        event_name_storage_class: row.get(2)?,
        event_json_storage_class: row.get(3)?,
        event_json_bytes: usize::try_from(event_json_bytes)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, event_json_bytes))?,
    })
}

fn bounded_snapshot_prefix_len(metadata: &[EventSnapshotMetadata], event_limit: usize) -> usize {
    let candidate_count = metadata.len().min(event_limit);
    let mut total_bytes = 0_usize;
    metadata[..candidate_count]
        .iter()
        .position(|record| {
            if record.event_json_bytes > MAX_SNAPSHOT_EVENT_JSON_BYTES {
                return true;
            }
            match total_bytes.checked_add(record.event_json_bytes) {
                Some(next_total) if next_total <= MAX_SNAPSHOT_EVENT_JSON_TOTAL_BYTES => {
                    total_bytes = next_total;
                    false
                }
                _ => true,
            }
        })
        .unwrap_or(candidate_count)
}

fn nonnegative_u64_from_row(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn session_id_from_row(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<SessionId> {
    let value = row.get::<_, String>(index)?;
    ulid::Ulid::from_string(&value)
        .map(SessionId)
        .map_err(|err| from_sql_ulid_error(index, err))
}

fn job_id_from_row(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<JobId> {
    let value = row.get::<_, String>(index)?;
    ulid::Ulid::from_string(&value)
        .map(JobId)
        .map_err(|err| from_sql_ulid_error(index, err))
}

fn run_id_from_row(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<RunId> {
    parse_run_id_at(index, row.get::<_, String>(index)?)
}

fn parse_run_id_at(index: usize, value: String) -> rusqlite::Result<RunId> {
    ulid::Ulid::from_string(&value)
        .map(RunId)
        .map_err(|err| from_sql_ulid_error(index, err))
}

fn from_sql_ulid_error(index: usize, err: ulid::DecodeError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn system_time_millis(value: SystemTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn rebase_index_path(raw: &str, from_state_dir: &Path, to_state_dir: &Path) -> Option<String> {
    if raw.is_empty() || !Path::new(raw).is_absolute() {
        return None;
    }

    let path_text = normalized_index_path_text(&normalize_unresolved_index_path(Path::new(raw)));
    let from_text = normalized_index_path_text(&normalize_unresolved_index_path(from_state_dir));
    #[cfg(windows)]
    let (path_cmp, from_cmp) = (
        path_text.to_ascii_lowercase(),
        from_text.to_ascii_lowercase(),
    );
    #[cfg(not(windows))]
    let (path_cmp, from_cmp) = (path_text.clone(), from_text.clone());

    let relative = if path_cmp == from_cmp {
        String::new()
    } else {
        let prefix = format!("{}/", from_cmp);
        let suffix = path_cmp.strip_prefix(&prefix)?;
        // ASCII case folding preserves byte offsets. Use the original text
        // for the returned suffix so non-ASCII path spelling is retained.
        path_text
            .get(from_text.len() + 1..)?
            .get(..suffix.len())?
            .to_string()
    };
    let rebased = if relative.is_empty() {
        to_state_dir.to_path_buf()
    } else {
        to_state_dir.join(relative)
    };
    Some(rebased.to_string_lossy().into_owned())
}

fn normalize_unresolved_index_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    }
    let mut ancestor = path.to_path_buf();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        if let Some(name) = ancestor.file_name() {
            missing.push(name.to_os_string());
        }
        let Some(parent) = ancestor.parent() else {
            return path.to_path_buf();
        };
        ancestor = parent.to_path_buf();
    }
    let mut normalized = ancestor.canonicalize().unwrap_or(ancestor);
    for name in missing.iter().rev() {
        normalized.push(name);
    }
    normalized
}

fn normalized_index_path_text(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        let mut text = text;
        if text
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
        {
            text = format!("//{}", &text[8..]);
        } else if let Some(stripped) = text.strip_prefix("//?/") {
            // Keep verbatim volume/device paths absolute. Drive-letter paths
            // can be safely compared in their ordinary spelling.
            if stripped
                .as_bytes()
                .get(1)
                .is_some_and(|value| *value == b':')
            {
                text = stripped.to_string();
            }
        }
        text
    }
    #[cfg(not(windows))]
    {
        text
    }
}

fn io_other(err: rusqlite::Error) -> std::io::Error {
    let invalid_data = matches!(
        &err,
        rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::Utf8Error(..)
            | rusqlite::Error::InvalidColumnIndex(..)
            | rusqlite::Error::InvalidColumnName(..)
            | rusqlite::Error::InvalidColumnType(..)
    ) || matches!(
        &err,
        rusqlite::Error::SqliteFailure(error, _)
            if matches!(
                error.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase | ErrorCode::TypeMismatch
            )
    );
    if invalid_data {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err)
    } else {
        std::io::Error::other(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    fn indexed_run() -> (tempfile::TempDir, StateIndex, SessionId, JobId, RunId) {
        let temp = tempfile::TempDir::new().unwrap();
        let index = StateIndex::new(temp.path());
        index.initialize().unwrap();
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        index
            .record_run_started(
                session_id,
                job_id,
                run_id,
                &temp.path().join("run"),
                &temp.path().join("trace.jsonl"),
            )
            .unwrap();
        (temp, index, session_id, job_id, run_id)
    }

    fn task_state(session_id: SessionId, job_id: JobId, run_id: RunId, goal: &str) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id,
            job_id,
            run_id,
            goal: goal.to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: None,
            plan: None,
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        }
    }

    #[test]
    fn historical_task_state_import_does_not_replace_the_latest_job_run() {
        let (temp, index, session_id, job_id, first_run_id) = indexed_run();
        let first_modified = UNIX_EPOCH + Duration::from_secs(1);
        index
            .record_task_state(
                &task_state(session_id, job_id, first_run_id, "first"),
                &temp.path().join("first-task-state.json"),
                first_modified,
            )
            .unwrap();

        let second_run_id = RunId::new();
        index
            .record_run_started(
                session_id,
                job_id,
                second_run_id,
                &temp.path().join("second-run"),
                &temp.path().join("second-trace.jsonl"),
            )
            .unwrap();

        index
            .record_task_state(
                &task_state(session_id, job_id, first_run_id, "first"),
                &temp.path().join("first-task-state.json"),
                first_modified,
            )
            .unwrap();
        assert_eq!(
            index.job_record(job_id).unwrap().unwrap().run_id,
            Some(second_run_id)
        );

        index
            .record_task_state(
                &task_state(session_id, job_id, second_run_id, "second"),
                &temp.path().join("second-task-state.json"),
                first_modified + Duration::from_secs(1),
            )
            .unwrap();
        index
            .record_task_state(
                &task_state(session_id, job_id, first_run_id, "first"),
                &temp.path().join("first-task-state.json"),
                first_modified,
            )
            .unwrap();

        let job = index.job_record(job_id).unwrap().unwrap();
        assert_eq!(job.run_id, Some(second_run_id));
        assert_eq!(job.message.as_deref(), Some("second"));
    }

    #[test]
    fn external_commit_guard_rechecks_the_singleton_latest_run() {
        let (temp, index, session_id, job_id, first_run_id) = indexed_run();
        index
            .record_report(
                first_run_id,
                &temp.path().join("first-report.json"),
                "success",
                "final",
            )
            .unwrap();
        let second_run_id = RunId::new();
        index
            .record_run_started(
                session_id,
                job_id,
                second_run_id,
                &temp.path().join("second-run"),
                &temp.path().join("second-trace.jsonl"),
            )
            .unwrap();
        index
            .record_report(
                second_run_id,
                &temp.path().join("second-report.json"),
                "success",
                "final",
            )
            .unwrap();

        let (eligible, _guard) = index
            .guard_job_runs_for_external_commit(&[(job_id, first_run_id)])
            .unwrap();

        assert_eq!(eligible, vec![false]);
    }

    #[test]
    fn external_commit_guard_does_not_create_missing_runtime_state() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = temp.path().join("missing.sqlite");
        let index = StateIndex::with_path(temp.path(), database.clone(), 1);

        let error = index
            .guard_job_runs_for_external_commit(&[(JobId::new(), RunId::new())])
            .err()
            .expect("missing runtime state must fail closed");

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(!database.exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn external_commit_guard_rejects_a_symlinked_database_parent() {
        let temp = tempfile::TempDir::new().unwrap();
        let target_dir = temp.path().join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target_index = StateIndex::new(&target_dir);
        target_index.initialize().unwrap();
        let linked_dir = temp.path().join("linked");
        if !create_directory_symlink(&target_dir, &linked_dir) {
            return;
        }
        let linked_index = StateIndex::with_path(
            &linked_dir,
            linked_dir.join("state.sqlite"),
            DEFAULT_BUSY_TIMEOUT_MS,
        );

        let error = linked_index
            .guard_job_runs_for_external_commit(&[(JobId::new(), RunId::new())])
            .err()
            .expect("external commit guard must not follow a symlinked parent");

        assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn external_commit_guard_blocks_resume_until_it_is_dropped() {
        let (temp, index, _session_id, job_id, run_id) = indexed_run();
        index
            .record_report(run_id, &temp.path().join("report.json"), "success", "final")
            .unwrap();
        let (eligible, guard) = index
            .guard_job_runs_for_external_commit(&[(job_id, run_id)])
            .unwrap();
        assert_eq!(eligible, vec![true]);

        let contender = StateIndex::with_path(temp.path(), index.path().to_path_buf(), 1);
        assert!(contender.claim_job_for_resume(job_id, run_id).is_err());

        drop(guard);
        let claim = contender
            .claim_job_for_resume(job_id, run_id)
            .unwrap()
            .expect("terminal run should become resumable after guard release");
        assert!(contender.release_job_resume_claim(&claim).unwrap());
    }

    #[test]
    fn snapshot_returns_safe_prefix_before_oversized_event() {
        let (_temp, index, _session_id, job_id, run_id) = indexed_run();
        let event = StreamEvent::RunStarted {
            run_id,
            job_id,
            user_message: "bounded snapshot".to_string(),
        };
        index.append_event(run_id, 1, &event, "{}").unwrap();
        index
            .append_event(
                run_id,
                2,
                &event,
                &"x".repeat(MAX_SNAPSHOT_EVENT_JSON_BYTES + 1),
            )
            .unwrap();
        index.append_event(run_id, 3, &event, "{}").unwrap();

        let snapshot = index.run_event_snapshot(run_id, 0, 10).unwrap().unwrap();

        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].seq, 1);
        assert_eq!(snapshot.high_water_seq, 3);
        assert!(snapshot.has_more);
    }

    #[test]
    fn snapshot_prefix_enforces_total_payload_limit() {
        let run_id = RunId::new();
        let metadata = (1..=17)
            .map(|seq| EventSnapshotMetadata {
                run_id,
                seq,
                event_name_storage_class: "text".to_string(),
                event_json_storage_class: "text".to_string(),
                event_json_bytes: MAX_SNAPSHOT_EVENT_JSON_BYTES,
            })
            .collect::<Vec<_>>();

        assert_eq!(bounded_snapshot_prefix_len(&metadata, 17), 16);
    }

    #[test]
    fn snapshot_maps_invalid_ulid_to_invalid_data() {
        let (_temp, index, _session_id, _job_id, run_id) = indexed_run();
        let conn = Connection::open(index.path()).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        conn.execute(
            "UPDATE runs SET session_id = 'not-a-ulid' WHERE run_id = ?1",
            params![run_id.to_string()],
        )
        .unwrap();

        let error = index.run_event_snapshot(run_id, 0, 10).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn snapshot_maps_invalid_column_type_to_invalid_data() {
        let (_temp, index, _session_id, _job_id, run_id) = indexed_run();
        let conn = Connection::open(index.path()).unwrap();
        conn.execute(
            "UPDATE runs SET last_event_seq = 'invalid' WHERE run_id = ?1",
            params![run_id.to_string()],
        )
        .unwrap();

        let error = index.run_event_snapshot(run_id, 0, 10).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_only_job_run_inspection_does_not_create_missing_state() {
        let temp = tempfile::TempDir::new().unwrap();
        let missing_state = temp.path().join("missing").join("state");
        let index = StateIndex::new(&missing_state);

        let error = index
            .inspect_job_run_read_only(JobId::new(), RunId::new(), 1)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(!missing_state.exists());
    }

    #[test]
    fn read_only_job_run_inspection_rejects_non_file_state_index() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_directory = temp.path().join("state.sqlite");
        std::fs::create_dir(&db_directory).unwrap();
        let index = StateIndex::with_path(temp.path(), db_directory, DEFAULT_BUSY_TIMEOUT_MS);

        let error = index
            .inspect_job_run_read_only(JobId::new(), RunId::new(), 1)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_only_job_run_inspection_rejects_oversized_text_before_materializing_it() {
        let (_temp, index, _session_id, job_id, run_id) = indexed_run();
        let connection = Connection::open(index.path()).unwrap();
        connection
            .execute(
                "UPDATE runs SET run_dir = CAST(zeroblob(?1) AS TEXT) WHERE run_id = ?2",
                params![MAX_INSPECTION_PATH_BYTES + 1, run_id.to_string()],
            )
            .unwrap();
        drop(connection);

        let error = index
            .inspect_job_run_read_only(job_id, run_id, 1)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn state_index_upgrade_installs_bounded_runs_by_job_index() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = StateIndex::new(temp.path());
        let connection = Connection::open(index.path()).unwrap();
        connection.execute_batch(MIGRATION_001).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, applied_at) VALUES (1, 'runtime_state_index', ?1)",
                params![now_rfc3339()],
            )
            .unwrap();
        drop(connection);

        index.initialize().unwrap();

        let connection = Connection::open(index.path()).unwrap();
        ensure_runs_by_job_index(&connection).unwrap();
        let newest: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(newest, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn read_only_job_run_inspection_does_not_migrate_an_old_index() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = StateIndex::new(temp.path());
        let connection = Connection::open(index.path()).unwrap();
        connection.execute_batch(MIGRATION_001).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, applied_at) VALUES (1, 'runtime_state_index', ?1)",
                params![now_rfc3339()],
            )
            .unwrap();
        drop(connection);

        let error = index
            .inspect_job_run_read_only(JobId::new(), RunId::new(), 1)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        let connection = Connection::open(index.path()).unwrap();
        let newest: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(newest, 1);
        let present: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![RUNS_BY_JOB_INDEX],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(present.is_none());
    }

    #[test]
    fn read_only_job_run_inspection_rejects_a_partial_lookalike_index() {
        let (_temp, index, _session_id, job_id, run_id) = indexed_run();
        let connection = Connection::open(index.path()).unwrap();
        connection
            .execute(&format!("DROP INDEX {RUNS_BY_JOB_INDEX}"), [])
            .unwrap();
        connection
            .execute(
                &format!(
                    "CREATE INDEX {RUNS_BY_JOB_INDEX} ON runs(job_id, started_at, run_id) WHERE status = 'done'"
                ),
                [],
            )
            .unwrap();
        drop(connection);

        let error = index
            .inspect_job_run_read_only(job_id, run_id, 1)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_only_job_run_inspection_preserves_main_database_and_schema() {
        let (_temp, index, _session_id, job_id, run_id) = indexed_run();
        let schema_before = sqlite_schema_snapshot(index.path());
        let metadata_before = std::fs::metadata(index.path()).unwrap();
        let files_before = sibling_file_names(index.path());

        index.inspect_job_run_read_only(job_id, run_id, 1).unwrap();

        let metadata_after = std::fs::metadata(index.path()).unwrap();
        let files_after = sibling_file_names(index.path());
        assert_eq!(metadata_before.len(), metadata_after.len());
        assert_eq!(
            metadata_before.modified().unwrap(),
            metadata_after.modified().unwrap()
        );
        assert_eq!(schema_before, sqlite_schema_snapshot(index.path()));
        let allowed_shm = format!(
            "{}-shm",
            index.path().file_name().unwrap().to_string_lossy()
        );
        assert!(
            files_after
                .difference(&files_before)
                .all(|name| name == &allowed_shm),
            "a read-only WAL reader may coordinate through -shm only"
        );
    }

    #[test]
    fn read_only_job_run_inspection_reports_a_truncated_prefix() {
        let (temp, index, session_id, job_id, first_run_id) = indexed_run();
        let second_run_id = RunId::new();
        index
            .record_run_started(
                session_id,
                job_id,
                second_run_id,
                &temp.path().join("second-run"),
                &temp.path().join("second-trace.jsonl"),
            )
            .unwrap();

        let snapshot = index
            .inspect_job_run_read_only(job_id, second_run_id, 1)
            .unwrap();

        assert_eq!(snapshot.job.unwrap().run_id, Some(second_run_id));
        assert_eq!(snapshot.run.unwrap().run_id, second_run_id);
        assert_eq!(snapshot.job_run_ids, vec![first_run_id]);
        assert!(snapshot.job_runs_truncated);
    }

    #[test]
    fn sqlite_corruption_maps_to_invalid_data() {
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            None,
        );

        assert_eq!(io_other(error).kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(windows)]
    #[test]
    fn normalized_index_paths_preserve_unc_roots() {
        assert_eq!(
            normalized_index_path_text(Path::new(r"\\?\UNC\server\share\state.sqlite")),
            "//server/share/state.sqlite"
        );
        assert_eq!(
            normalized_index_path_text(Path::new(r"\\?\C:\workspace\state.sqlite")),
            "C:/workspace/state.sqlite"
        );
    }

    fn sqlite_schema_snapshot(path: &Path) -> Vec<(String, String, String)> {
        let connection =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut statement = connection
            .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn sibling_file_names(path: &Path) -> std::collections::BTreeSet<String> {
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}
