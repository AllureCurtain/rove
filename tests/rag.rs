#![cfg(feature = "rag")]

use rove_cli::rag::{
    ChunkingManifest, DeterministicEmbedder, EmbeddingManifest, IndexManifest, IndexedFile,
    ManifestChunk, RagIndex, RagPromptService, RetrieveKind, RetrievedChunk,
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

#[test]
fn rag_prompt_service_formats_evidence_boundary() {
    let chunks = vec![RetrievedChunk {
        id: "src/auth.rs#0001".to_string(),
        path: "src/auth.rs".to_string(),
        kind: RetrieveKind::Code,
        content: "pub fn validate_token(token: &str) -> bool { !token.is_empty() }".to_string(),
        score: 0.91,
        source: "lexical+vector".to_string(),
        heading: None,
        chunk_hash: Some("sha256:abc".to_string()),
    }];

    let prompt = RagPromptService.format_context("validate token", &chunks);

    assert!(prompt.contains("RAG evidence for query: validate token"));
    assert!(prompt.contains("BEGIN RAG EVIDENCE"));
    assert!(prompt.contains("END RAG EVIDENCE"));
    assert!(prompt.contains("src/auth.rs#0001"));
    assert!(prompt.contains("Use only the evidence inside this boundary"));
}

#[test]
fn code_aware_chunker_keeps_rust_functions_and_tests_coherent() {
    use rove_cli::rag::ParsedDocument;
    use rove_cli::rag::ingest::chunking::{ChunkingStrategy, CodeAwareChunker};

    let document = ParsedDocument {
        path: "src/lib.rs".to_string(),
        kind: RetrieveKind::Code,
        content_hash: "sha256:code".to_string(),
        content: r#"
pub fn validate_token(token: &str) -> bool {
    !token.trim().is_empty()
}

#[test]
fn rejects_empty_token() {
    assert!(!validate_token(""));
}
"#
        .to_string(),
    };
    let chunker = CodeAwareChunker::new(80, 0);

    let chunks = chunker.chunk(&document);

    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].content.contains("pub fn validate_token"));
    assert!(!chunks[0].content.contains("rejects_empty_token"));
    assert!(chunks[1].content.contains("#[test]"));
    assert!(chunks[1].content.contains("fn rejects_empty_token"));
    assert_eq!(chunks[0].heading.as_deref(), Some("fn validate_token"));
    assert_eq!(chunks[1].heading.as_deref(), Some("fn rejects_empty_token"));
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
    assert_eq!(manifest["chunking"]["strategy"], "mixed-code-markdown");
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
    use rove_cli::rag::RagRetrieveTool;
    use rove_core::Tool;
    use rove_runtime::memory::paths::MemoryPaths;
    use rove_runtime::tools::runtime_context::runtime_tool_context;
    use rove_runtime::types::ApprovalPolicy;
    use rove_runtime::workspace::Workspace;
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
    let ctx = runtime_tool_context(
        rove_runtime::types::CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    );
    let tool = RagRetrieveTool::docs(workspace.root.clone());
    let schema = tool.schema();
    assert_eq!(schema.capability.as_ref().unwrap().status, "enabled");
    assert_eq!(
        schema.capability.as_ref().unwrap().feature.as_deref(),
        Some("rag")
    );

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

#[tokio::test]
async fn rag_retrieval_reads_from_configured_state_dir() {
    use rove_cli::rag::RagRetrieveTool;
    use rove_core::Tool;
    use rove_runtime::memory::paths::MemoryPaths;
    use rove_runtime::tools::runtime_context::runtime_tool_context;
    use rove_runtime::types::ApprovalPolicy;
    use rove_runtime::workspace::{Workspace, WorkspaceKind};
    use tokio_util::sync::CancellationToken;

    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("custom-state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        tmp.path().join("README.md"),
        "# Configured State\n\nRetrieval should read this custom state index.",
    )
    .unwrap();
    let workspace = Workspace {
        root: tmp.path().to_path_buf(),
        kind: WorkspaceKind::Folder,
        state_dir: state_dir.clone(),
    };
    let index = RagIndex::new_with_state_dir(workspace.root.clone(), state_dir);
    index
        .ingest_workspace(&DeterministicEmbedder)
        .await
        .unwrap();

    let ctx = runtime_tool_context(
        rove_runtime::types::CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    );
    let tool = RagRetrieveTool::docs(workspace.root.clone());

    let output = tool
        .execute(
            serde_json::json!({"query": "custom state index", "limit": 3}),
            &ctx,
        )
        .await
        .unwrap();

    assert!(output.content.contains("Configured State"));
}
