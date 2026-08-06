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
use rove_api::{
    ApiState, CreateJobResponse, JobStateResponse, MAX_M1_BROWSER_MIGRATION_BODY_BYTES,
    MAX_PRODUCT_TEXT_BYTES, ProductSessionId, ProductWorkspaceId, WorkspaceActivationState, router,
    serve_listener,
};
use rove_app_bootstrap::{AppConfig, AppConfigOverrides};
use rove_runtime::events::StreamEvent;
use rove_runtime::execution::StepRecordStatus;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{
    Message, Role, RunId, RunStatus, SessionId, TaskState, ToolCallRef, ToolMutation,
    ToolMutationOperation,
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
    ] {
        assert_eq!(
            runtime["execution_environment"]["capabilities"][capability],
            true
        );
    }
    assert_eq!(runtime["resume_health"]["status"], "healthy");
    assert_eq!(runtime["resume_health"]["workspace_count"], 1);
    assert_eq!(runtime["resume_health"]["session_count"], 1);
    assert_eq!(runtime["resume_health"]["bound_session_count"], 0);
    assert_eq!(runtime["resume_health"]["running_session_count"], 0);
    assert_eq!(runtime["resume_health"]["needs_attention_session_count"], 0);
    let keys = runtime.as_object().unwrap();
    assert_eq!(keys.len(), 5);
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
        ("/product/sessions/{session_id}/forks", "post"),
        ("/product/sessions/{session_id}/forks", "get"),
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
        ("/product/sessions/{session_id}/forks", "post"),
        ("/product/sessions/{session_id}/forks", "get"),
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
        "UpdateProductSessionModelConfigRequest",
        "UpdateProductPreferencesRequest",
    ] {
        assert!(schemas.contains_key(schema), "missing schema {schema}");
    }

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
    assert_eq!(
        sessions["sessions"][0]["runtime_binding"]["latest_run_id"],
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
    assert_eq!(
        get_product_session(&app, workspace_id, session_id).await["status"],
        "idle"
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
    assert_eq!(report["output"], "fake response: artifact api");
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
    for _ in 0..80 {
        let events = state_store.index.event_records(created.run_id).unwrap();
        if events
            .iter()
            .any(|event| event.event_name == "input_needed")
        {
            indexed_events = Some(events);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
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
    let trace_input_count = trace
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .filter(|event| matches!(event, StreamEvent::InputNeeded { .. }))
        .count();
    assert_eq!(trace_input_count, 1);

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
    assert_eq!(report["output"], "planned tool done");
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
    assert_eq!(std::fs::read_to_string(output_path).unwrap(), "ok");
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
    assert_eq!(created["env_names"], serde_json::json!(["PATH"]));
    assert!(created.get("env").is_none());

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

    let disabled = request_json(
        &app,
        "PUT",
        &format!("/product/mcp/servers/mock_server?workspace_id={workspace_a_id}"),
        serde_json::json!({
            "enabled": false,
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
    let mut config = AppConfig::load(server.path(), AppConfigOverrides::default()).unwrap();
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

async fn wait_for_product_control_status(
    app: &axum::Router,
    product_session_id: &str,
    control_id: &str,
    expected_status: &str,
) -> serde_json::Value {
    let mut last_control = None;
    for _ in 0..120 {
        let control = list_product_controls(app, product_session_id)
            .await
            .into_iter()
            .find(|control| control["id"] == control_id)
            .expect("product control");
        if control["status"] == expected_status {
            return control;
        }
        last_control = Some(control);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "product control {control_id} did not reach {expected_status}; last control: {last_control:?}"
    );
}

async fn wait_for_product_session_status(
    app: &axum::Router,
    workspace_id: &str,
    product_session_id: &str,
    expected_status: &str,
) -> serde_json::Value {
    let mut last_session = None;
    for _ in 0..120 {
        let session = get_product_session(app, workspace_id, product_session_id).await;
        if session["status"] == expected_status {
            return session;
        }
        last_session = Some(session);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "product session {product_session_id} did not reach {expected_status}; last session: {last_session:?}"
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
    assert_eq!(response.status(), StatusCode::OK);
    decode_json(response).await
}

async fn wait_for_done(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not finish; last state: {last_state:?}");
}

async fn wait_for_pending_input(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not wait for input; last state: {last_state:?}");
}

async fn wait_for_input_event(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not publish input event; last state: {last_state:?}");
}

async fn wait_for_approval_event(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not publish approval event; last state: {last_state:?}");
}

async fn wait_for_pending_approval(app: axum::Router, job_id: String) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not wait for approval; last state: {last_state:?}");
}

async fn wait_for_status(
    app: axum::Router,
    job_id: String,
    expected: RunStatus,
) -> JobStateResponse {
    let mut last_state = None;
    for _ in 0..80 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
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
    config
}
