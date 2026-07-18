# RAG Pipeline Hardening Design - 2026-05-24

本文定义 `rove` 下一阶段 RAG 能力的设计方向。它吸收
`D:/Study/project/agent/ragent` 的显式 Pipeline、节点化 ingestion、多通道检索、
后处理链、query rewrite fallback 和 retrieval eval 思路，但不照搬其 Java/Spring、
Milvus、数据库工作流或企业级意图树。

这是一份设计 spec，不是实现计划。后续 `/goal` 应基于本文再生成
`docs/plans/YYYY-MM-DD-rag-pipeline-hardening.md`。

## Suggested /goal Objective

后续可以使用这个目标启动开发：

> Based on `docs/design/2026-05-24-rag-pipeline-hardening-design.md`, implement Phase 3A RAG Pipeline Hardening for rove: split the current RAG indexer into explicit ingestion stages, add chunking strategies, add durable indexing logs/manifests under `.rove`, introduce lightweight retrieval channels and postprocessors, and add a pure retrieval eval/report path, while preserving the existing `rag` feature gate, deterministic tests, LanceDB storage, and manifest fallback.

## Current State

`rove` 当前已经有一个可用但较薄的 RAG 原型：

- `src/tools/mod.rs` 通过 `#[cfg(feature = "rag")]` 切换真实 RAG 和 `rag_stub.rs`。
- `src/tools/rag.rs` 包含 `RagIndex`、`RetrieveKind`、`RetrievedChunk`、
  `DeterministicEmbedder`、`OpenAiEmbedder` 和 `RagRetrieveTool`。
- `RagIndex::ingest_workspace()` 负责扫描文件、简单切分、embedding、写 LanceDB、
  写 `.rove/rag_manifest.json`。
- `retrieve()` 先查 LanceDB，空结果时 fallback 到 manifest。
- 当前 chunking 是简单的 `chunk_text(content, 1600)`，没有结构感知、overlap、
  chunk id、content hash、per-stage log 或 eval 输出。
- `rove-index` binary 已存在，并且通过 `required-features = ["rag"]` 约束。

这个状态适合继续演进，但如果继续把所有逻辑堆在 `src/tools/rag.rs`，后续会很快变成
不可测试的大文件。下一阶段的重点是把 RAG 从“一个工具文件”整理成“一个可观察、可测试、
可扩展的 pipeline”。

## Design Goals

1. **显式 Pipeline**
   RAG ingestion 和 retrieval 都要有明确阶段、输入输出和错误边界。调用方不应该依赖一个
   巨大的 `ingest_workspace()` 去推断里面发生了什么。

2. **本地优先**
   继续保留 deterministic embedder、manifest fallback 和本地文件系统 artifact。OpenAI
   embedding 与 LanceDB 是增强路径，不是测试和离线使用的硬依赖。

3. **可观察**
   每次索引要能回答：扫描了哪些文件、跳过了哪些文件、生成了多少 chunk、embedding 失败
   在哪里、最终写入了哪些 index artifact。

4. **检索可评估**
   在让 LLM 使用 RAG 结果之前，先提供纯 retrieval eval/report。RAG 质量不能只靠最终回答
   主观判断。

5. **轻量借鉴 ragent**
   借鉴其 pipeline、channel、postprocessor、fallback 和 eval 设计模式；暂不引入完整意图树、
   多租户知识库、Redis、数据库任务表或 MCP 意图分类。

6. **保持 rove 边界**
   RAG 仍然作为 Tools 层能力接入，不让 `core::engine` 直接依赖 LanceDB、chunker 或
   embedding provider。

## ragent Ideas To Adopt

| ragent idea | What it solves | rove adaptation |
|---|---|---|
| `StreamChatPipeline` staged context | 避免 orchestration 逻辑散落在 service 中 | RAG ingestion/retrieval 使用显式 context 和 stage |
| `IngestionNode` chain | 每个处理节点可独立记录成功、失败和耗时 | 用轻量 `IngestionStage`，写 `.rove/rag_index_log.jsonl` |
| fixed + structure-aware chunkers | 简单切分会破坏 Markdown/code 语义 | 先实现 fixed 和 Markdown-aware 两种 strategy |
| `SearchChannel` | 检索策略可组合、可按条件启用 | 增加 vector、lexical、path-scoped channel |
| postprocessor chain | 检索结果去重、排序、rerank 边界清晰 | 先做 dedupe + score normalization + noop rerank interface |
| query rewrite fallback | LLM rewrite 不可用时仍能稳定工作 | 先做 deterministic normalization/split，LLM rewrite 后置 |
| retrieval eval endpoint | 单独评估检索质量 | 增加 CLI/report path，不走最终 LLM 生成 |
| ambiguity guidance | 意图不确定时要求澄清 | 后续映射到 `request_input`，Phase 3A 不实现完整意图树 |

## Non-Goals

Phase 3A 不做以下内容：

- 不复制 ragent 的 Spring Boot service/controller 架构。
- 不引入 Milvus、Redis、数据库任务表或分布式索引队列。
- 不做完整 KB/MCP/SYSTEM intent tree。
- 不把 RAG memory、conversation summary、prompt scene selection 做成聊天产品级系统。
- 不要求所有 provider 都支持真实 embedding API 才能运行测试。
- 不把 RAG 逻辑放进 `core::engine`。
- 不在这一阶段做 tree-sitter 级代码语义 chunking；可以预留接口。

## Proposed Module Layout

当前 `src/tools/rag.rs` 应拆成目录模块。`src/tools/mod.rs` 的 `pub mod rag;`
入口保持不变，feature gate 行为也保持不变。

建议结构：

```text
src/tools/rag/
├── mod.rs              # public exports and RagRetrieveTool wiring
├── types.rs            # RetrieveKind, RetrievedChunk, chunk/index/eval structs
├── embed.rs            # Embedder, DeterministicEmbedder, OpenAiEmbedder
├── index.rs            # LanceDB + manifest storage adapter
├── ingest/
│   ├── mod.rs
│   ├── pipeline.rs     # IngestionPipeline and IngestionContext
│   ├── stages.rs       # scan, parse, chunk, embed, persist stages
│   ├── chunking.rs     # ChunkingStrategy implementations
│   └── log.rs          # per-stage indexing log writer
├── retrieve/
│   ├── mod.rs
│   ├── pipeline.rs     # RetrievalPipeline and RetrievalContext
│   ├── channel.rs      # SearchChannel trait and channel result type
│   ├── channels.rs     # vector, lexical, path-scoped channels
│   └── postprocess.rs  # dedupe, score normalization, noop rerank
├── rewrite.rs          # deterministic query normalization and split
└── eval.rs             # pure retrieval eval/report helpers
```

`src/tools/rag_stub.rs` remains a separate no-feature implementation. It should keep the same public
`RagRetrieveTool::code/docs` API so default builds do not pull LanceDB dependencies.

## Ingestion Design

### Pipeline Shape

Ingestion should be represented as explicit stages:

```text
ScanWorkspace
  -> ParseReadableFiles
  -> ChunkDocuments
  -> EmbedChunks
  -> PersistIndex
  -> WriteManifestAndLog
```

Each stage receives and mutates an `IngestionContext`. The context should contain:

- workspace root
- include/exclude policy
- selected chunking strategy
- discovered files
- parsed documents
- chunks
- embeddings
- stage logs
- output artifact paths

The stage boundary is more important than the exact trait name. A minimal Rust shape is enough:

```rust
#[async_trait::async_trait]
pub trait IngestionStage: Send + Sync {
    fn name(&self) -> &'static str;

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()>;
}
```

### Artifacts

Write artifacts under `.rove`:

```text
.rove/
├── rag.lancedb/
├── rag_manifest.json
├── rag_index_log.jsonl
└── rag_eval/
    └── <timestamp-or-run-id>.json
```

`rag_manifest.json` should evolve from a raw fallback list into an index manifest with enough metadata
to make future incremental indexing possible:

```json
{
  "schema_version": 1,
  "workspace_root": "D:/Study/project/agent/rove",
  "embedding": {
    "provider": "deterministic",
    "model": "deterministic-64",
    "dims": 64
  },
  "chunking": {
    "strategy": "markdown-aware",
    "target_chars": 1600,
    "overlap_chars": 160
  },
  "files": [
    {
      "path": "docs/04-架构与路线图.md",
      "kind": "docs",
      "content_hash": "sha256:...",
      "chunk_count": 5,
      "indexed_at": "2026-05-24T00:00:00Z"
    }
  ],
  "chunks": [
    {
      "id": "docs/04-架构与路线图.md#0001",
      "path": "docs/04-架构与路线图.md",
      "kind": "docs",
      "content_hash": "sha256:...",
      "chunk_hash": "sha256:...",
      "start_byte": 0,
      "end_byte": 1320,
      "heading": "M3 RAG retriever",
      "content": "..."
    }
  ]
}
```

Phase 3A keeps normalized vectors in `rag_manifest.json`, matching the current fallback behavior.
If the manifest later becomes too large, a separate spec can split vectors into another fallback file.
The required behavior for this phase is that retrieval still works without successfully opening LanceDB
when the manifest fallback is available.

### Stage Logs

`rag_index_log.jsonl` should be append-only. Each row should describe one stage outcome:

```json
{
  "schema_version": 1,
  "run_id": "01J...",
  "stage": "ChunkDocuments",
  "status": "completed",
  "duration_ms": 37,
  "input_count": 12,
  "output_count": 48,
  "message": "chunked 12 documents into 48 chunks"
}
```

For failures:

```json
{
  "schema_version": 1,
  "run_id": "01J...",
  "stage": "EmbedChunks",
  "status": "failed",
  "duration_ms": 821,
  "input_count": 48,
  "output_count": 17,
  "message": "embedding failed for docs/example.md#0003",
  "error": "embedding request failed: 401 Unauthorized"
}
```

This mirrors ragent's `NodeLog` idea without requiring database persistence.

## Chunking Design

Introduce a `ChunkingStrategy` abstraction:

```rust
pub trait ChunkingStrategy: Send + Sync {
    fn name(&self) -> &'static str;

    fn chunk(&self, document: &ParsedDocument) -> Vec<DocumentChunk>;
}
```

Phase 3A should include two strategies:

1. **FixedTextChunker**
   - Normalize line endings.
   - Preserve URLs split by accidental line breaks where practical.
   - Use target size and overlap.
   - Prefer boundaries at blank lines, headings, CJK punctuation, English sentence punctuation, then whitespace.
   - Keep deterministic behavior for tests.

2. **MarkdownAwareChunker**
   - Preserve code fences as atomic blocks when possible.
   - Track current heading path.
   - Treat Markdown headings, paragraphs, fenced code blocks and list blocks as packable units.
   - Attach heading metadata to each chunk.
   - Fall back to fixed chunking when a block exceeds max size.

Code-aware chunking is intentionally deferred. The markdown-aware chunker is still useful for Rust,
TypeScript and config files because it avoids breaking fenced examples and docs.

## Retrieval Design

### Pipeline Shape

Retrieval should be an explicit pipeline:

```text
NormalizeAndSplitQuery
  -> BuildRetrievalContext
  -> RunEnabledSearchChannels
  -> DedupeResults
  -> NormalizeScores
  -> OptionalRerank
  -> FormatToolOutput or EvalReport
```

Minimal context:

```rust
pub struct RetrievalContext {
    pub workspace_root: PathBuf,
    pub original_query: String,
    pub normalized_query: String,
    pub sub_queries: Vec<String>,
    pub kind: RetrieveKind,
    pub limit: usize,
    pub path_hint: Option<String>,
}
```

### Search Channels

Borrow ragent's `SearchChannel` idea, but keep the first version small:

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
    ) -> anyhow::Result<Vec<RetrievedChunk>>;
}
```

Phase 3A channels:

- **VectorSearchChannel**
  Uses LanceDB first and manifest vector fallback second.

- **LexicalSearchChannel**
  Uses token overlap/BM25-like local scoring from manifest chunks. It improves deterministic tests and
  helps queries that mention exact symbols, filenames, error names or config keys.

- **PathScopedSearchChannel**
  Enabled when the query or explicit argument includes a file/path hint. It scores only chunks from
  matching paths, then applies lexical/vector scoring inside that scope.

Channel execution should be concurrent where it is simple to do so, but concurrency is not the primary
goal. The important boundary is that each channel owns one retrieval strategy and reports its source.

### Postprocessors

Postprocessors should run after all channels return:

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

Phase 3A processors:

- **DeduplicationPostProcessor**
  Dedupes by chunk id first, then chunk hash, then normalized content hash. Preserve the highest score
  and strongest source metadata.

- **ScoreNormalizationPostProcessor**
  Normalizes vector/lexical/path scores into a comparable range and records channel contributions.

- **NoopRerankPostProcessor**
  Provides the interface boundary for future rerankers but does not call an external model yet.

The final `RetrievedChunk` should include enough metadata for debugging:

```rust
pub struct RetrievedChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content: String,
    pub score: f32,
    pub source: String,
    pub heading: Option<String>,
}
```

## Query Rewrite Design

Phase 3A should add deterministic rewrite only:

- trim repeated whitespace
- normalize path separators to `/`
- preserve quoted strings and code identifiers
- split obvious multi-question queries on newline, semicolon, CJK question mark and English question mark
- cap sub-query count to a small number such as 4

No LLM rewrite in Phase 3A. The interface may allow future LLM rewrite:

```rust
pub trait QueryRewriteService: Send + Sync {
    fn rewrite(&self, query: &str) -> RewriteResult;
}
```

This follows ragent's rule: LLM rewrite is useful, but local deterministic fallback must be the baseline.

## Tool Output And Prompt Boundary

`retrieve_code` and `retrieve_docs` should still return JSON-like tool output. Do not make the engine
assemble RAG prompts in Phase 3A.

Recommended output shape:

```json
{
  "query": "structured model events",
  "normalized_query": "structured model events",
  "kind": "code",
  "limit": 5,
  "results": [
    {
      "id": "src/models/traits.rs#0001",
      "path": "src/models/traits.rs",
      "score": 0.87,
      "source": "vector+lexical",
      "heading": null,
      "content": "..."
    }
  ]
}
```

Prompt formatting can become a later `RagPromptService` if RAG starts feeding a dedicated answer mode.
For now, tools return structured evidence and the existing agent loop decides how to use it.

## Retrieval Eval Design

Add a pure eval/report path that runs retrieval and writes the result under `.rove/rag_eval/`.

Required CLI shape:

```text
rove-index --eval "query text" --kind docs --limit 8
```

The required behavior is:

- run query normalization/split
- run enabled channels
- run postprocessors
- print a concise human-readable summary
- write JSON report with query, chunks, scores, channels, timings and artifact paths
- avoid final LLM generation

Example report:

```json
{
  "schema_version": 1,
  "query": "How does rove handle structured model events?",
  "normalized_query": "How does rove handle structured model events?",
  "kind": "docs",
  "limit": 8,
  "duration_ms": 42,
  "channels": [
    {"name": "vector", "status": "completed", "result_count": 8, "duration_ms": 21},
    {"name": "lexical", "status": "completed", "result_count": 6, "duration_ms": 5}
  ],
  "results": [
    {
      "rank": 1,
      "id": "docs/RAGENT-STREAM-MODEL-NOTES-2026-05-24.md#0002",
      "path": "docs/RAGENT-STREAM-MODEL-NOTES-2026-05-24.md",
      "score": 0.91,
      "source": "vector+lexical",
      "heading": "Model stream",
      "content_preview": "..."
    }
  ]
}
```

This is the RAG equivalent of a unit testable diagnostic endpoint. It should exist before changing
answer generation behavior.

## Error Handling

Errors should be stage-specific and observable:

- unreadable files should be logged and skipped unless all files fail
- unsupported binary files should be skipped with reason
- embedding provider failures should fail the embedding stage and record partial counts
- LanceDB write failures should return an error; manifest write should not be silently skipped
- LanceDB read failures during retrieval should fall back to manifest and record that fallback in eval metadata
- malformed manifest should surface a clear error instead of returning empty results silently

Tool execution should continue to map failures into `ToolError::ExecutionFailed` with actionable messages.

## Testing Strategy

Phase 3A should be test-driven. Required test coverage:

- chunking fixed-size behavior with overlap and stable boundaries
- Markdown-aware chunking preserves heading metadata and code fences
- ingestion pipeline writes manifest and stage logs
- deterministic embedder keeps retrieval tests local
- retrieval runs vector fallback and lexical channel deterministically
- dedupe keeps the highest scoring duplicate
- score normalization produces stable ordering for mixed channels
- no-feature `rag_stub.rs` still compiles and returns the current helpful message
- `--features rag` tests cover LanceDB path where practical
- eval report contains query, channels, timings and ranked results

Verification commands for implementation plans should include:

```text
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features rag
```

If LanceDB tests are slow or flaky on CI, the implementation plan may split storage adapter tests from
deterministic manifest/channel tests, but the local `--features rag` path must still be exercised before
claiming completion.

## Acceptance Criteria

The implementation of this spec is complete when:

1. `src/tools/rag.rs` has been split into focused RAG modules or an equivalent small-file structure.
2. Existing public tool names `retrieve_code` and `retrieve_docs` still work.
3. Default builds without `--features rag` still compile and use `rag_stub.rs`.
4. RAG-enabled builds still support `rove-index` and LanceDB storage.
5. Ingestion is represented as explicit stages with per-stage logs.
6. At least fixed and Markdown-aware chunking strategies exist and are tested.
7. Retrieval uses at least vector and lexical channels through a shared channel boundary.
8. Retrieval results pass through dedupe and score normalization postprocessors.
9. Manifest fallback remains functional and is covered by tests.
10. A pure retrieval eval/report path exists and does not call the final LLM.
11. The new design remains local-first and deterministic-test friendly.
12. Formatting, clippy and tests pass under the commands listed above.

## Recommended Implementation Phases

### Phase 3A.1: Split And Preserve Behavior

Move the current `src/tools/rag.rs` implementation into focused modules without changing behavior.
This lowers risk and creates places for the new pipeline code.

### Phase 3A.2: Ingestion Pipeline And Logs

Introduce `IngestionPipeline`, `IngestionContext`, stage logs and a richer manifest. Keep current
`rove-index` behavior working.

### Phase 3A.3: Chunking Strategies

Replace the simple line accumulator with fixed-size and Markdown-aware strategies. Keep default behavior
conservative and deterministic.

### Phase 3A.4: Retrieval Channels And Postprocessors

Introduce `RetrievalPipeline`, vector/lexical/path channels, dedupe and score normalization.

### Phase 3A.5: Retrieval Eval

Add a pure eval/report command path and tests that validate the report shape and retrieval metadata.

## Later Phases

After Phase 3A is stable, possible follow-up specs:

- **Phase 3B: RAG Prompt Integration**
  Introduce `RagPromptService` or a context formatting boundary if `rove` needs a dedicated answer mode.

- **Phase 3C: LLM Query Rewrite And Rerank**
  Add optional LLM rewrite and rerank providers with deterministic fallback.

- **Phase 3D: Code-Aware Chunking**
  Add tree-sitter or language-aware code chunking for symbols, tests and module boundaries.

- **Phase 3E: Ambiguity Guidance**
  Map ambiguous retrieval or intent uncertainty to `request_input`, borrowing ragent's guidance idea
  without adopting a full enterprise intent tree.

## Design Decision

The core decision is:

> `rove` should treat RAG as a staged, observable local pipeline. It should borrow ragent's design
> patterns, not its deployment architecture.

The first serious RAG goal should therefore be Phase 3A: harden indexing, chunking, retrieval and eval
before integrating deeper prompt generation or intent classification.
