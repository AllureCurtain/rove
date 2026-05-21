use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::ToolSchema;
use crate::errors::ToolError;

/// A minimal echo tool for testing.
///
/// Takes a "message" argument and returns it as output.
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "echo".to_string(),
            description: "Echoes back the given message. Useful for testing.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to echo back"
                    }
                },
                "required": ["message"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: message".to_string(),
            })?;

        Ok(ToolOutput {
            content: message.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_tool_returns_message() {
        let tool = EchoTool;
        let args = serde_json::json!({"message": "hello world"});
        let result = tool.execute(args).await.unwrap();
        assert_eq!(result.content, "hello world");
    }

    #[tokio::test]
    async fn echo_tool_missing_message() {
        let tool = EchoTool;
        let args = serde_json::json!({});
        let result = tool.execute(args).await;
        assert!(result.is_err());
    }
}
