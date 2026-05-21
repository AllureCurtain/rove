use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;

use super::traits::{Tool, ToolOutput};
use crate::core::types::ToolSchema;
use crate::errors::ToolError;

/// Execute a shell command in the workspace.
pub struct ShellTool {
    root: PathBuf,
}

impl ShellTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "shell".to_string(),
            description: "Run a shell command in the current workspace.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute with sh -lc"
                    }
                },
                "required": ["command"]
            }),
            destructive: true,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: command".to_string(),
            })?;

        let output = Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(&self.root)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                reason: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let content = if output.status.success() {
            stdout.to_string()
        } else {
            format!(
                "exit status: {}\nstdout:\n{}\nstderr:\n{}",
                output.status, stdout, stderr
            )
        };

        Ok(ToolOutput { content })
    }
}
