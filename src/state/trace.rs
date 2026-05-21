use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::events::StreamEvent;
use crate::core::types::RunId;

/// Manages trace file writing for a run.
///
/// Each run gets a `trace.jsonl` file with one JSON event per line.
pub struct TraceWriter {
    path: PathBuf,
}

impl TraceWriter {
    /// Create a new trace writer for the given run directory.
    pub fn new(run_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(run_dir)?;
        let path = run_dir.join("trace.jsonl");
        Ok(Self { path })
    }

    /// Append an event to the trace file.
    pub fn append(&self, event: &StreamEvent) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let json = serde_json::to_string(event).map_err(std::io::Error::other)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Manages the run directory structure under `.rove/runs/<run_id>/`.
pub struct RunStore {
    base_dir: PathBuf,
}

impl RunStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            base_dir: state_dir.join("runs"),
        }
    }

    /// Get the directory path for a specific run.
    pub fn run_dir(&self, run_id: &RunId) -> PathBuf {
        self.base_dir.join(run_id.to_string())
    }

    /// Create a trace writer for a new run.
    pub fn create_trace(&self, run_id: &RunId) -> std::io::Result<TraceWriter> {
        let run_dir = self.run_dir(run_id);
        TraceWriter::new(&run_dir)
    }
}
