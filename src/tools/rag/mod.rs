use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

mod embed;
mod index;
pub mod ingest;
pub mod rewrite;
mod types;

pub use embed::{DeterministicEmbedder, Embedder, OpenAiEmbedder};
pub use index::RagIndex;
pub use types::{
    ChunkingManifest, EmbeddingManifest, IndexManifest, IndexedFile, ManifestChunk, RetrieveKind,
    RetrievedChunk,
};

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
