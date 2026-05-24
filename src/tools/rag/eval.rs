use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::tools::rag::RagIndex;
use crate::tools::rag::embed::Embedder;
use crate::tools::rag::retrieve::channel::ChannelStatus;
use crate::tools::rag::retrieve::pipeline::RetrievalPipeline;
use crate::tools::rag::types::RetrieveKind;

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalChannelSummary {
    pub name: String,
    pub status: ChannelStatus,
    pub result_count: usize,
    pub duration_ms: u128,
    pub fallback_used: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalResult {
    pub rank: usize,
    pub id: String,
    pub path: String,
    pub score: f32,
    pub source: String,
    pub heading: Option<String>,
    pub content_preview: String,
}

pub async fn run_retrieval_eval(
    index: &RagIndex,
    embedder: &dyn Embedder,
    kind: RetrieveKind,
    query: &str,
    limit: usize,
) -> anyhow::Result<RetrievalEvalReport> {
    let start = Instant::now();
    let output = RetrievalPipeline::new(index, embedder)
        .run(kind, query, limit)
        .await?;
    let duration_ms = start.elapsed().as_millis();

    Ok(RetrievalEvalReport {
        schema_version: 1,
        query: output.context.original_query,
        normalized_query: output.context.normalized_query,
        kind,
        limit,
        duration_ms,
        channels: output
            .channels
            .into_iter()
            .map(|channel| EvalChannelSummary {
                name: channel.name,
                status: channel.status,
                result_count: channel.result_count,
                duration_ms: channel.duration_ms,
                fallback_used: channel.fallback_used,
                error: channel.error,
            })
            .collect(),
        results: output
            .results
            .into_iter()
            .enumerate()
            .map(|(idx, result)| EvalResult {
                rank: idx + 1,
                id: result.id,
                path: result.path,
                score: result.score,
                source: result.source,
                heading: result.heading,
                content_preview: content_preview(&result.content),
            })
            .collect(),
        artifact_path: String::new(),
    })
}

pub async fn write_eval_report(
    workspace_root: &Path,
    report: &RetrievalEvalReport,
) -> anyhow::Result<PathBuf> {
    let dir = workspace_root.join(".rove").join("rag_eval");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}.json", ulid::Ulid::new()));
    let mut value = serde_json::to_value(report)?;
    value["artifact_path"] = serde_json::Value::String(path.to_string_lossy().replace('\\', "/"));
    tokio::fs::write(&path, serde_json::to_vec_pretty(&value)?).await?;
    Ok(path)
}

fn content_preview(content: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 200;
    let mut preview = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.chars().count() > MAX_PREVIEW_CHARS {
        preview = preview.chars().take(MAX_PREVIEW_CHARS).collect();
    }
    preview
}
