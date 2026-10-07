use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

fn workspace_path_string(rel: impl AsRef<Path>) -> String {
    workspace_path(rel).to_string_lossy().into_owned()
}

use axum::body::Body;
use axum::extract::State as AxumState;
use axum::http::{HeaderMap, Request, StatusCode, header::AUTHORIZATION, header::CONTENT_TYPE};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use rove_api::{
    ApiState, CreateJobResponse, JobStateResponse, MAX_M1_BROWSER_MIGRATION_BODY_BYTES,
    MAX_PRODUCT_EVENT_PAGE, MAX_PRODUCT_EVENTS_RETAINED, MAX_PRODUCT_TEXT_BYTES, ProductSessionId,
    ProductWorkspaceId, WorkspaceActivationState, router, serve_listener,
};
use rove_app_bootstrap::state_migration::{
    ConflictPolicy, DEFAULT_MAX_MIGRATION_BYTES, MigrationOptions, run_state_migration,
};
use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ProjectActivationState, ProjectTrustDecision,
    ProjectTrustRepository, UserConfigPaths, UserStateRoots, capability_digest_map,
    provider_capability_selector_for_workspace,
};
use rove_runtime::events::StreamEvent;
use rove_runtime::execution::StepRecordStatus;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{
    Message, PromptCompactionMode, Role, RunId, RunStatus, SessionId, TaskState, TerminationReason,
    ToolCallRef, ToolMutation, ToolMutationOperation,
};
use rove_runtime::workspace::Workspace;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

fn python_command() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

#[tokio::test]
async fn api_does_not_serve_embedded_web_ui_anymore() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let index = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(index.status(), StatusCode::NOT_FOUND);

    let app_js = app
        .oneshot(
            Request::builder()
                .uri("/web/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(app_js.status(), StatusCode::NOT_FOUND);
}

#[test]
fn product_store_path_uses_the_bootstrap_config_state_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.state.state_dir = PathBuf::from("api-state");

    let state = ApiState::new(workspace, config);

    let product_store_path = state.product_store_path();
    assert_eq!(
        product_store_path.file_name(),
        Some(std::ffi::OsStr::new("product.sqlite"))
    );
    assert_eq!(
        product_store_path.parent().unwrap().parent().unwrap(),
        std::fs::canonicalize(tmp.path()).unwrap()
    );
    assert_eq!(
        product_store_path.parent().unwrap().file_name(),
        Some(std::ffi::OsStr::new("api-state"))
    );
}

#[tokio::test]
async fn product_job_returns_service_unavailable_when_the_store_cannot_open() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.state.sqlite_busy_timeout_ms = 0;
    let app = router(ApiState::new(workspace, config));

    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must not start without product state",
            "product_session_id": ProductSessionId::new()
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_store_unavailable");
}

#[tokio::test]
async fn product_transcript_returns_service_unavailable_when_the_store_cannot_open() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.state.sqlite_busy_timeout_ms = 0;
    let app = router(ApiState::new(workspace, config));
    let product_session_id = ProductSessionId::new();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{product_session_id}/transcript"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_store_unavailable");
}

#[tokio::test]
async fn product_preferences_support_legacy_updates_and_revision_cas() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let initial = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/preferences")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial.status(), StatusCode::OK);
    let initial: serde_json::Value = decode_json(initial).await;
    assert_eq!(initial["revision"], 0);
    assert_eq!(initial["default_approval_policy"], "ask");

    let legacy = request_json(
        &app,
        "PUT",
        "/product/preferences",
        serde_json::json!({
            "schema_version": 1,
            "theme": "dark",
            "active_workspace_id": null,
            "active_session_id": null,
            "provider_selection": null
        }),
    )
    .await;
    assert_eq!(legacy.status(), StatusCode::OK);
    let legacy: serde_json::Value = decode_json(legacy).await;
    assert_eq!(legacy["revision"], 1);
    assert_eq!(legacy["theme"], "dark");
    assert_eq!(legacy["default_approval_policy"], "ask");

    let updated = request_json(
        &app,
        "PUT",
        "/product/preferences",
        serde_json::json!({
            "schema_version": 1,
            "expected_revision": 1,
            "theme": "light",
            "default_approval_policy": "auto",
            "active_workspace_id": null,
            "active_session_id": null,
            "provider_selection": null
        }),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated: serde_json::Value = decode_json(updated).await;
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["default_approval_policy"], "auto");

    let stale = request_json(
        &app,
        "PUT",
        "/product/preferences",
        serde_json::json!({
            "schema_version": 1,
            "expected_revision": 1,
            "theme": "system",
            "default_approval_policy": "never",
            "active_workspace_id": null,
            "active_session_id": null,
            "provider_selection": null
        }),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale: serde_json::Value = decode_json(stale).await;
    assert_eq!(stale["code"], "product_revision_conflict");
}

#[tokio::test]
async fn product_provider_catalog_exposes_revision_and_rejects_stale_crud() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.source_summary.user_config_path = server.path().join("user/config.toml");
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    let listed = get_response(&app, "/product/provider-profiles").await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    let initial_revision = listed["catalog_revision"].as_str().unwrap();
    assert!(initial_revision.starts_with("sha256:"));
    assert_eq!(listed["provider_profiles"], serde_json::json!([]));

    let created = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Local deterministic",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake",
            "expected_revision": initial_revision
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = decode_json(created).await;
    let profile_id = created["id"].as_str().unwrap();
    let created_revision = created["catalog_revision"].as_str().unwrap();
    assert_ne!(created_revision, initial_revision);

    let stale_create = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Stale create",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake",
            "expected_revision": initial_revision
        }),
    )
    .await;
    assert_eq!(stale_create.status(), StatusCode::CONFLICT);
    let stale_create: serde_json::Value = decode_json(stale_create).await;
    assert_eq!(stale_create["code"], "product_revision_conflict");

    let stale_update = request_json(
        &app,
        "PUT",
        &format!("/product/provider-profiles/{profile_id}"),
        serde_json::json!({
            "label": "Stale update",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake",
            "expected_revision": initial_revision
        }),
    )
    .await;
    assert_eq!(stale_update.status(), StatusCode::CONFLICT);
    let stale_update: serde_json::Value = decode_json(stale_update).await;
    assert_eq!(stale_update["code"], "product_revision_conflict");

    let updated = request_json(
        &app,
        "PUT",
        &format!("/product/provider-profiles/{profile_id}"),
        serde_json::json!({
            "label": "Updated local deterministic",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake",
            "expected_revision": created_revision
        }),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated: serde_json::Value = decode_json(updated).await;
    let updated_revision = updated["catalog_revision"].as_str().unwrap();

    let stale_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/provider-profiles/{profile_id}?expected_revision={initial_revision}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_delete.status(), StatusCode::CONFLICT);
    let stale_delete: serde_json::Value = decode_json(stale_delete).await;
    assert_eq!(stale_delete["code"], "product_revision_conflict");

    let deleted = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/provider-profiles/{profile_id}?expected_revision={updated_revision}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn product_default_approval_is_honored_for_product_turns() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Approval policy").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;

    let automatic = request_json(
        &app,
        "PUT",
        "/product/preferences",
        serde_json::json!({
            "schema_version": 1,
            "expected_revision": 0,
            "theme": "system",
            "default_approval_policy": "auto",
            "active_workspace_id": workspace_id,
            "active_session_id": session_id,
            "provider_selection": null
        }),
    )
    .await;
    assert_eq!(automatic.status(), StatusCode::OK);
    let automatic: serde_json::Value = decode_json(automatic).await;

    let default_job = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "write_file",
                "args": {"path": "default-auto.txt", "content": "automatic"}
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(default_job.status(), StatusCode::OK);
    let default_job: CreateJobResponse = decode_json(default_job).await;
    let default_state = wait_for_done(app.clone(), default_job.job_id.to_string()).await;
    assert!(default_state.pending_approvals.is_empty());
    assert_eq!(
        std::fs::read_to_string(folder.path().join("default-auto.txt")).unwrap(),
        "automatic"
    );

    let never = request_json(
        &app,
        "PUT",
        "/product/preferences",
        serde_json::json!({
            "schema_version": 1,
            "expected_revision": automatic["revision"],
            "theme": "system",
            "default_approval_policy": "never",
            "active_workspace_id": workspace_id,
            "active_session_id": session_id,
            "provider_selection": null
        }),
    )
    .await;
    assert_eq!(never.status(), StatusCode::OK);

    let server_policy_job = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "write_file",
                "args": {"path": "explicit-auto.txt", "content": "explicit"}
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(server_policy_job.status(), StatusCode::OK);
    let server_policy_job: CreateJobResponse = decode_json(server_policy_job).await;
    let server_policy_state = wait_for_status(
        app.clone(),
        server_policy_job.job_id.to_string(),
        RunStatus::Error,
    )
    .await;
    assert_eq!(server_policy_state.status, RunStatus::Error);
    assert!(!folder.path().join("explicit-auto.txt").exists());
}

/// Acceptance, end to end: the catalog is a cache.
///
/// The store-level tests cover the recovery transaction; this covers the wiring
/// around it — that a real product turn leaves its ownership record behind, and
/// that constructing a fresh `ApiState` over a deleted catalog puts the session
/// list back without anyone running a repair command.
#[tokio::test]
async fn deleting_the_product_catalog_recovers_the_session_list_on_the_next_start() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let data = tempfile::TempDir::new().unwrap();
    // The contract layout, which is what an unconfigured install uses: one data
    // root holding the global catalog beside a per-workspace runtime directory.
    // Recovery sweeps that layout; see `candidate_runs_dirs` for why a config
    // that scatters run directories under each workspace root cannot be swept.
    let mut config = test_config();
    // Cleared, not set: the contract layout is what an *unconfigured* state path
    // resolves to, and the defaults name the legacy project-local `.rove`.
    config.state.state_dir.clear();
    config.state.sqlite_path.clear();
    config.data_root_override = Some(data.path().to_path_buf());
    config.user_state_roots = Some(UserStateRoots::from_root(data.path()));
    let product_sqlite = config.product_sqlite_path();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config.clone(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(&app, &workspace_id, "Recovered by ownership").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 1).await;

    let job = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "hello",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(job.status(), StatusCode::OK);
    let job: CreateJobResponse = decode_json(job).await;
    let state = wait_for_done(app.clone(), job.job_id.to_string()).await;

    // The record has to be in the run directory, beside the trace it describes.
    let run_dir = find_run_dir(data.path(), &state.run_id.to_string())
        .expect("a product run must materialize under the data root");
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(run_dir.join("product_owner.json"))
            .expect("a bound product run must record who owns it"),
    )
    .unwrap();
    assert_eq!(record["product_session_id"], session_id);
    assert_eq!(record["workspace_id"], workspace_id);
    assert_eq!(record["session_title"], "Recovered by ownership");
    assert_eq!(record["runtime_run_id"], state.run_id.to_string());
    assert_eq!(record["ordinal"], 1);
    assert_eq!(
        record["workspace_root"], workspace["canonical_root"],
        "the recorded root must be the canonical one the catalog stores, since \
         recovery derives the workspace key from it"
    );

    // Lose the catalog, keep the run directories.
    drop(app);
    std::fs::remove_file(&product_sqlite).expect("the catalog must exist to be deleted");

    let recovered = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config.clone(),
    ));
    // Recovery runs off the boot path, so the list is polled rather than assumed
    // ready the instant the state is constructed.
    let mut sessions = serde_json::Value::Null;
    for _ in 0..100 {
        let listed = recovered
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/product/sessions?workspace_id={workspace_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if listed.status() == StatusCode::OK {
            let body: serde_json::Value = decode_json(listed).await;
            if body["sessions"]
                .as_array()
                .is_some_and(|list| !list.is_empty())
            {
                sessions = body;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let listed = sessions["sessions"]
        .as_array()
        .expect("the recovered catalog must list the workspace's sessions");
    assert_eq!(listed.len(), 1, "the session comes back exactly once");
    assert_eq!(
        listed[0]["id"], session_id,
        "a recovered session keeps the id its transcript was written under"
    );
    assert_eq!(listed[0]["title"], "Recovered by ownership");
    assert_eq!(
        listed[0]["status"], "idle",
        "a recovered session must not claim to be running a process that is gone"
    );
    assert_eq!(
        listed[0]["runtime_binding"]["latest_run_id"],
        state.run_id.to_string(),
        "the run the transcript belongs to is the session's latest again"
    );

    // Usable, not just listed: the recovered session must accept the next turn,
    // which is the write that fails if any owner row went missing.
    let next = post_json(
        &recovered,
        "/jobs",
        serde_json::json!({
            "message": "again",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(
        next.status(),
        StatusCode::OK,
        "a recovered session must accept a new turn"
    );
    let next: CreateJobResponse = decode_json(next).await;
    wait_for_done(recovered.clone(), next.job_id.to_string()).await;
}

#[tokio::test]
async fn product_session_model_changes_apply_from_the_next_run_and_keep_snapshot_history() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Session model snapshots").await;
    let session_id = session["id"].as_str().unwrap();

    let initial_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/model-config"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial_response.status(), StatusCode::OK);
    let initial: serde_json::Value = decode_json(initial_response).await;
    assert_eq!(initial["model"], "fake");
    assert_eq!(initial["revision"], 1);

    let configured = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "model": "fake-raw",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);
    let configured: serde_json::Value = decode_json(configured).await;
    assert_eq!(configured["revision"], 2);

    let stale = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "model": "fake",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale: serde_json::Value = decode_json(stale).await;
    assert_eq!(stale["code"], "product_session_model_config_conflict");

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "wait before the model change" }
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let changed_while_running = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "model": "fake",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": configured["revision"]
        }),
    )
    .await;
    assert_eq!(changed_while_running.status(), StatusCode::OK);
    let changed_while_running: serde_json::Value = decode_json(changed_while_running).await;
    assert_eq!(changed_while_running["revision"], 3);

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": "continue with the captured model" }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);
    let first_state = wait_for_done(app.clone(), active.job_id.to_string()).await;
    assert!(first_state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::LlmMessage { full, .. } if full == "continue with the captured model"
        )
    }));

    let second = create_product_job(&app, session_id, "next run uses the new model").await;
    let second_state = wait_for_done(app.clone(), second.job_id.to_string()).await;
    assert!(second_state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::LlmMessage { full, .. } if full == "fake response: next run uses the new model"
        )
    }));

    let snapshots = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/run-models"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshots.status(), StatusCode::OK);
    let snapshots: serde_json::Value = decode_json(snapshots).await;
    let runs = snapshots["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["runtime_run_id"], active.run_id.to_string());
    assert_eq!(runs[0]["model"], "fake-raw");
    assert_eq!(runs[0]["max_steps"], 1);
    assert_eq!(runs[1]["runtime_run_id"], second.run_id.to_string());
    assert_eq!(runs[1]["model"], "fake");
    assert_eq!(runs[1]["max_steps"], 1);
}

#[tokio::test]
async fn product_session_usage_aggregates_report_totals_with_local_zero_cost() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Session usage rollup").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let first = create_product_job(&app, session_id, "usage first").await;
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;
    assert_eq!(first_state.status, RunStatus::Done);
    let second = create_product_job(&app, session_id, "usage second").await;
    let second_state = wait_for_done(app.clone(), second.job_id.to_string()).await;
    assert_eq!(second_state.status, RunStatus::Done);

    let usage = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/usage"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(usage.status(), StatusCode::OK);
    let usage: serde_json::Value = decode_json(usage).await;
    assert_eq!(usage["product_session_id"], session_id);
    let runs = usage["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    // Fake provider may report zero tokens; still must classify local_zero cost
    // from the frozen run pricing snapshot and keep both runs in the rollup.
    assert!(usage["totals"]["total_tokens"].as_u64().is_some());
    assert_eq!(usage["totals_cost"]["availability"], "local_zero");
    assert_eq!(usage["totals_cost"]["total_usd"], 0.0);
    assert_eq!(usage["totals_cost"]["pricing_source"], "bundled");
    assert!(
        usage["totals_cost"]["pricing_version"]
            .as_str()
            .unwrap_or("")
            .starts_with("2026-")
    );
}

#[tokio::test]
async fn product_workspace_files_list_and_content_reject_traversal() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    std::fs::write(folder.path().join("hello.txt"), b"hello product files").unwrap();
    std::fs::create_dir(folder.path().join("src")).unwrap();
    std::fs::write(folder.path().join("src").join("main.rs"), b"fn main() {}").unwrap();
    std::fs::write(folder.path().join(".env"), b"SECRET=1").unwrap();

    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/workspaces/{workspace_id}/files"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    let paths: Vec<&str> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"hello.txt"));
    assert!(paths.contains(&"src"));
    assert!(!paths.iter().any(|path| path.contains(".env")));

    let content = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=hello.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(content.status(), StatusCode::OK);
    let content: serde_json::Value = decode_json(content).await;
    assert_eq!(content["text"], "hello product files");
    assert_eq!(content["encoding"], "utf-8");

    let traversal = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=../hello.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);

    let secret = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=.env"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(secret.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn product_workspace_files_deny_a_revoked_project_trust_root() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    std::fs::write(folder.path().join("hello.txt"), b"hello product files").unwrap();

    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let authority = std::sync::Arc::new(ProjectTrustRepository::new(
        server.path().join("project-trust.sqlite"),
    ));
    let app = router(ApiState::with_project_trust_repository(
        Workspace::detect(server.path()).unwrap(),
        config,
        authority.clone(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let trust_uri = format!("/product/workspaces/{workspace_id}/trust");

    // No durable decision yet: the root is unknown, which still allows the
    // user-registered workspace root to be browsed.
    let before = get_response(&app, &format!("/product/workspaces/{workspace_id}/files")).await;
    assert_eq!(before.status(), StatusCode::OK);

    // An explicit restriction also keeps the exact root readable; trust then
    // only governs the capabilities run creation needs.
    let restricted = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({"decision": "deny", "capabilities": []}),
    )
    .await;
    assert_eq!(restricted.status(), StatusCode::OK);
    let restricted = get_response(&app, &format!("/product/workspaces/{workspace_id}/files")).await;
    assert_eq!(restricted.status(), StatusCode::OK);

    // A revoked root is denied on every bounded read surface: listing,
    // content, download and preview all fail closed with the same typed code
    // run creation uses.
    let revoked = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({"decision": "revoke", "capabilities": []}),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked: serde_json::Value = decode_json(revoked).await;
    assert_eq!(revoked["state"], "revoked");

    for uri in [
        format!("/product/workspaces/{workspace_id}/files"),
        format!("/product/workspaces/{workspace_id}/files/content?path=hello.txt"),
        format!("/product/workspaces/{workspace_id}/files/download?path=hello.txt"),
        format!("/product/workspaces/{workspace_id}/files/preview?path=hello.txt"),
    ] {
        let blocked = get_response(&app, &uri).await;
        assert_eq!(blocked.status(), StatusCode::CONFLICT, "{uri}");
        let blocked: serde_json::Value = decode_json(blocked).await;
        assert_eq!(blocked["code"], "project_trust_required", "{uri}");
    }
}

#[tokio::test]
async fn product_session_artifacts_list_system_files_after_a_completed_run() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Artifacts session").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake", 2).await;

    let created = create_product_job(&app, session_id, "artifacts first").await;
    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let artifacts = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/artifacts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(artifacts.status(), StatusCode::OK);
    let artifacts: serde_json::Value = decode_json(artifacts).await;
    let names: Vec<&str> = artifacts["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["safe_name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"report.json"));
    assert!(names.contains(&"trace.jsonl"));
    assert!(names.contains(&"task_state.json"));

    let diff = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/diff?scope=run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diff.status(), StatusCode::OK);
    let diff: serde_json::Value = decode_json(diff).await;
    assert_eq!(diff["scope"], "run");
    assert!(diff["entries"].as_array().is_some());
}

#[tokio::test]
async fn product_session_evidence_export_is_complete_bounded_and_redacted_in_all_formats() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Evidence export session").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 3).await;

    let secret_canary = "sk-export-content-canary-058761eb";
    let environment_canary = "EXPORT-ENV-CANARY-058761EB-SECRET";
    let authorization_canary = "EXPORT-AUTH-CANARY-058761EB";
    let environment_name = "ROVE_EVIDENCE_EXPORT_TEST_CANARY";
    // This test owns a unique environment name and removes it before return.
    unsafe { std::env::set_var(environment_name, environment_canary) };

    let active = create_product_job(
        &app,
        session_id,
        &serde_json::json!({
            "tool": "request_input",
            "args": {
                "prompt": format!(
                    "keep-normal-evidence; input request {secret_canary}; env={environment_canary}; path={}; Authorization: Bearer {authorization_canary}",
                    folder.path().display()
                )
            }
        })
        .to_string(),
    )
    .await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let steer = post_json(
        &app,
        &format!("/product/sessions/{session_id}/steers"),
        serde_json::json!({
            "content": format!("normal steer with {secret_canary}"),
            "idempotency_key": "evidence-export-steer"
        }),
    )
    .await;
    assert_eq!(steer.status(), StatusCode::CREATED);
    let followup = post_json(
        &app,
        &format!("/product/sessions/{session_id}/followups"),
        serde_json::json!({
            "content": format!("normal follow-up with {environment_canary}"),
            "idempotency_key": "evidence-export-followup"
        }),
    )
    .await;
    assert_eq!(followup.status(), StatusCode::CREATED);

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({
            "answer": format!(
                "normal input result with {secret_canary}, {environment_canary}, and {}",
                folder.path().display()
            )
        }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);
    let completed = wait_for_done(app.clone(), active.job_id.to_string()).await;
    assert_eq!(completed.status, RunStatus::Done);
    wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    let workspace_path = folder.path().to_string_lossy().into_owned();

    for (format, expected_type, extension) in [
        ("json", "application/json", "json"),
        ("html", "text/html", "html"),
        ("markdown", "text/markdown", "md"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/product/sessions/{session_id}/export?format={format}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(expected_type)
        );
        assert!(
            response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .ends_with(&format!("-evidence.{extension}\""))
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        if format == "html" {
            assert!(response.headers().contains_key("content-security-policy"));
        }
        let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        for forbidden in [
            secret_canary,
            environment_canary,
            authorization_canary,
            "Authorization: Bearer",
            workspace_path.as_str(),
        ] {
            assert!(
                !text.contains(forbidden),
                "{format} export leaked {forbidden}"
            );
        }
        assert!(text.contains("keep-normal-evidence"));
        assert!(text.contains("[REDACTED:"));
        assert!(text.contains("artifact_bytes_included"));
        assert!(text.contains("partial_reasons"));
        assert!(text.contains("controls"));
        assert!(text.contains("run_models"));
        assert!(text.contains("usage"));
        assert!(text.contains("artifacts"));
        if format == "html" {
            assert!(!text.contains("<script"));
        }
        if format == "json" {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["export_kind"], "rove.session.evidence");
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["safety"]["artifact_bytes_included"], false);
            assert_eq!(value["safety"]["raw_secrets_included"], false);
            assert!(value["redaction"]["secret_patterns"].as_u64().unwrap() > 0);
            assert!(value["redaction"]["environment_values"].as_u64().unwrap() > 0);
            assert!(value["redaction"]["absolute_paths"].as_u64().unwrap() > 0);
            assert!(
                !value["transcript"]["segments"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(value["controls"].as_array().unwrap().len() >= 2);
            assert!(value["artifacts"]["artifacts"].as_array().unwrap().len() >= 3);
        }
    }

    unsafe { std::env::remove_var(environment_name) };
}

/// A credential the runtime was *told* about must not survive any surface that
/// leaves the process as structured data — on the write side or the read side.
///
/// Covered here: `trace.jsonl` and the mirrored index, `report.json`, the job SSE
/// stream, the product directory SSE stream, all three evidence-export formats,
/// the message/control bodies and the message search snippet, the trace search
/// snippet, the runtime snapshot, and the workspace file preview (which is the
/// same envelope a tool artifact preview uses).
///
/// Deliberately **not** covered, and registered in design §14.3 instead: the
/// byte-transport download routes. They stream a file the requesting principal
/// can already read from disk, and rewriting the bytes would corrupt a binary
/// payload and break the `Content-Length`/`Content-Range` contract. For the same
/// reason the assertion below is about structured surfaces, not "every surface".
///
/// The canary matches none of the patterns the export and the trace search know,
/// so every removal below is evidence of the authority, not of shape. The
/// pattern-shaped canary in the same trace line is the control: it proves the
/// pre-existing backstop still runs and was not replaced.
#[tokio::test]
async fn product_known_credentials_are_redacted_on_every_structured_emitting_surface() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Authority redaction").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 3).await;
    // A file in the workspace whose preview must not hand the credential back.
    std::fs::write(
        folder.path().join("authority-notes.txt"),
        "keep-file-preview-head authority-file-preview-canary-6e2b90 keep-file-preview-tail\n",
    )
    .unwrap();

    // A provider key resolved from a file or keyring looks like this: no prefix
    // any pattern recognises. The declared field name is the other authority:
    // configuration saying "this name carries the credential".
    let canary = "authority-known-credential-canary-3d84af";
    const DECLARED_FIELD: &str = "x_tenant_credential_canary_name";
    let secrets = rove_runtime::secrets::registry();
    assert!(secrets.register_value(canary));
    secrets.register_field_name(DECLARED_FIELD);
    // A second, independent credential: the file preview is a different route
    // through a different helper, so its removal is asserted against its own
    // value rather than inferred from the first one.
    let file_canary = "authority-file-preview-canary-6e2b90";
    assert!(secrets.register_value(file_canary));

    // The pattern-shaped canary other tests already use, so no fixture gains a
    // second secret-shaped string.
    let pattern_canary = "sk-export-content-canary-058761eb";

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": {
                    "prompt": format!(
                        "keep-normal-output carrying {canary} and {pattern_canary}"
                    ),
                    DECLARED_FIELD: canary,
                }
            })
            .to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": format!("keep-normal-answer carrying {canary}") }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);
    let completed = wait_for_done(app.clone(), active.job_id.to_string()).await;
    assert_eq!(completed.status, RunStatus::Done);
    // A run whose final output carries a known credential is still a run whose
    // terminal fact was persisted: redaction must not look like a lost
    // terminal and move the session to `needs_attention`.
    wait_for_product_session_status(&app, workspace_id, &session_id, "idle").await;

    let run_id = active.run_id.to_string();
    let run_dir = find_run_dir(&folder.path().join("api-state"), &run_id)
        .expect("the completed run has a directory");

    // 1. `trace.jsonl`, the audit surface a support request reads.
    let trace = std::fs::read_to_string(run_dir.join("trace.jsonl")).unwrap();
    assert!(
        !trace.contains(canary),
        "trace.jsonl leaked a known credential: {trace}"
    );
    assert!(
        trace.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "trace.jsonl must show the removal: {trace}"
    );
    // The authority pass did not touch the pattern-shaped value: the trace file
    // is redacted by knowledge, and shape stays the read side's business.
    assert!(
        trace.contains(pattern_canary),
        "the trace writer must not become a second pattern matcher: {trace}"
    );
    // The declared name is a name, not a value: it stays readable, and only the
    // value under it was removed.
    assert!(trace.contains(DECLARED_FIELD), "{trace}");
    assert!(trace.contains("keep-normal-output"), "{trace}");

    // 2. `report.json`, the derived summary a user pastes into a bug report.
    let report = std::fs::read_to_string(run_dir.join("report.json")).unwrap();
    assert!(
        !report.contains(canary),
        "report.json leaked a known credential: {report}"
    );

    // 3. The SSE surface, which is built from an in-memory event and would not
    //    be covered by redacting only the durable files.
    let response = get_response(&app, &format!("/jobs/{}/events", active.job_id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let frames = String::from_utf8(body.to_vec()).unwrap();
    assert!(!frames.contains(canary), "an SSE frame leaked: {frames}");
    assert!(
        frames.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "the frames must show the removal: {frames}"
    );
    assert!(frames.contains("keep-normal-output"), "{frames}");

    // 4. The evidence export, in all three formats. `keep-normal-output` in the
    //    output is the control: the transcript really is in there, so absence of
    //    the credential is a removal rather than a missing segment.
    for format in ["json", "html", "markdown"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/product/sessions/{session_id}/export?format={format}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains(canary), "{format} export leaked: {text}");
        assert!(
            text.contains("keep-normal-output"),
            "{format} export lost the surrounding content: {text}"
        );
        // The backstop still removes what shape alone can catch, so the export
        // reads the same before and after this change for pattern-shaped keys.
        assert!(
            !text.contains(pattern_canary),
            "{format} export leaked a pattern-shaped secret"
        );
    }

    // 5. The read side. A queued message is stored verbatim in the ledger, so
    //    its snippet is redacted when it is excerpted, not when it was written.
    let queued = post_json(
        &app,
        &format!("/product/sessions/{session_id}/messages"),
        serde_json::json!({
            "content": format!("keep-searchable-output carrying {canary}"),
            "idempotency_key": "authority-redaction-message"
        }),
    )
    .await;
    assert_eq!(queued.status(), StatusCode::CREATED);

    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("keep-searchable-output")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "the queued message is searchable: {body}");
    for hit in hits {
        let snippet = hit["snippet"].as_str().unwrap();
        assert!(!snippet.contains(canary), "a snippet leaked: {snippet}");
        assert!(
            snippet.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
            "the snippet must show the removal: {snippet}"
        );
    }

    // 5b. The two responses that return the stored body itself: the message page
    //     and the control list. Both are read surfaces, so both redact the body
    //     they answer with while the row keeps what a run has to receive.
    for uri in [
        format!("/product/sessions/{session_id}/messages"),
        format!("/product/sessions/{session_id}/controls"),
    ] {
        let response = get_response(&app, &uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = decode_json(response).await;
        let text = body.to_string();
        assert!(
            !text.contains(canary),
            "{uri} leaked a known credential: {text}"
        );
        assert!(
            text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
            "{uri} must show the removal: {text}"
        );
        assert!(text.contains("keep-searchable-output"), "{uri}: {text}");
    }

    // 6. The same read side over the trace, where the value is still on disk
    //    because the writer redacts by knowledge only. This is the control that
    //    proves the pattern pass still runs before windowing.
    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={}",
            encode_query_value("keep-normal-output"),
            encode_query_value(&format!("trace:{session_id}"))
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "the trace line is searchable: {body}");
    for hit in hits {
        let snippet = hit["snippet"].as_str().unwrap();
        assert!(
            !snippet.contains(canary),
            "a trace snippet leaked: {snippet}"
        );
        assert!(
            !snippet.contains(pattern_canary),
            "the pattern backstop stopped running: {snippet}"
        );
        assert!(
            snippet.contains("[REDACTED:secret_pattern]"),
            "the snippet must show the pattern removal: {snippet}"
        );
    }

    // 7. The runtime snapshot surface, which reports bounded health and must
    //    never carry configuration values.
    let response = get_response(&app, "/product/runtime").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let snapshot = String::from_utf8(body.to_vec()).unwrap();
    assert!(!snapshot.contains(canary), "the runtime snapshot leaked");
    assert!(!snapshot.contains(pattern_canary));

    // 8. The product directory stream. It is a second SSE surface with its own
    //    frame builder, so it is checked rather than assumed to inherit the job
    //    stream's redaction. This stream has no terminal frame — it follows the
    //    directory until the server stops — so the read is bounded by time
    //    instead of by waiting for it to end.
    //
    //    A cursorless subscriber follows from *now*, so the connection alone
    //    carries no frames and an assertion over them would pass on an empty
    //    string. One committed mutation is made while it is open, which is what
    //    puts real frame bytes in front of the assertion below.
    let response = get_response(&app, "/product/events").await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let extra = create_product_workspace(&app, folder.path()).await;
    assert!(extra["id"].is_string());
    let mut frames = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(250), body.next()).await {
            Ok(Some(Ok(chunk))) => frames.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
        // Stop on a *complete* frame rather than on a particular event kind:
        // which fact the mutation publishes is not this test's subject.
        if frames.contains("\n\n") {
            break;
        }
    }
    assert!(
        frames.contains("id:"),
        "the product stream delivered no frame to check: {frames}"
    );
    assert!(
        !frames.contains(canary),
        "a product directory frame leaked: {frames}"
    );

    // 9. The workspace file preview, assembled as JSON for rendering. This is
    //    the *workspace* file route: it reads the caller's window and redacts
    //    what it read, and `size` keeps describing the file, because the caller
    //    can read those bytes from disk anyway. The artifact content route is a
    //    different contract for a different kind of file — it is checked at
    //    surface 13, where the redaction has to cover the whole artifact before
    //    any window is taken.
    let response = get_response(
        &app,
        &format!(
            "/product/workspaces/{workspace_id}/files/content?path={}",
            encode_query_value("authority-notes.txt")
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let text = body["text"].as_str().unwrap();
    assert!(
        !text.contains(file_canary),
        "the file preview leaked: {text}"
    );
    assert!(
        text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "the file preview must show the removal: {text}"
    );
    assert!(
        text.contains("keep-file-preview-head") && text.contains("keep-file-preview-tail"),
        "the file preview lost its surrounding content: {text}"
    );

    // 10. `task_state.json` as a downloadable system artifact. It is the one
    //     durable artifact that is deliberately written *raw* — a resumed run
    //     needs the original values — so the download is its only redaction
    //     boundary. The raw file is read first: absence of the credential in the
    //     response only means something if the file really carries it.
    let manifest = get_response(&app, &format!("/product/sessions/{session_id}/artifacts")).await;
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest: serde_json::Value = decode_json(manifest).await;
    let task_state_id = manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["safe_name"] == "task_state.json")
        .expect("the manifest lists task_state.json")["artifact_id"]
        .as_str()
        .unwrap()
        .to_string();

    let raw_state = std::fs::read_to_string(run_dir.join("task_state.json")).unwrap();
    assert!(
        raw_state.contains(canary),
        "the durable snapshot must stay raw for resume: {raw_state}"
    );

    let download = get_response(
        &app,
        &format!("/product/sessions/{session_id}/artifacts/{task_state_id}/download"),
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    let full = axum::body::to_bytes(download.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let served = String::from_utf8(full.to_vec()).unwrap();
    assert!(
        !served.contains(canary),
        "the task_state download leaked a known credential: {served}"
    );
    assert!(
        served.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "the task_state download must show the removal: {served}"
    );
    assert!(
        served.contains("keep-normal-answer"),
        "the download lost the surrounding content: {served}"
    );
    // Nothing was written back: the file resume reads is still raw.
    assert_eq!(
        std::fs::read_to_string(run_dir.join("task_state.json")).unwrap(),
        raw_state,
        "serving a redacted copy must not rewrite the durable artifact"
    );

    // 11. A range over the *redacted* representation: the length headers have to
    //     describe the bytes actually sent, not the bytes on disk.
    let ranged = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/artifacts/{task_state_id}/download"
                ))
                .header("range", "bytes=0-63")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    let content_range = ranged.headers()["content-range"]
        .to_str()
        .unwrap()
        .to_string();
    let total = served.len();
    assert_eq!(content_range, format!("bytes 0-63/{total}"));
    assert_eq!(
        ranged.headers()["content-length"].to_str().unwrap(),
        "64",
        "the declared length must match the body"
    );
    let ranged_body = axum::body::to_bytes(ranged.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(&ranged_body[..], &served.as_bytes()[..64]);
    assert!(!String::from_utf8_lossy(&ranged_body).contains(canary));

    // 12. The workspace file *download* is still the byte transport it documents
    //     itself as: the fix redacts run state, it does not turn a binary
    //     channel into a rewriter.
    let file_download = get_response(
        &app,
        &format!(
            "/product/workspaces/{workspace_id}/files/download?path={}",
            encode_query_value("authority-notes.txt")
        ),
    )
    .await;
    assert_eq!(file_download.status(), StatusCode::OK);
    let file_bytes = axum::body::to_bytes(file_download.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(file_bytes.to_vec()).unwrap(),
        "keep-file-preview-head authority-file-preview-canary-6e2b90 keep-file-preview-tail\n",
        "the byte transport must stay a byte transport"
    );

    // 13. The artifact *content* route is the second serving path for the same
    //     three run-state artifacts, and it takes a caller-chosen `Range`. The
    //     rewrite has to cover the whole artifact before that window: reading the
    //     window first would return the raw bytes at `bytes=N-N`, walking `N`
    //     would reassemble the raw file, and splitting the credential across two
    //     requests would defeat the value pass. The envelope is therefore in the
    //     redacted representation — `size` is the redacted length, the same thing
    //     the download's `Content-Range` reports.
    let content = get_response(
        &app,
        &format!("/product/sessions/{session_id}/artifacts/{task_state_id}/content"),
    )
    .await;
    assert_eq!(content.status(), StatusCode::OK);
    let envelope: serde_json::Value = decode_json(content).await;
    let whole = envelope["text"]
        .as_str()
        .expect("run state content is text")
        .to_string();
    assert!(
        !whole.contains(canary),
        "the artifact content route leaked: {whole}"
    );
    assert!(whole.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));

    //     A window *inside* the credential is the case a window-then-redact route
    //     cannot survive: eight bytes of a longer credential contain no whole
    //     value for the pass to match, so it hands those bytes out raw. Redacting
    //     the whole file first cannot. (The whole canary does not fit in a window
    //     this size, so the probe is its first eight bytes.)
    let credential_at = raw_state
        .find(canary)
        .expect("the raw file carries the canary");
    let split_start = credential_at.min(whole.len() - 1);
    let split_end = (split_start + 7).min(whole.len() - 1);
    let split = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/artifacts/{task_state_id}/content"
                ))
                .header("range", format!("bytes={split_start}-{split_end}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(split.status(), StatusCode::OK);
    let split: serde_json::Value = decode_json(split).await;
    let split_text = split["text"].as_str().expect("a window is text");
    assert!(
        !split_text.contains(&canary[..8]),
        "a window inside the credential handed out raw credential bytes: {split_text}"
    );
    assert_eq!(
        split_text,
        &whole[split_start..split_end + 1],
        "the window must come from the redacted text"
    );

    //     Every fourth offset, plus the offset the credential occupies in the raw
    //     file and the offset of the marker the rewrite produced. Each window must
    //     be the matching slice of the redacted text — never a raw byte — and the
    //     windows must cover the whole representation between them.
    let mut starts: Vec<usize> = (0..whole.len()).step_by(4).collect();
    starts.push(credential_at.min(whole.len() - 1));
    starts.push(
        whole
            .find(rove_runtime::secrets::KNOWN_SECRET_MARKER)
            .expect("the redacted text carries the marker"),
    );
    let mut covered = vec![false; whole.len()];
    for start in starts {
        let end = (start + 23).min(whole.len() - 1);
        let ranged = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/product/sessions/{session_id}/artifacts/{task_state_id}/content"
                    ))
                    .header("range", format!("bytes={start}-{end}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::OK);
        let window: serde_json::Value = decode_json(ranged).await;
        let text = window["text"].as_str().expect("a window is text");
        assert!(
            !text.contains(canary),
            "content window {start}-{end} leaked the credential: {text}"
        );
        assert_eq!(text, &whole[start..end + 1], "window {start}-{end}");
        for seen in &mut covered[start..=end] {
            *seen = true;
        }
    }
    assert!(
        covered.iter().all(|seen| *seen),
        "the windows covered every byte of the redacted artifact"
    );

    //     The envelope describes that same redacted representation: `size` is its
    //     length (the thing the range indexes, and the total the download's
    //     `Content-Range` reports), not the size of the file on disk.
    assert_eq!(
        envelope["size"],
        serde_json::json!(whole.len()),
        "size must describe the redacted representation the range indexes"
    );
    assert_eq!(envelope["truncated"], serde_json::json!(false));
    assert_eq!(envelope["encoding"], serde_json::json!("utf-8"));

    // 14. The workspace file content route keeps the other order on purpose: it
    //     reads the window and redacts what it read, and `size` still describes
    //     the file. The run-state fix must not change that contract, so the same
    //     window that contains the credential is checked here for the marker and
    //     the file's own length.
    let workspace_window = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path={}",
                    encode_query_value("authority-notes.txt")
                ))
                .header("range", "bytes=23-60")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workspace_window.status(), StatusCode::OK);
    let window: serde_json::Value = decode_json(workspace_window).await;
    let text = window["text"].as_str().expect("the window is text");
    assert!(
        !text.contains(file_canary),
        "the workspace window leaked: {text}"
    );
    assert!(
        text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "the workspace window must show the removal: {text}"
    );
    let file_size = std::fs::metadata(folder.path().join("authority-notes.txt"))
        .unwrap()
        .len();
    assert_eq!(
        window["size"],
        serde_json::json!(file_size),
        "the workspace envelope still describes the file"
    );
    assert_eq!(window["truncated"], serde_json::json!(true));
}

/// A body axum rejects never reaches a handler, so it never reaches
/// `ApiError::into_response` either.
///
/// serde prints the offending value for a type mismatch, which means a caller
/// who mistypes one field would otherwise get their own credential back inside
/// the rejection body — and into whatever proxy, browser capture, or support
/// bundle recorded the exchange. The routes take the rejection themselves and
/// answer with fixed text, so the value cannot come back regardless of whether
/// the runtime was ever told it was a credential.
#[tokio::test]
async fn a_typed_body_rejection_never_echoes_the_value_the_caller_sent() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    // Registered *and* unregistered spellings: the fix is that the rejection
    // text is discarded, not that the value happens to be on a redaction list.
    let canary = "rejected-body-credential-canary-8d41f0";
    assert!(rove_runtime::secrets::registry().register_value(canary));
    let unregistered = "rejected-body-unregistered-canary-2e7a";

    for value in [canary, unregistered] {
        let response = post_json(
            &app,
            "/jobs",
            serde_json::json!({ "message": "hello", "max_steps": value }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{value}");
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !text.contains(value),
            "a typed rejection echoed the caller's value: {text}"
        );
        // The answer is the documented error envelope, not axum's text/plain
        // body: the OpenAPI annotation for this route already promised JSON.
        assert!(
            content_type.starts_with("application/json"),
            "{content_type}"
        );
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["code"], "bad_request");
        assert_eq!(
            body["error"],
            "invalid or unknown field in job request body"
        );
    }

    // A well-formed body is untouched by the change.
    let accepted = post_json(
        &app,
        "/jobs",
        serde_json::json!({ "message": "a valid request", "max_steps": 3 }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    // The same rule on the routes that took `Json<T>` directly, where axum's own
    // rejection body would have echoed the value instead. Each case puts the
    // credential in a *typed* field so the serde error quotes it.
    for (path, body, expected) in [
        (
            "/bench/runs",
            serde_json::json!({ "suite": 5, "profile": canary }),
            "invalid or unknown field in benchmark request body",
        ),
        (
            "/debug/memory/recall",
            serde_json::json!({ "query": "anything", "limit": canary }),
            "invalid or unknown field in recall request body",
        ),
    ] {
        let response = post_json(&app, path, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !text.contains(canary) && !text.contains(unregistered),
            "a typed rejection echoed the caller's value on {path}: {text}"
        );
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["code"], "bad_request", "{path}");
        assert_eq!(body["error"], expected, "{path}");
    }
}

#[tokio::test]
async fn active_product_sessions_reject_archive_session_delete_and_workspace_delete() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Active mutation guard").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;
    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": {"prompt": "keep the turn active"}
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    wait_for_pending_input(app.clone(), active.job_id.to_string()).await;

    let archive = request_json(
        &app,
        "PATCH",
        &format!("/product/sessions/{session_id}"),
        serde_json::json!({"archived": true}),
    )
    .await;
    assert_eq!(archive.status(), StatusCode::CONFLICT);
    let archive: serde_json::Value = decode_json(archive).await;
    assert_eq!(archive["code"], "product_session_active");

    for uri in [
        format!("/product/sessions/{session_id}"),
        format!("/product/workspaces/{workspace_id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_session_active");
    }

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", active.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
}

#[tokio::test]
async fn product_memory_routes_are_workspace_scoped_bounded_and_redacted() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, server.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let memory_dir = server.path().join("platform-memory");

    let empty = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    let empty: serde_json::Value = decode_json(empty).await;
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["topics"], serde_json::json!([]));

    std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Private Source](topics/private-source.md) - project reference memory\n",
    )
    .unwrap();
    std::fs::write(
        memory_dir.join("topics/private-source.md"),
        "---\ntitle: Private Source\ntype: project\nscope: project\nsource: C:/private/source.md\nconfidence: 0.91\ncreated_at: 2026-07-27T00:00:00Z\nupdated_at: 2026-07-27T00:00:00Z\n---\nVisible body\n",
    )
    .unwrap();

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["topics"][0]["slug"], "private-source");
    assert_eq!(listed["topics"][0]["layer"], "durable");
    assert_eq!(listed["topics"][0]["source"], "other");
    assert!(!listed.to_string().contains("C:/private/source.md"));

    let content = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics/private-source?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(content.status(), StatusCode::OK);
    let content: serde_json::Value = decode_json(content).await;
    assert_eq!(content["content"], "Visible body\n");
    assert_eq!(content["topic"]["confidence"], 0.91);
    assert!(!content.to_string().contains("C:/private/source.md"));

    std::fs::write(
        memory_dir.join("topics/private-source.md"),
        format!(
            "---\ntitle: Private Source\ntype: project\nsource: hidden\nconfidence: NaN\n---\n{}",
            "a".repeat(rove_api::MAX_PRODUCT_MEMORY_CONTENT_BYTES + 1)
        ),
    )
    .unwrap();
    let bounded = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics/private-source?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bounded.status(), StatusCode::OK);
    let bounded: serde_json::Value = decode_json(bounded).await;
    assert_eq!(
        bounded["content"].as_str().unwrap().len(),
        rove_api::MAX_PRODUCT_MEMORY_CONTENT_BYTES
    );
    assert_eq!(bounded["topic"]["confidence"], 0.7);
    assert_eq!(bounded["truncated"], true);

    let invalid = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics/bad--slug?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let invalid: serde_json::Value = decode_json(invalid).await;
    assert_eq!(invalid["code"], "product_memory_invalid_slug");

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/memory/topics/private-source?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(!memory_dir.join("topics/private-source.md").exists());

    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Private Source](topics/private-source.md) - stale\n",
    )
    .unwrap();
    let retry = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/memory/topics/private-source?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::NOT_FOUND);
    let retry: serde_json::Value = decode_json(retry).await;
    assert_eq!(retry["code"], "product_memory_not_found");
    assert!(
        !std::fs::read_to_string(memory_dir.join("MEMORY.md"))
            .unwrap()
            .contains("private-source")
    );

    std::fs::write(memory_dir.join("MEMORY.md"), [0xff, 0xfe]).unwrap();
    let corrupt = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(corrupt.status(), StatusCode::CONFLICT);
    let corrupt: serde_json::Value = decode_json(corrupt).await;
    assert_eq!(corrupt["code"], "product_memory_conflict");
}

#[tokio::test]
async fn product_memory_crud_search_filters_and_cas_use_the_real_workspace() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, server.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let topic_url = format!("/product/memory/topics?workspace_id={workspace_id}");

    let create_body = serde_json::json!({
        "slug": "alpha-rules",
        "title": "Alpha Rules",
        "memory_type": "project",
        "scope": "session",
        "confidence": 0.85,
        "description": "Stable alpha conventions",
        "content": "Run focused tests first.\n"
    });
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&topic_url)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created["topic"]["layer"], "durable");
    assert_eq!(created["topic"]["scope"], "session");
    assert_eq!(created["topic"]["source"], "product_settings");
    assert_eq!(created["content"], "Run focused tests first.\n");
    assert_eq!(created["truncated"], false);
    let initial_updated_at = created["topic"]["updated_at"].as_str().unwrap().to_string();

    let stored =
        std::fs::read_to_string(server.path().join("platform-memory/topics/alpha-rules.md"))
            .unwrap();
    assert!(stored.contains("source: product_settings"));
    assert!(!stored.contains(server.path().to_string_lossy().as_ref()));

    let mut duplicate_body = create_body.clone();
    duplicate_body["content"] = serde_json::json!("Do not overwrite me.");
    let duplicate = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&topic_url)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&duplicate_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate: serde_json::Value = decode_json(duplicate).await;
    assert_eq!(duplicate["code"], "product_memory_conflict");

    let second_body = serde_json::json!({
        "slug": "beta-preference",
        "title": "Beta Preference",
        "memory_type": "user",
        "scope": "global",
        "confidence": 0.7,
        "description": "A different durable topic",
        "content": "Prefer concise output.\n"
    });
    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&topic_url)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&second_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);

    let filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}&q=ALPHA&memory_type=project&scope=session&source=product_settings"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered: serde_json::Value = decode_json(filtered).await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["topics"][0]["slug"], "alpha-rules");

    let no_match = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}&source=llm_tool"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_match.status(), StatusCode::OK);
    let no_match: serde_json::Value = decode_json(no_match).await;
    assert_eq!(no_match["total"], 0);

    let invalid_search = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}&q=%0A"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_search.status(), StatusCode::BAD_REQUEST);

    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let update_body = serde_json::json!({
        "title": "Alpha Rules Updated",
        "memory_type": "reference",
        "scope": "project",
        "confidence": 0.95,
        "description": "Updated stable conventions",
        "content": "Run focused tests, then the full gate.\n",
        "expected_updated_at": initial_updated_at
    });
    let updated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/product/memory/topics/alpha-rules?workspace_id={workspace_id}"
                ))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let updated: serde_json::Value = decode_json(updated).await;
    assert_eq!(updated["topic"]["memory_type"], "reference");
    assert_eq!(updated["topic"]["scope"], "project");
    assert_ne!(
        updated["topic"]["updated_at"],
        created["topic"]["updated_at"]
    );
    assert_eq!(
        updated["content"],
        "Run focused tests, then the full gate.\n"
    );

    let stale_body = serde_json::json!({
        "title": "Stale overwrite",
        "memory_type": "user",
        "scope": "global",
        "confidence": 0.1,
        "description": "Must not land",
        "content": "stale",
        "expected_updated_at": created["topic"]["updated_at"]
    });
    let stale = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/product/memory/topics/alpha-rules?workspace_id={workspace_id}"
                ))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&stale_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale: serde_json::Value = decode_json(stale).await;
    assert_eq!(stale["code"], "product_memory_conflict");
    assert!(
        std::fs::read_to_string(server.path().join("platform-memory/topics/alpha-rules.md"))
            .unwrap()
            .contains("Run focused tests, then the full gate.")
    );

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/product/memory/topics/missing?workspace_id={workspace_id}"
                ))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing: serde_json::Value = decode_json(missing).await;
    assert_eq!(missing["code"], "product_memory_not_found");

    let oversized_body = serde_json::json!({
        "slug": "oversized",
        "title": "Oversized",
        "memory_type": "project",
        "scope": "project",
        "confidence": 0.8,
        "description": "Must be rejected",
        "content": "x".repeat(rove_api::MAX_PRODUCT_MEMORY_CONTENT_BYTES + 1)
    });
    let oversized = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&topic_url)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&oversized_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
    assert!(
        !server
            .path()
            .join("platform-memory/topics/oversized.md")
            .exists()
    );
}

#[tokio::test]
async fn product_memory_delete_succeeds_for_an_unindexed_topic_file() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, server.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let memory_dir = server.path().join("platform-memory");
    std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "# rove Memory\n").unwrap();
    let topic_path = memory_dir.join("topics/unindexed.md");
    std::fs::write(&topic_path, "Unindexed selected-workspace topic").unwrap();

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    assert_eq!(listed["topics"], serde_json::json!([]));

    let deleted = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/memory/topics/unindexed?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(!topic_path.exists());
}

#[tokio::test]
async fn product_memory_routes_fail_closed_across_product_workspaces() {
    let server = tempfile::TempDir::new().unwrap();
    let workspace_a_root = server.path().join("workspace-a");
    let workspace_b_root = server.path().join("workspace-b");
    std::fs::create_dir_all(&workspace_a_root).unwrap();
    std::fs::create_dir_all(&workspace_b_root).unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace_a = create_product_workspace(&app, &workspace_a_root).await;
    let workspace_b = create_product_workspace(&app, &workspace_b_root).await;
    let workspace_a_id = workspace_a["id"].as_str().unwrap();
    let workspace_b_id = workspace_b["id"].as_str().unwrap();
    let memory_a = workspace_a_root.join("platform-memory");
    let memory_b = workspace_b_root.join("platform-memory");
    write_product_memory_topic(&memory_a, "only-a", "Only A", "workspace A body");
    write_product_memory_topic(&memory_b, "only-b", "Only B", "workspace B body");

    let missing_query = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/memory/topics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_query.status(), StatusCode::BAD_REQUEST);
    let missing_query: serde_json::Value = decode_json(missing_query).await;
    assert_eq!(missing_query["code"], "product_invalid_input");

    let unknown_workspace_id = ProductWorkspaceId::new();
    let unknown = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={unknown_workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let unknown: serde_json::Value = decode_json(unknown).await;
    assert_eq!(unknown["code"], "product_not_found");

    let listed_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed_a.status(), StatusCode::OK);
    let listed_a: serde_json::Value = decode_json(listed_a).await;
    assert_eq!(listed_a["topics"][0]["slug"], "only-a");

    let listed_b = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_b_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed_b.status(), StatusCode::OK);
    let listed_b: serde_json::Value = decode_json(listed_b).await;
    assert_eq!(listed_b["topics"][0]["slug"], "only-b");

    let mismatched_read = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics/only-b?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mismatched_read.status(), StatusCode::NOT_FOUND);
    let mismatched_read: serde_json::Value = decode_json(mismatched_read).await;
    assert_eq!(mismatched_read["code"], "product_memory_not_found");

    let mismatched_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/memory/topics/only-b?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mismatched_delete.status(), StatusCode::NOT_FOUND);
    let mismatched_delete: serde_json::Value = decode_json(mismatched_delete).await;
    assert_eq!(mismatched_delete["code"], "product_memory_not_found");
    assert!(memory_b.join("topics/only-b.md").exists());
}

#[tokio::test]
async fn product_memory_routes_reject_an_absolute_dir_outside_the_selected_workspace() {
    let server = tempfile::TempDir::new().unwrap();
    let workspace_a_root = server.path().join("workspace-a");
    let workspace_b_root = server.path().join("workspace-b");
    std::fs::create_dir_all(&workspace_a_root).unwrap();
    std::fs::create_dir_all(&workspace_b_root).unwrap();
    let memory_a = workspace_a_root.join("platform-memory");
    write_product_memory_topic(&memory_a, "only-a", "Only A", "workspace A body");

    let mut config = test_config();
    config.rebase_to_workspace(server.path());
    config.memory.durable_dir = memory_a.clone();
    assert_eq!(
        config.workspace_bounded_durable_memory_dir().unwrap(),
        memory_a.canonicalize().unwrap()
    );
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace_a = create_product_workspace(&app, &workspace_a_root).await;
    let workspace_b = create_product_workspace(&app, &workspace_b_root).await;
    let workspace_a_id = workspace_a["id"].as_str().unwrap();
    let workspace_b_id = workspace_b["id"].as_str().unwrap();

    let selected = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(selected.status(), StatusCode::OK);

    for (method, uri) in [
        (
            "GET",
            format!("/product/memory/topics?workspace_id={workspace_b_id}"),
        ),
        (
            "GET",
            format!("/product/memory/topics/only-a?workspace_id={workspace_b_id}"),
        ),
        (
            "DELETE",
            format!("/product/memory/topics/only-a?workspace_id={workspace_b_id}"),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_memory_conflict");
        assert!(!error.to_string().contains(&memory_a.display().to_string()));
    }

    assert!(memory_a.join("topics/only-a.md").exists());
}

#[tokio::test]
async fn product_memory_routes_reject_topic_and_index_symlinks_when_supported() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, server.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let memory_dir = server.path().join("platform-memory");
    std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Linked](topics/linked.md) - project reference memory\n",
    )
    .unwrap();
    let outside_topic = server.path().join("outside-topic.md");
    std::fs::write(&outside_topic, "outside").unwrap();
    let topic_link = memory_dir.join("topics/linked.md");
    if !create_test_file_symlink(&outside_topic, &topic_link) {
        return;
    }

    let linked = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics/linked?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(linked.status(), StatusCode::CONFLICT);
    let linked: serde_json::Value = decode_json(linked).await;
    assert_eq!(linked["code"], "product_memory_conflict");
    assert_eq!(std::fs::read_to_string(&outside_topic).unwrap(), "outside");

    std::fs::remove_file(topic_link).unwrap();
    std::fs::remove_file(memory_dir.join("MEMORY.md")).unwrap();
    let outside_index = server.path().join("outside-index.md");
    std::fs::write(&outside_index, "outside index").unwrap();
    if !create_test_file_symlink(&outside_index, &memory_dir.join("MEMORY.md")) {
        return;
    }
    let linked_index = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/memory/topics?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(linked_index.status(), StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read_to_string(outside_index).unwrap(),
        "outside index"
    );
}

#[tokio::test]
async fn product_runtime_reports_bounded_health_without_paths_or_secrets() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "private-runtime-state".into();
    config.memory.durable_dir = "private-runtime-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    create_product_session(&app, workspace["id"].as_str().unwrap(), "Runtime health").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/product/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let runtime: serde_json::Value = decode_json(response).await;
    assert!(
        runtime["api_version"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(runtime["connection"], "connected");
    assert_eq!(runtime["product_store"], "ready");
    assert_eq!(runtime["execution_environment"]["adapter"], "local");
    assert_eq!(runtime["execution_environment"]["workspace_kind"], "folder");
    assert!(
        runtime["execution_environment"]["workspace_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:") && value.len() == 71)
    );
    for capability in [
        "filesystem_read",
        "filesystem_write",
        "process_run",
        "process_stdio",
        "observations",
        "process_background",
        "workspace_checkpoints",
        "artifact_projection",
    ] {
        assert_eq!(
            runtime["execution_environment"]["capabilities"][capability],
            true
        );
    }
    assert_eq!(
        runtime["execution_environment"]["capabilities"]["process_pty"],
        false
    );
    assert_eq!(runtime["resume_health"]["status"], "healthy");
    assert_eq!(runtime["resume_health"]["workspace_count"], 1);
    assert_eq!(runtime["resume_health"]["session_count"], 1);
    assert_eq!(runtime["resume_health"]["bound_session_count"], 0);
    assert_eq!(runtime["resume_health"]["running_session_count"], 0);
    assert_eq!(runtime["resume_health"]["needs_attention_session_count"], 0);
    assert_eq!(runtime["agent"]["selector"], "builtin:legacy");
    assert_eq!(runtime["agent"]["workspace_source_authorized"], true);
    assert_eq!(runtime["agent"]["workspace_instructions_enabled"], false);
    assert_eq!(runtime["agent"]["allow_remediation_procedures"], false);
    assert_eq!(runtime["agent"]["max_procedure_selections"], 3);
    let keys = runtime.as_object().unwrap();
    assert_eq!(
        keys.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "agent",
            "api_version",
            "connection",
            "execution_environment",
            "product_store",
            "resume_health",
        ])
    );
    assert!(keys.get("path").is_none());
    let serialized = runtime.to_string();
    for forbidden in [
        "private-runtime-state",
        "private-runtime-memory",
        "api_key_env",
        "config",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(!serialized.contains(server.path().to_string_lossy().as_ref()));
    assert!(!serialized.contains(folder.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn api_exposes_openapi_json_for_all_routes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let spec: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert!(
        spec["openapi"]
            .as_str()
            .is_some_and(|version| version.starts_with("3.")),
        "OpenAPI version should be present: {spec:#}"
    );
    assert_eq!(spec["info"]["title"], "rove HTTP API");

    let paths = spec["paths"].as_object().expect("paths object");
    for (path, method) in [
        ("/providers/test", "post"),
        ("/product/workspaces", "get"),
        ("/product/workspaces", "post"),
        ("/product/workspaces/{workspace_id}", "delete"),
        ("/product/workspaces/{workspace_id}/trust", "get"),
        ("/product/workspaces/{workspace_id}/trust", "put"),
        ("/product/sessions", "get"),
        ("/product/sessions", "post"),
        ("/product/sessions/{session_id}", "patch"),
        ("/product/sessions/{session_id}", "delete"),
        ("/product/sessions/{session_id}/transcript", "get"),
        ("/product/sessions/{session_id}/model-config", "get"),
        ("/product/sessions/{session_id}/model-config", "put"),
        ("/product/sessions/{session_id}/run-models", "get"),
        ("/product/sessions/{session_id}/usage", "get"),
        ("/product/sessions/{session_id}/compact", "post"),
        ("/product/sessions/{session_id}/reviews", "post"),
        ("/product/sessions/{session_id}/reviews", "get"),
        ("/product/reviews/{review_id}", "get"),
        ("/product/reviews/{review_id}/findings", "get"),
        ("/product/reviews/{review_id}/cancel", "post"),
        ("/product/workspaces/{workspace_id}/files", "get"),
        ("/product/workspaces/{workspace_id}/files/content", "get"),
        ("/product/workspaces/{workspace_id}/files/download", "get"),
        ("/product/workspaces/{workspace_id}/files/preview", "get"),
        ("/product/sessions/{session_id}/artifacts", "get"),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/content",
            "get",
        ),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/download",
            "get",
        ),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/preview",
            "get",
        ),
        ("/product/sessions/{session_id}/diff", "get"),
        ("/product/sessions/{session_id}/authorizations", "get"),
        ("/product/sessions/{session_id}/attachments", "post"),
        (
            "/product/sessions/{session_id}/attachments/{attachment_id}",
            "get",
        ),
        ("/product/workspaces/{workspace_id}/previews", "post"),
        (
            "/product/workspaces/{workspace_id}/previews/{preview_id}",
            "delete",
        ),
        ("/product/sessions/{session_id}/forks", "post"),
        ("/product/sessions/{session_id}/forks", "get"),
        ("/product/sessions/{session_id}/search", "get"),
        ("/product/search", "get"),
        ("/product/sessions/{session_id}/steers", "post"),
        ("/product/sessions/{session_id}/followups", "post"),
        ("/product/sessions/{session_id}/controls", "get"),
        (
            "/product/sessions/{session_id}/controls/{control_id}/revoke",
            "post",
        ),
        (
            "/product/sessions/{session_id}/controls/{control_id}/confirm",
            "post",
        ),
        ("/product/provider-profiles", "get"),
        ("/product/provider-profiles", "post"),
        ("/product/provider-profiles/{profile_id}", "put"),
        ("/product/provider-profiles/{profile_id}", "delete"),
        ("/product/provider-profiles/{profile_id}/models", "get"),
        ("/product/preferences", "get"),
        ("/product/preferences", "put"),
        ("/product/memory/topics", "get"),
        ("/product/memory/topics", "post"),
        ("/product/memory/topics/{slug}", "get"),
        ("/product/memory/topics/{slug}", "put"),
        ("/product/memory/topics/{slug}", "delete"),
        ("/product/runtime", "get"),
        ("/product/migrations/m1-browser", "post"),
        ("/jobs", "post"),
        ("/jobs/{job_id}/events", "get"),
        ("/jobs/{job_id}/state", "get"),
        ("/jobs/{job_id}/cancel", "post"),
        ("/jobs/{job_id}/approvals/{call_id}", "post"),
        ("/jobs/{job_id}/inputs/{input_id}", "post"),
        ("/runs", "get"),
        ("/runs/{run_id}/report", "get"),
        ("/debug/memory", "get"),
        ("/debug/memory/topics/{slug}", "get"),
        ("/debug/memory/recall", "post"),
    ] {
        let path_item = paths
            .get(path)
            .and_then(|value| value.as_object())
            .unwrap_or_else(|| panic!("missing OpenAPI path {path}"));
        assert!(
            path_item.contains_key(method),
            "missing OpenAPI operation {method} {path}"
        );
    }
    let create_job_responses = spec["paths"]["/jobs"]["post"]["responses"]
        .as_object()
        .expect("POST /jobs responses");
    assert!(create_job_responses.contains_key("503"));
    assert!(!create_job_responses.contains_key("501"));

    // The message-search term is required, and the published operation has to
    // say so: a client that trusted an optional `q` would omit it and receive
    // the typed 400 the schema was supposed to have prevented.
    let search_parameters =
        spec["paths"]["/product/sessions/{session_id}/search"]["get"]["parameters"]
            .as_array()
            .expect("message search parameters");
    let search_term = search_parameters
        .iter()
        .find(|parameter| parameter["name"] == "q" && parameter["in"] == "query")
        .expect("message search `q` query parameter");
    assert_eq!(
        search_term["required"], true,
        "message search must publish its required term as required"
    );

    // The attachment upload publishes its display hint as optional and never
    // required: a client that omitted it would still upload a valid document,
    // and the name is a hint rather than a path.
    let upload_parameters =
        spec["paths"]["/product/sessions/{session_id}/attachments"]["post"]["parameters"]
            .as_array()
            .expect("attachment upload parameters");
    let display_name = upload_parameters
        .iter()
        .find(|parameter| parameter["name"] == "name" && parameter["in"] == "query")
        .expect("attachment `name` query parameter");
    assert!(
        !display_name["required"].as_bool().unwrap_or(false),
        "the attachment display name is an optional hint"
    );
    for path in [
        "/product/sessions/{session_id}/attachments",
        "/product/sessions/{session_id}/attachments/{attachment_id}",
    ] {
        let parameters = spec["paths"][path]
            .as_object()
            .and_then(|item| item.values().next())
            .and_then(|operation| operation["parameters"].as_array())
            .unwrap_or_else(|| panic!("{path} must publish its parameters"));
        let session_parameter = parameters
            .iter()
            .find(|parameter| parameter["name"] == "session_id" && parameter["in"] == "path")
            .unwrap_or_else(|| panic!("{path} must publish `session_id` as a path parameter"));
        assert_eq!(session_parameter["required"], true, "{path}");
    }

    // The unified search publishes two required parameters for the same reason:
    // a client that trusted an optional `scope` would search nothing in
    // particular, and one that trusted an optional `q` would receive a typed 400
    // the schema was supposed to have prevented.
    let unified_parameters = spec["paths"]["/product/search"]["get"]["parameters"]
        .as_array()
        .expect("unified search parameters");
    for name in ["q", "scope"] {
        let parameter = unified_parameters
            .iter()
            .find(|parameter| parameter["name"] == name && parameter["in"] == "query")
            .unwrap_or_else(|| panic!("unified search `{name}` query parameter"));
        assert_eq!(
            parameter["required"], true,
            "unified search must publish `{name}` as required"
        );
    }
    // The corpus a client may name is part of the published contract too.
    let sources = spec["components"]["schemas"]["ProductSearchSource"]["enum"]
        .as_array()
        .expect("ProductSearchSource enum in OpenAPI components");
    assert!(sources.contains(&serde_json::json!("message")));
    assert!(sources.contains(&serde_json::json!("trace")));

    // The artifact source kind is part of the published contract, so a consumer
    // can tell a durable Tool Artifact apart from a registered run file without
    // guessing from the name or MIME type.
    let source_kinds = spec["components"]["schemas"]["ProductArtifactSourceKind"]["enum"]
        .as_array()
        .expect("ProductArtifactSourceKind enum in OpenAPI components");
    for expected in [
        "report",
        "task_state",
        "trace",
        "registered",
        "tool_artifact",
    ] {
        assert!(
            source_kinds
                .iter()
                .any(|kind| kind.as_str() == Some(expected)),
            "missing artifact source kind {expected} in {source_kinds:?}"
        );
    }

    // The transcript cursor and the declared canonical-event contract are part of
    // the published contract: a client can discover paging and the negotiated
    // contract version without reading the server source.
    let transcript_params =
        spec["paths"]["/product/sessions/{session_id}/transcript"]["get"]["parameters"]
            .as_array()
            .expect("GET transcript parameters");
    for expected in ["before_ordinal", "limit_runs", "event_contract"] {
        assert!(
            transcript_params
                .iter()
                .any(|parameter| parameter["name"] == expected && parameter["in"] == "query"),
            "missing transcript query parameter {expected} in {transcript_params:?}"
        );
    }

    for (path, method) in [
        ("/product/workspaces", "get"),
        ("/product/workspaces", "post"),
        ("/product/workspaces/{workspace_id}", "delete"),
        ("/product/workspaces/{workspace_id}/trust", "get"),
        ("/product/workspaces/{workspace_id}/trust", "put"),
        ("/product/sessions", "get"),
        ("/product/sessions", "post"),
        ("/product/sessions/{session_id}", "patch"),
        ("/product/sessions/{session_id}", "delete"),
        ("/product/sessions/{session_id}/transcript", "get"),
        ("/product/sessions/{session_id}/model-config", "get"),
        ("/product/sessions/{session_id}/model-config", "put"),
        ("/product/sessions/{session_id}/run-models", "get"),
        ("/product/sessions/{session_id}/usage", "get"),
        ("/product/sessions/{session_id}/compact", "post"),
        ("/product/sessions/{session_id}/reviews", "post"),
        ("/product/sessions/{session_id}/reviews", "get"),
        ("/product/reviews/{review_id}", "get"),
        ("/product/reviews/{review_id}/findings", "get"),
        ("/product/reviews/{review_id}/cancel", "post"),
        ("/product/workspaces/{workspace_id}/files", "get"),
        ("/product/workspaces/{workspace_id}/files/content", "get"),
        ("/product/workspaces/{workspace_id}/files/download", "get"),
        ("/product/workspaces/{workspace_id}/files/preview", "get"),
        ("/product/sessions/{session_id}/artifacts", "get"),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/content",
            "get",
        ),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/download",
            "get",
        ),
        (
            "/product/sessions/{session_id}/artifacts/{artifact_id}/preview",
            "get",
        ),
        ("/product/sessions/{session_id}/diff", "get"),
        ("/product/sessions/{session_id}/authorizations", "get"),
        ("/product/sessions/{session_id}/attachments", "post"),
        (
            "/product/sessions/{session_id}/attachments/{attachment_id}",
            "get",
        ),
        ("/product/workspaces/{workspace_id}/previews", "post"),
        (
            "/product/workspaces/{workspace_id}/previews/{preview_id}",
            "delete",
        ),
        ("/product/sessions/{session_id}/forks", "post"),
        ("/product/sessions/{session_id}/forks", "get"),
        ("/product/sessions/{session_id}/search", "get"),
        ("/product/search", "get"),
        ("/product/sessions/{session_id}/steers", "post"),
        ("/product/sessions/{session_id}/followups", "post"),
        ("/product/sessions/{session_id}/controls", "get"),
        (
            "/product/sessions/{session_id}/controls/{control_id}/revoke",
            "post",
        ),
        (
            "/product/sessions/{session_id}/controls/{control_id}/confirm",
            "post",
        ),
        ("/product/provider-profiles", "get"),
        ("/product/provider-profiles", "post"),
        ("/product/provider-profiles/{profile_id}", "put"),
        ("/product/provider-profiles/{profile_id}", "delete"),
        ("/product/provider-profiles/{profile_id}/models", "get"),
        ("/product/preferences", "get"),
        ("/product/preferences", "put"),
        ("/product/memory/topics", "get"),
        ("/product/memory/topics", "post"),
        ("/product/memory/topics/{slug}", "get"),
        ("/product/memory/topics/{slug}", "put"),
        ("/product/memory/topics/{slug}", "delete"),
        ("/product/migrations/m1-browser", "post"),
    ] {
        let responses = spec["paths"][path][method]["responses"]
            .as_object()
            .unwrap_or_else(|| panic!("missing OpenAPI responses for {method} {path}"));
        assert!(
            responses.contains_key("500"),
            "{method} {path} must document product operation failures"
        );
        assert!(
            responses.contains_key("503"),
            "{method} {path} must document unavailable product state"
        );
        assert!(
            !responses.contains_key("501"),
            "wired product operation {method} {path} must not advertise 501"
        );
    }

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("components.schemas object");
    for schema in [
        "CreateJobRequest",
        "CreateJobResponse",
        "CreateJobWorkspace",
        "CreateJobWorkspaceKind",
        "JobStateResponse",
        "ListRunsResponse",
        "M1BrowserMigrationRequest",
        "M1BrowserMigrationResponse",
        "CreateProductControlRequest",
        "ProductControl",
        "ProductControlId",
        "ProductControlKind",
        "ProductControlsResponse",
        "ProductControlStatus",
        "ProductControlStatusFilter",
        "ProductAttachmentUploadResponse",
        "ProductAttachmentStatus",
        "CreateProductForkRequest",
        "CreateProductMemoryTopicRequest",
        "ProductFork",
        "ProductForkId",
        "ProductForkResponse",
        "ProductForksResponse",
        "ProductModelDescriptor",
        "ProductMemoryTopic",
        "ProductMemoryTopicContentResponse",
        "ProductMemoryTopicsResponse",
        "ProductMemoryLayer",
        "ProductMemorySource",
        "UpdateProductMemoryTopicRequest",
        "ProductPreferences",
        "ProductProviderProfile",
        "ProductProviderProfilesResponse",
        "ProductProviderModelsResponse",
        "ProductReasoningPreference",
        "ProductRuntimeInfo",
        "ProductSession",
        "ProductSessionModelConfig",
        "ProductSessionRunModelView",
        "ProductSessionRunModelsResponse",
        "ProductContextOccupancy",
        "ProductCostBreakdown",
        "ProductPricingAvailability",
        "ProductRunUsage",
        "ProductSessionUsageResponse",
        "ProductSessionCompaction",
        "ProductUsage",
        "ProductFileContentEnvelope",
        "ProductFileEntry",
        "ProductFileKind",
        "ProductFilesResponse",
        "ProductArtifactSourceKind",
        "ProductArtifactView",
        "ProductArtifactsResponse",
        "ProductDiffEntry",
        "ProductDiffOp",
        "ProductSessionDiffResponse",
        "ProductTranscriptResponse",
        "ProductWorkspace",
        "ProviderProfileRequest",
        "ProviderTestRequest",
        "ProviderTestResponse",
        "RecallTestRequest",
        "RecallTestResponse",
        "SubmitApprovalRequest",
        "SubmitInputRequest",
        "CreateProductProviderProfileRequest",
        "UpdateProductProviderProfileRequest",
        "UpdateProductSessionModelConfigRequest",
        "UpdateProductPreferencesRequest",
    ] {
        assert!(schemas.contains_key(schema), "missing schema {schema}");
    }

    // R6: edit-and-resend is one optional field on the existing fork contract,
    // so a client can address the message it replaces without a second endpoint.
    let fork_request_schema = schemas
        .get("CreateProductForkRequest")
        .cloned()
        .expect("CreateProductForkRequest schema");
    let fork_request_text = fork_request_schema.to_string();
    assert!(
        fork_request_text.contains("truncate_after_message_seq"),
        "the fork request must document the edit target: {fork_request_text}"
    );
    assert!(
        fork_request_text.contains("integer"),
        "the edit target is a message-ledger sequence: {fork_request_text}"
    );
    let required = fork_request_schema["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !required
            .iter()
            .any(|field| field == "truncate_after_message_seq"),
        "the edit target stays optional so a plain fork is unchanged: {fork_request_text}"
    );
    let fork_schema_text = schemas
        .get("ProductFork")
        .cloned()
        .expect("ProductFork schema")
        .to_string();
    assert!(
        fork_schema_text.contains("truncate_after_message_seq")
            && fork_schema_text.contains("truncate_after_message_id"),
        "the recorded fork must expose its cut: {fork_schema_text}"
    );

    let kind_schema = schemas
        .get("CreateJobWorkspaceKind")
        .cloned()
        .expect("CreateJobWorkspaceKind schema");
    let kind_text = kind_schema.to_string();
    assert!(
        kind_text.contains("folder"),
        "OpenAPI should list folder kind: {kind_text}"
    );
    assert!(
        kind_text.contains("repo"),
        "OpenAPI should list repo kind: {kind_text}"
    );
    assert!(
        kind_text.contains("task"),
        "OpenAPI should list task kind: {kind_text}"
    );

    let workspace_schema = schemas
        .get("CreateJobWorkspace")
        .cloned()
        .expect("CreateJobWorkspace schema");
    let workspace_text = workspace_schema.to_string();
    assert!(
        workspace_text.contains("root"),
        "OpenAPI CreateJobWorkspace should document root: {workspace_text}"
    );

    let create_job_schema = schemas
        .get("CreateJobRequest")
        .expect("CreateJobRequest schema");
    assert!(
        create_job_schema["properties"]
            .get("product_session_id")
            .is_some(),
        "CreateJobRequest should expose the additive product session id"
    );
    assert!(
        !create_job_schema["required"]
            .as_array()
            .is_some_and(|required| {
                required
                    .iter()
                    .any(|field| field.as_str() == Some("product_session_id"))
            }),
        "legacy create-job callers must not be required to send product_session_id"
    );

    let product_session_schema = schemas
        .get("ProductSession")
        .expect("ProductSession schema");
    assert!(
        product_session_schema["properties"]
            .get("last_outcome")
            .is_some(),
        "ProductSession should publish the additive last_outcome field"
    );
    assert!(
        product_session_schema["properties"]
            .get("last_outcome_at")
            .is_some(),
        "ProductSession should publish the additive last_outcome_at field"
    );
    assert!(
        !product_session_schema["required"]
            .as_array()
            .is_some_and(|required| {
                required.iter().any(|field| {
                    matches!(
                        field.as_str(),
                        Some("last_outcome") | Some("last_outcome_at")
                    )
                })
            }),
        "a session that never ran must not be required to carry an outcome"
    );

    let product_message_schema = schemas
        .get("ProductMessage")
        .expect("ProductMessage schema");
    assert!(
        product_message_schema["properties"]
            .get("queue_order")
            .is_some(),
        "ProductMessage should publish the additive queue_order position"
    );
    assert!(
        !product_message_schema["required"]
            .as_array()
            .is_some_and(|required| {
                required
                    .iter()
                    .any(|field| field.as_str() == Some("queue_order"))
            }),
        "a message that was never moved must not be required to carry a position"
    );
    for schema in [
        "PromoteProductMessageRequest",
        "ReorderProductMessagesRequest",
        "ProductQueueResponse",
    ] {
        assert!(
            schemas.contains_key(schema),
            "the queue protocol must publish {schema}"
        );
    }
    let reorder_responses =
        spec["paths"]["/product/sessions/{session_id}/messages/reorder"]["post"]["responses"]
            .as_object()
            .expect("reorder responses");
    for status in ["200", "400", "404", "409"] {
        assert!(
            reorder_responses.contains_key(status),
            "reorder must document its {status} outcome"
        );
    }

    let preference_schema = schemas
        .get("ProductPreferences")
        .expect("ProductPreferences schema");
    assert!(preference_schema["properties"].get("revision").is_some());
    assert!(
        preference_schema["properties"]
            .get("default_approval_policy")
            .is_some()
    );
    let update_preference_schema = schemas
        .get("UpdateProductPreferencesRequest")
        .expect("UpdateProductPreferencesRequest schema");
    assert!(
        update_preference_schema["properties"]
            .get("expected_revision")
            .is_some()
    );
    assert!(
        update_preference_schema["properties"]
            .get("default_approval_policy")
            .is_some()
    );
    let provider_list_schema = schemas
        .get("ProductProviderProfilesResponse")
        .expect("ProductProviderProfilesResponse schema");
    assert!(
        provider_list_schema["properties"]
            .get("catalog_revision")
            .is_some()
    );
    let provider_schema = schemas
        .get("ProductProviderProfile")
        .expect("ProductProviderProfile schema");
    assert!(
        provider_schema["properties"]
            .get("catalog_revision")
            .is_some()
    );
    for request_schema in [
        "CreateProductProviderProfileRequest",
        "UpdateProductProviderProfileRequest",
    ] {
        let schema = schemas
            .get(request_schema)
            .expect("Provider request schema");
        let revision_type = &schema["properties"]["expected_revision"]["type"];
        assert!(
            revision_type == "string"
                || revision_type
                    .as_array()
                    .is_some_and(|types| types.iter().any(|value| value == "string")),
            "{request_schema}.expected_revision must accept a string"
        );
    }
    for (path, method) in [
        ("/product/provider-profiles", "post"),
        ("/product/provider-profiles/{profile_id}", "put"),
        ("/product/provider-profiles/{profile_id}", "delete"),
    ] {
        let responses = spec["paths"][path][method]["responses"]
            .as_object()
            .expect("Provider mutation responses");
        assert!(responses.contains_key("409"));
    }
    let memory_responses = spec["paths"]["/product/memory/topics/{slug}"]["get"]["responses"]
        .as_object()
        .expect("product memory GET responses");
    assert!(memory_responses.contains_key("400"));
    assert!(memory_responses.contains_key("404"));
    assert!(memory_responses.contains_key("409"));
    for (path, method) in [
        ("/product/memory/topics", "get"),
        ("/product/memory/topics/{slug}", "get"),
        ("/product/memory/topics/{slug}", "delete"),
    ] {
        let parameters = spec["paths"][path][method]["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("missing OpenAPI parameters for {method} {path}"));
        let workspace_id = parameters
            .iter()
            .find(|parameter| parameter["name"] == "workspace_id" && parameter["in"] == "query")
            .unwrap_or_else(|| panic!("missing workspace_id query for {method} {path}"));
        assert_eq!(workspace_id["required"], true);
        let responses = spec["paths"][path][method]["responses"]
            .as_object()
            .expect("product memory responses");
        assert!(responses.contains_key("404"));
        assert!(responses.contains_key("503"));
    }
    let runtime_schema = schemas
        .get("ProductRuntimeInfo")
        .expect("ProductRuntimeInfo schema")
        .to_string();
    assert!(!runtime_schema.contains("path"));
    assert!(!runtime_schema.contains("config"));

    assert!(
        spec.pointer("/components/securitySchemes/BearerAuth")
            .is_some(),
        "missing BearerAuth security scheme"
    );
    assert!(text.contains("api_key_env"));
    assert!(text.contains("key_present"));
    assert!(!text.contains("dummy-provider-token"));
    assert!(!text.contains("\"api_key\""));
}

/// The Web types are written by hand against `/api/openapi.json`, so the
/// repository keeps a committed copy of the document that this test compares
/// against the served one. Regenerate the copy after changing a route, a
/// request type, or a schema:
///
/// ```text
/// ROVE_UPDATE_OPENAPI=1 cargo test -p rove-integration-tests --test api openapi_snapshot
/// ```
#[tokio::test]
async fn openapi_snapshot_matches_the_served_document() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value =
        serde_json::from_slice(&body).expect("the served OpenAPI document is JSON");
    let mut rendered =
        serde_json::to_string_pretty(&spec).expect("the OpenAPI document serializes");
    rendered.push('\n');

    let path = workspace_path("apps/api/openapi.json");
    if std::env::var_os("ROVE_UPDATE_OPENAPI").is_some() {
        std::fs::write(&path, &rendered).expect("the OpenAPI snapshot is writable");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "the OpenAPI snapshot {} is missing or unreadable ({err}); regenerate it with \
             ROVE_UPDATE_OPENAPI=1",
            path.display()
        )
    });
    let committed_lines: Vec<&str> = committed.lines().collect();
    let rendered_lines: Vec<&str> = rendered.lines().collect();
    let first_mismatch = (0..committed_lines.len().max(rendered_lines.len()))
        .find(|index| committed_lines.get(*index) != rendered_lines.get(*index));
    if let Some(index) = first_mismatch {
        // A single schema line can run to hundreds of characters, so the
        // failure reports the first divergent line instead of both documents.
        let shorten = |line: Option<&&str>| match line {
            Some(line) if line.chars().count() > 160 => {
                format!("{}…", line.chars().take(160).collect::<String>())
            }
            Some(line) => (*line).to_string(),
            None => "<end of document>".to_string(),
        };
        panic!(
            "the OpenAPI snapshot {} is stale: line {} differs\n  committed: {}\n  served:    {}\n\
             regenerate it with ROVE_UPDATE_OPENAPI=1 cargo test -p rove-integration-tests \
             --test api openapi_snapshot",
            path.display(),
            index + 1,
            shorten(committed_lines.get(index)),
            shorten(rendered_lines.get(index)),
        );
    }
}

#[tokio::test]
async fn api_exposes_swagger_ui() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/swagger-ui")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert!(
        status.is_success() || status.is_redirection(),
        "Swagger UI should be reachable, got {}",
        status
    );

    let response = if status.is_redirection() {
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect should include Location header")
            .to_string();
        let follow_uri = if location.starts_with("http://") || location.starts_with("https://") {
            let uri: axum::http::Uri = location.parse().expect("redirect Location should be a URI");
            uri.path_and_query()
                .map(|path| path.as_str().to_string())
                .unwrap_or_else(|| "/".to_string())
        } else if location.starts_with('/') {
            location
        } else {
            format!("/{location}")
        };

        app.clone()
            .oneshot(
                Request::builder()
                    .uri(follow_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    } else {
        response
    };

    assert!(
        response.status().is_success(),
        "Swagger UI final response should be successful, got {}",
        response.status()
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        text.contains("Swagger UI") || text.contains("swagger-ui"),
        "Swagger UI response should include Swagger UI content: {text}"
    );

    let initializer = app
        .oneshot(
            Request::builder()
                .uri("/swagger-ui/swagger-initializer.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        initializer.status().is_success(),
        "Swagger UI initializer should be reachable, got {}",
        initializer.status()
    );
    let body = axum::body::to_bytes(initializer.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        text.contains("/api/openapi.json"),
        "Swagger UI initializer should reference the OpenAPI spec: {text}"
    );
}

#[tokio::test]
async fn product_migration_rejects_unknown_secret_fields_before_store_access() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let payload = serde_json::json!({
        "source": "web_m1_local_storage",
        "source_schema_version": 1,
        "idempotency_key": "migration-secret-rejection",
        "workspaces": [],
        "sessions": [],
        "provider_profiles": [{
            "source_id": "prov_legacy",
            "label": "unsafe",
            "provider_type": "openai",
            "api_base": "https://api.openai.com/v1",
            "api_key_env": "OPENAI_API_KEY",
            "api_key": "must-not-cross-the-boundary",
            "updated_at": "2026-07-26T00:00:00Z"
        }],
        "safe_preferences": { "theme": "system" }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/migrations/m1-browser")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "product_invalid_input");
    assert!(!String::from_utf8_lossy(&body).contains("must-not-cross-the-boundary"));
}

#[tokio::test]
async fn product_migration_accepts_a_legal_body_larger_than_axum_default() {
    const SESSION_COUNT: usize = 1_500;

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace.clone(), test_config()));
    let workspace_source_id = format!(
        "workspace-{}",
        "w".repeat(MAX_PRODUCT_TEXT_BYTES - "workspace-".len())
    );
    let title = "t".repeat(MAX_PRODUCT_TEXT_BYTES);
    let sessions = (0..SESSION_COUNT)
        .map(|index| {
            let prefix = format!("session-{index:04}-");
            let source_id = format!(
                "{prefix}{}",
                "s".repeat(MAX_PRODUCT_TEXT_BYTES - prefix.len())
            );
            serde_json::json!({
                "source_id": source_id,
                "source_workspace_id": workspace_source_id,
                "title": title,
                "created_at": "2026-07-26T00:00:00Z",
                "updated_at": "2026-07-26T00:00:00Z"
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&serde_json::json!({
        "source": "web_m1_local_storage",
        "source_schema_version": 1,
        "idempotency_key": "migration-over-default-body-limit",
        "workspaces": [{
            "source_id": workspace_source_id,
            "root": workspace.root,
            "kind": "folder",
            "display_name": "Large legal migration",
            "pinned": false,
            "last_opened_at": "2026-07-26T00:00:00Z"
        }],
        "sessions": sessions,
        "provider_profiles": [],
        "safe_preferences": {}
    }))
    .unwrap();
    assert!(payload.len() > 2 * 1_048_576);
    assert!(payload.len() < MAX_M1_BROWSER_MIGRATION_BODY_BYTES);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/migrations/m1-browser")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(receipt["disposition"], "applied");
    assert_eq!(
        receipt["session_mappings"].as_array().unwrap().len(),
        SESSION_COUNT
    );
}

#[tokio::test]
async fn product_migration_rejects_a_body_above_its_route_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let payload = vec![b' '; MAX_M1_BROWSER_MIGRATION_BODY_BYTES + 1];

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/migrations/m1-browser")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "product_invalid_input");
}

#[tokio::test]
async fn product_migration_replays_receipt_before_runtime_artifact_inspection() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace.clone(), test_config()));
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"message":"migration singleton","model":"fake"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        wait_for_done(app.clone(), created.job_id.to_string())
            .await
            .status,
        RunStatus::Done
    );

    let payload = serde_json::json!({
        "source": "web_m1_local_storage",
        "source_schema_version": 1,
        "idempotency_key": "migration-receipt-before-artifacts",
        "workspaces": [{
            "source_id": "legacy-workspace",
            "root": workspace.root,
            "kind": "folder",
            "display_name": "Legacy workspace",
            "pinned": false,
            "last_opened_at": "2026-07-26T00:00:00Z"
        }],
        "sessions": [{
            "source_id": "legacy-session",
            "source_workspace_id": "legacy-workspace",
            "title": "Legacy session",
            "created_at": "2026-07-26T00:00:00Z",
            "updated_at": "2026-07-26T00:00:00Z",
            "legacy_active_job_id": created.job_id,
            "legacy_active_run_id": created.run_id,
            "legacy_has_durable_turn": true
        }],
        "provider_profiles": [],
        "safe_preferences": {}
    });
    let migrate = |app: Router, payload: serde_json::Value| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/product/migrations/m1-browser")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()
    };

    let applied = migrate(app.clone(), payload.clone()).await;
    assert_eq!(applied["disposition"], "applied");
    assert_eq!(applied["issues"], serde_json::json!([]));
    std::fs::remove_file(
        workspace
            .state_dir
            .join("runs")
            .join(created.run_id.to_string())
            .join("task_state.json"),
    )
    .unwrap();

    let replayed = migrate(app, payload).await;
    assert_eq!(replayed["disposition"], "already_applied");
    assert_eq!(replayed["receipt_id"], applied["receipt_id"]);
    assert_eq!(replayed["issues"], applied["issues"]);
}

#[tokio::test]
async fn product_sessions_in_one_workspace_resume_their_own_exact_runs() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session_a = create_product_session(&app, workspace_id, "Session A").await;
    let session_b = create_product_session(&app, workspace_id, "Session B").await;
    let session_a_id = session_a["id"].as_str().unwrap();
    let session_b_id = session_b["id"].as_str().unwrap();

    let first_a = create_product_job(&app, session_a_id, "first A").await;
    let first_a_state = wait_for_done(app.clone(), first_a.job_id.to_string()).await;
    assert_eq!(first_a_state.status, RunStatus::Done);
    assert_product_runtime_terminal_durable(folder.path(), &first_a, &first_a_state).await;
    let first_b = create_product_job(&app, session_b_id, "first B").await;
    let first_b_state = wait_for_done(app.clone(), first_b.job_id.to_string()).await;
    assert_eq!(first_b_state.status, RunStatus::Done);
    assert_product_runtime_terminal_durable(folder.path(), &first_b, &first_b_state).await;
    let second_a = create_product_job(&app, session_a_id, "second A").await;

    assert_eq!(second_a.job_id, first_a.job_id);
    assert_ne!(second_a.job_id, first_b.job_id);
    assert_eq!(second_a.resumed_from_run_id, Some(first_a.run_id));
    assert_ne!(second_a.resumed_from_run_id, Some(first_b.run_id));
    let second_a_state = wait_for_done(app.clone(), second_a.job_id.to_string()).await;
    assert_eq!(second_a_state.status, RunStatus::Done);
    assert!(
        second_a_state.events.iter().any(|event| matches!(
            &event.event,
            StreamEvent::LlmMessage { full, .. } if full == "fake response: second A"
        )),
        "a product follow-up must execute the new user message"
    );
    assert_product_runtime_terminal_durable(folder.path(), &second_a, &second_a_state).await;

    let transcript = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_a_id}/transcript"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transcript.status(), StatusCode::OK);
    let transcript: serde_json::Value = decode_json(transcript).await;
    assert_eq!(transcript["product_session_id"], session_a_id);
    assert_eq!(transcript["workspace_id"], workspace_id);
    assert_eq!(transcript["status"], "complete", "{transcript}");
    let segments = transcript["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0]["binding"]["ordinal"], 1);
    assert_eq!(
        segments[0]["binding"]["runtime_run_id"],
        first_a.run_id.to_string()
    );
    assert_eq!(segments[1]["binding"]["ordinal"], 2);
    assert_eq!(
        segments[1]["binding"]["runtime_run_id"],
        second_a.run_id.to_string()
    );

    let sessions = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions?workspace_id={workspace_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sessions.status(), StatusCode::OK);
    let sessions: serde_json::Value = decode_json(sessions).await;
    let session_a = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == session_a_id)
        .unwrap();
    assert_eq!(session_a["status"], "idle");
    assert_eq!(
        session_a["runtime_binding"]["latest_run_id"],
        second_a.run_id.to_string()
    );
}

#[tokio::test]
async fn product_session_fork_replays_exactly_and_keeps_child_history_independent() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let parent = create_product_session(&app, &workspace_id, "Fork parent").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();
    let source = create_product_job(&app, &parent_id, "Durable fork source").await;
    let source_state = wait_for_done(app.clone(), source.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &source, &source_state).await;

    let fork_request = serde_json::json!({
        "fork_at_run_id": source.run_id,
        "idempotency_key": "fork-parent-terminal-api-1"
    });
    let created = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request.clone(),
    )
    .await;
    let created_status = created.status();
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
    let child_id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["session"]["parent_session_id"], parent_id);
    assert_eq!(
        created["session"]["fork_point_run_id"],
        source.run_id.to_string()
    );
    assert_eq!(
        created["fork"]["source_runtime_run_id"],
        source.run_id.to_string()
    );
    assert_eq!(
        created["fork"]["fork_at_event_seq"],
        source_state.event_count
    );

    let replayed = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request.clone(),
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::OK);
    let replayed: serde_json::Value = decode_json(replayed).await;
    assert_eq!(replayed["session"]["id"], child_id);
    assert_eq!(replayed["fork"]["id"], created["fork"]["id"]);

    let conflict = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": source.run_id,
            "title": "Different fork request",
            "idempotency_key": "fork-parent-terminal-api-1"
        }),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict: serde_json::Value = decode_json(conflict).await;
    assert_eq!(conflict["code"], "product_fork_conflict");

    let child_response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "Child continuation",
            "product_session_id": child_id
        }),
    )
    .await;
    let child_status = child_response.status();
    if child_status != StatusCode::OK {
        let error: serde_json::Value = decode_json(child_response).await;
        panic!("child fork turn failed with {child_status}: {error}");
    }
    let child: CreateJobResponse = decode_json(child_response).await;
    assert_ne!(child.job_id, source.job_id);
    assert_ne!(child.run_id, source.run_id);
    assert_eq!(child.resumed_from_run_id, None);
    let child_state = wait_for_done(app.clone(), child.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &child, &child_state).await;
    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let source_task_state = state_store.load_task_state(source.run_id).await.unwrap();
    let child_task_state = state_store.load_task_state(child.run_id).await.unwrap();
    assert_ne!(child_task_state.session_id, source_task_state.session_id);
    assert_ne!(child_task_state.job_id, source_task_state.job_id);

    let transcript = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{child_id}/transcript"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transcript.status(), StatusCode::OK);
    let transcript: serde_json::Value = decode_json(transcript).await;
    assert_eq!(transcript["status"], "complete", "{transcript}");
    let segments = transcript["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0]["inherited"], true);
    assert_eq!(segments[0]["source_product_session_id"], parent_id);
    assert_eq!(segments[0]["binding"]["product_session_id"], parent_id);
    assert_eq!(
        segments[0]["binding"]["runtime_run_id"],
        source.run_id.to_string()
    );
    assert_eq!(segments[1]["inherited"], false);
    assert_eq!(segments[1]["binding"]["product_session_id"], child_id);
    assert_eq!(segments[1]["binding"]["ordinal"], 2);
    assert_eq!(
        segments[1]["binding"]["runtime_run_id"],
        child.run_id.to_string()
    );

    std::fs::remove_file(
        folder
            .path()
            .join("api-state")
            .join("runs")
            .join(source.run_id.to_string())
            .join("task_state.json"),
    )
    .unwrap();
    let corrupt_source = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": source.run_id,
            "idempotency_key": "fork-corrupt-source"
        }),
    )
    .await;
    assert_eq!(corrupt_source.status(), StatusCode::CONFLICT);
    let corrupt_source: serde_json::Value = decode_json(corrupt_source).await;
    assert_eq!(corrupt_source["code"], "product_fork_source_invalid");

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/product/sessions/{parent_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let after_delete = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request,
    )
    .await;
    assert_eq!(after_delete.status(), StatusCode::OK);
    let after_delete: serde_json::Value = decode_json(after_delete).await;
    assert_eq!(after_delete["session"]["id"], child_id);
    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{parent_id}/forks"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    assert_eq!(listed["forks"].as_array().unwrap().len(), 1);

    let after_delete_transcript = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{child_id}/transcript"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_delete_transcript.status(), StatusCode::OK);
    let after_delete_transcript: serde_json::Value = decode_json(after_delete_transcript).await;
    assert_eq!(after_delete_transcript["segments"][0]["inherited"], true);
    assert_eq!(
        after_delete_transcript["segments"][0]["source_product_session_id"],
        parent_id
    );
}

/// A fork child starts with a clean compaction breaker.
///
/// The fork seed is the *parent's* run state: the same checkpoint under a new
/// session id. Its failure count and the cooldown it armed describe an outage the
/// child never had, so the child starts from a closed breaker — otherwise the
/// parent's broken summary model would refuse the child's automatic compaction,
/// and the child's own failures would grow a count the parent reports.
#[tokio::test]
async fn product_session_fork_child_starts_with_a_clean_compaction_breaker() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let parent = create_product_session(&app, &workspace_id, "Fork breaker parent").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();
    let source = create_product_job(&app, &parent_id, "Breaker fork source").await;
    let source_state = wait_for_done(app.clone(), source.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &source, &source_state).await;

    // The state a parent session holds once its summary model has failed to the
    // threshold, with the window its last failure armed still open.
    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let closed_until = chrono::Utc::now() + chrono::Duration::minutes(5);
    let mut seeded = state_store.load_task_state(source.run_id).await.unwrap();
    {
        let checkpoint = seeded
            .checkpoint
            .as_mut()
            .expect("the source turn writes a prompt checkpoint");
        checkpoint.compaction.mode = PromptCompactionMode::Degraded;
        checkpoint.compaction.degraded = true;
        checkpoint.compaction.consecutive_failures = 3;
        checkpoint.compaction.circuit_open = true;
        checkpoint.compaction.next_attempt_after = Some(closed_until.to_rfc3339());
        checkpoint.compaction.last_error = Some("summary model failed".to_string());
    }
    state_store.write_task_state(&seeded).await.unwrap();

    let created = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": source.run_id,
            "idempotency_key": "fork-parent-breaker-1"
        }),
    )
    .await;
    let created_status = created.status();
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
    let child_id = created["session"]["id"].as_str().unwrap().to_string();

    // The child's first turn is the run that consumes the fork seed.
    let child_response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "Child continuation",
            "product_session_id": child_id
        }),
    )
    .await;
    let child_status = child_response.status();
    if child_status != StatusCode::OK {
        let error: serde_json::Value = decode_json(child_response).await;
        panic!("child fork turn failed with {child_status}: {error}");
    }
    let child: CreateJobResponse = decode_json(child_response).await;
    wait_for_done(app.clone(), child.job_id.to_string()).await;

    let child_state = state_store.load_task_state(child.run_id).await.unwrap();
    assert_ne!(
        child_state.session_id, seeded.session_id,
        "the child is its own session, which is the whole reason its breaker is its own"
    );
    let compaction = &child_state
        .checkpoint
        .as_ref()
        .expect("the child turn writes a prompt checkpoint")
        .compaction;
    assert_eq!(
        compaction.consecutive_failures, 0,
        "the parent's failures are not the child's"
    );
    assert!(
        !compaction.circuit_open,
        "the child must not start with the parent's breaker"
    );
    assert!(
        compaction.next_attempt_after.is_none(),
        "the child must not wait out the parent's cooldown"
    );
    assert!(
        !compaction.degraded && compaction.last_error.is_none(),
        "the parent's outage must not travel with its breaker"
    );

    // The parent keeps its own breaker: a child starting clean is a child-side
    // fact, not a parent-side reset.
    let parent_after = state_store.load_task_state(source.run_id).await.unwrap();
    let parent_compaction = &parent_after
        .checkpoint
        .as_ref()
        .expect("checkpoint")
        .compaction;
    assert_eq!(parent_compaction.consecutive_failures, 3);
    assert!(parent_compaction.circuit_open);
    assert_eq!(
        parent_compaction.next_attempt_after.as_deref(),
        Some(closed_until.to_rfc3339().as_str())
    );
}

#[tokio::test]
async fn product_session_fork_rejects_incomplete_and_active_sources() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let empty = create_product_session(&app, workspace_id, "No durable fork source").await;
    let empty_id = empty["id"].as_str().unwrap();
    let incomplete = post_json(
        &app,
        &format!("/product/sessions/{empty_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": RunId::new(),
            "idempotency_key": "fork-no-terminal-run"
        }),
    )
    .await;
    assert_eq!(incomplete.status(), StatusCode::CONFLICT);
    let incomplete: serde_json::Value = decode_json(incomplete).await;
    assert_eq!(incomplete["code"], "product_fork_source_invalid");

    let active = create_product_session(&app, workspace_id, "Active fork source").await;
    let active_id = active["id"].as_str().unwrap();
    configure_product_session_model(&app, active_id, "fake-raw", 1).await;
    let waiting_message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "keep the fork source active" }
    })
    .to_string();
    let active_job = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": waiting_message,
            "product_session_id": active_id
        }),
    )
    .await;
    assert_eq!(active_job.status(), StatusCode::OK);
    let active_job: CreateJobResponse = decode_json(active_job).await;
    wait_for_pending_input(app.clone(), active_job.job_id.to_string()).await;
    let active_source = post_json(
        &app,
        &format!("/product/sessions/{active_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": active_job.run_id,
            "idempotency_key": "fork-active-run"
        }),
    )
    .await;
    assert_eq!(active_source.status(), StatusCode::CONFLICT);
    let active_source: serde_json::Value = decode_json(active_source).await;
    assert_eq!(active_source["code"], "product_session_active");
}

// ─── R6: message-level edit-and-resend = fork-at-message ──────────────────
//
// rove never truncates a session in place. Editing a user message and resending
// it creates a new child session whose private seed stops before that message;
// the parent history, its message ledger, and its append-only trace stay
// untouched.

/// Wait until a unified message has been delivered as the trigger of its
/// successor run. `successor_run_id` is written when the successor turn claims
/// the message, so a string value also identifies the run the message belongs
/// to.
async fn wait_for_delivered_message(
    app: &axum::Router,
    product_session_id: &str,
    message_id: &str,
) -> serde_json::Value {
    let mut last = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let message = list_product_messages(app, product_session_id)
            .await
            .into_iter()
            .find(|message| message["id"] == message_id)
            .expect("product message");
        if message["successor_run_id"].is_string() {
            return message;
        }
        last = Some(message);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("product message {message_id} was never delivered; last: {last:?}");
}

/// Wait for the durable terminal boundary of the run a delivered message
/// started. The binding is committed when the turn claims the message, so
/// `latest_run_id == run_id` together with `idle` proves that turn already
/// finished instead of racing the drain that starts it.
async fn wait_for_successor_run_done(
    app: &axum::Router,
    workspace_id: &str,
    product_session_id: &str,
    run_id: &str,
) -> JobStateResponse {
    let mut last = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let session = get_product_session(app, workspace_id, product_session_id).await;
        let binding = &session["runtime_binding"];
        if session["status"] == "idle"
            && binding["latest_run_id"] == run_id
            && session["last_outcome"] == "success"
        {
            let job_id = binding["latest_job_id"].as_str().unwrap().to_string();
            return wait_for_done(app.clone(), job_id).await;
        }
        last = Some(session);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("successor run {run_id} never reached its durable terminal boundary; last: {last:?}");
}

/// The parent facts a fork must not touch: the durable artifacts of every one
/// of its runs, its canonical transcript projection, and its message ledger.
async fn parent_fork_facts(
    app: &axum::Router,
    workspace_root: &Path,
    product_session_id: &str,
    run_ids: &[&str],
) -> (Vec<(String, Vec<u8>)>, String, Vec<serde_json::Value>) {
    let mut artifacts = Vec::new();
    for run_id in run_ids {
        for name in ["task_state.json", "trace.jsonl", "report.json"] {
            let path = workspace_root
                .join("api-state")
                .join("runs")
                .join(run_id)
                .join(name);
            artifacts.push((
                format!("{run_id}/{name}"),
                std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}")),
            ));
        }
    }
    let transcript = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{product_session_id}/transcript"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transcript.status(), StatusCode::OK);
    let transcript = axum::body::to_bytes(transcript.into_body(), usize::MAX)
        .await
        .unwrap();
    let transcript = String::from_utf8(transcript.to_vec()).unwrap();
    let messages = list_product_messages(app, product_session_id).await;
    (artifacts, transcript, messages)
}

/// The canonical session a durable run started from, projected the way the
/// runtime rebuilds a provider prompt: entry identities plus role/content.
fn session_seed(state: &TaskState) -> (Vec<String>, Vec<(Role, String)>) {
    let checkpoint = state.checkpoint.as_ref().expect("prompt checkpoint");
    let session = checkpoint.session.as_ref().expect("canonical session");
    let ids = session
        .entries
        .iter()
        .map(|entry| entry.id().to_string())
        .collect();
    let messages = session
        .messages_for_compatibility_artifact()
        .expect("projectable session")
        .into_iter()
        .map(|message| (message.role, message.content))
        .collect();
    (ids, messages)
}

/// Fail when one durable artifact of a fork child still carries content the
/// edit removed, quoting the bytes around it so the leaking field is obvious.
fn assert_artifact_is_free_of(workspace_root: &Path, run_id: RunId, name: &str, marker: &str) {
    let path = workspace_root
        .join("api-state")
        .join("runs")
        .join(run_id.to_string())
        .join(name);
    let content = std::fs::read_to_string(&path).unwrap();
    let Some(index) = content.find(marker) else {
        return;
    };
    let start = index.saturating_sub(240);
    let end = (index + 240).min(content.len());
    panic!(
        "{name} still carries content the edit removed: ...{}...",
        content.get(start..end).unwrap_or(marker)
    );
}

#[tokio::test]
async fn product_message_fork_cuts_the_child_before_the_edited_user_message() {
    const PARENT_FIRST: &str = "parent first turn marker";
    const PARENT_SECOND: &str = "parent second turn marker to edit";
    const CHILD_EDITED: &str = "child edited second turn marker";

    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let parent = create_product_session(&app, &workspace_id, "Edit and resend parent").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();

    // Turn one is an ordinary job: it is the earlier history the child keeps.
    let first = create_product_job(&app, &parent_id, PARENT_FIRST).await;
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &first, &first_state).await;

    // Turn two is a unified product message, so it owns a ledger sequence a
    // client can address as the edit target.
    let second_message = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/messages"),
        serde_json::json!({ "content": PARENT_SECOND, "idempotency_key": "edit-source-1" }),
    )
    .await;
    assert_eq!(second_message.status(), StatusCode::CREATED);
    let second_message: serde_json::Value = decode_json(second_message).await;
    let edited_message_id = second_message["id"].as_str().unwrap().to_string();
    let edited_seq = second_message["seq"].as_i64().unwrap();
    assert_eq!(second_message["status"], "queued");

    let delivered = wait_for_delivered_message(&app, &parent_id, &edited_message_id).await;
    let second_run_id = delivered["successor_run_id"].as_str().unwrap().to_string();
    let second = wait_for_successor_run_done(&app, &workspace_id, &parent_id, &second_run_id).await;
    assert_eq!(second.run_id.to_string(), second_run_id);

    // Premise: the fork source already carries the earlier turn and the edited
    // message, so the cut has something to keep and something to remove.
    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let source_state = state_store.load_task_state(second.run_id).await.unwrap();
    let (_, source_messages) = session_seed(&source_state);
    assert!(
        source_messages
            .iter()
            .any(|(role, content)| *role == Role::User && content == PARENT_FIRST),
        "the fork source must inherit the earlier turn: {source_messages:?}"
    );
    assert!(
        source_messages
            .iter()
            .any(|(role, content)| *role == Role::User && content == PARENT_SECOND),
        "the fork source must carry the edited message: {source_messages:?}"
    );

    let first_run_id = first.run_id.to_string();
    let before = parent_fork_facts(
        &app,
        folder.path(),
        &parent_id,
        &[first_run_id.as_str(), second_run_id.as_str()],
    )
    .await;

    let fork_request = serde_json::json!({
        "fork_at_run_id": second_run_id,
        "idempotency_key": "fork-edit-message-1",
        "truncate_after_message_seq": edited_seq,
    });
    let created = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request.clone(),
    )
    .await;
    let created_status = created.status();
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
    let child_id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["session"]["parent_session_id"], parent_id);
    assert_eq!(created["session"]["fork_point_run_id"], second_run_id);
    assert_eq!(created["fork"]["source_runtime_run_id"], second_run_id);
    assert_eq!(created["fork"]["truncate_after_message_seq"], edited_seq);
    assert_eq!(
        created["fork"]["truncate_after_message_id"],
        edited_message_id
    );

    // The parent is append-only: its durable artifacts, its transcript, and its
    // message ledger are byte-identical across the fork.
    let after = parent_fork_facts(
        &app,
        folder.path(),
        &parent_id,
        &[first_run_id.as_str(), second_run_id.as_str()],
    )
    .await;
    assert_eq!(before, after, "the fork must not rewrite the parent");

    // Idempotent replay returns the same child with the same recorded cut.
    let replayed = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request.clone(),
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::OK);
    let replayed: serde_json::Value = decode_json(replayed).await;
    assert_eq!(replayed["session"]["id"], child_id);
    assert_eq!(replayed["fork"]["id"], created["fork"]["id"]);
    assert_eq!(replayed["fork"]["truncate_after_message_seq"], edited_seq);
    assert_eq!(
        replayed["fork"]["truncate_after_message_id"],
        edited_message_id
    );

    // The cut participates in request identity: the same key without it is a
    // different request, not a silent replay.
    let conflict = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": second_run_id,
            "idempotency_key": "fork-edit-message-1",
        }),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict: serde_json::Value = decode_json(conflict).await;
    assert_eq!(conflict["code"], "product_fork_conflict");

    // Drop the parent catalog row, its message ledger, and its fork index. The
    // cut is stored on the fork record itself, so replay and the child's seed
    // survive without the ledger the client originally addressed.
    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/product/sessions/{parent_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let orphaned_replay = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        fork_request,
    )
    .await;
    assert_eq!(orphaned_replay.status(), StatusCode::OK);
    let orphaned_replay: serde_json::Value = decode_json(orphaned_replay).await;
    assert_eq!(orphaned_replay["session"]["id"], child_id);
    assert_eq!(
        orphaned_replay["fork"]["truncate_after_message_seq"],
        edited_seq
    );

    // The child is a usable session whose first turn starts from the pruned
    // prefix instead of the parent's whole run.
    let child_turn = create_product_job(&app, &child_id, CHILD_EDITED).await;
    assert_eq!(child_turn.resumed_from_run_id, None);
    let child_live = wait_for_done(app.clone(), child_turn.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &child_turn, &child_live).await;

    let child_state = state_store
        .load_task_state(child_turn.run_id)
        .await
        .unwrap();
    let (entry_ids, messages) = session_seed(&child_state);
    let user_messages: Vec<&str> = messages
        .iter()
        .filter(|(role, _)| *role == Role::User)
        .map(|(_, content)| content.as_str())
        .collect();
    assert_eq!(
        user_messages,
        vec![PARENT_FIRST, CHILD_EDITED],
        "the child keeps the turns before the edited message and nothing after it: {messages:?}"
    );
    assert!(
        entry_ids.contains(&format!("user-{}", first.run_id)),
        "the kept prefix keeps its original entry identity: {entry_ids:?}"
    );
    assert!(
        !entry_ids.contains(&format!("user-{second_run_id}")),
        "the edited run's trigger entry must not be seeded into the child: {entry_ids:?}"
    );
    assert!(
        child_state
            .checkpoint
            .as_ref()
            .expect("prompt checkpoint")
            .history_pruned,
        "the child records that its pruned seed is deliberate, so resume cannot refill it"
    );
    assert!(
        child_state
            .history
            .iter()
            .all(|message| !message.content.contains(PARENT_SECOND)),
        "the legacy history projection must not carry the pruned message"
    );

    // Nothing the edit removed may appear in the child's own durable artifacts.
    // The inherited transcript segment is a projection of the parent's
    // append-only trace by contract, so it is deliberately not scanned here.
    for name in ["task_state.json", "trace.jsonl", "report.json"] {
        assert_artifact_is_free_of(folder.path(), child_turn.run_id, name, PARENT_SECOND);
    }
}

#[tokio::test]
async fn product_message_fork_from_a_runs_trigger_message_keeps_no_parent_entries() {
    const ONLY_PARENT_TURN: &str = "only parent turn marker";
    const CHILD_FROM_EMPTY_SEED: &str = "child from empty seed marker";

    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let parent = create_product_session(&app, &workspace_id, "Whole run edit parent").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();

    // The session's very first turn is a unified message, so the edit target is
    // the run's own trigger and the cut removes the entire run.
    let message = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/messages"),
        serde_json::json!({ "content": ONLY_PARENT_TURN, "idempotency_key": "empty-seed-source" }),
    )
    .await;
    assert_eq!(message.status(), StatusCode::CREATED);
    let message: serde_json::Value = decode_json(message).await;
    let message_id = message["id"].as_str().unwrap().to_string();
    let message_seq = message["seq"].as_i64().unwrap();

    let delivered = wait_for_delivered_message(&app, &parent_id, &message_id).await;
    let parent_run_id = delivered["successor_run_id"].as_str().unwrap().to_string();
    let parent_run =
        wait_for_successor_run_done(&app, &workspace_id, &parent_id, &parent_run_id).await;
    assert_eq!(parent_run.run_id.to_string(), parent_run_id);

    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let source_state = state_store
        .load_task_state(parent_run.run_id)
        .await
        .unwrap();
    let (source_entry_ids, source_messages) = session_seed(&source_state);
    assert_eq!(
        source_messages.len(),
        2,
        "the source run is its trigger plus its answer: {source_messages:?}"
    );
    assert_eq!(
        source_messages[0],
        (Role::User, ONLY_PARENT_TURN.to_string())
    );
    assert_eq!(source_entry_ids.len(), 2);

    let before =
        parent_fork_facts(&app, folder.path(), &parent_id, &[parent_run_id.as_str()]).await;

    let created = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": parent_run_id,
            "idempotency_key": "fork-empty-seed-1",
            "truncate_after_message_seq": message_seq,
        }),
    )
    .await;
    let created_status = created.status();
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
    let child_id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["fork"]["truncate_after_message_seq"], message_seq);

    let after = parent_fork_facts(&app, folder.path(), &parent_id, &[parent_run_id.as_str()]).await;
    assert_eq!(before, after, "the fork must not rewrite the parent");

    let child_turn = create_product_job(&app, &child_id, CHILD_FROM_EMPTY_SEED).await;
    let child_live = wait_for_done(app.clone(), child_turn.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(folder.path(), &child_turn, &child_live).await;

    let child_state = state_store
        .load_task_state(child_turn.run_id)
        .await
        .unwrap();
    let (entry_ids, messages) = session_seed(&child_state);
    let user_messages: Vec<&str> = messages
        .iter()
        .filter(|(role, _)| *role == Role::User)
        .map(|(_, content)| content.as_str())
        .collect();
    assert_eq!(
        user_messages,
        vec![CHILD_FROM_EMPTY_SEED],
        "cutting at the run's trigger message leaves no parent entry: {messages:?}"
    );
    assert!(
        !entry_ids.iter().any(|id| source_entry_ids.contains(id)),
        "the child must not inherit any entry of the run it replaces: {entry_ids:?}"
    );
    assert_eq!(
        entry_ids
            .iter()
            .filter(|id| id.starts_with("user-"))
            .count(),
        1,
        "the child's own turn is the only user entry: {entry_ids:?}"
    );
    assert!(
        child_state
            .checkpoint
            .as_ref()
            .expect("prompt checkpoint")
            .history_pruned,
        "an empty seed is deliberate, not a lost snapshot"
    );

    for name in ["task_state.json", "trace.jsonl", "report.json"] {
        assert_artifact_is_free_of(folder.path(), child_turn.run_id, name, ONLY_PARENT_TURN);
    }
}

#[tokio::test]
async fn product_message_fork_rejects_unusable_truncation_targets() {
    const PARENT_FIRST: &str = "rejection parent first turn";
    const PARENT_SECOND: &str = "rejection parent second turn";

    async fn rejected(
        app: &axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = post_json(app, uri, body).await;
        let status = response.status();
        (status, decode_json(response).await)
    }

    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let parent = create_product_session(&app, &workspace_id, "Edit rejection parent").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();
    let fork_uri = format!("/product/sessions/{parent_id}/forks");

    let first = create_product_job(&app, &parent_id, PARENT_FIRST).await;
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;

    let second_message = post_json(
        &app,
        &format!("/product/sessions/{parent_id}/messages"),
        serde_json::json!({ "content": PARENT_SECOND, "idempotency_key": "edit-source-reject" }),
    )
    .await;
    assert_eq!(second_message.status(), StatusCode::CREATED);
    let second_message: serde_json::Value = decode_json(second_message).await;
    let edited_seq = second_message["seq"].as_i64().unwrap();
    let delivered =
        wait_for_delivered_message(&app, &parent_id, second_message["id"].as_str().unwrap()).await;
    let second_run_id = delivered["successor_run_id"].as_str().unwrap().to_string();
    let second = wait_for_successor_run_done(&app, &workspace_id, &parent_id, &second_run_id).await;

    let ledger_seqs: Vec<i64> = list_product_messages(&app, &parent_id)
        .await
        .iter()
        .map(|message| message["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(ledger_seqs, vec![edited_seq]);

    // A sequence no product message holds, in any shape.
    for (key, seq) in [
        ("reject-unknown-sequence", serde_json::json!(9_999)),
        ("reject-zero-sequence", serde_json::json!(0)),
        ("reject-negative-sequence", serde_json::json!(-3)),
        ("reject-non-integer-sequence", serde_json::json!("1")),
    ] {
        let (status, error) = rejected(
            &app,
            &fork_uri,
            serde_json::json!({
                "fork_at_run_id": second_run_id,
                "idempotency_key": key,
                "truncate_after_message_seq": seq,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "case {key}: {error}");
        assert_eq!(
            error["code"], "product_invalid_input",
            "case {key}: {error}"
        );
    }

    // The parent run's canonical event stream has its own per-run sequence
    // space: an assistant turn, a tool fact, or any other lifecycle event is
    // never a product message sequence, so none of them can be an edit target.
    let event_seqs: Vec<i64> = second
        .events
        .iter()
        .map(|event| event.seq as i64)
        .filter(|seq| !ledger_seqs.contains(seq))
        .collect();
    assert!(
        event_seqs.len() > 1,
        "the fork run must expose its own event sequence space: {:?}",
        second.events.len()
    );
    for seq in event_seqs {
        let (status, error) = rejected(
            &app,
            &fork_uri,
            serde_json::json!({
                "fork_at_run_id": second_run_id,
                "idempotency_key": format!("reject-event-seq-{seq}"),
                "truncate_after_message_seq": seq,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "event seq {seq}: {error}");
        assert_eq!(
            error["code"], "product_invalid_input",
            "event seq {seq}: {error}"
        );
    }

    // A real user message of the same session, but delivered to another run.
    let (status, error) = rejected(
        &app,
        &fork_uri,
        serde_json::json!({
            "fork_at_run_id": first.run_id,
            "idempotency_key": "reject-foreign-run-sequence",
            "truncate_after_message_seq": edited_seq,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "product_fork_source_invalid", "{error}");

    // A run that is not bound to this session, with an otherwise valid target.
    let (status, error) = rejected(
        &app,
        &fork_uri,
        serde_json::json!({
            "fork_at_run_id": RunId::new(),
            "idempotency_key": "reject-unbound-run",
            "truncate_after_message_seq": edited_seq,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "product_fork_source_invalid", "{error}");

    // An active turn still dominates: the truncation target cannot fork a
    // session whose boundary is not terminal yet.
    let live = create_product_session(&app, &workspace_id, "Active edit target").await;
    let live_id = live["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, live_id.as_str(), "fake-raw", 1).await;
    let live_message = post_json(
        &app,
        &format!("/product/sessions/{live_id}/messages"),
        serde_json::json!({
            "content": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "keep the edit target active" }
            })
            .to_string(),
            "idempotency_key": "edit-source-live",
        }),
    )
    .await;
    assert_eq!(live_message.status(), StatusCode::CREATED);
    let live_message: serde_json::Value = decode_json(live_message).await;
    let live_delivered =
        wait_for_delivered_message(&app, &live_id, live_message["id"].as_str().unwrap()).await;
    let live_run_id = live_delivered["successor_run_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut live_status = String::new();
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let session = get_product_session(&app, &workspace_id, &live_id).await;
        if session["runtime_binding"]["latest_run_id"] == live_run_id.as_str() {
            live_status = session["status"].as_str().unwrap().to_string();
            if live_status == "running" {
                break;
            }
        }
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    assert_eq!(
        live_status, "running",
        "the live edit target must hold its session open"
    );
    let (status, error) = rejected(
        &app,
        &format!("/product/sessions/{live_id}/forks"),
        serde_json::json!({
            "fork_at_run_id": live_run_id,
            "idempotency_key": "reject-active-parent",
            "truncate_after_message_seq": live_message["seq"].as_i64().unwrap(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "product_session_active", "{error}");

    // Nothing above created a child or touched the parent it named.
    let listed = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{parent_id}/forks"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    assert!(
        listed["forks"].as_array().unwrap().is_empty(),
        "a rejected edit must not leave a fork behind: {listed}"
    );
    assert_eq!(first_state.status, RunStatus::Done);
}

#[tokio::test]
async fn product_session_resume_fails_closed_when_exact_task_state_is_missing() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Missing state").await;
    let session_id = session["id"].as_str().unwrap();
    let first = create_product_job(&app, session_id, "durable first turn").await;
    wait_for_done(app.clone(), first.job_id.to_string()).await;

    std::fs::remove_file(
        folder
            .path()
            .join("api-state")
            .join("runs")
            .join(first.run_id.to_string())
            .join("task_state.json"),
    )
    .unwrap();
    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must not become a disconnected turn",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_session_runtime_state_missing");

    let sessions = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions?workspace_id={workspace_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let sessions: serde_json::Value = decode_json(sessions).await;
    assert_eq!(sessions["sessions"][0]["status"], "needs_attention");
    // A run that needed attention did not deliver a final answer, so the
    // session must not report the latest turn as a success.
    assert_eq!(sessions["sessions"][0]["last_outcome"], "failed");
    assert_eq!(
        sessions["sessions"][0]["runtime_binding"]["latest_run_id"],
        first.run_id.to_string()
    );
}

/// The listing pages over HTTP, and the cursor a client
/// receives is the only thing it needs to continue.
#[tokio::test]
async fn product_session_listing_pages_over_http_and_rejects_broken_page_requests() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    for index in 0..7 {
        create_product_session(&app, workspace_id, &format!("Session {index}")).await;
    }

    // Walk the whole listing three at a time, following only what the responses
    // hand back — the same thing a client can see.
    let mut seen: Vec<String> = Vec::new();
    let mut uri = format!("/product/sessions?workspace_id={workspace_id}&limit=3");
    for _ in 0..8 {
        let response = get_response(&app, &uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = decode_json(response).await;
        let page = body["sessions"].as_array().unwrap();
        assert!(page.len() <= 3, "the server exceeded the requested limit");
        seen.extend(
            page.iter()
                .map(|session| session["id"].as_str().unwrap().to_string()),
        );
        match body["next_cursor"].as_str() {
            Some(cursor) => {
                uri = format!(
                    "/product/sessions?workspace_id={workspace_id}&limit=3&cursor={cursor}"
                );
            }
            None => break,
        }
    }
    assert_eq!(seen.len(), 7, "the paged walk did not cover the listing");
    let unique: std::collections::BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 7, "a session was delivered twice: {seen:?}");

    // The unpaged default still returns everything, so existing clients that
    // never send a limit are unaffected.
    let response = get_response(
        &app,
        &format!("/product/sessions?workspace_id={workspace_id}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["sessions"].as_array().unwrap().len(), 7);
    assert!(
        body["next_cursor"].is_null(),
        "a listing that fits in one page must not offer a cursor"
    );

    // A search narrows the listing, and the term is matched literally.
    let response = get_response(
        &app,
        &format!("/product/sessions?workspace_id={workspace_id}&q=Session%204"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);

    // Every malformed page request is refused. Returning page one instead would
    // make a client silently re-read the listing from the start.
    for bad in [
        "limit=0",
        "limit=201",
        "cursor=not-base64!",
        "cursor=e30",
        &format!("q={}", "x".repeat(129)),
    ] {
        let response = get_response(
            &app,
            &format!("/product/sessions?workspace_id={workspace_id}&{bad}"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{bad}` should have been rejected"
        );
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_invalid_input", "for `{bad}`");
    }
}

/// Start a product session whose turn is held open by a pending input.
///
/// Message search needs messages in the ledger; queueing them behind a live run
/// keeps them there as `queued` rows instead of racing one fake-provider turn
/// per message, which keeps these tests fast and deterministic.
async fn hold_product_session_turn_open(app: &axum::Router, session_id: &str) {
    hold_product_session_turn_open_with_prompt(app, session_id, "hold the turn open").await;
}

/// The same, with a prompt the caller chooses.
///
/// The prompt reaches the durable run through the canonical tool-call event, so
/// a caller can put a marker in `trace.jsonl` and nowhere else — which is what
/// makes "find a run by something only the event trace carries" testable.
async fn hold_product_session_turn_open_with_prompt(
    app: &axum::Router,
    session_id: &str,
    prompt: &str,
) {
    let active = post_json(
        app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": prompt }
            })
            .to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    assert!(
        !pending.pending_inputs.is_empty(),
        "the turn must be waiting for input"
    );
}

/// Percent-encode one query parameter value.
///
/// The URI builder rejects a raw non-ASCII byte, and a Chinese search term is
/// the case this feature exists for, so the tests must ask the way a browser
/// would.
fn encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(*byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

async fn get_product_message_search(
    app: &axum::Router,
    product_session_id: &str,
    query: &str,
) -> axum::response::Response {
    get_response(
        app,
        &format!("/product/sessions/{product_session_id}/search?{query}"),
    )
    .await
}

#[tokio::test]
async fn product_message_search_finds_hits_and_rejects_every_malformed_request() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Message search").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;
    hold_product_session_turn_open(&app, &session_id).await;

    let chinese = queue_message(&app, &session_id, "我们讨论了运行时合同的搜索能力", "s-1").await;
    let english = queue_message(
        &app,
        &session_id,
        "Runtime contract alignment for message search",
        "s-2",
    )
    .await;
    queue_message(&app, &session_id, "an unrelated note", "s-3").await;

    // A term above the three-character trigram floor goes through the index.
    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("运行时合同")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "one message contains the term: {body}");
    assert_eq!(hits[0]["message_seq"], chinese["seq"]);
    assert_eq!(
        hits[0]["created_at"], chinese["created_at"],
        "a hit must carry the ledger timestamp of the message it names"
    );
    assert!(
        hits[0]["snippet"].as_str().unwrap().contains("合同"),
        "the snippet must show the hit: {body}"
    );
    assert!(
        body.get("next_cursor").is_none(),
        "a page that holds every hit must not offer a cursor: {body}"
    );

    // A two-character Chinese term is below the trigram floor and must still be
    // found, through the bounded fallback scan.
    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("合同")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["hits"][0]["message_seq"], chinese["seq"]);

    // English, case-insensitively.
    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("CoNtRaCt AlIgNmEnT")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["hits"][0]["message_seq"], english["seq"]);

    // An empty result is a successful empty page, never an error.
    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("nothing contains this")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert!(body["hits"].as_array().unwrap().is_empty());
    assert!(body.get("next_cursor").is_none());

    // Every malformed request is a typed 400. Answering any of them with an
    // empty page would tell a client "no hits" when it asked a bad question.
    for query in [
        "limit=5".to_string(),
        "q=".to_string(),
        "q=%20%20".to_string(),
        format!("q={}", encode_query_value(&"x".repeat(129))),
        format!("q={}&limit=0", encode_query_value("合同")),
        format!("q={}&limit=101", encode_query_value("合同")),
        format!("q={}&cursor=not-base64!", encode_query_value("合同")),
        format!("q={}&cursor=e30", encode_query_value("合同")),
        // Inputs the axum `Query` extractor refuses before the handler runs:
        // a non-numeric limit and a repeated term. The last case is different in
        // kind — percent-decoding is lossy, so `%FF` arrives as U+FFFD and the
        // handler's unrepresentable-character check is what refuses it. All
        // three must still carry the documented typed envelope; axum's own
        // plain-text 400 has no `code` and a client parsing the envelope would
        // throw on it.
        format!("q={}&limit=abc", encode_query_value("合同")),
        format!(
            "q={}&q={}",
            encode_query_value("合同"),
            encode_query_value("合同")
        ),
        "q=%FF".to_string(),
    ] {
        let response = get_product_message_search(&app, &session_id, &query).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{query}` should have been rejected"
        );
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_invalid_input", "for `{query}`");
    }

    // A search term is not an FTS5 query expression: operators inside it are
    // literal text, not syntax, and an unbalanced quote is a miss rather than a
    // server error.
    for term in ["contract OR 合同", "\"unbalanced", "合同 AND 搜索", "NEAR("] {
        let response = get_product_message_search(
            &app,
            &session_id,
            &format!("q={}", encode_query_value(term)),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "`{term}` must be searched literally"
        );
    }

    // An unknown session is not found. An empty hit list here would answer
    // "this session has no matching message" for a session that does not exist.
    let response = get_product_message_search(
        &app,
        &ProductSessionId::new().to_string(),
        &format!("q={}", encode_query_value("合同")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_not_found");
}

#[tokio::test]
async fn product_message_search_answers_a_secret_shaped_multi_byte_message() {
    // `token=` followed by a long CJK run with no ASCII separator. The snippet's
    // secret redaction clamps the token on a byte offset; when that offset split
    // a character the panic unwound out of `spawn_blocking`, the store turned it
    // into a storage failure, and every search matching this message answered
    // 500. A valid search must answer 200 with the hit.
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Secret-shaped search").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;
    hold_product_session_turn_open(&app, &session_id).await;

    let body = format!("搜索命中 token={}", "密码".repeat(300));
    let hit = queue_message(&app, &session_id, &body, "secret-tail").await;

    let response = get_product_message_search(
        &app,
        &session_id,
        &format!("q={}", encode_query_value("搜索命中")),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a secret-shaped prefix must not turn a valid search into a server error"
    );
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "the hit must be returned: {body}");
    assert_eq!(hits[0]["message_seq"], hit["seq"]);
    assert!(
        hits[0]["snippet"].as_str().unwrap().contains("搜索命中"),
        "the excerpt must show the hit: {body}"
    );
}

#[tokio::test]
async fn product_message_search_pages_by_cursor_and_never_leaks_neighbouring_text() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Message search paging").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;
    hold_product_session_turn_open(&app, &session_id).await;

    let mut written = Vec::new();
    for index in 0..5 {
        written.push(
            queue_message(
                &app,
                &session_id,
                &format!("paged needle message {index}"),
                &format!("page-{index}"),
            )
            .await["seq"]
                .as_i64()
                .unwrap(),
        );
    }
    // A long message whose match sits behind far more text than a snippet may
    // carry. The two markers are thousands of codepoints away from the hit, so
    // a correct excerpt cannot contain either of them.
    let long_body = format!(
        "FAR-LEFT-MARKER {}{} {}FAR-RIGHT-MARKER",
        "filler ".repeat(600),
        "paged needle buried in a long message",
        "filler ".repeat(600)
    );
    let long_seq = queue_message(&app, &session_id, &long_body, "page-long").await["seq"]
        .as_i64()
        .unwrap();
    written.push(long_seq);
    let unrelated = queue_message(&app, &session_id, "no needle here at all", "page-other").await;

    let term = encode_query_value("paged needle");
    let mut seen: Vec<i64> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let query = match cursor.as_deref() {
            Some(cursor) => format!("q={term}&limit=2&cursor={cursor}"),
            None => format!("q={term}&limit=2"),
        };
        let response = get_product_message_search(&app, &session_id, &query).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = decode_json(response).await;
        let hits = body["hits"].as_array().unwrap();
        assert!(hits.len() <= 2, "the server exceeded the requested limit");
        for hit in hits {
            let snippet = hit["snippet"].as_str().unwrap();
            assert!(
                snippet.chars().count() <= 160,
                "a snippet must stay inside its codepoint budget: {snippet}"
            );
            assert!(
                snippet.contains("paged needle"),
                "a snippet must show the hit: {snippet}"
            );
            assert!(
                !snippet.contains("FAR-LEFT-MARKER") && !snippet.contains("FAR-RIGHT-MARKER"),
                "a snippet must not carry text from outside the window around the hit: {snippet}"
            );
            if hit["message_seq"].as_i64() == Some(long_seq) {
                assert!(
                    snippet.chars().count() < long_body.chars().count() / 2,
                    "the excerpt must be a window, not the message: {snippet}"
                );
            }
            seen.push(hit["message_seq"].as_i64().unwrap());
        }
        pages += 1;
        assert!(pages < 10, "pagination must terminate");
        match body.get("next_cursor").and_then(|value| value.as_str()) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }

    assert_eq!(pages, 3, "six hits at two per page is three pages");
    assert_eq!(seen, written, "the walk must return every hit exactly once");
    assert!(!seen.contains(&unrelated["seq"].as_i64().unwrap()));

    // The cursor is bound to the term that produced it, so a token reused with
    // another search is refused instead of silently skipping or repeating hits.
    let response =
        get_product_message_search(&app, &session_id, &format!("q={term}&cursor=e30")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_invalid_input");
}

#[tokio::test]
async fn product_message_search_is_session_scoped_and_authorized_like_the_message_listing() {
    let server = tempfile::TempDir::new().unwrap();
    let first_folder = tempfile::TempDir::new().unwrap();
    let second_folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let first_workspace = create_product_workspace(&app, first_folder.path()).await;
    let second_workspace = create_product_workspace(&app, second_folder.path()).await;
    let first =
        create_product_session(&app, first_workspace["id"].as_str().unwrap(), "First").await;
    let second =
        create_product_session(&app, second_workspace["id"].as_str().unwrap(), "Second").await;
    let first_id = first["id"].as_str().unwrap().to_string();
    let second_id = second["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &first_id, "fake-raw", 2).await;
    configure_product_session_model(&app, &second_id, "fake-raw", 2).await;
    hold_product_session_turn_open(&app, &first_id).await;
    hold_product_session_turn_open(&app, &second_id).await;

    // The same term matches in both sessions.
    queue_message(&app, &first_id, "shared secret-topic marker one", "scope-1").await;
    queue_message(
        &app,
        &second_id,
        "shared secret-topic marker two",
        "scope-2",
    )
    .await;

    let term = encode_query_value("secret-topic marker");
    for (session_id, expected) in [(&first_id, "one"), (&second_id, "two")] {
        let response = get_product_message_search(&app, session_id, &format!("q={term}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = decode_json(response).await;
        let hits = body["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "search must stay inside one session: {body}");
        assert!(
            hits[0]["snippet"].as_str().unwrap().contains(expected),
            "a session must never see another session's hit: {body}"
        );
    }

    // A cursor is bound to the session that minted it as well as to the term.
    // The second session holds the same term, so the cursor's `seq` is valid
    // there: without the session binding it would be accepted and would
    // silently skip that session's earlier hits, reading as "no results".
    queue_message(
        &app,
        &first_id,
        "shared secret-topic marker one continued",
        "scope-3",
    )
    .await;
    let response = get_product_message_search(&app, &first_id, &format!("q={term}&limit=1")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = decode_json(response).await;
    let foreign_cursor = page["next_cursor"]
        .as_str()
        .expect("two hits at one per page must offer a second page")
        .to_string();
    let response = get_product_message_search(
        &app,
        &second_id,
        &format!("q={term}&cursor={foreign_cursor}"),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a cursor minted in another session must be refused, not answered with a silent page"
    );
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_invalid_input");

    // Authorization is the same gate the message listing uses: this API's
    // product catalog is API-global, so the session id is the capability and
    // search must be neither weaker nor stricter than the ledger read it
    // projects. A session that has been removed is not found by both.
    let removed =
        create_product_session(&app, first_workspace["id"].as_str().unwrap(), "Removed").await;
    let removed_id = removed["id"].as_str().unwrap().to_string();
    delete_product_session_through_the_route(&app, &removed_id).await;
    let listing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{removed_id}/messages"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listing.status(), StatusCode::NOT_FOUND);
    let response = get_product_message_search(&app, &removed_id, &format!("q={term}")).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a removed session is not found, never an empty hit list"
    );
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_not_found");
}

/// Delete one product session through the public route, so the cascade the
/// store performs is the one a client can trigger.
async fn delete_product_session_through_the_route(app: &axum::Router, product_session_id: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/product/sessions/{product_session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// GET the unified search endpoint with an already-encoded query string.
async fn get_product_search(app: &axum::Router, query: &str) -> axum::response::Response {
    get_response(app, &format!("/product/search?{query}")).await
}

/// Build a session whose turn is held open and whose model is configured.
///
/// Every scope test needs a session that can accept queued messages; this is the
/// same four calls the step-one search tests make, in the same order.
async fn product_session_with_a_held_turn(
    app: &axum::Router,
    workspace_id: &str,
    title: &str,
) -> String {
    let session = create_product_session(app, workspace_id, title).await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(app, &session_id, "fake-raw", 2).await;
    hold_product_session_turn_open(app, &session_id).await;
    session_id
}

#[tokio::test]
async fn product_search_covers_a_whole_workspace_and_pages_without_gaps_or_repeats() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let other_folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let other_workspace = create_product_workspace(&app, other_folder.path()).await;
    let other_workspace_id = other_workspace["id"].as_str().unwrap().to_string();

    let mut written: Vec<(String, i64)> = Vec::new();
    for (index, title) in ["First", "Second", "Third"].iter().enumerate() {
        let session_id = product_session_with_a_held_turn(&app, &workspace_id, title).await;
        for hit in 0..=index {
            let message = queue_message(
                &app,
                &session_id,
                &format!("workspace-wide contract {index} {hit}"),
                &format!("scope-{index}-{hit}"),
            )
            .await;
            written.push((session_id.clone(), message["seq"].as_i64().unwrap()));
        }
        queue_message(
            &app,
            &session_id,
            "an unrelated note in this session",
            &format!("scope-note-{index}"),
        )
        .await;
    }
    // A session in another workspace carries the same term and must stay out:
    // the workspace is the boundary of the query, not a detail of the results.
    let outsider = product_session_with_a_held_turn(&app, &other_workspace_id, "Outsider").await;
    queue_message(
        &app,
        &outsider,
        "workspace-wide contract outsider",
        "scope-outsider",
    )
    .await;

    let term = encode_query_value("workspace-wide contract");
    let scope = encode_query_value(&format!("workspace:{workspace_id}"));
    let mut seen: Vec<(String, i64)> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut query = format!("q={term}&scope={scope}&limit=2");
        if let Some(cursor) = &cursor {
            query.push_str(&format!("&cursor={cursor}"));
        }
        let response = get_product_search(&app, &query).await;
        assert_eq!(response.status(), StatusCode::OK, "{query}");
        let body: serde_json::Value = decode_json(response).await;
        assert_eq!(
            body["scope"],
            format!("workspace:{workspace_id}"),
            "the response must name the scope it answered: {body}"
        );
        let hits = body["hits"].as_array().unwrap();
        assert!(hits.len() <= 2, "a page stays inside its limit: {body}");
        for hit in hits {
            assert_eq!(hit["source"], "message", "{body}");
            assert!(
                hit.get("run_id").is_none() && hit.get("run_ordinal").is_none(),
                "a message hit names no run: {body}"
            );
            seen.push((
                hit["session_id"].as_str().unwrap().to_string(),
                hit["seq"].as_i64().unwrap(),
            ));
        }
        pages += 1;
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
        assert!(pages < 10, "pagination must terminate");
    }

    assert_eq!(
        pages, 3,
        "six hits at two per page is three pages: {seen:?}"
    );
    assert!(
        seen.iter().all(|(session, _)| *session != outsider),
        "a workspace scope must never leave its workspace: {seen:?}"
    );
    let mut sorted = seen.clone();
    sorted.sort();
    let mut expected = written.clone();
    expected.sort();
    assert_eq!(
        sorted, expected,
        "the walk must return every hit exactly once"
    );
    assert!(
        seen.windows(2).all(|window| window[0] <= window[1]),
        "the page order is the cursor key, so it must never go backwards: {seen:?}"
    );

    // One page answers "which sessions contain this term", which is the question
    // a per-session search cannot answer at all.
    let response = get_product_search(&app, &format!("q={term}&scope={scope}&limit=100")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let sessions: std::collections::BTreeSet<String> = body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["session_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(sessions.len(), 3, "all three sessions must be reported");
    assert!(!sessions.contains(&outsider));

    // A term no session in the workspace carries is an empty page with no
    // cursor, never an error and never a cursor into nothing.
    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={scope}",
            encode_query_value("nothing in this workspace carries this")
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert!(body["hits"].as_array().unwrap().is_empty());
    assert!(body.get("next_cursor").is_none());
}

#[tokio::test]
async fn product_search_rejects_every_malformed_request_with_a_typed_failure() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session_id = product_session_with_a_held_turn(&app, &workspace_id, "Malformed").await;
    queue_message(&app, &session_id, "a contract in the ledger", "invalid-1").await;
    // A second matching message, so the workspace scope has a page to resume.
    queue_message(&app, &session_id, "another contract here", "invalid-2").await;
    // A session with no run and no message, for the scopes that must answer an
    // empty page rather than a failure.
    let quiet = create_product_session(&app, &workspace_id, "Quiet").await;
    let quiet_id = quiet["id"].as_str().unwrap().to_string();

    let session_scope = encode_query_value(&format!("session:{session_id}"));
    let workspace_scope = encode_query_value(&format!("workspace:{workspace_id}"));
    let unknown_workspace = encode_query_value(&format!("workspace:{}", ProductWorkspaceId::new()));
    let unknown_session = encode_query_value(&format!("session:{}", ProductSessionId::new()));
    let unknown_trace = encode_query_value(&format!("trace:{}", ProductSessionId::new()));
    let term = encode_query_value("contract");

    for (query, expected, note) in [
        // A missing required parameter reaches the handler as an extractor
        // rejection, which must still carry the documented typed envelope.
        (
            format!("scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "missing q",
        ),
        (
            format!("q={term}"),
            StatusCode::BAD_REQUEST,
            "missing scope",
        ),
        (
            format!("q=&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "empty q",
        ),
        (
            format!("q=%20%20&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "blank q",
        ),
        (
            format!(
                "q={}&scope={session_scope}",
                encode_query_value(&"x".repeat(129))
            ),
            StatusCode::BAD_REQUEST,
            "oversized q",
        ),
        (
            // Percent-decoding replaces `%FF` with U+FFFD instead of failing, so
            // this reaches the handler's unrepresentable-character check — but
            // only when every other required field is present: a missing `scope`
            // is refused by the extractor first, and the case would then prove
            // nothing about the term it claims to test.
            format!("q=%FF&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "q is not utf8",
        ),
        (
            format!("q={term}&limit=abc&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "non-numeric limit",
        ),
        (
            format!("q={term}&limit=0&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "zero limit",
        ),
        (
            format!("q={term}&limit=101&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "limit above the cap",
        ),
        (
            format!("q={term}&scope="),
            StatusCode::BAD_REQUEST,
            "empty scope",
        ),
        (
            format!("q={term}&scope=workspace"),
            StatusCode::BAD_REQUEST,
            "scope without an id",
        ),
        (
            format!("q={term}&scope=workspace:"),
            StatusCode::BAD_REQUEST,
            "scope with an empty id",
        ),
        (
            format!("q={term}&scope=unknown:{session_id}"),
            StatusCode::BAD_REQUEST,
            "unknown scope kind",
        ),
        (
            format!("q={term}&scope=workspace:not-a-ulid"),
            StatusCode::BAD_REQUEST,
            "unparsable scope id",
        ),
        (
            format!("q={term}&cursor=not-base64!&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "cursor is not a cursor",
        ),
        (
            format!("q={term}&cursor=e30&scope={session_scope}"),
            StatusCode::BAD_REQUEST,
            "cursor holds no position",
        ),
        (
            format!("q={term}&scope={unknown_workspace}"),
            StatusCode::NOT_FOUND,
            "unknown workspace",
        ),
        (
            format!("q={term}&scope={unknown_session}"),
            StatusCode::NOT_FOUND,
            "unknown session",
        ),
        (
            format!("q={term}&scope={unknown_trace}"),
            StatusCode::NOT_FOUND,
            "unknown trace session",
        ),
        // A workspace cannot answer a term below the trigram floor without
        // leaving the workspace; the session scope still can.
        (
            format!("q=ab&scope={workspace_scope}"),
            StatusCode::BAD_REQUEST,
            "short term across a workspace",
        ),
    ] {
        let response = get_product_search(&app, &query).await;
        assert_eq!(
            response.status(),
            expected,
            "`{query}` ({note}) should have been rejected"
        );
        let error: serde_json::Value = decode_json(response).await;
        let code = if expected == StatusCode::NOT_FOUND {
            "product_not_found"
        } else {
            "product_invalid_input"
        };
        assert_eq!(error["code"], code, "for `{query}` ({note})");
    }

    // The same short term is served by the session scope, so the refusal above
    // is about the workspace's cost and not about the term.
    let response = get_product_search(&app, &format!("q=ab&scope={session_scope}")).await;
    assert_eq!(response.status(), StatusCode::OK);

    // A cursor belongs to the scope and the term that minted it. The session
    // below holds the same term, so a token minted for the workspace would find
    // valid-looking rows there; answering it would be a page from the wrong
    // corpus rather than an error.
    let response =
        get_product_search(&app, &format!("q={term}&scope={workspace_scope}&limit=1")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = decode_json(response).await;
    let workspace_cursor = page["next_cursor"]
        .as_str()
        .expect("the workspace page must be resumable")
        .to_string();
    for (term, scope, note) in [
        (
            term.clone(),
            session_scope.clone(),
            "a workspace cursor on a session scope",
        ),
        (
            encode_query_value("nothing"),
            workspace_scope.clone(),
            "a workspace cursor with a different term",
        ),
    ] {
        let response = get_product_search(
            &app,
            &format!("q={term}&scope={scope}&cursor={workspace_cursor}"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{note} must be refused, not answered with a page"
        );
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_invalid_input", "for {note}");
    }

    // A session that exists but has nothing to match is an empty page in every
    // corpus, not a failure: "no results" and "no such session" must stay
    // distinguishable.
    let quiet_scope = encode_query_value(&format!("session:{quiet_id}"));
    for scope in [
        quiet_scope.clone(),
        encode_query_value(&format!("trace:{quiet_id}")),
    ] {
        let response = get_product_search(&app, &format!("q={term}&scope={scope}")).await;
        assert_eq!(response.status(), StatusCode::OK, "{scope}");
        let body: serde_json::Value = decode_json(response).await;
        assert!(body["hits"].as_array().unwrap().is_empty(), "{body}");
        assert!(body.get("next_cursor").is_none(), "{body}");
    }
}

#[tokio::test]
async fn product_trace_search_finds_a_run_by_something_only_the_event_trace_carries() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Trace search").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;
    // The marker exists only as a tool argument. It is not in any message body,
    // so the ledger genuinely cannot answer this question.
    let marker = "trace-only-marker-4c1f";
    hold_product_session_turn_open_with_prompt(&app, &session_id, marker).await;
    queue_message(&app, &session_id, "an ordinary ledger note", "trace-1").await;

    let trace_scope = encode_query_value(&format!("trace:{session_id}"));
    let response = get_product_search(
        &app,
        &format!("q={}&scope={trace_scope}", encode_query_value(marker)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(
        body["scope"],
        format!("trace:{session_id}"),
        "the response must name the scope it answered"
    );
    let hits = body["hits"].as_array().unwrap();
    assert!(
        !hits.is_empty(),
        "the marker is in the trace, so the trace search must find it: {body}"
    );
    for hit in hits {
        assert_eq!(hit["source"], "trace", "{body}");
        assert_eq!(hit["session_id"], session_id, "{body}");
        assert_eq!(
            hit["run_ordinal"], 1,
            "the first binding is run one: {body}"
        );
        assert!(
            hit["run_id"].as_str().is_some_and(|id| !id.is_empty()),
            "a trace hit must name the run it came from: {body}"
        );
        assert!(
            hit["seq"].as_i64().is_some_and(|seq| seq >= 1),
            "a trace hit must carry the record's own sequence: {body}"
        );
        assert!(
            hit["snippet"].as_str().unwrap().contains(marker),
            "the snippet must show the hit: {body}"
        );
    }

    // The same term through the message scope finds nothing, which is the whole
    // reason the trace corpus had to be searchable.
    let message_scope = encode_query_value(&format!("session:{session_id}"));
    let response = get_product_search(
        &app,
        &format!("q={}&scope={message_scope}", encode_query_value(marker)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert!(
        body["hits"].as_array().unwrap().is_empty(),
        "the marker must exist only in the trace: {body}"
    );

    // A term nothing in the trace carries is an empty page, and a run that has
    // no match is not a failure.
    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={trace_scope}",
            encode_query_value("no event ever recorded this")
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    assert!(body["hits"].as_array().unwrap().is_empty(), "{body}");
    assert!(body.get("next_cursor").is_none(), "{body}");

    // Paging through the trace is bounded and resumable: the page cursor is a
    // byte position, so a second page never repeats the first page's records.
    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={trace_scope}&limit=1",
            encode_query_value("request_input")
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value = decode_json(response).await;
    assert!(first["hits"].as_array().unwrap().len() <= 1, "{first}");
    if let Some(cursor) = first["next_cursor"].as_str() {
        let response = get_product_search(
            &app,
            &format!(
                "q={}&scope={trace_scope}&limit=1&cursor={cursor}",
                encode_query_value("request_input")
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let second: serde_json::Value = decode_json(response).await;
        assert!(
            first["hits"]
                .as_array()
                .unwrap()
                .iter()
                .all(|hit| second["hits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|next| next["seq"] != hit["seq"])),
            "a resumed trace page must not repeat a record: {first} {second}"
        );
    }

    // A cursor minted for another scope or term is refused rather than answered
    // with a page from the wrong corpus, and one minted for a message position
    // cannot be read as a trace position at all.
    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={message_scope}&limit=1",
            encode_query_value("ordinary")
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ledger: serde_json::Value = decode_json(response).await;
    if let Some(cursor) = ledger["next_cursor"].as_str() {
        let response = get_product_search(
            &app,
            &format!(
                "q={}&scope={trace_scope}&cursor={cursor}",
                encode_query_value("ordinary")
            ),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a ledger cursor is not a trace position"
        );
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_invalid_input");
    }
}

#[tokio::test]
async fn product_trace_search_redacts_the_trace_line_before_excerpting_it() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    // A secret-shaped token that a tool argument carried into the event trace.
    // The canary is the one the evidence-export tests already use, so no fixture
    // gains a new secret-shaped string.
    let canary = "sk-export-content-canary-058761eb";
    let session = create_product_session(&app, workspace_id, "Trace secret").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;
    hold_product_session_turn_open_with_prompt(&app, &session_id, &format!("canary {canary}"))
        .await;

    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={}",
            encode_query_value("canary"),
            encode_query_value(&format!("trace:{session_id}"))
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "the trace line is still a hit: {body}");
    for hit in hits {
        let snippet = hit["snippet"].as_str().unwrap();
        assert!(
            !snippet.contains(canary),
            "a trace snippet must never carry a secret pattern: {snippet}"
        );
        assert!(
            snippet.contains("[REDACTED:secret_pattern]"),
            "the snippet must show that something was redacted: {snippet}"
        );
    }

    // The same snippet builder runs over a multi-byte trace line. `token=`
    // followed by a long CJK run makes the secret clamp land on a byte offset
    // inside a character; a clamp that sliced there would panic out of the
    // blocking store and turn a valid search into a 500.
    let multi_byte = create_product_session(&app, workspace_id, "Trace multi-byte").await;
    let multi_byte_id = multi_byte["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &multi_byte_id, "fake-raw", 2).await;
    let prompt = format!("搜索命中 token={}", "密码".repeat(300));
    hold_product_session_turn_open_with_prompt(&app, &multi_byte_id, &prompt).await;

    let response = get_product_search(
        &app,
        &format!(
            "q={}&scope={}",
            encode_query_value("搜索命中"),
            encode_query_value(&format!("trace:{multi_byte_id}"))
        ),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a multi-byte trace line must not turn a valid search into an error"
    );
    let body: serde_json::Value = decode_json(response).await;
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "{body}");
    for hit in hits {
        let snippet = hit["snippet"].as_str().unwrap();
        assert!(
            snippet.chars().count() <= 160,
            "a trace snippet stays inside its codepoint budget: {}",
            snippet.chars().count()
        );
        assert!(!snippet.contains('\u{FFFD}'), "{snippet}");
        assert!(
            snippet.contains("[REDACTED:secret_pattern]"),
            "the secret-shaped prefix must still be redacted: {snippet}"
        );
    }
}

#[tokio::test]
async fn product_trace_search_reports_a_cleaned_trace_instead_of_no_matches() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Cleaned trace").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 2).await;

    let marker = "cleaned-trace-canary";
    hold_product_session_turn_open_with_prompt(&app, &session_id, marker).await;
    let scope = encode_query_value(&format!("trace:{session_id}"));
    let query = format!("q={}&scope={scope}", encode_query_value(marker));

    let response = get_product_search(&app, &query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    let run_id = body["hits"][0]["run_id"]
        .as_str()
        .expect("the hit names its run")
        .to_string();

    // Artifact cleanup removes a run's persisted trace the way the manifest
    // reports as `cleaned`, and the run binding stays in the catalog. Its
    // content is then missing rather than empty: answering "no matches" for the
    // session would report a hole in the corpus as a fact about the term.
    let run_dir =
        find_run_dir(&folder.path().join("api-state"), &run_id).expect("the run directory");
    std::fs::remove_file(run_dir.join("trace.jsonl")).unwrap();

    let response = get_product_search(&app, &query).await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "a cleaned trace is content that is gone, not a session without matches"
    );
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_events_expired");
}

#[tokio::test]
async fn product_resume_reports_unavailable_when_the_catalog_profile_is_deleted() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.source_summary.user_config_path = server.path().join("user/config.toml");
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Deleted Provider resume").await;
    let session_id = session["id"].as_str().unwrap();
    let profile = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Snapshot fake",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake"
        }),
    )
    .await;
    assert_eq!(profile.status(), StatusCode::CREATED);
    let profile: serde_json::Value = decode_json(profile).await;
    let profile_id = profile["id"].as_str().unwrap();
    let initial = get_response(
        &app,
        &format!("/product/sessions/{session_id}/model-config"),
    )
    .await;
    let initial: serde_json::Value = decode_json(initial).await;
    let configured = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "profile_id": profile_id,
            "model": "fake",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);
    let first = create_product_job(&app, session_id, "freeze Provider snapshot").await;
    wait_for_done(app.clone(), first.job_id.to_string()).await;

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/provider-profiles/{profile_id}?expected_revision={}",
                    profile["catalog_revision"].as_str().unwrap()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let resumed = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must not silently select another Provider",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(resumed.status(), StatusCode::CONFLICT);
    let resumed: serde_json::Value = decode_json(resumed).await;
    assert_eq!(resumed["code"], "provider_unavailable_for_resume");
    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "needs_attention");
    assert_eq!(
        session["runtime_binding"]["latest_run_id"],
        first.run_id.to_string()
    );
}

#[tokio::test]
async fn product_resume_rejects_same_selection_after_provider_identity_drift() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.source_summary.user_config_path = server.path().join("user/config.toml");
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Changed Provider resume").await;
    let session_id = session["id"].as_str().unwrap();
    let profile = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Snapshot fake",
            "provider_type": "fake",
            "api_base": "",
            "default_model": "fake"
        }),
    )
    .await;
    assert_eq!(profile.status(), StatusCode::CREATED);
    let profile: serde_json::Value = decode_json(profile).await;
    let profile_id = profile["id"].as_str().unwrap();
    let initial = get_response(
        &app,
        &format!("/product/sessions/{session_id}/model-config"),
    )
    .await;
    let initial: serde_json::Value = decode_json(initial).await;
    let configured = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "profile_id": profile_id,
            "model": "fake",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);
    let first = create_product_job(&app, session_id, "freeze original Provider").await;
    wait_for_done(app.clone(), first.job_id.to_string()).await;

    let changed = request_json(
        &app,
        "PUT",
        &format!("/product/provider-profiles/{profile_id}"),
        serde_json::json!({
            "label": "Drifted local endpoint",
            "provider_type": "ollama",
            "api_base": "http://127.0.0.1:9",
            "default_model": "fake",
            "expected_revision": profile["catalog_revision"]
        }),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::OK);

    let resumed = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must reject identity drift before network activity",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(resumed.status(), StatusCode::CONFLICT);
    let resumed: serde_json::Value = decode_json(resumed).await;
    assert_eq!(resumed["code"], "provider_changed_for_resume");
    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "needs_attention");
    assert_eq!(
        session["runtime_binding"]["latest_run_id"],
        first.run_id.to_string()
    );
}

#[tokio::test]
async fn product_session_resume_rejects_a_mismatched_runtime_run_identity() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Corrupt run identity").await;
    let session_id = session["id"].as_str().unwrap();
    let first = create_product_job(&app, session_id, "durable first turn").await;
    wait_for_done(app.clone(), first.job_id.to_string()).await;

    let connection = rusqlite::Connection::open(folder.path().join(".rove/state.sqlite")).unwrap();
    let mismatched_session_id = SessionId::new().to_string();
    connection
        .execute(
            "INSERT INTO sessions(session_id, created_at, updated_at) VALUES (?1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            [&mismatched_session_id],
        )
        .unwrap();
    let updated = connection
        .execute(
            "UPDATE runs SET session_id = ?2 WHERE run_id = ?1",
            rusqlite::params![first.run_id.to_string(), mismatched_session_id],
        )
        .unwrap();
    assert_eq!(updated, 1);
    drop(connection);

    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must reject mismatched indexed run identity",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_session_runtime_state_corrupt");

    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "needs_attention");
    assert_eq!(
        session["runtime_binding"]["latest_run_id"],
        first.run_id.to_string()
    );
}

#[tokio::test]
async fn product_session_resume_rejects_invalid_native_tool_call_ids() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Corrupt tool history").await;
    let session_id = session["id"].as_str().unwrap();
    let first = create_product_job(&app, session_id, "durable first turn").await;
    wait_for_done(app.clone(), first.job_id.to_string()).await;

    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let mut task_state = state_store.load_task_state(first.run_id).await.unwrap();
    task_state
        .checkpoint
        .as_mut()
        .expect("checkpoint")
        .preserved_tail
        .push(Message::assistant_with_tool_calls(
            "invalid duplicate native calls",
            vec![
                ToolCallRef {
                    id: "duplicate-call".to_string(),
                    name: "first_tool".to_string(),
                    args: serde_json::json!({}),
                },
                ToolCallRef {
                    id: "duplicate-call".to_string(),
                    name: "second_tool".to_string(),
                    args: serde_json::json!({}),
                },
            ],
        ));
    state_store.write_task_state(&task_state).await.unwrap();

    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must reject invalid provider history",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_session_runtime_state_corrupt");

    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "needs_attention");
    assert_eq!(
        session["runtime_binding"]["latest_run_id"],
        first.run_id.to_string()
    );
}

#[tokio::test]
async fn product_preflight_failure_preserves_the_claimed_session_error_status() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let other = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Preserve error").await;
    let session_id = session["id"].as_str().unwrap();
    let product_database = server.path().join("api-state/product.sqlite");
    let connection = rusqlite::Connection::open(product_database).unwrap();
    connection
        .execute(
            "UPDATE product_sessions SET status = 'error' WHERE product_session_id = ?1",
            [session_id],
        )
        .unwrap();
    drop(connection);

    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "wrong workspace",
            "product_session_id": session_id,
            "workspace": { "kind": "folder", "root": other.path() }
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(error["code"], "product_session_workspace_mismatch");

    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "error");
    assert!(session["runtime_binding"].is_null());
}

#[tokio::test]
async fn product_job_state_omits_the_salvage_marker_when_no_text_was_kept() {
    // A stop that produced nothing is not a salvaged partial. The additive
    // marker must therefore be absent from the response, not sent as `false`,
    // so a run without one keeps the exact bytes it had before the field
    // existed.
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Salvage marker").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;
    let waiting_message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "continue?" }
    })
    .to_string();
    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": waiting_message,
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    wait_for_pending_input(app.clone(), active.job_id.to_string()).await;

    let cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", active.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancelled = raw_body(cancel).await;
    assert!(
        !cancelled.contains("answer_aborted"),
        "an unmarked stop must not grow the cancel response: {cancelled}"
    );
    let decoded: JobStateResponse = serde_json::from_str(&cancelled).unwrap();
    assert_eq!(decoded.status, RunStatus::Cancelled);
    assert!(
        !decoded.answer_aborted,
        "a cancelled run without a kept partial reports no marker"
    );

    // The parameterless state read and the transcript agree with it.
    let state = get_response(&app, &format!("/jobs/{}/state", active.job_id)).await;
    assert_eq!(state.status(), StatusCode::OK);
    let state = raw_body(state).await;
    assert!(
        state.contains(r#""events""#) && !state.contains("answer_aborted"),
        "GET /jobs/{{job_id}}/state keeps its previous bytes: {state}"
    );

    let transcript =
        get_response(&app, &format!("/product/sessions/{session_id}/transcript")).await;
    assert_eq!(transcript.status(), StatusCode::OK);
    let transcript = raw_body(transcript).await;
    assert!(
        transcript.contains(r#""segments""#) && !transcript.contains("answer_aborted"),
        "the parameterless transcript keeps its previous bytes: {transcript}"
    );
}

#[tokio::test]
async fn product_cancel_after_streaming_marks_job_state_and_the_transcript() {
    // A run stopped while its model turn was still streaming is the case the
    // whole marker exists for, so it is driven through the real HTTP surface —
    // start, stop, read back — and not only through the projection unit tests.
    //
    // The local fake profile holds its turn open, so "the text is on screen and
    // the turn has not settled" is decided by the stop rather than by the clock:
    // a held request can only end when the abort-salvage window expires.
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    // A selected provider profile is the only route that reaches the factory on
    // a product turn, so the script is declared where a real profile would
    // declare it: in the user configuration the provider catalog loads. The
    // first turn answers the planner (the step budget plans by default) and the
    // second streams the answer and then stays in flight.
    let user_config = server.path().join("user/config.toml");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(
        &user_config,
        concat!(
            "schema_version = 1\n",
            "[provider.profiles.scripted]\n",
            "provider_type = \"fake\"\n",
            "base_url = \"\"\n",
            "model = \"scripted-fake\"\n",
            "protocol_options = { turns = [",
            "{ text = '{\"goal\":\"answer this\",\"steps\":[{\"id\":\"1\",",
            "\"title\":\"answer the request\"}]}' }, ",
            "{ hold = \"half an answer\" }] }\n",
        ),
    )
    .unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.source_summary.user_config_path = user_config;
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Streaming stop").await;
    let session_id = session["id"].as_str().unwrap();
    select_product_session_profile(&app, session_id, "scripted", "scripted-fake").await;

    let created = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "answer this",
            "product_session_id": session_id
        }),
    )
    .await;
    let created_status = created.status();
    let created_body = raw_body(created).await;
    assert_eq!(created_status, StatusCode::OK, "{created_body}");
    let created: CreateJobResponse = serde_json::from_str(&created_body).unwrap();

    let streaming =
        wait_for_streamed_text(app.clone(), created.job_id.to_string(), "half an answer").await;
    assert_eq!(
        streaming.status,
        RunStatus::Running,
        "the held turn must still be in flight"
    );
    // While a run is in flight its listing is only the published prefix, which
    // is exactly why the field documents the absent marker as inconclusive here.
    let in_flight = get_response(&app, &format!("/jobs/{}/state", created.job_id)).await;
    assert_eq!(in_flight.status(), StatusCode::OK);
    let in_flight = raw_body(in_flight).await;
    assert!(
        !in_flight.contains("answer_aborted"),
        "an in-flight run has published no salvage marker yet: {in_flight}"
    );

    let cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);

    let cancelled = wait_for_status(
        app.clone(),
        created.job_id.to_string(),
        RunStatus::Cancelled,
    )
    .await;
    assert!(
        cancelled.answer_aborted,
        "a stop that kept the text the user had already seen must mark the run"
    );
    // The live stream and the durable answer agree within the same run: one
    // salvaged message, marked, carrying the held text.
    let salvaged = cancelled
        .events
        .iter()
        .filter_map(|event| match &event.event {
            StreamEvent::LlmMessage {
                full,
                aborted: true,
                ..
            } => Some(full.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        salvaged,
        vec!["half an answer".to_string()],
        "{cancelled:?}"
    );

    let state = get_response(&app, &format!("/jobs/{}/state", created.job_id)).await;
    let state = raw_body(state).await;
    assert!(
        state.contains(r#""status":"cancelled""#) && state.contains(r#""answer_aborted":true"#),
        "the settled job state must carry the marker on the wire: {state}"
    );

    // The restored-session surface reads the same run: its segment must carry
    // the marker too, or the two response contracts would disagree.
    let transcript =
        get_response(&app, &format!("/product/sessions/{session_id}/transcript")).await;
    assert_eq!(transcript.status(), StatusCode::OK);
    let transcript: serde_json::Value = decode_json(transcript).await;
    assert_eq!(
        transcript["status"], "complete",
        "the segment list is complete, so an absent marker would be conclusive: {transcript}"
    );
    let segment = transcript["segments"]
        .as_array()
        .expect("segments are an array")
        .iter()
        .find(|segment| segment["binding"]["runtime_run_id"] == created.run_id.to_string())
        .unwrap_or_else(|| panic!("the stopped run needs a transcript segment: {transcript}"));
    assert_eq!(
        segment["answer_aborted"],
        serde_json::json!(true),
        "the transcript segment must mark the same salvaged partial: {transcript}"
    );
}

#[tokio::test]
async fn openapi_documents_the_abort_salvage_marker() {
    // The marker is part of the documented contract, not only of the opaque
    // event payload: a client can read it off `JobStateResponse` and a
    // transcript segment, and both document what an absence does and does not
    // settle.
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let response = get_response(&app, "/api/openapi.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    let spec: serde_json::Value = serde_json::from_str(&raw_body(response).await).unwrap();

    for schema in ["JobStateResponse", "ProductTranscriptRunSegment"] {
        let field = &spec["components"]["schemas"][schema]["properties"]["answer_aborted"];
        assert_eq!(
            field["type"], "boolean",
            "{schema}.answer_aborted must be a documented boolean: {field}"
        );
        let description = field["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{schema}.answer_aborted needs a description: {field}"));
        assert!(
            description.contains("salvaged partial") && description.contains("omitted"),
            "{schema}.answer_aborted must document what its absence means: {description}"
        );
        let required = spec["components"]["schemas"][schema]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            !required.iter().any(|name| name == "answer_aborted"),
            "{schema}.answer_aborted is additive and must stay optional: {required:?}"
        );
    }

    // The absence rule is the part a client has to get right, so the job-state
    // document must state its precondition instead of calling the omission
    // conclusive: a listing only settles a run that is terminal and complete.
    let description = &spec["components"]["schemas"]["JobStateResponse"]["properties"]["answer_aborted"]
        ["description"];
    let description = description.as_str().unwrap_or_default();
    assert!(
        description.contains("terminal") && description.contains("contiguous"),
        "JobStateResponse.answer_aborted must state when its absence is conclusive: {description}"
    );
}

#[tokio::test]
async fn product_cancel_releases_the_single_turn_claim_before_continuation() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let other = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Cancel flow").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;
    let waiting_message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "continue?" }
    })
    .to_string();
    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": waiting_message,
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    wait_for_pending_input(app.clone(), active.job_id.to_string()).await;

    let conflict = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "concurrent turn",
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict: serde_json::Value = decode_json(conflict).await;
    assert_eq!(conflict["code"], "product_session_active");

    let cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", active.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancelled: JobStateResponse = decode_json(cancel).await;
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert_eq!(
        cancelled
            .events
            .iter()
            .filter(|event| matches!(event.event, StreamEvent::RunCompleted { .. }))
            .count(),
        1
    );

    let mismatch = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "wrong workspace",
            "product_session_id": session_id,
            "workspace": { "kind": "folder", "root": other.path() }
        }),
    )
    .await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    let mismatch: serde_json::Value = decode_json(mismatch).await;
    assert_eq!(mismatch["code"], "product_session_workspace_mismatch");
    let cancelled_session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(cancelled_session["status"], "idle");
    // Cancelling is a decision, not a failure: the session reports the outcome
    // the user chose so the sidebar can distinguish it from an error.
    assert_eq!(cancelled_session["last_outcome"], "cancelled");
    assert!(
        cancelled_session["last_outcome_at"].is_string(),
        "a recorded outcome carries when it happened: {cancelled_session}"
    );

    let resumed = create_product_job(&app, session_id, "after cancellation").await;
    assert_eq!(resumed.job_id, active.job_id);
    assert_eq!(resumed.resumed_from_run_id, Some(active.run_id));
    let resumed_state = wait_for_done(app, resumed.job_id.to_string()).await;
    assert_eq!(resumed_state.status, RunStatus::Done);
    assert!(
        resumed_state.events.iter().any(|event| matches!(
            &event.event,
            StreamEvent::LlmMessage { full, .. } if full == "after cancellation"
        )),
        "a cancelled product turn must not replay its terminal plan decision"
    );
}

#[tokio::test]
async fn product_steer_route_is_idempotent_and_applies_after_an_input_safe_point() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Steer safe point").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "continue?" }
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let first = post_json(
        &app,
        &format!("/product/sessions/{session_id}/steers"),
        serde_json::json!({
            "content": "Prioritize the release notes after the input.",
            "idempotency_key": "steer-safe-point"
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first: serde_json::Value = decode_json(first).await;
    assert_eq!(first["status"], "pending");

    let replay = post_json(
        &app,
        &format!("/product/sessions/{session_id}/steers"),
        serde_json::json!({
            "content": "Prioritize the release notes after the input.",
            "idempotency_key": "steer-safe-point"
        }),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay: serde_json::Value = decode_json(replay).await;
    assert_eq!(replay["id"], first["id"]);
    assert_eq!(replay["status"], "pending");

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": "Continue with the release notes." }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);

    let control =
        wait_for_product_control_status(&app, session_id, first["id"].as_str().unwrap(), "applied")
            .await;
    let active_run_id = active.run_id.to_string();
    assert_eq!(control["run_id"].as_str(), Some(active_run_id.as_str()));

    let state = wait_for_done(app.clone(), active.job_id.to_string()).await;
    assert!(state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::SteerAccepted { id, .. } if id == first["id"].as_str().unwrap()
        )
    }));
    assert!(state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::SteerApplied { id } if id == first["id"].as_str().unwrap()
        )
    }));
}

#[tokio::test]
async fn product_steer_submitted_during_generation_applies_after_the_tool_safe_point() {
    let provider = start_delayed_tool_openai_server().await;
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Generation steer").await;
    let session_id = session["id"].as_str().unwrap();
    let key_env = unique_env_key("ROVE_TEST_GENERATION_STEER_KEY");
    unsafe {
        std::env::set_var(&key_env, "generation-steer-token");
    }

    let profile = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Delayed generation provider",
            "provider_type": "openai",
            "api_base": format!("{}/v1", provider.base_url),
            "api_key_env": key_env,
            "default_model": "delayed-tool-model"
        }),
    )
    .await;
    assert_eq!(profile.status(), StatusCode::CREATED);
    let profile: serde_json::Value = decode_json(profile).await;
    let profile_id = profile["id"].as_str().unwrap();

    let initial = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/model-config"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let initial: serde_json::Value = decode_json(initial).await;
    let configured = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "profile_id": profile_id,
            "model": "delayed-tool-model",
            "reasoning": "default",
            "max_steps": 2,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);

    let provider_trust = request_json(
        &app,
        "PUT",
        &format!("/product/workspaces/{workspace_id}/trust"),
        serde_json::json!({
            "decision": "grant",
            "capabilities": ["provider_credentials"]
        }),
    )
    .await;
    assert_eq!(provider_trust.status(), StatusCode::OK);

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "Call echo before the final answer.",
            "product_session_id": session_id
        }),
    )
    .await;
    unsafe {
        std::env::remove_var(&key_env);
    }
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        provider.first_generation_started.notified(),
    )
    .await
    .expect("the first provider generation should start");

    let steer_body = serde_json::json!({
        "content": "Include the generation-time correction.",
        "idempotency_key": "generation-safe-point"
    });
    let first = post_json(
        &app,
        &format!("/product/sessions/{session_id}/steers"),
        steer_body.clone(),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first: serde_json::Value = decode_json(first).await;
    let replay = post_json(
        &app,
        &format!("/product/sessions/{session_id}/steers"),
        steer_body,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay: serde_json::Value = decode_json(replay).await;
    assert_eq!(replay["id"], first["id"]);

    let state = wait_for_done(app.clone(), active.job_id.to_string()).await;
    let controls = list_product_controls(&app, session_id).await;
    let control = controls
        .iter()
        .find(|control| control["id"] == first["id"])
        .expect("generation steer control");
    let request_count = provider.requests.lock().unwrap().len();
    assert_eq!(
        control["status"], "applied",
        "generation steer was not applied; requests={request_count}; events={:?}",
        state.events
    );
    assert_eq!(control["run_id"], active.run_id.to_string());
    assert!(state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::SteerAccepted { id, .. } if id == first["id"].as_str().unwrap()
        )
    }));
    assert!(state.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::SteerApplied { id } if id == first["id"].as_str().unwrap()
        )
    }));

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the tool result must trigger a second model turn"
    );
    assert!(
        requests[1]
            .to_string()
            .contains("Include the generation-time correction."),
        "the second provider request must contain the steer accepted after the tool safe point"
    );
}

#[tokio::test]
async fn product_followup_after_final_is_server_owned_and_starts_one_successor() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Follow-up ownership").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "finish the first turn" }
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let queued = post_json(
        &app,
        &format!("/product/sessions/{session_id}/followups"),
        serde_json::json!({
            "content": "Run the server-owned follow-up.",
            "idempotency_key": "final-follow-up"
        }),
    )
    .await;
    assert_eq!(queued.status(), StatusCode::CREATED);
    let queued: serde_json::Value = decode_json(queued).await;
    assert_eq!(queued["status"], "pending");

    let replay = post_json(
        &app,
        &format!("/product/sessions/{session_id}/followups"),
        serde_json::json!({
            "content": "Run the server-owned follow-up.",
            "idempotency_key": "final-follow-up"
        }),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay: serde_json::Value = decode_json(replay).await;
    assert_eq!(replay["id"], queued["id"]);

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": "The first turn is complete." }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);

    let applied = wait_for_product_control_status(
        &app,
        session_id,
        queued["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    let successor_run_id = applied["run_id"].as_str().unwrap().to_string();
    assert_ne!(successor_run_id, active.run_id.to_string());

    let finished = wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    assert_eq!(finished["runtime_binding"]["ordinal"], 2);
    assert_eq!(
        finished["last_outcome"], "success",
        "a final answer is the only success this contract records"
    );
    assert_eq!(
        finished["runtime_binding"]["latest_run_id"],
        successor_run_id
    );
    let successor_job_id = finished["runtime_binding"]["latest_job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let successor = wait_for_done(app.clone(), successor_job_id).await;
    assert_eq!(successor.run_id.to_string(), successor_run_id);
    assert_eq!(successor.resumed_from_run_id, Some(active.run_id));
    assert!(successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::FollowupDequeued { id } if id == queued["id"].as_str().unwrap()
        )
    }));
    assert!(successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::LlmMessage { full, .. } if full == "Run the server-owned follow-up."
        )
    }));

    let controls = list_product_controls(&app, session_id).await;
    assert_eq!(
        controls.len(),
        1,
        "idempotency must not start a second turn"
    );
    let first_trace = std::fs::read_to_string(
        folder
            .path()
            .join("api-state")
            .join("runs")
            .join(active.run_id.to_string())
            .join("trace.jsonl"),
    )
    .unwrap();
    assert!(first_trace.contains("\"type\":\"followup_queued\""));
}

/// Read the unified message page for a session. The route serves the ledger in
/// `seq` order, so `queue_order` is what tests assert queue position from.
async fn list_product_messages(
    app: &axum::Router,
    product_session_id: &str,
) -> Vec<serde_json::Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{product_session_id}/messages?limit=128"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    body["messages"].as_array().unwrap().clone()
}

/// The session's successor queue in delivery order. A row with no
/// `queue_order` is delivered by its ledger sequence, just like the store's
/// `COALESCE(queue_order, seq)` ordering.
fn queued_message_ids(messages: &[serde_json::Value]) -> Vec<String> {
    let mut queued: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|message| message["status"] == "queued")
        .collect();
    queued.sort_by_key(|message| {
        (
            message["queue_order"]
                .as_i64()
                .unwrap_or_else(|| message["seq"].as_i64().unwrap()),
            message["seq"].as_i64().unwrap(),
        )
    });
    queued
        .into_iter()
        .map(|message| message["id"].as_str().unwrap().to_string())
        .collect()
}

async fn queue_message(
    app: &axum::Router,
    product_session_id: &str,
    content: &str,
    key: &str,
) -> serde_json::Value {
    let response = post_json(
        app,
        &format!("/product/sessions/{product_session_id}/messages"),
        serde_json::json!({ "content": content, "idempotency_key": key }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let message: serde_json::Value = decode_json(response).await;
    assert_eq!(message["status"], "queued");
    message
}

#[tokio::test]
async fn product_queue_reorder_is_validated_atomic_and_drives_the_next_turns() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Queue reorder").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    // Hold the turn open so the messages below queue behind a live run.
    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "hold the turn open" }
            })
            .to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let first = queue_message(&app, session_id, "first queued", "queue-1").await;
    let second = queue_message(&app, session_id, "second queued", "queue-2").await;
    let third = queue_message(&app, session_id, "third queued", "queue-3").await;
    // Nothing has moved yet, so no row carries a rewritten queue position and
    // the queue is still creation-ordered.
    assert!(
        list_product_messages(&app, session_id)
            .await
            .iter()
            .all(|message| message.get("queue_order").is_none()),
        "an untouched queue position must be omitted from the projection"
    );

    let reorder_uri = format!("/product/sessions/{session_id}/messages/reorder");
    let wanted = serde_json::json!([third["id"], first["id"], second["id"]]);
    let reordered = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": wanted }),
    )
    .await;
    assert_eq!(reordered.status(), StatusCode::OK);
    let reordered: serde_json::Value = decode_json(reordered).await;
    let positions: Vec<i64> = reordered["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["queue_order"].as_i64().unwrap())
        .collect();
    assert_eq!(
        reordered["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            third["id"].as_str().unwrap(),
            first["id"].as_str().unwrap(),
            second["id"].as_str().unwrap()
        ]
    );
    assert_eq!(positions, vec![0, 1, 2]);
    assert_eq!(
        queued_message_ids(&list_product_messages(&app, session_id).await),
        vec![
            third["id"].as_str().unwrap().to_string(),
            first["id"].as_str().unwrap().to_string(),
            second["id"].as_str().unwrap().to_string()
        ],
        "the durable projection must read back in the requested order"
    );

    // A list that does not exactly cover the queue is refused, and the refusal
    // leaves the accepted order untouched.
    let short = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": [third["id"]] }),
    )
    .await;
    assert_eq!(short.status(), StatusCode::CONFLICT);
    let short: serde_json::Value = decode_json(short).await;
    assert_eq!(short["code"], "product_control_conflict");
    let duplicated = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": [third["id"], third["id"], first["id"]] }),
    )
    .await;
    assert_eq!(duplicated.status(), StatusCode::BAD_REQUEST);
    let duplicated: serde_json::Value = decode_json(duplicated).await;
    assert_eq!(duplicated["code"], "product_invalid_input");
    let unknown = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": ["01JZZZZZZZZZZZZZZZZZZZZZZZ", first["id"], second["id"]] }),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::CONFLICT);
    // A message from another session of the same workspace is not part of this
    // session's queue, so it is refused exactly like an unknown id.
    let sibling = create_product_session(&app, workspace_id, "Sibling session").await;
    let sibling_id = sibling["id"].as_str().unwrap();
    let foreign = queue_message(&app, sibling_id, "foreign queued", "foreign-1").await;
    let cross_session = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": [foreign["id"], first["id"], second["id"]] }),
    )
    .await;
    assert_eq!(cross_session.status(), StatusCode::CONFLICT);
    let cross_session: serde_json::Value = decode_json(cross_session).await;
    assert_eq!(cross_session["code"], "product_control_conflict");
    // A row that has left the queue makes a previously valid list stale. That
    // staleness is what serializes reorder against revoke/promote.
    let revoke = post_json(
        &app,
        &format!(
            "/product/sessions/{session_id}/messages/{}/revoke",
            third["id"].as_str().unwrap()
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::OK);
    let stale = post_json(
        &app,
        &reorder_uri,
        serde_json::json!({ "ordered_ids": wanted }),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale: serde_json::Value = decode_json(stale).await;
    assert_eq!(stale["code"], "product_control_conflict");
    assert_eq!(
        queued_message_ids(&list_product_messages(&app, session_id).await),
        vec![
            first["id"].as_str().unwrap().to_string(),
            second["id"].as_str().unwrap().to_string()
        ],
        "a refused reorder must not move anything"
    );

    // Finish the held turn: the boundary claims the queue head, which is now
    // the first queued message because the rewrite to the head was revoked.
    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": "the first turn is complete" }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);
    let applied =
        wait_for_product_control_status(&app, session_id, first["id"].as_str().unwrap(), "applied")
            .await;
    assert_ne!(
        applied["run_id"].as_str().unwrap(),
        active.run_id.to_string()
    );
    let applied_message = list_product_messages(&app, session_id)
        .await
        .into_iter()
        .find(|message| message["id"] == first["id"])
        .unwrap();
    assert_eq!(applied_message["status"], "claimed_successor");
    assert_eq!(
        applied_message["queue_order"], 1,
        "the claimed successor keeps the position it was moved to"
    );

    // The next boundary continues down the same order, so the second successor
    // resumes from the first successor's run rather than from the answered turn.
    let second_applied = wait_for_product_control_status(
        &app,
        session_id,
        second["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    assert_ne!(
        second_applied["run_id"].as_str().unwrap(),
        applied["run_id"].as_str().unwrap()
    );
    let finished = wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    assert_eq!(
        finished["runtime_binding"]["ordinal"], 3,
        "two queued successors must produce exactly two more runs"
    );
    assert_eq!(
        finished["runtime_binding"]["latest_run_id"],
        second_applied["run_id"].as_str().unwrap()
    );
    let last_successor = wait_for_done(
        app.clone(),
        finished["runtime_binding"]["latest_job_id"]
            .as_str()
            .unwrap()
            .to_string(),
    )
    .await;
    assert_eq!(
        last_successor.resumed_from_run_id,
        Some(applied["run_id"].as_str().unwrap().parse().unwrap())
    );
    // Each dispatch carries the content of the control it claimed, in the run
    // order the queue asked for.
    let dispatched = dispatched_user_messages(&app, session_id).await;
    assert!(
        dispatched.contains(&(
            applied["run_id"].as_str().unwrap().to_string(),
            "first queued".to_string()
        )),
        "dispatched: {dispatched:?}"
    );
    assert!(
        dispatched.contains(&(
            second_applied["run_id"].as_str().unwrap().to_string(),
            "second queued".to_string()
        )),
        "dispatched: {dispatched:?}"
    );
}

/// The user message each settled run was dispatched with, read from the
/// session-scoped canonical transcript. This is the run-scoped view a client
/// uses, and unlike the global run index it is durable for a product session.
async fn dispatched_user_messages(
    app: &axum::Router,
    product_session_id: &str,
) -> Vec<(String, String)> {
    let response = get_response(
        app,
        &format!("/product/sessions/{product_session_id}/transcript"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    body["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|segment| {
            let run_id = segment["binding"]["runtime_run_id"].as_str()?.to_string();
            let user_message = segment["events"]
                .as_array()?
                .iter()
                .find(|stored| stored["event"]["type"] == "run_started")
                .and_then(|stored| stored["event"]["user_message"].as_str())
                .unwrap_or_default()
                .to_string();
            Some((run_id, user_message))
        })
        .collect()
}

#[tokio::test]
async fn product_successor_promotion_never_steers_the_live_run_and_survives_revoke() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Successor promotion").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "keep running" }
            })
            .to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    let first = queue_message(&app, session_id, "first queued", "promote-1").await;
    let second = queue_message(&app, session_id, "second queued", "promote-2").await;

    // Ask for the later message next. It must move to the queue head without
    // becoming a steer for the run that is still waiting for input.
    let promoted = post_json(
        &app,
        &format!(
            "/product/sessions/{session_id}/messages/{}/promote",
            second["id"].as_str().unwrap()
        ),
        serde_json::json!({ "delivery": "successor" }),
    )
    .await;
    assert_eq!(promoted.status(), StatusCode::OK);
    let promoted: serde_json::Value = decode_json(promoted).await;
    assert_eq!(promoted["status"], "queued");
    assert_eq!(promoted["requested_delivery"], "successor");
    assert!(
        promoted.get("actual_delivery").is_none(),
        "a successor promotion is not a delivery: {promoted}"
    );
    assert_eq!(promoted["queue_order"], 0);
    assert_eq!(
        queued_message_ids(&list_product_messages(&app, session_id).await),
        vec![
            second["id"].as_str().unwrap().to_string(),
            first["id"].as_str().unwrap().to_string()
        ]
    );

    let still_waiting = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/state", active.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let still_waiting: JobStateResponse = decode_json(still_waiting).await;
    assert_eq!(still_waiting.status, RunStatus::Running);
    assert!(
        !still_waiting.pending_inputs.is_empty(),
        "the live turn must still be waiting for the user's answer"
    );
    assert!(
        !still_waiting.events.iter().any(|stored| matches!(
            &stored.event,
            StreamEvent::MessageInterventionRequested { id }
                if id == second["id"].as_str().unwrap()
        )),
        "a successor promotion must not steer the live run"
    );

    // Revoking during the wait still works, and the revoked row is skipped by
    // the boundary drain.
    let revoke = post_json(
        &app,
        &format!(
            "/product/sessions/{session_id}/messages/{}/revoke",
            first["id"].as_str().unwrap()
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::OK);
    let revoke: serde_json::Value = decode_json(revoke).await;
    assert_eq!(revoke["status"], "revoked");

    let answer = post_json(
        &app,
        &format!("/jobs/{}/inputs/{input_id}", active.job_id),
        serde_json::json!({ "answer": "the first turn is complete" }),
    )
    .await;
    assert_eq!(answer.status(), StatusCode::OK);

    let applied = wait_for_product_control_status(
        &app,
        session_id,
        second["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    let successor_run = applied["run_id"].as_str().unwrap().to_string();
    let idle = wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    assert_eq!(idle["runtime_binding"]["latest_run_id"], successor_run);
    let successor = wait_for_done(
        app.clone(),
        idle["runtime_binding"]["latest_job_id"]
            .as_str()
            .unwrap()
            .to_string(),
    )
    .await;
    assert!(successor.events.iter().any(|stored| matches!(
        &stored.event,
        StreamEvent::LlmMessage { full, .. } if full == "second queued"
    )));
    assert!(
        !successor.events.iter().any(|stored| matches!(
            &stored.event,
            StreamEvent::LlmMessage { full, .. } if full == "first queued"
        )),
        "a revoked successor must never become a turn"
    );
    let revoked =
        wait_for_product_control_status(&app, session_id, first["id"].as_str().unwrap(), "revoked")
            .await;
    assert!(revoked.get("run_id").is_none() || revoked["run_id"].is_null());
}

#[tokio::test]
async fn product_queue_order_survives_a_reopen_and_keeps_one_claim_per_session() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config.clone(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Queue survives restart").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "survive a reopen" }
            })
            .to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    let pending = wait_for_pending_input(app.clone(), active.job_id.to_string()).await;
    assert!(
        !pending.pending_inputs.is_empty(),
        "the durable turn must be waiting for input before it is interrupted"
    );

    let first = queue_message(&app, session_id, "queued first", "reopen-1").await;
    let second = queue_message(&app, session_id, "queued second", "reopen-2").await;
    let third = queue_message(&app, session_id, "queued third", "reopen-3").await;
    let reorder = post_json(
        &app,
        &format!("/product/sessions/{session_id}/messages/reorder"),
        serde_json::json!({ "ordered_ids": [third["id"], first["id"], second["id"]] }),
    )
    .await;
    assert_eq!(reorder.status(), StatusCode::OK);

    // A second API state over the same state directory is what the process
    // restart gate can express in-process: it reopens the store, replays the
    // schema migrations, and runs startup recovery, which is the code path the
    // design asks about. The first state's supervisors stay alive in-test, so
    // this proves durable order plus the honest interrupted-delivery recovery
    // rather than a cold process start; the drain order after a restart is
    // covered by the store-level recovery test.
    let reopened = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let reopened_messages = list_product_messages(&reopened, session_id).await;
    // The move survives the reopen: the persisted positions still describe the
    // requested delivery order, and `seq` still describes creation order.
    let mut reopened_order: Vec<(i64, String)> = reopened_messages
        .iter()
        .map(|message| {
            (
                message["queue_order"].as_i64().unwrap(),
                message["id"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    reopened_order.sort();
    assert_eq!(
        reopened_order,
        vec![
            (0, third["id"].as_str().unwrap().to_string()),
            (1, first["id"].as_str().unwrap().to_string()),
            (2, second["id"].as_str().unwrap().to_string()),
        ],
        "reopened messages: {reopened_messages:?}"
    );
    assert_eq!(
        reopened_messages
            .iter()
            .map(|message| message["seq"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    // The interrupted run's fate is unknown, so its undelivered successors are
    // reported instead of being silently replayed or dropped.
    for message in &reopened_messages {
        assert_eq!(message["status"], "needs_attention");
        assert_eq!(
            message["reason"], "API process stopped during follow-up delivery",
            "an interrupted delivery must say why it stopped"
        );
    }
    let reopened_session = get_product_session(&reopened, workspace_id, session_id).await;
    assert_eq!(reopened_session["status"], "needs_attention");
    assert_eq!(
        reopened_session["runtime_binding"]["latest_run_id"],
        active.run_id.to_string(),
        "recovery must keep the last bound run and must not dispatch a new one"
    );
    let controls = list_product_controls(&reopened, session_id).await;
    assert_eq!(
        controls.len(),
        3,
        "a restart must not replay queued messages: {controls:?}"
    );
}

/// Whether the durable product log has a fact that returned this session to
/// `idle`.
///
/// A session is created `idle`, so such a fact cannot be written by creation:
/// it can only exist after a successor claim moved the session to `running` and
/// a release moved it back. That makes it the honest observation of the
/// stranded state a failed start leaves behind, which the catalog read alone
/// cannot distinguish from "the drain has not run yet" — `pending` + `idle` is
/// what both look like. It is the same durable authority `GET
/// /product/events` projects, read directly so the test does not have to follow
/// a long-lived stream.
fn session_has_a_released_successor_claim(server_root: &Path, session_id: &str) -> bool {
    let Ok(connection) = rusqlite::Connection::open(server_root.join("api-state/product.sqlite"))
    else {
        return false;
    };
    // The API holds its own connections; a read that loses a race with one of
    // its short write transactions must retry, not fail the test.
    connection
        .busy_timeout(std::time::Duration::from_millis(5_000))
        .ok();
    connection
        .query_row(
            r#"
            SELECT COUNT(*) FROM product_events
            WHERE kind = 'session.status_changed' AND session_id = ?1
              AND summary LIKE '%"status":"idle"%'
            "#,
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false)
}

async fn wait_for_released_successor_claim(server_root: &Path, session_id: &str) {
    for _ in 0..STATE_WAIT_ATTEMPTS {
        if session_has_a_released_successor_claim(server_root, session_id) {
            return;
        }
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!(
        "session {session_id} never committed a released successor claim within {}ms",
        state_wait_budget().as_millis()
    );
}

/// Move a registered workspace root out of the way.
///
/// The queue record is durable and needs no workspace, but starting its
/// successor does. Removing the root therefore makes the next claim fail in a
/// non-Provider way — the failure `requeue_followup_turn` answers with `pending`
/// plus `idle`.
fn hide_workspace_root(folder: &tempfile::TempDir) -> PathBuf {
    let moved = folder.path().with_extension("unavailable");
    std::fs::rename(folder.path(), &moved).unwrap();
    moved
}

fn restore_workspace_root(folder: &tempfile::TempDir, moved: &Path) {
    std::fs::rename(moved, folder.path()).unwrap();
}

#[tokio::test]
async fn product_requeued_successor_is_redriven_without_another_user_action() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Requeued successor redrive").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let moved_root = hide_workspace_root(&folder);
    let queued = queue_message(&app, session_id, "redriven successor", "redrive-ok-1").await;
    let message_id = queued["id"].as_str().unwrap();

    // The idle-session send asks for a drain, that drain claims the successor,
    // and the claim's start fails because the workspace is gone.
    wait_for_released_successor_claim(server.path(), session_id).await;

    // The stranded state this test exists for, asserted before anything else
    // happens: one queued successor, an idle session, no run, and — without the
    // redrive — nothing left that would ever claim it.
    let stranded_message = list_product_messages(&app, session_id)
        .await
        .into_iter()
        .find(|message| message["id"] == message_id)
        .expect("product message");
    assert_eq!(stranded_message["status"], "queued");
    assert!(stranded_message["successor_run_id"].is_null());
    let stranded_session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(stranded_session["status"], "idle");
    assert!(stranded_session["runtime_binding"]["latest_run_id"].is_null());
    assert_eq!(
        list_product_controls(&app, session_id).await[0]["status"],
        "pending"
    );

    // Only the workspace comes back: no send, no confirm, no restart, no other
    // request that a drain call site hangs off. The re-drain the failed start
    // scheduled is the only thing that can start this successor.
    restore_workspace_root(&folder, &moved_root);

    let delivered = wait_for_delivered_message(&app, session_id, message_id).await;
    let successor_run_id = delivered["successor_run_id"].as_str().unwrap().to_string();
    let successor =
        wait_for_successor_run_done(&app, workspace_id, session_id, &successor_run_id).await;
    assert!(successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::MessageClaimedSuccessor { id } if id == message_id
        )
    }));
    assert!(successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::LlmMessage { full, .. } if full == "redriven successor"
        )
    }));
    let applied = list_product_messages(&app, session_id)
        .await
        .into_iter()
        .find(|message| message["id"] == message_id)
        .expect("product message");
    assert_eq!(applied["status"], "claimed_successor");
    assert_eq!(applied["successor_run_id"], successor_run_id);
    assert!(applied.get("reason").is_none() || applied["reason"].is_null());
}

#[tokio::test]
async fn product_successor_start_attempts_are_bounded_and_escalate_to_needs_attention() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Bounded successor redrive").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let moved_root = hide_workspace_root(&folder);
    let queued = queue_message(&app, session_id, "never startable", "redrive-bound-1").await;
    let message_id = queued["id"].as_str().unwrap();
    wait_for_released_successor_claim(server.path(), session_id).await;

    // The failure persists, so the start attempts must run out — one initial
    // claim plus three re-drains, each after its own backoff (1s, 2s, 4s) — and
    // the message must stop looking queued. The budget here is deliberately
    // generous: the wait is on the implemented schedule, not on the assertion.
    wait_for_product_control_status_within(
        &app,
        session_id,
        message_id,
        "abandoned",
        8 * STATE_WAIT_ATTEMPTS,
    )
    .await;

    let session = get_product_session(&app, workspace_id, session_id).await;
    assert_eq!(session["status"], "needs_attention");
    assert!(
        session["runtime_binding"]["latest_run_id"].is_null(),
        "a successor that could never start must not report a run: {session}"
    );

    let message = list_product_messages(&app, session_id)
        .await
        .into_iter()
        .find(|message| message["id"] == message_id)
        .expect("product message");
    assert_eq!(message["status"], "needs_attention");
    assert!(message["successor_run_id"].is_null());
    let reason = message["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("an escalated successor must say why: {message}"));
    assert!(
        reason.contains("successor start failed 4 times"),
        "the escalation must state the exhausted budget: {reason}"
    );
    assert!(
        reason.contains("workspace validation"),
        "the escalation must name the failing phase: {reason}"
    );
    assert!(
        reason.contains("product_session_runtime_state_missing"),
        "the escalation must name the typed failure: {reason}"
    );

    // The escalated state is not a dead end: with the workspace back, the
    // existing confirm route requeues the message and starts it.
    restore_workspace_root(&folder, &moved_root);
    let confirmed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/sessions/{session_id}/controls/{message_id}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(confirmed.status(), StatusCode::OK);
    let delivered = wait_for_delivered_message(&app, session_id, message_id).await;
    let successor_run_id = delivered["successor_run_id"].as_str().unwrap().to_string();
    wait_for_successor_run_done(&app, workspace_id, session_id, &successor_run_id).await;
}

#[tokio::test]
async fn product_nonfinal_followups_require_confirmation_and_pending_controls_can_be_revoked() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Follow-up confirmation").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 2).await;

    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "keep this turn pending" }
            }).to_string(),
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(active.status(), StatusCode::OK);
    let active: CreateJobResponse = decode_json(active).await;
    wait_for_pending_input(app.clone(), active.job_id.to_string()).await;

    let queued = post_json(
        &app,
        &format!("/product/sessions/{session_id}/followups"),
        serde_json::json!({
            "content": "Only run after explicit confirmation.",
            "idempotency_key": "confirm-after-cancel"
        }),
    )
    .await;
    assert_eq!(queued.status(), StatusCode::CREATED);
    let queued: serde_json::Value = decode_json(queued).await;

    let revoked = post_json(
        &app,
        &format!("/product/sessions/{session_id}/followups"),
        serde_json::json!({
            "content": "This follow-up must be revoked.",
            "idempotency_key": "revoke-before-cancel"
        }),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::CREATED);
    let revoked: serde_json::Value = decode_json(revoked).await;
    let revoke = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/sessions/{session_id}/controls/{}/revoke",
                    revoked["id"].as_str().unwrap()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);
    let revoke: serde_json::Value = decode_json(revoke).await;
    assert_eq!(revoke["status"], "revoked");

    let cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", active.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancelled: JobStateResponse = decode_json(cancel).await;
    assert_eq!(cancelled.status, RunStatus::Cancelled);

    let abandoned = wait_for_product_control_status(
        &app,
        session_id,
        queued["id"].as_str().unwrap(),
        "abandoned",
    )
    .await;
    assert!(cancelled.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::FollowupAbandoned { id, .. } if id == abandoned["id"].as_str().unwrap()
        )
    }));
    let revoked_control = wait_for_product_control_status(
        &app,
        session_id,
        revoked["id"].as_str().unwrap(),
        "revoked",
    )
    .await;
    assert_eq!(revoked_control["status"], "revoked");
    let idle = wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    assert_eq!(
        idle["runtime_binding"]["latest_run_id"],
        active.run_id.to_string()
    );

    let confirm = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/sessions/{session_id}/controls/{}/confirm",
                    abandoned["id"].as_str().unwrap()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::OK);
    let confirm: serde_json::Value = decode_json(confirm).await;
    assert_eq!(confirm["id"], abandoned["id"]);

    let applied = wait_for_product_control_status(
        &app,
        session_id,
        abandoned["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    let finished = wait_for_product_session_status(&app, workspace_id, session_id, "idle").await;
    assert_eq!(finished["runtime_binding"]["ordinal"], 2);
    assert_eq!(
        finished["runtime_binding"]["latest_run_id"],
        applied["run_id"]
    );
    let successor = wait_for_done(
        app.clone(),
        finished["runtime_binding"]["latest_job_id"]
            .as_str()
            .unwrap()
            .to_string(),
    )
    .await;
    assert_eq!(successor.resumed_from_run_id, Some(active.run_id));
    assert!(successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::FollowupDequeued { id } if id == abandoned["id"].as_str().unwrap()
        )
    }));
    assert!(!successor.events.iter().any(|stored| {
        matches!(
            &stored.event,
            StreamEvent::FollowupDequeued { id } if id == revoked["id"].as_str().unwrap()
        )
    }));
}

#[tokio::test]
async fn api_server_stops_when_shutdown_token_is_cancelled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(serve_listener(
        listener,
        router(ApiState::new(workspace, test_config())),
        shutdown.clone(),
    ));

    shutdown.cancel();

    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .expect("server should stop after shutdown token is cancelled")
        .expect("server task should not panic")
        .unwrap();
}

#[tokio::test]
async fn api_rejects_missing_bearer_token_when_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.token_auth = Some("secret-token".to_string());
    let app = router(ApiState::new(workspace, config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"secured api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer"
    );
}

#[tokio::test]
async fn project_trust_is_exact_root_digest_bound_and_revocable() {
    let server = tempfile::TempDir::new().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(target.path().join(".rove")).unwrap();
    std::fs::write(target.path().join(".rove/mcp_servers.json"), "[]").unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, target.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let trust_uri = format!("/product/workspaces/{workspace_id}/trust");

    let unknown = get_response(&app, &trust_uri).await;
    assert_eq!(unknown.status(), StatusCode::OK);
    let unknown: serde_json::Value = decode_json(unknown).await;
    assert_eq!(unknown["state"], "unknown");
    assert!(unknown.get("canonical_root").is_none());

    let denied = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({"decision": "deny", "capabilities": []}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::OK);
    let denied: serde_json::Value = decode_json(denied).await;
    assert_eq!(denied["state"], "restricted");
    assert_eq!(denied["granted_capabilities"], serde_json::json!([]));

    let granted = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({
            "decision": "grant",
            "capabilities": ["project_configuration", "mcp_processes"]
        }),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);
    let granted: serde_json::Value = decode_json(granted).await;
    assert_eq!(granted["state"], "trusted");
    assert_eq!(
        granted["granted_capabilities"],
        serde_json::json!(["mcp_processes", "project_configuration"])
    );

    std::fs::write(
        target.path().join(".rove/mcp_servers.json"),
        r#"[{"name":"changed"}]"#,
    )
    .unwrap();
    let changed = get_response(&app, &trust_uri).await;
    assert_eq!(changed.status(), StatusCode::OK);
    let changed: serde_json::Value = decode_json(changed).await;
    assert_eq!(changed["state"], "trusted");
    assert_eq!(
        changed["invalidated_capabilities"],
        serde_json::json!(["mcp_processes"])
    );
    assert_eq!(
        changed["granted_capabilities"],
        serde_json::json!(["project_configuration"])
    );

    let nested_root = target.path().join("nested");
    std::fs::create_dir(&nested_root).unwrap();
    let nested = create_product_workspace(&app, &nested_root).await;
    let nested_id = nested["id"].as_str().unwrap();
    let nested_status = get_response(&app, &format!("/product/workspaces/{nested_id}/trust")).await;
    let nested_status: serde_json::Value = decode_json(nested_status).await;
    assert_eq!(nested_status["state"], "unknown");

    let revoked = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({"decision": "revoke", "capabilities": []}),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked: serde_json::Value = decode_json(revoked).await;
    assert_eq!(revoked["state"], "revoked");
    assert_eq!(revoked["granted_capabilities"], serde_json::json!([]));

    let session = create_product_session(&app, workspace_id, "Revoked workspace").await;
    let blocked = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must remain blocked after revocation",
            "product_session_id": session["id"]
        }),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    let blocked: serde_json::Value = decode_json(blocked).await;
    assert_eq!(blocked["code"], "project_trust_required");
}

#[tokio::test]
async fn product_provider_selection_is_part_of_trust_and_non_fake_jobs_fail_closed() {
    let server = tempfile::TempDir::new().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    let user_paths = UserConfigPaths::from_root(server.path().join("user-config"));
    let mut config = AppConfig::load_with_user_config_paths(
        server.path(),
        AppConfigOverrides {
            data_root: Some(server.path().join("data-root")),
            ..AppConfigOverrides::default()
        },
        user_paths,
    )
    .unwrap();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, target.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Provider trust selector").await;
    let session_id = session["id"].as_str().unwrap();
    let profile = post_json(
        &app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Trust selector provider",
            "provider_type": "openai",
            "api_base": "https://provider-a.example.test/v1",
            "api_key_env": "PROJECT_PROVIDER_SECRET",
            "default_model": "model-a"
        }),
    )
    .await;
    assert_eq!(profile.status(), StatusCode::CREATED);
    let profile: serde_json::Value = decode_json(profile).await;
    let profile_id = profile["id"].as_str().unwrap();
    let initial = get_response(
        &app,
        &format!("/product/sessions/{session_id}/model-config"),
    )
    .await;
    let initial: serde_json::Value = decode_json(initial).await;
    let configured = request_json(
        &app,
        "PUT",
        &format!("/product/sessions/{session_id}/model-config"),
        serde_json::json!({
            "profile_id": profile_id,
            "model": "model-a",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": initial["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);

    let blocked = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "must not contact the provider",
            "product_session_id": session_id
        }),
    )
    .await;
    let blocked_status = blocked.status();
    let blocked: serde_json::Value = decode_json(blocked).await;
    assert_eq!(blocked_status, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["code"], "project_trust_required");

    let trust_uri = format!("/product/workspaces/{workspace_id}/trust");
    let granted = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({
            "decision": "grant",
            "capabilities": ["provider_credentials"]
        }),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);
    let granted: serde_json::Value = decode_json(granted).await;
    assert_eq!(
        granted["granted_capabilities"],
        serde_json::json!(["provider_credentials"])
    );
    assert!(!granted.to_string().contains("PROJECT_PROVIDER_SECRET"));

    let updated_profile = request_json(
        &app,
        "PUT",
        &format!("/product/provider-profiles/{profile_id}"),
        serde_json::json!({
            "label": "Trust selector provider",
            "provider_type": "openai",
            "api_base": "https://provider-b.example.test/v1",
            "api_key_env": "PROJECT_PROVIDER_SECRET",
            "default_model": "model-a"
        }),
    )
    .await;
    assert_eq!(updated_profile.status(), StatusCode::OK);
    let invalidated = get_response(&app, &trust_uri).await;
    assert_eq!(invalidated.status(), StatusCode::OK);
    let invalidated: serde_json::Value = decode_json(invalidated).await;
    assert_eq!(
        invalidated["invalidated_capabilities"],
        serde_json::json!(["provider_credentials"])
    );
    assert_eq!(invalidated["granted_capabilities"], serde_json::json!([]));
}

#[tokio::test]
async fn operator_store_revocation_cancels_an_active_api_job() {
    let server = tempfile::TempDir::new().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    let authority_path = server.path().join("operator-project-trust.sqlite");
    let authority = Arc::new(ProjectTrustRepository::new(&authority_path));
    let mut config = AppConfig::load_with_user_config_paths(
        server.path(),
        AppConfigOverrides {
            model: Some("fake".to_string()),
            data_root: Some(server.path().join("data-root")),
            ..AppConfigOverrides::default()
        },
        UserConfigPaths::from_root(server.path().join("user-config")),
    )
    .unwrap();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::with_project_trust_repository(
        Workspace::detect(server.path()).unwrap(),
        config,
        authority,
    ));
    let workspace = create_product_workspace(&app, target.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "External revocation").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;
    let granted = request_json(
        &app,
        "PUT",
        &format!("/product/workspaces/{workspace_id}/trust"),
        serde_json::json!({"decision": "grant", "capabilities": []}),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);

    let message = serde_json::json!({
        "tool": "request_input",
        "args": {"prompt": "wait for an external trust decision"}
    })
    .to_string();
    let job = create_product_job(&app, session_id, &message).await;
    let pending = wait_for_pending_input(app.clone(), job.job_id.to_string()).await;
    assert_eq!(pending.status, RunStatus::Running);

    ProjectTrustRepository::new(authority_path)
        .revoke(
            target.path(),
            Workspace::detect(target.path()).unwrap().kind,
        )
        .unwrap();

    let cancelled = wait_for_status(app, job.job_id.to_string(), RunStatus::Cancelled).await;
    assert!(cancelled.pending_inputs.is_empty());
    assert!(cancelled.events.iter().any(|stored| matches!(
        stored.event,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Cancelled,
            ..
        }
    )));
}

#[tokio::test]
async fn api_and_bootstrap_share_one_canonical_project_trust_authority() {
    let server = tempfile::TempDir::new().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(target.path().join(".rove")).unwrap();
    std::fs::write(
        target.path().join(".rove/config.toml"),
        "[runtime]\nmax_steps = 7\n",
    )
    .unwrap();
    let authority = Arc::new(ProjectTrustRepository::new(
        server.path().join("canonical-project-trust.sqlite"),
    ));
    let user_paths = UserConfigPaths::from_root(server.path().join("user-config"));
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::with_project_trust_repository(
        Workspace::detect(server.path()).unwrap(),
        config,
        authority.clone(),
    ));
    let workspace = create_product_workspace(&app, target.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let trust_uri = format!("/product/workspaces/{workspace_id}/trust");

    let granted = request_json(
        &app,
        "PUT",
        &trust_uri,
        serde_json::json!({"decision": "grant", "capabilities": []}),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);
    let bootstrap = AppConfig::load_with_authorities(
        target.path(),
        AppConfigOverrides {
            data_root: Some(server.path().join("data-root")),
            ..AppConfigOverrides::default()
        },
        authority.as_ref(),
        user_paths,
    )
    .unwrap();
    assert_eq!(
        bootstrap.project_activation_state(),
        ProjectActivationState::Trusted
    );
    assert!(bootstrap.source_summary.project_config_loaded);

    let provider_selector = provider_capability_selector_for_workspace(target.path());
    authority
        .decide(
            target.path(),
            Workspace::detect(target.path()).unwrap().kind,
            ProjectTrustDecision::Deny,
            std::collections::BTreeMap::new(),
        )
        .unwrap();
    let api_status = get_response(&app, &trust_uri).await;
    assert_eq!(api_status.status(), StatusCode::OK);
    let api_status: serde_json::Value = decode_json(api_status).await;
    assert_eq!(api_status["state"], "restricted");
    let resolution = authority
        .resolve(
            target.path(),
            Workspace::detect(target.path()).unwrap().kind,
            &capability_digest_map(target.path(), None, Some(&provider_selector)),
        )
        .unwrap();
    assert_eq!(resolution.state, ProjectActivationState::Restricted);
}

#[tokio::test]
async fn project_trust_mutation_requires_bearer_and_allowed_origin() {
    let server = tempfile::TempDir::new().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.api.token_auth = Some("trust-token".to_string());
    config.api.cors_origins = vec!["https://allowed.example".to_string()];
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/workspaces")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer trust-token")
                .header("origin", "https://allowed.example")
                .body(Body::from(
                    serde_json::json!({
                        "root": target.path(),
                        "kind": "folder",
                        "display_name": "Secured trust workspace",
                        "pinned": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let workspace: serde_json::Value = decode_json(created).await;
    let trust_uri = format!(
        "/product/workspaces/{}/trust",
        workspace["id"].as_str().unwrap()
    );
    let body = serde_json::json!({"decision": "grant", "capabilities": []}).to_string();

    let missing_token = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&trust_uri)
                .header(CONTENT_TYPE, "application/json")
                .header("origin", "https://allowed.example")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);

    let disallowed_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&trust_uri)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer trust-token")
                .header("origin", "https://evil.example")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disallowed_origin.status(), StatusCode::FORBIDDEN);

    let status = app
        .oneshot(
            Request::builder()
                .uri(&trust_uri)
                .header(AUTHORIZATION, "Bearer trust-token")
                .header("origin", "https://allowed.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status: serde_json::Value = decode_json(status).await;
    assert_eq!(status["state"], "unknown");
}

#[tokio::test]
async fn api_docs_do_not_disable_bearer_token_for_business_routes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.token_auth = Some("secret-token".to_string());
    let app = router(ApiState::new(workspace, config));

    let docs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(docs.status(), StatusCode::OK);

    let business = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"secured api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(business.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_accepts_matching_bearer_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.token_auth = Some("secret-token".to_string());
    let app = router(ApiState::new(workspace, config));

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-token")
                .body(Body::from(r#"{"message":"secured api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let allowed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .header("authorization", "Bearer secret-token")
                .body(Body::from(r#"{"message":"secured api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_tests_openai_provider_profile_without_exposing_key() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_PROVIDER_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-provider-token");
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/providers/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": {
                            "provider_type": "openai",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        },
                        "model": "relay/deepseek-v3.2"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "pass");
    // The `openai` type maps to the openai-completions wire protocol; the display name
    // defaults from the endpoint host rather than echoing the type label.
    assert_eq!(json["provider_type"], "openai");
    assert_eq!(json["wire_protocol"], "openai-completions");
    let provider_label = json["provider"].as_str().unwrap_or_default();
    assert!(
        provider_label.starts_with("127.0.0.1:") || provider_label == "openai",
        "expected host-derived provider label, got {provider_label}"
    );
    assert_eq!(json["key_env"], key_env);
    assert_eq!(json["key_present"], true);
    assert_eq!(json["model"], "relay/deepseek-v3.2");
    assert_eq!(json["model_present"], true);
    assert_eq!(json["models_count"], 2);
    assert!(!text.contains("dummy-provider-token"));
    assert_eq!(
        provider.captured.lock().unwrap().models_auth.as_deref(),
        Some("Bearer dummy-provider-token")
    );
}

/// A provider key the secret authority refuses fails the request closed.
///
/// The key is about to be placed into an outbound provider request, so a value
/// the registry would not hold — here one below its length floor — must not be
/// sent: the failure is typed and visible, and the upstream test server records
/// that it was never asked for a catalog.
#[tokio::test]
async fn api_provider_test_refuses_a_key_the_secret_registry_cannot_hold() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_PROVIDER_SHORT_KEY");
    let refused = "7bytes!";
    unsafe {
        std::env::set_var(&key_env, refused);
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/providers/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": {
                            "provider_type": "openai",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        },
                        "model": "relay/deepseek-v3.2"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        text.contains(&key_env),
        "the failure names the environment variable: {text}"
    );
    assert!(
        text.contains("value_too_short"),
        "the failure carries the typed reason: {text}"
    );
    assert!(
        !text.contains(refused),
        "the failure never carries the key: {text}"
    );
    assert!(
        provider.captured.lock().unwrap().models_auth.is_none(),
        "the refused key must not reach the provider"
    );
}

#[tokio::test]
async fn api_lists_provider_models_without_exposing_key() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_PROVIDER_MODELS_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-models-token");
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/providers/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": {
                            "provider_type": "openai",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["provider_type"], "openai");
    assert_eq!(json["wire_protocol"], "openai-completions");
    assert_eq!(json["key_env"], key_env);
    assert_eq!(json["key_present"], true);
    assert_eq!(json["models_count"], 2);
    assert_eq!(
        json["models"],
        serde_json::json!(["relay/deepseek-v3.2", "official/gpt-compatible"])
    );
    assert!(!text.contains("dummy-models-token"));
    assert_eq!(
        provider.captured.lock().unwrap().models_auth.as_deref(),
        Some("Bearer dummy-models-token")
    );
}

#[tokio::test]
async fn api_provider_inventory_reports_typed_bounded_failures_without_upstream_body() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_PROVIDER_FAILURE_KEY");
    let secret_body = "upstream-secret-provider-token";
    unsafe {
        std::env::set_var(&key_env, secret_body);
    }

    let cases = [
        (
            "unauthorized",
            StatusCode::BAD_GATEWAY,
            "provider_authentication",
        ),
        (
            "rate-limited",
            StatusCode::TOO_MANY_REQUESTS,
            "provider_rate_limited",
        ),
        (
            "invalid",
            StatusCode::BAD_GATEWAY,
            "provider_protocol_mismatch",
        ),
        ("empty", StatusCode::BAD_GATEWAY, "provider_no_models"),
    ];
    for (suffix, expected_status, expected_code) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/providers/test")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": {
                                "provider_type": "openai",
                                "api_base": format!("{}/v1", provider.base_url),
                                "api_key_env": key_env
                            },
                            "models_endpoint": format!("{}/v1/models-{suffix}", provider.base_url)
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status, "case {suffix}");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], expected_code, "case {suffix}");
        assert!(
            !text.contains(secret_body),
            "case {suffix} leaked upstream body"
        );
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/providers/test")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": {
                            "provider_type": "openai",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        },
                        "models_endpoint": format!("{}/v1/models-slow", provider.base_url)
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    unsafe {
        std::env::remove_var(&key_env);
    }
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "provider_timeout");
}

#[tokio::test]
async fn api_tests_openai_responses_provider_profile_without_exposing_key() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_RESPONSES_PROVIDER_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-responses-provider-token");
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/providers/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": {
                            "provider_type": "openai-responses",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        },
                        "model": "gpt-4.1-mini"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "pass");
    assert_eq!(json["provider_type"], "openai-responses");
    assert_eq!(json["wire_protocol"], "openai-responses");
    let provider_label = json["provider"].as_str().unwrap_or_default();
    assert!(
        provider_label.starts_with("127.0.0.1:") || provider_label == "openai-responses",
        "expected host-derived provider label, got {provider_label}"
    );
    assert_eq!(json["key_env"], key_env);
    assert_eq!(json["key_present"], true);
    assert_eq!(json["model"], "gpt-4.1-mini");
    assert_eq!(json["model_present"], false);
    assert_eq!(json["models_count"], 2);
    assert!(!text.contains("dummy-responses-provider-token"));
    assert_eq!(
        provider.captured.lock().unwrap().models_auth.as_deref(),
        Some("Bearer dummy-responses-provider-token")
    );
}

#[tokio::test]
async fn api_jobs_accept_openai_provider_profile_per_request() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_JOB_PROVIDER_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-job-provider-token");
    }

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "Reply with exactly: routed provider ok",
                        "model": "relay/deepseek-v3.2",
                        "approval": "auto",
                        "max_steps": 1,
                        "provider": {
                            "provider_type": "openai",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(created.status(), StatusCode::OK);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::RunCompleted {
                output: Some(output),
                ..
            } if output.contains("routed provider ok")
        )
    }));
    let captured = provider.captured.lock().unwrap();
    assert_eq!(
        captured.chat_auth.as_deref(),
        Some("Bearer dummy-job-provider-token")
    );
    assert_eq!(captured.chat_model.as_deref(), Some("relay/deepseek-v3.2"));
}

#[tokio::test]
async fn api_job_agent_override_activates_a_trusted_workspace_package() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_api_agent_definition(tmp.path());
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let config = test_config();
    assert_eq!(config.runtime.agent.selector, "builtin:legacy");
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace, config));

    let response = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "diagnose rollback",
            "model": "fake",
            "max_steps": 1,
            "approval": "auto",
            "agent": "workspace:ops"
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let created: CreateJobResponse = decode_json(response).await;
    let state = wait_for_done(app, created.job_id.to_string()).await;
    assert!(state.events.iter().any(|stored| matches!(
        &stored.event,
        StreamEvent::AgentProfileActivated { identity, .. }
            if identity.selector.to_string() == "workspace:ops"
    )));
    let task_state = state_store.load_task_state(created.run_id).await.unwrap();
    assert_eq!(
        task_state
            .agent_profile
            .as_ref()
            .map(|profile| profile.selector.to_string())
            .as_deref(),
        Some("workspace:ops")
    );
}

#[tokio::test]
async fn api_rejects_untrusted_or_invalid_agent_selectors_before_creating_a_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_api_agent_definition(tmp.path());
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let mut config = test_config();
    config.source_summary.project_activation = ProjectActivationState::Restricted;
    let app = router(ApiState::new(workspace, config));

    let unauthorized = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "diagnose rollback",
            "model": "fake",
            "agent": "workspace:ops"
        }),
    )
    .await;
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);
    let unauthorized: serde_json::Value = decode_json(unauthorized).await;
    assert_eq!(unauthorized["code"], "workspace_source_not_authorized");
    assert!(state_store.list_task_states().await.unwrap().is_empty());

    let invalid = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": "diagnose rollback",
            "model": "fake",
            "agent": "ops"
        }),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let invalid: serde_json::Value = decode_json(invalid).await;
    assert_eq!(invalid["code"], "missing_source_namespace");
    assert!(state_store.list_task_states().await.unwrap().is_empty());
}

#[tokio::test]
async fn api_jobs_accept_openai_responses_provider_profile_per_request() {
    let provider = start_openai_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_RESPONSES_PROVIDER_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-responses-provider-token");
    }

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "Reply with exactly: responses profile ok",
                        "model": "gpt-4.1-mini",
                        "approval": "auto",
                        "max_steps": 1,
                        "provider": {
                            "provider_type": "openai-responses",
                            "api_base": format!("{}/v1", provider.base_url),
                            "api_key_env": key_env
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(created.status(), StatusCode::OK);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::RunCompleted {
                output: Some(output),
                ..
            } if output.contains("responses profile ok")
        )
    }));
    let captured = provider.captured.lock().unwrap();
    assert_eq!(
        captured.responses_auth.as_deref(),
        Some("Bearer dummy-responses-provider-token")
    );
    assert_eq!(captured.responses_model.as_deref(), Some("gpt-4.1-mini"));
    assert!(captured.responses_body.is_some());
}

#[tokio::test]
async fn api_jobs_accept_anthropic_provider_profile_per_request() {
    let provider = start_anthropic_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let key_env = unique_env_key("ROVE_TEST_ANTHROPIC_KEY");
    unsafe {
        std::env::set_var(&key_env, "dummy-anthropic-token");
    }

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "Reply with exactly: anthropic profile ok",
                        "model": "claude-test",
                        "approval": "auto",
                        "max_steps": 1,
                        "provider": {
                            "provider_type": "anthropic",
                            "api_base": provider.base_url,
                            "api_key_env": key_env
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    unsafe {
        std::env::remove_var(&key_env);
    }

    assert_eq!(created.status(), StatusCode::OK);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::RunCompleted {
                output: Some(output),
                ..
            } if output.contains("anthropic profile ok")
        )
    }));
    let captured = provider.captured.lock().unwrap();
    assert_eq!(
        captured.anthropic_auth.as_deref(),
        Some("dummy-anthropic-token")
    );
    assert_eq!(captured.anthropic_model.as_deref(), Some("claude-test"));
}

#[tokio::test]
async fn api_jobs_accept_ollama_provider_profile_without_key() {
    let provider = start_ollama_test_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "Reply with exactly: ollama profile ok",
                        "model": "llama-test",
                        "approval": "auto",
                        "max_steps": 1,
                        "provider": {
                            "provider_type": "ollama",
                            "api_base": provider.base_url
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(created.status(), StatusCode::OK);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::RunCompleted {
                output: Some(output),
                ..
            } if output.contains("ollama profile ok")
        )
    }));
    assert_eq!(
        provider.captured.lock().unwrap().ollama_model.as_deref(),
        Some("llama-test")
    );
}

#[tokio::test]
async fn api_rejects_disallowed_cors_origin() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.cors_origins = vec!["https://allowed.example".to_string()];
    let app = router(ApiState::new(workspace, config));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn api_rejects_browser_origin_when_cors_is_not_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[tokio::test]
async fn api_allows_same_origin_browser_requests_without_cors_config() {
    // When the API itself serves the console (router_with_web), the browser's
    // POSTs carry an Origin equal to the request's Host. That is a same-origin
    // request, not a cross-origin one, so the CORS allowlist must not gate it.
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
                .header("host", "127.0.0.1:8787")
                .header("origin", "http://127.0.0.1:8787")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // 404 (unknown job) proves the request passed the security layer; 403
    // would mean the same-origin Origin was treated as a foreign browser.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_rejects_an_origin_mismatching_the_host() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
                .header("host", "127.0.0.1:8787")
                .header("origin", "http://127.0.0.1:9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn web_console_bundle_serves_statics_health_and_api_prefix() {
    let tmp = tempfile::TempDir::new().unwrap();
    let web_root = tmp.path().join("web-dist");
    std::fs::create_dir_all(web_root.join("_next/static")).unwrap();
    std::fs::write(web_root.join("index.html"), "<html>rove</html>").unwrap();
    std::fs::write(web_root.join("_next/static/app.js"), "// bundle").unwrap();

    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app =
        rove_api::router_with_web(ApiState::new(workspace, test_config()), &web_root).unwrap();

    for (uri, status) in [
        ("/", StatusCode::OK),
        ("/w/abc/s/def", StatusCode::OK), // SPA fallback for a page navigation
        ("/_next/static/app.js", StatusCode::OK),
        ("/health", StatusCode::OK),
        // The nested mount reaches the real API (unknown job → typed 404).
        (
            "/api/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state",
            StatusCode::NOT_FOUND,
        ),
        // ...while an unmatched /api path is a JSON 404, never the SPA shell.
        ("/api/xyz/unknown", StatusCode::NOT_FOUND),
        // A missing chunk is a real 404 even for a browser-looking request.
        ("/_next/static/missing.js", StatusCode::NOT_FOUND),
    ] {
        let request = Request::builder()
            .uri(uri)
            .header("accept", "text/html,application/xhtml+xml")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), status, "uri {uri}");
    }

    // The /api miss is the JSON error envelope, not the bundle's HTML.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/xyz/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "not_found");
}

#[tokio::test]
async fn web_console_statics_stay_public_while_the_api_keeps_auth() {
    let tmp = tempfile::TempDir::new().unwrap();
    let web_root = tmp.path().join("web-dist");
    std::fs::create_dir_all(&web_root).unwrap();
    std::fs::write(web_root.join("index.html"), "<html>rove</html>").unwrap();

    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.token_auth = Some("secret".to_string());
    let app = rove_api::router_with_web(ApiState::new(workspace, config), &web_root).unwrap();

    // The bundle is public code: statics and liveness answer without a token.
    for uri in ["/", "/health"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "uri {uri}");
    }

    // ...while the API under both mounts still enforces Bearer.
    for uri in [
        "/api/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state",
        "/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "uri {uri}");
    }
}

#[tokio::test]
async fn api_allows_configured_cors_origin_and_sets_headers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.cors_origins = vec!["https://allowed.example".to_string()];
    let app = router(ApiState::new(workspace, config));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
                .header("origin", "https://allowed.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "https://allowed.example"
    );
}

#[tokio::test]
async fn api_allows_configured_authenticated_cors_preflight() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.token_auth = Some("desktop-secret".to_string());
    config.api.cors_origins = vec!["tauri://localhost".to_string()];
    let app = router(ApiState::new(workspace, config));

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/product/runtime")
                .header("origin", "tauri://localhost")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "tauri://localhost"
    );
    assert!(
        response
            .headers()
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("authorization")
    );
}

#[tokio::test]
async fn api_rate_limits_requests_when_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.api.rate_limit_per_minute = Some(2);
    let app = router(ApiState::new(workspace, config));

    let request = || {
        Request::builder()
            .uri("/jobs/01ARZ3NDEKTSV4RRFFQ69G5FAV/state")
            .body(Body::empty())
            .unwrap()
    };

    let first = app.clone().oneshot(request()).await.unwrap();
    let second = app.clone().oneshot(request()).await.unwrap();
    let third = app.oneshot(request()).await.unwrap();

    assert_eq!(first.status(), StatusCode::NOT_FOUND);
    assert_eq!(second.status(), StatusCode::NOT_FOUND);
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn api_creates_job_streams_events_and_reports_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"hello api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert!(state.event_count > 0);

    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("event: run_started"));
    assert!(text.contains("event: run_completed"));
}

#[tokio::test]
async fn api_can_create_job_in_task_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.memory.session_dir = "api-memory/sessions".into();
    config.memory.durable_dir = "api-memory/durable".into();
    let app = router(ApiState::new(workspace, config));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"message":"task api","model":"fake","workspace":{{"kind":"task","name":"api-task","base":{}}}}}"#,
                    serde_json::to_string(&tmp.path().to_string_lossy()).unwrap()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;

    assert_eq!(state.status, RunStatus::Done);
    let task_root = tmp.path().join("api-task");
    assert!(task_root.join("api-state").join("runs").is_dir());
    assert!(task_root.join("api-memory").join("sessions").is_dir());
}

#[tokio::test]
async fn api_can_create_job_in_explicit_folder_root() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let server_workspace = Workspace::detect(server.path()).unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.memory.session_dir = "api-memory/sessions".into();
    config.memory.durable_dir = "api-memory/durable".into();
    let app = router(ApiState::new(server_workspace, config));

    let marker = folder.path().join("marker.txt");
    std::fs::write(&marker, "folder-root").unwrap();

    let create_body = serde_json::json!({
        "message": serde_json::json!({
            "tool": "write_file",
            "args": { "path": "from-job.txt", "content": "folder-ok" }
        }).to_string(),
        "model": "fake-raw",
        "approval": "auto",
        "max_steps": 1,
        "workspace": {
            "kind": "folder",
            "root": folder.path()
        }
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;
    assert_eq!(state.status, RunStatus::Done);

    let written = folder.path().join("from-job.txt");
    assert_eq!(std::fs::read_to_string(written).unwrap(), "folder-ok");
    assert!(
        folder.path().join("api-state").join("runs").is_dir(),
        "state should live under the opened folder root"
    );
    assert!(
        !server.path().join("api-state").join("runs").exists(),
        "server cwd workspace must not receive the job state"
    );
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "folder-root");
}

#[tokio::test]
async fn api_can_create_job_in_explicit_repo_root() {
    let server = tempfile::TempDir::new().unwrap();
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    let server_workspace = Workspace::detect(server.path()).unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(server_workspace, config));

    let create_body = serde_json::json!({
        "message": serde_json::json!({
            "tool": "write_file",
            "args": { "path": "repo-note.txt", "content": "repo-ok" }
        }).to_string(),
        "model": "fake-raw",
        "approval": "auto",
        "max_steps": 1,
        "workspace": {
            "kind": "repo",
            "root": repo.path()
        }
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Done).await;
    assert_eq!(state.status, RunStatus::Done);

    assert_eq!(
        std::fs::read_to_string(repo.path().join("repo-note.txt")).unwrap(),
        "repo-ok"
    );
    assert!(repo.path().join("api-state").join("runs").is_dir());
    assert!(!server.path().join("api-state").join("runs").exists());
}

#[tokio::test]
async fn api_rejects_invalid_folder_and_repo_workspace_bindings() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));

    let missing_root = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "x",
                        "model": "fake",
                        "workspace": { "kind": "folder" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_root.status(), StatusCode::BAD_REQUEST);

    let relative = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "x",
                        "model": "fake",
                        "workspace": {
                            "kind": "folder",
                            "root": "relative/not-absolute"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(relative.status(), StatusCode::BAD_REQUEST);

    let repo_without_git = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "x",
                        "model": "fake",
                        "workspace": {
                            "kind": "repo",
                            "root": folder.path()
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(repo_without_git.status(), StatusCode::BAD_REQUEST);

    let mixed_fields = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "x",
                        "model": "fake",
                        "workspace": {
                            "kind": "folder",
                            "root": folder.path(),
                            "name": "should-not-be-here"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mixed_fields.status(), StatusCode::BAD_REQUEST);

    let task_with_root = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "x",
                        "model": "fake",
                        "workspace": {
                            "kind": "task",
                            "name": "x",
                            "root": folder.path()
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(task_with_root.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn api_hard_resumes_second_turn_in_explicit_folder_workspace() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace_binding = serde_json::json!({
        "kind": "folder",
        "root": folder.path()
    });

    let first_body = serde_json::json!({
        "message": "first folder turn",
        "model": "fake",
        "workspace": workspace_binding
    });
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(first_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let first: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;
    assert_eq!(first_state.status, RunStatus::Done);

    let state_store = rove_runtime::state::store::StateStore::new(&folder.path().join("api-state"));
    let first_task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            state_store
                .run_store
                .run_dir(&first.run_id)
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let resume_body = serde_json::json!({
        "message": "second folder turn",
        "model": "fake",
        "resume": "latest",
        "workspace": workspace_binding
    });
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(resume_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let resumed: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    assert_ne!(resumed.run_id, first.run_id);
    assert_eq!(resumed.resumed_from_run_id, Some(first.run_id));
    assert_eq!(resumed.job_id, first.job_id);

    let resumed_state = wait_for_done(app.clone(), resumed.job_id.to_string()).await;
    assert_eq!(resumed_state.status, RunStatus::Done);
    assert_eq!(resumed_state.resumed_from_run_id, Some(first.run_id));

    let resumed_task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            state_store
                .run_store
                .run_dir(&resumed.run_id)
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(resumed_task_state.session_id, first_task_state.session_id);
    assert_eq!(resumed_task_state.job_id, first_task_state.job_id);
    assert!(
        resumed_task_state
            .history
            .iter()
            .any(|message| message.role == Role::User && message.content == "first folder turn")
    );
    assert!(
        resumed_task_state
            .history
            .iter()
            .any(|message| message.role == Role::User && message.content == "second folder turn")
    );
    assert!(
        !server.path().join("api-state").join("runs").exists(),
        "hard resume must not fall back to server cwd workspace"
    );
}

#[tokio::test]
async fn api_rejects_resume_when_workspace_root_has_no_task_state() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let other = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    let first_body = serde_json::json!({
        "message": "seed folder turn",
        "model": "fake",
        "workspace": {
            "kind": "folder",
            "root": folder.path()
        }
    });
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(first_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let first: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;
    assert_eq!(first_state.status, RunStatus::Done);

    // Resume against a different explicit root must not invent soft continuity.
    let mismatched = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "resume wrong root",
                        "model": "fake",
                        "resume": "latest",
                        "workspace": {
                            "kind": "folder",
                            "root": other.path()
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mismatched.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(mismatched.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("nothing to resume in this workspace"),
        "expected hard-resume failure, got {error}"
    );

    // Omitting workspace falls back to the API process workspace, which has no
    // durable state for the folder job — also fail closed.
    let omitted = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "resume without workspace",
                        "model": "fake",
                        "resume": "latest"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(omitted.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(omitted.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("nothing to resume in this workspace"),
        "expected hard-resume failure without workspace, got {error}"
    );
    assert!(
        !other.path().join("api-state").join("runs").exists(),
        "failed resume must not create a silent one-shot job under the wrong root"
    );
}

#[tokio::test]
async fn api_approves_pending_tool_under_explicit_folder_root() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let output_path = folder.path().join("approved-folder.txt");

    let create_body = serde_json::json!({
        "message": serde_json::json!({
            "tool": "write_file",
            "args": {
                "path": "approved-folder.txt",
                "content": "ok"
            }
        }).to_string(),
        "model": "fake-raw",
        "approval": "ask",
        "max_steps": 1,
        "workspace": {
            "kind": "folder",
            "root": folder.path()
        }
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_approval_event(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap();
    assert_eq!(approval.name, "write_file");
    assert!(!output_path.exists(), "tool should wait before approval");

    let approve = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/approvals/{}",
                    created.job_id, approval.call_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approve.status(), StatusCode::OK);

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert_eq!(std::fs::read_to_string(output_path).unwrap(), "ok");
    assert!(!server.path().join("approved-folder.txt").exists());
}

#[tokio::test]
async fn api_sse_events_have_ids_and_support_after_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"resume api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.event_count, state.events.len());
    assert_eq!(state.events.first().unwrap().seq, 1);
    assert!(
        state
            .events
            .windows(2)
            .all(|pair| pair[1].seq > pair[0].seq)
    );

    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.lines().any(|line| line == "id: 1"));
    assert!(text.contains("event: run_started"));

    // Every frame is version-stamped, and the payload stays flattened so a
    // client written before versioning still reads `type` and the event fields
    // off the top level.
    let first_data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("expected at least one data frame");
    assert!(
        first_data.starts_with(&format!("{{\"v\":{},", rove_protocol::PROTOCOL_VERSION)),
        "expected the protocol version to lead the frame, got {first_data}"
    );
    let decoded: serde_json::Value = serde_json::from_str(first_data).unwrap();
    assert_eq!(decoded["v"], rove_protocol::PROTOCOL_VERSION);
    assert_eq!(decoded["type"], "run_started");
    assert!(
        decoded.get("payload").is_none(),
        "the event body must be flattened, not nested under a key"
    );

    let after_first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events?after=1", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_first.status(), StatusCode::OK);
    let body = axum::body::to_bytes(after_first.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.lines().any(|line| line == "id: 1"));
    assert!(!text.contains("event: run_started"));
    assert!(text.lines().any(|line| line == "id: 2"));

    let header_resume = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .header("last-event-id", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(header_resume.status(), StatusCode::OK);
    let body = axum::body::to_bytes(header_resume.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.lines().any(|line| line == "id: 1"));
    assert!(text.lines().any(|line| line == "id: 2"));
}

#[tokio::test]
async fn api_state_includes_input_needed_event_for_snapshot_recovery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "Which branch should I use?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.event_count, state.events.len());
    assert_eq!(state.pending_inputs.len(), 1);
    assert!(
        state
            .events
            .windows(2)
            .all(|pair| pair[1].seq > pair[0].seq)
    );
    let input_events: Vec<_> = state
        .events
        .iter()
        .filter_map(|stored| match &stored.event {
            StreamEvent::InputNeeded { input_id, prompt } => Some((*input_id, prompt.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        input_events,
        vec![(
            state.pending_inputs[0].input_id,
            "Which branch should I use?"
        )]
    );
}

#[tokio::test]
async fn api_writes_run_artifacts_for_completed_job() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"artifact api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let run_dir = state_store.run_store.run_dir(&created.run_id);
    let trace_path = run_dir.join("trace.jsonl");
    let task_state_path = run_dir.join("task_state.json");
    let report_path = run_dir.join("report.json");

    assert!(trace_path.exists(), "trace.jsonl should be written");
    assert!(
        task_state_path.exists(),
        "task_state.json should be written"
    );
    assert!(report_path.exists(), "report.json should be written");

    let task_state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&task_state_path).unwrap()).unwrap();
    assert_eq!(task_state["job_id"], created.job_id.to_string());
    assert_eq!(task_state["run_id"], created.run_id.to_string());
    assert_eq!(task_state["goal"], "artifact api");

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["job_id"], created.job_id.to_string());
    assert_eq!(report["run_id"], created.run_id.to_string());
    assert_eq!(report["status"], "success");
    // A planned run's final answer comes from the independent finalizer, which
    // synthesizes an evidence-grounded answer rather than forwarding the last
    // model message verbatim. The model's conclusion is retained as the step
    // summary so no answer content is lost.
    let output = report["output"].as_str().expect("report output");
    assert!(
        output.contains("fake response: artifact api"),
        "finalized output must retain the step conclusion: {output}"
    );
    assert!(
        output.contains("outcome: success"),
        "finalized output must state the resolved outcome: {output}"
    );
    assert_eq!(report["final_outcome"], "success");
    assert_eq!(
        report["execution_lifecycle"]["finalization"]["phase"],
        "completed"
    );
    let prompt_build = report["prompt_builds"][0]
        .as_object()
        .expect("report should include prompt build metadata");
    assert!(
        prompt_build["prompt_hash"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        prompt_build["stable_prefix_hash"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        prompt_build["workspace_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        prompt_build["tool_signature"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        prompt_build["prompt_cache_key"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );

    assert!(
        state_store.index.path().exists(),
        "state.sqlite should be written"
    );
    let indexed_job = state_store
        .index
        .job_record(created.job_id)
        .unwrap()
        .expect("job should be indexed");
    assert_eq!(indexed_job.status, "done");
    assert_eq!(indexed_job.run_id, Some(created.run_id));
    assert_eq!(indexed_job.message.as_deref(), Some("artifact api"));
    let indexed_run = state_store
        .index
        .run_record(created.run_id)
        .unwrap()
        .expect("run should be indexed");
    assert_eq!(indexed_run.status, "done");
    assert_eq!(
        indexed_run.task_state_path.as_deref(),
        Some(task_state_path.as_path())
    );
    assert_eq!(
        indexed_run.report_path.as_deref(),
        Some(report_path.as_path())
    );
    assert!(indexed_run.last_event_seq > 0);
    let indexed_report = state_store
        .index
        .report_record(created.run_id)
        .unwrap()
        .expect("report should be indexed");
    assert_eq!(indexed_report.path, report_path);
    assert_eq!(indexed_report.status, "success");
    assert_eq!(indexed_report.termination_reason, "final");
    let indexed_events = state_store.index.event_records(created.run_id).unwrap();
    assert!(
        indexed_events
            .iter()
            .any(|event| event.event_name == "run_started")
    );
    assert!(
        indexed_events
            .iter()
            .any(|event| event.event_name == "run_completed")
    );
}

#[tokio::test]
async fn api_lists_completed_runs_after_job_finishes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"listable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let response = app
        .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let runs = body["runs"].as_array().expect("runs array");
    assert!(
        runs.iter().any(|run| {
            run["run_id"] == created.run_id.to_string()
                && run["status"] == "done"
                && run["has_report"] == true
        }),
        "completed run should appear in /runs response: {body}"
    );
}

#[tokio::test]
async fn api_lists_step_limited_tool_runs_as_done_not_interrupted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"echo\",\"args\":{\"message\":\"list step-limited tool run\"}}","model":"fake-raw","approval":"auto","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let response = app
        .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let runs = body["runs"].as_array().expect("runs array");
    assert!(
        runs.iter().any(|run| {
            run["run_id"] == created.run_id.to_string()
                && run["status"] == "done"
                && run["has_report"] == true
        }),
        "step-limited run should appear as done in /runs response: {body}"
    );
}

#[tokio::test]
async fn api_fetches_completed_run_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"reportable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/report", created.run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(report["run_id"], created.run_id.to_string());
    assert_eq!(report["job_id"], created.job_id.to_string());
    assert_eq!(report["status"], "success");
}

#[tokio::test]
async fn api_returns_404_for_missing_run_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs/01ARZ3NDEKTSV4RRFFQ69G5FAV/report")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_lists_and_fetches_run_report_after_restart() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let config = test_config();
    let app = router(ApiState::new(workspace.clone(), config.clone()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"restart reportable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let restarted = router(ApiState::new(workspace, config));
    let runs = restarted
        .clone()
        .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(runs.status(), StatusCode::OK);
    let body = axum::body::to_bytes(runs.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["run_id"] == created.run_id.to_string())
    );

    let report = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/report", created.run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(report.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_can_resume_latest_task_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"first api","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let first: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let first_state = wait_for_done(app.clone(), first.job_id.to_string()).await;
    assert_eq!(first_state.status, RunStatus::Done);

    let first_task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            state_store
                .run_store
                .run_dir(&first.run_id)
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let resumed_body = serde_json::json!({
        "message": "continue api",
        "model": "fake",
        "resume": "latest"
    });
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(resumed_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let resumed: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    assert_ne!(resumed.run_id, first.run_id);
    assert_eq!(resumed.resumed_from_run_id, Some(first.run_id));

    let resumed_state = wait_for_done(app.clone(), resumed.job_id.to_string()).await;
    assert_eq!(resumed_state.status, RunStatus::Done);
    assert_eq!(resumed_state.resumed_from_run_id, Some(first.run_id));
    let resumed_task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            state_store
                .run_store
                .run_dir(&resumed.run_id)
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(resumed.job_id, first.job_id);
    assert_eq!(resumed_task_state.session_id, first_task_state.session_id);
    assert_eq!(resumed_task_state.job_id, first_task_state.job_id);
    assert_eq!(resumed_task_state.run_id, resumed.run_id);
    assert!(
        resumed_task_state
            .history
            .iter()
            .any(|message| message.role == Role::User && message.content == "first api")
    );
    assert!(
        resumed_task_state
            .history
            .iter()
            .any(|message| message.role == Role::User && message.content == "continue api")
    );
    assert!(resumed_task_state.step >= first_task_state.step);
}

#[tokio::test]
async fn api_rejects_resume_when_job_is_still_live() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "continue?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.status, RunStatus::Running);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "resume while live",
                        "model": "fake",
                        "resume": created.run_id.to_string()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn api_rejects_invalid_resume_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let body = serde_json::json!({
        "message": "continue api",
        "model": "fake",
        "resume": "not-a-run-id"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("expected latest or run_id")
    );
}

#[tokio::test]
async fn api_reads_completed_job_state_and_events_after_restart() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace.clone(), test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"restart replay","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let original_state = wait_for_done(app.clone(), created.job_id.to_string()).await;

    let restarted = router(ApiState::new(workspace, test_config()));
    let state = restarted
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/state", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body = axum::body::to_bytes(state.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Done);
    assert_eq!(state.job_id, created.job_id);
    assert_eq!(state.run_id, created.run_id);
    assert_eq!(state.event_count, original_state.event_count);
    assert!(state.pending_approvals.is_empty());
    assert!(state.pending_inputs.is_empty());

    let events = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events?after=1", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.lines().any(|line| line == "id: 1"));
    assert!(!text.contains("event: run_started"));
    assert!(text.contains("event: run_completed"));
}

#[tokio::test]
async fn api_startup_marks_stale_running_jobs_interrupted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let session_id = rove_runtime::types::SessionId::new();
    let job_id = rove_runtime::types::JobId::new();
    let run_id = rove_runtime::types::RunId::new();
    state_store
        .start_run(session_id, job_id, run_id)
        .expect("running job should be indexed");

    let app = router(ApiState::new(workspace, test_config()));
    let state = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{job_id}/state"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body = axum::body::to_bytes(state.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Interrupted);
    assert_eq!(state.job_id, job_id);
    assert_eq!(state.run_id, run_id);

    let indexed_job = state_store.index.job_record(job_id).unwrap().unwrap();
    assert_eq!(indexed_job.status, "interrupted");
    let indexed_run = state_store.index.run_record(run_id).unwrap().unwrap();
    assert_eq!(indexed_run.status, "interrupted");
}

#[tokio::test]
async fn api_restart_marks_pending_approval_interrupted_without_replaying_unknown_step() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace.clone(), test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"pending.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap();
    assert_eq!(
        state_store
            .index
            .pending_approval_status(approval.call_id)
            .unwrap()
            .as_deref(),
        Some("pending")
    );

    let restarted = router(ApiState::new(workspace, test_config()));
    let state = restarted
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/state", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body = axum::body::to_bytes(state.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Interrupted);
    assert!(state.pending_approvals.is_empty());
    assert!(state.pending_inputs.is_empty());
    assert_eq!(
        state_store
            .index
            .pending_approval_status(approval.call_id)
            .unwrap()
            .as_deref(),
        Some("interrupted")
    );

    let resume_body = serde_json::json!({
        "message": "continue after interrupted approval",
        "model": "fake",
        "resume": "latest"
    });
    let resume = restarted
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(resume_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resume.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resume.into_body(), usize::MAX)
        .await
        .unwrap();
    let resumed: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    assert_ne!(resumed.run_id, created.run_id);
    assert_eq!(resumed.job_id, created.job_id);
    let resumed_state =
        wait_for_status(restarted, resumed.job_id.to_string(), RunStatus::Error).await;
    assert!(resumed_state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::StepResult { record }
                if record.status == StepRecordStatus::Interrupted
                    && record.error_code.as_deref() == Some("interrupted")
        )
    }));
    assert!(
        !resumed_state
            .events
            .iter()
            .any(|event| matches!(event.event, StreamEvent::PlanStepStarted { .. }))
    );
    assert!(
        !resumed_state
            .events
            .iter()
            .any(|event| matches!(event.event, StreamEvent::ToolCallStarted { .. }))
    );
    assert!(!tmp.path().join("pending.txt").exists());
}

#[tokio::test]
async fn api_restart_marks_pending_input_interrupted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace.clone(), test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "Which branch should I use?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    let input = pending.pending_inputs.first().unwrap();
    assert_eq!(
        state_store
            .index
            .pending_input_status(input.input_id)
            .unwrap()
            .as_deref(),
        Some("pending")
    );

    let restarted = router(ApiState::new(workspace, test_config()));
    let state = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/state", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body = axum::body::to_bytes(state.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Interrupted);
    assert!(state.pending_approvals.is_empty());
    assert!(state.pending_inputs.is_empty());
    assert_eq!(
        state_store
            .index
            .pending_input_status(input.input_id)
            .unwrap()
            .as_deref(),
        Some("interrupted")
    );
}

#[tokio::test]
async fn api_replays_input_needed_event_after_restart() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace.clone(), test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "Which branch should I use?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.status, RunStatus::Running);

    let mut indexed_events = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let events = state_store.index.event_records(created.run_id).unwrap();
        if events
            .iter()
            .any(|event| event.event_name == "input_needed")
        {
            indexed_events = Some(events);
            break;
        }
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    let indexed_events = indexed_events.expect("input_needed event should be persisted");
    assert_eq!(
        indexed_events
            .iter()
            .filter(|event| event.event_name == "input_needed")
            .count(),
        1
    );
    let trace_path = state_store
        .run_store
        .run_dir(&created.run_id)
        .join("trace.jsonl");
    let trace = std::fs::read_to_string(trace_path).unwrap();
    let outcome = rove_runtime::state::trace_reader::read_trace_content(&trace);
    let trace_input_count = outcome
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.entry,
                rove_runtime::events::TraceEntry::Ui(StreamEvent::InputNeeded { .. })
            )
        })
        .count();
    assert_eq!(trace_input_count, 1);
    assert!(!outcome.truncated_tail);

    let restarted = router(ApiState::new(workspace, test_config()));
    let events = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("event: input_needed"), "{text}");
    assert!(text.contains("Which branch should I use?"), "{text}");
    assert_eq!(text.matches("event: input_needed").count(), 1, "{text}");
}

#[tokio::test]
async fn api_can_cancel_job() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"cancel me","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let body = axum::body::to_bytes(cancel.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn api_cancel_does_not_rewrite_completed_job() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let run_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir).run_store;
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"already done","model":"fake"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let body = axum::body::to_bytes(cancel.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Done);

    let report_path = run_store.run_dir(&created.run_id).join("report.json");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "success");
    assert_eq!(report["termination_reason"], "final");
}

#[tokio::test]
async fn api_planned_tool_step_completes_after_successful_tool_call() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    std::fs::write(workspace.root.join("note.txt"), "planned tool done").unwrap();
    let run_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir).run_store;
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"read_file\",\"args\":{\"path\":\"note.txt\"}}","model":"fake-raw","approval":"auto","max_steps":3}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    let tool_starts = state
        .events
        .iter()
        .filter(|event| matches!(&event.event, StreamEvent::ToolCallStarted { name, .. } if name == "read_file"))
        .count();
    assert_eq!(tool_starts, 1);
    let result_index = state
        .events
        .iter()
        .position(|event| {
            matches!(
                &event.event,
                StreamEvent::StepResult { record }
                    if record.status == StepRecordStatus::Succeeded
                        && record.tool_calls_used == 1
            )
        })
        .expect("API job state should retain the canonical step_result event");
    assert!(
        state.events.iter().any(|event| {
            matches!(
                &event.event,
                StreamEvent::PlanDecision { record }
                    if record.trigger_step_record_id
                        == match &state.events[result_index].event {
                            StreamEvent::StepResult { record } => record.record_id.as_str(),
                            _ => "",
                        }
            )
        }),
        "successful planned tool step should emit a correlated plan_decision"
    );

    let events = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let sse = String::from_utf8(body.to_vec()).unwrap();
    assert!(sse.contains("event: step_result"));

    let report_path = run_store.run_dir(&created.run_id).join("report.json");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "success");
    assert_eq!(report["termination_reason"], "final");
    assert_eq!(report["tool_calls"], 1);
    // The finalizer synthesizes the answer from recorded evidence and cites the
    // tool call that produced it, instead of forwarding the raw model message.
    let output = report["output"].as_str().expect("report output");
    assert!(
        output.contains("planned tool done"),
        "finalized output must retain the step conclusion: {output}"
    );
    assert!(
        output.contains("Evidence: tool_call:"),
        "a tool-backed step must cite its evidence: {output}"
    );
    assert_eq!(report["final_outcome"], "success");
    assert_eq!(report["step_records"].as_array().unwrap().len(), 1);
    assert_eq!(report["step_records"][0]["status"], "succeeded");
}

#[tokio::test]
async fn api_approves_pending_destructive_tool_call() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("approved.txt");
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"approved.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_approval_event(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap();
    assert_eq!(approval.name, "write_file");
    assert!(pending.events.iter().any(|stored| {
        matches!(&stored.event, StreamEvent::ToolCallApprovalNeeded { call_id, .. } if *call_id == approval.call_id)
    }));
    assert!(!output_path.exists(), "tool should wait before approval");

    let approve = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/approvals/{}",
                    created.job_id, approval.call_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approve.status(), StatusCode::OK);

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert_eq!(std::fs::read_to_string(output_path).unwrap(), "ok");
}

#[tokio::test]
async fn api_persists_approval_before_releasing_destructive_tool() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("approval-commit-order.txt");
    let state_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir);
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"approval-commit-order.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();
    let pending = wait_for_approval_event(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap().clone();

    let connection = rusqlite::Connection::open(state_store.index.path()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let approve_app = app.clone();
    let job_id = created.job_id;
    let call_id = approval.call_id;
    let approval_task = tokio::spawn(async move {
        approve_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/jobs/{job_id}/approvals/{call_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"decision":"approve"}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !approval_task.is_finished(),
        "approval should wait for the durable commit"
    );
    assert!(
        !output_path.exists(),
        "tool must not run before approval is durable"
    );

    drop(connection);
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), approval_task)
        .await
        .expect("approval should finish after the lock is released")
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let state = wait_for_done(app, created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert_eq!(std::fs::read_to_string(output_path).unwrap(), "ok");
}

#[tokio::test]
async fn api_rejects_pending_destructive_tool_call() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("rejected.txt");
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"rejected.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap();
    assert_eq!(approval.name, "write_file");

    let reject = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/approvals/{}",
                    created.job_id, approval.call_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"reject"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reject.status(), StatusCode::OK);

    let state = wait_for_status(app.clone(), created.job_id.to_string(), RunStatus::Error).await;
    assert_eq!(state.status, RunStatus::Error);
    assert!(!output_path.exists(), "rejected tool should not run");

    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("event: tool_call_failed"));
}

#[tokio::test]
async fn api_planned_rejected_destructive_tool_does_not_replan_same_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"rejected-planned.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":3}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap();
    assert_eq!(approval.name, "write_file");

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/approvals/{}",
                    created.job_id, approval.call_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"reject"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::OK);

    let state = wait_for_status(app, created.job_id.to_string(), RunStatus::Error).await;
    assert_eq!(state.status, RunStatus::Error);
    assert!(state.pending_approvals.is_empty());
    let approval_requests = state
        .events
        .iter()
        .filter(|event| matches!(event.event, StreamEvent::ToolCallApprovalNeeded { .. }))
        .count();
    assert_eq!(approval_requests, 1);
    assert!(state.events.iter().any(|event| {
        matches!(
            &event.event,
            StreamEvent::ToolCallFailed {
                error: rove_core::ToolError::PermissionDenied { .. },
                ..
            }
        )
    }));
}

#[tokio::test]
async fn api_cancel_clears_pending_destructive_tool_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("cancelled.txt");
    let run_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir).run_store;
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"cancelled.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.pending_approvals[0].name, "write_file");

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let body = axum::body::to_bytes(cancel.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(state.status, RunStatus::Cancelled);
    assert!(state.pending_approvals.is_empty());
    assert!(
        !output_path.exists(),
        "cancelled pending tool should not run"
    );

    let run_dir = run_store.run_dir(&created.run_id);
    let report_path = run_dir.join("report.json");
    assert!(
        report_path.exists(),
        "cancelled jobs should still write report.json"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "cancelled");
    assert_eq!(report["termination_reason"], "cancelled");
}

#[tokio::test]
async fn api_shutdown_token_cancels_pending_job_and_clears_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("shutdown-cancelled.txt");
    let run_store = rove_runtime::state::store::StateStore::new(&workspace.state_dir).run_store;
    let shutdown = CancellationToken::new();
    let app = router(ApiState::with_shutdown(
        workspace,
        test_config(),
        shutdown.clone(),
    ));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"shutdown-cancelled.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.pending_approvals[0].name, "write_file");

    shutdown.cancel();
    let state = wait_for_status(
        app.clone(),
        created.job_id.to_string(),
        RunStatus::Cancelled,
    )
    .await;

    assert!(state.pending_approvals.is_empty());
    assert!(
        !output_path.exists(),
        "shutdown-cancelled pending tool should not run"
    );

    let report_path = run_store.run_dir(&created.run_id).join("report.json");
    assert!(
        report_path.exists(),
        "shutdown-cancelled jobs should still write report.json"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "cancelled");
    assert_eq!(report["termination_reason"], "cancelled");
}

#[tokio::test]
async fn api_defaults_to_ask_for_destructive_tool_calls() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("default-ask.txt");
    let mut config = test_config();
    config.state.sqlite_busy_timeout_ms = 0;
    let app = router(ApiState::new(workspace, config));

    let unavailable = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/preferences")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"default-ask.txt\",\"content\":\"safe\"}}","model":"fake-raw","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_pending_approval(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.pending_approvals[0].name, "write_file");
    assert!(!output_path.exists(), "default approval should wait");
    let cancelled = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_auto_approval_runs_destructive_tool_without_pending_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("auto.txt");
    let mut config = test_config();
    config.state.sqlite_busy_timeout_ms = 0;
    let app = router(ApiState::new(workspace, config));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"write_file\",\"args\":{\"path\":\"auto.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"auto","max_steps":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert!(state.pending_approvals.is_empty());
    let output = std::fs::read_to_string(&output_path)
        .unwrap_or_else(|error| panic!("{error}; tool events: {:#?}", state.events));
    assert_eq!(output, "ok");
}

#[tokio::test]
async fn api_registers_save_memory_tool_for_jobs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let topic_path = workspace
        .root
        .join(".rove")
        .join("memory")
        .join("topics")
        .join("api-facts.md");
    let index_path = workspace
        .root
        .join(".rove")
        .join("memory")
        .join("MEMORY.md");
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "save_memory",
        "args": {
            "topic": "API Facts",
            "content": "API jobs can persist durable memory.",
            "type": "project"
        }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&create_body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let topic = std::fs::read_to_string(topic_path).unwrap();
    assert!(topic.contains("API jobs can persist durable memory."));
    let index = std::fs::read_to_string(index_path).unwrap();
    assert!(index.contains("[API Facts](topics/api-facts.md)"));
}

#[tokio::test]
async fn api_registers_memory_index_and_topic_read_tools_for_jobs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        topics_dir.join("manual-topic.md"),
        "---\ntitle: Manual Topic\ntype: reference\ncreated_at: 2026-05-23T00:00:00Z\nupdated_at: 2026-05-23T00:00:00Z\n---\n\nManual durable fact from API.\n",
    )
    .unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let update_message = serde_json::json!({
        "tool": "reindex_memory",
        "args": {}
    })
    .to_string();
    let update_body = serde_json::json!({
        "message": update_message,
        "model": "fake-raw",
        "max_steps": 1
    });
    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(update_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let update_body = axum::body::to_bytes(update.into_body(), usize::MAX)
        .await
        .unwrap();
    let updated: CreateJobResponse = serde_json::from_slice(&update_body).unwrap();

    let state = wait_for_done(app.clone(), updated.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    let index = std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap();
    assert!(index.contains("[Manual Topic](topics/manual-topic.md)"));

    let read_message = serde_json::json!({
        "tool": "read_memory",
        "args": { "name": "Manual Topic" }
    })
    .to_string();
    let read_body = serde_json::json!({
        "message": read_message,
        "model": "fake-raw",
        "max_steps": 1
    });
    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(read_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let read_body = axum::body::to_bytes(read.into_body(), usize::MAX)
        .await
        .unwrap();
    let read_created: CreateJobResponse = serde_json::from_slice(&read_body).unwrap();

    let state = wait_for_done(app.clone(), read_created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    let events = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", read_created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let events_body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(events_body.to_vec()).unwrap();
    assert!(text.contains("Manual durable fact from API."));
}

#[tokio::test]
async fn api_debug_memory_lists_topics_and_scores_recall() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        topics_dir.join("db-config.md"),
        "---\ntitle: 数据库配置\ntype: project\nscope: project\nsource: test\nconfidence: 0.90\ncreated_at: 2026-07-03T00:00:00Z\nupdated_at: 2026-07-03T00:00:00Z\n---\n\nMySQL 数据库连接字符串使用 DATABASE_URL。\n",
    )
    .unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [数据库配置](topics/db-config.md) — project project memory\n",
    )
    .unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/debug/memory")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list_json["total"], 1);
    assert_eq!(list_json["topics"][0]["slug"], "db-config");
    assert_eq!(list_json["topics"][0]["memory_type"], "project");

    let topic = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/debug/memory/topics/db-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(topic.status(), StatusCode::OK);
    let body = axum::body::to_bytes(topic.into_body(), usize::MAX)
        .await
        .unwrap();
    let topic_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        topic_json["content"]
            .as_str()
            .is_some_and(|content| content.contains("DATABASE_URL"))
    );

    let recall = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/debug/memory/recall")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "query": "数据库",
                        "type_filter": "project",
                        "limit": 5
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recall.status(), StatusCode::OK);
    let body = axum::body::to_bytes(recall.into_body(), usize::MAX)
        .await
        .unwrap();
    let recall_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(recall_json["total_hits"], 1);
    assert_eq!(recall_json["hits"][0]["slug"], "db-config");
    assert!(
        recall_json["hits"][0]["score"]
            .as_f64()
            .is_some_and(|score| score > 0.0)
    );
}

#[tokio::test]
async fn api_registers_configured_mcp_tools_for_jobs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let config_dir = workspace.root.join(".rove");
    std::fs::create_dir_all(&config_dir).unwrap();
    let mcp_config_path = config_dir.join("mcp_servers.json");
    std::fs::write(
        &mcp_config_path,
        serde_json::json!({
            "servers": [{
                "name": "mock-server",
                "transport": "stdio",
                "command": python_command(),
                "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")]
            }]
        })
        .to_string(),
    )
    .unwrap();
    let mut config = test_config();
    config.tool.mcp_config_path = mcp_config_path;
    let app = router(ApiState::new(workspace, config));
    let message = serde_json::json!({
        "tool": "mcp__mock_server__echo_remote",
        "args": { "message": "hello api mcp" }
    })
    .to_string();
    // A remote `readOnlyHint` is not a local policy grant, so MCP tools stay
    // destructive locally. This case covers registration and execution, so it
    // grants approval explicitly; the approval requirement itself is asserted by
    // `product_mcp_crud_is_workspace_scoped_secret_free_and_used_by_product_jobs`.
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "approval": "auto",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    let events = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("remote: hello api mcp"), "{text}");
}

#[tokio::test]
async fn product_mcp_first_write_promotes_legacy_into_marker_bound_contract_state() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let data = tempfile::TempDir::new().unwrap();
    let legacy_dir = folder.path().join(".rove");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("mcp_servers.json");
    std::fs::write(
        &legacy_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "servers": [{
                "name": "legacy_server",
                "transport": "streamable_http",
                "url": "http://127.0.0.1:9/mcp",
                "policy": {
                    "request_timeout_ms": 2_000,
                    "stderr_capture_bytes": 16_384
                }
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let roots = UserStateRoots::from_root(data.path());
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    config.tool.mcp_config_path.clear();
    config.data_root_override = Some(data.path().to_path_buf());
    config.user_state_roots = Some(roots.clone());
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let created = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_id}"),
        serde_json::json!({
            "name": "contract_server",
            "transport": "streamable_http",
            "url": "http://127.0.0.1:10/mcp",
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let layout = roots.workspace_layout(&folder.path().canonicalize().unwrap());
    layout.verify_marker().unwrap();
    let contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&layout.mcp_catalog).unwrap()).unwrap();
    let names = contract["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| server["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, BTreeSet::from(["contract_server", "legacy_server"]));

    let legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&legacy_path).unwrap()).unwrap();
    assert_eq!(legacy["servers"].as_array().unwrap().len(), 1);
    assert_eq!(legacy["servers"][0]["name"], "legacy_server");
}

#[tokio::test]
async fn product_mcp_crud_is_workspace_scoped_secret_free_and_used_by_product_jobs() {
    let server = tempfile::TempDir::new().unwrap();
    let folder_a = tempfile::TempDir::new().unwrap();
    let folder_b = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace_a = create_product_workspace(&app, folder_a.path()).await;
    let workspace_a_id = workspace_a["id"].as_str().unwrap();
    let workspace_b = create_product_workspace(&app, folder_b.path()).await;
    let workspace_b_id = workspace_b["id"].as_str().unwrap();
    let config_path = folder_a.path().join(".rove/mcp_servers.json");
    let secret_canary = "sk-rove-mcp-secret-canary-058761eb";

    for unsafe_body in [
        serde_json::json!({
            "name": "raw_env",
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")],
            "env": {"ROVE_SECRET": secret_canary}
        }),
        serde_json::json!({
            "name": "secret_arg",
            "transport": "stdio",
            "command": python_command(),
            "args": [
                workspace_path_string("tests/fixtures/mcp_mock_server.py"),
                format!("--token={secret_canary}")
            ]
        }),
    ] {
        let rejected = post_json(
            &app,
            &format!("/product/mcp/servers?workspace_id={workspace_a_id}"),
            unsafe_body,
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let error: serde_json::Value = decode_json(rejected).await;
        assert!(!error.to_string().contains(secret_canary));
    }
    assert!(!config_path.exists());

    let created = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "name": "mock_server",
            "enabled": true,
            "required": true,
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")],
            "env_names": ["PATH"],
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = decode_json(created).await;
    assert_eq!(created["name"], "mock_server");
    assert_eq!(created["required"], true);
    assert_eq!(created["env_names"], serde_json::json!(["PATH"]));
    assert!(created.get("env").is_none());
    // The server owns the deprecation verdict for every transport it returns.
    assert_eq!(created["transport_deprecated"], serde_json::json!(false));
    // A client cannot declare it: the field is unknown on a create request.
    let declared = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "name": "declares_deprecation",
            "transport": "streamable_http",
            "url": "https://mcp.example.com/mcp",
            "transport_deprecated": false
        }),
    )
    .await;
    assert_eq!(declared.status(), StatusCode::BAD_REQUEST);
    // Legacy SSE is reported deprecated, the current HTTP transport is not.
    for (name, transport, url, deprecated) in [
        ("legacy_sse", "sse", "https://mcp.example.com/sse", true),
        (
            "streaming_http",
            "streamable_http",
            "https://mcp.example.com/mcp",
            false,
        ),
    ] {
        let created = post_json(
            &app,
            &format!("/product/mcp/servers?workspace_id={workspace_a_id}"),
            serde_json::json!({
                "name": name,
                "transport": transport,
                "url": url,
                "request_timeout_ms": 2_000
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created: serde_json::Value = decode_json(created).await;
        assert_eq!(created["transport"], transport);
        assert_eq!(created["url"], url);
        assert_eq!(
            created["transport_deprecated"],
            serde_json::json!(deprecated)
        );
        let deleted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/product/mcp/servers/{name}?workspace_id={workspace_a_id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    let persisted = std::fs::read_to_string(&config_path).unwrap();
    let persisted_json: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(
        persisted_json["servers"][0]["env_names"],
        serde_json::json!(["PATH"])
    );
    assert!(persisted_json["servers"][0].get("env").is_none());
    assert!(!persisted.contains(secret_canary));

    let duplicate = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "name": "mock_server",
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")]
        }),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate: serde_json::Value = decode_json(duplicate).await;
    assert_eq!(duplicate["code"], "product_mcp_conflict");

    let listed_b = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/mcp/servers?workspace_id={workspace_b_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed_b.status(), StatusCode::OK);
    let listed_b: serde_json::Value = decode_json(listed_b).await;
    assert_eq!(listed_b["servers"], serde_json::json!([]));

    let probe = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/mcp/servers/mock_server/probe?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(probe.status(), StatusCode::OK);
    let probe: serde_json::Value = decode_json(probe).await;
    assert_eq!(probe["tools"].as_array().unwrap().len(), 2);
    assert_eq!(probe["tools"][0]["destructive"], true);
    assert_eq!(probe["tools"][0]["parallel_safe"], false);
    assert!(!probe.to_string().contains(secret_canary));

    let health = get_response(
        &app,
        &format!("/product/mcp/health?workspace_id={workspace_a_id}"),
    )
    .await;
    assert_eq!(health.status(), StatusCode::OK);
    let health: serde_json::Value = decode_json(health).await;
    assert_eq!(health["total"], 1);
    assert_eq!(health["servers"][0]["server_name"], "mock_server");
    assert_eq!(health["servers"][0]["required"], true);
    assert_eq!(health["servers"][0]["status"], "unknown");
    assert_eq!(health["servers"][0]["tool_count"], 0);
    assert!(health["servers"][0].get("server_config_hash").is_none());

    let session = create_product_session(&app, workspace_a_id, "MCP product job").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake-raw", 1).await;
    let message = serde_json::json!({
        "tool": "mcp__mock_server__echo_remote",
        "args": {"message": "product MCP catalog"}
    })
    .to_string();
    let job = create_product_job(&app, session_id, &message).await;
    let approval_state = wait_for_pending_approval(app.clone(), job.job_id.to_string()).await;
    let approval = approval_state.pending_approvals.first().unwrap();
    assert_eq!(approval.name, "mcp__mock_server__echo_remote");
    let approved = post_json(
        &app,
        &format!("/jobs/{}/approvals/{}", job.job_id, approval.call_id),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(approved.status(), StatusCode::OK);
    let completed = wait_for_done(app.clone(), job.job_id.to_string()).await;
    assert!(completed.events.iter().any(|stored| matches!(
        &stored.event,
        StreamEvent::ToolCallCompleted { result, .. }
            if result.output == "remote: product MCP catalog"
    )));

    let health = get_response(
        &app,
        &format!("/product/mcp/health?workspace_id={workspace_a_id}"),
    )
    .await;
    assert_eq!(health.status(), StatusCode::OK);
    let health: serde_json::Value = decode_json(health).await;
    let ready = &health["servers"][0];
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["tool_count"], 2);
    assert_eq!(ready["protocol_version"], "2025-06-18");
    assert!(
        ready["server_config_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert!(
        ready["capability_snapshot_id"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert!(!health.to_string().contains(secret_canary));

    let disabled = request_json(
        &app,
        "PUT",
        &format!("/product/mcp/servers/mock_server?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "enabled": false,
            "required": true,
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")],
            "env_names": ["PATH"],
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::OK);
    let disabled: serde_json::Value = decode_json(disabled).await;
    assert_eq!(disabled["enabled"], false);

    let health = get_response(
        &app,
        &format!("/product/mcp/health?workspace_id={workspace_a_id}"),
    )
    .await;
    assert_eq!(health.status(), StatusCode::OK);
    let health: serde_json::Value = decode_json(health).await;
    assert_eq!(health["servers"][0]["status"], "disabled");
    assert_eq!(health["servers"][0]["tool_count"], 0);
    assert!(health["servers"][0].get("refreshed_at").is_none());

    let disabled_session =
        create_product_session(&app, workspace_a_id, "Disabled MCP product job").await;
    let disabled_session_id = disabled_session["id"].as_str().unwrap();
    configure_product_session_model(&app, disabled_session_id, "fake-raw", 1).await;
    let disabled_job = create_product_job(&app, disabled_session_id, &message).await;
    let disabled_state = wait_for_done(app.clone(), disabled_job.job_id.to_string()).await;
    assert!(disabled_state.events.iter().any(|stored| matches!(
        &stored.event,
        StreamEvent::ToolCallFailed { error, .. }
            if error.to_string().contains("Unknown tool")
    )));
    assert!(!disabled_state.events.iter().any(|stored| matches!(
        &stored.event,
        StreamEvent::ToolCallCompleted { result, .. }
            if result.output == "remote: product MCP catalog"
    )));

    let optional = request_json(
        &app,
        "PUT",
        &format!("/product/mcp/servers/mock_server?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "enabled": true,
            "required": false,
            "transport": "stdio",
            "command": "rove-command-that-does-not-exist-058761eb",
            "args": [],
            "env_names": [],
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    assert_eq!(optional.status(), StatusCode::OK);
    let optional: serde_json::Value = decode_json(optional).await;
    assert_eq!(optional["required"], false);
    let optional_session =
        create_product_session(&app, workspace_a_id, "Optional MCP degradation").await;
    let optional_session_id = optional_session["id"].as_str().unwrap();
    configure_product_session_model(&app, optional_session_id, "fake", 1).await;
    let optional_job = create_product_job(&app, optional_session_id, "inspect safely").await;
    let optional_state = wait_for_done(app.clone(), optional_job.job_id.to_string()).await;
    assert_eq!(optional_state.status, RunStatus::Done);

    let health = get_response(
        &app,
        &format!("/product/mcp/health?workspace_id={workspace_a_id}"),
    )
    .await;
    assert_eq!(health.status(), StatusCode::OK);
    let health: serde_json::Value = decode_json(health).await;
    let degraded = &health["servers"][0];
    assert_eq!(degraded["required"], false);
    assert_eq!(degraded["status"], "degraded");
    assert_eq!(degraded["tool_count"], 0);
    assert!(degraded["failure_code"].as_str().is_some_and(|code| {
        matches!(code, "mcp_activation_failed" | "mcp_activation_unavailable")
    }));
    assert!(
        !health
            .to_string()
            .contains("rove-command-that-does-not-exist")
    );

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/mcp/servers/mock_server?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/mcp/servers/mock_server?workspace_id={workspace_a_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn restricted_product_workspace_cannot_probe_or_activate_mcp() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = AppConfig::load_with_user_config_paths(
        server.path(),
        AppConfigOverrides {
            model: Some("fake".to_string()),
            data_root: Some(server.path().join("data-root")),
            ..AppConfigOverrides::default()
        },
        UserConfigPaths::from_root(server.path().join("user-config")),
    )
    .unwrap();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let created = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_id}"),
        serde_json::json!({
            "name": "blocked",
            "transport": "stdio",
            "command": "rove-command-that-does-not-exist-058761eb",
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let probe = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/mcp/servers/blocked/probe?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(probe.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(probe).await;
    assert_eq!(error["code"], "project_trust_required");
    assert!(!error.to_string().contains("058761eb"));

    let session = create_product_session(&app, workspace_id, "Restricted project").await;
    let session_id = session["id"].as_str().unwrap();
    configure_product_session_model(&app, session_id, "fake", 1).await;
    let job = create_product_job(&app, session_id, "inspect safely").await;
    assert_eq!(
        job.workspace_activation,
        WorkspaceActivationState::Restricted
    );
    let state = wait_for_done(app, job.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
}

#[tokio::test]
async fn product_mcp_probe_returns_typed_stdio_failures() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let mock = workspace_path_string("tests/fixtures/mcp_mock_server.py");
    let hanging = workspace_path_string("tests/fixtures/mcp_hanging_server.py");
    let fixture_timeout_ms = 2_000_u64;
    let cases = [
        (
            "missing_env",
            python_command().to_string(),
            vec![mock.clone()],
            vec!["ROVE_MCP_ENV_MISSING_058761EB"],
            fixture_timeout_ms,
            StatusCode::BAD_REQUEST,
            "product_mcp_environment_missing",
        ),
        (
            "spawn_failure",
            "rove-command-that-does-not-exist-058761eb".to_string(),
            Vec::new(),
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_spawn_failed",
        ),
        (
            "timeout",
            python_command().to_string(),
            vec![hanging],
            Vec::new(),
            100,
            StatusCode::GATEWAY_TIMEOUT,
            "product_mcp_timeout",
        ),
        (
            "transport",
            python_command().to_string(),
            vec![mock.clone(), "--close".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_transport",
        ),
        (
            "protocol",
            python_command().to_string(),
            vec![mock.clone(), "--invalid-protocol".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_protocol_mismatch",
        ),
        (
            "oversized_line",
            python_command().to_string(),
            vec![mock.clone(), "--oversized-line".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_protocol_mismatch",
        ),
        (
            "no_tools",
            python_command().to_string(),
            vec![mock.clone(), "--no-tools".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_no_tools",
        ),
        (
            "empty_tool_name",
            python_command().to_string(),
            vec![mock.clone(), "--empty-tool-name".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_protocol_mismatch",
        ),
        (
            "too_many_tools",
            python_command().to_string(),
            vec![mock, "--too-many-tools".to_string()],
            Vec::new(),
            fixture_timeout_ms,
            StatusCode::BAD_GATEWAY,
            "product_mcp_protocol_mismatch",
        ),
    ];

    for (name, command, args, env_names, timeout_ms, status, code) in cases {
        let created = post_json(
            &app,
            &format!("/product/mcp/servers?workspace_id={workspace_id}"),
            serde_json::json!({
                "name": name,
                "transport": "stdio",
                "command": command,
                "args": args,
                "env_names": env_names,
                "request_timeout_ms": timeout_ms
            }),
        )
        .await;
        let created_status = created.status();
        let created_body: serde_json::Value = decode_json(created).await;
        assert_eq!(
            created_status,
            StatusCode::CREATED,
            "case {name}: {created_body}"
        );
        let probe = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/product/mcp/servers/{name}/probe?workspace_id={workspace_id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(probe.status(), status, "case {name}");
        let error: serde_json::Value = decode_json(probe).await;
        assert_eq!(error["code"], code, "case {name}: {error}");
        assert!(!error.to_string().contains("058761EB"));
    }
}

#[tokio::test]
async fn product_mcp_probe_discovers_tools_over_legacy_sse() {
    let mcp_router = Router::new()
        .route(
            "/sse",
            get(|| async { ([(CONTENT_TYPE, "text/event-stream")], "data: /messages\n\n") }),
        )
        .route(
            "/messages",
            post(|Json(message): Json<serde_json::Value>| async move {
                let method = message["method"].as_str().unwrap_or_default();
                let result = match method {
                    "initialize" => serde_json::json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "legacy_sse", "version": "1"}
                    }),
                    "tools/list" => serde_json::json!({
                        "tools": [{
                            "name": "legacy_echo",
                            "description": "Legacy SSE echo",
                            "inputSchema": {"type": "object"},
                            "annotations": {"readOnlyHint": true}
                        }]
                    }),
                    _ => serde_json::json!({}),
                };
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": message.get("id").cloned().unwrap_or(serde_json::Value::Null),
                    "result": result
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, mcp_router).await.unwrap();
    });

    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let created = post_json(
        &app,
        &format!("/product/mcp/servers?workspace_id={workspace_id}"),
        serde_json::json!({
            "name": "legacy_sse",
            "transport": "sse",
            "url": format!("http://{address}/sse"),
            "request_timeout_ms": 2_000
        }),
    )
    .await;
    let created_status = created.status();
    let created_body: serde_json::Value = decode_json(created).await;
    assert_eq!(created_status, StatusCode::CREATED, "{created_body}");

    let probe = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/product/mcp/servers/legacy_sse/probe?workspace_id={workspace_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    server_task.abort();
    assert_eq!(probe.status(), StatusCode::OK);
    let probe: serde_json::Value = decode_json(probe).await;
    assert_eq!(probe["transport"], "sse");
    assert_eq!(probe["tools"][0]["name"], "legacy_echo");
    assert_eq!(probe["tools"][0]["destructive"], true);
    assert_eq!(probe["tools"][0]["parallel_safe"], false);
}

#[tokio::test]
async fn product_long_session_deep_tree_large_dir_and_large_diff_stay_bounded() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(&app, &workspace_id, "Load smoke").await;
    let session_id = session["id"].as_str().unwrap().to_string();

    // --- Long session: consecutive turns keep exact ordinal and lineage. ---
    const TURNS: u64 = 6;
    let mut run_ids = Vec::new();
    for turn in 1..=TURNS {
        let created = create_product_job(&app, &session_id, &format!("load turn {turn}")).await;
        let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
        assert_eq!(state.status, RunStatus::Done);
        run_ids.push(created.run_id.to_string());
        let observed =
            wait_for_product_session_status(&app, &workspace_id, &session_id, "idle").await;
        assert_eq!(
            observed["runtime_binding"]["ordinal"], turn,
            "ordinal must advance exactly once per turn"
        );
    }
    assert_eq!(run_ids.len() as u64, TURNS);
    let unique: std::collections::HashSet<&String> = run_ids.iter().collect();
    assert_eq!(
        unique.len(),
        run_ids.len(),
        "each turn needs its own run id"
    );

    let transcript: serde_json::Value = decode_json(
        get_response(&app, &format!("/product/sessions/{session_id}/transcript")).await,
    )
    .await;
    let segments = transcript["segments"].as_array().unwrap();
    assert_eq!(segments.len() as u64, TURNS);
    let ordinals: Vec<u64> = segments
        .iter()
        .map(|segment| segment["binding"]["ordinal"].as_u64().unwrap())
        .collect();
    assert_eq!(ordinals, (1..=TURNS).collect::<Vec<_>>());

    // --- Deep tree: a deeply nested prefix resolves without unbounded walking. ---
    const DEPTH: usize = 24;
    let mut deep = folder.path().to_path_buf();
    let mut deep_prefix = String::new();
    for level in 0..DEPTH {
        deep = deep.join(format!("level{level:02}"));
        if !deep_prefix.is_empty() {
            deep_prefix.push('/');
        }
        deep_prefix.push_str(&format!("level{level:02}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("leaf.txt"), b"deep leaf").unwrap();

    let deep_listing: serde_json::Value = decode_json(
        get_response(
            &app,
            &format!("/product/workspaces/{workspace_id}/files?prefix={deep_prefix}"),
        )
        .await,
    )
    .await;
    let deep_entries = deep_listing["entries"].as_array().unwrap();
    assert_eq!(deep_entries.len(), 1);
    assert!(
        deep_entries[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("leaf.txt"),
        "deep listing must resolve the leaf: {deep_listing}"
    );
    assert_eq!(deep_listing["scan_limit_reached"], false);

    // Escaping upward from a deep prefix must still be refused.
    let escape = get_response(
        &app,
        &format!("/product/workspaces/{workspace_id}/files?prefix={deep_prefix}/../../../.."),
    )
    .await;
    assert!(
        escape.status() == StatusCode::BAD_REQUEST || escape.status() == StatusCode::NOT_FOUND,
        "traversal from a deep prefix must not succeed: {}",
        escape.status()
    );

    // --- Large dir: more entries than one page; pagination must be exact. ---
    const WIDE: usize = 250;
    let wide_dir = folder.path().join("wide");
    std::fs::create_dir_all(&wide_dir).unwrap();
    for index in 0..WIDE {
        std::fs::write(wide_dir.join(format!("entry{index:04}.txt")), b"x").unwrap();
    }
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let uri = match &cursor {
            Some(value) => format!(
                "/product/workspaces/{workspace_id}/files?prefix=wide&limit=100&cursor={value}"
            ),
            None => {
                format!("/product/workspaces/{workspace_id}/files?prefix=wide&limit=100")
            }
        };
        let page: serde_json::Value = decode_json(get_response(&app, &uri).await).await;
        assert_eq!(page["scan_limit_reached"], false);
        for entry in page["entries"].as_array().unwrap() {
            seen.push(entry["path"].as_str().unwrap().to_string());
        }
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => {
                assert_eq!(page["truncated"], false, "final page must not be truncated");
                cursor = None;
                break;
            }
        }
    }
    assert!(cursor.is_none(), "pagination did not terminate");
    assert_eq!(seen.len(), WIDE, "pagination must cover every entry once");
    let unique_paths: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(unique_paths.len(), WIDE, "pages must not overlap");
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(sorted, seen, "paged order must stay stable and sorted");

    // --- Large diff: more mutations than the entry cap; must cap, not balloon. ---
    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let last_run: RunId = run_ids.last().unwrap().parse().unwrap();
    let mut report = state_store.load_report(last_run).await.unwrap();
    const MUTATIONS: usize = 4200;
    for index in 0..MUTATIONS {
        report.tool_mutations.push(ToolMutation {
            path: format!("src/generated/file{index:05}.rs"),
            operation: ToolMutationOperation::Update,
            diff: Some(format!(
                "--- a/src/generated/file{index:05}.rs\n+++ b/src/generated/file{index:05}.rs\n@@ -1 +1 @@\n-old{index}\n+new{index}\n"
            )),
        });
    }
    rove_runtime::state::report::write_report(&state_store.run_store.run_dir(&last_run), &report)
        .unwrap();

    let diff: serde_json::Value = decode_json(
        get_response(
            &app,
            &format!("/product/sessions/{session_id}/diff?scope=run"),
        )
        .await,
    )
    .await;
    let entries = diff["entries"].as_array().unwrap();
    // MUTATIONS exceeds the 4096 entry cap, so the cap must be hit exactly:
    // a bare `<= 4096` would also pass on an empty response.
    assert_eq!(
        entries.len(),
        4096,
        "run diff must cap at exactly the declared entry limit"
    );
    let reasons = diff["partial_reasons"].as_array().unwrap();
    assert!(
        reasons
            .iter()
            .any(|reason| reason.as_str().unwrap_or_default().contains("capped")),
        "a capped diff must say so: {reasons:?}"
    );
    let total_diff_bytes: usize = entries
        .iter()
        .filter_map(|entry| entry["diff"].as_str())
        .map(str::len)
        .sum();
    assert!(
        total_diff_bytes <= 4 * 1024 * 1024,
        "total diff bytes must stay within the declared budget, saw {total_diff_bytes}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn product_concurrent_multi_workspace_control_operations_stay_isolated_and_serialized() {
    let server = tempfile::TempDir::new().unwrap();
    let folder_a = tempfile::TempDir::new().unwrap();
    let folder_b = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    let workspace_a = create_product_workspace(&app, folder_a.path()).await;
    let workspace_a_id = workspace_a["id"].as_str().unwrap().to_string();
    let workspace_b = create_product_workspace(&app, folder_b.path()).await;
    let workspace_b_id = workspace_b["id"].as_str().unwrap().to_string();

    let session_a = create_product_session(&app, &workspace_a_id, "Concurrent A").await;
    let session_a_id = session_a["id"].as_str().unwrap().to_string();
    let session_b = create_product_session(&app, &workspace_b_id, "Concurrent B").await;
    let session_b_id = session_b["id"].as_str().unwrap().to_string();

    configure_product_session_model(&app, &session_a_id, "fake-raw", 2).await;
    configure_product_session_model(&app, &session_b_id, "fake-raw", 2).await;

    // Bound up front: a `format!` temporary cannot outlive a `tokio::join!` arm.
    let steers_a_uri = format!("/product/sessions/{session_a_id}/steers");
    let steers_b_uri = format!("/product/sessions/{session_b_id}/steers");
    let followups_a_uri = format!("/product/sessions/{session_a_id}/followups");
    let followups_b_uri = format!("/product/sessions/{session_b_id}/followups");
    let controls_a_uri = format!("/product/sessions/{session_a_id}/controls");
    let controls_b_uri = format!("/product/sessions/{session_b_id}/controls");
    let model_a_uri = format!("/product/sessions/{session_a_id}/model-config");
    let model_b_uri = format!("/product/sessions/{session_b_id}/model-config");
    let forks_a_uri = format!("/product/sessions/{session_a_id}/forks");
    let forks_b_uri = format!("/product/sessions/{session_b_id}/forks");
    let sessions_a_uri = format!("/product/sessions?workspace_id={workspace_a_id}");
    let sessions_b_uri = format!("/product/sessions?workspace_id={workspace_b_id}");

    // Hold both sessions at a pending input so steer and follow-up land on live
    // runs in both workspaces at the same time.
    let mut active = Vec::new();
    for session_id in [&session_a_id, &session_b_id] {
        let response = post_json(
            &app,
            "/jobs",
            serde_json::json!({
                "message": serde_json::json!({
                    "tool": "request_input",
                    "args": { "prompt": "hold for concurrent controls" }
                })
                .to_string(),
                "product_session_id": session_id
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        active.push(decode_json::<CreateJobResponse>(response).await);
    }
    let mut active = active.into_iter();
    let job_a = active.next().unwrap();
    let job_b = active.next().unwrap();
    let pending_a = wait_for_pending_input(app.clone(), job_a.job_id.to_string()).await;
    let pending_b = wait_for_pending_input(app.clone(), job_b.job_id.to_string()).await;
    let input_a = pending_a.pending_inputs.first().unwrap().input_id;
    let input_b = pending_b.pending_inputs.first().unwrap().input_id;

    // Fire steer and follow-up against both workspaces concurrently. Each
    // control names its own session, so none may appear in the other workspace.
    let steer_a = post_json(
        &app,
        &steers_a_uri,
        serde_json::json!({ "content": "steer-for-a", "idempotency_key": "concurrent-steer-a" }),
    );
    let steer_b = post_json(
        &app,
        &steers_b_uri,
        serde_json::json!({ "content": "steer-for-b", "idempotency_key": "concurrent-steer-b" }),
    );
    let followup_a = post_json(
        &app,
        &followups_a_uri,
        serde_json::json!({ "content": "followup-for-a", "idempotency_key": "concurrent-followup-a" }),
    );
    let followup_b = post_json(
        &app,
        &followups_b_uri,
        serde_json::json!({ "content": "followup-for-b", "idempotency_key": "concurrent-followup-b" }),
    );
    let (steer_a, steer_b, followup_a, followup_b) =
        tokio::join!(steer_a, steer_b, followup_a, followup_b);
    for response in [&steer_a, &steer_b, &followup_a, &followup_b] {
        assert_eq!(response.status(), StatusCode::CREATED);
    }
    let steer_a: serde_json::Value = decode_json(steer_a).await;
    let steer_b: serde_json::Value = decode_json(steer_b).await;
    let followup_a: serde_json::Value = decode_json(followup_a).await;
    let followup_b: serde_json::Value = decode_json(followup_b).await;

    // Concurrently created controls in different workspaces must be distinct
    // records. Without this, a server that returned one shared control id for
    // both workspaces would still satisfy the idempotency assertions below.
    let control_ids = [
        steer_a["id"].as_str().unwrap(),
        steer_b["id"].as_str().unwrap(),
        followup_a["id"].as_str().unwrap(),
        followup_b["id"].as_str().unwrap(),
    ];
    let distinct: std::collections::HashSet<&str> = control_ids.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        control_ids.len(),
        "concurrent controls across workspaces must not share ids: {control_ids:?}"
    );

    // Concurrent duplicate submissions under one key must not create a second
    // control in either workspace.
    let (replay_a, replay_b) = tokio::join!(
        post_json(
            &app,
            &steers_a_uri,
            serde_json::json!({ "content": "steer-for-a", "idempotency_key": "concurrent-steer-a" }),
        ),
        post_json(
            &app,
            &followups_b_uri,
            serde_json::json!({ "content": "followup-for-b", "idempotency_key": "concurrent-followup-b" }),
        )
    );
    assert_eq!(replay_a.status(), StatusCode::OK);
    assert_eq!(replay_b.status(), StatusCode::OK);
    let replay_a: serde_json::Value = decode_json(replay_a).await;
    let replay_b: serde_json::Value = decode_json(replay_b).await;
    assert_eq!(replay_a["id"], steer_a["id"]);
    assert_eq!(replay_b["id"], followup_b["id"]);
    // A replay must not be mistaken for the other workspace's control.
    assert_ne!(replay_a["id"], steer_b["id"]);
    assert_ne!(replay_b["id"], followup_a["id"]);

    // Concurrent model-config writes against one session with the same expected
    // revision: exactly one may win, the loser must be a typed CAS conflict.
    let current = get_response(&app, &model_a_uri).await;
    assert_eq!(current.status(), StatusCode::OK);
    let current: serde_json::Value = decode_json(current).await;
    let expected_revision = current["revision"].clone();
    let (first_write, second_write) = tokio::join!(
        request_json(
            &app,
            "PUT",
            &model_a_uri,
            serde_json::json!({
                "model": "fake-raw",
                "reasoning": "default",
                "max_steps": 5,
                "expected_revision": expected_revision
            }),
        ),
        request_json(
            &app,
            "PUT",
            &model_a_uri,
            serde_json::json!({
                "model": "fake-raw",
                "reasoning": "default",
                "max_steps": 9,
                "expected_revision": expected_revision
            }),
        )
    );
    let mut statuses = [first_write.status(), second_write.status()];
    statuses.sort_by_key(|status| status.as_u16());
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "concurrent CAS writes must produce exactly one winner"
    );
    let conflict = if first_write.status() == StatusCode::CONFLICT {
        first_write
    } else {
        second_write
    };
    let conflict: serde_json::Value = decode_json(conflict).await;
    assert_eq!(conflict["code"], "product_session_model_config_conflict");

    // Workspace B's model config must be untouched by A's contention.
    let b_config = get_response(&app, &model_b_uri).await;
    assert_eq!(b_config.status(), StatusCode::OK);
    let b_config: serde_json::Value = decode_json(b_config).await;
    assert_eq!(b_config["max_steps"], 2);

    // Controls must be strictly partitioned by session.
    let (controls_a, controls_b) = tokio::join!(
        get_response(&app, &controls_a_uri),
        get_response(&app, &controls_b_uri)
    );
    assert_eq!(controls_a.status(), StatusCode::OK);
    assert_eq!(controls_b.status(), StatusCode::OK);
    let controls_a: serde_json::Value = decode_json(controls_a).await;
    let controls_b: serde_json::Value = decode_json(controls_b).await;
    let text_a = controls_a.to_string();
    let text_b = controls_b.to_string();
    assert!(text_a.contains("steer-for-a") && text_a.contains("followup-for-a"));
    assert!(!text_a.contains("steer-for-b") && !text_a.contains("followup-for-b"));
    assert!(text_b.contains("steer-for-b") && text_b.contains("followup-for-b"));
    assert!(!text_b.contains("steer-for-a") && !text_b.contains("followup-for-a"));

    // Release both runs at once; each follow-up must start exactly one successor
    // in its own session.
    let answer_a_uri = format!("/jobs/{}/inputs/{input_a}", job_a.job_id);
    let answer_b_uri = format!("/jobs/{}/inputs/{input_b}", job_b.job_id);
    let (answer_a, answer_b) = tokio::join!(
        post_json(
            &app,
            &answer_a_uri,
            serde_json::json!({ "answer": "release a" }),
        ),
        post_json(
            &app,
            &answer_b_uri,
            serde_json::json!({ "answer": "release b" }),
        )
    );
    assert_eq!(answer_a.status(), StatusCode::OK);
    assert_eq!(answer_b.status(), StatusCode::OK);

    let applied_a = wait_for_product_control_status(
        &app,
        &session_a_id,
        followup_a["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    let applied_b = wait_for_product_control_status(
        &app,
        &session_b_id,
        followup_b["id"].as_str().unwrap(),
        "applied",
    )
    .await;
    let successor_a = applied_a["run_id"].as_str().unwrap().to_string();
    let successor_b = applied_b["run_id"].as_str().unwrap().to_string();
    assert_ne!(successor_a, successor_b);
    assert_ne!(successor_a, job_a.run_id.to_string());
    assert_ne!(successor_b, job_b.run_id.to_string());

    let idle_a =
        wait_for_product_session_status(&app, &workspace_a_id, &session_a_id, "idle").await;
    let idle_b =
        wait_for_product_session_status(&app, &workspace_b_id, &session_b_id, "idle").await;
    // One original turn plus exactly one follow-up successor per session.
    assert_eq!(idle_a["runtime_binding"]["ordinal"], 2);
    assert_eq!(idle_b["runtime_binding"]["ordinal"], 2);
    assert_eq!(idle_a["runtime_binding"]["latest_run_id"], successor_a);
    assert_eq!(idle_b["runtime_binding"]["latest_run_id"], successor_b);

    // Concurrent forks at each session's terminal boundary must stay independent,
    // and a repeated key must not create a second child.
    let (fork_a, fork_b) = tokio::join!(
        post_json(
            &app,
            &forks_a_uri,
            serde_json::json!({
                "fork_at_run_id": successor_a,
                "idempotency_key": "concurrent-fork-a"
            }),
        ),
        post_json(
            &app,
            &forks_b_uri,
            serde_json::json!({
                "fork_at_run_id": successor_b,
                "idempotency_key": "concurrent-fork-b"
            }),
        )
    );
    assert_eq!(fork_a.status(), StatusCode::CREATED);
    assert_eq!(fork_b.status(), StatusCode::CREATED);
    let fork_a: serde_json::Value = decode_json(fork_a).await;
    let fork_b: serde_json::Value = decode_json(fork_b).await;
    let child_a = fork_a["session"]["id"].as_str().unwrap().to_string();
    let child_b = fork_b["session"]["id"].as_str().unwrap().to_string();
    assert_ne!(child_a, child_b);
    assert_eq!(fork_a["session"]["parent_session_id"], session_a_id);
    assert_eq!(fork_b["session"]["parent_session_id"], session_b_id);

    let (fork_replay_a, fork_replay_b) = tokio::join!(
        post_json(
            &app,
            &forks_a_uri,
            serde_json::json!({
                "fork_at_run_id": successor_a,
                "idempotency_key": "concurrent-fork-a"
            }),
        ),
        post_json(
            &app,
            &forks_b_uri,
            serde_json::json!({
                "fork_at_run_id": successor_b,
                "idempotency_key": "concurrent-fork-b"
            }),
        )
    );
    assert_eq!(fork_replay_a.status(), StatusCode::OK);
    assert_eq!(fork_replay_b.status(), StatusCode::OK);
    let fork_replay_a: serde_json::Value = decode_json(fork_replay_a).await;
    let fork_replay_b: serde_json::Value = decode_json(fork_replay_b).await;
    assert_eq!(fork_replay_a["session"]["id"], child_a);
    assert_eq!(fork_replay_b["session"]["id"], child_b);

    // Each child belongs only to its own workspace.
    let sessions_a: serde_json::Value =
        decode_json(get_response(&app, &sessions_a_uri).await).await;
    let sessions_b: serde_json::Value =
        decode_json(get_response(&app, &sessions_b_uri).await).await;
    let ids_a: Vec<&str> = sessions_a["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap())
        .collect();
    let ids_b: Vec<&str> = sessions_b["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap())
        .collect();
    assert!(ids_a.contains(&child_a.as_str()) && !ids_a.contains(&child_b.as_str()));
    assert!(ids_b.contains(&child_b.as_str()) && !ids_b.contains(&child_a.as_str()));
}

#[tokio::test]
async fn api_sse_stream_dropped_mid_flight_loses_no_events_on_reconnect() {
    use futures::StreamExt;

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let created = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": { "prompt": "hold the run open" }
            })
            .to_string(),
            "model": "fake-raw",
            "approval": "auto",
            "max_steps": 2
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created: CreateJobResponse = decode_json(created).await;
    let job_id = created.job_id.to_string();

    // Hold the run at a pending input so the stream is genuinely live, not a
    // finished replay.
    let pending = wait_for_pending_input(app.clone(), job_id.clone()).await;
    let input_id = pending.pending_inputs.first().unwrap().input_id;

    // Open a live SSE stream and read only part of it, then drop the body while
    // the run is still open. That is a client disconnect, not a clean close.
    let stream_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{job_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream_response.status(), StatusCode::OK);

    let mut body = stream_response.into_body().into_data_stream();
    let mut observed = String::new();
    let mut highest_seen = 0_u64;
    while let Some(chunk) = body.next().await {
        observed.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        for line in observed.lines() {
            if let Some(raw) = line.strip_prefix("id: ")
                && let Ok(seq) = raw.trim().parse::<u64>()
            {
                highest_seen = highest_seen.max(seq);
            }
        }
        if highest_seen >= 1 {
            break;
        }
    }
    assert!(
        highest_seen >= 1,
        "expected at least one identified event before the drop, saw: {observed}"
    );

    // Prove the run is still live at the moment of the drop. Without this the
    // test could be severing an already-finished stream, which would only
    // exercise replay-after-close rather than a mid-flight client disconnect.
    let at_drop = get_response(&app, &format!("/jobs/{job_id}/state")).await;
    assert_eq!(at_drop.status(), StatusCode::OK);
    let at_drop: JobStateResponse = decode_json(at_drop).await;
    assert_eq!(
        at_drop.status,
        RunStatus::Running,
        "the run must still be in flight when the stream is dropped"
    );
    assert!(
        !at_drop.pending_inputs.is_empty(),
        "the run must still be holding its pending input at the drop"
    );
    assert!(
        !observed.contains("event: run_completed"),
        "the dropped stream must not have already delivered the terminal event"
    );
    drop(body);

    // The severed stream must not affect the run. Answer the input and let it end.
    let answered = post_json(
        &app,
        &format!("/jobs/{job_id}/inputs/{input_id}"),
        serde_json::json!({ "answer": "continue" }),
    )
    .await;
    assert_eq!(answered.status(), StatusCode::OK);
    let final_state = wait_for_done(app.clone(), job_id.clone()).await;
    assert_eq!(final_state.status, RunStatus::Done);

    // Reconnecting with Last-Event-ID must deliver every event after the drop
    // point with no gap and no duplicate.
    let resumed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{job_id}/events"))
                .header("last-event-id", highest_seen.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resumed.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    let resumed_ids: Vec<u64> = text
        .lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .filter_map(|raw| raw.trim().parse::<u64>().ok())
        .collect();
    assert!(
        !resumed_ids.is_empty(),
        "reconnect returned no events; body: {text}"
    );
    assert!(
        resumed_ids.iter().all(|seq| *seq > highest_seen),
        "reconnect replayed already-delivered events: {resumed_ids:?} after {highest_seen}"
    );
    assert!(
        resumed_ids.windows(2).all(|pair| pair[1] > pair[0]),
        "reconnect returned out-of-order events: {resumed_ids:?}"
    );
    let expected: Vec<u64> = ((highest_seen + 1)..=final_state.event_count as u64).collect();
    assert_eq!(
        resumed_ids, expected,
        "reconnect must cover exactly the undelivered range"
    );
    assert!(text.contains("event: run_completed"));
}

#[tokio::test]
async fn product_mcp_maps_corrupt_locked_and_unsafe_config_to_typed_conflicts() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let config_dir = folder.path().join(".rove");
    let config_path = config_dir.join("mcp_servers.json");
    let lock_path = config_dir.join(".mcp_servers.lock");
    let list_uri = format!("/product/mcp/servers?workspace_id={workspace_id}");

    let created = post_json(
        &app,
        &list_uri,
        serde_json::json!({
            "name": "mapping_server",
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")]
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert!(config_path.exists());

    // A corrupt catalog must fail closed as a typed conflict, never as an empty list.
    std::fs::write(&config_path, b"{ this is not valid mcp json").unwrap();
    let corrupt = get_response(&app, &list_uri).await;
    assert_eq!(corrupt.status(), StatusCode::CONFLICT);
    let corrupt: serde_json::Value = decode_json(corrupt).await;
    assert_eq!(corrupt["code"], "product_mcp_conflict");

    let corrupt_write = post_json(
        &app,
        &list_uri,
        serde_json::json!({
            "name": "second_server",
            "transport": "stdio",
            "command": python_command(),
            "args": [workspace_path_string("tests/fixtures/mcp_mock_server.py")]
        }),
    )
    .await;
    assert_eq!(corrupt_write.status(), StatusCode::CONFLICT);
    let corrupt_write: serde_json::Value = decode_json(corrupt_write).await;
    assert_eq!(corrupt_write["code"], "product_mcp_conflict");

    // A fresh lock held by another writer must not be stolen.
    std::fs::write(&config_path, b"{\"servers\":[]}\n").unwrap();
    std::fs::write(&lock_path, b"999999\n").unwrap();
    let locked = get_response(&app, &list_uri).await;
    assert_eq!(locked.status(), StatusCode::CONFLICT);
    let locked: serde_json::Value = decode_json(locked).await;
    assert_eq!(locked["code"], "product_mcp_conflict");
    std::fs::remove_file(&lock_path).unwrap();

    let recovered = get_response(&app, &list_uri).await;
    assert_eq!(recovered.status(), StatusCode::OK);

    // A catalog path that is not a regular file must be rejected, not coerced.
    // This runs everywhere; the symlink case below needs OS symlink privileges.
    std::fs::remove_file(&config_path).unwrap();
    std::fs::create_dir(&config_path).unwrap();
    let irregular = get_response(&app, &list_uri).await;
    assert_eq!(irregular.status(), StatusCode::CONFLICT);
    let irregular: serde_json::Value = decode_json(irregular).await;
    assert_eq!(irregular["code"], "product_mcp_conflict");
    std::fs::remove_dir(&config_path).unwrap();

    // A symlinked catalog must be rejected instead of followed outside the workspace.
    let outside = tempfile::TempDir::new().unwrap();
    let outside_config = outside.path().join("attacker_mcp_servers.json");
    std::fs::write(&outside_config, b"{\"servers\":[]}\n").unwrap();
    if create_test_file_symlink(&outside_config, &config_path) {
        let unsafe_link = get_response(&app, &list_uri).await;
        assert_eq!(unsafe_link.status(), StatusCode::CONFLICT);
        let unsafe_link: serde_json::Value = decode_json(unsafe_link).await;
        assert_eq!(unsafe_link["code"], "product_mcp_conflict");
    }
}

#[tokio::test]
async fn api_exposes_pending_request_input_tool_for_jobs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "Which branch should I use?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.pending_inputs.len(), 1);
    assert_eq!(state.pending_inputs[0].prompt, "Which branch should I use?");

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{}/cancel", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let body = axum::body::to_bytes(cancel.into_body(), usize::MAX)
        .await
        .unwrap();
    let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(state.status, RunStatus::Cancelled);
    assert!(state.pending_inputs.is_empty());
}

#[tokio::test]
async fn api_answers_pending_request_input_tool_call() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "request_input",
        "args": { "prompt": "Which branch should I use?" }
    })
    .to_string();
    let body = serde_json::json!({
        "message": message,
        "model": "fake-raw",
        "max_steps": 1
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let pending = wait_for_input_event(app.clone(), created.job_id.to_string()).await;
    let input = pending.pending_inputs.first().unwrap();
    assert_eq!(input.prompt, "Which branch should I use?");
    assert!(pending.events.iter().any(|stored| {
        matches!(&stored.event, StreamEvent::InputNeeded { input_id, .. } if *input_id == input.input_id)
    }));

    let submit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/inputs/{}",
                    created.job_id, input.input_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"answer":"Use main."}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submit.status(), StatusCode::OK);

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
    assert!(state.pending_inputs.is_empty());

    let events = app
        .oneshot(
            Request::builder()
                .uri(format!("/jobs/{}/events", created.job_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("event: tool_call_completed"));
    assert!(text.contains("Use main."));
}

#[tokio::test]
async fn product_workspace_files_are_bounded_typed_and_safely_delivered() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    std::fs::write(folder.path().join("hello world.txt"), "hello, 世界\n").unwrap();
    std::fs::write(folder.path().join("bad.txt"), [0xff, 0xfe, 0x00, b'a']).unwrap();
    std::fs::write(folder.path().join(".env"), "API_KEY=never-return-this").unwrap();
    std::fs::write(folder.path().join("page.html"), "<script>alert(1)</script>").unwrap();
    std::fs::write(folder.path().join("broken.png"), "not a png").unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&3u32.to_be_bytes());
    std::fs::write(folder.path().join("image.png"), &png).unwrap();
    std::fs::write(
        folder.path().join("large.txt"),
        vec![b'x'; 1024 * 1024 + 32],
    )
    .unwrap();

    let outside = server.path().join("outside.txt");
    std::fs::write(&outside, "outside").unwrap();
    let symlink_created = create_test_file_symlink(&outside, &folder.path().join("escape.txt"));

    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files?limit=100"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    let paths: Vec<_> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect();
    assert!(paths.contains(&"hello world.txt"));
    assert!(!paths.contains(&".env"));
    assert!(!paths.contains(&"escape.txt"));
    assert_eq!(listed["scan_limit_reached"], false);

    let text = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=hello+world.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(text.status(), StatusCode::OK);
    let text: serde_json::Value = decode_json(text).await;
    assert_eq!(text["encoding"], "utf-8");
    assert_eq!(text["text"], "hello, 世界\n");
    assert_eq!(text["preview_allowed"], true);

    let invalid_utf8 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=bad.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_utf8.status(), StatusCode::OK);
    let invalid_utf8: serde_json::Value = decode_json(invalid_utf8).await;
    assert_eq!(invalid_utf8["encoding"], "binary");
    assert!(invalid_utf8.get("text").is_none());

    let large = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=large.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(large.status(), StatusCode::OK);
    let large: serde_json::Value = decode_json(large).await;
    assert_eq!(large["text"].as_str().unwrap().len(), 1024 * 1024);
    assert_eq!(large["truncated"], true);

    let preview = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/preview?path=image.png"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(preview.headers()["content-type"], "image/png");
    assert_eq!(preview.headers()["x-content-type-options"], "nosniff");
    assert!(
        preview.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("inline")
    );

    for unsafe_path in ["broken.png", "page.html"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/product/workspaces/{workspace_id}/files/preview?path={unsafe_path}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/download?path=page.html"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert!(
        download.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(download.headers()["x-content-type-options"], "nosniff");

    let secret = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/workspaces/{workspace_id}/files/content?path=.env"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(secret.status(), StatusCode::BAD_REQUEST);
    if symlink_created {
        let escaped = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/product/workspaces/{workspace_id}/files/content?path=escape.txt"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(escaped.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn product_artifacts_are_hashed_session_bound_and_report_cleanup() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Artifact evidence").await;
    let session_id = session["id"].as_str().unwrap();
    let created = create_product_job(&app, session_id, "create artifact evidence").await;
    wait_for_done(app.clone(), created.job_id.to_string()).await;

    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let run_dir = state_store.run_store.run_dir(&created.run_id);
    let artifact_dir = run_dir.join("artifacts");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::write(artifact_dir.join("evidence.txt"), "artifact body").unwrap();
    std::fs::write(artifact_dir.join("broken.png"), "not an image").unwrap();

    let manifest = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/artifacts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest: serde_json::Value = decode_json(manifest).await;
    let artifacts = manifest["artifacts"].as_array().unwrap();
    let evidence = artifacts
        .iter()
        .find(|artifact| artifact["safe_name"] == "evidence.txt")
        .unwrap();
    let artifact_id = evidence["artifact_id"].as_str().unwrap().to_string();
    assert_eq!(artifact_id.len(), 64);
    assert!(!artifact_id.contains(&created.run_id.to_string()));
    assert_eq!(
        evidence["sha256"],
        "9938be87d35f2a7a2b80237e8dc71806b209aaea8252f12c1b12949f61d40476"
    );
    assert_eq!(evidence["preview_kind"], "text");
    assert_eq!(evidence["availability"], "available");

    let broken = artifacts
        .iter()
        .find(|artifact| artifact["safe_name"] == "broken.png")
        .unwrap();
    assert_eq!(broken["preview_kind"], "unavailable");
    assert!(broken["validation_error"].as_str().is_some());

    let content = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/artifacts/{artifact_id}/content"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(content.status(), StatusCode::OK);
    let content: serde_json::Value = decode_json(content).await;
    assert_eq!(content["text"], "artifact body");

    let other = create_product_session(&app, workspace_id, "Other session").await;
    let other_id = other["id"].as_str().unwrap();
    let cross_session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{other_id}/artifacts/{artifact_id}/content"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_session.status(), StatusCode::NOT_FOUND);

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/artifacts/{artifact_id}/download"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["x-content-type-options"], "nosniff");
    let body = axum::body::to_bytes(download.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(&body[..], b"artifact body");

    std::fs::remove_file(artifact_dir.join("evidence.txt")).unwrap();
    let cleaned = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/artifacts/{artifact_id}/content"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cleaned.status(), StatusCode::NOT_FOUND);

    std::fs::remove_file(run_dir.join("trace.jsonl")).unwrap();
    let manifest = app
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/artifacts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let manifest: serde_json::Value = decode_json(manifest).await;
    let trace = manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["safe_name"] == "trace.jsonl")
        .unwrap();
    assert_eq!(trace["availability"], "cleaned");
    assert!(trace.get("sha256").is_none());
}

#[tokio::test]
async fn product_diff_returns_canonical_tool_and_git_patches() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Tool diff").await;
    let session_id = session["id"].as_str().unwrap();
    let created = create_product_job(&app, session_id, "record a diff").await;
    wait_for_done(app.clone(), created.job_id.to_string()).await;

    let state_store = StateStore::with_index_path(
        &folder.path().join("api-state"),
        folder.path().join(".rove/state.sqlite"),
        5_000,
    );
    let mut report = state_store.load_report(created.run_id).await.unwrap();
    report.tool_mutations.push(ToolMutation {
        path: "src/lib.rs".to_string(),
        operation: ToolMutationOperation::Update,
        diff: Some("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n".to_string()),
    });
    rove_runtime::state::report::write_report(
        &state_store.run_store.run_dir(&created.run_id),
        &report,
    )
    .unwrap();

    let diff = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/diff?scope=run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diff.status(), StatusCode::OK);
    let diff: serde_json::Value = decode_json(diff).await;
    let entry = &diff["entries"][0];
    assert_eq!(entry["source"], "run");
    assert_eq!(entry["source_run_id"], created.run_id.to_string());
    assert!(entry["diff"].as_str().unwrap().contains("+new"));
    assert_eq!(entry["reconstructable"], true);
    assert_eq!(entry["truncated"], false);

    let repo = tempfile::TempDir::new().unwrap();
    run_git(repo.path(), &["init"]);
    run_git(
        repo.path(),
        &["config", "user.email", "rove@example.invalid"],
    );
    run_git(repo.path(), &["config", "user.name", "Rove Test"]);
    std::fs::write(repo.path().join("tracked.txt"), "before\n").unwrap();
    run_git(repo.path(), &["add", "tracked.txt"]);
    run_git(repo.path(), &["commit", "-m", "base"]);
    let repo_workspace = post_json(
        &app,
        "/product/workspaces",
        serde_json::json!({
            "root": repo.path(),
            "kind": "repo",
            "display_name": "Diff repo",
            "pinned": false
        }),
    )
    .await;
    assert_eq!(repo_workspace.status(), StatusCode::CREATED);
    let repo_workspace: serde_json::Value = decode_json(repo_workspace).await;
    let repo_session =
        create_product_session(&app, repo_workspace["id"].as_str().unwrap(), "Git diff").await;
    std::fs::write(repo.path().join("tracked.txt"), "after\n").unwrap();
    std::fs::write(repo.path().join("binary.bin"), [0, 1, 2, 3]).unwrap();

    let git_diff = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{}/diff?scope=git",
                    repo_session["id"].as_str().unwrap()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(git_diff.status(), StatusCode::OK);
    let git_diff: serde_json::Value = decode_json(git_diff).await;
    let entries = git_diff["entries"].as_array().unwrap();
    let tracked = entries
        .iter()
        .find(|entry| entry["path"] == "tracked.txt")
        .unwrap();
    assert_eq!(tracked["source"], "git");
    assert!(tracked["diff"].as_str().unwrap().contains("+after"));
    assert_eq!(tracked["reconstructable"], true);
    let binary = entries
        .iter()
        .find(|entry| entry["path"] == "binary.bin")
        .unwrap();
    assert_eq!(binary["binary"], true);
    assert_eq!(binary["reconstructable"], false);
}

fn run_git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

// ─── R1: transcript cursor pagination ──────────────────────────────────────

/// Run ordinals of a transcript response, in response order.
fn transcript_ordinals(transcript: &serde_json::Value) -> Vec<u64> {
    transcript["segments"]
        .as_array()
        .unwrap_or_else(|| panic!("transcript segments: {transcript}"))
        .iter()
        .map(|segment| {
            segment["binding"]["ordinal"]
                .as_u64()
                .unwrap_or_else(|| panic!("segment ordinal: {segment}"))
        })
        .collect()
}

/// Compact transcript facts for assertion messages: a full projection carries
/// thousands of events and is unreadable inside a panic.
fn transcript_summary(transcript: &serde_json::Value) -> String {
    let reasons: Vec<&str> = transcript["partial_reasons"]
        .as_array()
        .map(|reasons| {
            reasons
                .iter()
                .map(|reason| reason["code"].as_str().unwrap_or("?"))
                .collect()
        })
        .unwrap_or_default();
    format!(
        "status={} ordinals={:?} has_more={} next={:?} reasons={reasons:?}",
        transcript["status"],
        transcript_ordinals(transcript),
        transcript["has_more"],
        transcript["next_before_ordinal"],
    )
}

async fn run_product_turns(
    app: &axum::Router,
    session_id: &str,
    turns: std::ops::RangeInclusive<u64>,
    message: impl Fn(u64) -> String,
) {
    for turn in turns {
        let created = create_product_job(app, session_id, &message(turn)).await;
        let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
        assert_eq!(state.status, RunStatus::Done, "turn {turn}");
    }
}

#[tokio::test]
async fn product_transcript_cursor_pages_older_runs_without_gaps_or_repeats() {
    const RUNS: u64 = 65;
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state-transcript-pages".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(&app, &workspace_id, "Transcript pages").await;
    let session_id = session["id"].as_str().unwrap().to_string();

    run_product_turns(&app, &session_id, 1..=RUNS, |turn| {
        format!("page turn {turn}")
    })
    .await;

    // 1. The parameterless request stays the pre-pagination response: every run
    // in ascending order and no cursor fields at all.
    let legacy: serde_json::Value = decode_json(
        get_response(&app, &format!("/product/sessions/{session_id}/transcript")).await,
    )
    .await;
    assert_eq!(
        legacy["status"],
        "complete",
        "{}",
        transcript_summary(&legacy)
    );
    assert_eq!(transcript_ordinals(&legacy), (1..=RUNS).collect::<Vec<_>>());
    assert!(
        legacy.get("has_more").is_none(),
        "a legacy response must not grow cursor fields: {}",
        transcript_summary(&legacy)
    );
    assert!(
        legacy.get("next_before_ordinal").is_none(),
        "a legacy response must not grow cursor fields: {}",
        transcript_summary(&legacy)
    );

    // 2. The cursor walk starts at the newest page and reaches every run exactly
    // once, with a cursor that always points at the page's oldest run.
    let mut pages: Vec<Vec<u64>> = Vec::new();
    let mut cursor: Option<u64> = None;
    let mut requests = 0;
    loop {
        let uri = match cursor {
            Some(before) => format!(
                "/product/sessions/{session_id}/transcript?before_ordinal={before}&limit_runs=8"
            ),
            None => format!("/product/sessions/{session_id}/transcript?limit_runs=8"),
        };
        let page: serde_json::Value = decode_json(get_response(&app, &uri).await).await;
        assert_eq!(
            page["status"],
            "complete",
            "page {requests}: {}",
            transcript_summary(&page)
        );
        let ordinals = transcript_ordinals(&page);
        assert!(
            !ordinals.is_empty(),
            "page {requests}: {}",
            transcript_summary(&page)
        );
        assert!(
            ordinals.len() <= 8,
            "page {requests} exceeded its page size: {}",
            transcript_summary(&page)
        );
        let expected = page["next_before_ordinal"].as_u64();
        let has_more = page["has_more"].as_bool().unwrap_or_else(|| {
            panic!(
                "a cursor page carries has_more: {}",
                transcript_summary(&page)
            )
        });
        assert_eq!(
            has_more,
            expected.is_some(),
            "has_more must mirror next_before_ordinal: {}",
            transcript_summary(&page)
        );
        if let Some(expected) = expected {
            assert_eq!(
                expected,
                ordinals[0],
                "the cursor must be the page's oldest run ordinal: {}",
                transcript_summary(&page)
            );
        }
        pages.push(ordinals);
        requests += 1;
        match expected {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(requests < 32, "the cursor walk did not terminate");
    }
    assert_eq!(requests, 9, "65 runs at 8 per page need nine pages");
    assert_eq!(
        pages[0],
        (58..=RUNS).collect::<Vec<_>>(),
        "the first cursor page is the newest one"
    );

    let mut merged: Vec<u64> = Vec::new();
    for page in pages.iter().rev() {
        merged.extend(page.iter().copied());
    }
    assert_eq!(
        merged,
        (1..=RUNS).collect::<Vec<_>>(),
        "prepending every page must reconstruct the full history without gaps or repeats"
    );

    // 3. A cursor past the newest run is an empty terminal page, and ordinal 0
    // is rejected as an invalid cursor.
    let beyond: serde_json::Value = decode_json(
        get_response(
            &app,
            &format!("/product/sessions/{session_id}/transcript?before_ordinal=1000&limit_runs=8"),
        )
        .await,
    )
    .await;
    assert!(
        transcript_ordinals(&beyond).is_empty(),
        "{}",
        transcript_summary(&beyond)
    );
    assert_eq!(beyond["has_more"], false, "{}", transcript_summary(&beyond));
    assert!(
        beyond.get("next_before_ordinal").is_none(),
        "{}",
        transcript_summary(&beyond)
    );

    let zero = get_response(
        &app,
        &format!("/product/sessions/{session_id}/transcript?before_ordinal=0"),
    )
    .await;
    assert_eq!(zero.status(), StatusCode::BAD_REQUEST);
    let zero: serde_json::Value = decode_json(zero).await;
    assert_eq!(zero["code"], "product_invalid_input", "{zero}");
}

#[tokio::test]
async fn product_transcript_page_query_rejects_unbounded_pages() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state-transcript-page-bounds".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(&app, &workspace_id, "Transcript page bounds").await;
    let session_id = session["id"].as_str().unwrap().to_string();

    run_product_turns(&app, &session_id, 1..=3, |turn| {
        format!("bounds turn {turn}")
    })
    .await;

    for bad in ["limit_runs=0", "limit_runs=65", "before_ordinal=0"] {
        let response = get_response(
            &app,
            &format!("/product/sessions/{session_id}/transcript?{bad}"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{bad}` should have been rejected"
        );
        let error: serde_json::Value = decode_json(response).await;
        assert_eq!(error["code"], "product_invalid_input", "for `{bad}`");
    }

    // The largest supported page is accepted and still reports its shape.
    let largest: serde_json::Value = decode_json(
        get_response(
            &app,
            &format!("/product/sessions/{session_id}/transcript?limit_runs=64"),
        )
        .await,
    )
    .await;
    assert_eq!(
        transcript_ordinals(&largest),
        vec![1, 2, 3],
        "{}",
        transcript_summary(&largest)
    );
    assert_eq!(
        largest["has_more"],
        false,
        "{}",
        transcript_summary(&largest)
    );

    // Strictly older than ordinal 1 is empty, and so is a cursor beyond the
    // newest run: the client advances its cursor and stops.
    for cursor in ["before_ordinal=1", "before_ordinal=4"] {
        let page: serde_json::Value = decode_json(
            get_response(
                &app,
                &format!("/product/sessions/{session_id}/transcript?{cursor}&limit_runs=2"),
            )
            .await,
        )
        .await;
        assert_eq!(
            page["status"],
            "complete",
            "{cursor}: {}",
            transcript_summary(&page)
        );
        assert!(
            transcript_ordinals(&page).is_empty(),
            "{cursor}: {}",
            transcript_summary(&page)
        );
        assert_eq!(
            page["has_more"],
            false,
            "{cursor}: {}",
            transcript_summary(&page)
        );
    }

    // A cursor window below the newest run keeps the ascending run order of the
    // legacy response inside the window.
    let window: serde_json::Value = decode_json(
        get_response(
            &app,
            &format!("/product/sessions/{session_id}/transcript?before_ordinal=3&limit_runs=2"),
        )
        .await,
    )
    .await;
    assert_eq!(
        transcript_ordinals(&window),
        vec![1, 2],
        "{}",
        transcript_summary(&window)
    );
    assert_eq!(window["has_more"], false, "{}", transcript_summary(&window));
}

// ─── R2c follow-up: the negotiated canonical-event contract ────────────────
//
// A durable row whose kind a client bundle predates must not fail the whole
// transcript response. The request declares the canonical-event contract it can
// decode; the projection then withholds the rows outside it and reports the
// withheld durable sequence positions instead of inventing a gap or a corrupt
// row. A request that declares nothing keeps the legacy contract exactly, and a
// request that declares the current contract is byte-identical to it.

/// Response status plus exact body bytes, so a test can compare two requests
/// byte for byte instead of only field by field.
async fn transcript_bytes(app: &axum::Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let response = get_response(app, uri).await;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, body.to_vec())
}

/// The transcript segment of one run ordinal, or a readable panic.
fn transcript_segment(transcript: &serde_json::Value, ordinal: u64) -> &serde_json::Value {
    transcript["segments"]
        .as_array()
        .unwrap_or_else(|| panic!("segments: {}", transcript_summary(transcript)))
        .iter()
        .find(|segment| segment["binding"]["ordinal"] == ordinal)
        .unwrap_or_else(|| {
            panic!(
                "segment {ordinal} is missing: {}",
                transcript_summary(transcript)
            )
        })
}

/// `(seq, event type)` of every delivered event of one run, in response order.
fn transcript_segment_events(transcript: &serde_json::Value, ordinal: u64) -> Vec<(u64, String)> {
    transcript_segment(transcript, ordinal)["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["seq"].as_u64().unwrap(),
                event["event"]["type"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Partial reason codes of one response, in response order.
fn transcript_reason_codes(transcript: &serde_json::Value) -> Vec<String> {
    transcript["partial_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason["code"].as_str().unwrap().to_string())
        .collect()
}

/// Partial reason codes naming one run ordinal, in response order. Reasons that
/// name no run are excluded, so a test can speak about one run exactly.
fn transcript_reason_codes_for(transcript: &serde_json::Value, ordinal: u64) -> Vec<String> {
    transcript["partial_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reason| reason["run_ordinal"] == ordinal)
        .map(|reason| reason["code"].as_str().unwrap().to_string())
        .collect()
}

/// The single reason of one code for one run ordinal.
fn transcript_reason<'a>(
    transcript: &'a serde_json::Value,
    ordinal: u64,
    code: &str,
) -> &'a serde_json::Value {
    let mut found = transcript["partial_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reason| reason["code"] == code && reason["run_ordinal"] == ordinal);
    let reason = found.next().unwrap_or_else(|| {
        panic!(
            "no `{code}` reason for run {ordinal}: {}",
            transcript_summary(transcript)
        )
    });
    assert!(
        found.next().is_none(),
        "one run reports one `{code}` reason: {}",
        transcript_summary(transcript)
    );
    reason
}

/// The single `unknown_event_type` reason for one run ordinal.
fn transcript_unknown_event_reason(
    transcript: &serde_json::Value,
    ordinal: u64,
) -> &serde_json::Value {
    let mut found = transcript["partial_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reason| {
            reason["code"] == "unknown_event_type" && reason["run_ordinal"] == ordinal
        });
    let reason = found
        .next()
        .unwrap_or_else(|| panic!("no withheld-row reason: {}", transcript_summary(transcript)));
    assert!(
        found.next().is_none(),
        "one run reports one withheld range: {}",
        transcript_summary(transcript)
    );
    reason
}

/// Durable event rows of one run in `seq` order: `(seq, event_name, event_json)`.
fn durable_event_rows(index_path: &Path, run_id: RunId) -> Vec<(u64, String, String)> {
    let connection = rusqlite::Connection::open(index_path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT seq, event_name, event_json FROM events WHERE run_id = ?1 ORDER BY seq ASC",
        )
        .unwrap();
    statement
        .query_map(rusqlite::params![run_id.to_string()], |row| {
            Ok((
                u64::try_from(row.get::<_, i64>(0)?).unwrap(),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Insert one durable event row at `seq`, shifting the run's later rows by one.
///
/// The fixture must hold a *contiguous* log, exactly as the build that wrote the
/// row left it: the projection treats a sequence hole as a missing range. The
/// shift is two-step because `events` is keyed by `(run_id, seq)`, so a single
/// `seq = seq + 1` would collide with the row it is about to move.
fn insert_durable_event_row(
    index_path: &Path,
    run_id: RunId,
    seq: u64,
    event_name: &str,
    event_json: &str,
) {
    const SHIFT: i64 = 1_000_000;
    let connection = rusqlite::Connection::open(index_path).unwrap();
    let run_id = run_id.to_string();
    let at = i64::try_from(seq).unwrap();
    connection
        .execute(
            "UPDATE events SET seq = seq + ?3 WHERE run_id = ?1 AND seq >= ?2",
            rusqlite::params![run_id, at, SHIFT],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE events SET seq = seq - ?3 WHERE run_id = ?1 AND seq >= ?2",
            rusqlite::params![run_id, at + SHIFT, SHIFT - 1],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO events(run_id, seq, event_name, event_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![run_id, at, event_name, event_json, "2026-09-26T00:00:00Z"],
        )
        .unwrap();
    let advanced = connection
        .execute(
            "UPDATE runs SET last_event_seq = last_event_seq + 1 WHERE run_id = ?1",
            rusqlite::params![run_id],
        )
        .unwrap();
    assert_eq!(advanced, 1, "the fixture run is unknown to the event index");
}

/// Advance one run's durable high-water mark without indexing a row for it.
///
/// `TraceWriter::append_history`/`append_resume_link` advance
/// `StateIndex::advance_event_seq` for lines that never become indexed events, so
/// `runs.last_event_seq` can legitimately exceed the highest indexed `seq`. This
/// reproduces that shortfall exactly.
fn advance_durable_high_water(index_path: &Path, run_id: RunId, by: u64) {
    let connection = rusqlite::Connection::open(index_path).unwrap();
    let advanced = connection
        .execute(
            "UPDATE runs SET last_event_seq = last_event_seq + ?2 WHERE run_id = ?1",
            rusqlite::params![run_id.to_string(), i64::try_from(by).unwrap()],
        )
        .unwrap();
    assert_eq!(advanced, 1, "the fixture run is unknown to the event index");
}

/// The JSON of one existing durable row of `event_name` for a run.
fn durable_row_json(index_path: &Path, run_id: RunId, event_name: &str) -> String {
    let rows = durable_event_rows(index_path, run_id);
    rows.iter()
        .find(|(_, name, _)| name == event_name)
        .map(|(_, _, json)| json.clone())
        .unwrap_or_else(|| panic!("the fixture holds no `{event_name}` row: {rows:?}"))
}

/// Replace one run's durable log with `rows` at contiguous `seq` 1..N.
///
/// The run's durable high-water mark becomes N, so the rewritten log is
/// internally consistent: no residue, and the terminal fact, when the caller
/// supplies one, is the last row.
fn replace_run_log(index_path: &Path, run_id: RunId, rows: &[(&str, String)]) {
    assert!(!rows.is_empty(), "a run log needs at least one row");
    let connection = rusqlite::Connection::open(index_path).unwrap();
    let run_id = run_id.to_string();
    let deleted = connection
        .execute(
            "DELETE FROM events WHERE run_id = ?1",
            rusqlite::params![run_id],
        )
        .unwrap();
    assert!(deleted > 0, "the fixture run holds no durable rows");
    for (offset, (event_name, event_json)) in rows.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO events(run_id, seq, event_name, event_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    run_id,
                    i64::try_from(offset).unwrap() + 1,
                    event_name,
                    event_json,
                    "2026-09-26T00:00:00Z"
                ],
            )
            .unwrap();
    }
    let updated = connection
        .execute(
            "UPDATE runs SET last_event_seq = ?2 WHERE run_id = ?1",
            rusqlite::params![run_id, i64::try_from(rows.len()).unwrap()],
        )
        .unwrap();
    assert_eq!(updated, 1, "the fixture run is unknown to the event index");
}

/// Rewrite one run's durable log to a single decodable row at `seq` 1.
///
/// The row reuses the JSON of an existing row of that name, so the projection
/// breaks on a real lifecycle violation (`seq` 1 is not `run_started`) rather
/// than on a payload this test made up.
fn keep_only_one_durable_row(index_path: &Path, run_id: RunId, event_name: &str) {
    let event_json = durable_row_json(index_path, run_id, event_name);
    replace_run_log(index_path, run_id, &[(event_name, event_json)]);
}

/// One canonical `provider_retry` payload with a `reason` of `chars` bytes.
fn provider_retry_json(chars: usize) -> String {
    let retry = StreamEvent::ProviderRetry {
        attempt: 2,
        max_attempts: 4,
        delay_ms: 2_000,
        reason: "x".repeat(chars),
        phase: "model_call".to_string(),
    };
    serde_json::to_string(&retry).unwrap()
}

/// One canonical `llm_chunk` payload with a `delta` of `chars` bytes.
fn llm_chunk_json(chars: usize) -> String {
    serde_json::to_string(&StreamEvent::LlmChunk {
        delta: "y".repeat(chars),
    })
    .unwrap()
}

/// Rewrite one run's durable log as a real start fact, `chunks` large
/// `llm_chunk` rows, and a real terminal fact.
///
/// Returns the response bytes the log carries, which is what the projection's
/// shared response budget is charged. The caller sizes `chunks` and `chars` so
/// the run spends nearly all of that budget.
fn write_budget_consuming_log(
    index_path: &Path,
    run_id: RunId,
    chunks: usize,
    chars: usize,
) -> usize {
    let start = durable_row_json(index_path, run_id, "run_started");
    let terminal = durable_row_json(index_path, run_id, "run_completed");
    let chunk = llm_chunk_json(chars);
    let mut rows: Vec<(&str, String)> = Vec::with_capacity(chunks + 2);
    rows.push(("run_started", start));
    for _ in 0..chunks {
        rows.push(("llm_chunk", chunk.clone()));
    }
    rows.push(("run_completed", terminal));
    let total: usize = rows.iter().map(|(_, json)| json.len()).sum();
    replace_run_log(index_path, run_id, &rows);
    total
}

/// One product session with two finished turns, and the durable index that holds
/// their canonical events.
struct ContractFixture {
    _server: tempfile::TempDir,
    /// Held so the workspace folder, and the durable index inside it, outlive the
    /// fixture.
    _folder: tempfile::TempDir,
    app: axum::Router,
    _state: ApiState,
    session_id: String,
    first_run: RunId,
    second_run: RunId,
    index_path: PathBuf,
}

impl ContractFixture {
    async fn start(label: &str) -> Self {
        let server = tempfile::TempDir::new().unwrap();
        let folder = tempfile::TempDir::new().unwrap();
        let mut config = test_config();
        config.state.state_dir = "api-state".into();
        let state = ApiState::new(Workspace::detect(server.path()).unwrap(), config);
        let app = router(state.clone());
        let workspace = create_product_workspace(&app, folder.path()).await;
        let workspace_id = workspace["id"].as_str().unwrap().to_string();
        let session = create_product_session(&app, &workspace_id, label).await;
        let session_id = session["id"].as_str().unwrap().to_string();

        let mut runs = Vec::new();
        for message in ["contract first turn", "contract second turn"] {
            let job = create_product_job(&app, &session_id, message).await;
            let finished = wait_for_done(app.clone(), job.job_id.to_string()).await;
            assert_eq!(finished.status, RunStatus::Done, "turn `{message}`");
            runs.push(job.run_id);
        }

        let store = product_state_store(folder.path());
        let index_path = store.index.path().to_path_buf();
        assert!(
            index_path.exists(),
            "the fixture has no durable event index at {}",
            index_path.display()
        );

        Self {
            _server: server,
            _folder: folder,
            app,
            _state: state,
            session_id,
            first_run: runs[0],
            second_run: runs[1],
            index_path,
        }
    }

    fn transcript_uri(&self, query: &str) -> String {
        let base = format!("/product/sessions/{}/transcript", self.session_id);
        if query.is_empty() {
            base
        } else {
            format!("{base}?{query}")
        }
    }

    async fn transcript(&self, query: &str) -> serde_json::Value {
        let response = get_response(&self.app, &self.transcript_uri(query)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "request `{query}` was refused"
        );
        decode_json(response).await
    }

    /// Append the canonical `provider_retry` row the R2c producer writes, just
    /// before the newest run's terminal fact.
    fn write_durable_provider_retry(&self) -> (u64, u64) {
        let rows = durable_event_rows(&self.index_path, self.second_run);
        let (terminal_seq, terminal_name, _) = rows
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("run {} has no durable rows", self.second_run));
        assert_eq!(
            terminal_name, "run_completed",
            "the fixture run is not terminal: {rows:?}"
        );
        assert!(
            rows.len() >= 4,
            "the fixture run is too small to prove a withheld row: {rows:?}"
        );
        let retry = StreamEvent::ProviderRetry {
            attempt: 2,
            max_attempts: 4,
            delay_ms: 2_000,
            reason: "transient:request_failed".to_string(),
            phase: "model_call".to_string(),
        };
        insert_durable_event_row(
            &self.index_path,
            self.second_run,
            terminal_seq,
            retry.event_name(),
            &serde_json::to_string(&retry).unwrap(),
        );
        (terminal_seq, terminal_seq + 1)
    }

    /// Rewrite the newest run's durable log so its last fact names a kind this
    /// build has never heard of, exactly as a newer build would leave it.
    fn write_durable_future_kind(&self, seq: u64) -> u64 {
        let row = r#"{"type":"future_widget_event","detail":"written by a newer build"}"#;
        insert_durable_event_row(
            &self.index_path,
            self.second_run,
            seq,
            "future_widget_event",
            row,
        );
        seq
    }
}

#[tokio::test]
async fn product_transcript_withholds_an_event_the_declared_contract_cannot_decode() {
    let fixture = ContractFixture::start("Negotiated transcript").await;
    let (retry_seq, high_water) = fixture.write_durable_provider_retry();
    let durable = durable_event_rows(&fixture.index_path, fixture.second_run);
    assert_eq!(
        durable
            .iter()
            .filter(|(seq, _, _)| *seq == retry_seq)
            .count(),
        1,
        "the fixture must hold exactly one retry row at {retry_seq}: {durable:?}"
    );
    assert_eq!(
        durable.last().unwrap().1,
        "run_completed",
        "the terminal fact must stay last: {durable:?}"
    );

    // 1. A request that declares nothing keeps the legacy contract: the retry row
    //    is delivered and the projection is complete.
    let (legacy_status, legacy_bytes) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("")).await;
    assert_eq!(legacy_status, StatusCode::OK);
    let legacy: serde_json::Value = serde_json::from_slice(&legacy_bytes).unwrap();
    assert_eq!(
        legacy["status"],
        "complete",
        "{}",
        transcript_summary(&legacy)
    );
    let legacy_events = transcript_segment_events(&legacy, 2);
    assert!(
        legacy_events.contains(&(retry_seq, "provider_retry".to_string())),
        "a legacy request must keep receiving every decodable row: {legacy_events:?}"
    );
    assert_eq!(
        legacy_events.len(),
        durable.len(),
        "a legacy request delivers every durable row: {legacy_events:?}"
    );
    assert!(
        transcript_reason_codes(&legacy).is_empty(),
        "{}",
        transcript_summary(&legacy)
    );
    assert_eq!(
        transcript_segment(&legacy, 2)["last_event_seq"],
        high_water,
        "{}",
        transcript_summary(&legacy)
    );

    // 2. A client that declares the current contract decodes those same facts, so
    //    its response is byte-identical to the legacy one.
    let (current_status, current_bytes) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("event_contract=2")).await;
    assert_eq!(current_status, StatusCode::OK);
    assert_eq!(
        current_bytes, legacy_bytes,
        "declaring the current contract must not change one byte of the response"
    );

    // 3. A client that declares the pre-retry contract still restores: every
    //    known event arrives with its own durable sequence, the retry row does
    //    not, and its position is reported instead of hidden.
    let legacy_contract = fixture.transcript("event_contract=1").await;
    assert_eq!(
        legacy_contract["status"],
        "partial",
        "{}",
        transcript_summary(&legacy_contract)
    );
    let delivered = transcript_segment_events(&legacy_contract, 2);
    assert!(
        !delivered.iter().any(|(_, name)| name == "provider_retry"),
        "the withheld row must not be reported as a known event: {delivered:?}"
    );
    let expected: Vec<(u64, String)> = durable
        .iter()
        .filter(|(seq, _, _)| *seq != retry_seq)
        .map(|(seq, name, _)| (*seq, name.clone()))
        .collect();
    assert_eq!(
        delivered,
        expected,
        "every other durable event must still arrive with its own sequence: {}",
        transcript_summary(&legacy_contract)
    );
    let reason = transcript_unknown_event_reason(&legacy_contract, 2);
    assert_eq!(reason["run_id"], fixture.second_run.to_string());
    assert_eq!(reason["expected_seq"], retry_seq);
    assert_eq!(reason["observed_seq"], retry_seq);
    assert_eq!(
        transcript_reason_codes(&legacy_contract),
        vec!["unknown_event_type"],
        "a withheld row is neither corruption nor a gap: {}",
        transcript_summary(&legacy_contract)
    );
    let segment = transcript_segment(&legacy_contract, 2);
    assert_eq!(
        segment["last_event_seq"], high_water,
        "the durable high-water mark must not move for a withheld row"
    );
    assert_eq!(
        segment["observed_through_seq"],
        high_water,
        "the delivered tail must keep its own position: {}",
        transcript_summary(&legacy_contract)
    );

    // 4. The other run, which holds no withheld row, is untouched, and paging is
    //    unchanged: the same page shape, cursor, and run window as a request that
    //    declares nothing.
    let first_run_segment = transcript_segment(&legacy_contract, 1);
    assert_eq!(
        first_run_segment["binding"]["runtime_run_id"],
        fixture.first_run.to_string()
    );
    assert_eq!(
        first_run_segment["last_event_seq"],
        first_run_segment["observed_through_seq"]
    );

    let legacy_page = fixture.transcript("limit_runs=1").await;
    let contract_page = fixture.transcript("limit_runs=1&event_contract=1").await;
    assert_eq!(
        transcript_ordinals(&legacy_page),
        transcript_ordinals(&contract_page)
    );
    assert_eq!(
        legacy_page["has_more"], contract_page["has_more"],
        "paging must not change for a withheld row"
    );
    assert_eq!(
        legacy_page["next_before_ordinal"], contract_page["next_before_ordinal"],
        "the paging cursor must not change for a withheld row"
    );
    assert_eq!(contract_page["has_more"], true);
    assert_eq!(contract_page["next_before_ordinal"], 2);
    assert_eq!(
        transcript_unknown_event_reason(&contract_page, 2)["expected_seq"],
        retry_seq
    );
}

#[tokio::test]
async fn product_transcript_handles_a_future_event_kind_through_the_same_contract() {
    let fixture = ContractFixture::start("Future event kind").await;
    let rows = durable_event_rows(&fixture.index_path, fixture.second_run);
    assert!(rows.len() >= 4, "the fixture run is too small: {rows:?}");
    // Write the unknown kind in the middle of the run, so known events follow it.
    let future_seq = fixture.write_durable_future_kind(3);

    // 1. The legacy contract is unchanged: a row this build cannot decode is
    //    still reported as corrupt, and the run stops at it.
    let legacy = fixture.transcript("").await;
    assert_eq!(
        legacy["status"],
        "partial",
        "{}",
        transcript_summary(&legacy)
    );
    assert_eq!(
        transcript_reason_codes(&legacy),
        vec!["corrupt_event"],
        "the legacy contract must keep its previous reading: {}",
        transcript_summary(&legacy)
    );
    assert_eq!(
        transcript_segment_events(&legacy, 2)
            .iter()
            .map(|(seq, _)| *seq)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "{}",
        transcript_summary(&legacy)
    );

    // 2. A negotiated request reads through it: the unknown row is withheld, the
    //    known events before and after it arrive, and the response names the
    //    withheld position instead of claiming corruption.
    let negotiated = fixture.transcript("event_contract=2").await;
    assert_eq!(
        negotiated["status"],
        "partial",
        "{}",
        transcript_summary(&negotiated)
    );
    assert_eq!(
        transcript_reason_codes(&negotiated),
        vec!["unknown_event_type"],
        "a kind this build cannot decode is withheld, not corrupt: {}",
        transcript_summary(&negotiated)
    );
    let reason = transcript_unknown_event_reason(&negotiated, 2);
    assert_eq!(reason["expected_seq"], future_seq);
    assert_eq!(reason["observed_seq"], future_seq);
    let delivered = transcript_segment_events(&negotiated, 2);
    assert!(
        !delivered
            .iter()
            .any(|(_, name)| name == "future_widget_event"),
        "the unknown row must not be reported as a known event: {delivered:?}"
    );
    assert!(
        delivered
            .iter()
            .any(|(seq, name)| *seq > future_seq && name == "llm_message"),
        "known events after the unknown row must still arrive: {delivered:?}"
    );
    assert_eq!(
        delivered.last().unwrap().1,
        "run_completed",
        "the terminal fact must still arrive: {delivered:?}"
    );
    assert_eq!(
        transcript_segment(&negotiated, 2)["last_event_seq"],
        delivered.last().unwrap().0,
        "{}",
        transcript_summary(&negotiated)
    );
}

/// A lifecycle break inside the snapshot must not change the legacy reason set.
///
/// `consumed_all_records` used to mean "every record reached the segment". Counting
/// rows the loop *broke on* would let a broken run look consumed, which turns a
/// single `corrupt_event` into an extra `missing_event_range` and, on a cursor
/// page, into `response_limit_reached` with a stopped walk.
#[tokio::test]
async fn product_transcript_a_lifecycle_break_keeps_the_legacy_reason_set() {
    let fixture = ContractFixture::start("Legacy lifecycle break").await;
    // `seq` 1 that is not `run_started` is the cheapest real lifecycle break, and
    // the row itself decodes, so the reason under test is the only variable.
    keep_only_one_durable_row(&fixture.index_path, fixture.second_run, "llm_chunk");

    // 1. The parameterless contract reports exactly one reason for that run and
    //    delivers nothing from it.
    let legacy = fixture.transcript("").await;
    assert_eq!(
        transcript_reason_codes_for(&legacy, 2),
        vec!["corrupt_event"],
        "a lifecycle break is corruption, not a gap: {}",
        transcript_summary(&legacy)
    );
    let segment = transcript_segment(&legacy, 2);
    assert_eq!(segment["observed_through_seq"], 0);
    assert_eq!(segment["last_event_seq"], 1);
    assert_eq!(segment["events"].as_array().unwrap().len(), 0);

    // 2. The other run is untouched: the page still carries it.
    assert!(
        transcript_segment_events(&legacy, 1)
            .iter()
            .any(|(_, name)| name == "run_started"),
        "the healthy run must keep its events: {}",
        transcript_summary(&legacy)
    );

    // 3. A cursor page large enough for both runs keeps both, with the same reasons
    //    as the parameterless response: a broken run must not look like a response
    //    that ran out of budget and stop the walk.
    let page = fixture.transcript("limit_runs=2").await;
    assert_eq!(
        transcript_ordinals(&page),
        vec![1, 2],
        "a broken run must not seal off its older sibling: {}",
        transcript_summary(&page)
    );
    assert_eq!(
        transcript_reason_codes_for(&page, 2),
        vec!["corrupt_event"],
        "a broken run must not also claim the response limit: {}",
        transcript_summary(&page)
    );
    let older = fixture.transcript("limit_runs=1&before_ordinal=2").await;
    assert_eq!(
        transcript_ordinals(&older),
        vec![1],
        "{}",
        transcript_summary(&older)
    );

    // 4. Declaring the current contract changes nothing for a break like this: the
    //    broken row is a kind this client can decode, so it is not withheld.
    let (_, current_bytes) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("event_contract=2")).await;
    assert_eq!(
        current_bytes,
        transcript_bytes(&fixture.app, &fixture.transcript_uri(""))
            .await
            .1,
        "a lifecycle break reads the same under the current contract"
    );
}

/// A withheld row must not hide an unrelated residue at the end of the run.
///
/// `runs.last_event_seq` can exceed the highest indexed `seq`, because trace lines
/// that are written but never indexed still advance the durable sequence. A
/// withheld row is accounted by its own reason; the residue beyond the highest
/// accounted sequence still has to be reported.
#[tokio::test]
async fn product_transcript_reports_a_residue_beyond_a_withheld_row() {
    let fixture = ContractFixture::start("Withheld row and residue").await;
    let (retry_seq, high_water) = fixture.write_durable_provider_retry();
    // Two durable sequence numbers that never became indexed events.
    advance_durable_high_water(&fixture.index_path, fixture.second_run, 2);
    let residue_end = high_water + 2;

    let negotiated = fixture.transcript("event_contract=1").await;
    assert_eq!(
        transcript_reason_codes_for(&negotiated, 2),
        vec!["unknown_event_type", "missing_event_range"],
        "the withheld row is accounted, and the residue is still a gap: {}",
        transcript_summary(&negotiated)
    );
    let withheld = transcript_unknown_event_reason(&negotiated, 2);
    assert_eq!(withheld["expected_seq"], retry_seq);
    assert_eq!(withheld["observed_seq"], retry_seq);
    let residue = transcript_reason(&negotiated, 2, "missing_event_range");
    assert_eq!(
        residue["expected_seq"],
        high_water + 1,
        "the residue starts after the highest accounted sequence: {}",
        transcript_summary(&negotiated)
    );
    assert_eq!(residue["observed_seq"], residue_end);
    let segment = transcript_segment(&negotiated, 2);
    assert_eq!(segment["last_event_seq"], residue_end);
    assert_eq!(
        segment["observed_through_seq"], high_water,
        "the delivered tail must not be stretched to the durable high-water mark"
    );

    // The legacy contract sees the same residue and no withholding at all.
    let legacy = fixture.transcript("").await;
    assert_eq!(
        transcript_reason_codes_for(&legacy, 2),
        vec!["missing_event_range"],
        "{}",
        transcript_summary(&legacy)
    );
    let legacy_residue = transcript_reason(&legacy, 2, "missing_event_range");
    assert_eq!(legacy_residue["expected_seq"], high_water + 1);
    assert_eq!(legacy_residue["observed_seq"], residue_end);
}

/// A withheld row must not charge the response budget.
///
/// The budget is shared by every run of one response, so the contract decision has
/// to be made before a row is billed against it. Otherwise one row the client will
/// never receive can end the response and report `response_limit_reached` for a run
/// that is short only because a newer build wrote into it.
#[tokio::test]
async fn product_transcript_does_not_charge_its_budget_for_a_withheld_row() {
    // `MAX_TOTAL_EVENT_JSON_BYTES` is 16 MiB and `MAX_SNAPSHOT_EVENT_JSON_BYTES` is
    // 1 MiB, and the store refuses a log past 16 MiB. The default transcript reads
    // the runs oldest first, so the *older* run has to spend nearly the whole budget
    // with rows the store still hands over, and the newer run then holds one fact
    // that no longer fits. Both logs are rewritten here, so the arithmetic below is
    // the whole fixture.
    const RESPONSE_BUDGET: usize = 16 * 1_048_576;
    let fixture = ContractFixture::start("Withheld row and response budget").await;
    let spent = write_budget_consuming_log(&fixture.index_path, fixture.first_run, 16, 1_000_000);
    // The newer run keeps a real lifecycle around the one fact this client cannot
    // decode, so the only reason it reports is the withheld range.
    let withheld_row = provider_retry_json(1_000_000);
    replace_run_log(
        &fixture.index_path,
        fixture.second_run,
        &[
            (
                "run_started",
                durable_row_json(&fixture.index_path, fixture.second_run, "run_started"),
            ),
            ("provider_retry", withheld_row.clone()),
            (
                "run_completed",
                durable_row_json(&fixture.index_path, fixture.second_run, "run_completed"),
            ),
        ],
    );
    assert!(
        spent < RESPONSE_BUDGET,
        "the older run must stay inside the snapshot byte limit: {spent}"
    );
    assert!(
        withheld_row.len() > RESPONSE_BUDGET - spent,
        "the withheld row must be the one that no longer fits: {} vs {} left",
        withheld_row.len(),
        RESPONSE_BUDGET - spent
    );

    // 1. A client on the pre-retry contract never receives that row, so it must not
    //    pay for it either: the row is withheld and the response keeps reading.
    let negotiated = fixture.transcript("event_contract=1").await;
    assert_eq!(
        transcript_reason_codes_for(&negotiated, 2),
        vec!["unknown_event_type"],
        "a withheld row must not charge the response budget: {}",
        transcript_summary(&negotiated)
    );
    let withheld = transcript_unknown_event_reason(&negotiated, 2);
    assert_eq!(withheld["expected_seq"], 2);
    assert_eq!(withheld["observed_seq"], 2);
    assert_eq!(
        transcript_segment_events(&negotiated, 2)
            .iter()
            .map(|(seq, name)| (*seq, name.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "run_started"), (3, "run_completed")],
        "the known facts around the withheld row must still arrive: {}",
        transcript_summary(&negotiated)
    );

    // 2. The same request without a declaration keeps the legacy reading: the row is
    //    past the response limit, so the run stops there.
    let legacy = fixture.transcript("").await;
    assert_eq!(
        transcript_reason_codes(&legacy),
        vec!["response_limit_reached"],
        "the legacy contract must keep its previous reading: {}",
        transcript_summary(&legacy)
    );
    assert_eq!(
        transcript_reason_codes_for(&legacy, 2),
        vec!["response_limit_reached"],
        "{}",
        transcript_summary(&legacy)
    );
    assert!(
        !transcript_segment_events(&legacy, 2)
            .iter()
            .any(|(seq, _)| *seq >= 2),
        "the row past the limit must not be delivered: {}",
        transcript_summary(&legacy)
    );
}

/// A row past the store's own snapshot limit is still reported as a limit.
///
/// The store refuses to load any single event past `MAX_SNAPSHOT_EVENT_JSON_BYTES`
/// (1 MiB) and reports `has_more`, so the response stops at
/// `response_limit_reached` before the projection ever sees the row. That bound is
/// deliberately not contract-aware: it protects the read, and materializing a
/// multi-megabyte row a client cannot decode would cost more than the truncation it
/// avoids. This test pins that honest limit, and it is why the withholding rule is
/// stated for the rows the projection could read.
#[tokio::test]
async fn product_transcript_reports_the_store_limit_for_an_oversized_row() {
    let fixture = ContractFixture::start("Oversized durable row").await;
    let rows = durable_event_rows(&fixture.index_path, fixture.second_run);
    let (terminal_seq, terminal_name, _) = rows.last().cloned().unwrap();
    assert_eq!(
        terminal_name, "run_completed",
        "the fixture run is not terminal: {rows:?}"
    );
    let oversized = provider_retry_json(1_100_000);
    assert!(oversized.len() > 1_048_576, "the row must exceed the limit");
    insert_durable_event_row(
        &fixture.index_path,
        fixture.second_run,
        terminal_seq,
        "provider_retry",
        &oversized,
    );

    for query in ["", "event_contract=1", "event_contract=2"] {
        let transcript = fixture.transcript(query).await;
        assert_eq!(
            transcript_reason_codes_for(&transcript, 2),
            vec!["response_limit_reached"],
            "`{query}` must report the read limit instead of guessing: {}",
            transcript_summary(&transcript)
        );
        let delivered = transcript_segment_events(&transcript, 2);
        assert!(
            !delivered.iter().any(|(seq, _)| *seq >= terminal_seq),
            "the oversized row and the terminal after it must not be delivered: {delivered:?}"
        );
    }
}

#[tokio::test]
async fn product_transcript_rejects_a_malformed_declared_contract() {
    let fixture = ContractFixture::start("Malformed contract").await;

    for raw in ["", "abc", "0", "-1", "1.5", "1e3", "%20", "00000000000"] {
        let uri = fixture.transcript_uri(&format!("event_contract={raw}"));
        let response = get_response(&fixture.app, &uri).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`event_contract={raw}` should have been refused"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "product_invalid_input", "for `{raw}`");
        let message = error["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains(raw) || raw.is_empty(),
            "the refusal must not echo the supplied value: {message}"
        );
    }

    // A declaration newer than this build is not malformed: the client decodes a
    // superset, so the intersection with this server is still the whole contract.
    let (status, future_bytes) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("event_contract=9")).await;
    assert_eq!(status, StatusCode::OK);
    let (_, legacy_bytes) = transcript_bytes(&fixture.app, &fixture.transcript_uri("")).await;
    assert_eq!(future_bytes, legacy_bytes);
}

#[tokio::test]
async fn product_transcript_parameterless_response_keeps_its_exact_shape() {
    let fixture = ContractFixture::start("Parameterless transcript").await;

    let (status, bytes) = transcript_bytes(&fixture.app, &fixture.transcript_uri("")).await;
    assert_eq!(status, StatusCode::OK);
    let transcript: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // The legacy response gained no field: same top-level keys, same segment keys.
    let mut keys: Vec<&str> = transcript
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "partial_reasons",
            "product_session_id",
            "segments",
            "status",
            "workspace_id",
        ],
        "the parameterless response must not grow fields: {transcript}"
    );
    assert!(
        transcript.get("has_more").is_none() && transcript.get("next_before_ordinal").is_none(),
        "a parameterless response carries no cursor fields: {transcript}"
    );
    let segment = transcript_segment(&transcript, 1);
    let mut segment_keys: Vec<&str> = segment
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    segment_keys.sort_unstable();
    assert_eq!(
        segment_keys,
        vec![
            "binding",
            "events",
            "inherited",
            "last_event_seq",
            "observed_through_seq",
            "run_status",
        ],
        "a segment must not grow fields either: {segment}"
    );

    // Declaring the current contract is a no-op for that response, and every
    // existing page parameter keeps its meaning next to it.
    let (_, current_bytes) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("event_contract=2")).await;
    assert_eq!(current_bytes, bytes);
    let (_, paged_legacy) =
        transcript_bytes(&fixture.app, &fixture.transcript_uri("limit_runs=1")).await;
    let (_, paged_current) = transcript_bytes(
        &fixture.app,
        &fixture.transcript_uri("limit_runs=1&event_contract=2"),
    )
    .await;
    assert_eq!(paged_current, paged_legacy);
}

// ─── R9: manual product-session compaction ─────────────────────────────────
//
// `POST /product/sessions/{session_id}/compact` is the API half of the CLI's
// `/compact`. It reuses the runtime's one manual-compaction implementation,
// persists what that produced, and is serialized against turns by the same
// exclusive per-session claim a turn takes. These tests pin the contract: the
// next turn really starts from the persisted summary, a busy session is refused
// without being touched, the breaker reports what the session actually holds,
// and a failed summary model is typed instead of reported as success.

/// The deterministic summary the `fake` profile answers a compaction with.
const COMPACTION_SUMMARY_SCRIPT: &str =
    "Compact summary:\nGoal: manual compaction of the product session history";

fn product_state_store(root: &Path) -> StateStore {
    StateStore::with_index_path(
        &root.join("api-state"),
        root.join(".rove/state.sqlite"),
        5_000,
    )
}

fn product_run_dir(root: &Path, run_id: RunId) -> PathBuf {
    find_run_dir(&root.join("api-state"), &run_id.to_string())
        .unwrap_or_else(|| panic!("run directory for {run_id}"))
}

async fn compact_product_session(app: &axum::Router, session_id: &str) -> axum::response::Response {
    post_json(
        app,
        &format!("/product/sessions/{session_id}/compact"),
        serde_json::json!({}),
    )
    .await
}

/// A product workspace with one session and, optionally, one finished turn.
struct CompactionFixture {
    _server: tempfile::TempDir,
    folder: tempfile::TempDir,
    app: axum::Router,
    _state: ApiState,
    workspace_id: String,
    session_id: String,
    run_id: Option<RunId>,
}

impl CompactionFixture {
    async fn start(label: &str, message: Option<&str>) -> Self {
        let server = tempfile::TempDir::new().unwrap();
        let folder = tempfile::TempDir::new().unwrap();
        let mut config = test_config();
        config.state.state_dir = "api-state".into();
        let state = ApiState::new(Workspace::detect(server.path()).unwrap(), config);
        let app = router(state.clone());
        let workspace = create_product_workspace(&app, folder.path()).await;
        let workspace_id = workspace["id"].as_str().unwrap().to_string();
        let session = create_product_session(&app, &workspace_id, label).await;
        let session_id = session["id"].as_str().unwrap().to_string();
        let mut fixture = Self {
            _server: server,
            folder,
            app,
            _state: state,
            workspace_id,
            session_id,
            run_id: None,
        };
        if let Some(message) = message {
            let job = create_product_job(&fixture.app, &fixture.session_id, message).await;
            let live = wait_for_done(fixture.app.clone(), job.job_id.to_string()).await;
            assert_product_runtime_terminal_durable(fixture.folder.path(), &job, &live).await;
            fixture.run_id = Some(job.run_id);
        }
        fixture
    }

    fn store(&self) -> StateStore {
        product_state_store(self.folder.path())
    }

    fn run_dir(&self, run_id: RunId) -> PathBuf {
        product_run_dir(self.folder.path(), run_id)
    }

    async fn task_state(&self, run_id: RunId) -> TaskState {
        self.store().load_task_state(run_id).await.unwrap()
    }

    /// The durable snapshot bytes, so a test can prove a request wrote nothing.
    fn task_state_bytes(&self, run_id: RunId) -> Vec<u8> {
        std::fs::read(self.run_dir(run_id).join("task_state.json")).unwrap()
    }

    fn trace_bytes(&self, run_id: RunId) -> Vec<u8> {
        std::fs::read(self.run_dir(run_id).join("trace.jsonl")).unwrap()
    }

    async fn compact(&self) -> axum::response::Response {
        compact_product_session(&self.app, &self.session_id).await
    }
}

#[tokio::test]
async fn product_session_compaction_persists_a_summary_the_next_turn_uses() {
    const FIRST: &str = "compaction first turn marker";
    const SECOND: &str = "compaction second turn marker";

    let fixture = CompactionFixture::start("Manual compaction", Some(FIRST)).await;
    let first_run = fixture.run_id.expect("the fixture ran a turn");
    let store = fixture.store();

    let before = fixture.task_state(first_run).await;
    assert!(
        !before.replayable_history("openai").unwrap().is_empty(),
        "the fixture has nothing to compact, so the test would prove nothing"
    );
    let trace_before = fixture.trace_bytes(first_run);

    let response = fixture.compact().await;
    let status = response.status();
    let facts: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::OK, "{facts}");
    assert_eq!(facts["product_session_id"], fixture.session_id);
    assert_eq!(facts["runtime_run_id"], first_run.to_string());
    assert_eq!(facts["triggered"], true);
    assert_eq!(facts["mode"], "model_generated");
    assert_eq!(facts["degraded"], false);
    assert_eq!(facts["consecutive_failures"], 0);
    assert_eq!(facts["circuit_open"], false);
    assert_eq!(facts["model"], "fake");
    assert_eq!(facts["prompt_version"], "rove.compaction.v3");
    assert!(
        facts["source_message_count"].as_u64().unwrap() >= 1,
        "{facts}"
    );
    assert_eq!(facts["summary"], COMPACTION_SUMMARY_SCRIPT);
    assert_eq!(facts["summary_truncated"], false);
    assert!(facts["token_estimate"].as_u64().unwrap() > 0, "{facts}");

    let compacted = fixture.task_state(first_run).await;
    assert!(
        compacted.history.is_empty(),
        "the replaced history survived in the compatibility projection: {compacted:#?}"
    );
    assert!(
        compacted.replayable_history("openai").unwrap().is_empty(),
        "the next prompt would carry both the summary and the history it replaces"
    );
    assert!(compacted.history_was_compacted_away());
    assert_eq!(
        compacted.summary.as_deref(),
        Some(COMPACTION_SUMMARY_SCRIPT)
    );
    let checkpoint = compacted.checkpoint.as_ref().expect("checkpoint");
    assert_eq!(
        checkpoint.summary.as_deref(),
        Some(COMPACTION_SUMMARY_SCRIPT)
    );
    assert!(checkpoint.session.is_none());
    assert!(checkpoint.preserved_tail.is_empty());
    assert_eq!(
        checkpoint.compaction.mode,
        PromptCompactionMode::ModelGenerated
    );
    assert!(
        !checkpoint.compaction.auto_triggered,
        "an operator-triggered compaction is not automatic"
    );
    assert!(!checkpoint.compaction.degraded);
    assert_eq!(checkpoint.compaction.consecutive_failures, 0);
    assert!(!checkpoint.compaction.circuit_open);
    assert_eq!(
        checkpoint.compaction.prompt_version.as_deref(),
        Some("rove.compaction.v3")
    );
    assert_eq!(
        facts["token_estimate"].as_u64().unwrap(),
        u64::try_from(checkpoint.token_estimate).unwrap()
    );

    // Compaction rewrites resumable state only. The source run keeps every fact
    // it recorded, and its lifecycle status is untouched.
    assert_eq!(
        fixture.trace_bytes(first_run),
        trace_before,
        "manual compaction wrote to the trace; it must only edit resumable state"
    );
    assert_eq!(
        store.index.run_record(first_run).unwrap().unwrap().status,
        "done"
    );

    // The next turn starts from the summary, and the turn the compaction
    // replaced is absent from its snapshot rather than replayed beside it.
    let second = create_product_job(&fixture.app, &fixture.session_id, SECOND).await;
    let second_live = wait_for_done(fixture.app.clone(), second.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(fixture.folder.path(), &second, &second_live).await;
    let resumed = fixture.task_state(second.run_id).await;
    assert_eq!(
        resumed.checkpoint.as_ref().unwrap().summary.as_deref(),
        Some(COMPACTION_SUMMARY_SCRIPT)
    );
    let serialized = serde_json::to_string(&resumed).unwrap();
    assert!(
        !serialized.contains(FIRST),
        "the compacted-away turn was replayed into the next run: {serialized}"
    );
    assert!(serialized.contains(SECOND));
}

#[tokio::test]
async fn product_session_compaction_refuses_a_session_with_an_active_turn() {
    let fixture = CompactionFixture::start("Busy compaction", Some("busy turn marker")).await;
    let first_run = fixture.run_id.expect("the fixture ran a turn");
    let before = fixture.task_state_bytes(first_run);

    // A genuinely active turn: the raw fake profile calls a tool, so the run
    // waits for an approval decision and the session stays running. Racing a
    // fast fake turn on purpose would be flaky; a pending approval is the same
    // product state and is deterministic.
    configure_product_session_model(&fixture.app, &fixture.session_id, "fake-raw", 1).await;
    let busy = create_product_job(
        &fixture.app,
        &fixture.session_id,
        r#"{"tool":"write_file","args":{"path":"compaction-busy.txt","content":"ok"}}"#,
    )
    .await;
    let pending = wait_for_approval_event(fixture.app.clone(), busy.job_id.to_string()).await;
    let approval = pending
        .pending_approvals
        .first()
        .expect("the busy turn waits for approval")
        .clone();

    let session =
        get_product_session(&fixture.app, &fixture.workspace_id, &fixture.session_id).await;
    assert_eq!(session["status"], "running");

    let response = fixture.compact().await;
    let status = response.status();
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "product_session_active");
    assert_eq!(
        fixture.task_state_bytes(first_run),
        before,
        "a refused compaction rewrote the snapshot"
    );

    // The turn reaches its terminal boundary, and only then does the same
    // request compact. This also proves the refusal released nothing it did not
    // own: the turn still completes normally.
    let approve = request_json(
        &fixture.app,
        "POST",
        &format!("/jobs/{}/approvals/{}", busy.job_id, approval.call_id),
        serde_json::json!({ "decision": "approve" }),
    )
    .await;
    assert_eq!(approve.status(), StatusCode::OK);
    wait_for_done(fixture.app.clone(), busy.job_id.to_string()).await;
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;

    let retried = fixture.compact().await;
    assert_eq!(retried.status(), StatusCode::OK);
    assert!(
        fixture
            .task_state(busy.run_id)
            .await
            .history_was_compacted_away()
    );
}

#[tokio::test]
async fn product_session_compaction_answers_nothing_to_compact_without_a_run() {
    let fixture = CompactionFixture::start("Empty compaction", None).await;

    let response = fixture.compact().await;
    let status = response.status();
    let facts: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::OK, "{facts}");
    assert_eq!(facts["triggered"], false);
    assert_eq!(facts["mode"], "none");
    assert_eq!(facts["degraded"], false);
    assert_eq!(facts["source_message_count"], 0);
    assert_eq!(facts["token_estimate"], 0);
    assert!(facts["runtime_run_id"].is_null(), "{facts}");
    assert!(facts["summary"].is_null(), "{facts}");

    // The claim was released: the session is claimable again afterwards.
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;
}

/// A dropped segment the runtime cannot bound is a refusal, not a no-op.
///
/// `triggered: false` cannot say which answer this is: "nothing to compact",
/// "already compacted", and "the material could not be sent" all answer with it.
/// A session whose newest replayable message is alone over the shrink request's
/// budget refuses to send anything, and the response has to name that refusal
/// rather than let a client render it as `nothing to compact`.
#[tokio::test]
async fn product_session_compaction_reports_a_segment_it_cannot_bound() {
    let fixture =
        CompactionFixture::start("Unbounded segment", Some("unbounded turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");
    let store = fixture.store();

    // Seed the segment the route would have to summarize: the run's canonical
    // session is replaced by the compatibility tail an older writer leaves, whose
    // one message is over the request budget by itself. This is the state that
    // makes the bound refuse, and it needs no 100 KiB HTTP body and no second
    // Provider to reach.
    let mut seeded = fixture.task_state(run_id).await;
    let summary_before = seeded
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.summary.clone());
    {
        let checkpoint = seeded.checkpoint.as_mut().expect("checkpoint");
        checkpoint.session = None;
        checkpoint.preserved_tail = vec![Message::assistant(
            "y".repeat(rove_runtime::compaction::COMPACTION_REQUEST_MAX_BYTES + 1),
        )];
    }
    store.write_task_state(&seeded).await.unwrap();
    let before = fixture.task_state_bytes(run_id);

    let response = fixture.compact().await;
    let status = response.status();
    let facts: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::OK, "{facts}");
    assert_eq!(facts["triggered"], false, "{facts}");
    assert_eq!(
        facts["failure_code"],
        rove_runtime::compaction::COMPACTION_REQUEST_UNBOUNDED_CODE,
        "a segment that cannot be bounded must not answer as `nothing to compact`: {facts}"
    );
    assert_eq!(
        facts["summary"].as_str(),
        summary_before.as_deref(),
        "the refusal must not produce or replace a summary: {facts}"
    );
    // The refusal is not a model failure and not a rewrite: nothing was charged
    // and nothing was persisted, so the operator can retry once the segment the
    // window dropped is smaller.
    assert_eq!(facts["degraded"], false, "{facts}");
    assert_eq!(facts["mode"], "none", "{facts}");
    assert_eq!(facts["consecutive_failures"], 0, "{facts}");
    assert_eq!(
        fixture.task_state_bytes(run_id),
        before,
        "a refusal must not rewrite the snapshot"
    );
}

#[tokio::test]
async fn product_session_compaction_rejects_an_unknown_session() {
    let fixture = CompactionFixture::start("Unknown compaction", None).await;
    let unknown = ProductSessionId::new().to_string();

    let response = post_json(
        &fixture.app,
        &format!("/product/sessions/{unknown}/compact"),
        serde_json::json!({}),
    )
    .await;
    let status = response.status();
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["code"], "product_not_found");
}

#[tokio::test]
async fn product_session_compaction_probes_a_tripped_breaker_and_clears_it() {
    let fixture = CompactionFixture::start("Tripped breaker", Some("breaker turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");
    let store = fixture.store();

    // Seed the state a session accumulates once its summary model has failed to
    // the threshold. Reaching the threshold over HTTP needs an automatic
    // compaction, which needs a tripped token budget, so the failure history is
    // written to the same durable snapshot the route reads.
    let mut seeded = fixture.task_state(run_id).await;
    {
        let checkpoint = seeded.checkpoint.as_mut().expect("checkpoint");
        checkpoint.compaction.mode = PromptCompactionMode::Degraded;
        checkpoint.compaction.degraded = true;
        checkpoint.compaction.consecutive_failures = 3;
        checkpoint.compaction.circuit_open = true;
        checkpoint.compaction.last_error = Some("summary model call failed".to_string());
    }
    store.write_task_state(&seeded).await.unwrap();
    assert!(
        !fixture
            .task_state(run_id)
            .await
            .replayable_history("openai")
            .unwrap()
            .is_empty(),
        "the breaker test needs history the route could otherwise compact"
    );

    // The count is inherited from the snapshot, so the response reports the
    // session's real state instead of a fresh one — and an explicit request
    // still runs. It is the operator's single probe of that breaker, and
    // refusing it would leave a session whose provider recovered with no request
    // that could ever clear it.
    let response = fixture.compact().await;
    let status = response.status();
    let facts: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::OK, "{facts}");
    assert_eq!(facts["triggered"], true, "{facts}");
    assert_eq!(facts["mode"], "model_generated");
    assert_eq!(facts["degraded"], false);
    assert_eq!(facts["consecutive_failures"], 0, "{facts}");
    assert_eq!(facts["circuit_open"], false, "{facts}");

    // A successful probe rewrites the persisted count to zero, which is what
    // makes the automatic path able to compact this session again.
    let compacted = fixture.task_state(run_id).await;
    assert!(compacted.history_was_compacted_away());
    let checkpoint = compacted.checkpoint.as_ref().expect("checkpoint");
    assert_eq!(checkpoint.compaction.consecutive_failures, 0);
    assert!(!checkpoint.compaction.circuit_open);
    assert!(!checkpoint.compaction.degraded);
}

#[tokio::test]
async fn product_session_compaction_reports_a_failed_breaker_probe_without_retrying() {
    let fixture = CompactionFixture::start("Failed probe", Some("failed probe turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");
    let store = fixture.store();

    let mut seeded = fixture.task_state(run_id).await;
    {
        let checkpoint = seeded.checkpoint.as_mut().expect("checkpoint");
        checkpoint.compaction.mode = PromptCompactionMode::Degraded;
        checkpoint.compaction.degraded = true;
        checkpoint.compaction.consecutive_failures = 3;
        checkpoint.compaction.circuit_open = true;
    }
    store.write_task_state(&seeded).await.unwrap();
    select_unreachable_summary_provider(&fixture).await;

    let response = fixture.compact().await;
    let status = response.status();
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    let facts: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(facts["triggered"], true, "{text}");
    assert_eq!(facts["degraded"], true, "{text}");
    assert_eq!(facts["mode"], "degraded");
    // One request, one attempt: the count moves by one. A retry loop would move
    // it by the retry budget and make the number meaningless to an operator.
    assert_eq!(facts["consecutive_failures"], 4, "{text}");
    assert_eq!(facts["circuit_open"], true, "{text}");
    assert!(
        facts["failure_code"]
            .as_str()
            .is_some_and(|code| !code.is_empty()),
        "a failed probe must classify the failure: {text}"
    );
    assert!(
        !text.contains("127.0.0.1"),
        "the provider error leaked into the response: {text}"
    );
    // A failed probe does not just trip the breaker; it arms the durable window
    // the automatic path has to respect. The response reports it and the snapshot
    // keeps it, so a restart reads the same answer.
    let armed = facts["next_attempt_after"]
        .as_str()
        .expect("a failed probe reports the window it armed")
        .to_string();
    assert!(
        chrono::DateTime::parse_from_rfc3339(&armed).is_ok(),
        "the reported window must be a timestamp: {armed}"
    );

    let state = fixture.task_state(run_id).await;
    assert!(state.history_was_compacted_away());
    let checkpoint = state.checkpoint.as_ref().expect("checkpoint");
    assert_eq!(checkpoint.compaction.consecutive_failures, 4);
    assert!(checkpoint.compaction.circuit_open);
    assert_eq!(
        checkpoint.compaction.next_attempt_after.as_deref(),
        Some(armed.as_str()),
        "the window the response reports must be the one on disk"
    );
}

/// Point the fixture's session at a closed loopback Provider.
///
/// An Ollama profile needs no credential, so it can be selected and built, and a
/// closed loopback port makes the summary call fail deterministically without
/// reaching any external service.
async fn select_unreachable_summary_provider(fixture: &CompactionFixture) {
    let profile = post_json(
        &fixture.app,
        "/product/provider-profiles",
        serde_json::json!({
            "label": "Unreachable summary provider",
            "provider_type": "ollama",
            "api_base": "http://127.0.0.1:1",
            "default_model": "unreachable-summary-model"
        }),
    )
    .await;
    assert_eq!(profile.status(), StatusCode::CREATED);
    let profile: serde_json::Value = decode_json(profile).await;
    let profile_id = profile["id"].as_str().unwrap().to_string();

    let current = get_response(
        &fixture.app,
        &format!("/product/sessions/{}/model-config", fixture.session_id),
    )
    .await;
    let current: serde_json::Value = decode_json(current).await;
    let configured = request_json(
        &fixture.app,
        "PUT",
        &format!("/product/sessions/{}/model-config", fixture.session_id),
        serde_json::json!({
            "profile_id": profile_id,
            "model": "unreachable-summary-model",
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": current["revision"]
        }),
    )
    .await;
    assert_eq!(configured.status(), StatusCode::OK);

    // The grant has to follow the selection: the capability digest covers the
    // workspace's Provider selector, so writing a new selection invalidates an
    // earlier grant.
    let trust = request_json(
        &fixture.app,
        "PUT",
        &format!("/product/workspaces/{}/trust", fixture.workspace_id),
        serde_json::json!({ "decision": "grant", "capabilities": ["provider_credentials"] }),
    )
    .await;
    assert_eq!(trust.status(), StatusCode::OK);
}

#[tokio::test]
async fn product_session_compaction_reports_a_degraded_fallback_without_the_provider_error() {
    let fixture =
        CompactionFixture::start("Degraded compaction", Some("degraded turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");

    select_unreachable_summary_provider(&fixture).await;

    let response = fixture.compact().await;
    let status = response.status();
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    let facts: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(facts["triggered"], true, "{text}");
    assert_eq!(facts["degraded"], true, "{text}");
    assert_eq!(facts["mode"], "degraded");
    assert_eq!(facts["consecutive_failures"], 1, "{text}");
    assert_eq!(facts["circuit_open"], false);
    assert!(
        facts["failure_code"]
            .as_str()
            .is_some_and(|code| !code.is_empty()),
        "a degraded compaction must classify the failure: {text}"
    );
    assert!(
        !text.contains("127.0.0.1"),
        "the provider error leaked into the response: {text}"
    );
    // The provider message is one of several things the response could leak, so
    // the guarantee is pinned by the whole projection: every key is one the
    // contract declares, and the runtime's own error text has no key at all.
    let mut keys: Vec<&str> = facts
        .as_object()
        .expect("the answer is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "circuit_open",
            "consecutive_failures",
            "degraded",
            "failure_code",
            "mode",
            "model",
            "product_session_id",
            "prompt_version",
            "runtime_run_id",
            "source_message_count",
            "summary",
            "summary_truncated",
            "token_estimate",
            "triggered",
        ],
        "the response grew a key the contract does not declare: {text}"
    );
    assert!(
        facts.get("last_error").is_none(),
        "the runtime's last_error reached the response: {text}"
    );
    // The failed probe armed a cooldown, but the breaker is still below the
    // threshold, so nothing is being refused and the answer must not report a
    // window: `next_attempt_after` is the refusal's deadline, not a log of every
    // window the session ever armed.
    assert!(
        facts["next_attempt_after"].is_null(),
        "an inert window must not ship beside circuit_open: false: {text}"
    );

    // The deterministic fallback is still a summary, so the session is
    // compacted and the failure is durable.
    let state = fixture.task_state(run_id).await;
    assert!(state.history_was_compacted_away());
    let checkpoint = state.checkpoint.as_ref().unwrap();
    assert_eq!(checkpoint.compaction.mode, PromptCompactionMode::Degraded);
    assert!(checkpoint.compaction.last_error.is_some());
    // The window is on disk either way: the response is honest about what it
    // refuses, the snapshot keeps the deadline the automatic path will read.
    assert!(
        checkpoint
            .compaction
            .next_attempt_after
            .as_deref()
            .is_some_and(|deadline| chrono::DateTime::parse_from_rfc3339(deadline).is_ok()),
        "a failed probe must arm a parseable window on disk"
    );
    assert!(
        checkpoint
            .summary
            .as_deref()
            .is_some_and(|summary| !summary.is_empty())
    );

    // A repeat on the already-compacted session neither spends another model
    // call nor reports a fresh breaker: the count comes from the snapshot.
    let repeat = fixture.compact().await;
    let repeat_status = repeat.status();
    let repeat: serde_json::Value = decode_json(repeat).await;
    assert_eq!(repeat_status, StatusCode::OK, "{repeat}");
    assert_eq!(repeat["triggered"], false, "{repeat}");
    assert_eq!(repeat["consecutive_failures"], 1, "{repeat}");
    assert_eq!(repeat["degraded"], true);
}

/// A Provider whose summary call never answers.
///
/// A compaction is exercised through the model, so a request that has to still
/// be in flight when a test looks at it needs a Provider that does not return.
/// The listener accepts the connection and never writes a response, which keeps
/// the call pending until the test drops the request.
struct StallingSummaryProvider {
    listener: tokio::task::JoinHandle<()>,
}

impl StallingSummaryProvider {
    /// Select a stalling Provider for the fixture's session. The listener lives
    /// as long as the returned value.
    async fn start(fixture: &CompactionFixture) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let accepting = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                // Held open without an answer, so the Provider call stays in
                // flight instead of failing or completing.
                tokio::spawn(async move {
                    let _held = stream;
                    std::future::pending::<()>().await;
                });
            }
        });

        let profile = post_json(
            &fixture.app,
            "/product/provider-profiles",
            serde_json::json!({
                "label": "Stalling summary provider",
                "provider_type": "ollama",
                "api_base": api_base,
                "default_model": "stalling-summary-model"
            }),
        )
        .await;
        assert_eq!(profile.status(), StatusCode::CREATED);
        let profile: serde_json::Value = decode_json(profile).await;
        let profile_id = profile["id"].as_str().unwrap().to_string();

        let current = get_response(
            &fixture.app,
            &format!("/product/sessions/{}/model-config", fixture.session_id),
        )
        .await;
        let current: serde_json::Value = decode_json(current).await;
        let configured = request_json(
            &fixture.app,
            "PUT",
            &format!("/product/sessions/{}/model-config", fixture.session_id),
            serde_json::json!({
                "profile_id": profile_id,
                "model": "stalling-summary-model",
                "reasoning": "default",
                "max_steps": 1,
                "expected_revision": current["revision"]
            }),
        )
        .await;
        assert_eq!(configured.status(), StatusCode::OK);

        let trust = request_json(
            &fixture.app,
            "PUT",
            &format!("/product/workspaces/{}/trust", fixture.workspace_id),
            serde_json::json!({ "decision": "grant", "capabilities": ["provider_credentials"] }),
        )
        .await;
        assert_eq!(trust.status(), StatusCode::OK);

        Self {
            listener: accepting,
        }
    }
}

impl Drop for StallingSummaryProvider {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

/// Start a compaction request whose future the test owns, so it can be aborted
/// while the handler is mid-flight.
fn spawn_compaction(
    fixture: &CompactionFixture,
) -> tokio::task::JoinHandle<axum::response::Response> {
    let app = fixture.app.clone();
    let session_id = fixture.session_id.clone();
    tokio::spawn(async move { compact_product_session(&app, &session_id).await })
}

/// A dropped request must not strand the claim it took.
///
/// An aborted request or a disconnected client runs none of the handler's
/// release paths. The claim row and `status = 'running'` would survive the
/// request, every later turn and compaction would answer 409
/// `product_session_active`, and the next API start would convert the session to
/// `needs_attention` — which the claim path refuses as well. Only the guard's
/// `Drop` runs on that path.
#[tokio::test]
async fn product_session_compaction_releases_its_claim_when_the_request_is_aborted() {
    let fixture = CompactionFixture::start("Aborted compaction", Some("aborted turn marker")).await;
    let _stalling = StallingSummaryProvider::start(&fixture).await;

    let abandoned = spawn_compaction(&fixture);
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "running",
    )
    .await;
    abandoned.abort();
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;

    // And the session is usable again: a later request gets past the claim and
    // reaches the model instead of being refused with `product_session_active`,
    // which is what an orphaned claim answers forever.
    let retried = spawn_compaction(&fixture);
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "running",
    )
    .await;
    retried.abort();
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;
}

/// Two compactions of one session: the claim lets exactly one in.
#[tokio::test]
async fn product_session_compaction_refuses_a_second_request_and_stays_usable() {
    let fixture =
        CompactionFixture::start("Concurrent compaction", Some("concurrent turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");
    let _stalling = StallingSummaryProvider::start(&fixture).await;
    let persisted_before = fixture.task_state_bytes(run_id);

    let first = spawn_compaction(&fixture);
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "running",
    )
    .await;

    let second = fixture.compact().await;
    let status = second.status();
    let error: serde_json::Value = decode_json(second).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "product_session_active", "{error}");
    assert_eq!(
        fixture.task_state_bytes(run_id),
        persisted_before,
        "the refused compaction rewrote the snapshot"
    );

    // The refusal is the in-flight request's, not a broken session: once that
    // request is gone the session takes the next one.
    first.abort();
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;
    let retried = spawn_compaction(&fixture);
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "running",
    )
    .await;
    retried.abort();
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;
}

#[tokio::test]
async fn product_session_compaction_touches_only_the_targeted_session() {
    let fixture = CompactionFixture::start("Compacted session", Some("target turn marker")).await;
    let target_run = fixture.run_id.expect("the fixture ran a turn");

    let other = tempfile::TempDir::new().unwrap();
    let other_workspace = create_product_workspace(&fixture.app, other.path()).await;
    let other_workspace_id = other_workspace["id"].as_str().unwrap().to_string();
    let other_session =
        create_product_session(&fixture.app, &other_workspace_id, "Untouched session").await;
    let other_session_id = other_session["id"].as_str().unwrap().to_string();
    let other_job =
        create_product_job(&fixture.app, &other_session_id, "untouched turn marker").await;
    let other_live = wait_for_done(fixture.app.clone(), other_job.job_id.to_string()).await;
    assert_product_runtime_terminal_durable(other.path(), &other_job, &other_live).await;
    let other_before =
        std::fs::read(product_run_dir(other.path(), other_job.run_id).join("task_state.json"))
            .unwrap();

    let response = fixture.compact().await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        fixture
            .task_state(target_run)
            .await
            .history_was_compacted_away()
    );

    assert_eq!(
        std::fs::read(product_run_dir(other.path(), other_job.run_id).join("task_state.json"))
            .unwrap(),
        other_before,
        "compacting one product session rewrote another workspace's snapshot"
    );
    let other_session_row =
        get_product_session(&fixture.app, &other_workspace_id, &other_session_id).await;
    assert_eq!(other_session_row["status"], "idle");
}

#[tokio::test]
async fn product_session_compaction_fails_closed_for_a_revoked_workspace() {
    let fixture = CompactionFixture::start("Revoked compaction", Some("revoked turn marker")).await;
    let run_id = fixture.run_id.expect("the fixture ran a turn");
    let before = fixture.task_state_bytes(run_id);

    let revoked = request_json(
        &fixture.app,
        "PUT",
        &format!("/product/workspaces/{}/trust", fixture.workspace_id),
        serde_json::json!({ "decision": "revoke", "capabilities": [] }),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::OK);

    let response = fixture.compact().await;
    let status = response.status();
    let error: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "project_trust_required");
    assert_eq!(
        fixture.task_state_bytes(run_id),
        before,
        "a refused compaction rewrote the snapshot"
    );

    // The refusal released the claim on the error path, so the session is idle
    // again instead of being stuck as running.
    wait_for_product_session_status(
        &fixture.app,
        &fixture.workspace_id,
        &fixture.session_id,
        "idle",
    )
    .await;
}

async fn post_json(
    app: &axum::Router,
    uri: &str,
    value: serde_json::Value,
) -> axum::response::Response {
    request_json(app, "POST", uri, value).await
}

async fn get_response(app: &axum::Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn request_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    value: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[cfg(unix)]
fn create_test_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_test_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

async fn decode_json<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// The response body exactly as it went on the wire, so a test can assert on
/// the bytes a client sees rather than on a re-serialization of them.
async fn raw_body(response: axum::response::Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

fn write_product_memory_topic(memory_dir: &Path, slug: &str, title: &str, body: &str) {
    std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        format!("# rove Memory\n\n- [{title}](topics/{slug}.md) - project memory\n"),
    )
    .unwrap();
    std::fs::write(
        memory_dir.join("topics").join(format!("{slug}.md")),
        format!(
            "---\ntitle: {title}\ntype: project\nscope: project\nconfidence: 0.9\n---\n{body}\n"
        ),
    )
    .unwrap();
}

/// Locate a run directory by id under a root, whatever state layout produced it.
///
/// Which layout a product run lands in depends on config resolution, and the
/// point of the test using this is the sidecar's presence, not the path.
fn find_run_dir(root: &Path, run_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == run_id) {
            return Some(path);
        }
        if let Some(found) = find_run_dir(&path, run_id) {
            return Some(found);
        }
    }
    None
}

async fn create_product_workspace(app: &axum::Router, root: &Path) -> serde_json::Value {
    let response = post_json(
        app,
        "/product/workspaces",
        serde_json::json!({
            "root": root,
            "kind": "folder",
            "display_name": "Product test workspace",
            "pinned": false
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    decode_json(response).await
}

async fn create_product_session(
    app: &axum::Router,
    workspace_id: &str,
    title: &str,
) -> serde_json::Value {
    let response = post_json(
        app,
        "/product/sessions",
        serde_json::json!({
            "workspace_id": workspace_id,
            "title": title
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    decode_json(response).await
}

async fn get_product_session(
    app: &axum::Router,
    workspace_id: &str,
    product_session_id: &str,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions?workspace_id={workspace_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == product_session_id)
        .cloned()
        .expect("product session")
}

async fn list_product_controls(
    app: &axum::Router,
    product_session_id: &str,
) -> Vec<serde_json::Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{product_session_id}/controls"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await;
    body["controls"].as_array().unwrap().clone()
}

/// The budget every 25ms state wait in this file gives a transition another task writes.
///
/// These loops poll state a runtime task, a detached release task or a background
/// transition produces, so they wait on the scheduler rather than on a bound, and
/// the count they used to carry *was* the assertion: 80 iterations at 25ms is two
/// seconds, 120 is three, 400 is ten. They expired on a loaded runner — the
/// `rust default` job of one CI run failed
/// `product_session_compaction_releases_its_claim_when_the_request_is_aborted` with
/// the session still `running` while the other `rust default` job of the same run
/// passed the same commit, and a local full-suite run under heavy Web load failed
/// `product_steer_submitted_during_generation_applies_after_the_tool_safe_point`
/// (a two-second `wait_for_pending_input`) while that test passed on its own. Both
/// are scheduling budgets, not behaviours: an orphaned claim stays `running`
/// forever and a job that never waits for input never waits, so every one of these
/// waits still fails its test, fifteen seconds later instead of two.
///
/// The boot-recovery poll in
/// `deleting_the_product_catalog_recovers_the_session_list_on_the_next_start`
/// keeps its own 50ms interval; it is a separate wait with its own comment.
///
/// The two product-status waits also print the budget they waited, because that is
/// where a CI failure was ambiguous; the others print the last state they observed,
/// which is the diagnostic half.
const STATE_WAIT_ATTEMPTS: usize = 600;
const STATE_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);

/// The elapsed budget, for the panic messages that state it: a failure should say
/// how long the wait actually waited instead of leaving that to the reader.
fn state_wait_budget() -> std::time::Duration {
    STATE_WAIT_INTERVAL * STATE_WAIT_ATTEMPTS as u32
}

async fn wait_for_product_control_status(
    app: &axum::Router,
    product_session_id: &str,
    control_id: &str,
    expected_status: &str,
) -> serde_json::Value {
    wait_for_product_control_status_within(
        app,
        product_session_id,
        control_id,
        expected_status,
        STATE_WAIT_ATTEMPTS,
    )
    .await
}

/// The same wait with a caller-chosen budget, for a transition whose implemented
/// schedule deliberately takes longer than one state-wait budget (the successor
/// start budget spends about seven seconds on backoff before it escalates).
async fn wait_for_product_control_status_within(
    app: &axum::Router,
    product_session_id: &str,
    control_id: &str,
    expected_status: &str,
    attempts: usize,
) -> serde_json::Value {
    let mut last_control = None;
    for _ in 0..attempts {
        let control = list_product_controls(app, product_session_id)
            .await
            .into_iter()
            .find(|control| control["id"] == control_id)
            .expect("product control");
        if control["status"] == expected_status {
            return control;
        }
        last_control = Some(control);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!(
        "product control {control_id} did not reach {expected_status} within {}ms; last control: {last_control:?}",
        STATE_WAIT_INTERVAL.as_millis() * attempts as u128
    );
}

async fn wait_for_product_session_status(
    app: &axum::Router,
    workspace_id: &str,
    product_session_id: &str,
    expected_status: &str,
) -> serde_json::Value {
    let mut last_session = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let session = get_product_session(app, workspace_id, product_session_id).await;
        if session["status"] == expected_status {
            return session;
        }
        last_session = Some(session);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!(
        "product session {product_session_id} did not reach {expected_status} within {}ms; last session: {last_session:?}",
        state_wait_budget().as_millis()
    );
}

async fn assert_product_runtime_terminal_durable(
    workspace_root: &Path,
    created: &CreateJobResponse,
    live_state: &JobStateResponse,
) {
    let state_store = StateStore::with_index_path(
        &workspace_root.join("api-state"),
        workspace_root.join(".rove/state.sqlite"),
        5_000,
    );
    let run = state_store
        .index
        .run_record(created.run_id)
        .unwrap()
        .expect("indexed runtime run");
    assert_eq!(run.job_id, created.job_id);
    assert_eq!(run.run_id, created.run_id);
    assert_eq!(run.status, "done");
    assert!(run.task_state_path.is_some());
    assert!(run.report_path.is_some());
    assert!(run.last_event_seq > 0);
    assert_eq!(
        live_state.events.last().map(|event| event.seq),
        Some(run.last_event_seq)
    );

    let task_state = state_store.load_task_state(created.run_id).await.unwrap();
    assert_eq!(task_state.session_id, run.session_id);
    assert_eq!(task_state.job_id, created.job_id);
    assert_eq!(task_state.run_id, created.run_id);
    assert_eq!(
        task_state
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.last_event_seq),
        Some(run.last_event_seq)
    );
    let report = state_store.load_report(created.run_id).await.unwrap();
    assert_eq!(report.session_id, run.session_id);
    assert_eq!(report.job_id, created.job_id);
    assert_eq!(report.run_id, created.run_id);
    let snapshot = state_store
        .index
        .run_event_snapshot_async(created.run_id, run.last_event_seq - 1, 1)
        .await
        .unwrap()
        .expect("terminal event snapshot");
    assert_eq!(snapshot.high_water_seq, run.last_event_seq);
    assert!(matches!(
        snapshot
            .events
            .last()
            .map(|event| serde_json::from_str::<StreamEvent>(&event.event_json).unwrap()),
        Some(StreamEvent::RunCompleted { .. })
    ));
}

async fn configure_product_session_model(
    app: &axum::Router,
    product_session_id: &str,
    model: &str,
    max_steps: u32,
) {
    let current = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{product_session_id}/model-config"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(current.status(), StatusCode::OK);
    let current: serde_json::Value = decode_json(current).await;
    let response = request_json(
        app,
        "PUT",
        &format!("/product/sessions/{product_session_id}/model-config"),
        serde_json::json!({
            "model": model,
            "reasoning": "default",
            "max_steps": max_steps,
            "expected_revision": current["revision"]
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// Selects a Provider profile for a product session.
///
/// The two `fake`/`fake-raw` model shortcuts need no profile, but any other
/// model is assembled from the selected catalog profile, so a test that wants a
/// scripted fake has to select one the same way the product UI does.
async fn select_product_session_profile(
    app: &axum::Router,
    product_session_id: &str,
    profile_id: &str,
    model: &str,
) {
    let current = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{product_session_id}/model-config"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(current.status(), StatusCode::OK);
    let current: serde_json::Value = decode_json(current).await;
    let response = request_json(
        app,
        "PUT",
        &format!("/product/sessions/{product_session_id}/model-config"),
        serde_json::json!({
            "profile_id": profile_id,
            "model": model,
            "reasoning": "default",
            "max_steps": 1,
            "expected_revision": current["revision"]
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "profile `{profile_id}` could not be selected: {}",
        raw_body(response).await
    );
}

async fn create_product_job(
    app: &axum::Router,
    product_session_id: &str,
    message: &str,
) -> CreateJobResponse {
    let response = post_json(
        app,
        "/jobs",
        serde_json::json!({
            "message": message,
            "product_session_id": product_session_id
        }),
    )
    .await;
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        panic!(
            "product job start failed with {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    decode_json(response).await
}

async fn wait_for_done(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if state.status == RunStatus::Done {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not finish; last state: {last_state:?}");
}

async fn wait_for_pending_input(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if !state.pending_inputs.is_empty() {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not wait for input; last state: {last_state:?}");
}

async fn wait_for_input_event(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if state
            .events
            .iter()
            .any(|stored| matches!(&stored.event, StreamEvent::InputNeeded { .. }))
        {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not publish input event; last state: {last_state:?}");
}

async fn wait_for_approval_event(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if state
            .events
            .iter()
            .any(|stored| matches!(&stored.event, StreamEvent::ToolCallApprovalNeeded { .. }))
        {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not publish approval event; last state: {last_state:?}");
}

async fn wait_for_pending_approval(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if !state.pending_approvals.is_empty() {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not wait for approval; last state: {last_state:?}");
}

/// Waits until the run has published a specific streamed assistant delta.
///
/// The scripted fake holds its turn right after that delta, so this returns
/// while the turn is still in flight — the state a stop must interrupt for the
/// marker to be produced at all. Matching the text rather than "any chunk" keeps
/// an earlier model turn (the planner's) from satisfying the wait.
async fn wait_for_streamed_text(
    app: axum::Router,
    job_id: String,
    expected: &str,
) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if state.events.iter().any(
            |event| matches!(&event.event, StreamEvent::LlmChunk { delta } if delta == expected),
        ) {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job never published the expected streamed text; last state: {last_state:?}");
}

async fn wait_for_status(
    app: axum::Router,
    job_id: String,
    expected: RunStatus,
) -> JobStateResponse {
    let mut last_state = None;
    // Matches `wait_for_done`. The lifecycle finalizer adds real events to the
    // terminal tail, so a shorter budget than the sibling helper only measures
    // scheduler load under a parallel suite rather than the asserted status.
    for _ in 0..STATE_WAIT_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/jobs/{job_id}/state"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let state: JobStateResponse = serde_json::from_slice(&body).unwrap();
        if state.status == expected {
            return state;
        }
        last_state = Some(state);
        tokio::time::sleep(STATE_WAIT_INTERVAL).await;
    }
    panic!("job did not reach {expected:?}; last state: {last_state:?}");
}

#[derive(Default)]
struct CapturedProviderRequests {
    models_auth: Option<String>,
    chat_auth: Option<String>,
    chat_model: Option<String>,
    responses_auth: Option<String>,
    responses_model: Option<String>,
    responses_body: Option<serde_json::Value>,
    anthropic_auth: Option<String>,
    anthropic_model: Option<String>,
    ollama_model: Option<String>,
}

struct OpenAiTestServer {
    base_url: String,
    captured: Arc<Mutex<CapturedProviderRequests>>,
}

struct DelayedToolOpenAiServer {
    base_url: String,
    first_generation_started: Arc<Notify>,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

struct ProviderProtocolTestServer {
    base_url: String,
    captured: Arc<Mutex<CapturedProviderRequests>>,
}

fn sse_response(
    frames: Vec<serde_json::Value>,
) -> ([(axum::http::HeaderName, &'static str); 1], String) {
    let body = frames
        .into_iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect::<String>();
    ([(CONTENT_TYPE, "text/event-stream")], body)
}

async fn start_openai_test_server() -> OpenAiTestServer {
    let captured = Arc::new(Mutex::new(CapturedProviderRequests::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/v1/models",
            get({
                let captured = captured.clone();
                move |headers: HeaderMap| {
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().models_auth = headers
                            .get(AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        Json(serde_json::json!({
                            "data": [
                                { "id": "relay/deepseek-v3.2", "owned_by": "relay" },
                                { "id": "official/gpt-compatible", "owned_by": "official" }
                            ]
                        }))
                    }
                }
            }),
        )
        .route(
            "/v1/models-unauthorized",
            get(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "invalid key upstream-secret-provider-token"
                    })),
                )
            }),
        )
        .route(
            "/v1/models-rate-limited",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({ "error": "slow down" })),
                )
            }),
        )
        .route(
            "/v1/models-invalid",
            get(|| async {
                (
                    [(CONTENT_TYPE, "application/json")],
                    "this is not json",
                )
            }),
        )
        .route(
            "/v1/models-empty",
            get(|| async { Json(serde_json::json!({ "data": [] })) }),
        )
        .route(
            "/v1/models-slow",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                Json(serde_json::json!({
                    "data": [{ "id": "eventually" }]
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post({
                let captured = captured.clone();
                move |headers: HeaderMap,
                      AxumState(()): AxumState<()>,
                      Json(body): Json<serde_json::Value>| {
                    let captured = captured.clone();
                    async move {
                        let mut captured = captured.lock().unwrap();
                        captured.chat_auth = headers
                            .get(AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        captured.chat_model = body
                            .get("model")
                            .and_then(|value| value.as_str())
                            .map(str::to_string);
                        let content = if body
                            .get("messages")
                            .and_then(|value| value.as_array())
                            .and_then(|messages| messages.first())
                            .and_then(|message| message.get("content"))
                            .and_then(|value| value.as_str())
                            .is_some_and(|content| content.contains("You are the planner for rove"))
                        {
                            r#"{"goal":"routed provider job","steps":[{"id":"1","title":"reply"}]}"#
                        } else {
                            "routed provider ok"
                        };
                        let chunk = serde_json::json!({
                            "choices": [
                                {
                                    "delta": {
                                        "content": content
                                    }
                                }
                            ]
                        });
                        let body = format!("data: {}\n\ndata: [DONE]\n\n", chunk);
                        ([(CONTENT_TYPE, "text/event-stream")], body)
                    }
                }
            }),
        )
        .route(
            "/v1/responses",
            post({
                let captured = captured.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let captured = captured.clone();
                    async move {
                        {
                            let mut captured = captured.lock().unwrap();
                            captured.responses_auth = headers
                                .get(AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_string);
                            captured.responses_model = body
                                .get("model")
                                .and_then(|value| value.as_str())
                                .map(str::to_string);
                            captured.responses_body = Some(body.clone());
                        }
                        let text = if body
                            .get("instructions")
                            .and_then(|value| value.as_str())
                            .is_some_and(|content| content.contains("You are the planner for rove"))
                            || body
                            .get("input")
                            .and_then(|value| value.as_array())
                            .into_iter()
                            .flatten()
                            .any(|item| {
                                item.get("content")
                                    .and_then(|value| value.as_array())
                                    .into_iter()
                                    .flatten()
                                    .any(|content| {
                                        content
                                            .get("text")
                                            .and_then(|value| value.as_str())
                                            .is_some_and(|text| {
                                                text.contains("You are the planner for rove")
                                            })
                                    })
                            })
                        {
                            r#"{"goal":"responses profile job","steps":[{"id":"1","title":"reply"}]}"#
                        } else {
                            "responses profile ok"
                        };
                        sse_response(vec![
                            serde_json::json!({
                                "type": "response.output_text.delta",
                                "delta": text
                            }),
                            serde_json::json!({
                                "type": "response.completed",
                                "response": {
                                    "usage": {
                                        "input_tokens": 1,
                                        "output_tokens": 1,
                                        "total_tokens": 2
                                    }
                                }
                            }),
                        ])
                    }
                }
            }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    OpenAiTestServer {
        base_url: format!("http://{addr}"),
        captured,
    }
}

async fn start_delayed_tool_openai_server() -> DelayedToolOpenAiServer {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_count = Arc::new(AtomicUsize::new(0));
    let first_generation_started = Arc::new(Notify::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let requests = requests.clone();
            let request_count = request_count.clone();
            let first_generation_started = first_generation_started.clone();
            move |Json(body): Json<serde_json::Value>| {
                let requests = requests.clone();
                let request_count = request_count.clone();
                let first_generation_started = first_generation_started.clone();
                async move {
                    let is_planner = body
                        .get("messages")
                        .and_then(|value| value.as_array())
                        .into_iter()
                        .flatten()
                        .any(|message| {
                            message
                                .get("content")
                                .and_then(|value| value.as_str())
                                .is_some_and(|content| {
                                    content.contains("You are the planner for rove")
                                })
                        });
                    if is_planner {
                        let plan = serde_json::json!({
                            "choices": [{
                                "delta": {
                                    "content": "{\"goal\":\"generation steer\",\"steps\":[{\"id\":\"1\",\"title\":\"call echo and answer\"}]}"
                                },
                                "finish_reason": "stop"
                            }]
                        });
                        return (
                            [(CONTENT_TYPE, "text/event-stream")],
                            format!("data: {plan}\n\ndata: [DONE]\n\n"),
                        );
                    }
                    requests.lock().unwrap().push(body);
                    let ordinal = request_count.fetch_add(1, Ordering::SeqCst);
                    if ordinal == 0 {
                        first_generation_started.notify_one();
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        let tool = serde_json::json!({
                            "choices": [{
                                "delta": {
                                    "tool_calls": [{
                                        "index": 0,
                                        "id": "generation_call_1",
                                        "function": {
                                            "name": "echo",
                                            "arguments": "{\"message\":\"tool-safe-point\"}"
                                        }
                                    }]
                                },
                                "finish_reason": "tool_calls"
                            }],
                            "usage": {
                                "prompt_tokens": 2,
                                "completion_tokens": 1,
                                "total_tokens": 3
                            }
                        });
                        return (
                            [(CONTENT_TYPE, "text/event-stream")],
                            format!("data: {tool}\n\ndata: [DONE]\n\n"),
                        );
                    }
                    let final_chunk = serde_json::json!({
                        "choices": [{
                            "delta": {"content": "generation steer applied"},
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 4,
                            "completion_tokens": 2,
                            "total_tokens": 6
                        }
                    });
                    (
                        [(CONTENT_TYPE, "text/event-stream")],
                        format!("data: {final_chunk}\n\ndata: [DONE]\n\n"),
                    )
                }
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    DelayedToolOpenAiServer {
        base_url: format!("http://{addr}"),
        first_generation_started,
        requests,
    }
}

async fn start_anthropic_test_server() -> ProviderProtocolTestServer {
    let captured = Arc::new(Mutex::new(CapturedProviderRequests::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/v1/messages",
            post({
                let captured = captured.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let captured = captured.clone();
                    async move {
                        let mut captured = captured.lock().unwrap();
                        captured.anthropic_auth = headers
                            .get("x-api-key")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        captured.anthropic_model = body
                            .get("model")
                            .and_then(|value| value.as_str())
                            .map(str::to_string);
                        let text = if body
                            .get("system")
                            .and_then(|value| value.as_str())
                            .is_some_and(|content| content.contains("You are the planner for rove"))
                            || body
                            .get("messages")
                            .and_then(|value| value.as_array())
                            .and_then(|messages| messages.first())
                            .and_then(|message| message.get("content"))
                            .and_then(|value| value.as_str())
                            .is_some_and(|content| content.contains("You are the planner for rove"))
                        {
                            r#"{"goal":"anthropic profile job","steps":[{"id":"1","title":"reply"}]}"#
                        } else {
                            "anthropic profile ok"
                        };
                        let chunk = serde_json::json!({
                            "type": "content_block_delta",
                            "index": 0,
                            "delta": {
                                "type": "text_delta",
                                "text": text
                            }
                        });
                        let message_stop = serde_json::json!({ "type": "message_stop" });
                        let body = format!(
                            "event: content_block_delta\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
                            chunk, message_stop
                        );
                        ([(CONTENT_TYPE, "text/event-stream")], body)
                    }
                }
            }),
        )
        .route(
            "/v1/models",
            get(|| async {
                Json(serde_json::json!({
                    "data": [
                        { "id": "claude-test" }
                    ]
                }))
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    ProviderProtocolTestServer {
        base_url: format!("http://{addr}"),
        captured,
    }
}

async fn start_ollama_test_server() -> ProviderProtocolTestServer {
    let captured = Arc::new(Mutex::new(CapturedProviderRequests::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/api/chat",
            post({
                let captured = captured.clone();
                move |Json(body): Json<serde_json::Value>| {
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().ollama_model = body
                            .get("model")
                            .and_then(|value| value.as_str())
                            .map(str::to_string);
                        let content = if body
                            .get("messages")
                            .and_then(|value| value.as_array())
                            .and_then(|messages| messages.first())
                            .and_then(|message| message.get("content"))
                            .and_then(|value| value.as_str())
                            .is_some_and(|content| content.contains("You are the planner for rove"))
                        {
                            r#"{"goal":"ollama profile job","steps":[{"id":"1","title":"reply"}]}"#
                        } else {
                            "ollama profile ok"
                        };
                        let chunk = serde_json::json!({
                            "message": {
                                "content": content
                            },
                            "done": false
                        });
                        let done = serde_json::json!({
                            "done": true,
                            "prompt_eval_count": 1,
                            "eval_count": 1
                        });
                        let body = format!("{chunk}\n{done}\n");
                        ([(CONTENT_TYPE, "application/x-ndjson")], body)
                    }
                }
            }),
        )
        .route(
            "/api/tags",
            get(|| async {
                Json(serde_json::json!({
                    "models": [
                        { "name": "llama-test" }
                    ]
                }))
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    ProviderProtocolTestServer {
        base_url: format!("http://{addr}"),
        captured,
    }
}

fn unique_env_key(prefix: &str) -> String {
    format!(
        "{}_{}",
        prefix,
        ulid::Ulid::new().to_string().replace('-', "_")
    )
}

fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    // Default profiles-only config already uses a fake provider.
    config.provider.model = "fake".to_string();
    config.runtime.max_steps = 4;
    config.source_summary.user_config_path = workspace_path("target/test-provider-catalogs")
        .join(ulid::Ulid::new().to_string())
        .join("config.toml");
    config
}

fn write_api_agent_definition(root: &Path) {
    std::fs::create_dir_all(root.join("agents/ops/procedures")).unwrap();
    std::fs::write(
        root.join("agents/ops/agent.toml"),
        r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Operations"
default_instructions_path = "instructions.md"

[capability_policy]
allow = ["workspace.fs.read"]

[procedure_policy]
max_selected = 1
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("agents/ops/instructions.md"),
        "Inspect before changing anything.",
    )
    .unwrap();
    std::fs::write(
        root.join("agents/ops/procedures/rollback.md"),
        "---\nschema_version: 1\nid: ops.rollback\nversion: 1.0.0\nstatus: active\ntitle: Roll back\nmode: diagnose\nrisk_level: low\nintents: [rollback]\n---\n\nInspect the deployment before rollback.\n",
    )
    .unwrap();
}

// ─── P5b: isolated executable HTML preview ─────────────────────────────────
//
// The preview surface runs on a dedicated loopback origin with its own
// per-session token, no product credentials, and the same path/secret/size
// discipline as the read-only file API. Threat model gates 2–5 are exercised
// here; gate 1's real-browser half lives in
// `apps/web/tests/e2e/workbench-preview-origin.spec.ts`.

struct PreviewServer {
    app: axum::Router,
    preview_addr: std::net::SocketAddr,
    _state: ApiState,
    _server_dir: tempfile::TempDir,
}

async fn spawn_preview_server(config: AppConfig) -> PreviewServer {
    let server_dir = tempfile::TempDir::new().unwrap();
    let state = ApiState::new(Workspace::detect(server_dir.path()).unwrap(), config);
    rove_api::spawn_preview_listener(&state).await;
    let preview_addr = state
        .preview_origin_addr()
        .await
        .expect("preview listener should bind on an ephemeral loopback port");
    let app = router(state.clone());
    PreviewServer {
        app,
        preview_addr,
        _state: state,
        _server_dir: server_dir,
    }
}

fn preview_site() -> tempfile::TempDir {
    let site = tempfile::TempDir::new().unwrap();
    std::fs::write(
        site.path().join("index.html"),
        "<!doctype html><html><head><link rel=\"stylesheet\" href=\"style.css\"></head>\
         <body><h1 id=\"marker\">rove-preview-marker</h1><script src=\"app.js\"></script></body></html>",
    )
    .unwrap();
    std::fs::write(
        site.path().join("style.css"),
        "h1 { color: rebeccapurple; }",
    )
    .unwrap();
    std::fs::write(site.path().join("app.js"), "console.log('preview');").unwrap();
    std::fs::write(site.path().join("notes.txt"), "plain text").unwrap();
    std::fs::write(site.path().join(".env"), "SECRET=hunter2").unwrap();
    std::fs::write(
        site.path().join("big.bin"),
        vec![b'x'; (8 * 1024 * 1024) + 1],
    )
    .unwrap();
    site
}

async fn create_preview(app: &axum::Router, workspace_id: &str, path: &str) -> serde_json::Value {
    let response = post_json(
        app,
        &format!("/product/workspaces/{workspace_id}/previews"),
        serde_json::json!({ "path": path }),
    )
    .await;
    let status = response.status();
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

fn preview_no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn assert_preview_security_headers(response: &reqwest::Response) {
    let headers = response.headers();
    let csp = headers
        .get("content-security-policy")
        .expect("preview responses must carry CSP")
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(csp.contains("connect-src 'none'"), "{csp}");
    assert!(csp.contains("form-action 'none'"), "{csp}");
    assert_eq!(
        headers.get("cross-origin-opener-policy").unwrap(),
        "same-origin"
    );
    assert_eq!(
        headers.get("cross-origin-resource-policy").unwrap(),
        "same-origin"
    );
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    assert!(
        headers.get("set-cookie").is_none(),
        "the preview origin must never set cookies"
    );
}

#[tokio::test]
async fn product_preview_serves_html_on_an_isolated_loopback_origin() {
    let site = preview_site();
    let server = spawn_preview_server(test_config()).await;
    let workspace = create_product_workspace(&server.app, site.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();

    let created = create_preview(&server.app, &workspace_id, "index.html").await;
    assert_eq!(created["workspace_id"], workspace_id);
    assert_eq!(created["entry"], "index.html");
    assert!(created["preview_id"].as_str().unwrap().len() >= 26);
    assert!(created["expires_at"].as_str().unwrap() > created["created_at"].as_str().unwrap());

    let url = created["url"].as_str().unwrap().to_string();
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(
        parsed.port().unwrap(),
        server.preview_addr.port(),
        "the preview URL must point at the isolated preview origin"
    );
    let token = parsed.path_segments().unwrap().next().unwrap().to_string();
    assert_eq!(token.len(), 52, "two ULIDs give a 160-bit token");

    let client = preview_no_redirect_client();

    // The bare session root redirects to the entry, preserving relative
    // resource resolution for the previewed page.
    let bare = client
        .get(format!("http://{}/{token}", server.preview_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        bare.headers().get("location").unwrap(),
        &format!("/{token}/index.html")
    );
    assert_preview_security_headers(&bare);

    // The entry page and its relative resources load with isolation headers.
    let entry = client.get(&url).send().await.unwrap();
    assert_eq!(entry.status(), StatusCode::OK);
    assert_preview_security_headers(&entry);
    assert!(
        entry
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let body = entry.text().await.unwrap();
    assert!(body.contains("rove-preview-marker"));

    for (resource, expected_mime) in [
        ("style.css", "text/css"),
        ("app.js", "text/javascript"),
        ("notes.txt", "text/plain"),
    ] {
        let response = client
            .get(format!("http://{}/{token}/{resource}", server.preview_addr))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{resource}");
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(expected_mime),
            "{resource}"
        );
    }

    // A4: secret-shaped files are refused even when they exist on disk.
    let secret = client
        .get(format!("http://{}/{token}/.env", server.preview_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(secret.status(), StatusCode::BAD_REQUEST);
    assert_preview_security_headers(&secret);

    // A10: one resource may not exceed the size cap.
    let oversized = client
        .get(format!("http://{}/{token}/big.bin", server.preview_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // A7: an unknown token resolves nothing.
    let unknown = client
        .get(format!(
            "http://{}/{}/index.html",
            server.preview_addr,
            "A".repeat(52)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_preview_security_headers(&unknown);
}

#[tokio::test]
async fn product_preview_rejects_raw_traversal_and_revokes_on_close() {
    let site = preview_site();
    let server = spawn_preview_server(test_config()).await;
    let workspace = create_product_workspace(&server.app, site.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let created = create_preview(&server.app, &workspace_id, "index.html").await;
    let url = created["url"].as_str().unwrap().to_string();
    let preview_id = created["preview_id"].as_str().unwrap().to_string();
    let token = reqwest::Url::parse(&url)
        .unwrap()
        .path_segments()
        .unwrap()
        .next()
        .unwrap()
        .to_string();

    // A3/A6: raw `..` traversal that a spec-compliant URL client would
    // normalize away must still be rejected when sent verbatim.
    let mut stream = tokio::net::TcpStream::connect(server.preview_addr)
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(
        &mut stream,
        format!(
            "GET /{token}/../Cargo.toml HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            server.preview_addr
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut raw = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw)
        .await
        .unwrap();
    let raw = String::from_utf8_lossy(&raw);
    assert!(
        raw.starts_with("HTTP/1.1 400"),
        "traversal must be rejected, got: {}",
        &raw[..raw.len().min(200)]
    );

    // A8: closing the session revokes the token immediately.
    let closed = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/workspaces/{workspace_id}/previews/{preview_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::NO_CONTENT);

    let client = preview_no_redirect_client();
    let after_close = client.get(&url).send().await.unwrap();
    assert_eq!(after_close.status(), StatusCode::NOT_FOUND);
    assert_preview_security_headers(&after_close);

    let closed_again = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/product/workspaces/{workspace_id}/previews/{preview_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(closed_again.status(), StatusCode::NOT_FOUND);
    let closed_again: serde_json::Value = decode_json(closed_again).await;
    assert_eq!(closed_again["code"], "product_preview_not_found");
}

#[tokio::test]
async fn product_preview_validates_the_entry_and_bounds_session_count() {
    let site = preview_site();
    let server = spawn_preview_server(test_config()).await;
    let workspace = create_product_workspace(&server.app, site.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let previews_uri = format!("/product/workspaces/{workspace_id}/previews");

    // Traversal, secret-shaped, missing, and non-HTML entries are refused.
    let traversal = post_json(
        &server.app,
        &previews_uri,
        serde_json::json!({ "path": "../outside.html" }),
    )
    .await;
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
    let traversal: serde_json::Value = decode_json(traversal).await;
    assert_eq!(traversal["code"], "product_preview_invalid_input");

    let secret_entry = post_json(
        &server.app,
        &previews_uri,
        serde_json::json!({ "path": ".env.html" }),
    )
    .await;
    assert_eq!(secret_entry.status(), StatusCode::BAD_REQUEST);

    let missing = post_json(
        &server.app,
        &previews_uri,
        serde_json::json!({ "path": "missing.html" }),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing: serde_json::Value = decode_json(missing).await;
    assert_eq!(missing["code"], "product_preview_not_found");

    let not_html = post_json(
        &server.app,
        &previews_uri,
        serde_json::json!({ "path": "notes.txt" }),
    )
    .await;
    assert_eq!(not_html.status(), StatusCode::BAD_REQUEST);
    let not_html: serde_json::Value = decode_json(not_html).await;
    assert_eq!(not_html["code"], "product_preview_invalid_input");

    // The session cap is a typed 429, not a silent eviction.
    for _ in 0..8 {
        create_preview(&server.app, &workspace_id, "index.html").await;
    }
    let ninth = post_json(
        &server.app,
        &previews_uri,
        serde_json::json!({ "path": "index.html" }),
    )
    .await;
    assert_eq!(ninth.status(), StatusCode::TOO_MANY_REQUESTS);
    let ninth: serde_json::Value = decode_json(ninth).await;
    assert_eq!(ninth["code"], "product_preview_limit");
}

#[tokio::test]
async fn product_preview_credentials_and_origins_stay_separate() {
    let site = preview_site();
    let mut config = test_config();
    config.api.token_auth = Some("secret-token".to_string());
    config.api.cors_origins = vec!["http://allowed.example".to_string()];
    let server = spawn_preview_server(config).await;

    // The product surface requires the product bearer token even to create a
    // preview session.
    let workspace_body = serde_json::json!({
        "root": site.path(),
        "kind": "folder",
        "display_name": "Preview auth workspace",
        "pinned": false
    });
    let unauthorized = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/workspaces")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(workspace_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/product/workspaces")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer secret-token")
                .body(Body::from(workspace_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::CREATED);
    let workspace: serde_json::Value = decode_json(authorized).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();

    // A browser page on the preview origin is not an allowed CORS origin for
    // the product API, so credentialed cross-origin calls fail closed.
    let preview_origin = format!("http://{}", server.preview_addr);
    let foreign_origin = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/product/workspaces/{workspace_id}/previews"))
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer secret-token")
                .header("origin", &preview_origin)
                .body(Body::from(r#"{"path":"index.html"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign_origin.status(), StatusCode::FORBIDDEN);

    let created = server
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/product/workspaces/{workspace_id}/previews"))
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer secret-token")
                .body(Body::from(r#"{"path":"index.html"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = decode_json(created).await;
    let url = created["url"].as_str().unwrap();

    // The preview origin never asks for or honours the product token.
    let client = preview_no_redirect_client();
    let without_credentials = client.get(url).send().await.unwrap();
    assert_eq!(without_credentials.status(), StatusCode::OK);
    let with_product_token = client
        .get(url)
        .header(AUTHORIZATION, "Bearer secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(with_product_token.status(), StatusCode::OK);
    assert_preview_security_headers(&with_product_token);
}

#[tokio::test]
async fn product_preview_reports_typed_unavailable_without_a_listener() {
    let site = preview_site();
    let server_dir = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server_dir.path()).unwrap(),
        test_config(),
    ));
    let workspace = create_product_workspace(&app, site.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let response = post_json(
        &app,
        &format!("/product/workspaces/{workspace_id}/previews"),
        serde_json::json!({ "path": "index.html" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["code"], "product_preview_unavailable");
}

#[tokio::test]
async fn product_preview_honours_project_trust_revocation() {
    let site = preview_site();
    std::fs::create_dir_all(site.path().join(".rove")).unwrap();
    std::fs::write(site.path().join(".rove/mcp_servers.json"), "[]").unwrap();
    let mut config = test_config();
    config.state.state_dir = PathBuf::from("api-state");
    let server = spawn_preview_server(config).await;
    let workspace = create_product_workspace(&server.app, site.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();

    let revoked = request_json(
        &server.app,
        "PUT",
        &format!("/product/workspaces/{workspace_id}/trust"),
        serde_json::json!({"decision": "revoke", "capabilities": []}),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::OK);

    let blocked = post_json(
        &server.app,
        &format!("/product/workspaces/{workspace_id}/previews"),
        serde_json::json!({ "path": "index.html" }),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    let blocked: serde_json::Value = decode_json(blocked).await;
    assert_eq!(blocked["code"], "project_trust_required");
}

// ─── P4: durable authorization request + decision history ───────────────────

#[tokio::test]
async fn product_session_authorizations_project_request_and_decision_side() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Authorization history").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    configure_product_session_model(&app, &session_id, "fake-raw", 1).await;

    let created = create_product_job(
        &app,
        &session_id,
        r#"{"tool":"write_file","args":{"path":"auth-approved.txt","content":"ok"}}"#,
    )
    .await;
    let pending = wait_for_approval_event(app.clone(), created.job_id.to_string()).await;
    let approval = pending.pending_approvals.first().unwrap().clone();

    let while_pending = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/authorizations"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(while_pending.status(), StatusCode::OK);
    let while_pending: serde_json::Value = decode_json(while_pending).await;
    assert_eq!(while_pending["session_id"], session_id);
    assert_eq!(while_pending["truncated"], false);
    let pending_rows = while_pending["authorizations"].as_array().unwrap();
    assert_eq!(pending_rows.len(), 1);
    assert_eq!(pending_rows[0]["call_id"], approval.call_id.to_string());
    assert_eq!(pending_rows[0]["status"], "pending");
    assert_eq!(pending_rows[0]["decided_via"], serde_json::Value::Null);

    let approve = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/jobs/{}/approvals/{}",
                    created.job_id, approval.call_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approve.status(), StatusCode::OK);
    wait_for_done(app.clone(), created.job_id.to_string()).await;

    let decided = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/product/sessions/{session_id}/authorizations?limit=10"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(decided.status(), StatusCode::OK);
    let decided: serde_json::Value = decode_json(decided).await;
    let rows = decided["authorizations"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "approved");
    assert_eq!(rows[0]["decided_via"], "job_api");
    assert_eq!(rows[0]["tool"], "write_file");
    assert!(!rows[0]["requested_at"].as_str().unwrap().is_empty());
    assert!(!rows[0]["updated_at"].as_str().unwrap().is_empty());
    // The outcome is projected only when the run's event log holds a terminal
    // tool event for the same call id.
    if !rows[0]["outcome"].is_null() {
        assert_eq!(rows[0]["outcome"]["event"], "tool_call_completed");
    }
}

#[tokio::test]
async fn product_session_authorizations_reject_unknown_session_and_bound_limit() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap();
    let session = create_product_session(&app, workspace_id, "Empty authorizations").await;
    let session_id = session["id"].as_str().unwrap();

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV/authorizations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let empty = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/sessions/{session_id}/authorizations"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    let empty: serde_json::Value = decode_json(empty).await;
    assert_eq!(empty["authorizations"].as_array().unwrap().len(), 0);
    assert_eq!(empty["truncated"], false);
}

/// One parsed product event frame: `id:` cursor, `event:` name, `data:` body.
struct ProductEventFrame {
    seq: i64,
    event: String,
    data: serde_json::Value,
}

fn parse_product_event_frame(raw: &str) -> ProductEventFrame {
    let mut seq = None;
    let mut event = None;
    let mut data = None;
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("id:") {
            seq = value.trim().parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data = serde_json::from_str(value.trim()).ok();
        }
    }
    ProductEventFrame {
        seq: seq.unwrap_or_else(|| panic!("frame without an id: {raw}")),
        event: event.unwrap_or_else(|| panic!("frame without an event: {raw}")),
        data: data.unwrap_or_else(|| panic!("frame without a JSON data body: {raw}")),
    }
}

/// Read `wanted` frames from the long-lived product event stream.
///
/// The stream never closes on its own, so every read is bounded by a deadline:
/// a stalled stream has to fail the test rather than hang it.
async fn read_product_event_frames(
    body: &mut axum::body::BodyDataStream,
    wanted: usize,
) -> Vec<ProductEventFrame> {
    use futures::StreamExt;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut buffer = String::new();
    let mut frames: Vec<ProductEventFrame> = Vec::new();
    while frames.len() < wanted {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let chunk = tokio::time::timeout(remaining, body.next())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "product event stream delivered {} of {wanted} frames before the deadline",
                    frames.len()
                )
            })
            .expect("the product event stream must stay open")
            .expect("the product event stream must not fail");
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = buffer.find("\n\n") {
            let raw = buffer[..end].to_string();
            buffer.drain(..end + 2);
            // A keep-alive is a comment frame, not an event. Skipping it keeps a
            // stalled stream reporting the deadline it missed instead of failing
            // while parsing the keep-alive as a fact.
            if !raw.trim().is_empty() && !raw.trim_start().starts_with(':') {
                frames.push(parse_product_event_frame(&raw));
            }
        }
    }
    frames
}

#[tokio::test]
async fn product_events_stream_delivers_catalog_facts_and_resumes_without_gaps() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    // Subscribe with no cursor first: this is the fresh-tab case, and the stream
    // must follow from now rather than answer with a replay window.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut body = response.into_body().into_data_stream();

    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(
        &app,
        &workspace_id,
        "Directory stream session with user text",
    )
    .await;
    let session_id = session["id"].as_str().unwrap().to_string();

    let frames = read_product_event_frames(&mut body, 2).await;
    assert_eq!(frames[0].event, "workspace.created");
    assert_eq!(frames[1].event, "session.created");
    assert!(
        frames[0].seq < frames[1].seq,
        "the stream must deliver facts in durable order"
    );
    for frame in &frames {
        assert_eq!(frame.data["v"], 1, "frames carry the protocol version");
        assert_eq!(frame.data["type"], frame.event);
        assert_eq!(frame.data["seq"].as_i64(), Some(frame.seq));
    }
    assert_eq!(
        frames[0].data["workspace_id"].as_str(),
        Some(workspace_id.as_str())
    );
    assert_eq!(
        frames[1].data["session_id"].as_str(),
        Some(session_id.as_str())
    );
    assert!(
        frames[1].data["workspace_id"].as_str() == Some(workspace_id.as_str()),
        "a created session reports the workspace it belongs to"
    );
    // The directory stream is readable by every API-token holder, so no user
    // text may reach it — not the session title either.
    let raw = serde_json::to_string(
        &frames
            .iter()
            .map(|frame| frame.data.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        !raw.contains("user text"),
        "user text must never enter the directory stream: {raw}"
    );

    let resume_from = frames.last().unwrap().seq;

    // Disconnect the client, then keep mutating: those facts must be waiting.
    drop(body);
    let renamed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/product/sessions/{session_id}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "title": "Renamed while disconnected" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    let queued = post_json(
        &app,
        &format!("/product/sessions/{session_id}/messages"),
        serde_json::json!({ "content": "queued after the disconnect" }),
    )
    .await;
    assert_eq!(queued.status(), StatusCode::CREATED);

    let resumed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events")
                .header("last-event-id", resume_from.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);
    let mut resumed_body = resumed.into_body().into_data_stream();
    let replay = read_product_event_frames(&mut resumed_body, 2).await;
    assert_eq!(
        replay.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        vec![resume_from + 1, resume_from + 2],
        "resuming must deliver every missed fact exactly once"
    );
    assert_eq!(replay[0].event, "session.updated");
    assert_eq!(replay[1].event, "control.queued");
    let summary = replay[1].data["summary"].as_str().expect("queued summary");
    assert!(summary.contains("\"status\":\"queued\""));
    assert!(
        !summary.contains("queued after the disconnect"),
        "a queue event carries state, never the message body"
    );
    drop(resumed_body);

    // An explicit cursor that predates the retained window cannot be served
    // silently, so it must fail closed with a typed conflict.
    let seeded =
        rusqlite::Connection::open(server.path().join("api-state/product.sqlite")).unwrap();
    let transaction = seeded.unchecked_transaction().unwrap();
    for offset in 0..(MAX_PRODUCT_EVENTS_RETAINED + 8) {
        transaction
            .execute(
                "INSERT INTO product_events(kind, created_at) VALUES ('session.updated', ?1)",
                [format!("seeded-{offset}")],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(seeded);
    // One ordinary mutation is what trims the log back to its window.
    let created = create_product_session(&app, &workspace_id, "Trim trigger").await;
    assert!(created["id"].is_string());

    let expired = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events?after=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(expired).await;
    assert_eq!(error["code"], "product_events_expired");

    // A client with no cursor is not resuming anything, so it still connects and
    // follows live even though the old window is gone.
    let followed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(followed.status(), StatusCode::OK);
    let mut followed_body = followed.into_body().into_data_stream();
    let created = create_product_session(&app, &workspace_id, "After the trim").await;
    let session_id = created["id"].as_str().unwrap().to_string();
    let live = read_product_event_frames(&mut followed_body, 1).await;
    assert_eq!(live[0].event, "session.created");
    assert_eq!(
        live[0].data["session_id"].as_str(),
        Some(session_id.as_str())
    );

    // A negative cursor is invalid input, not an expired one.
    let negative = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events?after=-5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(negative.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value = decode_json(negative).await;
    assert_eq!(error["code"], "product_invalid_input");

    // An unparsable query cursor is invalid input like any other: the endpoint
    // owns the typing, so it answers with the same JSON error contract instead of
    // axum's default plain-text rejection.
    let malformed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events?after=abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    let content_type = malformed
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.contains("application/json"),
        "a rejected cursor must be the typed JSON error, not a plain-text default: {content_type}"
    );
    let error: serde_json::Value = decode_json(malformed).await;
    assert_eq!(error["code"], "product_invalid_input");

    // A cursor ahead of every retained fact can never be served: every later
    // append lands behind it, so following from there would wait forever. That
    // is what a reset or replaced log under a cursor-holding client looks like,
    // and it must be reported as the same typed conflict as an evicted cursor.
    let ahead = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/events?after={}", i64::MAX))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ahead.status(), StatusCode::CONFLICT);
    let error: serde_json::Value = decode_json(ahead).await;
    assert_eq!(error["code"], "product_events_expired");

    // The header is the other cursor source, so it answers the same way.
    let unparsable = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events")
                .header("last-event-id", "not-a-seq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unparsable.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value = decode_json(unparsable).await;
    assert_eq!(error["code"], "product_invalid_input");

    // The stream ends with the server, so an open connection cannot outlive it.
    let shutdown = CancellationToken::new();
    let shutdown_app = router(ApiState::with_shutdown(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
        shutdown.clone(),
    ));
    let response = shutdown_app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    shutdown.cancel();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        use futures::StreamExt;
        while let Some(chunk) = body.next().await {
            chunk.expect("the shutting-down stream must not fail");
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "a cancelled server must close the product event stream"
    );
}

/// A newer build can share the same SQLite file and write kinds this build does
/// not know. Skipping such a row is right; waiting in front of it is not, because
/// the cursor only advances on an emitted frame. With a full page of undecodable
/// rows between the cursor and the next decodable fact, a stream that skipped
/// rows without moving its cursor would re-read the same page forever and never
/// deliver anything again.
#[tokio::test]
async fn product_events_stream_skips_undecodable_rows_instead_of_stalling() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));

    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let created = create_product_session(&app, &workspace_id, "Before the future kinds").await;
    assert!(created["id"].is_string());

    // Seed exactly one read window of rows only a newer build could have written.
    let database = server.path().join("api-state/product.sqlite");
    let head: i64 = {
        let seeded = rusqlite::Connection::open(&database).unwrap();
        let head: i64 = seeded
            .query_row("SELECT MAX(id) FROM product_events", [], |row| row.get(0))
            .unwrap();
        let transaction = seeded.unchecked_transaction().unwrap();
        for offset in 0..=MAX_PRODUCT_EVENT_PAGE {
            transaction
                .execute(
                    "INSERT INTO product_events(kind, created_at) VALUES ('future.unknown_kind', ?1)",
                    [format!("future-{offset}")],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        head
    };

    // The resuming client's cursor sits immediately before that page.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/product/events?after={head}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();

    // One ordinary mutation appends the next decodable fact, one row past the
    // undecodable window the stream has to step over.
    let created = create_product_session(&app, &workspace_id, "After the future kinds").await;
    let session_id = created["id"].as_str().unwrap().to_string();

    let frames = read_product_event_frames(&mut body, 1).await;
    assert_eq!(
        frames[0].event, "session.created",
        "the fact behind the undecodable rows must still be delivered"
    );
    assert_eq!(
        frames[0].data["session_id"].as_str(),
        Some(session_id.as_str())
    );
    assert_eq!(
        frames[0].seq,
        head + MAX_PRODUCT_EVENT_PAGE as i64 + 2,
        "the undecodable rows cost skipped sequence numbers, not a stalled cursor"
    );
}

// --- Product session attachments (PR-1: store, upload, and serve) ---

/// The 24-byte PNG header the raster validator accepts: an 8-byte signature, an
/// `IHDR` chunk, and a 2x3 pixel declaration.
fn attachment_png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0, 0, 0, 13]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&2u32.to_be_bytes());
    bytes.extend_from_slice(&3u32.to_be_bytes());
    bytes
}

fn attachment_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF\n".to_vec()
}

/// Upload one raw attachment body.
async fn post_attachment(
    app: &Router,
    uri: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> axum::response::Response {
    let mut request = Request::builder().method("POST").uri(uri);
    if let Some(content_type) = content_type {
        request = request.header(CONTENT_TYPE, content_type);
    }
    app.clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn get_attachment(app: &Router, uri: &str, range: Option<&str>) -> axum::response::Response {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(range) = range {
        request = request.header("range", range);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Find one file by name under a root, whatever layout the API resolved.
///
/// The test asserts *where* the payload is relative to the derived root, so it
/// must find it without knowing which state dir the config resolved to.
fn find_file_named(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|candidate| candidate == name) {
            return Some(path);
        }
    }
    None
}

/// Find one directory by name under a root. The session's attachment directory
/// is named after the session, whatever state dir the config resolved to.
fn find_dir_named(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|candidate| candidate == name) {
            return Some(path);
        }
        if let Some(found) = find_dir_named(&path, name) {
            return Some(found);
        }
    }
    None
}

async fn attachment_error(response: axum::response::Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let body: serde_json::Value = decode_json(response).await;
    assert_eq!(body["code"], code, "typed error code");
}

async fn create_attachment_session(server: &Path) -> (Router, tempfile::TempDir, String, String) {
    let folder = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(Workspace::detect(server).unwrap(), config));
    let workspace = create_product_workspace(&app, folder.path()).await;
    let workspace_id = workspace["id"].as_str().unwrap().to_string();
    let session = create_product_session(&app, &workspace_id, "Attachments").await;
    let session_id = session["id"].as_str().unwrap().to_string();
    // The caller keeps the folder alive: it is the workspace root, and the
    // payloads must be provably outside it.
    (app, folder, workspace_id, session_id)
}
#[tokio::test]
async fn product_attachment_upload_validates_before_it_stores_anything() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments");
    let png = attachment_png_bytes();

    // An unknown session is refused before any path is derived.
    attachment_error(
        post_attachment(
            &app,
            "/product/sessions/01J0000000000000000000000Z/attachments?name=shot.png",
            None,
            png.clone(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "product_not_found",
    )
    .await;

    // Every remaining failure is a typed 400 for invalid input. Each case is
    // named by its position in this list, in the order the checks run.
    for (_case, target, body) in [
        ("no name", uri.clone(), png.clone()),
        (
            "unknown extension",
            format!("{uri}?name=script.sh"),
            png.clone(),
        ),
        (
            "path separator in the name",
            format!("{uri}?name=..%2Fshot.png"),
            png.clone(),
        ),
        (
            "parent-directory name",
            format!("{uri}?name=.."),
            png.clone(),
        ),
        (
            "control character in the name",
            format!("{uri}?name=shot%0A.png"),
            png.clone(),
        ),
        (
            "extension and signature disagree",
            format!("{uri}?name=shot.png"),
            b"GIF89a\x02\x00\x03\x00".to_vec(),
        ),
        (
            "archive signature behind a document name",
            format!("{uri}?name=report.pdf"),
            b"PK\x03\x04\x14\x00\x00\x00".to_vec(),
        ),
        (
            "executable signature behind a text name",
            format!("{uri}?name=notes.txt"),
            b"\0asm\x01\x00\x00\x00".to_vec(),
        ),
        (
            "text that is not UTF-8",
            format!("{uri}?name=notes.md"),
            vec![0xff, 0xfe],
        ),
        ("empty body", format!("{uri}?name=notes.txt"), Vec::new()),
    ] {
        attachment_error(
            post_attachment(&app, &target, None, body).await,
            StatusCode::BAD_REQUEST,
            "product_attachment_invalid_input",
        )
        .await;
    }

    // A raster over 16 MiB is a 413 even though the body is inside the route
    // limit, and a body over 20 MiB is refused by the route limit itself.
    let mut oversized_raster = png.clone();
    oversized_raster.resize(16 * 1_048_576 + 1, 0);
    attachment_error(
        post_attachment(
            &app,
            &format!("{uri}?name=shot.png"),
            None,
            oversized_raster,
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "product_attachment_too_large",
    )
    .await;

    let mut oversized_document = attachment_pdf_bytes();
    oversized_document.resize(20 * 1_048_576 + 1, b' ');
    attachment_error(
        post_attachment(
            &app,
            &format!("{uri}?name=report.pdf"),
            None,
            oversized_document,
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "product_attachment_too_large",
    )
    .await;

    // Nothing above may have written a payload, and nothing may have landed in
    // the workspace either.
    let attachments = server.path().join("attachments");
    assert!(
        !attachments.exists()
            || std::fs::read_dir(attachments.join(&session_id))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true),
        "a refused upload must not leave a payload"
    );
    assert!(
        std::fs::read_dir(folder.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(&session_id)),
        "the workspace root must never receive an attachment"
    );
}

/// A body with no declared length is bounded by the read itself rather than by
/// `Content-Length`.
#[tokio::test]
async fn product_attachment_chunked_body_without_a_length_is_bounded() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments?name=notes.txt");

    // 21 MiB in 1 MiB chunks: the stream has no known total, so a refusal that
    // came from comparing a declared length could not be reached at all.
    let chunks = (0..21).map(|_| {
        Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(vec![b'a'; 1_048_576]))
    });
    let request = Request::builder()
        .method("POST")
        .uri(&uri)
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    assert!(
        request
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .is_none(),
        "the case under test is the one with no declared length"
    );

    attachment_error(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "product_attachment_too_large",
    )
    .await;

    // The refusal happened while reading, before any path was derived.
    assert!(
        find_dir_named(server.path(), &session_id).is_none(),
        "a body refused during the read must not create the session tree"
    );
}

/// The upload slots are bounded process-wide, and a slot is refunded when the
/// request holding it goes away.
#[tokio::test]
async fn product_attachment_upload_slots_are_bounded_and_refunded() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments?name=notes.txt");

    // Four uploads whose bodies never complete hold every slot. They are
    // spawned rather than awaited because the point is that they stay pending.
    let mut held = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        let uri = uri.clone();
        held.push(tokio::spawn(async move {
            let request = Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >()))
                .unwrap();
            app.oneshot(request).await
        }));
    }
    // Let the four reach the guard and take their permits: the test runtime is
    // current-thread, so awaiting here is what polls them.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for task in &held {
        assert!(
            !task.is_finished(),
            "a stalled upload must still be holding its slot"
        );
    }

    // The fifth is refused with the attachment-specific code, not with a
    // generic rate-limit answer.
    attachment_error(
        post_attachment(&app, &uri, None, b"fifth".to_vec()).await,
        StatusCode::TOO_MANY_REQUESTS,
        "product_attachment_busy",
    )
    .await;

    // Dropping the stalled requests refunds their permits.
    for task in held {
        task.abort();
        let _ = task.await;
    }
    let response = post_attachment(&app, &uri, None, b"after the slots return".to_vec()).await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a refunded slot must be usable again"
    );
}

#[tokio::test]
async fn product_attachment_upload_and_download_pin_the_headers_and_the_pair() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, folder, workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments");
    let png = attachment_png_bytes();

    // A client `Content-Type` claim of `application/octet-stream` is the raw
    // body default and must not become a mismatch warning.
    let uploaded = post_attachment(
        &app,
        &format!("{uri}?name=screenshot.png"),
        Some("application/octet-stream"),
        png.clone(),
    )
    .await;
    assert_eq!(uploaded.status(), StatusCode::CREATED);
    let uploaded: serde_json::Value = decode_json(uploaded).await;
    assert_eq!(uploaded["content_type"], "image/png");
    assert_eq!(uploaded["size"], png.len() as u64);
    assert_eq!(uploaded["status"], "staged");
    assert_eq!(uploaded["name"], "screenshot.png");
    assert_eq!(
        uploaded["sha256"], "db42d7b740a36256f694172427189b90e7d94a9abebab81435bf4bb3d7b9bf9d",
        "the digest is of the bytes this server hashed"
    );
    assert_eq!(uploaded["warnings"], serde_json::json!([]));
    assert!(uploaded["expires_at"].as_str().is_some());
    let attachment_id = uploaded["attachment_id"].as_str().unwrap().to_string();

    // The payload is under the data root's own `attachments/<session>/` tree,
    // never inside the workspace.
    let payload = find_file_named(server.path(), &attachment_id)
        .unwrap_or_else(|| panic!("payload {attachment_id} must exist under the data root"));
    assert!(
        payload.ends_with(
            Path::new("attachments")
                .join(&session_id)
                .join(&attachment_id)
        ),
        "unexpected payload layout: {}",
        payload.display()
    );
    assert!(!payload.starts_with(folder.path()));
    assert_eq!(std::fs::read(&payload).unwrap(), png);

    // A download sends the durable, locally verified type and the fixed header
    // set — never the client's claim.
    let download = get_attachment(&app, &format!("{uri}/{attachment_id}"), None).await;
    assert_eq!(download.status(), StatusCode::OK);
    let headers = download.headers().clone();
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "image/png"
    );
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(headers.get("cache-control").unwrap(), "private, no-store");
    assert_eq!(
        headers.get("content-security-policy").unwrap(),
        "default-src 'none'; sandbox"
    );
    assert_eq!(headers.get("accept-ranges").unwrap(), "bytes");
    assert_eq!(
        headers.get("content-disposition").unwrap(),
        "inline; filename=\"screenshot.png\"",
        "a raster is rendered in place"
    );
    let served = axum::body::to_bytes(download.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(served.to_vec(), png);

    // A text payload has no signature, so its type can only come from the row.
    let text = post_attachment(
        &app,
        &format!("{uri}?name=notes.txt"),
        Some("application/pdf"),
        b"attachment sha256 canary".to_vec(),
    )
    .await;
    let text: serde_json::Value = decode_json(text).await;
    // The claim disagrees with the verified type: warn, never store the claim.
    assert_eq!(
        text["warnings"],
        serde_json::json!(["content_type_claim_mismatch"])
    );
    assert_eq!(text["content_type"], "text/plain");
    assert_eq!(
        text["sha256"],
        "540946d4b46083cb5c6c41427f211f3ca641cbe647c79afd13ec97141708be5c"
    );
    let text_id = text["attachment_id"].as_str().unwrap().to_string();

    let text_download = get_attachment(&app, &format!("{uri}/{text_id}"), None).await;
    assert_eq!(text_download.status(), StatusCode::OK);
    assert_eq!(
        text_download
            .headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain",
        "a text attachment is served as what validation established, not as sniffed bytes"
    );
    assert_eq!(
        text_download.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"notes.txt\"",
        "text is a download, not something the browser may render in place"
    );

    // A satisfiable range is a 206 with the exact bytes, still typed.
    let ranged = get_attachment(&app, &format!("{uri}/{text_id}"), Some("bytes=11-15")).await;
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        ranged.headers().get("content-range").unwrap(),
        "bytes 11-15/24"
    );
    let ranged_bytes = axum::body::to_bytes(ranged.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&ranged_bytes[..], b"sha25");

    // An unsatisfiable range is a typed 400, not an empty 200.
    let unsatisfiable =
        get_attachment(&app, &format!("{uri}/{text_id}"), Some("bytes=9999-10000")).await;
    assert_eq!(unsatisfiable.status(), StatusCode::BAD_REQUEST);

    // The pair is the whole lookup: another session sees nothing, and an
    // unknown id is the same 404 with the same code.
    let other = create_product_session(&app, &workspace_id, "Other").await;
    let other_id = other["id"].as_str().unwrap();
    attachment_error(
        get_attachment(
            &app,
            &format!("/product/sessions/{other_id}/attachments/{attachment_id}"),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "product_attachment_not_found",
    )
    .await;
    attachment_error(
        get_attachment(&app, &format!("{uri}/01J0000000000000000000000Z"), None).await,
        StatusCode::NOT_FOUND,
        "product_attachment_not_found",
    )
    .await;
    attachment_error(
        get_attachment(
            &app,
            "/product/sessions/01J0000000000000000000000Z/attachments/01J0000000000000000000000Z",
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "product_attachment_not_found",
    )
    .await;

    // Removing the bytes behind an intact row is a 410: the reference was
    // valid, so it must not be retried as a new upload.
    std::fs::remove_file(&payload).unwrap();
    attachment_error(
        get_attachment(&app, &format!("{uri}/{attachment_id}"), None).await,
        StatusCode::GONE,
        "product_attachment_unavailable",
    )
    .await;

    // A row whose payload was replaced by different bytes is corrupt, not
    // silently served.
    let corrupt = find_file_named(server.path(), &text_id).unwrap();
    std::fs::write(&corrupt, b"attachment sha256 canarY").unwrap();
    attachment_error(
        get_attachment(&app, &format!("{uri}/{text_id}"), None).await,
        StatusCode::GONE,
        "product_attachment_unavailable",
    )
    .await;

    // An expired staged row is a 409 conflict, and its payload is no longer
    // reachable.
    let expired = post_attachment(
        &app,
        &format!("{uri}?name=stale.txt"),
        None,
        b"expires".to_vec(),
    )
    .await;
    let expired: serde_json::Value = decode_json(expired).await;
    let expired_id = expired["attachment_id"].as_str().unwrap().to_string();
    expire_attachment_row(server.path(), &expired_id);
    attachment_error(
        get_attachment(&app, &format!("{uri}/{expired_id}"), None).await,
        StatusCode::CONFLICT,
        "product_attachment_conflict",
    )
    .await;
}

/// Mark one attachment row expired the way the TTL sweep will.
fn expire_attachment_row(data_root: &Path, attachment_id: &str) {
    let database = find_file_named(data_root, "product.sqlite")
        .expect("the product store database must exist under the data root");
    let connection = rusqlite::Connection::open(database).unwrap();
    let updated = connection
        .execute(
            "UPDATE product_attachments SET status = 'expired' WHERE attachment_id = ?1",
            [attachment_id],
        )
        .unwrap();
    assert_eq!(updated, 1, "exactly one row must be expired");
}

#[tokio::test]
async fn product_attachment_uploads_are_quota_bounded_and_refunds_a_refusal() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments");

    // The staged count ceiling is 32 per session.
    for index in 0..32 {
        let response = post_attachment(
            &app,
            &format!("{uri}?name=note-{index}.txt"),
            None,
            format!("bounded {index}").into_bytes(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "upload {index}");
    }

    // The 33rd is refused, and the bytes it wrote before the refusal are
    // removed again: the row is the recovery point, so an orphan would be
    // unreachable.
    attachment_error(
        post_attachment(
            &app,
            &format!("{uri}?name=note-33.txt"),
            None,
            b"over quota".to_vec(),
        )
        .await,
        StatusCode::CONFLICT,
        "product_attachment_quota",
    )
    .await;

    let session_dir = find_dir_named(server.path(), &session_id)
        .expect("the session's own attachment directory must exist");
    let stored = std::fs::read_dir(&session_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".part"))
        .count();
    assert_eq!(
        stored, 32,
        "a quota refusal must leave exactly the admitted payloads"
    );
    assert!(
        std::fs::read_dir(&session_dir)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".part")),
        "no partial write may survive"
    );
}

#[tokio::test]
async fn product_attachment_secrets_warn_and_the_bytes_are_never_rewritten() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;
    let uri = format!("/product/sessions/{session_id}/attachments");

    // Synthetic shapes only; no fixture here is a real credential.
    let body = b"token=SYNTHETIC-CANARY-0001\n".to_vec();
    let uploaded =
        post_attachment(&app, &format!("{uri}?name=id_rsa.txt"), None, body.clone()).await;
    assert_eq!(uploaded.status(), StatusCode::CREATED);
    let uploaded: serde_json::Value = decode_json(uploaded).await;
    let mut warnings: Vec<String> = uploaded["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    warnings.sort();
    assert_eq!(
        warnings,
        vec![
            "possible_secret_content".to_string(),
            "secret_shaped_name".to_string(),
        ]
    );

    // A warning is not a rewrite: the stored bytes are the uploaded bytes.
    let attachment_id = uploaded["attachment_id"].as_str().unwrap();
    let served = axum::body::to_bytes(
        get_attachment(&app, &format!("{uri}/{attachment_id}"), None)
            .await
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(served.to_vec(), body);
}
#[tokio::test]
async fn product_attachment_payloads_survive_a_workspace_switch_and_a_state_migration() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, folder_one, _workspace_one, session_one) =
        create_attachment_session(server.path()).await;

    let first = post_attachment(
        &app,
        &format!("/product/sessions/{session_one}/attachments?name=one.txt"),
        None,
        b"first workspace".to_vec(),
    )
    .await;
    let first: serde_json::Value = decode_json(first).await;
    let first_id = first["attachment_id"].as_str().unwrap().to_string();
    let first_payload = find_file_named(server.path(), &first_id).unwrap();

    // A second workspace with its own session, over the same data root.
    let folder_two = tempfile::TempDir::new().unwrap();
    let workspace_two = create_product_workspace(&app, folder_two.path()).await;
    let workspace_two_id = workspace_two["id"].as_str().unwrap();
    let session_two = create_product_session(&app, workspace_two_id, "Second").await;
    let session_two_id = session_two["id"].as_str().unwrap().to_string();
    let second = post_attachment(
        &app,
        &format!("/product/sessions/{session_two_id}/attachments?name=two.txt"),
        None,
        b"second workspace".to_vec(),
    )
    .await;
    let second: serde_json::Value = decode_json(second).await;
    let second_id = second["attachment_id"].as_str().unwrap().to_string();
    let second_payload = find_file_named(server.path(), &second_id).unwrap();

    // Both payloads live under one attachment root, keyed only by their own
    // session: adding, renaming, or removing a workspace cannot relocate or
    // collide with them.
    let root_one = first_payload.parent().unwrap().parent().unwrap();
    let root_two = second_payload.parent().unwrap().parent().unwrap();
    assert_eq!(root_one, root_two);
    assert_eq!(root_one.file_name().unwrap(), "attachments");
    assert_eq!(
        first_payload.parent().unwrap().file_name().unwrap(),
        session_one.as_str()
    );
    assert_eq!(
        second_payload.parent().unwrap().file_name().unwrap(),
        session_two_id.as_str()
    );
    assert!(!first_payload.starts_with(folder_one.path()));
    assert!(!second_payload.starts_with(folder_two.path()));

    // `rove state migrate` classifies only `<workspace_root>/.rove`. The
    // attachment tree is under the data root and outside every workspace, so an
    // apply+prune run leaves it byte-identical.
    std::fs::create_dir_all(folder_one.path().join(".rove/memory")).unwrap();
    std::fs::write(
        folder_one.path().join(".rove/memory/MEMORY.md"),
        "# legacy\n",
    )
    .unwrap();
    let report = run_state_migration(&MigrationOptions {
        workspace_root: folder_one.path().to_path_buf(),
        data_root: Some(server.path().to_path_buf()),
        on_conflict: ConflictPolicy::KeepTarget,
        max_bytes: DEFAULT_MAX_MIGRATION_BYTES,
        prune_legacy: true,
        apply: true,
    })
    .expect("the migration must plan and apply against the workspace's legacy state");
    let report = serde_json::to_string(&report).unwrap();
    assert!(
        !report.contains("attachments"),
        "no attachment path may be classified by state migration: {report}"
    );
    assert_eq!(std::fs::read(&first_payload).unwrap(), b"first workspace");
    assert_eq!(std::fs::read(&second_payload).unwrap(), b"second workspace");
}

// --- Product message attachments (PR-2/PR-3: reference, serve, and cascade) ---

/// Every string anywhere inside a JSON document.
///
/// A path can be escaped differently depending on where it is embedded — a
/// Windows separator is `\\` inside a JSON string literal, `/` inside a POSIX
/// one — so a raw-substring search over the serialized document proves less
/// than it looks. Walking the decoded values removes that ambiguity.
fn json_strings(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => found.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                json_strings(item, found);
            }
        }
        serde_json::Value::Object(fields) => {
            for field in fields.values() {
                json_strings(field, found);
            }
        }
        _ => {}
    }
}

/// Upload one text attachment and return its server-issued id and verified type.
async fn upload_text_attachment(app: &Router, session_id: &str, name: &str, body: &[u8]) -> String {
    let response = post_attachment(
        app,
        &format!("/product/sessions/{session_id}/attachments?name={name}"),
        Some("text/plain"),
        body.to_vec(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let uploaded: serde_json::Value = decode_json(response).await;
    assert_eq!(uploaded["content_type"], "text/plain");
    assert_eq!(uploaded["size"].as_u64(), Some(body.len() as u64));
    assert_eq!(uploaded["status"], "staged");
    uploaded["attachment_id"].as_str().unwrap().to_string()
}

/// POST one message naming the given `(attachment id, display name)` pairs.
async fn send_message_with_attachments(
    app: &Router,
    session_id: &str,
    content: &str,
    key: &str,
    attachments: &[(String, String)],
) -> axum::response::Response {
    let attachments: Vec<serde_json::Value> = attachments
        .iter()
        .map(|(id, name)| serde_json::json!({ "attachment_id": id, "name": name }))
        .collect();
    post_json(
        app,
        &format!("/product/sessions/{session_id}/messages"),
        serde_json::json!({
            "content": content,
            "idempotency_key": key,
            "attachments": attachments,
        }),
    )
    .await
}

#[tokio::test]
async fn product_message_attachments_are_referenced_served_and_cascaded() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, workspace_id, session_id) = create_attachment_session(server.path()).await;
    configure_product_session_model(&app, &session_id, "fake", 1).await;
    let notes = upload_text_attachment(&app, &session_id, "notes.txt", b"the note body").await;
    let png = {
        let response = post_attachment(
            &app,
            &format!("/product/sessions/{session_id}/attachments?name=shot.png"),
            Some("image/png"),
            attachment_png_bytes(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let uploaded: serde_json::Value = decode_json(response).await;
        uploaded["attachment_id"].as_str().unwrap().to_string()
    };

    // An attachment-free message keeps the pre-attachment payload byte for byte:
    // the field is omitted, not sent as `[]`. It is checked in its own session so
    // this session runs exactly one turn and can be deleted deterministically.
    let legacy = create_product_session(&app, &workspace_id, "Legacy payload").await;
    let legacy_id = legacy["id"].as_str().unwrap().to_string();
    let plain = post_json(
        &app,
        &format!("/product/sessions/{legacy_id}/messages"),
        serde_json::json!({ "content": "no attachment here", "idempotency_key": "plain-1" }),
    )
    .await;
    assert_eq!(plain.status(), StatusCode::CREATED);
    let plain: serde_json::Value = decode_json(plain).await;
    assert!(
        plain.get("attachments").is_none(),
        "an old payload must serialise exactly as it did before the field existed: {plain}"
    );

    let response = send_message_with_attachments(
        &app,
        &session_id,
        "read these",
        "with-attachments-1",
        &[
            (notes.clone(), "notes.txt".to_string()),
            (png.clone(), "shot.png".to_string()),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let message: serde_json::Value = decode_json(response).await;
    let references = message["attachments"].as_array().unwrap();
    assert_eq!(references.len(), 2);
    // The projection reports server-verified metadata, and nothing that could be
    // used as a path.
    assert_eq!(references[0]["attachment_id"], notes.as_str());
    assert_eq!(references[0]["content_type"], "text/plain");
    assert_eq!(references[0]["size"].as_u64(), Some(13));
    assert_eq!(references[0]["availability"], "available");
    assert_eq!(references[0]["name"], "notes.txt");
    assert_eq!(references[1]["content_type"], "image/png");
    for reference in references {
        for forbidden in ["path", "root", "file", "directory", "url"] {
            assert!(
                reference.get(forbidden).is_none(),
                "no reference may carry a resolvable location: {reference}"
            );
        }
        assert_eq!(
            reference["sha256"].as_str().map(str::len),
            Some(64),
            "the digest the server computed is reported"
        );
    }

    // Reading the ledger reproduces the same attachment set, because it is
    // stored on the row rather than reconstructed from the request.
    let listed = list_product_messages(&app, &session_id).await;
    let stored = listed
        .iter()
        .find(|candidate| candidate["id"] == message["id"])
        .expect("the message must be in the ledger");
    assert_eq!(stored["attachments"], message["attachments"]);

    // Referencing promotes the row, so the payload is still servable.
    let download = get_attachment(
        &app,
        &format!("/product/sessions/{session_id}/attachments/{notes}"),
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    let body = axum::body::to_bytes(download.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"the note body");

    let payload_directory =
        find_dir_named(server.path(), &session_id).expect("the session's payload directory");
    assert!(payload_directory.is_dir());

    // Deleting the session cascades the rows and then the bytes. The session must
    // first be past its one turn: a delete while a turn is live is refused.
    let delivered =
        wait_for_delivered_message(&app, &session_id, message["id"].as_str().unwrap()).await;
    let run_id = delivered["successor_run_id"]
        .as_str()
        .expect("the queued message must start a run")
        .to_string();
    wait_for_successor_run_done(&app, &workspace_id, &session_id, &run_id).await;
    wait_for_product_session_status(&app, &workspace_id, &session_id, "idle").await;
    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/product/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(
        find_dir_named(server.path(), &session_id).is_none(),
        "a deleted session leaves no attachment payload directory behind"
    );
    let after_delete = get_attachment(
        &app,
        &format!("/product/sessions/{session_id}/attachments/{notes}"),
        None,
    )
    .await;
    assert_eq!(
        after_delete.status(),
        StatusCode::NOT_FOUND,
        "the payload is unreachable once its session and rows are gone"
    );
}

#[tokio::test]
async fn product_message_attachment_references_refuse_unknown_duplicate_foreign_and_unknown_fields()
{
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, workspace_id, session_id) = create_attachment_session(server.path()).await;
    let notes = upload_text_attachment(&app, &session_id, "notes.txt", b"body").await;
    let uri = format!("/product/sessions/{session_id}/messages");

    // An id this server never issued.
    let unknown = "01J8Z0M6Q3W9F2V7B4K1N5T8XZ";
    attachment_error(
        send_message_with_attachments(
            &app,
            &session_id,
            "unknown",
            "unknown-1",
            &[(unknown.to_string(), "ghost.txt".to_string())],
        )
        .await,
        StatusCode::NOT_FOUND,
        "product_attachment_not_found",
    )
    .await;

    // The same attachment twice: the shared message domain refuses a duplicate
    // before the store sees it, which is a typed `product_invalid_input` (400)
    // rather than the store's own `product_attachment_invalid_input`; either
    // way the send is refused and nothing is promoted.
    attachment_error(
        send_message_with_attachments(
            &app,
            &session_id,
            "duplicate",
            "duplicate-1",
            &[
                (notes.clone(), "notes.txt".to_string()),
                (notes.clone(), "notes.txt".to_string()),
            ],
        )
        .await,
        StatusCode::BAD_REQUEST,
        "product_invalid_input",
    )
    .await;

    // Another session's attachment is not addressable from this one.
    let other = create_product_session(&app, &workspace_id, "Foreign attachments").await;
    let other_id = other["id"].as_str().unwrap().to_string();
    let foreign = upload_text_attachment(&app, &other_id, "private.txt", b"not yours").await;
    attachment_error(
        send_message_with_attachments(
            &app,
            &session_id,
            "foreign",
            "foreign-1",
            &[(foreign.clone(), "private.txt".to_string())],
        )
        .await,
        StatusCode::NOT_FOUND,
        "product_attachment_not_found",
    )
    .await;

    // The unknown-field rule is not weakened for the new field: an attachment
    // entry cannot smuggle a client-claimed type through.
    let claimed = post_json(
        &app,
        &uri,
        serde_json::json!({
            "content": "client claims a type",
            "attachments": [{ "attachment_id": notes, "content_type": "text/plain" }],
        }),
    )
    .await;
    assert_eq!(claimed.status(), StatusCode::BAD_REQUEST);
    let claimed: serde_json::Value = decode_json(claimed).await;
    assert_eq!(claimed["code"], "product_invalid_input");

    // An unknown field on the message itself is refused the same way.
    let unknown_field = post_json(
        &app,
        &uri,
        serde_json::json!({ "content": "x", "max_steps": 4 }),
    )
    .await;
    assert_eq!(unknown_field.status(), StatusCode::BAD_REQUEST);
    let unknown_field: serde_json::Value = decode_json(unknown_field).await;
    assert_eq!(unknown_field["code"], "product_invalid_input");

    // Nothing above wrote a message: a refused send is a refused transaction,
    // not a partial row.
    let listed = list_product_messages(&app, &session_id).await;
    assert!(
        listed.is_empty(),
        "every refused send must leave the ledger untouched: {listed:?}"
    );
}

#[tokio::test]
async fn product_message_attachment_reference_ceiling_is_a_typed_refusal() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, _folder, _workspace_id, session_id) = create_attachment_session(server.path()).await;

    // Fill the referenced ceiling through the real path: 32 uploaded, then all
    // 32 referenced by one message.
    let mut first_batch = Vec::new();
    for index in 0..32 {
        let name = format!("n{index}.txt");
        first_batch.push((
            upload_text_attachment(&app, &session_id, &name, b"x").await,
            name,
        ));
    }
    let response =
        send_message_with_attachments(&app, &session_id, "all of them", "ceiling-1", &first_batch)
            .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "referencing exactly the ceiling is allowed"
    );

    // A 33rd reference crosses it. The refusal is typed and visible, and the
    // staged row it names keeps its status.
    let extra = upload_text_attachment(&app, &session_id, "extra.txt", b"y").await;
    attachment_error(
        send_message_with_attachments(
            &app,
            &session_id,
            "one too many",
            "ceiling-2",
            &[(extra.clone(), "extra.txt".to_string())],
        )
        .await,
        StatusCode::CONFLICT,
        "product_attachment_quota",
    )
    .await;
    let still_served = get_attachment(
        &app,
        &format!("/product/sessions/{session_id}/attachments/{extra}"),
        None,
    )
    .await;
    assert_eq!(
        still_served.status(),
        StatusCode::OK,
        "a quota refusal leaves the payload downloadable and the row staged"
    );
}

#[tokio::test]
async fn product_message_image_attachment_reaches_the_model_as_an_image_content_block() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, folder, workspace_id, session_id) = create_attachment_session(server.path()).await;
    configure_product_session_model(&app, &session_id, "fake", 1).await;

    // The store sniffs the type from the magic bytes and never trusts the
    // client's claim, so the body must genuinely open like a PNG.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&3u32.to_be_bytes());
    png.extend_from_slice(b"pixel-bytes");
    let upload = post_attachment(
        &app,
        &format!("/product/sessions/{session_id}/attachments?name=screenshot.png"),
        None,
        png,
    )
    .await;
    assert_eq!(upload.status(), StatusCode::CREATED);
    let uploaded: serde_json::Value = decode_json(upload).await;
    assert_eq!(uploaded["content_type"], "image/png");
    let shot = uploaded["attachment_id"].as_str().unwrap().to_string();

    let response = send_message_with_attachments(
        &app,
        &session_id,
        "look at this",
        "inject-image-1",
        &[(shot.clone(), "screenshot.png".to_string())],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let message: serde_json::Value = decode_json(response).await;
    let delivered =
        wait_for_delivered_message(&app, &session_id, message["id"].as_str().unwrap()).await;
    let run_id = delivered["successor_run_id"]
        .as_str()
        .expect("the queued message must start a run")
        .to_string();
    wait_for_successor_run_done(&app, &workspace_id, &session_id, &run_id).await;

    // The fake model echoes a deterministic back-reference for image content,
    // so this offline run proves an image block reached the client.
    let run_directory = folder.path().join("api-state").join("runs").join(&run_id);
    let task_state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_directory.join("task_state.json")).unwrap(),
    )
    .unwrap();
    let mut strings = Vec::new();
    json_strings(&task_state, &mut strings);
    assert!(
        strings
            .iter()
            .any(|text| text.contains("image blocks received: 1")),
        "the fake model must receive exactly one image block"
    );
    // The composed prompt labels the projected image and keeps the block open
    // and closed around it.
    assert!(
        strings
            .iter()
            .any(|text| text.contains("[image sent to the model as an image content block]")),
        "the prompt must label the projected image"
    );
    // The run summary is a derived artifact bounded at 120 chars and may cut a
    // marker mid-way; only the full prompts are held to block balance.
    for prompt in strings.iter().filter(|text| text.len() > 160) {
        if prompt.contains("<<<ROVE-ATTACHMENT") {
            let opens = prompt.matches("<<<ROVE-ATTACHMENT").count();
            let closes = prompt.matches("<<<END ROVE-ATTACHMENT").count();
            assert_eq!(opens, closes, "every opened block is closed: {prompt}");
        }
    }

    // The ledger (an API response) reports the reference metadata and never
    // the bytes; with an image-capable model nothing is reported degraded.
    let listed = serde_json::to_string(&list_product_messages(&app, &session_id).await).unwrap();
    assert!(listed.contains(&shot));
    assert!(!listed.contains("not sent:"));
    assert!(
        !listed.contains("pixel-bytes"),
        "the ledger never carries content"
    );
    let payload_directory = find_dir_named(server.path(), &session_id).unwrap();
    assert!(
        payload_directory.join(&shot).is_file(),
        "the payload is kept"
    );
}

#[tokio::test]
async fn product_message_attachment_reaches_the_model_as_labelled_text_and_never_as_a_path() {
    let server = tempfile::TempDir::new().unwrap();
    let (app, folder, workspace_id, session_id) = create_attachment_session(server.path()).await;
    configure_product_session_model(&app, &session_id, "fake", 1).await;

    let body = b"SYNTHETIC-NOTE-BODY-0001\nsecond line\n".to_vec();
    let notes = upload_text_attachment(&app, &session_id, "notes.txt", &body).await;
    let response = send_message_with_attachments(
        &app,
        &session_id,
        "summarise the note",
        "inject-1",
        &[(notes.clone(), "notes.txt".to_string())],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let message: serde_json::Value = decode_json(response).await;

    let delivered =
        wait_for_delivered_message(&app, &session_id, message["id"].as_str().unwrap()).await;
    let run_id = delivered["successor_run_id"]
        .as_str()
        .expect("the queued message must start a run")
        .to_string();
    wait_for_successor_run_done(&app, &workspace_id, &session_id, &run_id).await;

    // Run artifacts live under the *product workspace* root, while the payload
    // root is a sibling of the API-global product database.
    let run_directory = folder.path().join("api-state").join("runs").join(&run_id);
    let task_state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_directory.join("task_state.json")).unwrap(),
    )
    .unwrap();
    let mut strings = Vec::new();
    json_strings(&task_state, &mut strings);
    let carriers: Vec<&String> = strings
        .iter()
        .filter(|text| text.contains("SYNTHETIC-NOTE-BODY-0001"))
        .collect();
    assert!(
        !carriers.is_empty(),
        "the attachment's text must reach the model request"
    );

    // The message the user sent is still its own text, and the attachment is
    // appended as one labelled block. The context may carry the same message in
    // more than one place, so each place is checked for balance rather than the
    // whole document being checked for a count.
    for prompt in &carriers {
        let opens = prompt.matches("<<<ROVE-ATTACHMENT").count();
        let closes = prompt.matches("<<<END ROVE-ATTACHMENT").count();
        assert!(opens >= 1, "the block must be opened: {prompt}");
        assert_eq!(opens, closes, "every opened block is closed: {prompt}");
        assert!(prompt.contains("summarise the note"));
        assert!(prompt.contains(&format!("<<<ROVE-ATTACHMENT {notes}>>>")));
        assert!(prompt.contains(&format!("<<<END ROVE-ATTACHMENT {notes}>>>")));
        assert!(
            prompt.contains("notes.txt"),
            "the block is labelled with the name the client gave: {prompt}"
        );
    }

    // V10: the payload location never reaches the model, the trace, the report,
    // or an API response. The prompt is checked in every string of the durable
    // state, not in the raw document, so no escaping can hide a path.
    let payload_directory = find_dir_named(server.path(), &session_id)
        .expect("the session's payload directory must exist while it is referenced");
    let payload = payload_directory.join(&notes);
    assert!(payload.is_file(), "the referenced payload is kept");
    let forbidden = [
        payload.to_string_lossy().into_owned(),
        payload_directory.to_string_lossy().into_owned(),
        run_directory.to_string_lossy().into_owned(),
        format!("attachments/{session_id}"),
        format!("attachments\\{session_id}"),
    ];
    for text in &strings {
        for needle in &forbidden {
            assert!(
                !text.contains(needle.as_str()),
                "a model request must not carry {needle}: {text}"
            );
        }
    }
    for name in ["trace.jsonl", "report.json"] {
        let raw = std::fs::read_to_string(run_directory.join(name)).unwrap();
        for needle in &forbidden {
            assert!(
                !raw.contains(needle.as_str()),
                "{name} must not carry {needle}"
            );
        }
    }
    // The run's own durable record of its input does carry the composed user
    // message: `run_started.user_message` in `trace.jsonl`, the checkpoint in
    // `task_state.json`, and the derived `report.json` summary all record the
    // message the run was started with, exactly as they have always recorded
    // the user's typed text verbatim. Injected attachment text joins that
    // message, bounded by the inline caps, and the loop above holds every one of
    // those artifacts to the rule that actually changes here: no path, no
    // attachment root, and no run directory. The ledger (an API response) is
    // held to the stricter rule that it never carries content at all.
    let trace = std::fs::read_to_string(run_directory.join("trace.jsonl")).unwrap();
    assert!(
        trace.contains("SYNTHETIC-NOTE-BODY-0001"),
        "the run records the input it was started with"
    );
    let listed = list_product_messages(&app, &session_id).await;
    let listed = serde_json::to_string(&listed).unwrap();
    for needle in &forbidden {
        assert!(
            !listed.contains(needle.as_str()),
            "an API response must not carry {needle}"
        );
    }
    assert!(
        !listed.contains("SYNTHETIC-NOTE-BODY-0001"),
        "the ledger reports attachment metadata, never its content"
    );
}
