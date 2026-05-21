use std::path::Path;

use crate::core::types::RunId;

use super::trace::RunStore;

/// Top-level state store.
///
/// Coordinates run directories, trace files, and (later) report generation.
pub struct StateStore {
    pub run_store: RunStore,
}

impl StateStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            run_store: RunStore::new(state_dir),
        }
    }

    /// Create a new run and return its ID.
    pub fn new_run(&self) -> RunId {
        RunId::new()
    }
}
