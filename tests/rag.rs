#![cfg(feature = "rag")]

use rove::tools::rag::{DeterministicEmbedder, RagIndex, RetrieveKind};

#[tokio::test]
async fn retrieve_code_finds_relevant_chunk() {
    let tmp = tempfile::TempDir::new().unwrap();
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("auth.rs"),
        "pub fn validate_authentication_token(token: &str) -> bool { token == \"ok\" }",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("billing.rs"),
        "pub fn calculate_invoice_total(cents: u64) -> u64 { cents }",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    let indexed = index.ingest_workspace(&embedder).await.unwrap();
    assert!(indexed >= 2);

    let hits = index
        .retrieve(&embedder, RetrieveKind::Code, "authentication token", 3)
        .await
        .unwrap();

    assert!(!hits.is_empty());
    assert_eq!(hits[0].path, "src/auth.rs");
    assert!(hits[0].content.contains("validate_authentication_token"));
}

#[tokio::test]
async fn retrieve_docs_ignores_code_chunks() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("README.md"),
        "Deployment guide: set ROVE_MODEL before running production tasks.",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src").join("deploy.rs"),
        "pub fn deploy_binary() {}",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();

    let hits = index
        .retrieve(&embedder, RetrieveKind::Docs, "deployment guide", 3)
        .await
        .unwrap();

    assert!(!hits.is_empty());
    assert_eq!(hits[0].path, "README.md");
}
