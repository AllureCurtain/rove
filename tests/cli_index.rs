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
fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
