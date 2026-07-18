# RAG Pipeline Hardening Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Phase 3A RAG Pipeline Hardening for rove by turning the current single-file RAG prototype into a staged, observable, local-first ingestion and retrieval pipeline.

**Architecture:** Keep `src/tools/mod.rs` as the feature gate: default builds continue compiling `src/tools/rag_stub.rs`, while `--features rag` compiles the real RAG module tree under `src/tools/rag/`. The real RAG path borrows ragent's explicit pipeline, ingestion node, chunking strategy, search channel, postprocessor, rewrite fallback, and pure retrieval eval patterns, adapted to rove's local filesystem, deterministic embedder, LanceDB storage, and manifest fallback.

**Tech Stack:** Rust 2024, tokio, async-trait, serde/serde_json, chrono, ulid, walkdir, LanceDB/Arrow behind `rag`, deterministic local embeddings, optional OpenAI-compatible embeddings.

---

## Review Gate

This plan is the only artifact for this turn. Do not start implementation until the user reviews and explicitly asks to execute this file.

## ragent Ideas To Adapt

- `IngestionNode` and `IngestionEngine`: adapt to `IngestionStage` plus `IngestionPipeline`, with in-memory context and append-only `.rove/rag_index_log.jsonl` instead of Spring beans and database task rows.
- `ChunkingStrategy`: adapt fixed-size and structure-aware chunkers into `FixedTextChunker` and `MarkdownAwareChunker`.
- `SearchChannel` and `MultiChannelRetrievalEngine`: adapt to `SearchChannel`, `SearchChannelResult`, and `RetrievalPipeline` with vector, lexical, and path-scoped channels.
- `SearchResultPostProcessor`: adapt to ordered postprocessors for dedupe, score normalization, and a no-op rerank boundary.
- `QueryRewriteService`: adapt to deterministic `QueryRewriteService` and `RewriteResult`; no LLM rewrite in Phase 3A.
- `EvalController`/`EvalResponse`: adapt to a CLI-only pure retrieval report path that writes `.rove/rag_eval/<run_id>.json` and does not call final LLM generation.

## File Structure

The implementation should end with this real RAG module layout:

```text
src/tools/rag/
├── mod.rs
├── types.rs
├── embed.rs
├── index.rs
├── rewrite.rs
├── eval.rs
├── ingest/
│   ├── mod.rs
│   ├── pipeline.rs
│   ├── stages.rs
│   ├── chunking.rs
│   └── log.rs
└── retrieve/
    ├── mod.rs
    ├── pipeline.rs
    ├── channel.rs
    ├── channels.rs
    └── postprocess.rs
```

`src/tools/rag.rs` must be removed after the module directory is created, because Rust cannot have both `src/tools/rag.rs` and `src/tools/rag/mod.rs` for the same module.

## Public Compatibility Rules

- Keep `src/tools/mod.rs` unchanged in behavior:

```rust
#[cfg(feature = "rag")]
pub mod rag;
#[cfg(not(feature = "rag"))]
#[path = "rag_stub.rs"]
pub mod rag;
```

- Keep `RagRetrieveTool::code(root)` and `RagRetrieveTool::docs(root)`.
- Keep tool names `retrieve_code` and `retrieve_docs`.
- Keep `RagIndex::new(workspace_root)` and keep a simple `ingest_workspace(&dyn Embedder)` path for `rove-index`.
- Keep deterministic tests runnable without network credentials.
- Keep LanceDB as the RAG-enabled primary vector store.
- Keep manifest fallback functional when LanceDB is missing or cannot be opened during retrieval.

---

### Task 1: Split The RAG Module Without Changing Behavior

**Files:**
- Delete after move: `src/tools/rag.rs`
- Create: `src/tools/rag/mod.rs`
- Create: `src/tools/rag/types.rs`
- Create: `src/tools/rag/embed.rs`
- Create: `src/tools/rag/index.rs`
- Modify: `src/interfaces/cli/index.rs`
- Test: `tests/rag.rs`
- Test: `tests/rag_default.rs`
- Test: `tests/cli_index.rs`

- [ ] **Step 1: Add a characterization test for the public RAG API**

Add this test to `tests/rag.rs`:

```rust
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
```

- [ ] **Step 2: Run the characterization test**

Run:

```powershell
cargo test --features rag --test rag rag_public_api_survives_module_split
```

Expected: PASS before the refactor. This locks the current public behavior before files move.

- [ ] **Step 3: Move public types into `types.rs`**

Move `RetrieveKind` and `RetrievedChunk` into `src/tools/rag/types.rs`. Keep `RetrieveKind::as_str()` available inside the module tree and expose these public fields:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrieveKind {
    Code,
    Docs,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievedChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content: String,
    pub score: f32,
    pub source: String,
    pub heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_hash: Option<String>,
}
```

While preserving behavior in this task, populate new fields with deterministic defaults:

```rust
id = format!("{}#{}", path, row_or_index);
source = "vector";
heading = None;
chunk_hash = None;
```

- [ ] **Step 4: Move embedders into `embed.rs`**

Move `EMBEDDING_DIMS`, `Embedder`, `DeterministicEmbedder`, `OpenAiEmbedder`, `deterministic_embedding`, `normalize_dims`, `normalize`, `cosine_similarity`, `tokenize`, and `stable_hash` into `src/tools/rag/embed.rs`.

Expose this shape:

```rust
pub const EMBEDDING_DIMS: usize = 64;

#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

pub fn normalize_dims(vector: &[f32]) -> Vec<f32>;
pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32;
pub fn tokenize(text: &str) -> Vec<String>;
```

- [ ] **Step 5: Move storage/indexing into `index.rs`**

Move `RagIndex`, LanceDB read/write code, manifest read/write code, `classify_path`, `is_ignored`, `chunk_text`, and the temporary `ChunkRecord`/`ManifestRecord` into `src/tools/rag/index.rs`.

Keep the public methods:

```rust
impl RagIndex {
    pub fn new(workspace_root: PathBuf) -> Self;

    pub async fn ingest_workspace(&self, embedder: &dyn Embedder) -> anyhow::Result<usize>;

    pub async fn retrieve(
        &self,
        embedder: &dyn Embedder,
        kind: RetrieveKind,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;
}
```

- [ ] **Step 6: Rebuild the module entry point in `mod.rs`**

Create `src/tools/rag/mod.rs` with public re-exports and the existing tool wiring:

```rust
mod embed;
mod index;
mod types;

pub use embed::{DeterministicEmbedder, Embedder, OpenAiEmbedder};
pub use index::RagIndex;
pub use types::{RetrieveKind, RetrievedChunk};
```

Keep `RagRetrieveTool` in `mod.rs` for now so the API surface remains easy to find.

- [ ] **Step 7: Run split verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag --test rag
cargo test --features rag --test cli_index
cargo test --test rag_default
```

Expected: all pass; `tests/rag_default.rs` still proves the default build uses `rag_stub.rs`.

- [ ] **Step 8: Commit**

```powershell
git add src/tools/rag src/tools/rag.rs src/interfaces/cli/index.rs tests/rag.rs tests/rag_default.rs tests/cli_index.rs
git commit -m "refactor: split rag module"
```

---

### Task 2: Add Manifest Schema And Storage Contracts

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/tools/rag/types.rs`
- Modify: `src/tools/rag/index.rs`
- Test: `tests/rag.rs`

- [ ] **Step 1: Write failing manifest schema tests**

Add these tests to `tests/rag.rs`:

```rust
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
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```powershell
cargo test --features rag --test rag index_manifest_serializes_schema_files_and_chunks malformed_manifest_returns_clear_error
```

Expected: compile failure because `IndexManifest`, `EmbeddingManifest`, `ChunkingManifest`, `IndexedFile`, and `ManifestChunk` do not exist yet.

- [ ] **Step 3: Add a direct optional SHA-256 dependency**

Update `Cargo.toml`:

```toml
[features]
rag = ["dep:arrow-array", "dep:arrow-schema", "dep:lancedb", "dep:walkdir", "dep:sha2"]

[dependencies]
sha2 = { version = "0.10", optional = true }
```

Use a local helper to format hashes as lowercase hex without adding another dependency:

```rust
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}
```

- [ ] **Step 4: Add manifest and chunk/index structs**

Add these serializable structs to `src/tools/rag/types.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexManifest {
    pub schema_version: u32,
    pub workspace_root: String,
    pub embedding: EmbeddingManifest,
    pub chunking: ChunkingManifest,
    pub files: Vec<IndexedFile>,
    pub chunks: Vec<ManifestChunk>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingManifest {
    pub provider: String,
    pub model: String,
    pub dims: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkingManifest {
    pub strategy: String,
    pub target_chars: usize,
    pub overlap_chars: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexedFile {
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_count: usize,
    pub indexed_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_hash: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub heading: Option<String>,
    pub content: String,
    pub vector: Vec<f32>,
}
```

- [ ] **Step 5: Make manifest parsing strict and actionable**

Change `RagIndex` manifest loading in `src/tools/rag/index.rs` so malformed JSON returns:

```rust
anyhow::Context::context(err, "failed to parse RAG manifest")
```

Do not silently return empty results on malformed manifests. Still return `Ok(Vec::new())` when `.rove/rag_manifest.json` does not exist.

- [ ] **Step 6: Keep legacy manifest compatibility for one task**

During this task only, support both shapes:

```rust
#[serde(untagged)]
enum ManifestOnDisk {
    V1(IndexManifest),
    Legacy(Vec<LegacyManifestRecord>),
}
```

This lets Task 1 artifacts continue to be readable while the richer writer is added in Task 4.

- [ ] **Step 7: Run manifest tests**

Run:

```powershell
cargo test --features rag --test rag index_manifest_serializes_schema_files_and_chunks malformed_manifest_returns_clear_error
```

Expected: PASS.

- [ ] **Step 8: Commit**

```powershell
git add Cargo.toml Cargo.lock src/tools/rag/types.rs src/tools/rag/index.rs tests/rag.rs
git commit -m "feat: add rag manifest schema"
```

---

### Task 3: Add Fixed And Markdown-Aware Chunking Strategies

**Files:**
- Create: `src/tools/rag/ingest/mod.rs`
- Create: `src/tools/rag/ingest/chunking.rs`
- Modify: `src/tools/rag/types.rs`
- Modify: `src/tools/rag/mod.rs`
- Test: `src/tools/rag/ingest/chunking.rs`

- [ ] **Step 1: Write failing fixed chunker tests**

Create `src/tools/rag/ingest/chunking.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::rag::types::{ParsedDocument, RetrieveKind};

    #[test]
    fn fixed_chunker_uses_overlap_and_stable_boundaries() {
        let document = ParsedDocument {
            path: "docs/guide.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:doc".to_string(),
            content: "Alpha sentence. Beta sentence.\n\nGamma sentence. Delta sentence.".to_string(),
        };
        let chunker = FixedTextChunker::new(35, 8);

        let chunks = chunker.chunk(&document);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "docs/guide.md#0000");
        assert_eq!(chunks[1].id, "docs/guide.md#0001");
        assert!(chunks[0].content.ends_with("Beta sentence."));
        assert!(chunks[1].start_byte < chunks[0].end_byte);
        assert!(chunks[1].content.contains("Gamma sentence."));
    }

    #[test]
    fn fixed_chunker_preserves_broken_url_lines() {
        let document = ParsedDocument {
            path: "docs/link.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:url".to_string(),
            content: "See https://example.\ncom/path for details.".to_string(),
        };
        let chunker = FixedTextChunker::new(1600, 160);

        let chunks = chunker.chunk(&document);

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("https://example.com/path"));
    }
}
```

- [ ] **Step 2: Run fixed chunker tests and verify RED**

Run:

```powershell
cargo test --features rag tools::rag::ingest::chunking::tests::fixed_chunker
```

Expected: compile failure because `ParsedDocument`, `ChunkingStrategy`, and `FixedTextChunker` do not exist.

- [ ] **Step 3: Add parsed document and document chunk types**

Add to `src/tools/rag/types.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct DocumentChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_hash: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub heading: Option<String>,
    pub content: String,
}
```

- [ ] **Step 4: Implement the chunking trait and fixed chunker**

Add to `src/tools/rag/ingest/chunking.rs`:

```rust
pub trait ChunkingStrategy: Send + Sync {
    fn name(&self) -> &'static str;
    fn target_chars(&self) -> usize;
    fn overlap_chars(&self) -> usize;
    fn chunk(&self, document: &ParsedDocument) -> Vec<DocumentChunk>;
}

#[derive(Debug, Clone)]
pub struct FixedTextChunker {
    target_chars: usize,
    overlap_chars: usize,
}
```

Implement ragent-inspired fixed chunking behavior:

- normalize `\r\n` and `\r` to `\n`
- join obvious URL hard wraps such as `https://example.\ncom`
- use target size and overlap
- prefer blank lines, headings, CJK sentence punctuation, English sentence punctuation followed by whitespace, then whitespace
- generate stable ids as `path#0000`, `path#0001`
- compute `chunk_hash` with `sha256_hex(content.as_bytes())`

- [ ] **Step 5: Write failing markdown-aware chunker tests**

Add these tests to the same test module:

```rust
#[test]
fn markdown_chunker_tracks_heading_metadata() {
    let document = ParsedDocument {
        path: "docs/rag.md".to_string(),
        kind: RetrieveKind::Docs,
        content_hash: "sha256:md".to_string(),
        content: "# RAG\n\nIntro paragraph.\n\n## Retrieval\n\nDetails paragraph.".to_string(),
    };
    let chunker = MarkdownAwareChunker::new(60, 8);

    let chunks = chunker.chunk(&document);

    assert_eq!(chunks[0].heading.as_deref(), Some("RAG"));
    assert!(chunks.iter().any(|chunk| chunk.heading.as_deref() == Some("RAG > Retrieval")));
}

#[test]
fn markdown_chunker_keeps_code_fences_atomic_when_possible() {
    let document = ParsedDocument {
        path: "docs/code.md".to_string(),
        kind: RetrieveKind::Docs,
        content_hash: "sha256:code".to_string(),
        content: "## Example\n\n```rust\nfn searchable_symbol() {}\n```\n\nAfter.".to_string(),
    };
    let chunker = MarkdownAwareChunker::new(120, 8);

    let chunks = chunker.chunk(&document);

    let code_chunks: Vec<_> = chunks
        .iter()
        .filter(|chunk| chunk.content.contains("searchable_symbol"))
        .collect();
    assert_eq!(code_chunks.len(), 1);
    assert!(code_chunks[0].content.contains("```rust"));
    assert!(code_chunks[0].content.contains("```"));
    assert_eq!(code_chunks[0].heading.as_deref(), Some("Example"));
}
```

- [ ] **Step 6: Run markdown tests and verify RED**

Run:

```powershell
cargo test --features rag tools::rag::ingest::chunking::tests::markdown_chunker
```

Expected: compile failure because `MarkdownAwareChunker` does not exist.

- [ ] **Step 7: Implement `MarkdownAwareChunker`**

Implement a structure-aware chunker that:

- segments Markdown into heading, paragraph, fenced code, list, and atomic line blocks
- preserves fenced code blocks as one block when the block fits inside target/max size
- tracks heading path as `H1 > H2 > H3`
- packs blocks up to `target_chars`
- falls back to `FixedTextChunker` for a single block larger than the max budget
- returns deterministic chunk ids, byte offsets, content hash, chunk hash, and heading metadata

Expose constructors:

```rust
impl FixedTextChunker {
    pub fn new(target_chars: usize, overlap_chars: usize) -> Self;
}

impl MarkdownAwareChunker {
    pub fn new(target_chars: usize, overlap_chars: usize) -> Self;
}
```

- [ ] **Step 8: Run chunking verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag tools::rag::ingest::chunking
```

Expected: all chunking tests pass deterministically.

- [ ] **Step 9: Commit**

```powershell
git add src/tools/rag/ingest src/tools/rag/types.rs src/tools/rag/mod.rs
git commit -m "feat: add rag chunking strategies"
```

---

### Task 4: Add Explicit Ingestion Pipeline And Stage Logs

**Files:**
- Create: `src/tools/rag/ingest/pipeline.rs`
- Create: `src/tools/rag/ingest/stages.rs`
- Create: `src/tools/rag/ingest/log.rs`
- Modify: `src/tools/rag/ingest/mod.rs`
- Modify: `src/tools/rag/index.rs`
- Modify: `src/tools/rag/types.rs`
- Test: `tests/rag.rs`
- Test: `tests/cli_index.rs`

- [ ] **Step 1: Write failing ingestion pipeline artifact test**

Add to `tests/rag.rs`:

```rust
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
    assert!(log.lines().all(|line| line.contains("\"schema_version\":1")));
    assert!(log.lines().all(|line| line.contains("\"status\":\"completed\"")));
}
```

- [ ] **Step 2: Run the artifact test and verify RED**

Run:

```powershell
cargo test --features rag --test rag ingestion_pipeline_writes_manifest_and_stage_log
```

Expected: failure because current ingest writes a legacy manifest and no JSONL stage log.

- [ ] **Step 3: Define ingestion context, stage trait, and stage log row**

Add to `src/tools/rag/ingest/pipeline.rs`:

```rust
#[async_trait::async_trait]
pub trait IngestionStage: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()>;
}

pub struct IngestionContext {
    pub run_id: String,
    pub workspace_root: PathBuf,
    pub chunker: Box<dyn ChunkingStrategy>,
    pub discovered_files: Vec<DiscoveredFile>,
    pub parsed_documents: Vec<ParsedDocument>,
    pub chunks: Vec<DocumentChunk>,
    pub embedded_chunks: Vec<EmbeddedChunk>,
    pub logs: Vec<StageLogRow>,
    pub artifact_paths: RagArtifactPaths,
}
```

Add to `src/tools/rag/ingest/log.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageLogRow {
    pub schema_version: u32,
    pub run_id: String,
    pub stage: String,
    pub status: StageStatus,
    pub duration_ms: u128,
    pub input_count: usize,
    pub output_count: usize,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Completed,
    Failed,
    Skipped,
}
```

- [ ] **Step 4: Implement the stage runner**

Add `IngestionPipeline` with a `run` method that:

- executes stages in fixed order
- records duration, input count, output count, message, and error per stage
- appends failed stage log before returning the error
- writes log rows after every stage to `.rove/rag_index_log.jsonl`

Use this fixed stage list:

```rust
ScanWorkspace
ParseReadableFiles
ChunkDocuments
EmbedChunks
PersistIndex
WriteManifestAndLog
```

- [ ] **Step 5: Implement scan and parse stages**

In `src/tools/rag/ingest/stages.rs`, implement:

- `ScanWorkspaceStage`: uses `walkdir`, skips `.git`, `.rove`, `target`, `node_modules`, `.next`, and `dist`, classifies file kind by extension.
- `ParseReadableFilesStage`: reads UTF-8 text files, records `content_hash`, skips unsupported/binary paths discovered by scan policy, returns clear errors only when a path was classified as readable but cannot be read.

Use `DiscoveredFile` in `types.rs`:

```rust
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub kind: RetrieveKind,
}
```

- [ ] **Step 6: Implement chunk, embed, persist, and manifest stages**

In `stages.rs`, implement:

- `ChunkDocumentsStage`: default to `MarkdownAwareChunker::new(1600, 160)`.
- `EmbedChunksStage`: calls the provided `Embedder`, normalizes dimensions, and fails stage-specific on provider errors.
- `PersistIndexStage`: writes LanceDB using ids, paths, kinds, contents, headings, and vectors.
- `WriteManifestAndLogStage`: writes `IndexManifest` and appends the final log row.

Add `EmbeddedChunk`:

```rust
#[derive(Debug, Clone)]
pub struct EmbeddedChunk {
    pub chunk: DocumentChunk,
    pub vector: Vec<f32>,
}
```

- [ ] **Step 7: Route `RagIndex::ingest_workspace` through the pipeline**

Change `RagIndex::ingest_workspace` to construct:

```rust
let pipeline = IngestionPipeline::default_markdown(self.workspace_root.clone(), embedder);
let result = pipeline.run().await?;
Ok(result.chunk_count)
```

Keep the existing method signature so `src/interfaces/cli/index.rs` does not need a behavioral change in this task.

- [ ] **Step 8: Update CLI index test for the richer artifacts**

Extend `deterministic_index_run_writes_manifest` in `tests/cli_index.rs`:

```rust
assert!(tmp.path().join(".rove").join("rag_manifest.json").exists());
assert!(tmp.path().join(".rove").join("rag_index_log.jsonl").exists());
```

- [ ] **Step 9: Run ingestion verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag --test rag ingestion_pipeline_writes_manifest_and_stage_log
cargo test --features rag --test cli_index deterministic_index_run_writes_manifest
```

Expected: PASS and artifacts exist under `.rove`.

- [ ] **Step 10: Commit**

```powershell
git add src/tools/rag src/interfaces/cli/index.rs tests/rag.rs tests/cli_index.rs
git commit -m "feat: add rag ingestion pipeline logs"
```

---

### Task 5: Add Deterministic Query Rewrite Fallback

**Files:**
- Create: `src/tools/rag/rewrite.rs`
- Modify: `src/tools/rag/mod.rs`
- Test: `src/tools/rag/rewrite.rs`

- [ ] **Step 1: Write failing rewrite tests**

Create `src/tools/rag/rewrite.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rewrite_normalizes_whitespace_and_paths() {
        let service = DeterministicQueryRewriteService::default();

        let result = service.rewrite("  src\\tools\\rag.rs   retrieve   docs  ");

        assert_eq!(result.normalized_query, "src/tools/rag.rs retrieve docs");
        assert_eq!(result.sub_queries, vec!["src/tools/rag.rs retrieve docs"]);
        assert_eq!(result.path_hint.as_deref(), Some("src/tools/rag.rs"));
    }

    #[test]
    fn deterministic_rewrite_splits_multi_question_queries() {
        let service = DeterministicQueryRewriteService::default();

        let result = service.rewrite("How index? How retrieve；manifest fallback？\nscore normalization");

        assert_eq!(
            result.sub_queries,
            vec![
                "How index",
                "How retrieve",
                "manifest fallback",
                "score normalization"
            ]
        );
    }

    #[test]
    fn deterministic_rewrite_preserves_quoted_strings_and_caps_subqueries() {
        let service = DeterministicQueryRewriteService::default();

        let result = service.rewrite("\"retrieve_docs\"? alpha? beta? gamma? delta? epsilon?");

        assert_eq!(result.sub_queries.len(), 4);
        assert_eq!(result.sub_queries[0], "\"retrieve_docs\"");
    }
}
```

- [ ] **Step 2: Run rewrite tests and verify RED**

Run:

```powershell
cargo test --features rag tools::rag::rewrite
```

Expected: compile failure because rewrite types do not exist.

- [ ] **Step 3: Implement rewrite service boundary**

Add this interface and deterministic implementation:

```rust
pub trait QueryRewriteService: Send + Sync {
    fn rewrite(&self, query: &str) -> RewriteResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResult {
    pub original_query: String,
    pub normalized_query: String,
    pub sub_queries: Vec<String>,
    pub path_hint: Option<String>,
}

#[derive(Debug, Default)]
pub struct DeterministicQueryRewriteService;
```

The deterministic implementation must:

- trim repeated whitespace to single spaces
- normalize `\` to `/`
- preserve quoted strings as intact split units
- split on newline, semicolon, Chinese semicolon, Chinese question mark, and English question mark when outside quotes
- cap sub-query count to 4
- detect a path hint when any token contains `/` and has an extension-like suffix

- [ ] **Step 4: Run rewrite verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag tools::rag::rewrite
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add src/tools/rag/rewrite.rs src/tools/rag/mod.rs
git commit -m "feat: add deterministic rag query rewrite"
```

---

### Task 6: Add Retrieval Channels And Postprocessors

**Files:**
- Create: `src/tools/rag/retrieve/mod.rs`
- Create: `src/tools/rag/retrieve/channel.rs`
- Create: `src/tools/rag/retrieve/channels.rs`
- Create: `src/tools/rag/retrieve/postprocess.rs`
- Create: `src/tools/rag/retrieve/pipeline.rs`
- Modify: `src/tools/rag/index.rs`
- Modify: `src/tools/rag/mod.rs`
- Modify: `src/tools/rag/types.rs`
- Test: `tests/rag.rs`
- Test: `src/tools/rag/retrieve/postprocess.rs`

- [ ] **Step 1: Write failing postprocessor tests**

Create `src/tools/rag/retrieve/postprocess.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::rag::types::{RetrieveKind, RetrievedChunk};

    fn chunk(id: &str, content: &str, score: f32, source: &str) -> RetrievedChunk {
        RetrievedChunk {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            kind: RetrieveKind::Code,
            content: content.to_string(),
            score,
            source: source.to_string(),
            heading: None,
            chunk_hash: None,
        }
    }

    #[test]
    fn dedupe_keeps_highest_score_and_merges_sources() {
        let processor = DeduplicationPostProcessor;
        let context = RetrievalContext::for_test("authentication token", RetrieveKind::Code, 5);

        let results = processor
            .process(
                &context,
                vec![
                    chunk("src/lib.rs#0000", "same content", 0.2, "vector"),
                    chunk("src/lib.rs#0000", "same content", 0.8, "lexical"),
                ],
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].score, 0.8);
        assert_eq!(results[0].source, "vector+lexical");
    }

    #[test]
    fn score_normalization_orders_mixed_channel_scores() {
        let processor = ScoreNormalizationPostProcessor;
        let context = RetrievalContext::for_test("invoice total", RetrieveKind::Code, 5);

        let results = processor
            .process(
                &context,
                vec![
                    chunk("a#0000", "invoice total exact", 4.0, "lexical"),
                    chunk("b#0000", "near vector match", 0.75, "vector"),
                ],
            )
            .unwrap();

        assert!(results[0].score <= 1.0);
        assert!(results[1].score <= 1.0);
        assert!(results[0].score >= results[1].score);
    }
}
```

- [ ] **Step 2: Run postprocessor tests and verify RED**

Run:

```powershell
cargo test --features rag tools::rag::retrieve::postprocess
```

Expected: compile failure because retrieval context and postprocessors do not exist.

- [ ] **Step 3: Define retrieval context and channel result types**

Add to `src/tools/rag/retrieve/channel.rs`:

```rust
#[derive(Debug, Clone)]
pub struct RetrievalContext {
    pub workspace_root: PathBuf,
    pub original_query: String,
    pub normalized_query: String,
    pub sub_queries: Vec<String>,
    pub kind: RetrieveKind,
    pub limit: usize,
    pub path_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchChannelResult {
    pub name: String,
    pub status: ChannelStatus,
    pub result_count: usize,
    pub duration_ms: u128,
    pub fallback_used: bool,
    pub error: Option<String>,
    pub results: Vec<RetrievedChunk>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelStatus {
    Completed,
    Failed,
    Skipped,
}
```

Add a `RetrievalContext::for_test` helper behind `#[cfg(test)]`.

- [ ] **Step 4: Define search channel and postprocessor traits**

Add to `channel.rs`:

```rust
#[async_trait::async_trait]
pub trait SearchChannel: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> u8;
    fn is_enabled(&self, context: &RetrievalContext) -> bool;

    async fn search(
        &self,
        context: &RetrievalContext,
        index: &RagIndex,
        embedder: &dyn Embedder,
    ) -> anyhow::Result<SearchChannelResult>;
}
```

Add to `postprocess.rs`:

```rust
pub trait SearchResultPostProcessor: Send + Sync {
    fn name(&self) -> &'static str;
    fn order(&self) -> u8;
    fn is_enabled(&self, context: &RetrievalContext) -> bool;

    fn process(
        &self,
        context: &RetrievalContext,
        results: Vec<RetrievedChunk>,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;
}
```

- [ ] **Step 5: Implement dedupe, score normalization, and no-op rerank**

Implement:

- `DeduplicationPostProcessor`: dedupe by id, then `RetrievedChunk::chunk_hash`, then normalized content hash; preserve highest score; merge sources into deterministic `vector+lexical+path` order.
- `ScoreNormalizationPostProcessor`: normalize per source into `0.0..=1.0`, sort descending, truncate to `context.limit`.
- `NoopRerankPostProcessor`: final boundary that returns input unchanged and truncates to `context.limit`.

- [ ] **Step 6: Run postprocessor tests and verify GREEN**

Run:

```powershell
cargo test --features rag tools::rag::retrieve::postprocess
```

Expected: PASS.

- [ ] **Step 7: Write failing retrieval channel integration tests**

Add to `tests/rag.rs`:

```rust
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
    std::fs::write(tmp.path().join("src").join("auth.rs"), "pub fn shared_name() {}").unwrap();
    std::fs::write(tmp.path().join("src").join("billing.rs"), "pub fn shared_name() {}").unwrap();

    let index = RagIndex::new(tmp.path().to_path_buf());
    let embedder = DeterministicEmbedder;
    index.ingest_workspace(&embedder).await.unwrap();

    let hits = index
        .retrieve(&embedder, RetrieveKind::Code, "src/billing.rs shared_name", 3)
        .await
        .unwrap();

    assert_eq!(hits[0].path, "src/billing.rs");
    assert!(hits[0].source.contains("path"));
}
```

- [ ] **Step 8: Run retrieval integration tests and verify RED**

Run:

```powershell
cargo test --features rag --test rag manifest_fallback_retrieval_still_works_without_lancedb lexical_channel_ranks_exact_symbol_matches path_scoped_channel_prefers_matching_path_hint
```

Expected: at least the source metadata and path-scoped assertions fail before channels exist.

- [ ] **Step 9: Implement vector, lexical, and path-scoped channels**

In `src/tools/rag/retrieve/channels.rs`, implement:

- `VectorSearchChannel`: uses LanceDB first and manifest vector fallback second; if LanceDB open/query fails, returns manifest results and sets `fallback_used = true`.
- `LexicalSearchChannel`: loads manifest chunks and scores token overlap plus exact identifier/path token matches.
- `PathScopedSearchChannel`: enabled only when `context.path_hint.is_some()`; filters manifest chunks to matching path substrings and scores the scoped set lexically.

Expose storage helpers in `index.rs` for channels:

```rust
impl RagIndex {
    pub(crate) async fn load_manifest(&self) -> anyhow::Result<Option<IndexManifest>>;
    pub(crate) async fn search_lancedb(
        &self,
        kind: RetrieveKind,
        query_vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;
}
```

- [ ] **Step 10: Implement `RetrievalPipeline` and route `RagIndex::retrieve` through it**

In `retrieve/pipeline.rs`, implement the ragent-inspired flow:

```text
DeterministicQueryRewriteService
  -> build RetrievalContext
  -> run enabled channels ordered by priority
  -> merge channel results
  -> DeduplicationPostProcessor
  -> ScoreNormalizationPostProcessor
  -> NoopRerankPostProcessor
```

Keep `RagIndex::retrieve` as:

```rust
pub async fn retrieve(
    &self,
    embedder: &dyn Embedder,
    kind: RetrieveKind,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<RetrievedChunk>>;
```

- [ ] **Step 11: Run retrieval verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag tools::rag::retrieve
cargo test --features rag --test rag
```

Expected: PASS. Existing retrieval tests and new channel tests both pass.

- [ ] **Step 12: Commit**

```powershell
git add src/tools/rag tests/rag.rs
git commit -m "feat: add rag retrieval channels"
```

---

### Task 7: Update Tool Output And Add Pure Retrieval Eval CLI

**Files:**
- Create: `src/tools/rag/eval.rs`
- Modify: `src/tools/rag/mod.rs`
- Modify: `src/tools/rag/retrieve/pipeline.rs`
- Modify: `src/bin/rove-index.rs`
- Modify: `src/interfaces/cli/index.rs`
- Modify: `tests/rag.rs`
- Modify: `tests/cli_index.rs`

- [ ] **Step 1: Write failing structured tool output test**

Add to `tests/rag.rs`:

```rust
#[tokio::test]
async fn rag_tool_output_contains_query_metadata_and_results() {
    use rove::core::types::{ApprovalPolicy, ToolContext};
    use rove::core::workspace::Workspace;
    use rove::tools::rag::RagRetrieveTool;
    use rove::tools::traits::Tool;
    use tokio_util::sync::CancellationToken;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("README.md"), "# RAG\n\nretrieval eval report").unwrap();

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
        .execute(serde_json::json!({"query": "retrieval eval", "limit": 2}), &ctx)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&output.content).unwrap();

    assert_eq!(json["query"], "retrieval eval");
    assert_eq!(json["normalized_query"], "retrieval eval");
    assert_eq!(json["kind"], "docs");
    assert_eq!(json["limit"], 2);
    assert!(json["results"][0]["source"].as_str().unwrap().contains("lexical"));
}
```

- [ ] **Step 2: Run tool output test and verify RED**

Run:

```powershell
cargo test --features rag --test rag rag_tool_output_contains_query_metadata_and_results
```

Expected: fail because the tool currently returns only an array of hits.

- [ ] **Step 3: Return structured tool output**

Change `RagRetrieveTool::execute` to return:

```json
{
  "query": "retrieval eval",
  "normalized_query": "retrieval eval",
  "kind": "docs",
  "limit": 2,
  "results": []
}
```

Use `DeterministicQueryRewriteService` to compute `normalized_query` and keep result objects with `id`, `path`, `score`, `source`, `heading`, and `content`.

- [ ] **Step 4: Write failing eval report test**

Add to `tests/cli_index.rs`:

```rust
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
    assert!(report["channels"].as_array().unwrap().iter().any(|channel| channel["name"] == "lexical"));
    assert_eq!(report["results"][0]["rank"], 1);
    assert!(report.get("llm_output").is_none());
}
```

- [ ] **Step 5: Run eval test and verify RED**

Run:

```powershell
cargo test --features rag --test cli_index eval_run_writes_report_without_llm_generation
```

Expected: compile failure because `IndexOptions` has no eval fields and `eval.rs` does not exist.

- [ ] **Step 6: Extend CLI args and index options**

Update `src/bin/rove-index.rs`:

```rust
#[arg(long)]
eval: Option<String>,

#[arg(long, default_value = "docs")]
kind: String,

#[arg(long, default_value_t = 8)]
limit: usize,
```

Update `IndexOptions` in `src/interfaces/cli/index.rs`:

```rust
pub struct IndexOptions {
    pub cwd: Option<PathBuf>,
    pub deterministic: bool,
    pub embedding_model: Option<String>,
    pub eval_query: Option<String>,
    pub eval_kind: Option<String>,
    pub eval_limit: usize,
}
```

Update all existing tests and call sites to pass the new fields.

- [ ] **Step 7: Implement eval report generation**

Create `src/tools/rag/eval.rs` with:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievalEvalReport {
    pub schema_version: u32,
    pub query: String,
    pub normalized_query: String,
    pub kind: RetrieveKind,
    pub limit: usize,
    pub duration_ms: u128,
    pub channels: Vec<EvalChannelSummary>,
    pub results: Vec<EvalResult>,
    pub artifact_path: String,
}
```

Add:

```rust
pub async fn run_retrieval_eval(
    index: &RagIndex,
    embedder: &dyn Embedder,
    kind: RetrieveKind,
    query: &str,
    limit: usize,
) -> anyhow::Result<RetrievalEvalReport>;

pub async fn write_eval_report(
    workspace_root: &Path,
    report: &RetrievalEvalReport,
) -> anyhow::Result<PathBuf>;
```

The report must include channel names, statuses, result counts, timings, fallback metadata, ranked results, and content previews. It must not call any model or final LLM generation path.

- [ ] **Step 8: Route `rove-index --eval` through eval**

In `src/interfaces/cli/index.rs`:

- if `eval_query` is `None`, keep the current indexing behavior
- if `eval_query` is `Some`, do not re-index automatically
- build deterministic or OpenAI embedder using the same rule as indexing
- parse `eval_kind` as `docs` or `code`, returning an actionable error for any other value
- print a concise summary with query, report path, channel counts, and top result paths

Add:

```rust
pub fn format_eval_result(report: &RetrievalEvalReport, path: &Path) -> String;
```

- [ ] **Step 9: Run eval verification**

Run:

```powershell
cargo fmt --all --check
cargo test --features rag --test rag rag_tool_output_contains_query_metadata_and_results
cargo test --features rag --test cli_index eval_run_writes_report_without_llm_generation
```

Expected: PASS.

- [ ] **Step 10: Commit**

```powershell
git add src/tools/rag src/bin/rove-index.rs src/interfaces/cli/index.rs tests/rag.rs tests/cli_index.rs
git commit -m "feat: add rag retrieval eval"
```

---

### Task 8: Preserve Feature Gate And Run Full Verification

**Files:**
- Modify only if a previous task exposed issues: `src/tools/rag_stub.rs`
- Modify only if a previous task exposed issues: `src/tools/mod.rs`
- Test: `tests/rag_default.rs`
- Test: `tests/rag.rs`
- Test: `tests/cli_index.rs`

- [ ] **Step 1: Verify no-feature stub behavior**

Run:

```powershell
cargo test --test rag_default
cargo test --test cli_index index_run_explains_when_rag_feature_is_disabled
```

Expected: PASS. Default builds must compile without LanceDB and must still return the helpful `requires the rag feature` message.

- [ ] **Step 2: Verify RAG-enabled behavior**

Run:

```powershell
cargo test --features rag --test rag
cargo test --features rag --test cli_index
```

Expected: PASS. RAG-enabled indexing, retrieval, manifest fallback, stage logs, channels, postprocessors, and eval report tests all pass.

- [ ] **Step 3: Run workspace formatting and default tests**

Run:

```powershell
cargo fmt --all --check
cargo test
```

Expected: PASS. Default `cargo test` must not require `--features rag`.

- [ ] **Step 4: Run all-features clippy**

Run:

```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 5: Run a local RAG smoke command**

Run:

```powershell
cargo run --features rag --bin rove-index -- --deterministic -C .
cargo run --features rag --bin rove-index -- --deterministic -C . --eval "structured model events" --kind docs --limit 5
```

Expected: the first command prints an indexed chunk count and writes `.rove/rag_manifest.json`, `.rove/rag_index_log.jsonl`, and `.rove/rag.lancedb`; the second command writes a JSON report under `.rove/rag_eval/`.

- [ ] **Step 6: Inspect artifact shape**

Run:

```powershell
Get-Content .rove\rag_index_log.jsonl -TotalCount 6
Get-ChildItem .rove\rag_eval | Sort-Object LastWriteTime -Descending | Select-Object -First 1
```

Expected: stage logs contain all six ingestion stages with `schema_version: 1`; the eval directory contains the newest report file.

- [ ] **Step 7: Commit final verification adjustments**

Only commit if this task required code or test changes:

```powershell
git add src tests Cargo.toml Cargo.lock
git commit -m "test: verify rag pipeline hardening"
```

---

## Acceptance Mapping

- Spec item 1, split focused modules: Task 1.
- Existing public tool names still work: Tasks 1 and 7.
- Default no-feature build uses `rag_stub.rs`: Task 8.
- RAG-enabled `rove-index` and LanceDB storage remain: Tasks 1, 4, and 8.
- Explicit ingestion stages and per-stage logs: Task 4.
- Fixed and Markdown-aware chunking strategies: Task 3.
- Vector and lexical retrieval channels through a shared boundary: Task 6.
- Path-scoped retrieval channel: Task 6.
- Dedupe and score normalization postprocessors: Task 6.
- Manifest fallback remains functional and tested: Tasks 2 and 6.
- Deterministic query rewrite fallback: Task 5.
- Pure retrieval eval/report path without final LLM generation: Task 7.
- Local-first deterministic tests: Tasks 2 through 8.
- Formatting, clippy, default tests, and `--features rag` tests: Task 8.

## Self-Review Notes

- No implementation code should be changed before review; this file is the handoff artifact.
- The plan keeps all LanceDB-dependent code behind the existing `rag` feature.
- The plan keeps `src/tools/rag_stub.rs` as the default build implementation.
- The plan retains deterministic embedding for all tests and local eval.
- The plan borrows ragent patterns as interfaces and flow boundaries, not its Spring, Milvus, Redis, database task, or intent-tree architecture.
- The plan adds `sha2` only as an optional `rag` dependency so default builds remain lightweight.
