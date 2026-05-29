use std::path::PathBuf;

use crate::core::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPaths {
    pub session_dir: PathBuf,
    pub durable_dir: PathBuf,
    pub recall_limit: usize,
}

impl MemoryPaths {
    pub fn from_workspace(workspace: &Workspace, recall_limit: usize) -> Self {
        let durable_dir = workspace.state_dir.join("memory");
        Self {
            session_dir: durable_dir.join("sessions"),
            durable_dir,
            recall_limit,
        }
    }
}
