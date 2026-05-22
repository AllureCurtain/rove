use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::types::{JobId, RunId, RunRequest, SessionId, TaskState};

use super::trace::RunStore;
use super::trace::TraceWriter;

const TASK_STATE_SCHEMA_VERSION: u32 = 1;

/// Top-level state store.
///
/// Coordinates run directories, trace files, and (later) report generation.
pub struct StateStore {
    pub run_store: RunStore,
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

struct TaskStateEntry {
    path: PathBuf,
    modified: SystemTime,
}

impl StateStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            run_store: RunStore::new(state_dir),
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
        atomic_write(&path, &json).await
    }

    pub async fn load_latest_task_state(&self) -> std::io::Result<Option<TaskState>> {
        let mut entries = self.task_state_entries().await?;
        sort_newest_first(&mut entries);
        let Some(entry) = entries.first() else {
            return Ok(None);
        };

        self.load_task_state_path(&entry.path).await.map(Some)
    }

    pub async fn load_task_state(&self, run_id: RunId) -> std::io::Result<TaskState> {
        let path = self.run_store.run_dir(&run_id).join("task_state.json");
        if !tokio::fs::try_exists(&path).await? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("task_state not found for run {run_id}"),
            ));
        }
        self.load_task_state_path(&path).await
    }

    pub async fn list_resumable_task_states(
        &self,
        session_id: SessionId,
    ) -> std::io::Result<Vec<TaskState>> {
        let mut states = Vec::new();
        let mut entries = self.task_state_entries().await?;
        sort_newest_first(&mut entries);

        for entry in entries {
            let state = self.load_task_state_path(&entry.path).await?;
            if state.session_id == session_id {
                states.push(state);
            }
        }

        Ok(states)
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

    async fn load_task_state_path(&self, path: &Path) -> std::io::Result<TaskState> {
        let bytes = tokio::fs::read(path).await?;
        let state: TaskState = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        validate_task_state_schema(&state)?;
        Ok(state)
    }
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

fn sort_newest_first(entries: &mut [TaskStateEntry]) {
    entries.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.path.cmp(&left.path))
    });
}
