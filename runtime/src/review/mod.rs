//! Hard read-only code-review contracts.
//!
//! This module owns target snapshots and result validation. It does not add a
//! second agent loop. Hosts run the normal Engine with RunMode::Review, a
//! review registry, and the immutable snapshot captured here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::environment::{ExecutionCapabilities, ExecutionEnvironmentIdentity};
use crate::runtime_identity::{RunModelSnapshot, RuntimeIdentity};
use crate::types::RunMode;
use crate::types::TerminationReason;
use crate::workspace::{Workspace, WorkspaceKind};
use rove_core::ToolDescriptor;

pub const REVIEW_ALLOWED_CAPABILITIES: &[&str] = &[
    "workspace.fs.read",
    "workspace.fs.list",
    "workspace.search.glob",
    "workspace.search.text",
    "workspace.repository.map",
    "runtime.artifact.read",
    "review.target.read",
    "review.findings.submit",
];

pub const REVIEW_RESULT_SCHEMA_VERSION: u32 = 1;
pub const MAX_REVIEW_ENTRIES: usize = 512;
pub const MAX_REVIEW_DIFF_BYTES: usize = 128 * 1024;
pub const MAX_REVIEW_DIFF_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REVIEW_MATERIALIZED_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REVIEW_MATERIALIZED_TOTAL_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REVIEW_FINDINGS: usize = 64;
const MAX_GIT_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 64 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const GIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Capability IDs are not permission proofs. Review binds the exact tool name,
/// capability, and non-destructive effect class.
pub fn descriptor_allowed(descriptor: &ToolDescriptor) -> bool {
    let Some(capability) = descriptor.capability_id.as_deref() else {
        return false;
    };
    !descriptor.destructive
        && descriptor
            .capability
            .as_ref()
            .is_none_or(|capability| capability.status == "available")
        && REVIEW_ALLOWED_CAPABILITIES.contains(&capability)
        && matches!(
            (descriptor.name.as_str(), capability),
            ("read_file", "workspace.fs.read")
                | ("list_directory", "workspace.fs.list")
                | ("glob_paths", "workspace.search.glob")
                | ("search_code", "workspace.search.text")
                | ("repository_map", "workspace.repository.map")
                | ("resolve_tool_artifact", "runtime.artifact.read")
                | ("review_target_diff", "review.target.read")
                | ("review_submit_findings", "review.findings.submit")
        )
}

pub fn is_review_mode(mode: RunMode) -> bool {
    matches!(mode, RunMode::Review)
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("review target is unavailable: {0}")]
    TargetUnavailable(String),
    #[error("review target is not a Git repository")]
    NotRepository,
    #[error("Git is unavailable: {0}")]
    GitUnavailable(String),
    #[error("invalid review revision: {0}")]
    InvalidRevision(String),
    #[error("review state root is invalid: {0}")]
    InvalidStateRoot(String),
    #[error("review result is invalid: {0}")]
    InvalidResult(String),
    #[error("review findings were already submitted")]
    FindingsAlreadySubmitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTargetKind {
    Uncommitted,
    Base,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTargetSpec {
    pub kind: ReviewTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl Default for ReviewTargetSpec {
    fn default() -> Self {
        Self {
            kind: ReviewTargetKind::Uncommitted,
            revision: None,
        }
    }
}

impl ReviewTargetSpec {
    pub fn uncommitted() -> Self {
        Self::default()
    }

    pub fn base(revision: impl Into<String>) -> Self {
        Self {
            kind: ReviewTargetKind::Base,
            revision: Some(revision.into()),
        }
    }

    pub fn commit(revision: impl Into<String>) -> Self {
        Self {
            kind: ReviewTargetKind::Commit,
            revision: Some(revision.into()),
        }
    }

    fn validate(&self) -> Result<(), ReviewError> {
        match self.kind {
            ReviewTargetKind::Uncommitted if self.revision.is_some() => Err(
                ReviewError::InvalidRevision("uncommitted target does not accept revision".into()),
            ),
            ReviewTargetKind::Base | ReviewTargetKind::Commit
                if self
                    .revision
                    .as_deref()
                    .is_none_or(|revision| revision.trim().is_empty()) =>
            {
                Err(ReviewError::InvalidRevision(
                    "base/commit target requires revision".into(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewGitState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub exists: bool,
    #[serde(default)]
    pub hash_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub change_kind: String,
    pub staged_status: char,
    pub worktree_status: char,
    pub head: ReviewGitState,
    pub index: ReviewGitState,
    pub worktree: ReviewGitState,
    pub binary: bool,
    #[serde(default)]
    pub content_truncated: bool,
    #[serde(default)]
    pub diff_truncated: bool,
    pub diff: String,
    /// Bounded immutable bytes used by Review read tools and location checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewTargetSnapshot {
    pub schema_version: u32,
    pub spec: ReviewTargetSpec,
    pub workspace_kind: WorkspaceKind,
    pub workspace_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_base: Option<String>,
    pub captured_at: String,
    pub entries: Vec<ReviewEntry>,
    #[serde(default)]
    pub entries_truncated: usize,
    pub digest: String,
}

impl ReviewTargetSnapshot {
    pub fn read_file(&self, path: &str, max_bytes: usize) -> Result<(Vec<u8>, bool), ReviewError> {
        let normalized = normalize_relative_path(path)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == normalized)
            .ok_or_else(|| {
                ReviewError::TargetUnavailable(format!("path is not in snapshot: {normalized}"))
            })?;
        let Some(bytes) = entry.snapshot_bytes.as_ref() else {
            return Err(ReviewError::TargetUnavailable(
                "file is binary, deleted, or not materialized".to_string(),
            ));
        };
        let truncated = entry.content_truncated || bytes.len() > max_bytes;
        Ok((bytes[..bytes.len().min(max_bytes)].to_vec(), truncated))
    }

    pub fn current_digest(&self, workspace: &Workspace) -> Result<String, ReviewError> {
        capture_target(workspace, self.spec.clone()).map(|snapshot| snapshot.digest)
    }

    pub fn is_stale(&self, workspace: &Workspace) -> Result<bool, ReviewError> {
        Ok(self.current_digest(workspace)? != self.digest)
    }

    pub fn changed_paths(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn summary(&self) -> ReviewTargetSummary {
        ReviewTargetSummary {
            schema_version: self.schema_version,
            spec: self.spec.clone(),
            workspace_kind: self.workspace_kind.clone(),
            workspace_digest: self.workspace_digest.clone(),
            resolved_base: self.resolved_base.clone(),
            captured_at: self.captured_at.clone(),
            entries: self.entries.len(),
            entries_truncated: self.entries_truncated,
            digest: self.digest.clone(),
        }
    }
}

/// Secret-free, bounded target projection used by CLI, API, Web, and durable
/// Review results. Materialized bytes and diff payloads remain in the external
/// Review snapshot store only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewTargetSummary {
    pub schema_version: u32,
    pub spec: ReviewTargetSpec,
    pub workspace_kind: WorkspaceKind,
    pub workspace_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_base: Option<String>,
    pub captured_at: String,
    pub entries: usize,
    #[serde(default)]
    pub entries_truncated: usize,
    pub digest: String,
}

/// Capture an immutable target snapshot. This function never invokes a model
/// tool and never writes to the repository or its index.
pub fn capture_target(
    workspace: &Workspace,
    spec: ReviewTargetSpec,
) -> Result<ReviewTargetSnapshot, ReviewError> {
    spec.validate()?;
    if workspace.kind != WorkspaceKind::Repo || !workspace.root.join(".git").exists() {
        return Err(ReviewError::NotRepository);
    }

    let resolved_base = match spec.kind {
        ReviewTargetKind::Uncommitted => None,
        ReviewTargetKind::Base | ReviewTargetKind::Commit => Some(resolve_revision(
            workspace,
            spec.revision.as_deref().unwrap_or_default(),
        )?),
    };
    let mut statuses = match spec.kind {
        ReviewTargetKind::Uncommitted | ReviewTargetKind::Base => status_records(workspace)?,
        ReviewTargetKind::Commit => BTreeMap::new(),
    };
    let diff_paths = match spec.kind {
        ReviewTargetKind::Uncommitted => diff_name_status(workspace, &["HEAD"])?,
        ReviewTargetKind::Base => {
            diff_name_status(workspace, &[resolved_base.as_deref().unwrap_or("HEAD")])?
        }
        ReviewTargetKind::Commit => {
            let commit = resolved_base.as_deref().unwrap_or_default();
            let parent = format!("{commit}^");
            diff_name_status(workspace, &[&parent, commit])?
        }
    };
    let base_diff_paths = matches!(spec.kind, ReviewTargetKind::Base)
        .then(|| diff_paths.keys().cloned().collect::<BTreeSet<String>>());
    for (path, status) in diff_paths {
        if matches!(spec.kind, ReviewTargetKind::Base)
            && let Some(existing) = statuses.get_mut(&path)
        {
            // Preserve the live staged/worktree status, but retain the base
            // side's rename/status facts so the target remains base→worktree
            // rather than silently collapsing to HEAD→worktree.
            existing.diff_status = status.diff_status;
            if status.old_path.is_some() {
                existing.old_path = status.old_path.clone();
                existing.head_path = status.old_path.clone();
            }
        } else {
            statuses.entry(path).or_insert(status);
        }
    }
    if let Some(base_diff_paths) = base_diff_paths {
        // A live status is relative to HEAD, not the requested base. Exclude a
        // tracked path whose worktree bytes have returned to the base state;
        // untracked paths remain explicit because `git diff <base>` omits them.
        statuses.retain(|path, status| {
            base_diff_paths.contains(path) || status.staged == '?' || status.worktree == '?'
        });
    }

    let mut entries = Vec::new();
    let mut entries_truncated = 0;
    let mut remaining_diff_bytes = MAX_REVIEW_DIFF_TOTAL_BYTES;
    let mut remaining_materialized_bytes = MAX_REVIEW_MATERIALIZED_TOTAL_BYTES;
    for (path, status) in statuses {
        if entries.len() >= MAX_REVIEW_ENTRIES {
            entries_truncated += 1;
            continue;
        }
        let entry = capture_entry(
            workspace,
            &spec,
            resolved_base.as_deref(),
            &path,
            &status,
            remaining_diff_bytes,
            remaining_materialized_bytes,
        )?;
        remaining_diff_bytes = remaining_diff_bytes.saturating_sub(entry.diff.len());
        remaining_materialized_bytes = remaining_materialized_bytes.saturating_sub(
            entry
                .snapshot_bytes
                .as_ref()
                .map(Vec::len)
                .unwrap_or_default(),
        );
        entries.push(entry);
    }
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let workspace_digest = workspace_digest(workspace);
    let digest = target_digest(
        &workspace_digest,
        &spec,
        resolved_base.as_deref(),
        &entries,
        entries_truncated,
    );
    Ok(ReviewTargetSnapshot {
        schema_version: 1,
        spec,
        workspace_kind: workspace.kind.clone(),
        workspace_digest,
        resolved_base,
        captured_at: now_rfc3339(),
        entries,
        entries_truncated,
        digest,
    })
}

#[derive(Debug, Clone)]
struct GitStatus {
    staged: char,
    worktree: char,
    old_path: Option<String>,
    head_path: Option<String>,
    /// Status from an explicit revision diff (base/commit), kept separate
    /// from the live index/worktree porcelain columns.
    diff_status: Option<char>,
}

fn capture_entry(
    workspace: &Workspace,
    spec: &ReviewTargetSpec,
    resolved_base: Option<&str>,
    path: &str,
    status: &GitStatus,
    remaining_diff_bytes: usize,
    remaining_materialized_bytes: usize,
) -> Result<ReviewEntry, ReviewError> {
    let sensitive = crate::environment::is_sensitive_traversal_path(path);
    let materialized_limit = if sensitive {
        0
    } else {
        remaining_materialized_bytes.min(MAX_REVIEW_MATERIALIZED_FILE_BYTES)
    };
    let (head, index, worktree, snapshot_bytes, mut content_truncated) = match spec.kind {
        ReviewTargetKind::Commit => {
            let commit = resolved_base
                .ok_or_else(|| ReviewError::InvalidRevision("missing commit".into()))?;
            let parent = format!("{commit}^");
            let old_path = status.old_path.as_deref().unwrap_or(path);
            let old = capture_git_object(workspace, &parent, old_path, 0)?;
            let new = capture_git_object(workspace, commit, path, materialized_limit)?;
            (
                old.state,
                ReviewGitState::default(),
                new.state,
                new.bytes,
                new.content_truncated,
            )
        }
        ReviewTargetKind::Uncommitted | ReviewTargetKind::Base => {
            let base_object = match spec.kind {
                ReviewTargetKind::Uncommitted => "HEAD",
                ReviewTargetKind::Base => resolved_base.unwrap_or("HEAD"),
                ReviewTargetKind::Commit => unreachable!(),
            };
            let head_path = status
                .head_path
                .as_deref()
                .or(status.old_path.as_deref())
                .unwrap_or(path);
            let head = capture_git_object(workspace, base_object, head_path, 0)?;
            let index = capture_git_object(workspace, "", path, 0)?;
            let worktree = capture_worktree_file(workspace, path, materialized_limit)?;
            (
                head.state,
                index.state,
                worktree.state,
                worktree.bytes,
                worktree.content_truncated,
            )
        }
    };
    if sensitive {
        content_truncated = true;
    }
    let snapshot_bytes = (!sensitive).then_some(snapshot_bytes).flatten();
    let mut binary = snapshot_bytes
        .as_deref()
        .is_some_and(|bytes| bytes.contains(&0))
        || status.staged == 'B'
        || status.worktree == 'B';
    let diff_args: Vec<String> = match spec.kind {
        ReviewTargetKind::Uncommitted => vec!["HEAD".into()],
        ReviewTargetKind::Base => vec![resolved_base.unwrap_or("HEAD").into()],
        ReviewTargetKind::Commit => vec![
            format!("{}^", resolved_base.unwrap_or_default()),
            resolved_base.unwrap_or_default().into(),
        ],
    };
    let diff_limit = remaining_diff_bytes.min(MAX_REVIEW_DIFF_BYTES);
    let (diff, mut diff_truncated) = if sensitive {
        (String::new(), true)
    } else {
        bounded_diff(
            workspace,
            &diff_args,
            path,
            snapshot_bytes.as_deref(),
            status.staged == '?' || status.worktree == '?',
            content_truncated,
            diff_limit,
        )
    };
    binary = binary
        || diff.contains("GIT binary patch")
        || diff.contains("Binary files ")
        || diff.contains("Binary file ");
    if binary && diff.contains("GIT binary patch") {
        diff_truncated = true;
    }
    let change_kind = status_kind(
        status.staged,
        status.worktree,
        status.diff_status,
        !worktree.exists,
        !head.exists,
    );
    Ok(ReviewEntry {
        path: path.to_string(),
        old_path: status.old_path.clone(),
        change_kind,
        staged_status: status.staged,
        worktree_status: status.worktree,
        head,
        index,
        worktree,
        binary,
        content_truncated,
        diff_truncated,
        diff,
        snapshot_bytes,
    })
}

fn status_kind(
    staged: char,
    worktree: char,
    diff_status: Option<char>,
    missing_worktree: bool,
    missing_old: bool,
) -> String {
    if staged == '?' || worktree == '?' {
        return "untracked".to_string();
    }
    if staged == 'R' || worktree == 'R' {
        return "renamed".to_string();
    }
    if staged == 'D' || worktree == 'D' || missing_worktree {
        return "deleted".to_string();
    }
    if staged == 'B' || worktree == 'B' {
        return "binary".to_string();
    }
    if staged != ' ' && worktree == ' ' {
        return "staged".to_string();
    }
    if worktree != ' ' && staged == ' ' {
        return "unstaged".to_string();
    }
    if missing_old {
        return "added".to_string();
    }
    if let Some(status) = diff_status {
        return match status {
            'A' => "added".to_string(),
            'D' => "deleted".to_string(),
            'R' | 'C' => "renamed".to_string(),
            'B' => "binary".to_string(),
            _ => "modified".to_string(),
        };
    }
    "modified".to_string()
}

fn status_records(workspace: &Workspace) -> Result<BTreeMap<String, GitStatus>, ReviewError> {
    let bytes = git_output_bytes(
        workspace,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut records = BTreeMap::new();
    let mut index = 0;
    while index < bytes.len() {
        let end = bytes[index..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| index + offset)
            .unwrap_or(bytes.len());
        if end <= index {
            index = end.saturating_add(1);
            continue;
        }
        let record = String::from_utf8_lossy(&bytes[index..end]);
        let (staged, worktree, path) = parse_status_record(&record)?;
        index = end.saturating_add(1);
        let old_path = if staged == 'R' || worktree == 'R' || staged == 'C' || worktree == 'C' {
            let old_end = bytes[index..]
                .iter()
                .position(|byte| *byte == 0)
                .map(|offset| index + offset)
                .unwrap_or(bytes.len());
            let old = String::from_utf8_lossy(&bytes[index..old_end]).to_string();
            index = old_end.saturating_add(1);
            Some(old)
        } else {
            None
        };
        records.insert(
            normalize_relative_path(&path)?,
            GitStatus {
                staged,
                worktree,
                head_path: old_path.clone(),
                old_path,
                diff_status: None,
            },
        );
    }
    Ok(records)
}

fn parse_status_record(record: &str) -> Result<(char, char, String), ReviewError> {
    let bytes = record.as_bytes();
    if bytes.len() < 4 || bytes[2] != b' ' {
        return Err(ReviewError::GitUnavailable(
            "malformed porcelain status".to_string(),
        ));
    }
    Ok((bytes[0] as char, bytes[1] as char, record[3..].to_string()))
}

fn diff_name_status(
    workspace: &Workspace,
    revisions: &[&str],
) -> Result<BTreeMap<String, GitStatus>, ReviewError> {
    let mut args = vec![
        "diff",
        "--name-status",
        "-z",
        "--find-renames",
        "--no-ext-diff",
    ];
    args.extend(revisions.iter().copied());
    args.push("--");
    let bytes = git_output_bytes(workspace, &args)?;
    let mut result = BTreeMap::new();
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    while let Some(status_field) = fields.next() {
        let status = String::from_utf8_lossy(status_field);
        let code = status.chars().next().unwrap_or('M');
        let first_path = fields
            .next()
            .ok_or_else(|| ReviewError::GitUnavailable("malformed diff status".to_string()))?;
        let (path, old_path) = if code == 'R' || code == 'C' {
            let old_path = normalize_relative_path(&String::from_utf8_lossy(first_path))?;
            let new_path =
                normalize_relative_path(&String::from_utf8_lossy(fields.next().ok_or_else(
                    || ReviewError::GitUnavailable("malformed rename status".to_string()),
                )?))?;
            (new_path, Some(old_path))
        } else {
            (
                normalize_relative_path(&String::from_utf8_lossy(first_path))?,
                None,
            )
        };
        result.insert(
            path,
            GitStatus {
                staged: ' ',
                worktree: ' ',
                head_path: None,
                old_path,
                diff_status: Some(code),
            },
        );
    }
    Ok(result)
}

fn bounded_diff(
    workspace: &Workspace,
    revisions: &[String],
    path: &str,
    snapshot_bytes: Option<&[u8]>,
    untracked: bool,
    snapshot_incomplete: bool,
    limit: usize,
) -> (String, bool) {
    if limit == 0 {
        return (String::new(), true);
    }
    let mut args = vec![
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--binary".to_string(),
        "--unified=20".to_string(),
    ];
    args.extend(revisions.iter().cloned());
    args.push("--".to_string());
    args.push(path.to_string());
    let string_args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let capture = run_git_capture(workspace, &string_args, limit);
    let (bytes, command_truncated) = match capture {
        Ok(capture) => (capture.bytes, capture.truncated),
        Err(_) => (Vec::new(), true),
    };
    if untracked && bytes.is_empty() && snapshot_bytes.is_some() {
        let raw = snapshot_bytes.unwrap_or_default();
        if raw.contains(&0) || std::str::from_utf8(raw).is_err() {
            return (
                "Binary change recorded; content omitted.".to_string(),
                snapshot_incomplete,
            );
        }
        let content = String::from_utf8_lossy(raw);
        let mut synthetic = format!(
            "--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
            content.lines().count().max(1)
        );
        for line in content.lines() {
            synthetic.push('+');
            synthetic.push_str(line);
            synthetic.push('\n');
            if synthetic.len() > limit {
                break;
            }
        }
        let (synthetic, truncated) = truncate_utf8_bytes(synthetic, limit);
        return (synthetic, truncated || snapshot_incomplete);
    }
    (
        String::from_utf8_lossy(&bytes).to_string(),
        command_truncated,
    )
}

#[derive(Debug)]
struct ObjectCapture {
    state: ReviewGitState,
    bytes: Option<Vec<u8>>,
    content_truncated: bool,
}

impl ObjectCapture {
    fn missing() -> Self {
        Self {
            state: ReviewGitState::default(),
            bytes: None,
            content_truncated: false,
        }
    }

    fn unknown() -> Self {
        Self {
            state: ReviewGitState {
                hash: None,
                exists: false,
                hash_truncated: true,
            },
            bytes: None,
            content_truncated: true,
        }
    }
}

fn capture_git_object(
    workspace: &Workspace,
    object: &str,
    path: &str,
    materialized_limit: usize,
) -> Result<ObjectCapture, ReviewError> {
    let spec = if object.is_empty() {
        format!(":{path}")
    } else {
        format!("{object}:{path}")
    };
    match run_git_capture(workspace, &["cat-file", "blob", &spec], materialized_limit) {
        Ok(capture) => Ok(ObjectCapture {
            state: ReviewGitState {
                hash: Some(capture.hash),
                exists: true,
                hash_truncated: false,
            },
            bytes: Some(capture.bytes),
            content_truncated: capture.truncated,
        }),
        Err(GitCommandFailure::Exit(_)) => Ok(ObjectCapture::missing()),
        Err(GitCommandFailure::Timeout) => Ok(ObjectCapture::unknown()),
        Err(error) => Err(git_failure(error)),
    }
}

fn capture_worktree_file(
    workspace: &Workspace,
    path: &str,
    materialized_limit: usize,
) -> Result<ObjectCapture, ReviewError> {
    let target = workspace.root.join(path);
    let metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ObjectCapture::missing());
        }
        Err(error) => return Err(ReviewError::TargetUnavailable(error.to_string())),
    };
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(&target)
            .map_err(|error| ReviewError::TargetUnavailable(error.to_string()))?;
        let bytes = link.to_string_lossy().as_bytes().to_vec();
        return Ok(object_from_bytes(bytes, materialized_limit));
    }
    if !metadata.is_file() {
        return Ok(ObjectCapture::unknown());
    }
    let canonical_root = workspace
        .root
        .canonicalize()
        .unwrap_or_else(|_| workspace.root.clone());
    let canonical_target = target
        .canonicalize()
        .map_err(|error| ReviewError::TargetUnavailable(error.to_string()))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Ok(ObjectCapture::unknown());
    }
    let file = fs::File::open(&canonical_target)
        .map_err(|error| ReviewError::TargetUnavailable(error.to_string()))?;
    let capture = read_bounded_and_hash(file, materialized_limit, Some(GIT_TIMEOUT))
        .map_err(|error| ReviewError::TargetUnavailable(error.to_string()))?;
    Ok(ObjectCapture {
        state: ReviewGitState {
            hash: capture.complete.then_some(capture.hash),
            exists: true,
            hash_truncated: !capture.complete,
        },
        bytes: Some(capture.bytes),
        content_truncated: capture.truncated || !capture.complete,
    })
}

fn object_from_bytes(bytes: Vec<u8>, materialized_limit: usize) -> ObjectCapture {
    let hash = hash_bytes(&bytes);
    let truncated = bytes.len() > materialized_limit;
    ObjectCapture {
        state: ReviewGitState {
            hash: Some(hash),
            exists: true,
            hash_truncated: false,
        },
        bytes: Some(bytes.into_iter().take(materialized_limit).collect()),
        content_truncated: truncated,
    }
}

fn resolve_revision(workspace: &Workspace, revision: &str) -> Result<String, ReviewError> {
    let revision = revision.trim();
    if revision.is_empty() || revision.starts_with('-') || revision.contains('\0') {
        return Err(ReviewError::InvalidRevision(
            "revision must be a non-option token".to_string(),
        ));
    }
    let expression = format!("{revision}^{{commit}}");
    let bytes = git_output_bytes(
        workspace,
        &["rev-parse", "--verify", "--end-of-options", &expression],
    )?;
    let resolved = String::from_utf8_lossy(&bytes).trim().to_string();
    if resolved.len() != 40 || !resolved.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ReviewError::InvalidRevision(
            "Git did not return a commit id".to_string(),
        ));
    }
    Ok(resolved)
}

fn git_output_bytes(workspace: &Workspace, args: &[&str]) -> Result<Vec<u8>, ReviewError> {
    let capture = run_git_capture(workspace, args, MAX_GIT_METADATA_BYTES).map_err(git_failure)?;
    if capture.truncated {
        return Err(ReviewError::TargetUnavailable(
            "Git metadata exceeded the Review capture limit".to_string(),
        ));
    }
    Ok(capture.bytes)
}

#[derive(Debug)]
struct BoundedCapture {
    bytes: Vec<u8>,
    truncated: bool,
    hash: String,
    complete: bool,
}

#[derive(Debug)]
enum GitCommandFailure {
    Spawn(String),
    Io(String),
    Timeout,
    Exit(Vec<u8>),
}

fn run_git_capture(
    workspace: &Workspace,
    args: &[&str],
    stdout_limit: usize,
) -> Result<BoundedCapture, GitCommandFailure> {
    let mut child = Command::new("git")
        .current_dir(&workspace.root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(args)
        .spawn()
        .map_err(|error| GitCommandFailure::Spawn(error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| GitCommandFailure::Io("Git stdout pipe was not available".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| GitCommandFailure::Io("Git stderr pipe was not available".to_string()))?;
    let stdout_reader = thread::spawn(move || read_bounded_and_hash(stdout, stdout_limit, None));
    let stderr_reader =
        thread::spawn(move || read_bounded_and_hash(stderr, MAX_GIT_STDERR_BYTES, None));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < GIT_TIMEOUT => thread::sleep(GIT_POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitCommandFailure::Io(error.to_string()));
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| GitCommandFailure::Io("Git stdout reader panicked".to_string()))?
        .map_err(|error| GitCommandFailure::Io(error.to_string()))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| GitCommandFailure::Io("Git stderr reader panicked".to_string()))?
        .map_err(|error| GitCommandFailure::Io(error.to_string()))?;
    let Some(status) = status else {
        return Err(GitCommandFailure::Timeout);
    };
    if !status.success() {
        return Err(GitCommandFailure::Exit(stderr.bytes));
    }
    Ok(stdout)
}

fn read_bounded_and_hash(
    mut reader: impl Read,
    limit: usize,
    deadline: Option<Duration>,
) -> std::io::Result<BoundedCapture> {
    let started = Instant::now();
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut hasher = Sha256::new();
    let mut total = 0_usize;
    let mut complete = true;
    loop {
        if deadline.is_some_and(|deadline| started.elapsed() >= deadline) {
            complete = false;
            break;
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read);
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(BoundedCapture {
        bytes,
        truncated: total > limit,
        hash: format!("sha256:{:x}", hasher.finalize()),
        complete,
    })
}

fn git_failure(error: GitCommandFailure) -> ReviewError {
    match error {
        GitCommandFailure::Spawn(message) | GitCommandFailure::Io(message) => {
            ReviewError::GitUnavailable(message)
        }
        GitCommandFailure::Timeout => {
            ReviewError::TargetUnavailable("Git command timed out".to_string())
        }
        GitCommandFailure::Exit(stderr) => ReviewError::TargetUnavailable(
            crate::context::prompt_metadata::stable_hash(String::from_utf8_lossy(&stderr).trim()),
        ),
    }
}

fn truncate_utf8_bytes(mut value: String, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value, false);
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value.truncate(end);
    (value, true)
}

fn normalize_relative_path(path: &str) -> Result<String, ReviewError> {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .next()
            .is_some_and(|component| component.contains(':'))
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ReviewError::InvalidResult(
            "path is not workspace-relative".to_string(),
        ));
    }
    Ok(path.to_string())
}

/// Normalize a user-supplied snapshot path for the built-in Review tools.
/// Keeping this wrapper crate-visible prevents tool implementations from
/// reimplementing the workspace-boundary rules independently.
pub(crate) fn normalize_path_for_tool(path: &str) -> Result<String, ReviewError> {
    normalize_relative_path(path)
}

fn workspace_digest(workspace: &Workspace) -> String {
    crate::runtime_identity::workspace_fingerprint(workspace)
}

fn target_digest(
    workspace_digest: &str,
    spec: &ReviewTargetSpec,
    resolved_base: Option<&str>,
    entries: &[ReviewEntry],
    entries_truncated: usize,
) -> String {
    #[derive(Serialize)]
    struct DigestEntry<'a> {
        path: &'a str,
        old_path: Option<&'a str>,
        change_kind: &'a str,
        staged_status: char,
        worktree_status: char,
        head_hash: Option<&'a str>,
        head_exists: bool,
        head_hash_truncated: bool,
        index_hash: Option<&'a str>,
        index_exists: bool,
        index_hash_truncated: bool,
        worktree_hash: Option<&'a str>,
        worktree_exists: bool,
        worktree_hash_truncated: bool,
        binary: bool,
        content_truncated: bool,
        diff_truncated: bool,
    }
    let values = entries
        .iter()
        .map(|entry| DigestEntry {
            path: &entry.path,
            old_path: entry.old_path.as_deref(),
            change_kind: &entry.change_kind,
            staged_status: entry.staged_status,
            worktree_status: entry.worktree_status,
            head_hash: entry.head.hash.as_deref(),
            head_exists: entry.head.exists,
            head_hash_truncated: entry.head.hash_truncated,
            index_hash: entry.index.hash.as_deref(),
            index_exists: entry.index.exists,
            index_hash_truncated: entry.index.hash_truncated,
            worktree_hash: entry.worktree.hash.as_deref(),
            worktree_exists: entry.worktree.exists,
            worktree_hash_truncated: entry.worktree.hash_truncated,
            binary: entry.binary,
            content_truncated: entry.content_truncated,
            diff_truncated: entry.diff_truncated,
        })
        .collect::<Vec<_>>();
    let canonical = serde_json::json!({
        "workspace_digest": workspace_digest,
        "kind": spec.kind,
        "resolved_base": resolved_base,
        "entries": values,
        "entries_truncated": entries_truncated,
    });
    hash_bytes(
        serde_json::to_string(&canonical)
            .unwrap_or_default()
            .as_bytes(),
    )
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Resolve a Review state root that cannot be the target workspace or a child
/// of it. The default is outside the workspace.
pub fn resolve_external_state_root(
    workspace: &Workspace,
    requested: Option<&Path>,
) -> Result<PathBuf, ReviewError> {
    let candidate = requested
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::temp_dir().join("rove-review-state"));
    if !candidate.is_absolute() {
        return Err(ReviewError::InvalidStateRoot(
            "state root must be absolute".to_string(),
        ));
    }
    let workspace_root = workspace
        .root
        .canonicalize()
        .unwrap_or_else(|_| workspace.root.clone());
    let candidate_resolved = resolve_existing_ancestor(&candidate)?;
    if path_is_same_or_child(&candidate_resolved, &workspace_root) {
        return Err(ReviewError::InvalidStateRoot(
            "Review state must be outside the target workspace".to_string(),
        ));
    }
    Ok(candidate_resolved)
}

fn path_is_same_or_child(candidate: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalize = |path: &Path| {
            path.to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        let candidate = normalize(candidate);
        let root = normalize(root);
        candidate == root
            || candidate
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('\\'))
    }
    #[cfg(not(windows))]
    {
        candidate == root || candidate.starts_with(root)
    }
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, ReviewError> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let name = ancestor.file_name().ok_or_else(|| {
            ReviewError::InvalidStateRoot("state root has no existing ancestor".to_string())
        })?;
        suffix.push(name.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| {
            ReviewError::InvalidStateRoot("state root has no existing ancestor".to_string())
        })?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| {
        ReviewError::InvalidStateRoot(format!("state root cannot be resolved: {error}"))
    })?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLocationStatus {
    Validated,
    Unvalidated,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewLocation {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvidence {
    pub snippet: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewStats {
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub duration_ms: u64,
    pub concurrency_limit: usize,
    pub findings_total: usize,
    pub truncated_findings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewUnchecked {
    pub reason: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Deterministically finalize a Review from the immutable snapshot and the
/// sanitized submission ledger. No model text is consulted for the conclusion.
#[allow(clippy::too_many_arguments)]
pub fn finalize_result(
    review_id: impl Into<String>,
    run_id: impl Into<String>,
    session_id: impl Into<String>,
    snapshot: ReviewTargetSnapshot,
    submission: Option<crate::tools::review::ReviewSubmission>,
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
    submission: Option<crate::tools::review::ReviewSubmission>,
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn review_descriptor_allowlist_binds_name_capability_and_effect_class() {
        let descriptor =
            |name: &str, capability_id: Option<&str>, destructive: bool| ToolDescriptor {
                name: name.to_string(),
                description: "test".to_string(),
                parameters: serde_json::json!({"type":"object"}),
                destructive,
                parallel_safe: true,
                capability_id: capability_id.map(str::to_string),
                capability: None,
            };
        assert!(descriptor_allowed(&descriptor(
            "read_file",
            Some("workspace.fs.read"),
            false,
        )));
        assert!(!descriptor_allowed(&descriptor(
            "write_file",
            Some("workspace.fs.read"),
            false,
        )));
        assert!(!descriptor_allowed(&descriptor(
            "read_file",
            Some("workspace.fs.write"),
            false,
        )));
        assert!(!descriptor_allowed(&descriptor(
            "read_file",
            Some("workspace.fs.read"),
            true,
        )));
        assert!(!descriptor_allowed(&descriptor("read_file", None, false)));

        let mut unavailable = descriptor("read_file", Some("workspace.fs.read"), false);
        unavailable.capability = Some(rove_core::ToolCapability {
            status: "unavailable".to_string(),
            feature: None,
            message: None,
        });
        assert!(!descriptor_allowed(&unavailable));
    }

    #[test]
    fn captures_staged_unstaged_and_untracked_with_independent_states() {
        let (_temp, workspace) = repo();
        fs::write(workspace.root.join("a.txt"), "staged\n").unwrap();
        git(&workspace.root, &["add", "a.txt"]);
        fs::write(workspace.root.join("a.txt"), "worktree\n").unwrap();
        fs::write(workspace.root.join("new.txt"), "new\n").unwrap();
        let snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let staged = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == "a.txt")
            .unwrap();
        assert_ne!(staged.head.hash, staged.index.hash);
        assert_ne!(staged.index.hash, staged.worktree.hash);
        let untracked = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == "new.txt")
            .unwrap();
        assert!(!untracked.head.exists);
        assert!(!untracked.index.exists);
        assert!(untracked.worktree.exists);
        assert_eq!(untracked.worktree.hash, Some(hash_bytes(b"new\n")));
        assert!(untracked.diff.contains("+++ b/new.txt"));
    }

    #[test]
    fn captures_rename_direction_and_commit_target_without_live_worktree_bytes() {
        let (_temp, workspace) = repo();
        git(&workspace.root, &["mv", "a.txt", "renamed.txt"]);
        let uncommitted = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let renamed = uncommitted
            .entries
            .iter()
            .find(|entry| entry.path == "renamed.txt")
            .unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("a.txt"));
        assert_eq!(renamed.change_kind, "renamed");
        assert!(renamed.head.exists);
        assert!(renamed.index.exists);

        git(&workspace.root, &["commit", "-qm", "rename"]);
        let commit = resolve_revision(&workspace, "HEAD").unwrap();
        let committed = capture_target(&workspace, ReviewTargetSpec::commit(commit)).unwrap();
        let renamed = committed
            .entries
            .iter()
            .find(|entry| entry.path == "renamed.txt")
            .unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("a.txt"));
        assert!(renamed.head.exists);
        assert!(!renamed.index.exists);
        assert!(renamed.worktree.exists);
        assert_eq!(renamed.snapshot_bytes.as_deref(), Some(b"one\n".as_slice()));
    }

    #[test]
    fn base_target_uses_the_resolved_base_object_not_head() {
        let (_temp, workspace) = repo();
        let base = String::from_utf8(
            Command::new("git")
                .current_dir(&workspace.root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        fs::write(workspace.root.join("a.txt"), "second\n").unwrap();
        git(&workspace.root, &["add", "a.txt"]);
        git(&workspace.root, &["commit", "-qm", "second"]);

        let snapshot = capture_target(&workspace, ReviewTargetSpec::base(base)).unwrap();
        let entry = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == "a.txt")
            .expect("base diff should include the committed change");
        assert_eq!(entry.head.hash, Some(hash_bytes(b"one\n")));
        assert_eq!(entry.worktree.hash, Some(hash_bytes(b"second\n")));
        assert_eq!(entry.change_kind, "modified");
        assert_eq!(entry.staged_status, ' ');
        assert_eq!(entry.worktree_status, ' ');
    }

    #[test]
    fn base_target_excludes_head_relative_changes_that_match_the_base() {
        let (_temp, workspace) = repo();
        let base = resolve_revision(&workspace, "HEAD").unwrap();
        fs::write(workspace.root.join("a.txt"), "head\n").unwrap();
        git(&workspace.root, &["add", "a.txt"]);
        git(&workspace.root, &["commit", "-qm", "head change"]);
        fs::write(workspace.root.join("a.txt"), "one\n").unwrap();

        let snapshot = capture_target(&workspace, ReviewTargetSpec::base(base)).unwrap();

        assert!(snapshot.entries.is_empty());
    }

    #[test]
    fn sensitive_and_oversized_files_are_hashed_but_not_fully_materialized() {
        let (_temp, workspace) = repo();
        fs::write(workspace.root.join(".env"), "API_KEY=do-not-persist\n").unwrap();
        git(&workspace.root, &["add", "-f", ".env"]);
        let oversized = vec![b'x'; MAX_REVIEW_MATERIALIZED_FILE_BYTES + 17];
        fs::write(workspace.root.join("large.txt"), &oversized).unwrap();

        let snapshot = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        let sensitive = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == ".env")
            .unwrap();
        assert!(sensitive.worktree.hash.is_some());
        assert!(sensitive.snapshot_bytes.is_none());
        assert!(sensitive.content_truncated);
        assert!(sensitive.diff.is_empty());

        let large = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == "large.txt")
            .unwrap();
        assert_eq!(
            large.snapshot_bytes.as_ref().unwrap().len(),
            MAX_REVIEW_MATERIALIZED_FILE_BYTES
        );
        assert!(large.content_truncated);
        assert_eq!(large.worktree.hash, Some(hash_bytes(&oversized)));

        let result = finalize_result("review", "run", "session", snapshot, None, false, false, 1);
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("do-not-persist"));
        assert!(!json.contains("snapshot_bytes"));
        assert!(!json.contains("\"diff\""));
    }

    #[test]
    fn target_digest_changes_and_empty_target_is_a_complete_pass() {
        let (_temp, workspace) = repo();
        let empty = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        assert!(empty.entries.is_empty());
        let result = finalize_result(
            "review",
            "run",
            "session",
            empty.clone(),
            None,
            false,
            false,
            1,
        );
        assert_eq!(result.conclusion, ReviewConclusion::Pass);

        fs::write(workspace.root.join("a.txt"), "changed\n").unwrap();
        assert!(empty.is_stale(&workspace).unwrap());
        let changed = capture_target(&workspace, ReviewTargetSpec::default()).unwrap();
        assert_ne!(empty.digest, changed.digest);
    }

    #[test]
    fn rejects_revision_option_injection_and_workspace_state_root() {
        let (_temp, workspace) = repo();
        let error = capture_target(&workspace, ReviewTargetSpec::base("--output"))
            .expect_err("option must be rejected");
        assert!(matches!(error, ReviewError::InvalidRevision(_)));
        let error = resolve_external_state_root(&workspace, Some(&workspace.root.join(".rove")))
            .expect_err("state must be outside target");
        assert!(matches!(error, ReviewError::InvalidStateRoot(_)));
    }

    #[test]
    fn rejects_state_root_reaching_workspace_through_symlink() {
        let (_temp, workspace) = repo();
        let external = TempDir::new().unwrap();
        let link = external.path().join("workspace-link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&workspace.root, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&workspace.root, &link).is_err() {
            // Developer Mode or the symlink privilege may be unavailable on a
            // Windows test host. The path comparison test below still covers
            // case-insensitive containment on every Windows build.
            return;
        }

        let error = resolve_external_state_root(&workspace, Some(&link.join("review-state")))
            .expect_err("symlinked workspace state must be rejected");
        assert!(matches!(error, ReviewError::InvalidStateRoot(_)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_state_root_containment_is_case_insensitive() {
        assert!(path_is_same_or_child(
            Path::new(r"C:\USERS\ALICE\REPO\review-state"),
            Path::new(r"c:\users\alice\repo"),
        ));
        assert!(!path_is_same_or_child(
            Path::new(r"C:\USERS\ALICE\REPOSITORY"),
            Path::new(r"c:\users\alice\repo"),
        ));
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
