//! Bounded canonical tool mutations and repository Git patches.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use utoipa::{IntoParams, ToSchema};

use rove_core::ToolMutationOperation;

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::{ProductSessionId, ProductWorkspaceKind};

const MAX_DIFF_ENTRIES: usize = 4096;
const MAX_GIT_ENTRIES: usize = 512;
const MAX_DIFF_BYTES_PER_ENTRY: usize = 128 * 1024;
const MAX_DIFF_BYTES_TOTAL: usize = 4 * 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    Run,
    Git,
    #[default]
    All,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionDiffQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductDiffOp {
    Create,
    Update,
    Delete,
    Modified,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductDiffSource {
    Run,
    Git,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductDiffEntry {
    pub path: String,
    pub op: ProductDiffOp,
    pub source: ProductDiffSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub binary: bool,
    pub truncated: bool,
    pub reconstructable: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductSessionDiffResponse {
    pub session_id: ProductSessionId,
    pub scope: String,
    pub entries: Vec<ProductDiffEntry>,
    pub partial_reasons: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/diff",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        SessionDiffQuery
    ),
    responses(
        (status = 200, description = "Bounded canonical session diff", body = ProductSessionDiffResponse),
        (status = 400, description = "Invalid scope", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store or runtime state operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_session_diff(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    Query(query): Query<SessionDiffQuery>,
) -> Result<Json<ProductSessionDiffResponse>, ApiError> {
    let scope = parse_scope(query.scope.as_deref())?;
    let store = state.product_store()?;
    let context = store.get_session_context(&session_id).await?;
    let bindings = store.list_run_bindings(&session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;

    let mut entries = Vec::new();
    let mut partial_reasons = Vec::new();
    let mut remaining_bytes = MAX_DIFF_BYTES_TOTAL;

    if matches!(scope, DiffScope::Run | DiffScope::All) {
        for binding in &bindings {
            match state_store.load_report(binding.runtime_run_id).await {
                Ok(report) => {
                    for mutation in &report.tool_mutations {
                        if entries.len() >= MAX_DIFF_ENTRIES {
                            partial_reasons
                                .push(format!("run diff capped at {MAX_DIFF_ENTRIES} entries"));
                            break;
                        }
                        let op = match mutation.operation {
                            ToolMutationOperation::Create => ProductDiffOp::Create,
                            ToolMutationOperation::Update => ProductDiffOp::Update,
                            ToolMutationOperation::Delete => ProductDiffOp::Delete,
                            ToolMutationOperation::Unknown => ProductDiffOp::Unknown,
                        };
                        let diff = bounded_diff(mutation.diff.as_deref(), &mut remaining_bytes);
                        entries.push(ProductDiffEntry {
                            path: mutation.path.clone(),
                            op,
                            source: ProductDiffSource::Run,
                            source_run_id: Some(binding.runtime_run_id.to_string()),
                            diff: diff.text,
                            binary: diff.binary,
                            truncated: diff.truncated,
                            reconstructable: diff.present && !diff.binary && !diff.truncated,
                        });
                    }
                }
                Err(error) => partial_reasons.push(format!(
                    "run {}: report.json unavailable ({error})",
                    binding.runtime_run_id
                )),
            }
        }
    }

    if matches!(scope, DiffScope::Git | DiffScope::All)
        && matches!(context.workspace.kind, ProductWorkspaceKind::Repo)
    {
        match git_diff(&context.workspace.canonical_root, &mut remaining_bytes).await {
            Ok(mut git_entries) => entries.append(&mut git_entries),
            Err(reason) => partial_reasons.push(format!("git diff unavailable: {reason}")),
        }
    }

    if remaining_bytes == 0 {
        partial_reasons.push(format!("diff text capped at {MAX_DIFF_BYTES_TOTAL} bytes"));
    }
    let scope_label = match scope {
        DiffScope::Run => "run",
        DiffScope::Git => "git",
        DiffScope::All => "all",
    };
    Ok(Json(ProductSessionDiffResponse {
        session_id,
        scope: scope_label.to_string(),
        entries,
        partial_reasons,
    }))
}

#[derive(Default)]
struct BoundedDiff {
    text: Option<String>,
    present: bool,
    binary: bool,
    truncated: bool,
}

fn bounded_diff(raw: Option<&str>, remaining_bytes: &mut usize) -> BoundedDiff {
    let Some(raw) = raw else {
        return BoundedDiff::default();
    };
    let binary = raw.as_bytes().contains(&0)
        || raw.contains("GIT binary patch")
        || raw.contains("Binary files ");
    if binary {
        return BoundedDiff {
            text: Some("Binary change recorded; binary patch content omitted.".to_string()),
            present: true,
            binary: true,
            truncated: false,
        };
    }
    let cap = (*remaining_bytes).min(MAX_DIFF_BYTES_PER_ENTRY);
    if cap == 0 {
        return BoundedDiff {
            text: None,
            present: true,
            binary: false,
            truncated: true,
        };
    }
    let (text, truncated) = truncate_utf8(raw, cap);
    *remaining_bytes = remaining_bytes.saturating_sub(text.len());
    BoundedDiff {
        text: Some(text),
        present: true,
        binary: false,
        truncated,
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

fn parse_scope(raw: Option<&str>) -> Result<DiffScope, ApiError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(DiffScope::All),
        Some("run") => Ok(DiffScope::Run),
        Some("git") => Ok(DiffScope::Git),
        Some("all") => Ok(DiffScope::All),
        Some(_) => Err(ApiError::bad_request(
            "scope must be one of run, git, or all",
        )),
    }
}

async fn git_diff(
    root: &Path,
    remaining_bytes: &mut usize,
) -> Result<Vec<ProductDiffEntry>, String> {
    let status = run_git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await?;
    let changes = parse_git_status(&status)?;
    let mut entries = Vec::new();
    for change in changes.into_iter().take(MAX_GIT_ENTRIES) {
        let raw = if change.untracked {
            synthesize_untracked_diff(root, &change.path).await?
        } else {
            let args = [
                "diff",
                "HEAD",
                "--no-ext-diff",
                "--binary",
                "--unified=3",
                "--",
                change.path.as_str(),
            ];
            match run_git(root, &args).await {
                Ok(output) => String::from_utf8_lossy(&output).into_owned(),
                Err(_) => {
                    let fallback = [
                        "diff",
                        "--no-ext-diff",
                        "--binary",
                        "--unified=3",
                        "--",
                        change.path.as_str(),
                    ];
                    String::from_utf8_lossy(&run_git(root, &fallback).await?).into_owned()
                }
            }
        };
        let diff = bounded_diff(Some(&raw), remaining_bytes);
        entries.push(ProductDiffEntry {
            path: change.path,
            op: change.op,
            source: ProductDiffSource::Git,
            source_run_id: None,
            diff: diff.text,
            binary: diff.binary,
            truncated: diff.truncated,
            reconstructable: diff.present && !diff.binary && !diff.truncated,
        });
    }
    Ok(entries)
}

struct GitChange {
    path: String,
    op: ProductDiffOp,
    untracked: bool,
}

fn parse_git_status(bytes: &[u8]) -> Result<Vec<GitChange>, String> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 || field[2] != b' ' {
            return Err("unexpected git status record".to_string());
        }
        let x = field[0] as char;
        let y = field[1] as char;
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let renamed = matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C');
        if renamed {
            // In porcelain v1 -z, the following NUL field is the original
            // path. The first field is the destination that should be shown.
            let _ = fields.next();
        }
        let untracked = x == '?' && y == '?';
        let op = if untracked || x == 'A' || y == 'A' {
            ProductDiffOp::Create
        } else if x == 'D' || y == 'D' {
            ProductDiffOp::Delete
        } else if renamed {
            ProductDiffOp::Update
        } else if x == 'M' || y == 'M' {
            ProductDiffOp::Modified
        } else {
            ProductDiffOp::Unknown
        };
        changes.push(GitChange {
            path,
            op,
            untracked,
        });
    }
    Ok(changes)
}

async fn synthesize_untracked_diff(root: &Path, relative: &str) -> Result<String, String> {
    let path = root.join(relative);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("repo root unavailable: {error}"))?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("untracked file unavailable: {error}"))?;
    if !canonical.starts_with(canonical_root) {
        return Err("untracked path escapes repository".to_string());
    }
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|error| format!("untracked file stat failed: {error}"))?;
    if !metadata.is_file() {
        return Ok("Untracked non-regular path; content omitted.".to_string());
    }
    let file = tokio::fs::File::open(&canonical)
        .await
        .map_err(|error| format!("untracked file open failed: {error}"))?;
    let mut bytes = Vec::new();
    file.take((MAX_DIFF_BYTES_PER_ENTRY + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("untracked file read failed: {error}"))?;
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return Ok("Binary files /dev/null and untracked file differ".to_string());
    }
    let text = String::from_utf8(bytes).map_err(|_| "untracked file is not UTF-8".to_string())?;
    let line_count = text.lines().count();
    let mut patch = format!(
        "diff --git a/{relative} b/{relative}\nnew file mode 100644\n--- /dev/null\n+++ b/{relative}\n@@ -0,0 +1,{line_count} @@\n"
    );
    for line in text.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

async fn run_git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| "command timed out".to_string())?
        .map_err(|error| format!("spawn failed: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parser_accepts_known_values() {
        assert!(matches!(parse_scope(None).unwrap(), DiffScope::All));
        assert!(matches!(parse_scope(Some("run")).unwrap(), DiffScope::Run));
        assert!(matches!(parse_scope(Some("git")).unwrap(), DiffScope::Git));
        assert!(parse_scope(Some("nope")).is_err());
    }

    #[test]
    fn bounded_diff_preserves_text_and_reports_truncation() {
        let mut remaining = 5;
        let bounded = bounded_diff(Some("abcdef"), &mut remaining);
        assert_eq!(bounded.text.as_deref(), Some("abcde"));
        assert!(bounded.truncated);
        assert!(!bounded.binary);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn binary_patch_is_classified_without_returning_payload() {
        let mut remaining = 100;
        let bounded = bounded_diff(
            Some("GIT binary patch\nliteral 999\nsecret"),
            &mut remaining,
        );
        assert!(bounded.binary);
        assert!(!bounded.truncated);
        assert_eq!(
            bounded.text.as_deref(),
            Some("Binary change recorded; binary patch content omitted.")
        );
    }

    #[test]
    fn parses_git_status_including_untracked_and_deleted() {
        let changes = parse_git_status(b" M src/main.rs\0?? new.txt\0D  old.txt\0").unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].op, ProductDiffOp::Modified);
        assert!(changes[1].untracked);
        assert_eq!(changes[2].op, ProductDiffOp::Delete);
    }
}
