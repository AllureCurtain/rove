use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::database::CreateTableMode;
use lancedb::query::{ExecutableQuery, QueryBase};

use super::embed::{EMBEDDING_DIMS, Embedder, cosine_similarity, normalize_dims, tokenize};
use super::ingest::pipeline::IngestionPipeline;
use super::types::{EmbeddedChunk, IndexManifest, RetrieveKind, RetrievedChunk};

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
        let pipeline = IngestionPipeline::default_markdown(self.workspace_root.clone(), embedder);
        let result = pipeline.run().await?;
        Ok(result.chunk_count)
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

    pub(crate) async fn write_lancedb_embedded(
        &self,
        chunks: &[EmbeddedChunk],
    ) -> anyhow::Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let db_dir = self.db_dir();
        tokio::fs::create_dir_all(&db_dir).await?;
        let db = lancedb::connect(db_dir.to_str().unwrap()).execute().await?;

        let ids: Vec<_> = chunks.iter().map(|chunk| chunk.chunk.id.clone()).collect();
        let paths: Vec<_> = chunks
            .iter()
            .map(|chunk| chunk.chunk.path.clone())
            .collect();
        let kinds: Vec<_> = chunks
            .iter()
            .map(|chunk| chunk.chunk.kind.as_str().to_string())
            .collect();
        let contents: Vec<_> = chunks
            .iter()
            .map(|chunk| chunk.chunk.content.clone())
            .collect();
        let headings: Vec<_> = chunks
            .iter()
            .map(|chunk| chunk.chunk.heading.clone().unwrap_or_default())
            .collect();

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("path", DataType::Utf8, false),
            Field::new("kind", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("heading", DataType::Utf8, false),
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
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(paths)),
                Arc::new(StringArray::from(kinds)),
                Arc::new(StringArray::from(contents)),
                Arc::new(StringArray::from(headings)),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        chunks.iter().map(|chunk| {
                            let fixed = normalize_dims(&chunk.vector);
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
            let ids = batch.column_by_name("id").unwrap().as_string::<i32>();
            let contents = batch.column_by_name("content").unwrap().as_string::<i32>();
            let headings = batch.column_by_name("heading").unwrap().as_string::<i32>();
            for row in 0..batch.num_rows() {
                if kinds.value(row) != kind.as_str() {
                    continue;
                }
                let path = paths.value(row).to_string();
                let content = contents.value(row).to_string();
                let heading = headings.value(row);
                hits.push(RetrievedChunk {
                    id: ids.value(row).to_string(),
                    path,
                    kind,
                    score: lexical_score(query, &content),
                    source: "vector".to_string(),
                    heading: if heading.is_empty() {
                        None
                    } else {
                        Some(heading.to_string())
                    },
                    chunk_hash: None,
                    content,
                });
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
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
        let bytes = tokio::fs::read(path).await?;
        let manifest: ManifestOnDisk =
            serde_json::from_slice(&bytes).context("failed to parse RAG manifest")?;
        let mut hits: Vec<_> = match manifest {
            ManifestOnDisk::V1(manifest) => manifest
                .chunks
                .into_iter()
                .filter(|record| record.kind == kind)
                .map(|record| RetrievedChunk {
                    id: record.id,
                    score: cosine_similarity(query_vector, &record.vector)
                        + lexical_score(query, &record.content),
                    path: record.path,
                    kind,
                    source: "manifest".to_string(),
                    heading: record.heading,
                    chunk_hash: Some(record.chunk_hash),
                    content: record.content,
                })
                .collect(),
            ManifestOnDisk::Legacy(records) => records
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
                .collect(),
        };
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ManifestRecord {
    path: String,
    kind: String,
    content: String,
    vector: Vec<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
enum ManifestOnDisk {
    V1(IndexManifest),
    Legacy(Vec<ManifestRecord>),
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
