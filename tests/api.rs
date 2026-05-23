use axum::body::Body;
use axum::http::{Request, StatusCode};
use rove::config::AppConfig;
use rove::core::types::RunStatus;
use rove::core::workspace::Workspace;
use rove::interfaces::api::{
    ApiState, CreateJobResponse, JobStateResponse, router, serve_listener,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

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
async fn api_writes_run_artifacts_for_completed_job() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let run_store = rove::state::store::StateStore::new(&workspace.state_dir).run_store;
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

    let run_dir = run_store.run_dir(&created.run_id);
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
        serde_json::from_slice(&std::fs::read(task_state_path).unwrap()).unwrap();
    assert_eq!(task_state["job_id"], created.job_id.to_string());
    assert_eq!(task_state["run_id"], created.run_id.to_string());
    assert_eq!(task_state["goal"], "artifact api");

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["job_id"], created.job_id.to_string());
    assert_eq!(report["run_id"], created.run_id.to_string());
    assert_eq!(report["status"], "success");
    assert_eq!(report["output"], "fake response: artifact api");
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
    let run_store = rove::state::store::StateStore::new(&workspace.state_dir).run_store;
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
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"approved.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
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
    assert_eq!(approval.name, "fs_write");
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
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"rejected.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
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
    assert_eq!(approval.name, "fs_write");

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

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);
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
async fn api_cancel_clears_pending_destructive_tool_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("cancelled.txt");
    let run_store = rove::state::store::StateStore::new(&workspace.state_dir).run_store;
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"cancelled.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
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
    assert_eq!(pending.pending_approvals[0].name, "fs_write");

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
    let run_store = rove::state::store::StateStore::new(&workspace.state_dir).run_store;
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
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"shutdown-cancelled.txt\",\"content\":\"no\"}}","model":"fake-raw","approval":"ask","max_steps":1}"#,
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
    assert_eq!(pending.pending_approvals[0].name, "fs_write");

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
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"default-ask.txt\",\"content\":\"safe\"}}","model":"fake-raw","max_steps":1}"#,
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
    assert_eq!(pending.pending_approvals[0].name, "fs_write");
    assert!(!output_path.exists(), "default approval should wait");
}

#[tokio::test]
async fn api_auto_approval_runs_destructive_tool_without_pending_approval() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let output_path = workspace.root.join("auto.txt");
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"{\"tool\":\"fs_write\",\"args\":{\"path\":\"auto.txt\",\"content\":\"ok\"}}","model":"fake-raw","approval":"auto","max_steps":1}"#,
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
        "tool": "update_memory_index",
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
        "tool": "read_memory_topic",
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

#[cfg(not(feature = "rag"))]
#[tokio::test]
async fn api_registers_rag_stub_tools_without_rag_feature() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let message = serde_json::json!({
        "tool": "retrieve_code",
        "args": { "query": "authentication token" }
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
    assert!(text.contains("requires the `rag` feature"));
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

    let pending = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    let input = pending.pending_inputs.first().unwrap();
    assert_eq!(input.prompt, "Which branch should I use?");

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

fn test_config() -> AppConfig {
    AppConfig {
        api_base: "http://127.0.0.1".to_string(),
        api_key: String::new(),
        model: "fake".to_string(),
        max_steps: 4,
        system_prompt_path: "prompts/system.md".into(),
        mcp_config_path: ".rove/mcp_servers.json".into(),
    }
}
