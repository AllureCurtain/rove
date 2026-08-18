//! Snapshot-backed tools for hard read-only Review runs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use globset::Glob;
use regex::RegexBuilder;
use serde::Serialize;
use serde_json::Value;

use crate::review::{ReviewFinding, ReviewFindingInput, ReviewTargetSnapshot, sanitize_findings};
use rove_core::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_LIST_ENTRIES: usize = 512;

#[derive(Debug, Clone, Serialize)]
pub struct ReviewSubmission {
    pub findings: Vec<ReviewFinding>,
    pub warnings: Vec<String>,
    pub truncated_findings: usize,
}

/// Process-local staging authority. Only the sanitized submission is retained.
#[derive(Clone)]
pub struct ReviewSubmissionStore {
    review_id: String,
    snapshot: Arc<ReviewTargetSnapshot>,
    submission: Arc<Mutex<Option<ReviewSubmission>>>,
}

impl ReviewSubmissionStore {
    pub fn new(review_id: impl Into<String>, snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self {
            review_id: review_id.into(),
            snapshot,
            submission: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get(&self) -> Option<ReviewSubmission> {
        self.submission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn submit(&self, raw: Vec<ReviewFindingInput>) -> Result<ReviewSubmission, ToolError> {
        let mut slot = self
            .submission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            return Err(ToolError::ExecutionFailed {
                reason: "review_findings_already_submitted".to_string(),
            });
        }
        let (findings, mut warnings, truncated_findings) =
            sanitize_findings(raw, &self.snapshot, &self.review_id);
        warnings.sort();
        warnings.dedup();
        let submission = ReviewSubmission {
            findings,
            warnings,
            truncated_findings,
        };
        *slot = Some(submission.clone());
        Ok(submission)
    }
}

pub struct ReviewReadFileTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewReadFileTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewReadFileTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "read_file",
            "Read a bounded UTF-8 range from the immutable Review target snapshot.",
            "workspace.fs.read",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type":"string","minLength":1,"maxLength":4096},
                    "offset": {"type":"integer","minimum":0,"maximum":16777216},
                    "limit": {"type":"integer","minimum":1,"maximum":262144,"default":65536}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let path = required_string(&args, "path")?;
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(64 * 1024) as usize;
        let (bytes, source_truncated) = self
            .snapshot
            .read_file(path, MAX_READ_BYTES.saturating_add(offset))
            .map_err(review_error)?;
        if offset > bytes.len() {
            return Err(ToolError::InvalidInput {
                reason: "read offset exceeds snapshot file length".to_string(),
            });
        }
        let end = offset.saturating_add(limit).min(bytes.len());
        let content =
            std::str::from_utf8(&bytes[offset..end]).map_err(|_| ToolError::InvalidInput {
                reason: "read_file requires a UTF-8 snapshot file".to_string(),
            })?;
        Ok(ToolOutput::text(
            serde_json::json!({
                "path": path.replace('\\', "/"),
                "content": content,
                "offset": offset,
                "end": end,
                "total_bytes": bytes.len(),
                "version": self.snapshot.digest,
                "truncated": source_truncated || end < bytes.len()
            })
            .to_string(),
        ))
    }
}

pub struct ReviewListDirectoryTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewListDirectoryTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewListDirectoryTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "list_directory",
            "List paths present in the immutable Review target snapshot.",
            "workspace.fs.list",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","maxLength":4096},
                    "recursive":{"type":"boolean","default":true}
                },
                "additionalProperties":false
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_prefix = args.get("path").and_then(Value::as_str).unwrap_or("");
        let prefix = normalize_prefix(raw_prefix)?;
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut paths = Vec::new();
        for entry in &self.snapshot.entries {
            if !path_in_prefix(&entry.path, &prefix) {
                continue;
            }
            let relative = entry
                .path
                .strip_prefix(&prefix)
                .unwrap_or(&entry.path)
                .trim_start_matches('/');
            if relative.is_empty() || (!recursive && relative.contains('/')) {
                continue;
            }
            paths.push(serde_json::json!({
                "path": entry.path,
                "kind": "file",
                "binary": entry.binary,
                "truncated": entry.content_truncated
            }));
            if paths.len() == MAX_LIST_ENTRIES {
                break;
            }
        }
        Ok(ToolOutput::text(
            serde_json::json!({
                "path": prefix,
                "entries": paths,
                "truncated": paths.len() == MAX_LIST_ENTRIES || self.snapshot.entries_truncated > 0
            })
            .to_string(),
        ))
    }
}

pub struct ReviewGlobPathsTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewGlobPathsTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewGlobPathsTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "glob_paths",
            "Match immutable Review snapshot paths with a bounded glob.",
            "workspace.search.glob",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "pattern":{"type":"string","minLength":1,"maxLength":512},
                    "limit":{"type":"integer","minimum":1,"maximum":512,"default":100}
                },
                "required":["pattern"],
                "additionalProperties":false
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let pattern = required_string(&args, "pattern")?;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        let matcher = Glob::new(pattern)
            .map_err(|error| ToolError::InvalidInput {
                reason: format!("invalid glob: {error}"),
            })?
            .compile_matcher();
        let matches = self
            .snapshot
            .entries
            .iter()
            .filter(|entry| matcher.is_match(&entry.path))
            .take(limit)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        Ok(ToolOutput::text(
            serde_json::json!({"matches":matches,"truncated":matches.len()==limit}).to_string(),
        ))
    }
}

pub struct ReviewSearchCodeTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewSearchCodeTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewSearchCodeTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "search_code",
            "Search UTF-8 files in the immutable Review target snapshot.",
            "workspace.search.text",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string","minLength":1,"maxLength":1024},
                    "path":{"type":"string","maxLength":4096},
                    "regex":{"type":"boolean","default":false},
                    "case_insensitive":{"type":"boolean","default":false},
                    "limit":{"type":"integer","minimum":1,"maximum":100,"default":50}
                },
                "required":["query"],
                "additionalProperties":false
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let query = required_string(&args, "query")?;
        let prefix = normalize_prefix(args.get("path").and_then(Value::as_str).unwrap_or(""))?;
        let regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
        let expression = if regex {
            query.to_string()
        } else {
            regex::escape(query)
        };
        let matcher = RegexBuilder::new(&expression)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|error| ToolError::InvalidInput {
                reason: format!("invalid regex: {error}"),
            })?;
        let mut matches = Vec::new();
        for entry in &self.snapshot.entries {
            if !path_in_prefix(&entry.path, &prefix) || entry.binary {
                continue;
            }
            let Some(bytes) = entry.snapshot_bytes.as_ref() else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(bytes) else {
                continue;
            };
            for (line_index, line) in text.lines().enumerate() {
                for found in matcher.find_iter(line) {
                    matches.push(serde_json::json!({
                        "path":entry.path,
                        "line":line_index + 1,
                        "column":line[..found.start()].chars().count() + 1,
                        "text":line
                    }));
                    if matches.len() == limit {
                        break;
                    }
                }
                if matches.len() == limit {
                    break;
                }
            }
            if matches.len() == limit {
                break;
            }
        }
        Ok(ToolOutput::text(
            serde_json::json!({"matches":matches,"truncated":matches.len()==limit}).to_string(),
        ))
    }
}

pub struct ReviewRepositoryMapTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewRepositoryMapTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewRepositoryMapTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "repository_map",
            "Return a deterministic directory map of the immutable Review snapshot.",
            "workspace.repository.map",
            serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        )
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let mut directories = BTreeMap::<String, usize>::new();
        for entry in &self.snapshot.entries {
            let directory = entry
                .path
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .unwrap_or(".");
            *directories.entry(directory.to_string()).or_default() += 1;
        }
        Ok(ToolOutput::text(
            serde_json::json!({
                "source":"review_snapshot",
                "directories":directories,
                "files":self.snapshot.entries.len(),
                "digest":self.snapshot.digest,
                "truncated":self.snapshot.entries_truncated > 0
            })
            .to_string(),
        ))
    }
}

pub struct ReviewTargetDiffTool {
    snapshot: Arc<ReviewTargetSnapshot>,
}

impl ReviewTargetDiffTool {
    pub fn new(snapshot: Arc<ReviewTargetSnapshot>) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl Tool for ReviewTargetDiffTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "review_target_diff",
            "Read status and bounded diff facts from the immutable Review target snapshot.",
            "review.target.read",
            serde_json::json!({
                "type":"object",
                "properties":{"path":{"type":"string","maxLength":4096}},
                "additionalProperties":false
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(normalize_prefix)
            .transpose()?;
        let entries = self
            .snapshot
            .entries
            .iter()
            .filter(|entry| path.as_deref().is_none_or(|path| path == entry.path))
            .map(|entry| {
                serde_json::json!({
                    "path":entry.path,
                    "old_path":entry.old_path,
                    "change_kind":entry.change_kind,
                    "staged_status":entry.staged_status,
                    "worktree_status":entry.worktree_status,
                    "binary":entry.binary,
                    "content_truncated":entry.content_truncated,
                    "diff_truncated":entry.diff_truncated,
                    "diff":entry.diff
                })
            })
            .collect::<Vec<_>>();
        Ok(ToolOutput::text(
            serde_json::json!({
                "target_digest":self.snapshot.digest,
                "entries":entries,
                "entries_truncated":self.snapshot.entries_truncated
            })
            .to_string(),
        ))
    }
}

pub struct ReviewSubmitFindingsTool {
    store: ReviewSubmissionStore,
}

impl ReviewSubmitFindingsTool {
    pub fn new(store: ReviewSubmissionStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ReviewSubmitFindingsTool {
    fn schema(&self) -> ToolDescriptor {
        descriptor(
            "review_submit_findings",
            "Submit the complete bounded finding set once. This terminal tool validates, deduplicates, and redacts before persistence.",
            "review.findings.submit",
            finding_schema(),
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw = serde_json::from_value::<Vec<ReviewFindingInput>>(
            args.get("findings")
                .cloned()
                .unwrap_or_else(|| Value::Array(vec![])),
        )
        .map_err(|error| ToolError::InvalidArgs {
            reason: format!("invalid review findings: {error}"),
        })?;
        let submission = self.store.submit(raw)?;
        Ok(ToolOutput::text(
            serde_json::json!({
                "accepted":true,
                "findings":submission.findings.len(),
                "warnings":submission.warnings,
                "truncated_findings":submission.truncated_findings
            })
            .to_string(),
        ))
    }
}

fn descriptor(
    name: &str,
    description: &str,
    capability: &str,
    parameters: Value,
) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        destructive: false,
        parallel_safe: true,
        capability_id: Some(capability.to_string()),
        capability: None,
    }
}

fn required_string<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArgs {
            reason: format!("Missing required argument: {field}"),
        })
}

fn normalize_prefix(raw: &str) -> Result<String, ToolError> {
    let trimmed = raw.trim_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    crate::review::normalize_path_for_tool(trimmed).map_err(review_error)
}

fn path_in_prefix(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn review_error(error: crate::review::ReviewError) -> ToolError {
    ToolError::ExecutionFailed {
        reason: error.to_string(),
    }
}

fn finding_schema() -> Value {
    serde_json::json!({
        "type":"object",
        "properties":{
            "findings":{
                "type":"array",
                "maxItems":64,
                "items":{
                    "type":"object",
                    "properties":{
                        "severity":{"type":"string","enum":["critical","high","medium","low","info"]},
                        "confidence":{"type":"string","enum":["high","medium","low"]},
                        "category":{"type":"string","minLength":1,"maxLength":64},
                        "path":{"type":"string","minLength":1,"maxLength":4096},
                        "location":{
                            "type":"object",
                            "properties":{
                                "start_line":{"type":"integer","minimum":0,"maximum":10000000},
                                "start_col":{"type":"integer","minimum":0,"maximum":10000000},
                                "end_line":{"type":"integer","minimum":0,"maximum":10000000},
                                "end_col":{"type":"integer","minimum":0,"maximum":10000000}
                            },
                            "required":["start_line","start_col","end_line","end_col"],
                            "additionalProperties":false
                        },
                        "title":{"type":"string","minLength":1,"maxLength":200},
                        "explanation":{"type":"string","minLength":1,"maxLength":4096},
                        "evidence":{
                            "type":"array",
                            "maxItems":3,
                            "items":{
                                "type":"object",
                                "properties":{
                                    "snippet":{"type":"string","maxLength":2048},
                                    "source":{"type":"string","enum":["diff","file","artifact"]},
                                    "reference":{"type":"string","maxLength":256}
                                },
                                "required":["snippet","source"],
                                "additionalProperties":false
                            }
                        },
                        "rule":{"type":"string","maxLength":200},
                        "suggestion":{"type":"string","maxLength":2048}
                    },
                    "required":["severity","confidence","category","path","location","title","explanation"],
                    "additionalProperties":false
                }
            }
        },
        "required":["findings"],
        "additionalProperties":false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{ReviewEntry, ReviewGitState, ReviewTargetKind, ReviewTargetSpec};
    use crate::workspace::WorkspaceKind;

    fn snapshot() -> Arc<ReviewTargetSnapshot> {
        Arc::new(ReviewTargetSnapshot {
            schema_version: 1,
            spec: ReviewTargetSpec {
                kind: ReviewTargetKind::Uncommitted,
                revision: None,
            },
            workspace_kind: WorkspaceKind::Repo,
            workspace_digest: "workspace".to_string(),
            resolved_base: None,
            captured_at: "now".to_string(),
            entries: vec![ReviewEntry {
                path: "src/lib.rs".to_string(),
                old_path: None,
                change_kind: "modified".to_string(),
                staged_status: 'M',
                worktree_status: ' ',
                head: ReviewGitState::default(),
                index: ReviewGitState::default(),
                worktree: ReviewGitState::default(),
                binary: false,
                content_truncated: false,
                diff_truncated: false,
                diff: "+problem".to_string(),
                snapshot_bytes: Some(b"one\nproblem\n".to_vec()),
            }],
            entries_truncated: 0,
            digest: "digest".to_string(),
        })
    }

    #[test]
    fn submission_store_retains_only_sanitized_findings_once() {
        let store = ReviewSubmissionStore::new("review", snapshot());
        let finding = ReviewFindingInput {
            severity: crate::review::ReviewSeverity::High,
            confidence: crate::review::ReviewConfidence::High,
            category: "bug".to_string(),
            path: "src/lib.rs".to_string(),
            location: crate::review::ReviewLocation {
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 8,
            },
            title: "Problem".to_string(),
            explanation: "Authorization: Bearer secret".to_string(),
            evidence: vec![],
            rule: String::new(),
            suggestion: String::new(),
        };
        store.submit(vec![finding]).unwrap();
        let saved = store.get().unwrap();
        assert_eq!(saved.findings[0].explanation, "[redacted]");
        assert!(matches!(
            store.submit(vec![]),
            Err(ToolError::ExecutionFailed { .. })
        ));
    }
}
