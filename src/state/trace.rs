use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::events::StreamEvent;
use crate::core::types::RunId;

use super::index::StateIndex;

/// Manages trace file writing for a run.
///
/// Each run gets a `trace.jsonl` file with one JSON event per line.
#[derive(Clone)]
pub struct TraceWriter {
    path: PathBuf,
    run_id: Option<RunId>,
    index: Option<StateIndex>,
}

impl TraceWriter {
    /// Create a new trace writer for the given run directory.
    pub fn new(run_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(run_dir)?;
        let path = run_dir.join("trace.jsonl");
        Ok(Self {
            path,
            run_id: None,
            index: None,
        })
    }

    pub fn for_run(run_dir: &Path, run_id: RunId, index: StateIndex) -> std::io::Result<Self> {
        let mut writer = Self::new(run_dir)?;
        writer.run_id = Some(run_id);
        writer.index = Some(index);
        Ok(writer)
    }

    /// Append an event to the trace file.
    pub fn append(&self, event: &StreamEvent) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let json = serde_json::to_string(event).map_err(std::io::Error::other)?;
        writeln!(file, "{}", json)?;
        if let (Some(index), Some(run_id)) = (&self.index, self.run_id) {
            let seq = index.last_event_seq(run_id)? + 1;
            index.append_event(run_id, seq, event, &json)?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Manages the run directory structure under `.rove/runs/<run_id>/`.
pub struct RunStore {
    base_dir: PathBuf,
    index: Option<StateIndex>,
}

impl RunStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            base_dir: state_dir.join("runs"),
            index: None,
        }
    }

    pub fn with_index(state_dir: &Path, index: StateIndex) -> Self {
        Self {
            base_dir: state_dir.join("runs"),
            index: Some(index),
        }
    }

    /// Get the directory path for a specific run.
    pub fn run_dir(&self, run_id: &RunId) -> PathBuf {
        self.base_dir.join(run_id.to_string())
    }

    /// Create a trace writer for a new run.
    pub fn create_trace(&self, run_id: &RunId) -> std::io::Result<TraceWriter> {
        let run_dir = self.run_dir(run_id);
        if let Some(index) = &self.index {
            TraceWriter::for_run(&run_dir, *run_id, index.clone())
        } else {
            TraceWriter::new(&run_dir)
        }
    }
}
