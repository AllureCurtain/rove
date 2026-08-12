use std::{fs, path::PathBuf};

use chrono::{Duration, TimeZone, Utc};
use rove_runtime::types::{JobId, RunId, SessionId};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::product::{
    CommitProductRunBinding, CreateProductControlRequest, CreateProductForkRequest,
    CreateProductMessageRequest, CreateProductProviderProfileRequest, CreateProductSessionRequest,
    CreateProductWorkspaceRequest, M1BrowserMigrationPreflight, M1BrowserMigrationRequest,
    M1BrowserMigrationSource, M1MigrationDisposition, M1MigrationIssueCode, M1PreferencesBaseline,
    M1ProviderProfileImport, M1ProviderSelectionImport, M1SafePreferencesImport, M1SessionImport,
    M1WorkspaceImport, PreparedM1BrowserMigration, ProductApprovalPreference, ProductControlKind,
    ProductControlStatus, ProductErrorCode, ProductMessageStatus, ProductProviderSelection,
    ProductProviderType, ProductReasoningPreference, ProductSessionStatus, ProductStore,
    ProductThemePreference, ProductWorkspaceKind, UpdateProductPreferencesRequest,
    UpdateProductSessionModelConfigRequest, VerifiedM1SessionRunBinding,
    VerifiedProductForkBoundary,
};

use super::SqliteProductStore;
use super::repository::{now_rfc3339, remove_expired_migration_preparations_at};

fn open_store(temp: &TempDir) -> SqliteProductStore {
    SqliteProductStore::open(temp.path().join("product.sqlite"), 5_000).unwrap()
}

async fn preflight_baseline(
    store: &SqliteProductStore,
    request: &M1BrowserMigrationRequest,
) -> M1PreferencesBaseline {
    match store.preflight_m1_browser_migration(request).await.unwrap() {
        M1BrowserMigrationPreflight::Prepare(baseline) => baseline,
        M1BrowserMigrationPreflight::Replay(_) => {
            panic!("new migration unexpectedly replayed an existing receipt")
        }
    }
}

fn preference_migration_request(
    idempotency_key: &str,
    safe_preferences: M1SafePreferencesImport,
) -> M1BrowserMigrationRequest {
    M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: idempotency_key.to_string(),
        workspaces: Vec::new(),
        sessions: Vec::new(),
        provider_profiles: Vec::new(),
        safe_preferences,
    }
}

fn source_mapped_session_migration_request(
    root: PathBuf,
    idempotency_key: &str,
    title: &str,
    updated_at: &str,
) -> M1BrowserMigrationRequest {
    M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: idempotency_key.to_string(),
        workspaces: vec![M1WorkspaceImport {
            source_id: "claim-workspace".to_string(),
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: "Claim workspace".to_string(),
            pinned: false,
            last_opened_at: updated_at.to_string(),
        }],
        sessions: vec![M1SessionImport {
            source_id: "claim-session".to_string(),
            source_workspace_id: "claim-workspace".to_string(),
            title: title.to_string(),
            created_at: "2026-07-26T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            legacy_active_job_id: None,
            legacy_active_run_id: None,
            legacy_resumed_from_run_id: None,
            legacy_has_durable_turn: false,
        }],
        provider_profiles: Vec::new(),
        safe_preferences: M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    }
}

fn raw_preference_metadata(temp: &TempDir) -> (String, i64) {
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .query_row(
            "SELECT updated_at, revision FROM product_preferences WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn raw_migration_preparation_count(temp: &TempDir) -> i64 {
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM product_migration_preparations",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

async fn create_workspace_and_session(
    store: &SqliteProductStore,
    temp: &TempDir,
) -> (
    crate::product::ProductWorkspace,
    crate::product::ProductSession,
) {
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    let workspace = store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Test workspace".to_string()),
            pinned: false,
        })
        .await
        .unwrap();
    let session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: workspace.id.clone(),
            title: Some("Test session".to_string()),
        })
        .await
        .unwrap();
    (workspace, session)
}

async fn create_forkable_parent(
    store: &SqliteProductStore,
    temp: &TempDir,
) -> (
    crate::product::ProductWorkspace,
    crate::product::ProductSession,
    VerifiedProductForkBoundary,
) {
    let (workspace, parent) = create_workspace_and_session(store, temp).await;
    let claim = store.claim_session_turn(&parent.id).await.unwrap();
    let source_runtime_session_id = SessionId::new();
    let source_runtime_job_id = JobId::new();
    let source_runtime_run_id = RunId::new();
    store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim.claim_id.clone(),
            product_session_id: parent.id.clone(),
            runtime_session_id: source_runtime_session_id,
            runtime_job_id: source_runtime_job_id,
            runtime_run_id: source_runtime_run_id,
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: claim.model_config.clone(),
        })
        .await
        .unwrap();
    store
        .finish_session_turn(&claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();
    let boundary = VerifiedProductForkBoundary {
        parent_product_session_id: parent.id.clone(),
        parent_workspace_id: workspace.id.clone(),
        parent_title: parent.title.clone(),
        source_runtime_session_id,
        source_runtime_job_id,
        source_runtime_run_id,
        fork_at_event_seq: 4,
    };
    (workspace, parent, boundary)
}

#[tokio::test]
async fn session_model_config_is_seeded_cas_safe_and_forked_with_a_new_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;

    let initial = store.get_session_model_config(&session.id).await.unwrap();
    assert_eq!(initial.model, "fake");
    assert_eq!(initial.max_steps, 8);
    assert_eq!(initial.reasoning, ProductReasoningPreference::Default);
    assert_eq!(initial.revision, 1);

    let updated = store
        .update_session_model_config(
            &session.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: None,
                model: "fake-raw".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: 16,
                expected_revision: Some(initial.revision),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.model, "fake-raw");
    assert_eq!(updated.max_steps, 16);
    assert_eq!(updated.revision, 2);

    let stale = store
        .update_session_model_config(
            &session.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: None,
                model: "fake".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: 8,
                expected_revision: Some(initial.revision),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        stale.code,
        ProductErrorCode::ProductSessionModelConfigConflict
    );

    let (_, parent, boundary) = create_forkable_parent(&store, &temp).await;
    let parent_config = store.get_session_model_config(&parent.id).await.unwrap();
    let parent_config = store
        .update_session_model_config(
            &parent.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: None,
                model: "fake-raw".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: 16,
                expected_revision: Some(parent_config.revision),
            },
        )
        .await
        .unwrap();
    let (child, _, _) = store
        .create_fork(
            CreateProductForkRequest {
                fork_at_run_id: boundary.source_runtime_run_id,
                title: None,
                idempotency_key: "model-config-fork".to_string(),
            },
            boundary,
        )
        .await
        .unwrap();
    let child_config = store.get_session_model_config(&child.id).await.unwrap();
    assert_eq!(child_config.model, parent_config.model);
    assert_eq!(child_config.max_steps, parent_config.max_steps);
    assert_eq!(child_config.reasoning, parent_config.reasoning);
    assert_eq!(child_config.revision, 1);
}

#[tokio::test]
async fn run_model_snapshot_is_captured_at_claim_time() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let initial = store.get_session_model_config(&session.id).await.unwrap();
    let configured = store
        .update_session_model_config(
            &session.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: None,
                model: "fake-raw".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: 20,
                expected_revision: Some(initial.revision),
            },
        )
        .await
        .unwrap();
    let claim = store.claim_session_turn(&session.id).await.unwrap();
    assert_eq!(claim.model_config.revision, configured.revision);

    store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim.claim_id.clone(),
            product_session_id: session.id.clone(),
            runtime_session_id: SessionId::new(),
            runtime_job_id: JobId::new(),
            runtime_run_id: RunId::new(),
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: claim.model_config.clone(),
        })
        .await
        .unwrap();
    let snapshots = store.list_session_run_models(&session.id).await.unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].model, "fake-raw");
    assert_eq!(snapshots[0].max_steps, 20);
}

#[tokio::test]
async fn deleting_a_provider_profile_unbinds_session_model_configs() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let profile = store
        .create_provider_profile(CreateProductProviderProfileRequest {
            label: "Fake profile".to_string(),
            provider_type: ProductProviderType::Fake,
            api_base: String::new(),
            api_key_env: None,
            default_model: Some("fake".to_string()),
        })
        .await
        .unwrap();
    let initial = store.get_session_model_config(&session.id).await.unwrap();
    let configured = store
        .update_session_model_config(
            &session.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: Some(profile.id.clone()),
                model: "fake".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: 8,
                expected_revision: Some(initial.revision),
            },
        )
        .await
        .unwrap();

    let claim = store.claim_session_turn(&session.id).await.unwrap();
    store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim.claim_id.clone(),
            product_session_id: session.id.clone(),
            runtime_session_id: SessionId::new(),
            runtime_job_id: JobId::new(),
            runtime_run_id: RunId::new(),
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: claim.model_config.clone(),
        })
        .await
        .unwrap();
    store
        .finish_session_turn(&claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();

    store.delete_provider_profile(&profile.id).await.unwrap();
    let after = store.get_session_model_config(&session.id).await.unwrap();
    assert_eq!(after.profile_id, None);
    assert_eq!(after.model, configured.model);
    assert_eq!(after.revision, configured.revision + 1);
    let snapshots = store.list_session_run_models(&session.id).await.unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].profile_id, None);
    assert_eq!(snapshots[0].model, "fake");
}

#[tokio::test]
async fn claims_are_exclusive_and_bindings_are_contiguous() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;

    let first_claim = store.claim_session_turn(&session.id).await.unwrap();
    let conflict = store.claim_session_turn(&session.id).await.unwrap_err();
    assert_eq!(conflict.code, ProductErrorCode::ProductSessionActive);

    let runtime_session_id = SessionId::new();
    let runtime_job_id = JobId::new();
    let first_run_id = RunId::new();
    let first = store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: first_claim.claim_id.clone(),
            product_session_id: session.id.clone(),
            runtime_session_id,
            runtime_job_id,
            runtime_run_id: first_run_id,
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: first_claim.model_config.clone(),
        })
        .await
        .unwrap();
    assert_eq!(first.ordinal, 1);
    store
        .finish_session_turn(
            &first_claim.claim_id,
            crate::product::ProductSessionStatus::Idle,
        )
        .await
        .unwrap();

    let second_claim = store.claim_session_turn(&session.id).await.unwrap();
    let second = store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: second_claim.claim_id.clone(),
            product_session_id: session.id.clone(),
            runtime_session_id,
            runtime_job_id,
            runtime_run_id: RunId::new(),
            resumed_from_run_id: Some(first_run_id),
            followup_control_id: None,
            model_config: second_claim.model_config.clone(),
        })
        .await
        .unwrap();
    assert_eq!(second.ordinal, 2);
    assert_eq!(store.list_run_bindings(&session.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn synchronous_open_recovers_stale_claims() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();

    let reopened = open_store(&temp);
    let context = reopened.get_session_context(&session.id).await.unwrap();
    assert_eq!(
        context.session.status,
        crate::product::ProductSessionStatus::NeedsAttention
    );
    let error = reopened
        .finish_session_turn(&claim.claim_id, crate::product::ProductSessionStatus::Idle)
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductSessionResumeConflict);
}

#[tokio::test]
async fn canonical_workspace_root_cannot_change_kind() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("workspace-kind");
    fs::create_dir_all(&root).unwrap();
    let folder = store
        .create_workspace(CreateProductWorkspaceRequest {
            root: root.clone(),
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Folder workspace".to_string()),
            pinned: false,
        })
        .await
        .unwrap();

    fs::create_dir(root.join(".git")).unwrap();
    let error = store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Repo,
            display_name: Some("Repo workspace".to_string()),
            pinned: true,
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.code,
        ProductErrorCode::ProductSessionWorkspaceMismatch
    );
    let workspaces = store.list_workspaces().await.unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, folder.id);
    assert_eq!(workspaces[0].kind, ProductWorkspaceKind::Folder);
    assert_eq!(workspaces[0].display_name, "Folder workspace");
    assert!(!workspaces[0].pinned);
}

#[tokio::test]
async fn provider_max_steps_is_bounded_at_4096() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let preferences = |max_steps| UpdateProductPreferencesRequest {
        schema_version: 1,
        expected_revision: None,
        theme: ProductThemePreference::System,
        default_approval_policy: None,
        active_workspace_id: None,
        active_session_id: None,
        provider_selection: Some(ProductProviderSelection {
            profile_id: None,
            model: "fake".to_string(),
            approval: ProductApprovalPreference::Ask,
            max_steps,
        }),
    };

    store.update_preferences(preferences(4_096)).await.unwrap();
    let error = store
        .update_preferences(preferences(4_097))
        .await
        .unwrap_err();

    assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);
}

#[tokio::test]
async fn preference_updates_support_legacy_writes_and_revision_cas() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let initial = store.get_preferences().await.unwrap();
    assert_eq!(initial.revision, 0);
    assert_eq!(
        initial.default_approval_policy,
        ProductApprovalPreference::Ask
    );

    let legacy = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::Dark,
            default_approval_policy: None,
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap();
    assert_eq!(legacy.revision, 1);
    assert_eq!(
        legacy.default_approval_policy,
        ProductApprovalPreference::Ask
    );

    let updated = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: Some(legacy.revision),
            theme: ProductThemePreference::Light,
            default_approval_policy: Some(ProductApprovalPreference::Never),
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        updated.default_approval_policy,
        ProductApprovalPreference::Never
    );

    let error = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: Some(legacy.revision),
            theme: ProductThemePreference::System,
            default_approval_policy: Some(ProductApprovalPreference::Auto),
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductRevisionConflict);
    assert_eq!(store.get_preferences().await.unwrap().revision, 2);
}

#[tokio::test]
async fn migration_is_idempotent_and_normalizes_legacy_fake_base() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("migrated-workspace");
    fs::create_dir_all(&root).unwrap();
    let request = M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: "migration-test-1".to_string(),
        workspaces: vec![M1WorkspaceImport {
            source_id: "ws_legacy".to_string(),
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: "Migrated workspace".to_string(),
            pinned: true,
            last_opened_at: "2026-07-26T00:00:00Z".to_string(),
        }],
        sessions: vec![M1SessionImport {
            source_id: "sess_legacy".to_string(),
            source_workspace_id: "ws_legacy".to_string(),
            title: "Migrated session".to_string(),
            created_at: "2026-07-26T00:00:00Z".to_string(),
            updated_at: "2026-07-26T00:00:00Z".to_string(),
            legacy_active_job_id: None,
            legacy_active_run_id: None,
            legacy_resumed_from_run_id: None,
            legacy_has_durable_turn: false,
        }],
        provider_profiles: vec![M1ProviderProfileImport {
            source_id: "provider_legacy".to_string(),
            label: "Fake".to_string(),
            provider_type: ProductProviderType::Fake,
            api_base: "local".to_string(),
            api_key_env: None,
            default_model: Some("fake".to_string()),
            updated_at: "2026-07-26T00:00:00Z".to_string(),
        }],
        safe_preferences: M1SafePreferencesImport {
            theme: Some(ProductThemePreference::System),
            source_active_workspace_id: Some("ws_legacy".to_string()),
            source_active_session_id: Some("sess_legacy".to_string()),
            provider_selection: None,
        },
    };
    let preferences_baseline = preflight_baseline(&store, &request).await;
    let migration = PreparedM1BrowserMigration {
        request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline,
    };
    assert_eq!(raw_migration_preparation_count(&temp), 1);
    let applied = store
        .apply_m1_browser_migration(migration.clone())
        .await
        .unwrap();
    assert_eq!(raw_migration_preparation_count(&temp), 0);
    let preflight_replay = match store
        .preflight_m1_browser_migration(&migration.request)
        .await
        .unwrap()
    {
        M1BrowserMigrationPreflight::Replay(response) => response,
        M1BrowserMigrationPreflight::Prepare(_) => {
            panic!("committed migration receipt was not replayed")
        }
    };
    let replayed = store.apply_m1_browser_migration(migration).await.unwrap();

    assert_eq!(applied.disposition, M1MigrationDisposition::Applied);
    assert_eq!(
        preflight_replay.disposition,
        M1MigrationDisposition::AlreadyApplied
    );
    assert_eq!(replayed.disposition, M1MigrationDisposition::AlreadyApplied);
    assert_eq!(applied.receipt_id, preflight_replay.receipt_id);
    assert_eq!(applied.receipt_id, replayed.receipt_id);
    assert_eq!(applied.workspace_mappings.len(), 1);
    assert_eq!(applied.session_mappings.len(), 1);
    assert_eq!(applied.provider_profile_mappings.len(), 1);
    assert!(applied.issues.is_empty());
    assert_eq!(
        store.list_provider_profiles().await.unwrap()[0].api_base,
        ""
    );
}

#[tokio::test]
async fn migration_rejects_an_active_source_mapped_session_then_retries_after_turn_finishes() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("claimed-migration-workspace");
    fs::create_dir_all(&root).unwrap();
    let initial_request = source_mapped_session_migration_request(
        root.clone(),
        "claimed-session-initial",
        "Initial title",
        "2026-07-26T00:00:00Z",
    );
    let initial_baseline = preflight_baseline(&store, &initial_request).await;
    let initial = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: initial_request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline: initial_baseline,
        })
        .await
        .unwrap();
    let session_id = initial.session_mappings[0].product_session_id.clone();

    let retry_request = source_mapped_session_migration_request(
        root,
        "claimed-session-retry",
        "Updated title",
        "2026-07-26T00:01:00Z",
    );
    let retry_baseline = preflight_baseline(&store, &retry_request).await;
    let retry = PreparedM1BrowserMigration {
        request: retry_request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline: retry_baseline,
    };
    let claim = store.claim_session_turn(&session_id).await.unwrap();

    let error = store
        .apply_m1_browser_migration(retry.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductSessionActive);

    let binding = store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim.claim_id.clone(),
            product_session_id: session_id.clone(),
            runtime_session_id: SessionId::new(),
            runtime_job_id: JobId::new(),
            runtime_run_id: RunId::new(),
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: claim.model_config.clone(),
        })
        .await
        .unwrap();
    assert_eq!(binding.ordinal, 1);
    store
        .finish_session_turn(&claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();

    let applied = store
        .apply_m1_browser_migration(retry.clone())
        .await
        .unwrap();
    let replayed = store.apply_m1_browser_migration(retry).await.unwrap();

    assert_eq!(applied.disposition, M1MigrationDisposition::Applied);
    assert_eq!(replayed.disposition, M1MigrationDisposition::AlreadyApplied);
    assert_eq!(replayed.receipt_id, applied.receipt_id);
    let bindings = store.list_run_bindings(&session_id).await.unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].ordinal, binding.ordinal);
    assert_eq!(bindings[0].runtime_run_id, binding.runtime_run_id);
}

#[tokio::test]
async fn migration_ignores_an_active_unrelated_session() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("unrelated-claim-migration-workspace");
    fs::create_dir_all(&root).unwrap();
    let initial_request = source_mapped_session_migration_request(
        root.clone(),
        "unrelated-claim-initial",
        "Initial title",
        "2026-07-26T00:00:00Z",
    );
    let initial_baseline = preflight_baseline(&store, &initial_request).await;
    store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: initial_request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline: initial_baseline,
        })
        .await
        .unwrap();

    let retry_request = source_mapped_session_migration_request(
        root,
        "unrelated-claim-retry",
        "Updated title",
        "2026-07-26T00:01:00Z",
    );
    let retry_baseline = preflight_baseline(&store, &retry_request).await;
    let (_, unrelated_session) = create_workspace_and_session(&store, &temp).await;
    let unrelated_claim = store
        .claim_session_turn(&unrelated_session.id)
        .await
        .unwrap();

    let applied = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: retry_request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline: retry_baseline,
        })
        .await
        .unwrap();

    assert_eq!(applied.disposition, M1MigrationDisposition::Applied);
    store
        .finish_session_turn(&unrelated_claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();
}

#[tokio::test]
async fn migration_preserves_durable_preferences_for_omitted_browser_fields() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("durable-workspace");
    fs::create_dir_all(&root).unwrap();
    let workspace = store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Durable workspace".to_string()),
            pinned: false,
        })
        .await
        .unwrap();
    let session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: workspace.id.clone(),
            title: Some("Durable session".to_string()),
        })
        .await
        .unwrap();
    let durable = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::Dark,
            default_approval_policy: None,
            active_workspace_id: Some(workspace.id),
            active_session_id: Some(session.id),
            provider_selection: Some(ProductProviderSelection {
                profile_id: None,
                model: "fake".to_string(),
                approval: ProductApprovalPreference::Never,
                max_steps: 12,
            }),
        })
        .await
        .unwrap();
    let request = M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: "migration-omitted-preferences".to_string(),
        workspaces: Vec::new(),
        sessions: Vec::new(),
        provider_profiles: Vec::new(),
        safe_preferences: M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    };
    let preferences_baseline = preflight_baseline(&store, &request).await;
    let migration = PreparedM1BrowserMigration {
        request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline,
    };

    store.apply_m1_browser_migration(migration).await.unwrap();

    let unchanged = store.get_preferences().await.unwrap();
    assert_eq!(
        serde_json::to_value(&unchanged).unwrap(),
        serde_json::to_value(&durable).unwrap()
    );

    let partial_request = M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: "migration-partial-preferences".to_string(),
        workspaces: Vec::new(),
        sessions: Vec::new(),
        provider_profiles: Vec::new(),
        safe_preferences: M1SafePreferencesImport {
            theme: Some(ProductThemePreference::System),
            source_active_workspace_id: Some("missing-workspace".to_string()),
            source_active_session_id: Some("missing-session".to_string()),
            provider_selection: Some(M1ProviderSelectionImport {
                source_profile_id: Some("missing-profile".to_string()),
                model: "fake".to_string(),
                approval: ProductApprovalPreference::Ask,
                max_steps: 8,
            }),
        },
    };
    let preferences_baseline = preflight_baseline(&store, &partial_request).await;
    let partial = PreparedM1BrowserMigration {
        request: partial_request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline,
    };

    let acknowledgement = store.apply_m1_browser_migration(partial).await.unwrap();
    let mut expected = durable;
    expected.revision += 1;
    expected.theme = ProductThemePreference::System;
    assert_eq!(acknowledgement.issues.len(), 3);
    assert_eq!(
        serde_json::to_value(store.get_preferences().await.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
async fn migration_preserves_preferences_saved_after_preflight_and_replays_before_cas() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let request = preference_migration_request(
        "migration-preference-conflict",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Light),
            source_active_workspace_id: Some("missing-workspace".to_string()),
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let preferences_baseline = preflight_baseline(&store, &request).await;
    assert_eq!(preferences_baseline, M1PreferencesBaseline::Revision(0));
    let migration = PreparedM1BrowserMigration {
        request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline,
    };

    // Even an equal-value save is newer user intent than the browser snapshot.
    let durable = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::System,
            default_approval_policy: None,
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap();
    let retry_baseline = preflight_baseline(&store, &migration.request).await;
    assert_eq!(retry_baseline, M1PreferencesBaseline::Revision(0));
    let applied = store
        .apply_m1_browser_migration(migration.clone())
        .await
        .unwrap();

    assert_eq!(applied.disposition, M1MigrationDisposition::Applied);
    assert_eq!(applied.issues.len(), 2);
    assert!(applied.issues.iter().any(|issue| {
        issue.code == M1MigrationIssueCode::InvalidPreferenceReference
            && issue.entity == "active_workspace"
    }));
    assert!(applied.issues.iter().any(|issue| {
        issue.code == M1MigrationIssueCode::PreferenceWriteConflict
            && issue.entity == "preferences"
            && issue.source_id.is_none()
    }));
    assert_eq!(
        serde_json::to_value(store.get_preferences().await.unwrap()).unwrap(),
        serde_json::to_value(&durable).unwrap()
    );

    let newest = store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::Dark,
            default_approval_policy: None,
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap();
    let replayed = store.apply_m1_browser_migration(migration).await.unwrap();

    assert_eq!(replayed.disposition, M1MigrationDisposition::AlreadyApplied);
    assert_eq!(replayed.receipt_id, applied.receipt_id);
    assert_eq!(
        serde_json::to_value(&replayed.issues).unwrap(),
        serde_json::to_value(&applied.issues).unwrap()
    );
    assert_eq!(
        serde_json::to_value(store.get_preferences().await.unwrap()).unwrap(),
        serde_json::to_value(newest).unwrap()
    );
}

#[tokio::test]
async fn migration_preflight_rejects_same_key_with_different_uncommitted_payload() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let first = preference_migration_request(
        "migration-preparation-digest-conflict",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Light),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let different = preference_migration_request(
        "migration-preparation-digest-conflict",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Dark),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );

    assert_eq!(
        preflight_baseline(&store, &first).await,
        M1PreferencesBaseline::Revision(0)
    );
    let error = store
        .preflight_m1_browser_migration(&different)
        .await
        .unwrap_err();

    assert_eq!(error.code, ProductErrorCode::MigrationIdempotencyConflict);
    assert_eq!(raw_migration_preparation_count(&temp), 1);
    assert_eq!(
        preflight_baseline(&store, &first).await,
        M1PreferencesBaseline::Revision(0)
    );
}

#[test]
fn migration_preparation_ttl_removes_the_boundary_and_invalid_timestamps() {
    let temp = TempDir::new().unwrap();
    let _store = open_store(&temp);
    let connection = Connection::open(temp.path().join("product.sqlite")).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 7, 27, 0, 0, 0).unwrap();
    let at_boundary = (now - Duration::hours(24)).to_rfc3339();
    let inside_boundary = (now - Duration::hours(24) + Duration::seconds(1)).to_rfc3339();
    for (key, created_at) in [
        ("ttl-boundary", at_boundary.as_str()),
        ("ttl-inside", inside_boundary.as_str()),
        ("ttl-invalid", "not-a-timestamp"),
    ] {
        connection
            .execute(
                r#"
                INSERT INTO product_migration_preparations(
                    source, source_schema_version, idempotency_key, request_digest,
                    preferences_requested, preferences_revision, created_at
                ) VALUES ('web_m1_local_storage', 1, ?1, ?2, 0, NULL, ?3)
                "#,
                params![key, format!("digest-{key}"), created_at],
            )
            .unwrap();
    }

    let removed = remove_expired_migration_preparations_at(&connection, now).unwrap();
    let remaining: String = connection
        .query_row(
            "SELECT idempotency_key FROM product_migration_preparations",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(removed, 2);
    assert_eq!(remaining, "ttl-inside");
}

#[test]
fn expired_migration_preparations_are_removed_when_store_reopens() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .execute(
            r#"
            INSERT INTO product_migration_preparations(
                source, source_schema_version, idempotency_key, request_digest,
                preferences_requested, preferences_revision, created_at
            ) VALUES ('web_m1_local_storage', 1, 'expired-on-open', 'digest', 0, NULL,
                      '2000-01-01T00:00:00Z')
            "#,
            [],
        )
        .unwrap();
    drop(store);

    let _reopened = open_store(&temp);

    assert_eq!(raw_migration_preparation_count(&temp), 0);
}

#[tokio::test]
async fn expired_migration_key_can_prepare_a_different_payload() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let first = preference_migration_request(
        "expired-preparation-key",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Light),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    preflight_baseline(&store, &first).await;
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .execute(
            "UPDATE product_migration_preparations SET created_at = '2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    let different = preference_migration_request(
        "expired-preparation-key",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Dark),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );

    let baseline = preflight_baseline(&store, &different).await;

    assert_eq!(baseline, M1PreferencesBaseline::Revision(0));
    assert_eq!(raw_migration_preparation_count(&temp), 1);
}

#[tokio::test]
async fn migration_preparations_are_bounded() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let mut connection = Connection::open(temp.path().join("product.sqlite")).unwrap();
    let transaction = connection.transaction().unwrap();
    for ordinal in 0..4_096 {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_preparations(
                    source, source_schema_version, idempotency_key, request_digest,
                    preferences_requested, preferences_revision, created_at
                ) VALUES ('web_m1_local_storage', 1, ?1, ?2, 0, NULL, ?3)
                "#,
                params![
                    format!("bounded-preparation-{ordinal}"),
                    format!("digest-{ordinal}"),
                    now_rfc3339(),
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();

    let request = preference_migration_request(
        "bounded-preparation-overflow",
        M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let error = store
        .preflight_m1_browser_migration(&request)
        .await
        .unwrap_err();

    assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);
    assert_eq!(raw_migration_preparation_count(&temp), 4_096);
}

#[tokio::test]
async fn expired_migration_preparations_do_not_exhaust_the_limit() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let mut connection = Connection::open(temp.path().join("product.sqlite")).unwrap();
    let transaction = connection.transaction().unwrap();
    for ordinal in 0..4_096 {
        transaction
            .execute(
                r#"
                INSERT INTO product_migration_preparations(
                    source, source_schema_version, idempotency_key, request_digest,
                    preferences_requested, preferences_revision, created_at
                ) VALUES ('web_m1_local_storage', 1, ?1, ?2, 0, NULL,
                          '2000-01-01T00:00:00Z')
                "#,
                params![
                    format!("expired-bounded-preparation-{ordinal}"),
                    format!("expired-digest-{ordinal}"),
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    let request = preference_migration_request(
        "preparation-after-expired-capacity",
        M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );

    preflight_baseline(&store, &request).await;

    assert_eq!(raw_migration_preparation_count(&temp), 1);
}

#[tokio::test]
async fn different_migration_keys_share_baseline_but_only_first_writes_preferences() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let first_request = preference_migration_request(
        "migration-preference-first",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Light),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let second_request = preference_migration_request(
        "migration-preference-second",
        M1SafePreferencesImport {
            theme: Some(ProductThemePreference::Dark),
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let first_baseline = preflight_baseline(&store, &first_request).await;
    let second_baseline = preflight_baseline(&store, &second_request).await;
    assert_eq!(first_baseline, M1PreferencesBaseline::Revision(0));
    assert_eq!(second_baseline, first_baseline);

    let first = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: first_request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline: first_baseline,
        })
        .await
        .unwrap();
    let second = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: second_request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline: second_baseline,
        })
        .await
        .unwrap();

    assert!(first.issues.is_empty());
    assert_eq!(second.issues.len(), 1);
    assert_eq!(
        second.issues[0].code,
        M1MigrationIssueCode::PreferenceWriteConflict
    );
    assert_eq!(
        store.get_preferences().await.unwrap().theme,
        ProductThemePreference::Light
    );
}

#[tokio::test]
async fn migration_without_preferences_never_touches_newer_preference_metadata() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let request = preference_migration_request(
        "migration-no-preferences",
        M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    );
    let preferences_baseline = preflight_baseline(&store, &request).await;
    assert_eq!(preferences_baseline, M1PreferencesBaseline::NotRequested);
    let migration = PreparedM1BrowserMigration {
        request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
        preferences_baseline,
    };

    store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::Dark,
            default_approval_policy: None,
            active_workspace_id: None,
            active_session_id: None,
            provider_selection: None,
        })
        .await
        .unwrap();
    let metadata_before = raw_preference_metadata(&temp);
    let applied = store.apply_m1_browser_migration(migration).await.unwrap();
    let metadata_after = raw_preference_metadata(&temp);

    assert!(applied.issues.is_empty());
    assert_eq!(metadata_after, metadata_before);
}

#[tokio::test]
async fn preference_reference_cleanup_paths_increment_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (workspace, session) = create_workspace_and_session(&store, &temp).await;
    let profile = store
        .create_provider_profile(CreateProductProviderProfileRequest {
            label: "Fake".to_string(),
            provider_type: ProductProviderType::Fake,
            api_base: String::new(),
            api_key_env: None,
            default_model: Some("fake".to_string()),
        })
        .await
        .unwrap();
    let selection = ProductProviderSelection {
        profile_id: Some(profile.id.clone()),
        model: "fake".to_string(),
        approval: ProductApprovalPreference::Never,
        max_steps: 12,
    };
    store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::System,
            default_approval_policy: None,
            active_workspace_id: Some(workspace.id.clone()),
            active_session_id: Some(session.id.clone()),
            provider_selection: Some(selection.clone()),
        })
        .await
        .unwrap();
    assert_eq!(raw_preference_metadata(&temp).1, 1);

    store.delete_session(&session.id).await.unwrap();
    assert_eq!(raw_preference_metadata(&temp).1, 2);

    let replacement = store
        .create_session(CreateProductSessionRequest {
            workspace_id: workspace.id.clone(),
            title: Some("Replacement session".to_string()),
        })
        .await
        .unwrap();
    store
        .update_preferences(UpdateProductPreferencesRequest {
            schema_version: 1,
            expected_revision: None,
            theme: ProductThemePreference::System,
            default_approval_policy: None,
            active_workspace_id: Some(workspace.id.clone()),
            active_session_id: Some(replacement.id),
            provider_selection: Some(selection),
        })
        .await
        .unwrap();
    assert_eq!(raw_preference_metadata(&temp).1, 3);

    store.delete_workspace(&workspace.id).await.unwrap();
    assert_eq!(raw_preference_metadata(&temp).1, 4);
    store.delete_provider_profile(&profile.id).await.unwrap();
    assert_eq!(raw_preference_metadata(&temp).1, 5);
}

#[tokio::test]
async fn migrated_session_needing_attention_cannot_claim_a_fresh_turn() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("unbound-workspace");
    fs::create_dir_all(&root).unwrap();
    let request = M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: "migration-unbound-session".to_string(),
        workspaces: vec![M1WorkspaceImport {
            source_id: "workspace-unbound".to_string(),
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: "Unbound workspace".to_string(),
            pinned: false,
            last_opened_at: "2026-07-26T00:00:00Z".to_string(),
        }],
        sessions: vec![M1SessionImport {
            source_id: "session-unbound".to_string(),
            source_workspace_id: "workspace-unbound".to_string(),
            title: "Unbound session".to_string(),
            created_at: "2026-07-26T00:00:00Z".to_string(),
            updated_at: "2026-07-26T00:00:00Z".to_string(),
            legacy_active_job_id: None,
            legacy_active_run_id: None,
            legacy_resumed_from_run_id: None,
            legacy_has_durable_turn: true,
        }],
        provider_profiles: Vec::new(),
        safe_preferences: M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    };
    let preferences_baseline = preflight_baseline(&store, &request).await;
    let acknowledgement = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request,
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
            preferences_baseline,
        })
        .await
        .unwrap();
    let session_id = acknowledgement.session_mappings[0]
        .product_session_id
        .clone();

    let error = store.claim_session_turn(&session_id).await.unwrap_err();

    assert_eq!(
        error.code,
        ProductErrorCode::ProductSessionRuntimeStateMissing
    );
    assert_eq!(
        store
            .get_session_context(&session_id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::NeedsAttention
    );
}

#[tokio::test]
async fn migration_rechecks_verified_workspace_seal_before_apply() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let verified_root = temp.path().join("verified-workspace");
    let changed_root = temp.path().join("changed-workspace");
    fs::create_dir_all(&verified_root).unwrap();
    fs::create_dir_all(&changed_root).unwrap();
    let runtime_session_id = SessionId::new();
    let runtime_job_id = JobId::new();
    let runtime_run_id = RunId::new();
    let request = M1BrowserMigrationRequest {
        source: M1BrowserMigrationSource::WebM1LocalStorage,
        source_schema_version: 1,
        idempotency_key: "migration-workspace-seal".to_string(),
        workspaces: vec![M1WorkspaceImport {
            source_id: "workspace-seal".to_string(),
            root: changed_root,
            kind: ProductWorkspaceKind::Folder,
            display_name: "Changed workspace".to_string(),
            pinned: false,
            last_opened_at: "2026-07-26T00:00:00Z".to_string(),
        }],
        sessions: vec![M1SessionImport {
            source_id: "session-seal".to_string(),
            source_workspace_id: "workspace-seal".to_string(),
            title: "Changed session".to_string(),
            created_at: "2026-07-26T00:00:00Z".to_string(),
            updated_at: "2026-07-26T00:00:00Z".to_string(),
            legacy_active_job_id: Some(runtime_job_id.to_string()),
            legacy_active_run_id: Some(runtime_run_id.to_string()),
            legacy_resumed_from_run_id: None,
            legacy_has_durable_turn: true,
        }],
        provider_profiles: Vec::new(),
        safe_preferences: M1SafePreferencesImport {
            theme: None,
            source_active_workspace_id: None,
            source_active_session_id: None,
            provider_selection: None,
        },
    };
    let preferences_baseline = preflight_baseline(&store, &request).await;

    let error = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request,
            verified_run_bindings: vec![VerifiedM1SessionRunBinding {
                source_session_id: "session-seal".to_string(),
                ordinal: 1,
                runtime_session_id,
                runtime_job_id,
                runtime_run_id,
                resumed_from_run_id: None,
                verified_workspace_root: fs::canonicalize(verified_root).unwrap(),
                verified_workspace_kind: ProductWorkspaceKind::Folder,
            }],
            issues: Vec::new(),
            preferences_baseline,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code, ProductErrorCode::ProductBindingCorrupt);
    assert!(store.list_workspaces().await.unwrap().is_empty());
}

#[tokio::test]
async fn controls_create_list_and_transition() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_root = temp.path().join("ws");
    fs::create_dir_all(&workspace_root).unwrap();
    let ws = store
        .create_workspace(CreateProductWorkspaceRequest {
            root: fs::canonicalize(&workspace_root).unwrap(),
            kind: ProductWorkspaceKind::Folder,
            display_name: None,
            pinned: false,
        })
        .await
        .unwrap();
    let session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: ws.id.clone(),
            title: Some("s1".to_string()),
        })
        .await
        .unwrap();

    let (ctrl, existed) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: " steer-1 ".to_string(),
                idempotency_key: Some("key1".to_string()),
            },
        )
        .await
        .unwrap();
    assert!(!existed);
    assert_eq!(ctrl.status, ProductControlStatus::Pending);
    assert_eq!(ctrl.content, "steer-1");
    assert_eq!(ctrl.seq, 1);

    let (ctrl2, existed2) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: " steer-1 ".to_string(),
                idempotency_key: Some("key1".to_string()),
            },
        )
        .await
        .unwrap();
    assert!(existed2);
    assert_eq!(ctrl.id, ctrl2.id);

    let err = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "different".to_string(),
                idempotency_key: Some("key1".to_string()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ProductErrorCode::ProductControlConflict);

    let all = store.list_controls(&session.id, None).await.unwrap();
    assert_eq!(all.len(), 1);

    let run_id = RunId::new();
    let updated = store
        .transition_control(
            &session.id,
            &ctrl.id,
            ProductControlStatus::Pending,
            ProductControlStatus::Accepted,
            Some(&run_id),
        )
        .await
        .unwrap();
    assert_eq!(updated.status, ProductControlStatus::Accepted);
    assert!(updated.applied_at.is_none());
    assert!(updated.run_id.is_some());

    let applied = store
        .transition_control(
            &session.id,
            &ctrl.id,
            ProductControlStatus::Accepted,
            ProductControlStatus::Applied,
            Some(&run_id),
        )
        .await
        .unwrap();
    assert_eq!(applied.status, ProductControlStatus::Applied);
    assert!(applied.applied_at.is_some());

    let err = store
        .transition_control(
            &session.id,
            &ctrl.id,
            ProductControlStatus::Accepted,
            ProductControlStatus::Accepted,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ProductErrorCode::ProductControlRejected);

    let (fu, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "fu-1".to_string(),
                idempotency_key: None,
            },
        )
        .await
        .unwrap();
    let n = store
        .abandon_pending_controls(&session.id, "cancelled")
        .await
        .unwrap();
    assert_eq!(n, 1);
    let fu2 = store.get_control(&session.id, &fu.id).await.unwrap();
    assert_eq!(fu2.status, ProductControlStatus::Abandoned);
}

#[tokio::test]
async fn unified_messages_are_fifo_idempotent_and_race_through_one_authority() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();

    let (first, replayed) = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: " first ".to_string(),
                idempotency_key: Some("message-1".to_string()),
            },
        )
        .await
        .unwrap();
    assert!(!replayed);
    assert_eq!(first.status, ProductMessageStatus::Queued);
    let (same, replayed) = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: "first".to_string(),
                idempotency_key: Some("message-1".to_string()),
            },
        )
        .await
        .unwrap();
    assert!(replayed);
    assert_eq!(same.id, first.id);
    let (second, _) = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: "second".to_string(),
                idempotency_key: Some("message-2".to_string()),
            },
        )
        .await
        .unwrap();
    assert!(first.seq < second.seq);

    let promoted = store.promote_message(&session.id, &first.id).await.unwrap();
    assert_eq!(promoted.status, ProductMessageStatus::InterventionRequested);
    let claimed = store
        .finish_session_turn_and_claim_followup(&claim.claim_id)
        .await
        .unwrap()
        .expect("second message must retain FIFO successor delivery");
    assert_eq!(claimed.control.id, second.id);
    let promotion_lost = store
        .promote_message(&session.id, &second.id)
        .await
        .unwrap_err();
    assert_eq!(
        promotion_lost.code,
        ProductErrorCode::ProductControlRejected
    );
    let revoked_after_claim = store
        .revoke_message(&session.id, &second.id)
        .await
        .unwrap_err();
    assert_eq!(
        revoked_after_claim.code,
        ProductErrorCode::ProductControlRejected
    );

    let conflict = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: "different".to_string(),
                idempotency_key: Some("message-1".to_string()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ProductErrorCode::ProductControlConflict);
}

#[tokio::test]
async fn unified_message_rejects_legacy_idempotency_and_terminal_revoke() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_workspace, session) = create_workspace_and_session(&store, &temp).await;

    store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "legacy".to_string(),
                idempotency_key: Some("same-key".to_string()),
            },
        )
        .await
        .unwrap();
    let conflict = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: "legacy".to_string(),
                idempotency_key: Some("same-key".to_string()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ProductErrorCode::ProductControlConflict);
    store
        .abandon_pending_controls(&session.id, "legacy compatibility test")
        .await
        .unwrap();

    let (message, _) = store
        .create_message(
            &session.id,
            CreateProductMessageRequest {
                content: "next".to_string(),
                idempotency_key: Some("message-key".to_string()),
            },
        )
        .await
        .unwrap();
    let claimed = store
        .claim_next_followup_turn(&session.id)
        .await
        .unwrap()
        .expect("idle message should be claimed");
    assert_eq!(claimed.control.id, message.id);
    let rejected = store
        .revoke_message(&session.id, &message.id)
        .await
        .unwrap_err();
    assert_eq!(rejected.code, ProductErrorCode::ProductControlRejected);
}

#[tokio::test]
async fn empty_control_rejected() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_root = temp.path().join("ws");
    fs::create_dir_all(&workspace_root).unwrap();
    let ws = store
        .create_workspace(CreateProductWorkspaceRequest {
            root: fs::canonicalize(&workspace_root).unwrap(),
            kind: ProductWorkspaceKind::Folder,
            display_name: None,
            pinned: false,
        })
        .await
        .unwrap();
    let session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: ws.id.clone(),
            title: Some("s".to_string()),
        })
        .await
        .unwrap();
    let err = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "   ".to_string(),
                idempotency_key: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ProductErrorCode::ProductInvalidInput);
}

#[tokio::test]
async fn claim_next_pending_followup_is_atomic() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_root = temp.path().join("ws");
    fs::create_dir_all(&workspace_root).unwrap();
    let ws = store
        .create_workspace(CreateProductWorkspaceRequest {
            root: fs::canonicalize(&workspace_root).unwrap(),
            kind: ProductWorkspaceKind::Folder,
            display_name: None,
            pinned: false,
        })
        .await
        .unwrap();
    let session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: ws.id.clone(),
            title: Some("s".to_string()),
        })
        .await
        .unwrap();

    let (first, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "first".to_string(),
                idempotency_key: Some("fu-a".to_string()),
            },
        )
        .await
        .unwrap();
    let (second, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "second".to_string(),
                idempotency_key: Some("fu-b".to_string()),
            },
        )
        .await
        .unwrap();

    let claimed = store
        .claim_next_pending_followup(&session.id)
        .await
        .unwrap()
        .expect("first follow-up");
    assert_eq!(claimed.id, first.id);
    assert_eq!(claimed.status, ProductControlStatus::Accepted);

    let pending = store.list_pending_followups(&session.id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, second.id);

    let claimed2 = store
        .claim_next_pending_followup(&session.id)
        .await
        .unwrap()
        .expect("second follow-up");
    assert_eq!(claimed2.id, second.id);

    assert!(
        store
            .claim_next_pending_followup(&session.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn final_turn_claims_oldest_followup_without_an_idle_gap() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();

    let (first, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "first queued instruction".to_string(),
                idempotency_key: Some("first-final".to_string()),
            },
        )
        .await
        .unwrap();
    let (second, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "second queued instruction".to_string(),
                idempotency_key: Some("second-final".to_string()),
            },
        )
        .await
        .unwrap();

    let next = store
        .finish_session_turn_and_claim_followup(&claim.claim_id)
        .await
        .unwrap()
        .expect("the oldest follow-up is claimed");
    assert_eq!(next.control.id, first.id);
    assert_eq!(next.control.status, ProductControlStatus::Accepted);
    assert_eq!(
        store
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::Running
    );
    assert_eq!(
        store
            .get_control(&session.id, &second.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Pending
    );
    assert!(
        store
            .claim_next_followup_turn(&session.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn final_turn_drops_unapplied_steers_before_claiming_followup() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();
    let (pending, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "pending steer".to_string(),
                idempotency_key: Some("final-pending-steer".to_string()),
            },
        )
        .await
        .unwrap();
    let (accepted, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "accepted steer".to_string(),
                idempotency_key: Some("final-accepted-steer".to_string()),
            },
        )
        .await
        .unwrap();
    store
        .transition_control(
            &session.id,
            &accepted.id,
            ProductControlStatus::Pending,
            ProductControlStatus::Accepted,
            Some(&RunId::new()),
        )
        .await
        .unwrap();
    let (applied, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "applied steer".to_string(),
                idempotency_key: Some("final-applied-steer".to_string()),
            },
        )
        .await
        .unwrap();
    let applied_run = RunId::new();
    store
        .transition_control(
            &session.id,
            &applied.id,
            ProductControlStatus::Pending,
            ProductControlStatus::Accepted,
            Some(&applied_run),
        )
        .await
        .unwrap();
    store
        .transition_control(
            &session.id,
            &applied.id,
            ProductControlStatus::Accepted,
            ProductControlStatus::Applied,
            Some(&applied_run),
        )
        .await
        .unwrap();
    let (followup, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "successor turn".to_string(),
                idempotency_key: Some("final-followup".to_string()),
            },
        )
        .await
        .unwrap();

    let dropped = store
        .drop_unapplied_steers_for_turn(
            &claim.claim_id,
            RunId::new(),
            "finalized before model turn",
        )
        .await
        .unwrap();
    assert_eq!(
        dropped
            .iter()
            .map(|control| control.id.clone())
            .collect::<Vec<_>>(),
        vec![pending.id.clone(), accepted.id.clone()]
    );
    assert_eq!(
        store
            .get_control(&session.id, &pending.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Dropped
    );
    assert_eq!(
        store
            .get_control(&session.id, &accepted.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Dropped
    );
    let preserved = store.get_control(&session.id, &applied.id).await.unwrap();
    assert_eq!(preserved.status, ProductControlStatus::Applied);
    assert_eq!(preserved.run_id, Some(applied_run));

    let successor = store
        .finish_session_turn_and_claim_followup(&claim.claim_id)
        .await
        .unwrap()
        .expect("queued follow-up starts after old steers are closed");
    assert_eq!(successor.control.id, followup.id);
    assert_eq!(successor.control.status, ProductControlStatus::Accepted);
}

#[tokio::test]
async fn nonfinal_turn_drops_steers_and_abandons_followups_atomically() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();
    let (steer, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "change direction".to_string(),
                idempotency_key: Some("nonfinal-steer".to_string()),
            },
        )
        .await
        .unwrap();
    let (followup, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "continue later".to_string(),
                idempotency_key: Some("nonfinal-followup".to_string()),
            },
        )
        .await
        .unwrap();
    let (accepted_steer, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Steer,
            CreateProductControlRequest {
                content: "safe-point steer".to_string(),
                idempotency_key: Some("nonfinal-accepted-steer".to_string()),
            },
        )
        .await
        .unwrap();
    store
        .transition_control(
            &session.id,
            &accepted_steer.id,
            ProductControlStatus::Pending,
            ProductControlStatus::Accepted,
            Some(&RunId::new()),
        )
        .await
        .unwrap();

    let finished = store
        .finish_session_turn_and_abandon_pending_controls(
            &claim.claim_id,
            Some(RunId::new()),
            ProductSessionStatus::NeedsAttention,
            "run cancelled",
        )
        .await
        .unwrap();
    assert_eq!(finished.dropped_steers.len(), 2);
    assert_eq!(finished.dropped_steers[0].id, steer.id);
    assert_eq!(finished.dropped_steers[1].id, accepted_steer.id);
    assert_eq!(finished.abandoned_followups.len(), 1);
    assert_eq!(finished.abandoned_followups[0].id, followup.id);
    assert_eq!(
        store
            .get_control(&session.id, &steer.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Dropped
    );
    assert_eq!(
        store
            .get_control(&session.id, &accepted_steer.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Dropped
    );
    assert_eq!(
        store
            .get_control(&session.id, &followup.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Abandoned
    );
    assert_eq!(
        store
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::NeedsAttention
    );
}

#[tokio::test]
async fn stale_unreserved_followup_claim_requeues_but_reserved_claim_needs_attention() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let (safe_control, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "safe recovery".to_string(),
                idempotency_key: Some("safe-recovery".to_string()),
            },
        )
        .await
        .unwrap();
    let safe_claim = store
        .claim_next_followup_turn(&session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(safe_claim.control.id, safe_control.id);
    drop(store);
    let store = open_store(&temp);
    assert_eq!(
        store
            .get_control(&session.id, &safe_control.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Pending
    );
    assert_eq!(
        store
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::Idle
    );

    let reserved_claim = store
        .claim_next_followup_turn(&session.id)
        .await
        .unwrap()
        .unwrap();
    store
        .reserve_followup_run(
            &reserved_claim.turn.claim_id,
            &reserved_claim.control.id,
            RunId::new(),
        )
        .await
        .unwrap();
    drop(store);
    let store = open_store(&temp);
    assert_eq!(
        store
            .get_control(&session.id, &safe_control.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Abandoned
    );
    assert_eq!(
        store
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::NeedsAttention
    );
}

#[tokio::test]
async fn restart_after_followup_binding_commit_never_starts_the_reserved_run_twice() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let (control, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "bound before supervisor start".to_string(),
                idempotency_key: Some("bound-crash-window".to_string()),
            },
        )
        .await
        .unwrap();
    let claimed = store
        .claim_next_followup_turn(&session.id)
        .await
        .unwrap()
        .expect("follow-up claim");
    let runtime_session_id = SessionId::new();
    let runtime_job_id = JobId::new();
    let runtime_run_id = RunId::new();
    store
        .reserve_followup_run(&claimed.turn.claim_id, &claimed.control.id, runtime_run_id)
        .await
        .unwrap();
    store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claimed.turn.claim_id.clone(),
            product_session_id: session.id.clone(),
            runtime_session_id,
            runtime_job_id,
            runtime_run_id,
            resumed_from_run_id: None,
            followup_control_id: Some(control.id.clone()),
            model_config: claimed.turn.model_config,
        })
        .await
        .unwrap();

    drop(store);
    let reopened = open_store(&temp);
    let recovered = reopened
        .get_control(&session.id, &control.id)
        .await
        .unwrap();
    assert_eq!(recovered.status, ProductControlStatus::Abandoned);
    assert_eq!(recovered.run_id, Some(runtime_run_id));
    assert_eq!(
        reopened
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::NeedsAttention
    );
    let bindings = reopened.list_run_bindings(&session.id).await.unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].runtime_run_id, runtime_run_id);
    assert!(
        reopened
            .claim_next_followup_turn(&session.id)
            .await
            .unwrap()
            .is_none(),
        "a durably bound follow-up must never be auto-claimed again"
    );

    drop(reopened);
    let reopened_again = open_store(&temp);
    assert_eq!(
        reopened_again
            .get_control(&session.id, &control.id)
            .await
            .unwrap()
            .status,
        ProductControlStatus::Abandoned
    );
    assert_eq!(
        reopened_again
            .list_run_bindings(&session.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn abandoned_followup_requires_explicit_confirmation_before_redrain() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, session) = create_workspace_and_session(&store, &temp).await;
    let claim = store.claim_session_turn(&session.id).await.unwrap();
    let (followup, _) = store
        .create_control(
            &session.id,
            ProductControlKind::Followup,
            CreateProductControlRequest {
                content: "only after confirmation".to_string(),
                idempotency_key: Some("confirm-followup".to_string()),
            },
        )
        .await
        .unwrap();
    store
        .finish_session_turn_and_abandon_pending_controls(
            &claim.claim_id,
            Some(RunId::new()),
            ProductSessionStatus::NeedsAttention,
            "tool effect is uncertain",
        )
        .await
        .unwrap();
    assert!(
        store
            .claim_next_followup_turn(&session.id)
            .await
            .unwrap()
            .is_none()
    );

    let confirmed = store
        .confirm_abandoned_followup(&session.id, &followup.id)
        .await
        .unwrap();
    assert_eq!(confirmed.status, ProductControlStatus::Pending);
    assert_eq!(
        store
            .get_session_context(&session.id)
            .await
            .unwrap()
            .session
            .status,
        ProductSessionStatus::Idle
    );
    let claimed = store
        .claim_next_followup_turn(&session.id)
        .await
        .unwrap()
        .expect("explicitly confirmed follow-up can run");
    assert_eq!(claimed.control.id, followup.id);
}

#[tokio::test]
async fn forks_are_idempotent_independent_and_survive_parent_deletion() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (workspace, parent, boundary) = create_forkable_parent(&store, &temp).await;
    let request = CreateProductForkRequest {
        fork_at_run_id: boundary.source_runtime_run_id,
        title: Some("Investigate alternate path".to_string()),
        idempotency_key: "fork-parent-terminal-1".to_string(),
    };

    let (child, fork, already_exists) = store
        .create_fork(request.clone(), boundary.clone())
        .await
        .unwrap();
    assert!(!already_exists);
    assert_eq!(child.parent_session_id, Some(parent.id.clone()));
    assert_eq!(
        child.fork_point_run_id,
        Some(boundary.source_runtime_run_id)
    );
    assert_eq!(child.fork_point_seq, Some(boundary.fork_at_event_seq));
    assert_eq!(fork.parent_title, parent.title);

    let (replayed_child, replayed_fork, replayed) = store
        .create_fork(request.clone(), boundary.clone())
        .await
        .unwrap();
    assert!(replayed);
    assert_eq!(replayed_child.id, child.id);
    assert_eq!(replayed_fork.id, fork.id);
    let conflict = store
        .create_fork(
            CreateProductForkRequest {
                title: Some("A different body".to_string()),
                ..request.clone()
            },
            boundary.clone(),
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ProductErrorCode::ProductForkConflict);

    let child_context = store.get_session_context(&child.id).await.unwrap();
    let inherited = child_context
        .fork
        .expect("child fork context")
        .inherited_runs;
    assert_eq!(inherited.len(), 1);
    assert_eq!(inherited[0].source_product_session_id, parent.id);
    assert_eq!(
        inherited[0].runtime_session_id,
        boundary.source_runtime_session_id
    );
    assert_eq!(inherited[0].runtime_job_id, boundary.source_runtime_job_id);
    assert_eq!(inherited[0].runtime_run_id, boundary.source_runtime_run_id);
    assert_eq!(
        inherited[0].through_event_seq,
        Some(boundary.fork_at_event_seq)
    );

    let child_claim = store.claim_session_turn(&child.id).await.unwrap();
    let child_binding = store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: child_claim.claim_id.clone(),
            product_session_id: child.id.clone(),
            runtime_session_id: SessionId::new(),
            runtime_job_id: JobId::new(),
            runtime_run_id: RunId::new(),
            resumed_from_run_id: None,
            followup_control_id: None,
            model_config: child_claim.model_config.clone(),
        })
        .await
        .unwrap();
    assert_eq!(child_binding.ordinal, 1);
    assert_ne!(
        child_binding.runtime_session_id,
        boundary.source_runtime_session_id
    );
    assert_ne!(child_binding.runtime_job_id, boundary.source_runtime_job_id);
    store
        .finish_session_turn(&child_claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();

    store.delete_session(&parent.id).await.unwrap();
    let listed = store.list_forks(&parent.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, fork.id);
    let after_delete = store.get_session_context(&child.id).await.unwrap();
    assert_eq!(
        after_delete.session.parent_session_id,
        Some(parent.id.clone())
    );
    assert_eq!(
        after_delete
            .fork
            .expect("child provenance remains after parent removal")
            .inherited_runs[0]
            .runtime_run_id,
        boundary.source_runtime_run_id
    );
    let (after_delete_child, after_delete_fork) = store
        .replay_fork(&parent.id, &request)
        .await
        .unwrap()
        .expect("exact retry remains recoverable after parent removal");
    assert_eq!(after_delete_child.id, child.id);
    assert_eq!(after_delete_fork.id, fork.id);
    assert!(
        store
            .list_sessions(&workspace.id)
            .await
            .unwrap()
            .iter()
            .any(|session| session.id == child.id)
    );
}

#[tokio::test]
async fn forks_reject_a_parent_with_an_active_turn() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let (_, parent, boundary) = create_forkable_parent(&store, &temp).await;
    let active_claim = store.claim_session_turn(&parent.id).await.unwrap();
    let error = store
        .create_fork(
            CreateProductForkRequest {
                fork_at_run_id: boundary.source_runtime_run_id,
                title: None,
                idempotency_key: "fork-active-parent".to_string(),
            },
            boundary,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductSessionActive);
    store
        .finish_session_turn(&active_claim.claim_id, ProductSessionStatus::Idle)
        .await
        .unwrap();
}
