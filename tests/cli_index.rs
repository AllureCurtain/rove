#[cfg(feature = "rag")]
use std::path::Path;
use std::path::PathBuf;

use rove::interfaces::cli::index::{IndexOptions, format_index_result, run};

#[test]
fn format_index_result_reports_chunk_count_and_db_path() {
    let root = PathBuf::from("workspace");
    let output = format_index_result(3, &root);

    assert!(output.contains("indexed 3 chunks"));
    assert!(output.contains(&root.join(".rove").join("rag.lancedb").display().to_string()));
    assert!(output.ends_with('\n'));
}

#[cfg(not(feature = "rag"))]
#[tokio::test]
async fn index_run_explains_when_rag_feature_is_disabled() {
    let err = run(IndexOptions {
        cwd: Some(PathBuf::from(".")),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap_err();

    assert!(err.to_string().contains("requires the `rag` feature"));
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn deterministic_index_run_writes_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path().join("src").join("lib.rs"),
        "fn searchable_symbol() {}\n",
    );
    write_file(tmp.path().join("README.md"), "# searchable docs\n");

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    assert!(tmp.path().join(".rove").join("rag_manifest.json").exists());
    assert!(
        tmp.path()
            .join(".rove")
            .join("rag_index_log.jsonl")
            .exists()
    );
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn deterministic_index_run_honors_configured_state_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("custom-state");
    write_file(
        tmp.path().join(".rove").join("config.toml"),
        &format!("[state]\nstate_dir = \"{}\"\n", "custom-state"),
    );
    write_file(
        tmp.path().join("src").join("lib.rs"),
        "fn configured_state_dir_symbol() {}\n",
    );
    write_file(tmp.path().join("README.md"), "# configured state docs\n");

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    assert!(state_dir.join("rag_manifest.json").exists());
    assert!(state_dir.join("rag_index_log.jsonl").exists());
    assert!(state_dir.join("rag.lancedb").exists());
    assert!(!tmp.path().join(".rove").join("rag_manifest.json").exists());
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn eval_run_writes_report_without_llm_generation() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path().join("README.md"),
        "# Retrieval\n\nRAG eval reports ranked retrieval chunks.",
    );

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: Some("RAG eval reports".to_string()),
        eval_kind: Some("docs".to_string()),
        eval_limit: 3,
    })
    .await
    .unwrap();

    let eval_dir = tmp.path().join(".rove").join("rag_eval");
    let mut reports = std::fs::read_dir(eval_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    reports.sort();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reports.last().unwrap()).unwrap()).unwrap();

    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["query"], "RAG eval reports");
    assert_eq!(report["kind"], "docs");
    assert_eq!(
        report["embedder"],
        "embedding-deterministic:local:deterministic-64"
    );
    assert_eq!(report["reranker"], "rerank-noop");
    assert!(report["duration_ms"].as_u64().is_some());
    assert!(
        report["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|channel| channel["name"] == "lexical")
    );
    assert_eq!(report["results"][0]["rank"], 1);
    assert!(report.get("llm_output").is_none());
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn eval_run_honors_configured_state_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("custom-state");
    write_file(
        tmp.path().join(".rove").join("config.toml"),
        &format!("[state]\nstate_dir = \"{}\"\n", "custom-state"),
    );
    write_file(
        tmp.path().join("README.md"),
        "# Retrieval\n\nConfigured state eval report.",
    );

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: Some("Configured state eval".to_string()),
        eval_kind: Some("docs".to_string()),
        eval_limit: 3,
    })
    .await
    .unwrap();

    let eval_dir = state_dir.join("rag_eval");
    assert!(eval_dir.exists());
    assert!(!tmp.path().join(".rove").join("rag_eval").exists());
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn eval_run_uses_configured_remote_reranker() {
    let server = start_rerank_server().await;
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path().join(".rove").join("config.toml"),
        &format!(
            r#"
[rag]
deterministic = true
embedding_api_base = "{}"
rerank_provider = "dashscope"
rerank_model = "qwen3-rerank"
rerank_api_key = "secret"
fallback_to_deterministic = false
"#,
            server.base_url
        ),
    );
    write_file(
        tmp.path().join("docs").join("a.md"),
        "remote rerank query alpha candidate",
    );
    write_file(
        tmp.path().join("docs").join("b.md"),
        "remote rerank query beta candidate",
    );

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: true,
        embedding_model: None,
        eval_query: Some("remote rerank query".to_string()),
        eval_kind: Some("docs".to_string()),
        eval_limit: 2,
    })
    .await
    .unwrap();

    let requested_documents = server.requested_documents.lock().unwrap().clone().unwrap();
    let eval_dir = tmp.path().join(".rove").join("rag_eval");
    let mut reports = std::fs::read_dir(eval_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    reports.sort();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reports.last().unwrap()).unwrap()).unwrap();

    assert_eq!(
        report["reranker"],
        format!("rerank-dashscope:{}:qwen3-rerank", server.base_url)
    );
    assert_eq!(
        report["results"][0]["content_preview"],
        requested_documents[1]
    );
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn provider_embedding_without_key_falls_back_to_deterministic_when_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path().join(".rove").join("config.toml"),
        r#"
[rag]
deterministic = false
embedding_provider = "openai-compatible"
embedding_model = "text-embedding-3-small"
fallback_to_deterministic = true
"#,
    );
    write_file(tmp.path().join("README.md"), "# fallback docs\n");

    run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: false,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap();

    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(tmp.path().join(".rove").join("rag_manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["embedding"]["provider"], "deterministic");
    assert_eq!(manifest["embedding"]["model"], "deterministic-64");
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn provider_embedding_without_key_errors_when_fallback_disabled() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path().join(".rove").join("config.toml"),
        r#"
[rag]
deterministic = false
embedding_provider = "openai-compatible"
embedding_model = "text-embedding-3-small"
fallback_to_deterministic = false
"#,
    );
    write_file(tmp.path().join("README.md"), "# missing key docs\n");

    let err = run(IndexOptions {
        cwd: Some(tmp.path().to_path_buf()),
        deterministic: false,
        embedding_model: None,
        eval_query: None,
        eval_kind: None,
        eval_limit: 8,
    })
    .await
    .unwrap_err();

    assert!(err.to_string().contains("rag.embedding_api_key"));
}

#[cfg(feature = "rag")]
fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[cfg(feature = "rag")]
struct RerankTestServer {
    base_url: String,
    requested_documents: std::sync::Arc<std::sync::Mutex<Option<Vec<String>>>>,
}

#[cfg(feature = "rag")]
async fn start_rerank_server() -> RerankTestServer {
    use axum::Json;
    use axum::Router;
    use axum::routing::post;
    use std::sync::{Arc, Mutex};

    let requested_documents = Arc::new(Mutex::new(None));
    let requested_documents_for_handler = requested_documents.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/services/rerank/text-rerank/text-rerank",
        post(move |Json(body): Json<serde_json::Value>| {
            let requested_documents = requested_documents_for_handler.clone();
            async move {
                let documents = body["input"]["documents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<Vec<_>>();
                *requested_documents.lock().unwrap() = Some(documents);
                Json(serde_json::json!({
                    "output": {
                        "results": [
                            { "index": 1, "relevance_score": 0.91 },
                            { "index": 0, "relevance_score": 0.42 }
                        ]
                    }
                }))
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    RerankTestServer {
        base_url: format!("http://{addr}"),
        requested_documents,
    }
}
