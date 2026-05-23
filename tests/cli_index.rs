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
    })
    .await
    .unwrap();

    assert!(tmp.path().join(".rove").join("rag_manifest.json").exists());
}

#[cfg(feature = "rag")]
fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
