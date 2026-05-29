use async_trait::async_trait;
use serde_json::Value;

use crate::core::types::{ToolContext, ToolMutation, ToolSchema};
use crate::errors::ToolError;

/// Result of a tool execution.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub mutations: Vec<ToolMutation>,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            mutations: Vec::new(),
        }
    }
}

/// Trait that all tools must implement.
///
/// Each tool has a schema (for LLM) and an execute method.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool's schema definition exposed to the LLM.
    fn schema(&self) -> ToolSchema;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError>;
}
