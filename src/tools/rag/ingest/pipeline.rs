use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;

use super::chunking::{ChunkingStrategy, MixedCodeMarkdownChunker};
use super::log::{StageLogRow, StageStatus, append_stage_log};
use super::stages::{
    ChunkDocumentsStage, EmbedChunksStage, ParseReadableFilesStage, PersistIndexStage,
    ScanWorkspaceStage, WriteManifestAndLogStage,
};
use crate::tools::rag::embed::Embedder;
use crate::tools::rag::types::{DiscoveredFile, DocumentChunk, EmbeddedChunk, ParsedDocument};

#[async_trait]
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

#[derive(Debug, Clone)]
pub struct RagArtifactPaths {
    pub db_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub index_log_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct IngestionResult {
    pub chunk_count: usize,
}

pub struct IngestionPipeline<'a> {
    workspace_root: PathBuf,
    state_dir: PathBuf,
    embedder: &'a dyn Embedder,
    chunker: Box<dyn ChunkingStrategy>,
}

impl<'a> IngestionPipeline<'a> {
    pub fn default_markdown(workspace_root: PathBuf, embedder: &'a dyn Embedder) -> Self {
        let state_dir = workspace_root.join(".rove");
        Self::default_markdown_with_state_dir(workspace_root, state_dir, embedder)
    }

    pub fn default_markdown_with_state_dir(
        workspace_root: PathBuf,
        state_dir: PathBuf,
        embedder: &'a dyn Embedder,
    ) -> Self {
        Self {
            workspace_root,
            state_dir,
            embedder,
            chunker: Box::new(MixedCodeMarkdownChunker::new(1600, 160)),
        }
    }

    pub async fn run(self) -> anyhow::Result<IngestionResult> {
        let artifact_paths = RagArtifactPaths {
            db_dir: self.state_dir.join("rag.lancedb"),
            manifest_path: self.state_dir.join("rag_manifest.json"),
            index_log_path: self.state_dir.join("rag_index_log.jsonl"),
        };
        let mut context = IngestionContext {
            run_id: ulid::Ulid::new().to_string(),
            workspace_root: self.workspace_root,
            chunker: self.chunker,
            discovered_files: Vec::new(),
            parsed_documents: Vec::new(),
            chunks: Vec::new(),
            embedded_chunks: Vec::new(),
            logs: Vec::new(),
            artifact_paths,
        };

        if context.artifact_paths.index_log_path.exists() {
            tokio::fs::remove_file(&context.artifact_paths.index_log_path).await?;
        }

        let stages: Vec<Box<dyn IngestionStage + '_>> = vec![
            Box::new(ScanWorkspaceStage),
            Box::new(ParseReadableFilesStage),
            Box::new(ChunkDocumentsStage),
            Box::new(EmbedChunksStage {
                embedder: self.embedder,
            }),
            Box::new(PersistIndexStage),
            Box::new(WriteManifestAndLogStage),
        ];

        for stage in stages {
            let input_count = count_for_stage(stage.name(), &context);
            let start = Instant::now();
            let result = stage.run(&mut context).await;
            let duration_ms = start.elapsed().as_millis();
            let output_count = count_for_stage(stage.name(), &context);
            let row = match result {
                Ok(()) => StageLogRow {
                    schema_version: 1,
                    run_id: context.run_id.clone(),
                    stage: stage.name().to_string(),
                    status: StageStatus::Completed,
                    duration_ms,
                    input_count,
                    output_count,
                    message: format!(
                        "{} completed: {input_count} input, {output_count} output",
                        stage.name()
                    ),
                    error: None,
                },
                Err(err) => {
                    let row = StageLogRow {
                        schema_version: 1,
                        run_id: context.run_id.clone(),
                        stage: stage.name().to_string(),
                        status: StageStatus::Failed,
                        duration_ms,
                        input_count,
                        output_count,
                        message: format!("{} failed", stage.name()),
                        error: Some(err.to_string()),
                    };
                    append_stage_log(&context.artifact_paths.index_log_path, &row).await?;
                    context.logs.push(row);
                    return Err(err);
                }
            };
            append_stage_log(&context.artifact_paths.index_log_path, &row).await?;
            context.logs.push(row);
        }

        Ok(IngestionResult {
            chunk_count: context.embedded_chunks.len(),
        })
    }
}

fn count_for_stage(stage: &str, context: &IngestionContext) -> usize {
    match stage {
        "ScanWorkspace" => context.discovered_files.len(),
        "ParseReadableFiles" => context.parsed_documents.len(),
        "ChunkDocuments" => context.chunks.len(),
        "EmbedChunks" | "PersistIndex" | "WriteManifestAndLog" => context.embedded_chunks.len(),
        _ => 0,
    }
}
