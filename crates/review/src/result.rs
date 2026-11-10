//! Durable Review result envelope and the deterministic finalize pipeline.

use serde::{Deserialize, Serialize};

use rove_foundation::environment_types::{ExecutionCapabilities, ExecutionEnvironmentIdentity};
use rove_foundation::runtime_identity::{RunModelSnapshot, RuntimeIdentity};
use rove_foundation::types::TerminationReason;

use crate::finding::ReviewFinding;
use crate::target::{
    MAX_REVIEW_FINDINGS, REVIEW_RESULT_SCHEMA_VERSION, ReviewTargetSnapshot, ReviewTargetSummary,
};
use crate::tool::ReviewSubmission;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewConclusion {
    Pass,
    Findings,
    Partial,
    Stale,
    Unavailable,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, utoipa::ToSchema)]
pub struct ReviewStats {
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub duration_ms: u64,
    pub concurrency_limit: usize,
    pub findings_total: usize,
    pub truncated_findings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ReviewUnchecked {
    pub reason: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ReviewResult {
    pub schema_version: u32,
    pub review_id: String,
    pub run_id: String,
    pub session_id: String,
    pub target: ReviewTargetSummary,
    pub conclusion: ReviewConclusion,
    pub findings: Vec<ReviewFinding>,
    pub stats: ReviewStats,
    pub unchecked: Vec<ReviewUnchecked>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_snapshot: Option<RunModelSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_environment: Option<ExecutionEnvironmentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_capabilities: Option<ExecutionCapabilities>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewRuntimeEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_snapshot: Option<RunModelSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_environment: Option<ExecutionEnvironmentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_capabilities: Option<ExecutionCapabilities>,
}

impl From<&RuntimeIdentity> for ReviewRuntimeEvidence {
    fn from(identity: &RuntimeIdentity) -> Self {
        Self {
            model_snapshot: identity.run_model.clone(),
            capability_snapshot_id: identity.capability_snapshot_id.clone(),
            execution_environment: identity.execution_environment.clone(),
            execution_capabilities: identity.execution_capabilities,
        }
    }
}

/// Deterministically finalize a Review from the immutable snapshot and the
/// sanitized submission ledger. No model text is consulted for the conclusion.
#[allow(clippy::too_many_arguments)]
pub fn finalize_result(
    review_id: impl Into<String>,
    run_id: impl Into<String>,
    session_id: impl Into<String>,
    snapshot: ReviewTargetSnapshot,
    submission: Option<ReviewSubmission>,
    stale: bool,
    cancelled: bool,
    duration_ms: u64,
) -> ReviewResult {
    finalize_result_with_evidence(
        review_id,
        run_id,
        session_id,
        snapshot,
        submission,
        stale,
        cancelled,
        duration_ms,
        ReviewRuntimeEvidence::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_result_with_evidence(
    review_id: impl Into<String>,
    run_id: impl Into<String>,
    session_id: impl Into<String>,
    snapshot: ReviewTargetSnapshot,
    submission: Option<ReviewSubmission>,
    stale: bool,
    cancelled: bool,
    duration_ms: u64,
    evidence: ReviewRuntimeEvidence,
) -> ReviewResult {
    let mut unchecked = Vec::new();
    let mut bytes_scanned = 0_u64;
    for entry in &snapshot.entries {
        bytes_scanned = bytes_scanned.saturating_add(
            entry
                .snapshot_bytes
                .as_ref()
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(0),
        );
        if entry.binary {
            unchecked.push(ReviewUnchecked {
                reason: "binary".to_string(),
                paths: vec![entry.path.clone()],
            });
        } else if entry.content_truncated {
            unchecked.push(ReviewUnchecked {
                reason: "content_truncated".to_string(),
                paths: vec![entry.path.clone()],
            });
        }
        if entry.diff_truncated {
            unchecked.push(ReviewUnchecked {
                reason: "diff_truncated".to_string(),
                paths: vec![entry.path.clone()],
            });
        }
        if entry.head.hash_truncated || entry.index.hash_truncated || entry.worktree.hash_truncated
        {
            unchecked.push(ReviewUnchecked {
                reason: "hash_truncated".to_string(),
                paths: vec![entry.path.clone()],
            });
        }
    }
    if snapshot.entries_truncated > 0 {
        unchecked.push(ReviewUnchecked {
            reason: "entries_truncated".to_string(),
            paths: Vec::new(),
        });
    }
    let had_submission = submission.is_some();
    let (findings, warnings, truncated_findings) = match submission {
        Some(submission) => (
            submission.findings,
            submission.warnings,
            submission.truncated_findings,
        ),
        None => (
            Vec::new(),
            vec!["review_findings_not_submitted".to_string()],
            0,
        ),
    };
    let conclusion = if stale {
        ReviewConclusion::Stale
    } else if cancelled {
        if findings.is_empty() {
            ReviewConclusion::Cancelled
        } else {
            ReviewConclusion::Partial
        }
    } else if !had_submission {
        if snapshot.entries.is_empty() {
            ReviewConclusion::Pass
        } else {
            ReviewConclusion::Partial
        }
    } else if !findings.is_empty() {
        ReviewConclusion::Findings
    } else if !warnings.is_empty() || !unchecked.is_empty() || truncated_findings > 0 {
        ReviewConclusion::Partial
    } else {
        ReviewConclusion::Pass
    };
    let mut stats = ReviewStats {
        files_scanned: snapshot.entries.len(),
        bytes_scanned,
        duration_ms,
        concurrency_limit: 1,
        findings_total: findings.len(),
        truncated_findings,
    };
    if stats.findings_total > MAX_REVIEW_FINDINGS {
        stats.findings_total = MAX_REVIEW_FINDINGS;
    }
    ReviewResult {
        schema_version: REVIEW_RESULT_SCHEMA_VERSION,
        review_id: review_id.into(),
        run_id: run_id.into(),
        session_id: session_id.into(),
        target: snapshot.summary(),
        conclusion,
        findings,
        stats,
        unchecked,
        model_snapshot: evidence.model_snapshot,
        capability_snapshot_id: evidence.capability_snapshot_id,
        execution_environment: evidence.execution_environment,
        execution_capabilities: evidence.execution_capabilities,
        warnings,
    }
}

/// Apply the authoritative Runtime terminal fact after deterministic finding
/// finalization. Error and budget terminals take precedence over artifact
/// completeness; an incomplete otherwise-final run is conservatively partial.
pub fn apply_runtime_outcome(
    result: &mut ReviewResult,
    termination: &TerminationReason,
    runtime_durable: bool,
) {
    match termination {
        TerminationReason::Error => {
            result.conclusion = ReviewConclusion::Error;
            result.warnings.push("review_runtime_failed".to_string());
        }
        TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => {
            result.conclusion = ReviewConclusion::Partial;
            result.warnings.push("review_budget_exhausted".to_string());
        }
        TerminationReason::Final | TerminationReason::Cancelled if !runtime_durable => {
            result.conclusion = ReviewConclusion::Partial;
            result
                .warnings
                .push("review_runtime_artifacts_incomplete".to_string());
        }
        TerminationReason::Final | TerminationReason::Cancelled => {}
    }
    result.warnings.sort();
    result.warnings.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{ReviewTargetSpec, capture_target};
    use rove_foundation::root::Workspace;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    fn repo() -> (TempDir, Workspace) {
        let temp = TempDir::new().unwrap();
        git(temp.path(), &["init", "-q"]);
        git(
            temp.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(temp.path(), &["config", "user.name", "Test"]);
        fs::write(temp.path().join("a.txt"), "one\n").unwrap();
        git(temp.path(), &["add", "a.txt"]);
        git(temp.path(), &["commit", "-qm", "initial"]);
        let workspace = Workspace::open_repo(&temp.path().canonicalize().unwrap()).unwrap();
        (temp, workspace)
    }

    #[test]
    fn incomplete_content_hashes_are_reported_as_unchecked() {
        let (_temp, workspace) = repo();
        fs::write(workspace.root.join("a.txt"), "changed\n").unwrap();
        let mut snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        snapshot.entries[0].worktree.hash_truncated = true;

        let result = finalize_result("review", "run", "session", snapshot, None, false, false, 1);

        assert!(result.unchecked.iter().any(|unchecked| {
            unchecked.reason == "hash_truncated" && unchecked.paths == ["a.txt"]
        }));
    }

    #[test]
    fn runtime_outcome_precedence_is_shared_by_review_hosts() {
        let (_temp, workspace) = repo();
        let snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let base = finalize_result("review", "run", "session", snapshot, None, false, false, 1);

        let mut failed = base.clone();
        apply_runtime_outcome(&mut failed, &TerminationReason::Error, false);
        assert_eq!(failed.conclusion, ReviewConclusion::Error);
        assert!(
            failed
                .warnings
                .contains(&"review_runtime_failed".to_string())
        );
        assert!(
            !failed
                .warnings
                .contains(&"review_runtime_artifacts_incomplete".to_string())
        );

        let mut incomplete = base;
        apply_runtime_outcome(&mut incomplete, &TerminationReason::Final, false);
        assert_eq!(incomplete.conclusion, ReviewConclusion::Partial);
        assert!(
            incomplete
                .warnings
                .contains(&"review_runtime_artifacts_incomplete".to_string())
        );
    }
}
