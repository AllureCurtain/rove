//! Review finding contracts and the untrusted-input sanitize pipeline.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::target::{MAX_REVIEW_FINDINGS, ReviewTargetSnapshot, normalize_relative_path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLocationStatus {
    Validated,
    Unvalidated,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, utoipa::ToSchema)]
pub struct ReviewLocation {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ReviewEvidence {
    pub snippet: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ReviewFinding {
    pub finding_id: String,
    pub severity: ReviewSeverity,
    pub confidence: ReviewConfidence,
    pub category: String,
    pub path: String,
    pub location: ReviewLocation,
    pub location_status: ReviewLocationStatus,
    pub title: String,
    pub explanation: String,
    #[serde(default)]
    pub evidence: Vec<ReviewEvidence>,
    pub rule: String,
    pub suggestion: String,
    pub status: String,
}

/// Bounded, untrusted model input. It is intentionally separate from the
/// durable ReviewFinding type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewFindingInput {
    pub severity: ReviewSeverity,
    pub confidence: ReviewConfidence,
    pub category: String,
    pub path: String,
    #[serde(default)]
    pub location: ReviewLocation,
    pub title: String,
    pub explanation: String,
    #[serde(default)]
    pub evidence: Vec<ReviewEvidenceInput>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvidenceInput {
    pub snippet: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub reference: Option<String>,
}

/// Validate and sanitize untrusted model findings before they become durable
/// facts. IDs are based on stable content, not list position.
pub fn sanitize_findings(
    raw: Vec<ReviewFindingInput>,
    snapshot: &ReviewTargetSnapshot,
    review_id: &str,
) -> (Vec<ReviewFinding>, Vec<String>, usize) {
    let changed = snapshot.changed_paths();
    let mut output = Vec::new();
    let mut warnings = Vec::new();
    let mut truncated = 0;
    let mut seen = BTreeSet::new();
    for input in raw {
        if output.len() >= MAX_REVIEW_FINDINGS {
            truncated += 1;
            continue;
        }
        let path = match normalize_relative_path(&input.path) {
            Ok(path) => path,
            Err(_) => {
                warnings.push("finding_invalid_path".to_string());
                continue;
            }
        };
        if !changed.contains(&path) {
            warnings.push("finding_outside_target".to_string());
        }
        let title = redact_text(&bounded_text(&input.title, 200), snapshot);
        let category = redact_text(&bounded_text(&input.category, 64), snapshot);
        let dedup = format!(
            "{path}\0{category}\0{}\0{}",
            input.location.start_line,
            title.to_ascii_lowercase()
        );
        if !seen.insert(dedup.clone()) {
            warnings.push("finding_deduplicated".to_string());
            continue;
        }
        let location_status = location_status(snapshot, &path, &input.location);
        if location_status == ReviewLocationStatus::Invalid {
            warnings.push("finding_location_invalid".to_string());
        }
        let finding_digest = hash_bytes(format!("{review_id}\0{dedup}").as_bytes());
        let finding_id = format!(
            "rfd_{}",
            finding_digest
                .strip_prefix("sha256:")
                .unwrap_or(&finding_digest)
        );
        let evidence = input
            .evidence
            .into_iter()
            .take(3)
            .map(|evidence| ReviewEvidence {
                snippet: redact_text(&bounded_text(&evidence.snippet, 2 * 1024), snapshot),
                source: bounded_text(&evidence.source, 32),
                reference: evidence
                    .reference
                    .map(|reference| redact_text(&bounded_text(&reference, 256), snapshot)),
            })
            .collect();
        output.push(ReviewFinding {
            finding_id,
            severity: input.severity,
            confidence: input.confidence,
            category,
            path,
            location: input.location,
            location_status,
            title,
            explanation: redact_text(&bounded_text(&input.explanation, 4 * 1024), snapshot),
            evidence,
            rule: redact_text(&bounded_text(&input.rule, 200), snapshot),
            suggestion: redact_text(&bounded_text(&input.suggestion, 2 * 1024), snapshot),
            status: "open".to_string(),
        });
    }
    (output, warnings, truncated)
}

fn location_status(
    snapshot: &ReviewTargetSnapshot,
    path: &str,
    location: &ReviewLocation,
) -> ReviewLocationStatus {
    let Some(entry) = snapshot.entries.iter().find(|entry| entry.path == path) else {
        return ReviewLocationStatus::Unvalidated;
    };
    let Some(bytes) = entry.snapshot_bytes.as_ref() else {
        return ReviewLocationStatus::Unvalidated;
    };
    if entry.content_truncated || entry.binary {
        return ReviewLocationStatus::Unvalidated;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return ReviewLocationStatus::Unvalidated;
    };
    let lines = text.split('\n').collect::<Vec<_>>();
    let line_count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    if location.start_line == 0
        || location.end_line == 0
        || location.start_line > line_count
        || location.end_line > line_count
        || location.start_line > location.end_line
    {
        return ReviewLocationStatus::Invalid;
    }
    let start_line = lines[(location.start_line - 1) as usize];
    let end_line = lines[(location.end_line - 1) as usize];
    let start_max = u32::try_from(start_line.chars().count().saturating_add(1)).unwrap_or(u32::MAX);
    let end_max = u32::try_from(end_line.chars().count().saturating_add(1)).unwrap_or(u32::MAX);
    if (location.start_col > 0 && location.start_col > start_max)
        || (location.end_col > 0 && location.end_col > end_max)
        || (location.start_line == location.end_line
            && location.start_col > 0
            && location.end_col > 0
            && location.start_col > location.end_col)
    {
        return ReviewLocationStatus::Invalid;
    }
    ReviewLocationStatus::Validated
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn redact_text(text: &str, snapshot: &ReviewTargetSnapshot) -> String {
    let mut result = text.replace(&snapshot.workspace_digest, "[redacted]");
    let mut lines = Vec::new();
    for line in result.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("authorization:")
            || lower.contains("bearer ")
            || lower.contains("api_key")
            || lower.contains("api-key")
            || lower.contains("private_key")
            || lower.contains("password=")
            || lower.contains("token=")
            || lower.contains("secret=")
        {
            lines.push("[redacted]".to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        result
    } else {
        result = lines.join("\n");
        result
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
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
    fn finding_identity_is_stable_and_sensitive_text_is_redacted() {
        let (_temp, workspace) = repo();
        fs::write(workspace.root.join("a.txt"), "one\ntwo\n").unwrap();
        let snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let input = ReviewFindingInput {
            severity: ReviewSeverity::High,
            confidence: ReviewConfidence::High,
            category: "token=category-secret".to_string(),
            path: "a.txt".to_string(),
            location: ReviewLocation {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 2,
            },
            title: "token=title-secret".to_string(),
            explanation: "token=secret".to_string(),
            evidence: vec![ReviewEvidenceInput {
                snippet: "token=snippet-secret".to_string(),
                source: "diff".to_string(),
                reference: Some("token=reference-secret".to_string()),
            }],
            rule: "token=rule-secret".to_string(),
            suggestion: "token=suggestion-secret".to_string(),
        };
        let (first, warnings, _) = sanitize_findings(vec![input.clone()], &snapshot, "rev");
        let (second, _, _) = sanitize_findings(vec![input], &snapshot, "rev");
        assert_eq!(first[0].finding_id, second[0].finding_id);
        assert_eq!(first[0].category, "[redacted]");
        assert_eq!(first[0].title, "[redacted]");
        assert_eq!(first[0].explanation, "[redacted]");
        assert_eq!(first[0].evidence[0].snippet, "[redacted]");
        assert_eq!(
            first[0].evidence[0].reference.as_deref(),
            Some("[redacted]")
        );
        assert_eq!(first[0].rule, "[redacted]");
        assert_eq!(first[0].suggestion, "[redacted]");
        assert!(warnings.is_empty());
    }

    #[test]
    fn findings_are_deduplicated_bounded_and_location_checked() {
        let (_temp, workspace) = repo();
        fs::write(workspace.root.join("a.txt"), "one\ntwo\n").unwrap();
        let snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let input = ReviewFindingInput {
            severity: ReviewSeverity::Medium,
            confidence: ReviewConfidence::High,
            category: "correctness".to_string(),
            path: "a.txt".to_string(),
            location: ReviewLocation {
                start_line: 2,
                start_col: 99,
                end_line: 2,
                end_col: 100,
            },
            title: "Repeated".to_string(),
            explanation: "x".repeat(5_000),
            evidence: vec![ReviewEvidenceInput {
                snippet: "password=secret".to_string(),
                source: "diff".to_string(),
                reference: None,
            }],
            rule: "rule".to_string(),
            suggestion: "suggestion".to_string(),
        };
        let (findings, warnings, truncated) =
            sanitize_findings(vec![input.clone(), input], &snapshot, "review");
        assert_eq!(findings.len(), 1);
        assert_eq!(truncated, 0);
        assert_eq!(findings[0].location_status, ReviewLocationStatus::Invalid);
        assert_eq!(findings[0].explanation.chars().count(), 4 * 1024);
        assert_eq!(findings[0].evidence[0].snippet, "[redacted]");
        assert!(warnings.contains(&"finding_location_invalid".to_string()));
        assert!(warnings.contains(&"finding_deduplicated".to_string()));
        assert!(!findings[0].finding_id.contains(':'));
    }
}
