use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema, UserInputRequest};
use crate::errors::ToolError;

/// Ask the user for input mid-task.
///
/// The tool surface is exposed now so models can learn the contract. Actual
/// interface-backed input routing will plug into this tool in a later slice.
pub struct RequestInputTool;

#[async_trait]
impl Tool for RequestInputTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
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

        if let Some(provider) = &ctx.input_provider {
            let answer = provider
                .request_input(UserInputRequest {
                    prompt: prompt.clone(),
                })
                .await?;
            return Ok(ToolOutput { content: answer });
        }

        Ok(ToolOutput {
            content: format!(
                "request_input requires an interactive input provider. Prompt: {prompt}"
            ),
        })
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
