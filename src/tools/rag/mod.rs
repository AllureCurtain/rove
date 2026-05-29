use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolCapability, ToolContext, ToolSchema};
use crate::errors::ToolError;
use crate::tools::rag::rewrite::{DeterministicQueryRewriteService, QueryRewriteService};

mod embed;
pub mod eval;
mod index;
pub mod ingest;
mod prompt;
pub mod retrieve;
pub mod rewrite;
mod types;

pub use embed::{DeterministicEmbedder, Embedder, OpenAiEmbedder, RoutingEmbedder};
pub use index::RagIndex;
pub use prompt::RagPromptService;
pub use types::{
    ChunkingManifest, EmbeddingManifest, IndexManifest, IndexedFile, ManifestChunk, ParsedDocument,
    RetrieveKind, RetrievedChunk,
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
            parallel_safe: true,
            capability: Some(ToolCapability {
                status: "enabled".to_string(),
                feature: Some("rag".to_string()),
                message: None,
            }),
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
        let index =
            RagIndex::new_with_state_dir(self.root.clone(), _ctx.workspace.state_dir.clone());
        let embedder = DeterministicEmbedder;
        let hits = index
            .retrieve(&embedder, self.kind, query, limit)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        let rewrite = DeterministicQueryRewriteService.rewrite(query);
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "normalized_query": rewrite.normalized_query,
            "kind": self.kind.as_str(),
            "limit": limit,
            "results": hits_as_json(&hits),
        }))
        .map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;
        Ok(ToolOutput::text(content))
    }
}

fn hits_as_json(hits: &[RetrievedChunk]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|hit| {
            serde_json::json!({
                "id": hit.id,
                "path": hit.path,
                "score": hit.score,
                "source": hit.source,
                "heading": hit.heading,
                "content": hit.content,
            })
        })
        .collect()
}
