//! Serialized identity/capability records for the execution environment.
//! The process/filesystem behaviour (`ExecutionEnvironment`) stays in
//! `rove-runtime::environment`.

use serde::{Deserialize, Serialize};

use crate::root::WorkspaceKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ExecutionEnvironmentIdentity {
    pub adapter: String,
    pub workspace_kind: WorkspaceKind,
    /// A stable redacted identity. The canonical local path is never persisted
    /// in this structure or in resume diagnostics.
    pub workspace_digest: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, utoipa::ToSchema)]
#[serde(default)]
pub struct ExecutionCapabilities {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub process_run: bool,
    pub process_stdio: bool,
    pub observations: bool,
    pub process_background: bool,
    pub process_pty: bool,
    pub workspace_checkpoints: bool,
    pub artifact_projection: bool,
}
