use axum::body::Body;
use axum::http::{Request, StatusCode};
use rove::config::AppConfig;
use rove::core::types::RunStatus;
use rove::core::workspace::Workspace;
use rove::interfaces::api::{ApiState, CreateJobResponse, JobStateResponse, router};
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

async fn wait_for_done(app: axum::Router, job_id: String) -> JobStateResponse {
    for _ in 0..20 {
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
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not finish");
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
