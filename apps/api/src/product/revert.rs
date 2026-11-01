//! Revert a single recorded file mutation (issue #29).
//!
//! The transcript's change cards offer a rollback affordance; this is the
//! endpoint it calls. A revert undoes the *latest* recorded mutation for one
//! path — it does not roll the session back to a point in time, and it never
//! touches a file it cannot prove is still in the state the mutation left it
//! in.
//!
//! What each operation can safely do:
//! - `create` → delete the file, but only when its current bytes are still the
//!   bytes the create wrote (reconstructed from the diff). If anything edited
//!   the file afterwards we refuse rather than destroy newer work.
//! - `update` → `git apply -R` the recorded diff, in a Repo workspace only and
//!   only after `git apply --check -R` proves the reverse patch applies cleanly
//!   against the file as it is now.
//! - `delete` is refused: rebuilding content from a unified diff is a separate,
//!   larger mechanism, and guessing at a file's prior content is worse than
//!   saying no.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use utoipa::ToSchema;

use rove_core::ToolMutationOperation;

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::files::{join_safe, workspace_root};
use super::trust::{ensure_workspace_read_allowed, product_workspace_kind};
use super::{ProductSessionId, ProductWorkspaceKind};

/// How long a single `git apply` invocation may run before it is abandoned.
const GIT_APPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// A single revert never patches a diff larger than this.
const MAX_REVERT_DIFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductMutationRevertRequest {
    /// Workspace-relative path whose latest recorded mutation is reverted.
    pub path: String,
    /// Restrict the search to mutations recorded by this run. When omitted the
    /// most recent mutation for `path` across the session's runs is reverted.
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductMutationRevertResponse {
    pub path: String,
    /// The operation that was undone (`create` or `update`).
    pub reverted_operation: String,
    /// The run that recorded the reverted mutation.
    pub run_id: String,
    /// What the revert did to the file on disk.
    pub action: ProductMutationRevertAction,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductMutationRevertAction {
    /// A `create` was undone by deleting the created file.
    DeletedFile,
    /// An `update` was undone by reverse-applying its recorded diff.
    ReversePatched,
}

/// One recorded mutation plus the run that produced it.
struct LocatedMutation {
    run_id: String,
    operation: ToolMutationOperation,
    diff: Option<String>,
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/mutations/revert",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    request_body = ProductMutationRevertRequest,
    responses(
        (status = 200, description = "The recorded mutation was reverted", body = ProductMutationRevertResponse),
        (status = 400, description = "Invalid request or path escapes the workspace", body = ApiErrorResponse),
        (status = 404, description = "Product session not found, or no recorded mutation for the path", body = ApiErrorResponse),
        (status = 409, description = "The file no longer matches the state the mutation left it in, or the operation cannot be reverted", body = ApiErrorResponse),
        (status = 500, description = "Product store or filesystem operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn revert_product_mutation(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    body: Result<Json<ProductMutationRevertRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ProductMutationRevertResponse>, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::bad_request("request body must be JSON with a `path` field"))?;
    let path = request.path.trim();
    if path.is_empty() {
        return Err(ApiError::bad_request("`path` must not be empty"));
    }

    let store = state.product_store()?;
    let context = store.get_session_context(&session_id).await?;
    let bindings = store.list_run_bindings(&session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;

    let root = workspace_root(&context.workspace.kind, &context.workspace.canonical_root)?;
    // A revert writes. The product trust boundary must be at least as strong as
    // the read path — a revoked workspace is refused here exactly as it is when
    // browsing files.
    ensure_workspace_read_allowed(
        &state,
        &root,
        product_workspace_kind(&context.workspace.kind),
    )
    .await?;

    let mutation = locate_mutation(&state_store, &bindings, path, request.run_id.as_deref())
        .await
        .ok_or_else(|| {
            ApiError::not_found_with_code(
                "mutation_not_found",
                "no recorded mutation for that path",
            )
        })?;

    let absolute = resolve_in_workspace(&root, path)?;
    let action = match &mutation.operation {
        ToolMutationOperation::Create => revert_create(&mutation, &absolute, path).await?,
        ToolMutationOperation::Update => {
            revert_update(&mutation, &root, &absolute, path, &context.workspace.kind).await?
        }
        ToolMutationOperation::Delete => {
            return Err(ApiError::conflict_with_code(
                "revert_unsupported",
                "reverting a deleted file is not supported yet",
            ));
        }
        ToolMutationOperation::Unknown => {
            return Err(ApiError::conflict_with_code(
                "revert_unsupported",
                "this mutation recorded no operation to revert",
            ));
        }
    };

    Ok(Json(ProductMutationRevertResponse {
        path: path.to_string(),
        reverted_operation: operation_label(&mutation.operation).to_string(),
        run_id: mutation.run_id,
        action,
    }))
}

fn operation_label(operation: &ToolMutationOperation) -> &'static str {
    match operation {
        ToolMutationOperation::Create => "create",
        ToolMutationOperation::Update => "update",
        ToolMutationOperation::Delete => "delete",
        ToolMutationOperation::Unknown => "unknown",
    }
}

/// Find the most recent recorded mutation for `path`, newest binding first and
/// newest mutation within a report first. When `run_id` is given only that
/// run's report is searched.
async fn locate_mutation(
    state_store: &rove_runtime::state::store::StateStore,
    bindings: &[rove_product_store::ProductSessionRunBinding],
    path: &str,
    run_id: Option<&str>,
) -> Option<LocatedMutation> {
    for binding in bindings.iter().rev() {
        if let Some(wanted) = run_id {
            if binding.runtime_run_id.to_string() != wanted {
                continue;
            }
        }
        let Ok(report) = state_store.load_report(binding.runtime_run_id).await else {
            continue;
        };
        for mutation in report.tool_mutations.iter().rev() {
            if mutation.path == path {
                return Some(LocatedMutation {
                    run_id: binding.runtime_run_id.to_string(),
                    operation: mutation.operation.clone(),
                    diff: mutation.diff.clone(),
                });
            }
        }
    }
    None
}

/// Resolve `path` to an absolute path that is provably inside the workspace.
/// `join_safe` rejects traversal; the follow-up canonicalize rejects symlink
/// escapes. A file that does not exist yet (e.g. the target of a `create`
/// revert already deleted) canonicalizes through its parent instead.
fn resolve_in_workspace(root: &Path, path: &str) -> Result<PathBuf, ApiError> {
    let joined = join_safe(root, path)?;
    let canonical = match joined.canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => {
            let parent = joined
                .parent()
                .ok_or_else(|| ApiError::bad_request("path has no parent directory"))?;
            let canonical_parent = parent.canonicalize().map_err(|_| {
                ApiError::conflict_with_code(
                    "revert_target_missing",
                    "the mutated path's directory no longer exists",
                )
            })?;
            canonical_parent.join(
                joined
                    .file_name()
                    .ok_or_else(|| ApiError::bad_request("path has no file name"))?,
            )
        }
    };
    if !canonical.starts_with(root) {
        return Err(ApiError::bad_request("path escapes the workspace"));
    }
    Ok(canonical)
}

/// A `create` is undone by deleting the file — but only when the file is still
/// exactly what the create wrote. The recorded diff of a pure create is all
/// `+` lines; stripping the `+` prefixes reconstructs the created content. If
/// the current bytes differ, something else touched the file and we refuse
/// rather than destroy newer work.
async fn revert_create(
    mutation: &LocatedMutation,
    absolute: &Path,
    path: &str,
) -> Result<ProductMutationRevertAction, ApiError> {
    let expected = created_content(mutation.diff.as_deref()).ok_or_else(|| {
        ApiError::conflict_with_code(
            "revert_unverifiable",
            "the create recorded no diff to verify against",
        )
    })?;
    let current = match tokio::fs::read(absolute).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Already gone — the revert is a no-op that still reports what it
            // undid, which keeps the action idempotent.
            return Ok(ProductMutationRevertAction::DeletedFile);
        }
        Err(error) => {
            return Err(ApiError::internal(format!(
                "read before revert failed: {error}"
            )));
        }
    };
    if current != expected {
        return Err(ApiError::conflict_with_code(
            "revert_conflict",
            format!("{path} was changed after it was created; refusing to delete newer work"),
        ));
    }
    tokio::fs::remove_file(absolute)
        .await
        .map_err(|error| ApiError::internal(format!("delete during revert failed: {error}")))?;
    Ok(ProductMutationRevertAction::DeletedFile)
}

/// Reconstruct the bytes a `create` wrote from its recorded unified diff: every
/// `+` line in every hunk body, in order. Returns `None` when the diff is not a
/// clean all-additions create (headers, context, or `-` lines present).
fn created_content(diff: Option<&str>) -> Option<Vec<u8>> {
    let diff = diff?;
    let mut out = String::new();
    let mut saw_addition = false;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        if line.starts_with("diff ") || line.starts_with("index ") || line.starts_with("new file") {
            continue;
        }
        if let Some(added) = line.strip_prefix('+') {
            saw_addition = true;
            out.push_str(added);
            out.push('\n');
        } else if line.starts_with('-') || (!line.is_empty() && !line.starts_with('\\')) {
            // A `-` line or a context line means this is not a pure create.
            return None;
        }
    }
    if saw_addition {
        Some(out.into_bytes())
    } else {
        None
    }
}

/// An `update` is undone by reverse-applying its recorded diff. This is only
/// offered in a Repo workspace, and only after `git apply --check -R` proves
/// the reverse patch applies cleanly against the file as it exists now — a
/// file that no longer matches is a 409, never a clobber.
async fn revert_update(
    mutation: &LocatedMutation,
    root: &Path,
    absolute: &Path,
    path: &str,
    workspace_kind: &ProductWorkspaceKind,
) -> Result<ProductMutationRevertAction, ApiError> {
    if !matches!(workspace_kind, ProductWorkspaceKind::Repo) {
        return Err(ApiError::conflict_with_code(
            "revert_unsupported",
            "reverting an edit is only supported in a repository workspace",
        ));
    }
    let diff = mutation.diff.as_deref().ok_or_else(|| {
        ApiError::conflict_with_code(
            "revert_unverifiable",
            "the update recorded no diff to reverse-apply",
        )
    })?;
    if diff.len() > MAX_REVERT_DIFF_BYTES {
        return Err(ApiError::payload_too_large_with_code(
            "revert_diff_too_large",
            "the recorded diff is too large to revert safely",
        ));
    }
    if !absolute.exists() {
        return Err(ApiError::conflict_with_code(
            "revert_target_missing",
            format!("{path} no longer exists"),
        ));
    }
    // Prove the reverse applies before touching the file.
    git_apply(
        root,
        &["apply", "--check", "-R", "--whitespace=nowarn", "-"],
        diff,
    )
    .await?;
    git_apply(root, &["apply", "-R", "--whitespace=nowarn", "-"], diff).await?;
    Ok(ProductMutationRevertAction::ReversePatched)
}

async fn git_apply(root: &Path, args: &[&str], diff: &str) -> Result<(), ApiError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| ApiError::internal(format!("git apply spawn failed: {error}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(diff.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let output = tokio::time::timeout(GIT_APPLY_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ApiError::internal("git apply timed out"))?
        .map_err(|error| ApiError::internal(format!("git apply wait failed: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::conflict_with_code(
            "revert_conflict",
            format!("the recorded diff no longer applies cleanly: {stderr}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure create diff is all `+` lines; stripping the prefix rebuilds the
    /// file the create wrote, which is what lets `revert_create` prove the
    /// current bytes are still the created ones.
    #[test]
    fn created_content_rebuilds_a_pure_create() {
        let diff = "--- a/src/greet.py\n+++ b/src/greet.py\n@@ -1,0 +1,2 @@\n+def hello(name):\n+    return \"Hello!\"\n";
        assert_eq!(
            created_content(Some(diff)),
            Some(b"def hello(name):\n    return \"Hello!\"\n".to_vec())
        );
    }

    /// A create diff that carries context or a `-` line is not a pure create —
    /// the safety check must refuse rather than guess at the original content.
    #[test]
    fn created_content_refuses_a_non_create_diff() {
        let with_context =
            "@@ -1,2 +1,2 @@\n def hello(name):\n-    return \"Hi\"\n+    return \"Hello!\"\n";
        assert_eq!(created_content(Some(with_context)), None);
        let delete_like = "@@ -1,2 +1,0 @@\n-def a():\n-    pass\n";
        assert_eq!(created_content(Some(delete_like)), None);
        assert_eq!(created_content(None), None);
    }

    #[test]
    fn operation_labels_match_the_recorded_vocabulary() {
        assert_eq!(operation_label(&ToolMutationOperation::Create), "create");
        assert_eq!(operation_label(&ToolMutationOperation::Update), "update");
        assert_eq!(operation_label(&ToolMutationOperation::Delete), "delete");
    }
}
