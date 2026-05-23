use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use futures::TryStreamExt;
use lancedb::database::CreateTableMode;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde_json::Value;
use walkdir::WalkDir;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

const TABLE_NAME: &str = "chunks";
const EMBEDDING_DIMS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrieveKind {
    Code,
    Docs,
}

impl RetrieveKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Docs => "docs",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub path: String,
    pub content: String,
    pub score: f32,
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

#[derive(Debug, Default)]
pub struct DeterministicEmbedder;

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(deterministic_embedding(text))
    }
}

pub struct OpenAiEmbedder {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl OpenAiEmbedder {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
        });
        let response = self
            .client
            .post(format!("{}/embeddings", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("embedding request failed: {}", response.text().await?);
        }
        let json: serde_json::Value = response.json().await?;
        let values = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("embedding response missing data[0].embedding"))?;
        Ok(values
            .iter()
            .map(|value| value.as_f64().unwrap_or_default() as f32)
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct RagIndex {
    workspace_root: PathBuf,
}

impl RagIndex {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub async fn ingest_workspace(&self, embedder: &dyn Embedder) -> anyhow::Result<usize> {
        let chunks = self.collect_chunks()?;
        if chunks.is_empty() {
            return Ok(0);
        }

        let mut paths = Vec::new();
        let mut kinds = Vec::new();
        let mut contents = Vec::new();
        let mut vectors = Vec::new();

        for chunk in &chunks {
            paths.push(chunk.path.clone());
            kinds.push(chunk.kind.as_str().to_string());
            contents.push(chunk.content.clone());
            vectors.push(embedder.embed(&chunk.content).await?);
        }

        self.write_lancedb(&paths, &kinds, &contents, &vectors)
            .await?;
        self.write_manifest(&chunks, &vectors).await?;
        Ok(chunks.len())
    }

    pub async fn retrieve(
        &self,
        embedder: &dyn Embedder,
        kind: RetrieveKind,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        let query_vector = embedder.embed(query).await?;
        let mut hits = self
            .retrieve_lancedb(kind, query, &query_vector, limit)
            .await?;
        if hits.is_empty() {
            hits = self
                .retrieve_manifest(kind, query, &query_vector, limit)
                .await?;
        }
        Ok(hits)
    }

    fn collect_chunks(&self) -> anyhow::Result<Vec<ChunkRecord>> {
        let mut chunks = Vec::new();
        for entry in WalkDir::new(&self.workspace_root)
            .into_iter()
            .filter_entry(|entry| !is_ignored(entry.path()))
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(kind) = classify_path(entry.path()) else {
                continue;
            };
            let content = std::fs::read_to_string(entry.path())?;
            let path = entry
                .path()
                .strip_prefix(&self.workspace_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            for chunk in chunk_text(&content, 1600) {
                chunks.push(ChunkRecord {
                    path: path.clone(),
                    kind,
                    content: chunk,
                });
            }
        }
        Ok(chunks)
    }

    async fn write_lancedb(
        &self,
        paths: &[String],
        kinds: &[String],
        contents: &[String],
        vectors: &[Vec<f32>],
    ) -> anyhow::Result<()> {
        let db_dir = self.db_dir();
        tokio::fs::create_dir_all(&db_dir).await?;
        let db = lancedb::connect(db_dir.to_str().unwrap()).execute().await?;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("path", DataType::Utf8, false),
            Field::new("kind", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    EMBEDDING_DIMS as i32,
                ),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from_iter_values(0..paths.len() as i32)),
                Arc::new(StringArray::from(paths.to_vec())),
                Arc::new(StringArray::from(kinds.to_vec())),
                Arc::new(StringArray::from(contents.to_vec())),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        vectors.iter().map(|vector| {
                            let fixed = normalize_dims(vector);
                            Some(fixed.into_iter().map(Some).collect::<Vec<_>>())
                        }),
                        EMBEDDING_DIMS as i32,
                    ),
                ),
            ],
        )?;
        db.create_table(TABLE_NAME, batch)
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await?;
        Ok(())
    }

    async fn retrieve_lancedb(
        &self,
        kind: RetrieveKind,
        query: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        let db_dir = self.db_dir();
        if !db_dir.exists() {
            return Ok(Vec::new());
        }
        let db = lancedb::connect(db_dir.to_str().unwrap()).execute().await?;
        let table = match db.open_table(TABLE_NAME).execute().await {
            Ok(table) => table,
            Err(_) => return Ok(Vec::new()),
        };
        let fixed_query = normalize_dims(query_vector);
        let batches = table
            .query()
            .nearest_to(fixed_query.as_slice())?
            .limit((limit * 4).max(limit))
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;

        let mut hits = Vec::new();
        for batch in batches {
            let paths = batch.column_by_name("path").unwrap().as_string::<i32>();
            let kinds = batch.column_by_name("kind").unwrap().as_string::<i32>();
            let contents = batch.column_by_name("content").unwrap().as_string::<i32>();
            for row in 0..batch.num_rows() {
                if kinds.value(row) != kind.as_str() {
                    continue;
                }
                let content = contents.value(row).to_string();
                hits.push(RetrievedChunk {
                    path: paths.value(row).to_string(),
                    score: lexical_score(query, &content),
                    content,
                });
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }

    async fn write_manifest(
        &self,
        chunks: &[ChunkRecord],
        vectors: &[Vec<f32>],
    ) -> anyhow::Result<()> {
        let path = self.manifest_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let records: Vec<ManifestRecord> = chunks
            .iter()
            .zip(vectors.iter())
            .map(|(chunk, vector)| ManifestRecord {
                path: chunk.path.clone(),
                kind: chunk.kind.as_str().to_string(),
                content: chunk.content.clone(),
                vector: normalize_dims(vector),
            })
            .collect();
        tokio::fs::write(path, serde_json::to_vec_pretty(&records)?).await?;
        Ok(())
    }

    async fn retrieve_manifest(
        &self,
        kind: RetrieveKind,
        query: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let records: Vec<ManifestRecord> = serde_json::from_slice(&tokio::fs::read(path).await?)?;
        let mut hits: Vec<_> = records
            .into_iter()
            .filter(|record| record.kind == kind.as_str())
            .map(|record| RetrievedChunk {
                score: cosine_similarity(query_vector, &record.vector)
                    + lexical_score(query, &record.content),
                path: record.path,
                content: record.content,
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }

    fn db_dir(&self) -> PathBuf {
        self.workspace_root.join(".rove").join("rag.lancedb")
    }

    fn manifest_path(&self) -> PathBuf {
        self.workspace_root.join(".rove").join("rag_manifest.json")
    }
}

pub struct RagRetrieveTool {
    root: PathBuf,
    kind: RetrieveKind,
}

impl RagRetrieveTool {
    pub fn code(root: PathBuf) -> Self {
        Self {
            root,
            kind: RetrieveKind::Code,
        }
    }

    pub fn docs(root: PathBuf) -> Self {
        Self {
            root,
            kind: RetrieveKind::Docs,
        }
    }
}

#[async_trait]
impl Tool for RagRetrieveTool {
    fn schema(&self) -> ToolSchema {
        let name = match self.kind {
            RetrieveKind::Code => "retrieve_code",
            RetrieveKind::Docs => "retrieve_docs",
        };
        ToolSchema {
            name: name.to_string(),
            description: format!(
                "Retrieve relevant {} chunks from the workspace RAG index.",
                self.kind.as_str()
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "number", "description": "Maximum number of chunks" }
                },
                "required": ["query"]
            }),
            destructive: false,
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: query".to_string(),
            })?;
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5) as usize;
        let index = RagIndex::new(self.root.clone());
        let embedder = DeterministicEmbedder;
        let hits = index
            .retrieve(&embedder, self.kind, query, limit)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        let content = serde_json::to_string_pretty(&hits_as_json(&hits)).map_err(|err| {
            ToolError::ExecutionFailed {
                reason: err.to_string(),
            }
        })?;
        Ok(ToolOutput { content })
    }
}

#[derive(Debug, Clone)]
struct ChunkRecord {
    path: String,
    kind: RetrieveKind,
    content: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ManifestRecord {
    path: String,
    kind: String,
    content: String,
    vector: Vec<f32>,
}

fn hits_as_json(hits: &[RetrievedChunk]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|hit| {
            serde_json::json!({
                "path": hit.path,
                "score": hit.score,
                "content": hit.content,
            })
        })
        .collect()
}

fn classify_path(path: &Path) -> Option<RetrieveKind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "rs" | "toml" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "h"
        | "hpp" => Some(RetrieveKind::Code),
        "md" | "mdx" | "txt" | "rst" => Some(RetrieveKind::Docs),
        _ => None,
    }
}

fn is_ignored(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name,
                ".git" | ".rove" | "target" | "node_modules" | ".next" | "dist"
            )
        })
        .unwrap_or(false)
}

fn chunk_text(content: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        if current.len() + line.len() + 1 > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

fn deterministic_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0; EMBEDDING_DIMS];
    for token in tokenize(text) {
        let idx = stable_hash(&token) % EMBEDDING_DIMS;
        vector[idx] += 1.0;
    }
    normalize(vector)
}

fn normalize_dims(vector: &[f32]) -> Vec<f32> {
    let mut fixed = vec![0.0; EMBEDDING_DIMS];
    for (idx, value) in vector.iter().take(EMBEDDING_DIMS).enumerate() {
        fixed[idx] = *value;
    }
    normalize(fixed)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let left = normalize_dims(left);
    let right = normalize_dims(right);
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn lexical_score(query: &str, content: &str) -> f32 {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let content_tokens = tokenize(content);
    let matches = query_tokens
        .iter()
        .filter(|token| content_tokens.contains(token))
        .count();
    matches as f32 / query_tokens.len() as f32
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn stable_hash(value: &str) -> usize {
    let mut hash: usize = 1469598103934665603usize;
    for byte in value.bytes() {
        hash ^= byte as usize;
        hash = hash.wrapping_mul(1099511628211usize);
    }
    hash
}
