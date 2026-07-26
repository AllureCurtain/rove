use std::fs;

use rove_runtime::types::{JobId, RunId, SessionId};
use tempfile::TempDir;

use crate::product::{
    CommitProductRunBinding, CreateProductSessionRequest, CreateProductWorkspaceRequest,
    M1BrowserMigrationRequest, M1BrowserMigrationSource, M1MigrationDisposition,
    M1ProviderProfileImport, M1ProviderSelectionImport, M1SafePreferencesImport, M1SessionImport,
    M1WorkspaceImport, PreparedM1BrowserMigration, ProductApprovalPreference, ProductErrorCode,
    ProductProviderSelection, ProductProviderType, ProductSessionStatus, ProductStore,
    ProductThemePreference, ProductWorkspaceKind, UpdateProductPreferencesRequest,
    VerifiedM1SessionRunBinding,
};

use super::SqliteProductStore;

fn open_store(temp: &TempDir) -> SqliteProductStore {
    SqliteProductStore::open(temp.path().join("product.sqlite"), 5_000).unwrap()
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
        theme: ProductThemePreference::System,
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
    let migration = PreparedM1BrowserMigration {
        request,
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
    };

    assert!(
        store
            .preflight_m1_browser_migration(&migration.request)
            .await
            .unwrap()
            .is_none()
    );
    let applied = store
        .apply_m1_browser_migration(migration.clone())
        .await
        .unwrap();
    let preflight_replay = store
        .preflight_m1_browser_migration(&migration.request)
        .await
        .unwrap()
        .unwrap();
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
            theme: ProductThemePreference::Dark,
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
    let migration = PreparedM1BrowserMigration {
        request: M1BrowserMigrationRequest {
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
        },
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
    };

    store.apply_m1_browser_migration(migration).await.unwrap();

    let unchanged = store.get_preferences().await.unwrap();
    assert_eq!(
        serde_json::to_value(&unchanged).unwrap(),
        serde_json::to_value(&durable).unwrap()
    );

    let partial = PreparedM1BrowserMigration {
        request: M1BrowserMigrationRequest {
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
        },
        verified_run_bindings: Vec::new(),
        issues: Vec::new(),
    };

    let acknowledgement = store.apply_m1_browser_migration(partial).await.unwrap();
    let mut expected = durable;
    expected.theme = ProductThemePreference::System;
    assert_eq!(acknowledgement.issues.len(), 3);
    assert_eq!(
        serde_json::to_value(store.get_preferences().await.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
async fn migrated_session_needing_attention_cannot_claim_a_fresh_turn() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let root = temp.path().join("unbound-workspace");
    fs::create_dir_all(&root).unwrap();
    let acknowledgement = store
        .apply_m1_browser_migration(PreparedM1BrowserMigration {
            request: M1BrowserMigrationRequest {
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
            },
            verified_run_bindings: Vec::new(),
            issues: Vec::new(),
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
        })
        .await
        .unwrap_err();

    assert_eq!(error.code, ProductErrorCode::ProductBindingCorrupt);
    assert!(store.list_workspaces().await.unwrap().is_empty());
}
