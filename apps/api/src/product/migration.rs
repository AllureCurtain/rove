//! Conservative M1 browser-state migration coordination.
//!
//! Browser runtime IDs are hints only. This module verifies the one runtime
//! shape the current schema can prove: a terminal job with exactly one terminal
//! run and an exact, workspace-consistent task-state snapshot. Multi-run chains
//! remain ambiguous until the runtime persists an explicit predecessor.

use std::collections::{BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rove_runtime::state::index::JobRunInspectionSnapshot;
use rove_runtime::state::store::{StateStore, validate_task_state_schema};
use rove_runtime::types::{JobId, RunId, TaskState};
use rove_runtime::workspace::{Workspace, WorkspaceKind};
use tokio::io::AsyncReadExt;

use super::{
    M1BrowserMigrationRequest, M1MigrationIssue, M1MigrationIssueCode, M1SessionImport,
    M1WorkspaceImport, PreparedM1BrowserMigration, ProductErrorCode, ProductStoreError,
    ProductWorkspaceKind, VerifiedM1SessionRunBinding,
};

const MAX_MIGRATION_RUNTIME_INSPECTIONS: usize = 256;
const MAX_MIGRATION_TASK_STATE_BYTES: u64 = 16 * 1_048_576;
const MAX_MIGRATION_TASK_STATE_TOTAL_BYTES: u64 = 64 * 1_048_576;

pub(crate) async fn prepare_m1_browser_migration<F>(
    request: M1BrowserMigrationRequest,
    mut state_store_for: F,
) -> Result<PreparedM1BrowserMigration, ProductStoreError>
where
    F: FnMut(&Workspace) -> StateStore,
{
    let mut runtime_workspaces = HashMap::new();
    for workspace in &request.workspaces {
        if let Some(runtime) = runtime_workspace_for_import(workspace).await {
            runtime_workspaces.insert(workspace.source_id.trim().to_string(), runtime);
        }
    }
    let mut hint_candidates = Vec::new();
    let mut issues = Vec::new();

    for session in &request.sessions {
        let hint = match parse_runtime_hint(session) {
            Ok(Some(hint)) => hint,
            Ok(None) => continue,
            Err(code) => {
                push_runtime_issue(&mut issues, session, code);
                continue;
            }
        };
        let Some(workspace) = runtime_workspaces.get(session.source_workspace_id.trim()) else {
            // The store records invalid or missing workspaces in the same
            // atomic acknowledgement and creates no session to bind.
            continue;
        };
        hint_candidates.push(RuntimeHintCandidate {
            session: session.clone(),
            hint,
            workspace: workspace.clone(),
        });
    }

    let contested = contested_hint_candidate_indices(&hint_candidates);
    for index in &contested {
        push_runtime_issue(
            &mut issues,
            &hint_candidates[*index].session,
            M1MigrationIssueCode::AmbiguousRuntimeBinding,
        );
    }
    let hint_candidates = hint_candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, candidate)| (!contested.contains(&index)).then_some(candidate))
        .collect::<Vec<_>>();
    if hint_candidates.len() > MAX_MIGRATION_RUNTIME_INSPECTIONS {
        return Err(runtime_inspection_limit_error());
    }

    let mut candidates = Vec::new();
    let mut read_budget = MigrationReadBudget::new();
    for candidate in hint_candidates {
        let state_store = state_store_for(&candidate.workspace);
        let snapshot = match state_store
            .index
            .inspect_job_run_read_only_async(candidate.hint.job_id, candidate.hint.run_id, 1)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                push_runtime_issue(
                    &mut issues,
                    &candidate.session,
                    M1MigrationIssueCode::RuntimeBindingNotFound,
                );
                continue;
            }
            Err(_) => return Err(runtime_storage_error()),
        };
        match verify_singleton_binding(
            &candidate.session,
            candidate.hint,
            &candidate.workspace,
            &state_store,
            snapshot,
            &mut read_budget,
        )
        .await
        {
            Ok(binding) => candidates.push(binding),
            Err(BindingVerificationError::Issue(code)) => {
                push_runtime_issue(&mut issues, &candidate.session, code);
            }
            Err(BindingVerificationError::Storage) => return Err(runtime_storage_error()),
            Err(BindingVerificationError::Limit) => return Err(runtime_inspection_limit_error()),
        }
    }

    reject_contested_runtime_identities(&mut candidates, &mut issues);
    Ok(PreparedM1BrowserMigration {
        request,
        verified_run_bindings: candidates,
        issues,
    })
}

#[derive(Debug, Clone, Copy)]
struct ExactRuntimeHint {
    job_id: JobId,
    run_id: RunId,
}

#[derive(Debug, Clone)]
struct RuntimeHintCandidate {
    session: M1SessionImport,
    hint: ExactRuntimeHint,
    workspace: Workspace,
}

#[derive(Debug)]
struct MigrationReadBudget {
    remaining: u64,
}

impl MigrationReadBudget {
    fn new() -> Self {
        Self {
            remaining: MAX_MIGRATION_TASK_STATE_TOTAL_BYTES,
        }
    }

    fn reserve_read_limit(&mut self, observed_bytes: u64) -> Result<u64, BindingVerificationError> {
        let read_limit = observed_bytes.min(self.remaining);
        if read_limit != observed_bytes {
            return Err(BindingVerificationError::Limit);
        }
        self.remaining -= read_limit;
        Ok(read_limit)
    }
}

#[derive(Debug, Clone, Copy)]
enum BindingVerificationError {
    Issue(M1MigrationIssueCode),
    Storage,
    Limit,
}

impl From<M1MigrationIssueCode> for BindingVerificationError {
    fn from(code: M1MigrationIssueCode) -> Self {
        Self::Issue(code)
    }
}

fn parse_runtime_hint(
    session: &M1SessionImport,
) -> Result<Option<ExactRuntimeHint>, M1MigrationIssueCode> {
    let pair = match (
        session.legacy_active_job_id.as_deref(),
        session.legacy_active_run_id.as_deref(),
    ) {
        (None, None) => {
            if session.legacy_resumed_from_run_id.is_some() {
                return Err(M1MigrationIssueCode::InvalidRuntimeHint);
            }
            return if session.legacy_has_durable_turn {
                Err(M1MigrationIssueCode::RuntimeBindingNotFound)
            } else {
                Ok(None)
            };
        }
        (Some(job_id), Some(run_id)) => (job_id, run_id),
        _ => return Err(M1MigrationIssueCode::InvalidRuntimeHint),
    };

    let job_id = pair
        .0
        .parse::<JobId>()
        .map_err(|_| M1MigrationIssueCode::InvalidRuntimeHint)?;
    let run_id = pair
        .1
        .parse::<RunId>()
        .map_err(|_| M1MigrationIssueCode::InvalidRuntimeHint)?;
    if let Some(predecessor) = session.legacy_resumed_from_run_id.as_deref() {
        predecessor
            .parse::<RunId>()
            .map_err(|_| M1MigrationIssueCode::InvalidRuntimeHint)?;
        return Err(M1MigrationIssueCode::AmbiguousRuntimeBinding);
    }
    Ok(Some(ExactRuntimeHint { job_id, run_id }))
}

fn contested_hint_candidate_indices(candidates: &[RuntimeHintCandidate]) -> BTreeSet<usize> {
    let mut jobs = HashMap::new();
    let mut runs = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        jobs.entry(candidate.hint.job_id)
            .or_insert_with(Vec::new)
            .push(index);
        runs.entry(candidate.hint.run_id)
            .or_insert_with(Vec::new)
            .push(index);
    }
    let mut contested = BTreeSet::new();
    for indices in jobs.values().chain(runs.values()) {
        if indices.len() > 1 {
            contested.extend(indices.iter().copied());
        }
    }
    contested
}

async fn runtime_workspace_for_import(import: &M1WorkspaceImport) -> Option<Workspace> {
    if !migration_import_root_is_local(&import.root) {
        return None;
    }
    let root = tokio::fs::canonicalize(&import.root).await.ok()?;
    if !runtime_path_uses_local_disk_namespace(&root)
        || !tokio::fs::metadata(&root).await.ok()?.is_dir()
    {
        return None;
    }
    let kind = match import.kind {
        ProductWorkspaceKind::Folder => WorkspaceKind::Folder,
        ProductWorkspaceKind::Repo => {
            if !tokio::fs::try_exists(root.join(".git")).await.ok()? {
                return None;
            }
            WorkspaceKind::Repo
        }
    };
    Some(Workspace {
        state_dir: root.join(".rove"),
        root,
        kind,
    })
}

fn migration_import_root_is_local(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        return matches!(
            path.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        );
    }
    #[cfg(not(windows))]
    {
        true
    }
}

async fn verify_singleton_binding(
    session: &M1SessionImport,
    hint: ExactRuntimeHint,
    workspace: &Workspace,
    state_store: &StateStore,
    snapshot: JobRunInspectionSnapshot,
    read_budget: &mut MigrationReadBudget,
) -> Result<VerifiedM1SessionRunBinding, BindingVerificationError> {
    let job = snapshot
        .job
        .ok_or(M1MigrationIssueCode::RuntimeBindingNotFound)?;
    let run = snapshot
        .run
        .ok_or(M1MigrationIssueCode::RuntimeBindingNotFound)?;
    let task_state_path = snapshot
        .task_state_path
        .ok_or(M1MigrationIssueCode::RuntimeBindingNotFound)?;

    if job.job_id != hint.job_id
        || run.run_id != hint.run_id
        || run.job_id != hint.job_id
        || job.session_id != run.session_id
    {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }
    if job.run_id != Some(hint.run_id)
        || snapshot.job_runs_truncated
        || snapshot.job_run_ids.len() != 1
        || snapshot.job_run_ids[0] != hint.run_id
        || !is_terminal_runtime_status(&job.status)
        || !is_terminal_runtime_status(&run.status)
    {
        return Err(M1MigrationIssueCode::AmbiguousRuntimeBinding.into());
    }

    let expected_run_dir = state_store.run_store.run_dir(&hint.run_id);
    let expected_task_state_path = expected_run_dir.join("task_state.json");
    let run_task_state_path = run
        .task_state_path
        .as_ref()
        .ok_or(M1MigrationIssueCode::RuntimeBindingNotFound)?;
    if !runtime_record_path_matches(&run.run_dir, &expected_run_dir)
        || !runtime_record_path_matches(run_task_state_path, &expected_task_state_path)
        || !runtime_record_path_matches(&task_state_path, &expected_task_state_path)
    {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }
    let expected_runs_dir = expected_run_dir
        .parent()
        .ok_or(M1MigrationIssueCode::InvalidRuntimeHint)?;
    let expected_state_dir = expected_runs_dir
        .parent()
        .ok_or(M1MigrationIssueCode::InvalidRuntimeHint)?;
    let canonical_state_dir = canonical_runtime_artifact_path(expected_state_dir).await?;
    let canonical_runs_dir = canonical_runtime_artifact_path(expected_runs_dir).await?;
    let canonical_run_dir = canonical_runtime_artifact_path(&expected_run_dir).await?;
    let canonical_task_state_path =
        canonical_runtime_artifact_path(&expected_task_state_path).await?;
    if !runtime_path_uses_local_disk_namespace(&canonical_state_dir)
        || !runtime_path_uses_local_disk_namespace(&canonical_runs_dir)
        || !runtime_path_uses_local_disk_namespace(&canonical_run_dir)
        || !runtime_path_uses_local_disk_namespace(&canonical_task_state_path)
        || canonical_runs_dir.parent() != Some(canonical_state_dir.as_path())
        || canonical_run_dir.parent() != Some(canonical_runs_dir.as_path())
        || canonical_task_state_path.parent() != Some(canonical_run_dir.as_path())
    {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }
    let task_state = read_task_state(&canonical_task_state_path, read_budget).await?;
    if task_state.session_id != job.session_id
        || task_state.job_id != hint.job_id
        || task_state.run_id != hint.run_id
        || !task_state_matches_workspace(&task_state, workspace)
    {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }

    let verified_workspace_kind = match workspace.kind {
        WorkspaceKind::Folder => ProductWorkspaceKind::Folder,
        WorkspaceKind::Repo => ProductWorkspaceKind::Repo,
        WorkspaceKind::Task => return Err(M1MigrationIssueCode::InvalidRuntimeHint.into()),
    };

    Ok(VerifiedM1SessionRunBinding {
        source_session_id: session.source_id.clone(),
        ordinal: 1,
        runtime_session_id: job.session_id,
        runtime_job_id: hint.job_id,
        runtime_run_id: hint.run_id,
        resumed_from_run_id: None,
        verified_workspace_root: workspace.root.clone(),
        verified_workspace_kind,
    })
}

fn is_terminal_runtime_status(status: &str) -> bool {
    matches!(status, "done" | "error" | "cancelled" | "interrupted")
}

fn runtime_record_path_matches(candidate: &Path, expected: &Path) -> bool {
    candidate.is_absolute()
        && runtime_path_uses_local_disk_namespace(candidate)
        && candidate == expected
}

fn runtime_path_uses_local_disk_namespace(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        return matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        );
    }
    #[cfg(not(windows))]
    {
        true
    }
}

async fn canonical_runtime_artifact_path(path: &Path) -> Result<PathBuf, BindingVerificationError> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => {
                BindingVerificationError::Issue(M1MigrationIssueCode::RuntimeBindingNotFound)
            }
            _ => BindingVerificationError::Storage,
        })
}

async fn read_task_state(
    path: &Path,
    read_budget: &mut MigrationReadBudget,
) -> Result<TaskState, BindingVerificationError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => {
                BindingVerificationError::Issue(M1MigrationIssueCode::RuntimeBindingNotFound)
            }
            _ => BindingVerificationError::Storage,
        })?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| BindingVerificationError::Storage)?;
    if !metadata.is_file() || metadata.len() > MAX_MIGRATION_TASK_STATE_BYTES {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }
    let expected_len = metadata.len();
    let read_limit = read_budget.reserve_read_limit(expected_len)?;
    let expected_modified = metadata
        .modified()
        .map_err(|_| BindingVerificationError::Storage)?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len().min(MAX_MIGRATION_TASK_STATE_BYTES)).unwrap_or_default(),
    );
    let mut limited = file.take(read_limit);
    limited
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| BindingVerificationError::Storage)?;
    let metadata_after = limited
        .get_ref()
        .metadata()
        .await
        .map_err(|_| BindingVerificationError::Storage)?;
    if !task_state_read_matches_metadata(
        expected_len,
        expected_modified,
        &metadata_after,
        bytes.len(),
    )? {
        return Err(M1MigrationIssueCode::InvalidRuntimeHint.into());
    }
    let state = serde_json::from_slice(&bytes)
        .map_err(|_| BindingVerificationError::Issue(M1MigrationIssueCode::InvalidRuntimeHint))?;
    validate_task_state_schema(&state)
        .map_err(|_| BindingVerificationError::Issue(M1MigrationIssueCode::InvalidRuntimeHint))?;
    Ok(state)
}

fn task_state_read_matches_metadata(
    expected_len: u64,
    expected_modified: std::time::SystemTime,
    metadata_after: &std::fs::Metadata,
    bytes_read: usize,
) -> Result<bool, BindingVerificationError> {
    Ok(
        u64::try_from(bytes_read).unwrap_or(u64::MAX) == expected_len
            && metadata_after.len() == expected_len
            && metadata_after
                .modified()
                .map_err(|_| BindingVerificationError::Storage)?
                == expected_modified,
    )
}

fn task_state_matches_workspace(task_state: &TaskState, workspace: &Workspace) -> bool {
    let Some(identity) = task_state.runtime_identity.as_ref() else {
        return false;
    };
    if identity.workspace_kind != workspace.kind {
        return false;
    }
    let actual = PathBuf::from(&identity.cwd);
    actual.is_absolute()
        && runtime_path_uses_local_disk_namespace(&actual)
        && actual == workspace.root
}

fn reject_contested_runtime_identities(
    candidates: &mut Vec<VerifiedM1SessionRunBinding>,
    issues: &mut Vec<M1MigrationIssue>,
) {
    let mut sessions = HashMap::new();
    let mut jobs = HashMap::new();
    let mut runs = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        sessions
            .entry(candidate.runtime_session_id)
            .or_insert_with(Vec::new)
            .push(index);
        jobs.entry(candidate.runtime_job_id)
            .or_insert_with(Vec::new)
            .push(index);
        runs.entry(candidate.runtime_run_id)
            .or_insert_with(Vec::new)
            .push(index);
    }
    let mut contested = BTreeSet::new();
    for indices in sessions.values().chain(jobs.values()).chain(runs.values()) {
        if indices.len() > 1 {
            contested.extend(indices.iter().copied());
        }
    }
    if contested.is_empty() {
        return;
    }
    for index in &contested {
        let source_id = candidates[*index].source_session_id.clone();
        push_issue_unique(
            issues,
            M1MigrationIssue {
                code: M1MigrationIssueCode::AmbiguousRuntimeBinding,
                entity: "session_runtime_binding".to_string(),
                source_id: Some(source_id),
            },
        );
    }
    let mut index = 0;
    candidates.retain(|_| {
        let keep = !contested.contains(&index);
        index += 1;
        keep
    });
}

fn push_runtime_issue(
    issues: &mut Vec<M1MigrationIssue>,
    session: &M1SessionImport,
    code: M1MigrationIssueCode,
) {
    push_issue_unique(
        issues,
        M1MigrationIssue {
            code,
            entity: "session_runtime_binding".to_string(),
            source_id: Some(session.source_id.clone()),
        },
    );
}

fn push_issue_unique(issues: &mut Vec<M1MigrationIssue>, issue: M1MigrationIssue) {
    if !issues.iter().any(|existing| {
        existing.code == issue.code
            && existing.entity == issue.entity
            && existing.source_id == issue.source_id
    }) {
        issues.push(issue);
    }
}

fn runtime_storage_error() -> ProductStoreError {
    ProductStoreError::new(
        ProductErrorCode::ProductStorageFailure,
        "workspace runtime state inspection failed",
    )
}

fn runtime_inspection_limit_error() -> ProductStoreError {
    ProductStoreError::new(
        ProductErrorCode::ProductInvalidInput,
        "browser migration runtime inspection budget exceeded",
    )
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use rove_runtime::execution::StepLedgerState;
    use rove_runtime::runtime_identity::RuntimeIdentity;
    use rove_runtime::types::{ApprovalPolicy, SessionId};

    use super::super::{M1BrowserMigrationSource, M1SafePreferencesImport, ProductThemePreference};
    use super::*;

    #[tokio::test]
    async fn singleton_terminal_runtime_binding_is_verified() {
        let fixture = RuntimeFixture::new().await;
        let request = fixture.request(vec![fixture.session_import("legacy-session")]);

        let prepared = prepare_m1_browser_migration(request, |workspace| {
            StateStore::new(&workspace.state_dir)
        })
        .await
        .unwrap();

        assert!(prepared.issues.is_empty());
        assert_eq!(prepared.verified_run_bindings.len(), 1);
        let binding = &prepared.verified_run_bindings[0];
        assert_eq!(binding.source_session_id, "legacy-session");
        assert_eq!(binding.ordinal, 1);
        assert_eq!(binding.runtime_session_id, fixture.session_id);
        assert_eq!(binding.runtime_job_id, fixture.job_id);
        assert_eq!(binding.runtime_run_id, fixture.run_id);
        assert_eq!(binding.resumed_from_run_id, None);
        assert_eq!(binding.verified_workspace_root, fixture.workspace_root);
        assert_eq!(
            binding.verified_workspace_kind,
            ProductWorkspaceKind::Folder
        );
    }

    #[tokio::test]
    async fn contested_runtime_identity_rejects_every_candidate() {
        let fixture = RuntimeFixture::new().await;
        let request = fixture.request(vec![
            fixture.session_import("legacy-session-a"),
            fixture.session_import("legacy-session-b"),
        ]);

        let prepared = prepare_m1_browser_migration(request, |_| {
            panic!("duplicate runtime hints must be rejected before state inspection")
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(prepared.issues.len(), 2);
        assert!(prepared.issues.iter().all(|issue| {
            issue.code == M1MigrationIssueCode::AmbiguousRuntimeBinding
                && issue.entity == "session_runtime_binding"
        }));
        assert_eq!(
            prepared.issues[0].source_id.as_deref(),
            Some("legacy-session-a")
        );
        assert_eq!(
            prepared.issues[1].source_id.as_deref(),
            Some("legacy-session-b")
        );
    }

    #[tokio::test]
    async fn predecessor_hint_is_ambiguous_without_accessing_runtime_state() {
        let fixture = RuntimeFixture::new().await;
        let mut session = fixture.session_import("legacy-session");
        session.legacy_resumed_from_run_id = Some(RunId::new().to_string());
        let request = fixture.request(vec![session]);

        let prepared = prepare_m1_browser_migration(request, |_| {
            panic!("ambiguous predecessor hints must not inspect runtime state")
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(prepared.issues.len(), 1);
        assert_eq!(
            prepared.issues[0].code,
            M1MigrationIssueCode::AmbiguousRuntimeBinding
        );
    }

    #[tokio::test]
    async fn unsupported_task_state_schema_rejects_runtime_binding() {
        let fixture = RuntimeFixture::new().await;
        fixture
            .rewrite_task_state(|state| state.schema_version += 1)
            .await;
        let request = fixture.request(vec![fixture.session_import("legacy-session")]);

        let prepared = prepare_m1_browser_migration(request, |workspace| {
            StateStore::new(&workspace.state_dir)
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(prepared.issues.len(), 1);
        assert_eq!(
            prepared.issues[0].code,
            M1MigrationIssueCode::InvalidRuntimeHint
        );
    }

    #[tokio::test]
    async fn task_state_identity_mismatch_rejects_runtime_binding() {
        let fixture = RuntimeFixture::new().await;
        fixture
            .rewrite_task_state(|state| state.job_id = JobId::new())
            .await;
        let request = fixture.request(vec![fixture.session_import("legacy-session")]);

        let prepared = prepare_m1_browser_migration(request, |workspace| {
            StateStore::new(&workspace.state_dir)
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(
            prepared.issues[0].code,
            M1MigrationIssueCode::InvalidRuntimeHint
        );
    }

    #[tokio::test]
    async fn untrusted_index_artifact_path_is_rejected_without_resolution() {
        let fixture = RuntimeFixture::new().await;
        fixture.tamper_run_dir(r"\\migration.invalid\share\run");
        let request = fixture.request(vec![fixture.session_import("legacy-session")]);

        let prepared = prepare_m1_browser_migration(request, |workspace| {
            StateStore::new(&workspace.state_dir)
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(
            prepared.issues[0].code,
            M1MigrationIssueCode::InvalidRuntimeHint
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonical_run_directory_cannot_escape_its_runs_parent() {
        use std::os::unix::fs::symlink;

        let fixture = RuntimeFixture::new().await;
        let expected_run_dir = fixture
            .workspace_root
            .join(".rove")
            .join("runs")
            .join(fixture.run_id.to_string());
        let escaped_run_dir = fixture.workspace_root.join("escaped-run");
        std::fs::rename(&expected_run_dir, &escaped_run_dir).unwrap();
        symlink(&escaped_run_dir, &expected_run_dir).unwrap();
        let request = fixture.request(vec![fixture.session_import("legacy-session")]);

        let prepared = prepare_m1_browser_migration(request, |workspace| {
            StateStore::new(&workspace.state_dir)
        })
        .await
        .unwrap();

        assert!(prepared.verified_run_bindings.is_empty());
        assert_eq!(
            prepared.issues[0].code,
            M1MigrationIssueCode::InvalidRuntimeHint
        );
    }

    #[tokio::test]
    async fn runtime_inspection_count_is_rejected_before_any_state_access() {
        let fixture = RuntimeFixture::new().await;
        let sessions = (0..=MAX_MIGRATION_RUNTIME_INSPECTIONS)
            .map(|index| {
                let mut session = fixture.session_import(&format!("legacy-session-{index}"));
                session.legacy_active_job_id = Some(JobId::new().to_string());
                session.legacy_active_run_id = Some(RunId::new().to_string());
                session
            })
            .collect();
        let request = fixture.request(sessions);

        let error = prepare_m1_browser_migration(request, |_| {
            panic!("over-budget migrations must fail before state inspection")
        })
        .await
        .unwrap_err();

        assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);
    }

    #[test]
    fn task_state_read_budget_is_reserved_before_reading() {
        let mut budget = MigrationReadBudget { remaining: 3 };

        assert!(matches!(
            budget.reserve_read_limit(4),
            Err(BindingVerificationError::Limit)
        ));
        assert_eq!(budget.remaining, 3);
    }

    #[test]
    fn task_state_growth_between_metadata_and_read_is_rejected() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("task_state.json");
        std::fs::write(&path, b"{}").unwrap();
        let metadata_before = std::fs::metadata(&path).unwrap();
        std::fs::write(&path, b"{}\n").unwrap();
        let metadata_after = std::fs::metadata(&path).unwrap();

        assert!(
            !task_state_read_matches_metadata(
                metadata_before.len(),
                metadata_before.modified().unwrap(),
                &metadata_after,
                usize::try_from(metadata_after.len()).unwrap(),
            )
            .unwrap()
        );
    }

    #[tokio::test]
    async fn oversized_task_state_is_rejected_before_budget_reservation() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("task_state.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_MIGRATION_TASK_STATE_BYTES + 1).unwrap();
        drop(file);
        let mut budget = MigrationReadBudget::new();

        let error = read_task_state(&path, &mut budget).await.unwrap_err();

        assert!(matches!(
            error,
            BindingVerificationError::Issue(M1MigrationIssueCode::InvalidRuntimeHint)
        ));
        assert_eq!(budget.remaining, MAX_MIGRATION_TASK_STATE_TOTAL_BYTES);
    }

    struct RuntimeFixture {
        _temp: TempDir,
        workspace_root: PathBuf,
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
    }

    impl RuntimeFixture {
        async fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let workspace = Workspace::open_folder(temp.path()).unwrap();
            let state_store = StateStore::new(&workspace.state_dir);
            let session_id = SessionId::new();
            let job_id = JobId::new();
            let run_id = RunId::new();
            let handle = state_store.start_run(session_id, job_id, run_id).unwrap();
            let task_state = TaskState {
                schema_version: 1,
                session_id,
                job_id,
                run_id,
                goal: "migrated task".to_string(),
                step: 1,
                history: Vec::new(),
                summary: Some("complete".to_string()),
                checkpoint: None,
                plan: None,
                runtime_identity: Some(RuntimeIdentity {
                    cwd: workspace.root.display().to_string(),
                    workspace_kind: workspace.kind.clone(),
                    model_id: "fake".to_string(),
                    provider_target: "fake".to_string(),
                    approval_policy: ApprovalPolicy::Never,
                    max_steps: 1,
                    plan_enabled: false,
                    system_prompt_hash: "system".to_string(),
                    planner_prompt_hash: "planner".to_string(),
                    workspace_fingerprint: "workspace".to_string(),
                    tool_signature: "tools".to_string(),
                }),
                step_ledger: StepLedgerState::default(),
            };
            state_store.write_task_state(&task_state).await.unwrap();
            state_store
                .record_report(
                    run_id,
                    handle.run_dir.join("report.json"),
                    "success".to_string(),
                    "final".to_string(),
                )
                .await
                .unwrap();
            drop(handle);
            Self {
                workspace_root: workspace.root,
                _temp: temp,
                session_id,
                job_id,
                run_id,
            }
        }

        fn session_import(&self, source_id: &str) -> M1SessionImport {
            M1SessionImport {
                source_id: source_id.to_string(),
                source_workspace_id: "legacy-workspace".to_string(),
                title: source_id.to_string(),
                created_at: "2026-07-26T00:00:00Z".to_string(),
                updated_at: "2026-07-26T00:00:00Z".to_string(),
                legacy_active_job_id: Some(self.job_id.to_string()),
                legacy_active_run_id: Some(self.run_id.to_string()),
                legacy_resumed_from_run_id: None,
                legacy_has_durable_turn: true,
            }
        }

        fn state_store(&self) -> StateStore {
            StateStore::new(&self.workspace_root.join(".rove"))
        }

        async fn rewrite_task_state(&self, update: impl FnOnce(&mut TaskState)) {
            let state_store = self.state_store();
            let mut state = state_store.load_task_state(self.run_id).await.unwrap();
            update(&mut state);
            state_store.write_task_state(&state).await.unwrap();
        }

        fn tamper_run_dir(&self, path: &str) {
            let state_store = self.state_store();
            let connection = Connection::open(state_store.index.path()).unwrap();
            connection
                .execute(
                    "UPDATE runs SET run_dir = ?2 WHERE run_id = ?1",
                    params![self.run_id.to_string(), path],
                )
                .unwrap();
        }

        fn request(&self, sessions: Vec<M1SessionImport>) -> M1BrowserMigrationRequest {
            M1BrowserMigrationRequest {
                source: M1BrowserMigrationSource::WebM1LocalStorage,
                source_schema_version: 1,
                idempotency_key: "migration-runtime-fixture".to_string(),
                workspaces: vec![M1WorkspaceImport {
                    source_id: "legacy-workspace".to_string(),
                    root: self.workspace_root.clone(),
                    kind: ProductWorkspaceKind::Folder,
                    display_name: "Legacy workspace".to_string(),
                    pinned: false,
                    last_opened_at: "2026-07-26T00:00:00Z".to_string(),
                }],
                sessions,
                provider_profiles: Vec::new(),
                safe_preferences: M1SafePreferencesImport {
                    theme: Some(ProductThemePreference::System),
                    source_active_workspace_id: None,
                    source_active_session_id: None,
                    provider_selection: None,
                },
            }
        }
    }
}
