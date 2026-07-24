use async_trait::async_trait;

use crate::{CallId, ToolContext, ToolDescriptor, ToolError, ToolOutput};

#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub call_id: CallId,
    pub tool_use_id: Option<String>,
    pub name: String,
    pub args: serde_json::Value,
}

#[async_trait]
pub trait ToolPolicy: Send + Sync {
    async fn before_tool(
        &self,
        _invocation: &ToolInvocation,
        _descriptor: &ToolDescriptor,
        _context: &ToolContext<'_>,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    async fn after_tool(
        &self,
        _invocation: &ToolInvocation,
        _descriptor: &ToolDescriptor,
        _context: &ToolContext<'_>,
        _output: &ToolOutput,
    ) -> Result<(), ToolError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AllowAllToolPolicy;

impl ToolPolicy for AllowAllToolPolicy {}
