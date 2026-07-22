use std::collections::BTreeMap;
use std::path::Path;

use async_trait::async_trait;
use chrono::Utc;
use walkdir::WalkDir;

use super::pipeline::{IngestionContext, IngestionStage};
use crate::rag::embed::{EMBEDDING_DIMS, Embedder, normalize_dims};
use crate::rag::index::RagIndex;
use crate::rag::types::{
    ChunkingManifest, DiscoveredFile, EmbeddedChunk, EmbeddingManifest, IndexManifest, IndexedFile,
    ManifestChunk, ParsedDocument, RetrieveKind, sha256_hex,
};

pub struct ScanWorkspaceStage;

#[async_trait]
impl IngestionStage for ScanWorkspaceStage {
    fn name(&self) -> &'static str {
        "ScanWorkspace"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        let mut files = Vec::new();
        for entry in WalkDir::new(&context.workspace_root)
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
            let relative_path = entry
                .path()
                .strip_prefix(&context.workspace_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            files.push(DiscoveredFile {
                absolute_path: entry.path().to_path_buf(),
                relative_path,
                kind,
            });
        }
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        context.discovered_files = files;
        Ok(())
    }
}

pub struct ParseReadableFilesStage;

#[async_trait]
impl IngestionStage for ParseReadableFilesStage {
    fn name(&self) -> &'static str {
        "ParseReadableFiles"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        let mut documents = Vec::new();
        for file in &context.discovered_files {
            let content = tokio::fs::read_to_string(&file.absolute_path).await?;
            let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
            documents.push(ParsedDocument {
                path: file.relative_path.clone(),
                kind: file.kind,
                content_hash: sha256_hex(normalized.as_bytes()),
                content: normalized,
            });
        }
        context.parsed_documents = documents;
        Ok(())
    }
}

pub struct ChunkDocumentsStage;

#[async_trait]
impl IngestionStage for ChunkDocumentsStage {
    fn name(&self) -> &'static str {
        "ChunkDocuments"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        let mut chunks = Vec::new();
        for document in &context.parsed_documents {
            chunks.extend(context.chunker.chunk(document));
        }
        context.chunks = chunks;
        Ok(())
    }
}

pub struct EmbedChunksStage<'a> {
    pub embedder: &'a dyn Embedder,
}

#[async_trait]
impl IngestionStage for EmbedChunksStage<'_> {
    fn name(&self) -> &'static str {
        "EmbedChunks"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        let mut embedded = Vec::new();
        for chunk in &context.chunks {
            let vector = normalize_dims(&self.embedder.embed(&chunk.content).await?);
            embedded.push(EmbeddedChunk {
                chunk: chunk.clone(),
                vector,
            });
        }
        context.embedded_chunks = embedded;
        Ok(())
    }
}

pub struct PersistIndexStage;

#[async_trait]
impl IngestionStage for PersistIndexStage {
    fn name(&self) -> &'static str {
        "PersistIndex"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        let index = RagIndex::new_with_state_dir(
            context.workspace_root.clone(),
            context
                .artifact_paths
                .manifest_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.workspace_root.join(".rove")),
        );
        index.write_lancedb_embedded(&context.embedded_chunks).await
    }
}

pub struct WriteManifestAndLogStage;

#[async_trait]
impl IngestionStage for WriteManifestAndLogStage {
    fn name(&self) -> &'static str {
        "WriteManifestAndLog"
    }

    async fn run(&self, context: &mut IngestionContext) -> anyhow::Result<()> {
        if let Some(parent) = context.artifact_paths.manifest_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let manifest = build_manifest(context);
        tokio::fs::write(
            &context.artifact_paths.manifest_path,
            serde_json::to_vec_pretty(&manifest)?,
        )
        .await?;
        Ok(())
    }
}

fn build_manifest(context: &IngestionContext) -> IndexManifest {
    let indexed_at = Utc::now().to_rfc3339();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for embedded in &context.embedded_chunks {
        *counts.entry(&embedded.chunk.path).or_default() += 1;
    }

    let files = context
        .parsed_documents
        .iter()
        .map(|document| IndexedFile {
            path: document.path.clone(),
            kind: document.kind,
            content_hash: document.content_hash.clone(),
            chunk_count: *counts.get(document.path.as_str()).unwrap_or(&0),
            indexed_at: indexed_at.clone(),
        })
        .collect();

    let chunks = context
        .embedded_chunks
        .iter()
        .map(|embedded| ManifestChunk {
            id: embedded.chunk.id.clone(),
            path: embedded.chunk.path.clone(),
            kind: embedded.chunk.kind,
            content_hash: embedded.chunk.content_hash.clone(),
            chunk_hash: embedded.chunk.chunk_hash.clone(),
            start_byte: embedded.chunk.start_byte,
            end_byte: embedded.chunk.end_byte,
            heading: embedded.chunk.heading.clone(),
            content: embedded.chunk.content.clone(),
            vector: embedded.vector.clone(),
        })
        .collect();

    IndexManifest {
        schema_version: 1,
        workspace_root: context.workspace_root.to_string_lossy().replace('\\', "/"),
        embedding: EmbeddingManifest {
            provider: "deterministic".to_string(),
            model: "deterministic-64".to_string(),
            dims: EMBEDDING_DIMS,
        },
        chunking: ChunkingManifest {
            strategy: context.chunker.name().to_string(),
            target_chars: context.chunker.target_chars(),
            overlap_chars: context.chunker.overlap_chars(),
        },
        files,
        chunks,
    }
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
