use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::events::StreamEvent;
use crate::core::types::{JobId, RunId, RunRequest, SessionId, TaskState};

use super::index::{CleanupResult, StateIndex, TaskStateIndexRecord};
use super::report::RunReport;
use super::trace::RunStore;
use super::trace::TraceWriter;

const TASK_STATE_SCHEMA_VERSION: u32 = 1;

/// Top-level state store.
///
/// Coordinates run directories, trace files, and (later) report generation.
pub struct StateStore {
    pub run_store: RunStore,
    pub index: StateIndex,
    state_dir: PathBuf,
}

/// Identity and filesystem bundle for a single run.
pub struct RunHandle {
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub run_dir: PathBuf,
    pub trace_writer: TraceWriter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairResult {
    pub task_state_count: usize,
    pub event_count: usize,
    pub report_count: usize,
    pub corrupt_trace_line_count: usize,
}

struct TaskStateEntry {
    path: PathBuf,
    modified: SystemTime,
}

struct TraceImportResult {
    event_count: usize,
    corrupt_line_count: usize,
}

impl StateStore {
    pub fn new(state_dir: &Path) -> Self {
        let index = StateIndex::new(state_dir);
        Self::with_index(state_dir, index)
    }

    pub fn with_index_path(state_dir: &Path, db_path: PathBuf, busy_timeout_ms: u64) -> Self {
        let index = StateIndex::with_path(state_dir, db_path, busy_timeout_ms);
        Self::with_index(state_dir, index)
    }

    pub fn with_index(state_dir: &Path, index: StateIndex) -> Self {
        Self {
            run_store: RunStore::with_index(state_dir, index.clone()),
            index,
            state_dir: state_dir.to_path_buf(),
        }
    }

    /// Create a new run and return its filesystem handle.
    pub fn start_run(
        &self,
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
    ) -> std::io::Result<RunHandle> {
        let run_dir = self.run_store.run_dir(&run_id);
        let trace_writer = self.run_store.create_trace(&run_id)?;
        self.index
            .record_run_started(session_id, job_id, run_id, &run_dir, trace_writer.path())?;
        Ok(RunHandle {
            session_id,
            job_id,
            run_id,
            run_dir,
            trace_writer,
        })
    }

    pub async fn write_task_state(&self, state: &TaskState) -> std::io::Result<()> {
        let run_dir = self.run_store.run_dir(&state.run_id);
        tokio::fs::create_dir_all(&run_dir).await?;
        let path = run_dir.join("task_state.json");
        let json = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
        atomic_write(&path, &json).await?;
        let modified = tokio::fs::metadata(&path)
            .await?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        self.index
            .record_task_state_async(state.clone(), path, modified)
            .await
    }

    pub async fn load_latest_task_state(&self) -> std::io::Result<Option<TaskState>> {
        self.lazy_import_task_states().await?;
        let records = self.index.list_task_state_records_async(None).await?;
        let Some(record) = records.first() else {
            return Ok(None);
        };

        let state = self.load_task_state_path(&record.path).await?;
        if state.run_id != record.run_id {
            return Err(task_state_identity_error(record.run_id, state.run_id));
        }
        Ok(Some(state))
    }

    pub async fn load_task_state(&self, run_id: RunId) -> std::io::Result<TaskState> {
        self.lazy_import_task_states().await?;
        let path = match self.index.task_state_path_async(run_id).await? {
            Some(path) => path,
            None => self.run_store.run_dir(&run_id).join("task_state.json"),
        };
        if !tokio::fs::try_exists(&path).await? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("task_state not found for run {run_id}"),
            ));
        }
        let state = self.load_task_state_path(&path).await?;
        if state.run_id != run_id {
            return Err(task_state_identity_error(run_id, state.run_id));
        }
        Ok(state)
    }

    pub async fn list_resumable_task_states(
        &self,
        session_id: SessionId,
    ) -> std::io::Result<Vec<TaskState>> {
        self.lazy_import_task_states().await?;
        self.load_task_state_records(
            self.index
                .list_task_state_records_async(Some(session_id))
                .await?,
        )
        .await
    }

    pub async fn list_task_states(&self) -> std::io::Result<Vec<TaskState>> {
        self.lazy_import_task_states().await?;
        self.load_task_state_records(self.index.list_task_state_records_async(None).await?)
            .await
    }

    /// Load a bounded set of snapshots that still point at their owning job's
    /// latest terminal run. Malformed or concurrently removed artifacts are
    /// skipped so one bad historical file cannot freeze the TUI picker.
    pub async fn list_resumable_task_states_limited(
        &self,
        limit: usize,
    ) -> std::io::Result<Vec<TaskState>> {
        let mut records = self
            .index
            .list_resumable_task_state_records_async(limit)
            .await?;
        if records.is_empty() {
            // A fresh index may still need one legacy artifact import. This
            // path is deliberately cold; normal picker opens stay bounded by
            // the SQL LIMIT above.
            self.import_task_states().await?;
            records = self
                .index
                .list_resumable_task_state_records_async(limit)
                .await?;
        }
        let mut states = Vec::with_capacity(records.len());
        for record in records {
            match self.load_task_state_path(&record.path).await {
                Ok(state) if state.run_id == record.run_id => states.push(state),
                Ok(state) => tracing::warn!(
                    path = %record.path.display(),
                    indexed_run_id = %record.run_id,
                    task_state_run_id = %state.run_id,
                    "Skipping task state with mismatched run identity in resumable picker"
                ),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::InvalidData | std::io::ErrorKind::NotFound
                    ) =>
                {
                    tracing::warn!(
                        path = %record.path.display(),
                        error = %error,
                        "Skipping malformed or missing task state in resumable picker"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        Ok(states)
    }

    pub async fn record_report(
        &self,
        run_id: RunId,
        report_path: PathBuf,
        status: String,
        termination_reason: String,
    ) -> std::io::Result<()> {
        self.index
            .record_report_async(run_id, report_path, status, termination_reason)
            .await
    }

    pub async fn load_report(&self, run_id: RunId) -> std::io::Result<RunReport> {
        let Some(record) = self.index.report_record(run_id)? else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("report not found for run {run_id}"),
            ));
        };
        self.load_report_path(&record.path).await
    }

    pub async fn repair_index(&self) -> std::io::Result<RepairResult> {
        let task_state_count = self.import_task_states().await?;
        let report_count = self.import_reports().await?;
        let trace_import = self.import_trace_events().await?;
        Ok(RepairResult {
            task_state_count,
            event_count: trace_import.event_count,
            report_count,
            corrupt_trace_line_count: trace_import.corrupt_line_count,
        })
    }

    pub async fn cleanup_expired(&self) -> std::io::Result<CleanupResult> {
        self.index.cleanup_expired_async().await
    }

    async fn load_task_state_records(
        &self,
        records: Vec<TaskStateIndexRecord>,
    ) -> std::io::Result<Vec<TaskState>> {
        let mut states = Vec::new();
        for record in records {
            let state = self.load_task_state_path(&record.path).await?;
            if state.run_id != record.run_id {
                return Err(task_state_identity_error(record.run_id, state.run_id));
            }
            states.push(state);
        }
        Ok(states)
    }

    async fn lazy_import_task_states(&self) -> std::io::Result<()> {
        self.import_task_states().await.map(|_| ())
    }

    async fn import_task_states(&self) -> std::io::Result<usize> {
        let mut entries = self.task_state_entries().await?;
        entries.sort_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut imported = 0;
        for entry in entries {
            let state = match self.load_task_state_path(&entry.path).await {
                Ok(state) => state,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::InvalidData | std::io::ErrorKind::NotFound
                    ) =>
                {
                    tracing::warn!(
                        path = %entry.path.display(),
                        error = %error,
                        "Skipping malformed or missing task state during index import"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            if Some(state.run_id) != run_id_from_artifact_path(&entry.path) {
                tracing::warn!(
                    path = %entry.path.display(),
                    task_state_run_id = %state.run_id,
                    "Skipping task state with mismatched run identity during state repair"
                );
                continue;
            }
            let run_id = state.run_id;
            let job_id = state.job_id;
            let state_path = entry.path.clone();
            self.index
                .record_task_state_async(state, state_path, entry.modified)
                .await?;
            let report_path = entry.path.parent().map(|parent| parent.join("report.json"));
            if let Some(report_path) = report_path
                && tokio::fs::try_exists(&report_path).await?
            {
                match self.load_report_path(&report_path).await {
                    Ok(report) if report.run_id == run_id && report.job_id == job_id => {
                        self.index.record_report(
                            run_id,
                            &report_path,
                            &report.status,
                            termination_reason_label(&report.termination_reason),
                        )?;
                    }
                    Ok(_) => tracing::warn!(
                        path = %report_path.display(),
                        "Skipping report with mismatched identity during task-state import"
                    ),
                    Err(error) => tracing::warn!(
                        path = %report_path.display(),
                        error = %error,
                        "Skipping malformed report during task-state import"
                    ),
                }
            }
            imported += 1;
        }
        Ok(imported)
    }

    async fn import_trace_events(&self) -> std::io::Result<TraceImportResult> {
        let entries = self.run_artifact_entries("trace.jsonl").await?;
        let mut event_count = 0;
        let mut corrupt_line_count = 0;
        for entry in entries {
            let Some(run_id) = run_id_from_artifact_path(&entry) else {
                continue;
            };
            let content = tokio::fs::read_to_string(&entry).await?;
            let mut seq = 0;
            for (line_index, line) in content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let line_number = line_index + 1;
                match serde_json::from_str::<StreamEvent>(line) {
                    Ok(event) => {
                        seq += 1;
                        self.index.append_event(run_id, seq, &event, line)?;
                        event_count += 1;
                    }
                    Err(err) => {
                        corrupt_line_count += 1;
                        tracing::warn!(
                            path = %entry.display(),
                            line = line_number,
                            error = %err,
                            "Skipping corrupted trace line during state repair"
                        );
                    }
                }
            }
        }
        Ok(TraceImportResult {
            event_count,
            corrupt_line_count,
        })
    }

    async fn import_reports(&self) -> std::io::Result<usize> {
        let entries = self.run_artifact_entries("report.json").await?;
        let mut imported = 0;
        for entry in entries {
            let report = self.load_report_path(&entry).await?;
            if Some(report.run_id) != run_id_from_artifact_path(&entry) {
                tracing::warn!(
                    path = %entry.display(),
                    report_run_id = %report.run_id,
                    "Skipping report with mismatched run identity during state repair"
                );
                continue;
            }
            let run_dir = entry
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.run_store.run_dir(&report.run_id));
            self.index.record_run_started(
                report.session_id,
                report.job_id,
                report.run_id,
                &run_dir,
                &run_dir.join("trace.jsonl"),
            )?;
            self.record_report(
                report.run_id,
                entry,
                report.status,
                termination_reason_label(&report.termination_reason).to_string(),
            )
            .await?;
            imported += 1;
        }
        Ok(imported)
    }

    async fn task_state_entries(&self) -> std::io::Result<Vec<TaskStateEntry>> {
        let runs_dir = self.state_dir.join("runs");
        let mut entries = match tokio::fs::read_dir(&runs_dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        let mut state_paths = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path().join("task_state.json");
            if tokio::fs::try_exists(&path).await? {
                let modified = tokio::fs::metadata(&path)
                    .await?
                    .modified()
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                state_paths.push(TaskStateEntry { path, modified });
            }
        }

        Ok(state_paths)
    }

    async fn run_artifact_entries(&self, file_name: &str) -> std::io::Result<Vec<PathBuf>> {
        let runs_dir = self.state_dir.join("runs");
        let mut entries = match tokio::fs::read_dir(&runs_dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        let mut artifact_paths = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path().join(file_name);
            if tokio::fs::try_exists(&path).await? {
                artifact_paths.push(path);
            }
        }
        artifact_paths.sort();
        Ok(artifact_paths)
    }

    async fn load_task_state_path(&self, path: &Path) -> std::io::Result<TaskState> {
        let bytes = tokio::fs::read(path).await?;
        let state: TaskState = serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        validate_task_state_schema(&state)?;
        Ok(state)
    }

    async fn load_report_path(&self, path: &Path) -> std::io::Result<RunReport> {
        let bytes = tokio::fs::read(path).await?;
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }
}

fn task_state_identity_error(expected: RunId, actual: RunId) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "task_state run identity mismatch: requested {expected}, artifact contains {actual}"
        ),
    )
}

impl RunHandle {
    pub fn request(&self, user_message: String, resume_state: Option<TaskState>) -> RunRequest {
        RunRequest {
            session_id: self.session_id,
            job_id: self.job_id,
            run_id: self.run_id,
            user_message,
            resume_state,
        }
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    tokio::fs::write(&tmp_path, bytes).await?;
    tokio::fs::rename(tmp_path, path).await
}

fn validate_task_state_schema(state: &TaskState) -> std::io::Result<()> {
    if state.schema_version != TASK_STATE_SCHEMA_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported task_state schema_version {}; supported version is {}",
                state.schema_version, TASK_STATE_SCHEMA_VERSION
            ),
        ));
    }
    Ok(())
}

fn run_id_from_artifact_path(path: &Path) -> Option<RunId> {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .and_then(|value| ulid::Ulid::from_string(value).ok())
        .map(RunId)
}

fn termination_reason_label(reason: &crate::core::types::TerminationReason) -> &'static str {
    match reason {
        crate::core::types::TerminationReason::Final => "final",
        crate::core::types::TerminationReason::StepLimit => "step_limit",
        crate::core::types::TerminationReason::TokenLimit => "token_limit",
        crate::core::types::TerminationReason::TimeLimit => "time_limit",
        crate::core::types::TerminationReason::Error => "error",
        crate::core::types::TerminationReason::Cancelled => "cancelled",
    }
}
