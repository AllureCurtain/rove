use async_trait::async_trait;
use serde_json::Value;

use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

/// Ask the user for input mid-task.
///
/// The tool surface is exposed now so models can learn the contract. Actual
/// interface-backed input routing will plug into this tool in a later slice.
pub struct RequestInputTool;

#[async_trait]
impl Tool for RequestInputTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "request_input".to_string(),
            description: "Ask the user for input mid-task.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Question or clarification to ask the user"
                    }
                },
                "required": ["prompt"]
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("interaction.user.request-input".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: prompt".to_string(),
            })?;
        let prompt = validate_prompt(prompt)?;

        if let Some(provider) = &runtime_tool_services(ctx)?.input_provider {
            let answer =
                crate::tool_input::request_input(provider.as_ref(), prompt.clone()).await?;
            return Ok(ToolOutput::text(answer));
        }

        Ok(ToolOutput::text(format!(
            "request_input requires an interactive input provider. Prompt: {prompt}"
        )))
    }
}

fn validate_prompt(raw: &str) -> Result<String, ToolError> {
    let prompt = raw.trim();
    if prompt.is_empty() {
        return Err(ToolError::InvalidInput {
            reason: "request_input prompt must not be empty".to_string(),
        });
    }
    if prompt.contains('\0') {
        return Err(ToolError::InvalidInput {
            reason: "request_input prompt may not contain NUL bytes".to_string(),
        });
    }
    Ok(prompt.to_string())
}
