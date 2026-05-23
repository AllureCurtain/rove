use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
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

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: command".to_string(),
            })?;
        validate_shell_command(command)?;

        let mut process = shell_command(command);
        let output = process
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

fn validate_shell_command(command: &str) -> Result<(), ToolError> {
    if command.trim().is_empty() {
        return Err(ToolError::InvalidInput {
            reason: "empty shell commands are not allowed".to_string(),
        });
    }

    if command.contains('\0') {
        return Err(ToolError::InvalidInput {
            reason: "shell commands may not contain NUL bytes".to_string(),
        });
    }

    Ok(())
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("powershell");
    process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    process
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.args(["-lc", command]);
    process
}
