use std::path::{Path, PathBuf};

use crate::core::types::{RunId, TaskState};

use super::trace::RunStore;

/// Top-level state store.
///
/// Coordinates run directories, trace files, and (later) report generation.
pub struct StateStore {
    pub run_store: RunStore,
    state_dir: PathBuf,
}

impl StateStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            run_store: RunStore::new(state_dir),
            state_dir: state_dir.to_path_buf(),
        }
    }

    /// Create a new run and return its ID.
    pub fn new_run(&self) -> RunId {
        RunId::new()
    }

    pub async fn write_task_state(&self, state: &TaskState) -> std::io::Result<()> {
        let run_dir = self.run_store.run_dir(&state.run_id);
        tokio::fs::create_dir_all(&run_dir).await?;
        let path = run_dir.join("task_state.json");
        let json = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
        tokio::fs::write(path, json).await
    }

    pub async fn load_latest_task_state(&self) -> std::io::Result<Option<TaskState>> {
        let runs_dir = self.state_dir.join("runs");
        let mut entries = match tokio::fs::read_dir(&runs_dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };

        let mut state_paths = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path().join("task_state.json");
            if tokio::fs::try_exists(&path).await? {
                state_paths.push(path);
            }
        }

        state_paths.sort();
        let Some(path) = state_paths.pop() else {
            return Ok(None);
        };

        let bytes = tokio::fs::read(path).await?;
        let state = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Ok(Some(state))
    }
}
