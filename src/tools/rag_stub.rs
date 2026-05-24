use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

#[derive(Debug, Clone, Copy)]
enum RetrieveKind {
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

    fn tool_name(self) -> &'static str {
        match self {
            Self::Code => "retrieve_code",
            Self::Docs => "retrieve_docs",
        }
    }
}

pub struct RagRetrieveTool {
    _root: PathBuf,
    kind: RetrieveKind,
}

impl RagRetrieveTool {
    pub fn code(root: PathBuf) -> Self {
        Self {
            _root: root,
            kind: RetrieveKind::Code,
        }
    }

    pub fn docs(root: PathBuf) -> Self {
        Self {
            _root: root,
            kind: RetrieveKind::Docs,
        }
    }
}

#[async_trait]
impl Tool for RagRetrieveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.kind.tool_name().to_string(),
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
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let _query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: query".to_string(),
            })?;

        Ok(ToolOutput {
            content: format!(
                "`{}` requires the `rag` feature. Rebuild with `--features rag` or use a RAG-enabled binary.",
                self.kind.tool_name()
            ),
        })
    }
}
