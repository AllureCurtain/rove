use std::path::PathBuf;

use async_trait::async_trait;
use rove_core::{
    Tool, ToolCapability, ToolContext, ToolDescriptor as ToolSchema, ToolError, ToolOutput,
};

/// Disabled-by-default RAG retrieve tools for product assemblies that do not
/// enable the heavy RAG feature. Real RAG implementations remain deferred.
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

pub struct RagRetrieveStub {
    _root: PathBuf,
    kind: RetrieveKind,
}

impl RagRetrieveStub {
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
impl Tool for RagRetrieveStub {
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
            capability: Some(ToolCapability {
                status: "disabled".to_string(),
                feature: Some("rag".to_string()),
                message: Some("RAG feature is not enabled in this build".to_string()),
            }),
        }
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let _query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: query".to_string(),
            })?;

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&serde_json::json!({
                "capability": "disabled",
                "feature": "rag",
                "tool": self.kind.tool_name(),
                "message": format!(
                    "`{}` requires the `rag` feature. Rebuild with `--features rag` or use a RAG-enabled binary.",
                    self.kind.tool_name()
                )
            }))
            .unwrap_or_else(|_| {
                format!(
                    "`{}` requires the `rag` feature. Rebuild with `--features rag` or use a RAG-enabled binary.",
                    self.kind.tool_name()
                )
            }),
        ))
    }
}
