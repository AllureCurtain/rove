use crate::core::types::{CallId, ToolResult};
use crate::errors::ToolError;
use crate::tools::registry::ToolRegistry;

/// The executor runs tools through the pipeline.
///
/// M0 pipeline (simplified):
///   1. Look up tool in registry
///   2. Execute
///
/// M1 pipeline (full):
///   schema → validate_input → pre-hook → permission → exec → post-hook → diff
pub struct Executor<'a> {
    registry: &'a ToolRegistry,
}

impl<'a> Executor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self { registry }
    }

    /// Execute a tool call through the pipeline.
    pub async fn run(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: CallId,
    ) -> Result<ToolResult, ToolError> {
        // Step 1: Execute (registry handles lookup + dispatch)
        let output = self.registry.execute(name, args).await?;

        Ok(ToolResult {
            call_id,
            output: output.content,
        })
    }
}
