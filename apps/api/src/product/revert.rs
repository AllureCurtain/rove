//! Revert verified file mutations without trusting display diffs as byte identity.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use rove_core::ToolMutationOperation;
use rove_runtime::workspace::is_symlink_or_reparse;

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::compaction::SessionMaintenanceClaim;
use super::files::{join_safe, workspace_root};
use super::trust::{ensure_workspace_read_allowed, product_workspace_kind};
use super::{ProductErrorCode, ProductSessionId, ProductSessionStatus, ProductWorkspaceKind};

/// A single revert never patches a diff larger than this.
const MAX_REVERT_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_REVERT_FILE_BYTES: usize = 16 * 1024 * 1024;

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
    before_version: Option<String>,
    after_version: Option<String>,
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
    let path = request.path.as_str();
    if path.trim().is_empty() || path.len() > 4096 {
        return Err(ApiError::bad_request(
            "`path` must be nonempty and at most 4096 bytes",
        ));
    }

    let store = state.product_store()?;
    let (claim, mut guard) =
        SessionMaintenanceClaim::acquire(store.clone(), session_id.clone()).await?;
    if claim.previous_status != ProductSessionStatus::Idle {
        guard.release().await;
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionActive.as_str(),
            "only an idle product session can be reverted",
        ));
    }
    let context = &claim.context;
    let bindings = store.list_run_bindings(&session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;
    let root = workspace_root(&context.workspace.kind, &context.workspace.canonical_root)?;
    if root != context.workspace.canonical_root {
        return Err(unverifiable("workspace identity changed"));
    }
    ensure_workspace_read_allowed(
        &state,
        &root,
        product_workspace_kind(&context.workspace.kind),
    )
    .await?;
    let mutation = locate_mutation(&state_store, &bindings, path, request.run_id.as_deref())
        .await?
        .ok_or_else(|| {
            ApiError::not_found_with_code(
                "mutation_not_found",
                "no recorded mutation for that path",
            )
        })?;
    let path = path.to_string();
    let kind = context.workspace.kind;
    // The blocking operation owns the claim: disconnecting the HTTP request
    // cannot release exclusivity while its file mutation is still running.
    let (outcome, mut guard) = tokio::task::spawn_blocking(move || {
        let outcome = revert_verified_mutation(&root, &path, &kind, &mutation);
        (outcome, guard)
    })
    .await
    .map_err(|_| ApiError::internal("rollback worker failed"))?;
    guard.release().await;
    outcome.map(Json)
}

fn revert_verified_mutation(
    root: &Path,
    path: &str,
    kind: &ProductWorkspaceKind,
    mutation: &LocatedMutation,
) -> Result<ProductMutationRevertResponse, ApiError> {
    let absolute = resolve_in_workspace(root, path)?;
    let action = match mutation.operation {
        ToolMutationOperation::Create => revert_create(mutation, root, &absolute, path)?,
        ToolMutationOperation::Update => revert_update(mutation, root, &absolute, path, kind)?,
        _ => {
            return Err(ApiError::conflict_with_code(
                "revert_unsupported",
                "this mutation cannot be reverted",
            ));
        }
    };
    Ok(ProductMutationRevertResponse {
        path: path.to_string(),
        reverted_operation: operation_label(&mutation.operation).to_string(),
        run_id: mutation.run_id.clone(),
        action,
    })
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
) -> Result<Option<LocatedMutation>, ApiError> {
    for binding in bindings.iter().rev() {
        if let Some(wanted) = run_id
            && binding.runtime_run_id.to_string() != wanted
        {
            continue;
        }
        let report = state_store
            .load_report(binding.runtime_run_id)
            .await
            .map_err(|_| unverifiable("a run's mutation evidence is unavailable"))?;
        if report.run_id != binding.runtime_run_id
            || report.session_id != binding.runtime_session_id
            || report.job_id != binding.runtime_job_id
        {
            return Err(unverifiable("the mutation evidence belongs to another run"));
        }
        if let Some(mutation) = report
            .tool_mutations
            .iter()
            .rev()
            .find(|mutation| mutation.path == path)
        {
            return Ok(Some(LocatedMutation {
                run_id: binding.runtime_run_id.to_string(),
                operation: mutation.operation.clone(),
                diff: mutation.diff.clone(),
                before_version: mutation.before_version.clone(),
                after_version: mutation.after_version.clone(),
            }));
        }
    }
    Ok(None)
}

fn unverifiable(message: &str) -> ApiError {
    ApiError::conflict_with_code("revert_unverifiable", message)
}

fn conflict() -> ApiError {
    ApiError::conflict_with_code(
        "revert_conflict",
        "the file no longer matches the recorded mutation",
    )
}

fn resolve_in_workspace(root: &Path, path: &str) -> Result<PathBuf, ApiError> {
    if path.contains(['\r', '\n']) {
        return Err(ApiError::bad_request("path contains a line break"));
    }
    let canonical = root
        .canonicalize()
        .map_err(|_| unverifiable("workspace root cannot be verified"))?;
    if canonical != root {
        return Err(unverifiable("workspace identity changed"));
    }
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|_| unverifiable("workspace root cannot be verified"))?;
    if is_symlink_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(unverifiable("workspace root must be an ordinary directory"));
    }
    // Inspect lexical components before join_safe canonicalizes them: an
    // internal symlink must not silently replace the requested mutation path.
    let components: Vec<_> = Path::new(path).components().collect();
    let mut cursor = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        if !matches!(component, Component::Normal(_)) {
            return Err(ApiError::bad_request("path must be workspace relative"));
        }
        cursor.push(component.as_os_str());
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if is_symlink_or_reparse(&metadata) => {
                return Err(unverifiable("rollback does not follow symlinks"));
            }
            Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                return Err(unverifiable("rollback parent is not a directory"));
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && index + 1 == components.len() => {}
            Err(_) => return Err(unverifiable("rollback path cannot be verified")),
        }
    }
    let joined = join_safe(root, path)?;
    if joined != cursor {
        return Err(unverifiable("rollback path identity changed"));
    }
    Ok(cursor)
}

fn version(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn required_version(value: Option<&str>) -> Result<&str, ApiError> {
    let value =
        value.ok_or_else(|| unverifiable("the mutation predates exact byte verification"))?;
    if value.strip_prefix("sha256:").is_none_or(|digest| {
        digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(unverifiable("the mutation has no valid byte digest"));
    }
    Ok(value)
}

fn read_current(root: &Path, absolute: &Path, path: &str) -> Result<Option<Vec<u8>>, ApiError> {
    if resolve_in_workspace(root, path)? != absolute {
        return Err(conflict());
    }
    let metadata = match std::fs::symlink_metadata(absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unverifiable("rollback target cannot be read")),
    };
    if !metadata.is_file() || metadata.len() > MAX_REVERT_FILE_BYTES as u64 {
        return Err(unverifiable("rollback needs a bounded ordinary file"));
    }
    let file = std::fs::File::open(absolute)
        .map_err(|_| unverifiable("rollback target cannot be opened"))?;
    let mut bytes = Vec::new();
    file.take(MAX_REVERT_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unverifiable("rollback target cannot be read"))?;
    if bytes.len() > MAX_REVERT_FILE_BYTES {
        return Err(unverifiable("rollback target exceeds the size limit"));
    }
    Ok(Some(bytes))
}

fn verify_current(
    root: &Path,
    absolute: &Path,
    path: &str,
    expected: &str,
) -> Result<Option<Vec<u8>>, ApiError> {
    let bytes = read_current(root, absolute, path)?;
    if bytes
        .as_deref()
        .is_some_and(|bytes| version(bytes) != expected)
    {
        return Err(conflict());
    }
    Ok(bytes)
}

fn revert_create(
    mutation: &LocatedMutation,
    root: &Path,
    absolute: &Path,
    path: &str,
) -> Result<ProductMutationRevertAction, ApiError> {
    let expected = required_version(mutation.after_version.as_deref())?;
    if verify_current(root, absolute, path, expected)?.is_none() {
        return Ok(ProductMutationRevertAction::DeletedFile);
    }
    // Recheck at the side-effect boundary; this also rejects a symlink swap
    // after the first bounded read. External writers do not share this claim.
    if verify_current(root, absolute, path, expected)?.is_some() {
        std::fs::remove_file(absolute).map_err(|_| ApiError::internal("rollback delete failed"))?;
    }
    Ok(ProductMutationRevertAction::DeletedFile)
}

fn revert_update(
    mutation: &LocatedMutation,
    root: &Path,
    absolute: &Path,
    path: &str,
    kind: &ProductWorkspaceKind,
) -> Result<ProductMutationRevertAction, ApiError> {
    if !matches!(kind, ProductWorkspaceKind::Repo) {
        return Err(ApiError::conflict_with_code(
            "revert_unsupported",
            "reverting an edit requires a repository workspace",
        ));
    }
    let before = required_version(mutation.before_version.as_deref())?;
    let after = required_version(mutation.after_version.as_deref())?;
    let current = verify_current(root, absolute, path, after)?.ok_or_else(conflict)?;
    let diff = mutation
        .diff
        .as_deref()
        .ok_or_else(|| unverifiable("the mutation has no reverse patch"))?;
    let restored = reverse_patch(path, &current, diff)?;
    if version(&restored) != before {
        return Err(unverifiable(
            "the reverse patch does not restore the recorded bytes",
        ));
    }
    write_restored(root, absolute, path, after, &restored)?;
    Ok(ProductMutationRevertAction::ReversePatched)
}

struct StagedFile(PathBuf);
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_restored(
    root: &Path,
    absolute: &Path,
    path: &str,
    after: &str,
    bytes: &[u8],
) -> Result<(), ApiError> {
    resolve_in_workspace(root, path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| unverifiable("rollback target has no parent"))?;
    let staged = StagedFile(parent.join(format!(".rove-revert-{}.tmp", ulid::Ulid::new())));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged.0)
        .map_err(|_| ApiError::internal("cannot stage rollback"))?;
    let permissions = std::fs::metadata(absolute)
        .map_err(|_| conflict())?
        .permissions();
    file.set_permissions(permissions)
        .map_err(|_| ApiError::internal("cannot preserve rollback permissions"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| ApiError::internal("cannot stage rollback bytes"))?;
    drop(file);
    verify_current(root, absolute, path, after)?.ok_or_else(conflict)?;
    std::fs::rename(&staged.0, absolute)
        .map_err(|_| ApiError::internal("cannot publish rollback"))?;
    Ok(())
}

fn range(value: &str) -> Option<(usize, usize)> {
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    let start: usize = start.parse().ok()?;
    let count: usize = count.parse().ok()?;
    if count > MAX_REVERT_FILE_BYTES || (count != 0 && start == 0) {
        return None;
    }
    Some((if count == 0 { start } else { start - 1 }, count))
}

fn reverse_patch(path: &str, current: &[u8], diff: &str) -> Result<Vec<u8>, ApiError> {
    if diff.len() > MAX_REVERT_DIFF_BYTES || !diff.ends_with('\n') {
        return Err(unverifiable("reverse patch is incomplete or too large"));
    }
    let invalid = || unverifiable("reverse patch is not a complete single-file diff");
    let mut records = diff.split_inclusive('\n');
    if records.next() != Some(format!("--- a/{path}\n").as_str())
        || records.next() != Some(format!("+++ b/{path}\n").as_str())
    {
        return Err(invalid());
    }
    let body: Vec<_> = records.collect();
    let current: Vec<&[u8]> = current.split_inclusive(|byte| *byte == b'\n').collect();
    let mut cursor = 0;
    let mut index = 0;
    let mut output = Vec::new();
    let mut before_cursor = 0;
    let mut hunks = 0;
    while index < body.len() {
        let header = body[index]
            .strip_prefix("@@ -")
            .and_then(|line| line.strip_suffix(" @@\n"))
            .ok_or_else(invalid)?;
        let (old_range, new_range) = header.split_once(" +").ok_or_else(invalid)?;
        let (old_start, old_count) = range(old_range).ok_or_else(invalid)?;
        let (new_start, new_count) = range(new_range).ok_or_else(invalid)?;
        if new_start < cursor
            || new_start > current.len()
            || old_start != before_cursor + new_start - cursor
        {
            return Err(invalid());
        }
        for line in &current[cursor..new_start] {
            output.extend_from_slice(line);
        }
        before_cursor = old_start;
        cursor = new_start;
        index += 1;
        let mut old_seen = 0;
        let mut new_seen = 0;
        while index < body.len() && !body[index].starts_with("@@ ") {
            let record = body[index];
            let prefix = record.as_bytes()[0];
            if !matches!(prefix, b' ' | b'+' | b'-') {
                return Err(invalid());
            }
            let mut line = &record.as_bytes()[1..];
            index += 1;
            if body.get(index) == Some(&"\\ No newline at end of file\n") {
                line = line.strip_suffix(b"\n").ok_or_else(invalid)?;
                index += 1;
            }
            if prefix != b'-' {
                if current.get(cursor).copied() != Some(line) {
                    return Err(conflict());
                }
                cursor += 1;
                new_seen += 1;
            }
            if prefix != b'+' {
                output.extend_from_slice(line);
                old_seen += 1;
                before_cursor += 1;
            }
            if output.len() > MAX_REVERT_FILE_BYTES || old_seen > old_count || new_seen > new_count
            {
                return Err(invalid());
            }
        }
        if old_seen != old_count || new_seen != new_count {
            return Err(invalid());
        }
        hunks += 1;
    }
    if hunks == 0 {
        return Err(invalid());
    }
    for line in &current[cursor..] {
        output.extend_from_slice(line);
    }
    if output.len() > MAX_REVERT_FILE_BYTES {
        return Err(invalid());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mutation(
        operation: ToolMutationOperation,
        before: &[u8],
        after: &[u8],
        diff: &str,
    ) -> LocatedMutation {
        LocatedMutation {
            run_id: "run".to_string(),
            operation,
            diff: Some(diff.to_string()),
            before_version: Some(version(before)),
            after_version: Some(version(after)),
        }
    }

    fn root() -> tempfile::TempDir {
        tempfile::Builder::new()
            .tempdir_in(std::env::temp_dir().canonicalize().unwrap())
            .unwrap()
    }

    #[test]
    fn create_uses_exact_bytes_including_empty_crlf_and_header_like_content() {
        for bytes in [
            b"".as_slice(),
            b"no final newline",
            b"line\r\n",
            b"++ header\nkeep\n",
        ] {
            let dir = root();
            let path = dir.path().join("new.txt");
            std::fs::write(&path, bytes).unwrap();
            let mutation = mutation(
                ToolMutationOperation::Create,
                b"",
                bytes,
                "... diff truncated\n",
            );
            assert_eq!(
                revert_create(&mutation, dir.path(), &path, "new.txt").unwrap(),
                ProductMutationRevertAction::DeletedFile
            );
            assert!(!path.exists());
            assert_eq!(
                revert_create(&mutation, dir.path(), &path, "new.txt").unwrap(),
                ProductMutationRevertAction::DeletedFile
            );
        }
    }

    #[test]
    fn create_refuses_later_normalization_and_header_removal() {
        for (before, later) in [
            (b"hello".as_slice(), b"hello\n".as_slice()),
            (b"hello\r\n", b"hello\n"),
            (b"++ header\nkeep\n", b"keep\n"),
        ] {
            let dir = root();
            let path = dir.path().join("new.txt");
            std::fs::write(&path, later).unwrap();
            let mutation = mutation(ToolMutationOperation::Create, b"", before, "");
            assert_eq!(
                revert_create(&mutation, dir.path(), &path, "new.txt")
                    .unwrap_err()
                    .code(),
                "revert_conflict"
            );
            assert_eq!(std::fs::read(path).unwrap(), later);
        }
    }

    #[test]
    fn legacy_and_invalid_digest_never_authorize_a_delete() {
        let dir = root();
        let path = dir.path().join("new.txt");
        std::fs::write(&path, b"hello\n").unwrap();
        let mut mutation = mutation(
            ToolMutationOperation::Create,
            b"",
            b"hello\n",
            "@@ -0,0 +1,1 @@\n+hello\n",
        );
        for digest in [None, Some("sha256:bad".to_string())] {
            mutation.after_version = digest;
            assert_eq!(
                revert_create(&mutation, dir.path(), &path, "new.txt")
                    .unwrap_err()
                    .code(),
                "revert_unverifiable"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"hello\n");
        }
    }

    #[test]
    fn reverse_patch_preserves_line_terminators_and_header_like_body() {
        let cases = [
            (
                "old",
                "new",
                "@@ -1,1 +1,1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
            ),
            ("old\r\n", "new\r\n", "@@ -1,1 +1,1 @@\n-old\r\n+new\r\n"),
            (
                "hello",
                "hello\n",
                "@@ -1,1 +1,1 @@\n-hello\n\\ No newline at end of file\n+hello\n",
            ),
            (
                "hello\n",
                "hello",
                "@@ -1,1 +1,1 @@\n-hello\n+hello\n\\ No newline at end of file\n",
            ),
            (
                "hello\n",
                "hello\r\n",
                "@@ -1,1 +1,1 @@\n-hello\n+hello\r\n",
            ),
            (
                "old\n",
                "++ header\n",
                "@@ -1,1 +1,1 @@\n-old\n+++ header\n",
            ),
            ("", "added\n", "@@ -0,0 +1,1 @@\n+added\n"),
            ("removed\n", "", "@@ -1,1 +0,0 @@\n-removed\n"),
        ];
        for (before, after, hunk) in cases {
            let diff = format!("--- a/space name.txt\n+++ b/space name.txt\n{hunk}");
            assert_eq!(
                reverse_patch("space name.txt", after.as_bytes(), &diff).unwrap(),
                before.as_bytes()
            );
        }
    }

    #[test]
    fn reverse_patch_checks_disjoint_hunk_offsets_and_context() {
        let diff = "--- a/a\n+++ b/a\n@@ -2,1 +2,2 @@\n-old\n+new\n+extra\n@@ -4,1 +5,1 @@\n-old2\n+new2\n";
        assert_eq!(
            reverse_patch("a", b"start\nnew\nextra\nmiddle\nnew2\nend\n", diff).unwrap(),
            b"start\nold\nmiddle\nold2\nend\n"
        );
        assert!(reverse_patch("a", b"start\nchanged\nextra\nmiddle\nnew2\nend\n", diff).is_err());
        assert!(
            reverse_patch(
                "a",
                b"start\nnew\nextra\nmiddle\nnew2\nend\n",
                &diff.replace("-4,1", "-5,1")
            )
            .is_err()
        );
    }

    #[test]
    fn incomplete_mismatched_and_multifile_patches_are_rejected() {
        let valid = "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        for diff in [
            valid.replace("a/a", "a/other"),
            valid.replace("b/a", "b/other"),
            format!("{valid}{valid}"),
            valid.replace("+1,1", "+1,2"),
            valid.trim_end().to_string(),
            format!("{valid}... diff truncated\n"),
            "--- a/a\n+++ b/a\n".to_string(),
        ] {
            assert!(
                reverse_patch("a", b"new\n", &diff).is_err(),
                "accepted {diff:?}"
            );
        }
    }

    #[test]
    fn update_checks_both_digests_and_publishes_only_verified_target() {
        let dir = root();
        let path = dir.path().join("a");
        let diff = "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        let mutation = mutation(ToolMutationOperation::Update, b"old\n", b"new\n", diff);
        std::fs::write(&path, b"new\n").unwrap();
        assert_eq!(
            revert_update(
                &mutation,
                dir.path(),
                &path,
                "a",
                &ProductWorkspaceKind::Repo
            )
            .unwrap(),
            ProductMutationRevertAction::ReversePatched
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old\n");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "staging file must be removed"
        );
        assert_eq!(
            revert_update(
                &mutation,
                dir.path(),
                &path,
                "a",
                &ProductWorkspaceKind::Repo
            )
            .unwrap_err()
            .code(),
            "revert_conflict"
        );
    }

    #[test]
    fn redacted_preimage_and_later_edits_leave_file_intact() {
        let dir = root();
        let path = dir.path().join("a");
        let diff = "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-[redacted]\n+public\n";
        let mutation = mutation(
            ToolMutationOperation::Update,
            b"private original\n",
            b"public\n",
            diff,
        );
        std::fs::write(&path, b"public\n").unwrap();
        assert_eq!(
            revert_update(
                &mutation,
                dir.path(),
                &path,
                "a",
                &ProductWorkspaceKind::Repo
            )
            .unwrap_err()
            .code(),
            "revert_unverifiable"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"public\n");
        std::fs::write(&path, b"later edit\n").unwrap();
        assert_eq!(
            revert_update(
                &mutation,
                dir.path(),
                &path,
                "a",
                &ProductWorkspaceKind::Repo
            )
            .unwrap_err()
            .code(),
            "revert_conflict"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"later edit\n");
    }

    #[test]
    fn traversal_directories_and_oversized_files_are_refused() {
        let dir = root();
        for path in ["../outside", "/outside", "a\nb", "a\rb"] {
            assert!(resolve_in_workspace(dir.path(), path).is_err());
        }
        let path = dir.path().join("a");
        std::fs::create_dir(&path).unwrap();
        assert!(read_current(dir.path(), &path, "a").is_err());
        std::fs::remove_dir(path).unwrap();
        let path = dir.path().join("large");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_REVERT_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(read_current(dir.path(), &path, "large").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn internal_external_and_parent_symlinks_never_redirect_rollback() {
        use std::os::unix::fs::symlink;
        let dir = root();
        let outside = root();
        let mutation = mutation(ToolMutationOperation::Create, b"", b"keep\n", "");
        std::fs::write(dir.path().join("unrelated"), b"keep\n").unwrap();
        std::fs::write(outside.path().join("unrelated"), b"keep\n").unwrap();
        for (name, target) in [
            ("internal", dir.path().join("unrelated")),
            ("external", outside.path().join("unrelated")),
        ] {
            symlink(&target, dir.path().join(name)).unwrap();
            assert!(
                revert_verified_mutation(
                    dir.path(),
                    name,
                    &ProductWorkspaceKind::Folder,
                    &mutation
                )
                .is_err()
            );
            assert_eq!(std::fs::read(target).unwrap(), b"keep\n");
            assert!(
                std::fs::symlink_metadata(dir.path().join(name))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
        std::fs::create_dir(dir.path().join("real-parent")).unwrap();
        std::fs::write(dir.path().join("real-parent/unrelated"), b"keep\n").unwrap();
        for (name, target) in [
            ("internal-parent", dir.path().join("real-parent")),
            ("external-parent", outside.path().to_path_buf()),
        ] {
            symlink(&target, dir.path().join(name)).unwrap();
            assert!(
                revert_verified_mutation(
                    dir.path(),
                    &format!("{name}/unrelated"),
                    &ProductWorkspaceKind::Folder,
                    &mutation
                )
                .is_err()
            );
            assert_eq!(std::fs::read(target.join("unrelated")).unwrap(), b"keep\n");
            assert!(resolve_in_workspace(dir.path(), &format!("{name}/missing")).is_err());
        }
    }

    #[tokio::test]
    async fn unreadable_newest_report_does_not_fall_back_to_older_mutation() {
        use rove_runtime::state::report::{RunReport, write_report};
        use rove_runtime::state::store::StateStore;
        use rove_runtime::types::{JobId, RunId, SessionId, TerminationReason};
        let dir = root();
        let store = StateStore::new(dir.path());
        let session = ProductSessionId::new();
        let bindings: Vec<_> = (1..=2)
            .map(|ordinal| rove_product_store::ProductSessionRunBinding {
                product_session_id: session.clone(),
                ordinal,
                runtime_session_id: SessionId::new(),
                runtime_job_id: JobId::new(),
                runtime_run_id: RunId::new(),
                resumed_from_run_id: None,
                bound_at: "2026-11-09T00:00:00Z".to_string(),
            })
            .collect();
        for binding in &bindings {
            store
                .start_run(
                    binding.runtime_session_id,
                    binding.runtime_job_id,
                    binding.runtime_run_id,
                )
                .unwrap();
        }
        let first = &bindings[0];
        let mut report = RunReport::new(
            first.runtime_session_id,
            first.runtime_job_id,
            first.runtime_run_id,
            dir.path().to_path_buf(),
            rove_runtime::workspace::WorkspaceKind::Folder,
            "fake".to_string(),
            TerminationReason::Final,
        );
        report.tool_mutations.push(rove_core::ToolMutation {
            path: "a".to_string(),
            operation: ToolMutationOperation::Create,
            diff: None,
            before_version: None,
            after_version: Some(version(b"old")),
        });
        let report_path =
            write_report(&store.run_store.run_dir(&first.runtime_run_id), &report).unwrap();
        store
            .record_report(
                first.runtime_run_id,
                report_path,
                "success".to_string(),
                "final".to_string(),
            )
            .await
            .unwrap();
        assert!(
            locate_mutation(&store, &bindings[..1], "a", None)
                .await
                .unwrap()
                .is_some()
        );
        let newer_dir = store.run_store.run_dir(&bindings[1].runtime_run_id);
        std::fs::create_dir_all(&newer_dir).unwrap();
        let newer_path = newer_dir.join("report.json");
        std::fs::write(&newer_path, "invalid").unwrap();
        store
            .record_report(
                bindings[1].runtime_run_id,
                newer_path,
                "success".to_string(),
                "final".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(
            locate_mutation(&store, &bindings, "a", None)
                .await
                .err()
                .unwrap()
                .code(),
            "revert_unverifiable"
        );
    }
}
