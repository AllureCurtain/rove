use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::execution::{
    ExecutionLifecycleState, FinalOutcomeStatus, PlanDecisionRecord, PlanRevision, StepRecord,
};
use crate::prompt_metadata::PromptBuildMetadata;
use crate::runtime_identity::RuntimeIdentity;
use crate::types::MessageDeliveryRecord;
use crate::types::{JobId, RunId, SessionId, TerminationReason};
use crate::workspace::WorkspaceKind;
use rove_core::{
    ArtifactValidation, Sensitivity, ToolArtifactKind, ToolArtifactRef, ToolExecutionMetadata,
    ToolMutation,
};
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
    /// Artifacts this run produced, by reference only.
    ///
    /// The report never copies a payload: a large artifact stays on disk and
    /// the report records how to find it and whether it is still there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_artifacts: Vec<ReportArtifactEntry>,
    /// Artifacts a quota refused, so a bounded run stays explainable after
    /// the fact rather than looking like the tool returned nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_tool_artifacts: Vec<ReportArtifactRejection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_deliveries: Vec<MessageDeliveryRecord>,
    pub output: Option<String>,
    pub timestamp: String,
}

/// One artifact as recorded in a report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportArtifactEntry {
    pub artifact_id: String,
    pub call_id: String,
    pub kind: ToolArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub byte_length: u64,
    pub sha256: String,
    pub storage_ref: String,
    /// False once retention has removed the payload. The record of the tool
    /// outcome is never rewritten, so an expired artifact is shown as expired
    /// rather than deleted from history.
    pub payload_available: bool,
    #[serde(default)]
    pub sensitivity: Sensitivity,
    #[serde(default)]
    pub validation: ArtifactValidation,
}

impl ReportArtifactEntry {
    /// Projects a reference into a report entry.
    pub fn from_ref(artifact: &ToolArtifactRef, payload_available: bool) -> Self {
        Self {
            artifact_id: artifact.artifact_id.to_string(),
            call_id: artifact.source.call_id.clone(),
            kind: artifact.kind,
            mime_type: artifact.mime_type.clone(),
            byte_length: artifact.byte_length,
            sha256: artifact.sha256.clone(),
            storage_ref: artifact.storage_ref.clone(),
            payload_available,
            sensitivity: artifact.sensitivity,
            validation: artifact.validation,
        }
    }
}

/// One refused artifact as recorded in a report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportArtifactRejection {
    pub call_id: String,
    pub block_ordinal: u32,
    pub reason: String,
    pub observed_bytes: u64,
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
            tool_artifacts: Vec::new(),
            rejected_tool_artifacts: Vec::new(),
            message_deliveries: Vec::new(),
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
    // `report.json` is a derived summary that leaves the machine when it is
    // pasted into a bug report, so it gets the same authority-first redaction
    // as the trace: a credential the process knows is removed here too, not
    // only by the export's pattern backstop.
    let json = crate::secrets::registry().redact_json_text(&json);
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

    #[test]
    fn a_known_credential_never_reaches_report_json() {
        let dir = tempfile::TempDir::new().unwrap();
        // Matches no pattern the export backstop knows: only the registry can
        // remove it, and a report is exactly the file a user pastes into a bug
        // report.
        let canary = "report-known-credential-canary-2e64";
        assert!(crate::secrets::registry().register_value(canary));
        let mut report = RunReport::new(
            SessionId::new(),
            JobId::new(),
            RunId::new(),
            std::path::PathBuf::from("."),
            WorkspaceKind::Folder,
            "fake".to_string(),
            TerminationReason::Final,
        );
        report.output = Some(format!("final answer referencing {canary}"));

        let path = super::write_report(dir.path(), &report).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        assert!(!written.contains(canary), "{written}");
        assert!(
            written.contains(crate::secrets::KNOWN_SECRET_MARKER),
            "{written}"
        );
        // The redacted report still deserializes into the same type, so the
        // report stays a derived summary rather than becoming unusable.
        let parsed: RunReport = serde_json::from_str(&written).unwrap();
        assert_eq!(
            parsed.output.unwrap(),
            format!(
                "final answer referencing {}",
                crate::secrets::KNOWN_SECRET_MARKER
            )
        );
    }
}
