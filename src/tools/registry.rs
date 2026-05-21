use std::collections::HashMap;

use crate::core::types::ToolSchema;
use crate::errors::ToolError;

use super::traits::{Tool, ToolOutput};

/// Registry of available tools.
///
/// The engine uses this to look up tools by name and get their schemas.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites if name already exists.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.schema().name.clone();
        self.tools.insert(name, tool);
    }

    /// Get all tool schemas (for sending to the LLM).
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// Get one tool schema by name.
    pub fn schema(&self, name: &str) -> Result<ToolSchema, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::UnknownTool {
            name: name.to_string(),
        })?;
        Ok(tool.schema())
    }

    /// Execute a tool by name.
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::UnknownTool {
            name: name.to_string(),
        })?;
        tool.execute(args).await
    }

    /// Check if a tool exists.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
