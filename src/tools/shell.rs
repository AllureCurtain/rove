use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

/// Execute a shell command in the workspace.
pub struct ShellTool {
    root: PathBuf,
    policy: ShellPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellPolicy {
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub inherit_environment: bool,
    pub denylist: Vec<String>,
}

impl Default for ShellPolicy {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_output_bytes: 64 * 1024,
            inherit_environment: true,
            denylist: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct ShellOutput {
    command: String,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl ShellTool {
    pub fn new(root: PathBuf) -> Self {
        Self::with_policy(root, ShellPolicy::default())
    }

    pub fn with_policy(root: PathBuf, policy: ShellPolicy) -> Self {
        Self { root, policy }
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
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: command".to_string(),
            })?;
        validate_shell_command(command, &self.policy)?;

        let mut process = shell_command(command);
        process.current_dir(&self.root).kill_on_drop(true);
        if !self.policy.inherit_environment {
            process.env_clear();
        }
        let output = tokio::time::timeout(
            Duration::from_millis(self.policy.timeout_ms),
            process.output(),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            timeout_ms: self.policy.timeout_ms,
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            reason: e.to_string(),
        })?;

        let (stdout, stdout_truncated) =
            truncate_lossy(&output.stdout, self.policy.max_output_bytes);
        let (stderr, stderr_truncated) =
            truncate_lossy(&output.stderr, self.policy.max_output_bytes);
        let content = serde_json::to_string(&ShellOutput {
            command: command.to_string(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
        .map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;

        Ok(ToolOutput::text(content))
    }
}

fn validate_shell_command(command: &str, policy: &ShellPolicy) -> Result<(), ToolError> {
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

    if let Some(blocked) = policy
        .denylist
        .iter()
        .find(|blocked| !blocked.is_empty() && command.contains(blocked.as_str()))
    {
        return Err(ToolError::PermissionDenied {
            reason: format!("shell command contains denied pattern: {blocked}"),
        });
    }

    Ok(())
}

fn truncate_lossy(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        return (String::from_utf8_lossy(bytes).to_string(), false);
    }
    (
        String::from_utf8_lossy(&bytes[..max_bytes]).to_string(),
        true,
    )
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
