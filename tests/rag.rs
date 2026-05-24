#![cfg(feature = "rag")]

use rove::tools::rag::{
    ChunkingManifest, DeterministicEmbedder, EmbeddingManifest, IndexManifest, IndexedFile,
    ManifestChunk, RagIndex, RetrieveKind,
};

#[test]
fn index_manifest_serializes_schema_files_and_chunks() {
    let manifest = IndexManifest {
        schema_version: 1,
        workspace_root: "D:/workspace".to_string(),
        embedding: EmbeddingManifest {
            provider: "deterministic".to_string(),
            model: "deterministic-64".to_string(),
            dims: 64,
        },
        chunking: ChunkingManifest {
            strategy: "markdown-aware".to_string(),
            target_chars: 1600,
            overlap_chars: 160,
        },
        files: vec![IndexedFile {
            path: "docs/guide.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:abc".to_string(),
            chunk_count: 1,
            indexed_at: "2026-05-24T00:00:00Z".to_string(),
        }],
        chunks: vec![ManifestChunk {
            id: "docs/guide.md#0000".to_string(),
            path: "docs/guide.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:abc".to_string(),
            chunk_hash: "sha256:def".to_string(),
            start_byte: 0,
            end_byte: 12,
            heading: Some("Intro".to_string()),
            content: "hello world".to_string(),
            vector: vec![0.0; 64],
        }],
    };

    let json = serde_json::to_value(&manifest).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["embedding"]["provider"], "deterministic");
    assert_eq!(json["chunking"]["strategy"], "markdown-aware");
    assert_eq!(json["files"][0]["path"], "docs/guide.md");
    assert_eq!(json["chunks"][0]["id"], "docs/guide.md#0000");
}

#[tokio::test]
async fn malformed_manifest_returns_clear_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join(".rove");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("rag_manifest.json"), "{not-json").unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    let err = index
        .retrieve(&embedder, RetrieveKind::Docs, "anything", 3)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("failed to parse RAG manifest"));
}

#[tokio::test]
async fn ingestion_pipeline_writes_manifest_and_stage_log() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
    std::fs::write(
        tmp.path().join("docs").join("guide.md"),
        "# Guide\n\n## Retrieval\n\nUse retrieve_docs for indexed docs.",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    let count = index.ingest_workspace(&embedder).await.unwrap();

    assert_eq!(count, 1);

    let manifest_path = tmp.path().join(".rove").join("rag_manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["chunking"]["strategy"], "markdown-aware");
    assert_eq!(manifest["files"][0]["path"], "docs/guide.md");
    assert_eq!(manifest["chunks"][0]["id"], "docs/guide.md#0000");
    assert_eq!(manifest["chunks"][0]["heading"], "Guide > Retrieval");

    let log_path = tmp.path().join(".rove").join("rag_index_log.jsonl");
    let log = std::fs::read_to_string(log_path).unwrap();
    for stage in [
        "ScanWorkspace",
        "ParseReadableFiles",
        "ChunkDocuments",
        "EmbedChunks",
        "PersistIndex",
        "WriteManifestAndLog",
    ] {
        assert!(log.contains(stage), "missing stage log for {stage}");
    }
    assert!(
        log.lines()
            .all(|line| line.contains("\"schema_version\":1"))
    );
    assert!(
        log.lines()
            .all(|line| line.contains("\"status\":\"completed\""))
    );
}

#[tokio::test]
async fn rag_public_api_survives_module_split() {
    let tmp = tempfile::TempDir::new().unwrap();
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("auth.rs"),
        "pub fn validate_authentication_token(token: &str) -> bool { token == \"ok\" }",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    let indexed = index.ingest_workspace(&embedder).await.unwrap();
    assert_eq!(indexed, 1);

    let hits = index
        .retrieve(&embedder, RetrieveKind::Code, "authentication token", 5)
        .await
        .unwrap();

    assert_eq!(hits[0].path, "src/auth.rs");
    assert!(hits[0].content.contains("validate_authentication_token"));
}

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

#[tokio::test]
async fn manifest_fallback_retrieval_still_works_without_lancedb() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src").join("auth.rs"),
        "pub fn validate_authentication_token(token: &str) -> bool { token == \"ok\" }",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();
    std::fs::remove_dir_all(tmp.path().join(".rove").join("rag.lancedb")).unwrap();

    let hits = index
        .retrieve(&embedder, RetrieveKind::Code, "authentication token", 3)
        .await
        .unwrap();

    assert_eq!(hits[0].path, "src/auth.rs");
    assert!(hits[0].source.contains("vector"));
}

#[tokio::test]
async fn lexical_channel_ranks_exact_symbol_matches() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src").join("auth.rs"),
        "pub fn validate_authentication_token(token: &str) -> bool { token == \"ok\" }",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src").join("billing.rs"),
        "pub fn calculate_invoice_total(cents: u64) -> u64 { cents }",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();

    let hits = index
        .retrieve(&embedder, RetrieveKind::Code, "calculate_invoice_total", 3)
        .await
        .unwrap();

    assert_eq!(hits[0].path, "src/billing.rs");
    assert!(hits[0].source.contains("lexical"));
}

#[tokio::test]
async fn path_scoped_channel_prefers_matching_path_hint() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src").join("auth.rs"),
        "pub fn shared_name() {}",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src").join("billing.rs"),
        "pub fn shared_name() {}",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();

    let hits = index
        .retrieve(
            &embedder,
            RetrieveKind::Code,
            "src/billing.rs shared_name",
            3,
        )
        .await
        .unwrap();

    assert_eq!(hits[0].path, "src/billing.rs");
    assert!(hits[0].source.contains("path"));
}

#[tokio::test]
async fn rag_tool_output_contains_query_metadata_and_results() {
    use rove::core::types::{ApprovalPolicy, ToolContext};
    use rove::core::workspace::Workspace;
    use rove::tools::rag::RagRetrieveTool;
    use rove::tools::traits::Tool;
    use tokio_util::sync::CancellationToken;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("README.md"),
        "# RAG\n\nretrieval eval report",
    )
    .unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();

    let workspace = Workspace::detect(tmp.path()).unwrap();
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
        input_provider: None,
    };
    let tool = RagRetrieveTool::docs(workspace.root.clone());

    let output = tool
        .execute(
            serde_json::json!({"query": "retrieval eval", "limit": 2}),
            &ctx,
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&output.content).unwrap();

    assert_eq!(json["query"], "retrieval eval");
    assert_eq!(json["normalized_query"], "retrieval eval");
    assert_eq!(json["kind"], "docs");
    assert_eq!(json["limit"], 2);
    assert!(
        json["results"][0]["source"]
            .as_str()
            .unwrap()
            .contains("lexical")
    );
}
