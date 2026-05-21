use async_trait::async_trait;
use serde_json::Value;

use crate::core::types::ToolSchema;
use crate::errors::ToolError;

/// Result of a tool execution.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
}

/// Trait that all tools must implement.
///
/// Each tool has a schema (for LLM) and an execute method.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool's schema definition exposed to the LLM.
    fn schema(&self) -> ToolSchema;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError>;
}
