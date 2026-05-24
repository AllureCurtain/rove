use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::database::CreateTableMode;
use lancedb::query::{ExecutableQuery, QueryBase};
use walkdir::WalkDir;

use super::embed::{EMBEDDING_DIMS, Embedder, cosine_similarity, normalize_dims, tokenize};
use super::types::{RetrieveKind, RetrievedChunk};

const TABLE_NAME: &str = "chunks";

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
                let path = paths.value(row).to_string();
                let content = contents.value(row).to_string();
                hits.push(RetrievedChunk {
                    id: format!("{path}#{row}"),
                    path,
                    kind,
                    score: lexical_score(query, &content),
                    source: "vector".to_string(),
                    heading: None,
                    chunk_hash: None,
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
            .enumerate()
            .filter(|(_, record)| record.kind == kind.as_str())
            .map(|(idx, record)| RetrievedChunk {
                id: format!("{}#{idx}", record.path),
                score: cosine_similarity(query_vector, &record.vector)
                    + lexical_score(query, &record.content),
                path: record.path,
                kind,
                source: "manifest".to_string(),
                heading: None,
                chunk_hash: None,
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
