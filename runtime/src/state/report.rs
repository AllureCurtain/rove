use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::execution::{
    ExecutionLifecycleState, FinalOutcomeStatus, PlanDecisionRecord, PlanRevision, StepRecord,
};
use crate::prompt_metadata::PromptBuildMetadata;
use crate::runtime_identity::RuntimeIdentity;
use crate::types::{JobId, RunId, SessionId, TerminationReason};
use crate::workspace::WorkspaceKind;
use rove_core::{ToolExecutionMetadata, ToolMutation};
use rove_models::Usage;

/// Summary report for a completed run.
///
/// Written to `.rove/runs/<run_id>/report.json` after the run finishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub workspace_root: PathBuf,
    pub workspace_kind: WorkspaceKind,
    pub model_id: String,
    pub status: String,
    pub termination_reason: TerminationReason,
    pub steps: u32,
    pub total_usage: Usage,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub tool_mutations: Vec<ToolMutation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_execution_metadata: Vec<ToolExecutionMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_builds: Vec<PromptBuildMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_identity: Option<RuntimeIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_records: Vec<StepRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_decisions: Vec<PlanDecisionRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_revisions: Vec<PlanRevision>,
    #[serde(default)]
    pub execution_lifecycle: ExecutionLifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_outcome: Option<FinalOutcomeStatus>,
    pub output: Option<String>,
    pub timestamp: String,
}

impl RunReport {
    pub fn new(
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        workspace_root: PathBuf,
        workspace_kind: WorkspaceKind,
        model_id: String,
        reason: TerminationReason,
    ) -> Self {
        Self {
            session_id,
            job_id,
            run_id,
            workspace_root,
            workspace_kind,
            model_id,
            status: match &reason {
                TerminationReason::Final => "success".to_string(),
                TerminationReason::Error => "error".to_string(),
                TerminationReason::Cancelled => "cancelled".to_string(),
                _ => "incomplete".to_string(),
            },
            termination_reason: reason,
            steps: 0,
            total_usage: Usage::default(),
            tool_calls: 0,
            tool_failures: 0,
            tool_mutations: Vec::new(),
            tool_execution_metadata: Vec::new(),
            prompt_builds: Vec::new(),
            runtime_identity: None,
            step_records: Vec::new(),
            plan_decisions: Vec::new(),
            plan_revisions: Vec::new(),
            execution_lifecycle: ExecutionLifecycleState::default(),
            final_outcome: None,
            output: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Write a report to the run directory and return the artifact path.
pub fn write_report(run_dir: &Path, report: &RunReport) -> std::io::Result<PathBuf> {
    fs::create_dir_all(run_dir)?;
    let path = run_dir.join("report.json");
    let json = serde_json::to_string_pretty(report).map_err(std::io::Error::other)?;
    atomic_write(&path, json.as_bytes())?;
    Ok(path)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, bytes)?;
    fs::rename(tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::RunReport;
    use crate::types::{JobId, RunId, SessionId, TerminationReason};
    use crate::workspace::WorkspaceKind;

    #[test]
    fn legacy_report_without_step_records_deserializes() {
        let report = RunReport::new(
            SessionId::new(),
            JobId::new(),
            RunId::new(),
            std::path::PathBuf::from("."),
            WorkspaceKind::Folder,
            "fake".to_string(),
            TerminationReason::Final,
        );
        let mut value = serde_json::to_value(report).unwrap();
        value.as_object_mut().unwrap().remove("step_records");

        let report: RunReport = serde_json::from_value(value).unwrap();

        assert!(report.step_records.is_empty());
    }
}
