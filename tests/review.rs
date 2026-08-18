use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
use rove_api::{ApiState, router};
use rove_app_bootstrap::AppConfig;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::RunId;
use rove_runtime::workspace::Workspace;
use serde_json::Value;
use tower::ServiceExt;

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.provider.model = "fake".to_string();
    config.runtime.max_steps = 4;
    config.source_summary.user_config_path = workspace_root()
        .join("target/test-review-provider-catalogs")
        .join(ulid::Ulid::new().to_string())
        .join("config.toml");
    config
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
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}

fn assert_tree_omits(root: &Path, needle: &str) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_tree_omits(&path, needle);
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "{} leaked Review source text",
            path.display()
        );
    }
}

async fn request_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    value: Value,
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

async fn get(app: &axum::Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn create_workspace(app: &axum::Router, root: &Path, kind: &str) -> Value {
    let response = request_json(
        app,
        "POST",
        "/product/workspaces",
        serde_json::json!({
            "root": root,
            "kind": kind,
            "display_name": "Review workspace",
            "pinned": false
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json(response).await
}

async fn create_session(app: &axum::Router, workspace_id: &str) -> Value {
    let response = request_json(
        app,
        "POST",
        "/product/sessions",
        serde_json::json!({"workspace_id": workspace_id, "title": "Review session"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json(response).await
}

async fn wait_for_review(app: &axum::Router, review_id: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..400 {
        let response = get(app, &format!("/product/reviews/{review_id}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        last = json(response).await;
        if !matches!(last["status"].as_str(), Some("queued" | "running")) {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("Review did not finish: {last:#}");
}

#[tokio::test]
async fn product_review_is_hard_read_only_idempotent_and_stale_aware() {
    let server = tempfile::TempDir::new().unwrap();
    let repo = tempfile::TempDir::new().unwrap();
    let source_marker = "REVIEW_SOURCE_SECRET_4f8c";
    run_git(repo.path(), &["init", "-q"]);
    run_git(
        repo.path(),
        &["config", "user.email", "review@example.invalid"],
    );
    run_git(repo.path(), &["config", "user.name", "Review Test"]);
    std::fs::write(repo.path().join("tracked.txt"), "before\n").unwrap();
    run_git(repo.path(), &["add", "tracked.txt"]);
    run_git(repo.path(), &["commit", "-qm", "initial"]);
    std::fs::write(
        repo.path().join("tracked.txt"),
        format!("{source_marker}\n"),
    )
    .unwrap();

    let workspace = Workspace::detect(server.path()).unwrap();
    let mut config = test_config();
    config.state.state_dir = "api-state".into();
    let app = router(ApiState::new(workspace, config.clone()));
    let product_workspace = create_workspace(&app, repo.path(), "repo").await;
    let session = create_session(&app, product_workspace["id"].as_str().unwrap()).await;
    let session_id = session["id"].as_str().unwrap();
    let before_status = git_output(repo.path(), &["status", "--porcelain=v1", "-z"]);
    let before_content = std::fs::read(repo.path().join("tracked.txt")).unwrap();

    let request = serde_json::json!({
        "target": {"kind": "uncommitted"},
        "idempotency_key": "review-once",
        "max_steps": 4
    });
    let created = request_json(
        &app,
        "POST",
        &format!("/product/sessions/{session_id}/reviews"),
        request.clone(),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json(created).await;
    let review_id = created["id"].as_str().unwrap().to_string();
    assert!(created["job_id"].as_str().is_some());
    assert!(created["run_id"].as_str().is_some());

    let replay = request_json(
        &app,
        "POST",
        &format!("/product/sessions/{session_id}/reviews"),
        request,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(json(replay).await["id"], review_id);

    let review = wait_for_review(&app, &review_id).await;
    assert_eq!(review["status"], "pass");
    assert_eq!(review["conclusion"], "pass");
    assert_eq!(review["target"]["entries"], 1);
    assert_eq!(review["findings_count"], 0);
    assert_eq!(
        review["result"]["execution_capabilities"]["filesystem_write"],
        false
    );
    assert_eq!(
        review["result"]["execution_capabilities"]["process_run"],
        false
    );
    assert_eq!(
        review["result"]["execution_capabilities"]["process_stdio"],
        false
    );
    let serialized = review.to_string();
    assert!(!serialized.contains("snapshot_bytes"));
    assert!(!serialized.contains(source_marker));

    let run_id: RunId = serde_json::from_value(review["run_id"].clone()).unwrap();
    let review_state_root = std::fs::canonicalize(
        std::env::temp_dir()
            .join("rove-review-state")
            .join(&review_id),
    )
    .unwrap();
    let state_store = StateStore::with_index_path(
        &review_state_root,
        review_state_root.join("state.sqlite"),
        5_000,
    );
    let run_dir = state_store.run_store.run_dir(&run_id);
    let target_snapshot =
        std::fs::read_to_string(review_state_root.join("target_snapshot.json")).unwrap();
    assert!(target_snapshot.contains(source_marker));
    for artifact in ["trace.jsonl", "task_state.json", "report.json"] {
        let persisted = std::fs::read_to_string(run_dir.join(artifact)).unwrap();
        assert!(
            !persisted.contains(source_marker),
            "{artifact} leaked Review source text"
        );
    }
    assert_tree_omits(&run_dir, source_marker);

    assert_eq!(
        std::fs::read(repo.path().join("tracked.txt")).unwrap(),
        before_content
    );
    assert_eq!(
        git_output(repo.path(), &["status", "--porcelain=v1", "-z"]),
        before_status
    );
    assert!(!repo.path().join(".rove").exists());

    let findings = get(
        &app,
        &format!("/product/reviews/{review_id}/findings?limit=1"),
    )
    .await;
    assert_eq!(findings.status(), StatusCode::OK);
    let findings = json(findings).await;
    assert_eq!(findings["findings"], serde_json::json!([]));

    for _ in 0..2 {
        let cancelled = request_json(
            &app,
            "POST",
            &format!("/product/reviews/{review_id}/cancel"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(cancelled.status(), StatusCode::OK);
        let cancelled = json(cancelled).await;
        assert_eq!(cancelled["id"], review_id);
        assert_eq!(cancelled["status"], "pass");
    }

    drop(app);
    std::fs::write(repo.path().join("tracked.txt"), "changed again\n").unwrap();
    let restarted_app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        config,
    ));
    let stale = get(&restarted_app, &format!("/product/reviews/{review_id}")).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(json(stale).await["status"], "needs_attention");
}

#[tokio::test]
async fn product_review_rejects_non_repo_workspace_with_typed_error() {
    let server = tempfile::TempDir::new().unwrap();
    let folder = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(server.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));
    let product_workspace = create_workspace(&app, folder.path(), "folder").await;
    let session = create_session(&app, product_workspace["id"].as_str().unwrap()).await;

    let response = request_json(
        &app,
        "POST",
        &format!(
            "/product/sessions/{}/reviews",
            session["id"].as_str().unwrap()
        ),
        serde_json::json!({"target": {"kind": "uncommitted"}}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json(response).await["code"], "review_target_unavailable");
}

#[tokio::test]
async fn review_routes_are_published_in_openapi() {
    let server = tempfile::TempDir::new().unwrap();
    let app = router(ApiState::new(
        Workspace::detect(server.path()).unwrap(),
        test_config(),
    ));
    let response = get(&app, "/api/openapi.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    let spec = json(response).await;
    for (path, method) in [
        ("/product/sessions/{session_id}/reviews", "post"),
        ("/product/sessions/{session_id}/reviews", "get"),
        ("/product/reviews/{review_id}", "get"),
        ("/product/reviews/{review_id}/findings", "get"),
        ("/product/reviews/{review_id}/cancel", "post"),
    ] {
        assert!(
            spec["paths"][path][method].is_object(),
            "missing {method} {path}"
        );
    }
}
