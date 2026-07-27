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
    MAX_PRODUCT_TEXT_BYTES, ProductSessionId, router, serve_listener,
};
use rove_app_bootstrap::AppConfig;
use rove_runtime::events::StreamEvent;
use rove_runtime::execution::StepRecordStatus;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{Message, Role, RunStatus, SessionId, TaskState, ToolCallRef};
use rove_runtime::workspace::Workspace;
use std::sync::{Arc, Mutex};
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
            "model": "fake",
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
async fn product_default_approval_is_honored_and_explicit_approval_wins() {
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
            "model": "fake-raw",
            "max_steps": 1,
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

    let explicit_job = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "write_file",
                "args": {"path": "explicit-auto.txt", "content": "explicit"}
            }).to_string(),
            "model": "fake-raw",
            "approval": "auto",
            "max_steps": 1,
            "product_session_id": session_id
        }),
    )
    .await;
    assert_eq!(explicit_job.status(), StatusCode::OK);
    let explicit_job: CreateJobResponse = decode_json(explicit_job).await;
    let explicit_state = wait_for_done(app.clone(), explicit_job.job_id.to_string()).await;
    assert!(explicit_state.pending_approvals.is_empty());
    assert_eq!(
        std::fs::read_to_string(folder.path().join("explicit-auto.txt")).unwrap(),
        "explicit"
    );
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
    let active = post_json(
        &app,
        "/jobs",
        serde_json::json!({
            "message": serde_json::json!({
                "tool": "request_input",
                "args": {"prompt": "keep the turn active"}
            }).to_string(),
            "model": "fake-raw",
            "max_steps": 1,
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
async fn product_memory_routes_are_bounded_redacted_and_idempotent() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let memory_dir = server.path().join("platform-memory");

    let empty = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/memory/topics")
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
                .uri("/product/memory/topics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = decode_json(listed).await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["topics"][0]["slug"], "private-source");
    assert!(listed["topics"][0].get("source").is_none());

    let content = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/memory/topics/private-source")
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
                .uri("/product/memory/topics/private-source")
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
                .uri("/product/memory/topics/bad--slug")
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
                .uri("/product/memory/topics/private-source")
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
    for _ in 0..2 {
        let retry = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/product/memory/topics/private-source")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    }
    assert!(
        !std::fs::read_to_string(memory_dir.join("MEMORY.md"))
            .unwrap()
            .contains("private-source")
    );

    std::fs::write(memory_dir.join("MEMORY.md"), [0xff, 0xfe]).unwrap();
    let corrupt = app
        .oneshot(
            Request::builder()
                .uri("/product/memory/topics")
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
async fn product_memory_routes_reject_topic_and_index_symlinks_when_supported() {
    let server = tempfile::TempDir::new().unwrap();
    let mut config = test_config();
    config.memory.durable_dir = "platform-memory".into();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
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
                .uri("/product/memory/topics/linked")
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
                .uri("/product/memory/topics")
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
    assert_eq!(runtime["resume_health"]["status"], "healthy");
    assert_eq!(runtime["resume_health"]["workspace_count"], 1);
    assert_eq!(runtime["resume_health"]["session_count"], 1);
    assert_eq!(runtime["resume_health"]["bound_session_count"], 0);
    assert_eq!(runtime["resume_health"]["running_session_count"], 0);
    assert_eq!(runtime["resume_health"]["needs_attention_session_count"], 0);
    let keys = runtime.as_object().unwrap();
    assert_eq!(keys.len(), 4);
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
        ("/product/sessions", "get"),
        ("/product/sessions", "post"),
        ("/product/sessions/{session_id}", "patch"),
        ("/product/sessions/{session_id}", "delete"),
        ("/product/sessions/{session_id}/transcript", "get"),
        ("/product/provider-profiles", "get"),
        ("/product/provider-profiles", "post"),
        ("/product/provider-profiles/{profile_id}", "put"),
        ("/product/provider-profiles/{profile_id}", "delete"),
        ("/product/preferences", "get"),
        ("/product/preferences", "put"),
        ("/product/memory/topics", "get"),
        ("/product/memory/topics/{slug}", "get"),
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
        ("/product/sessions", "get"),
        ("/product/sessions", "post"),
        ("/product/sessions/{session_id}", "patch"),
        ("/product/sessions/{session_id}", "delete"),
        ("/product/sessions/{session_id}/transcript", "get"),
        ("/product/provider-profiles", "get"),
        ("/product/provider-profiles", "post"),
        ("/product/provider-profiles/{profile_id}", "put"),
        ("/product/provider-profiles/{profile_id}", "delete"),
        ("/product/preferences", "get"),
        ("/product/preferences", "put"),
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
        "ProductMemoryTopic",
        "ProductMemoryTopicContentResponse",
        "ProductMemoryTopicsResponse",
        "ProductPreferences",
        "ProductRuntimeInfo",
        "ProductSession",
        "ProductTranscriptResponse",
        "ProductWorkspace",
        "ProviderProfileRequest",
        "ProviderTestRequest",
        "ProviderTestResponse",
        "RecallTestRequest",
        "RecallTestResponse",
        "SubmitApprovalRequest",
        "SubmitInputRequest",
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
    assert_eq!(transcript["status"], "complete");
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
            "model": "fake",
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
            "model": "fake",
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
            "model": "fake",
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
            "model": "fake",
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
            "model": "fake-raw",
            "max_steps": 1,
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
            "model": "fake",
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
            "model": "fake",
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
            StreamEvent::LlmMessage { full, .. } if full == "fake response: after cancellation"
        )),
        "a cancelled product turn must not replay its terminal plan decision"
    );
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

async fn post_json(
    app: &axum::Router,
    uri: &str,
    value: serde_json::Value,
) -> axum::response::Response {
    request_json(app, "POST", uri, value).await
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
            "model": "fake",
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
