use std::path::PathBuf;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::environment::{EnvironmentError, ProcessRequest};
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

/// Execute a shell command in the workspace.
pub struct ShellTool {
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

    pub fn with_policy(_root: PathBuf, policy: ShellPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "run_shell".to_string(),
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

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: command".to_string(),
            })?;
        validate_shell_command(command, &self.policy)?;

        let (program, args) = shell_command(command);
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().process_run {
            return Err(map_environment_error(
                EnvironmentError::CapabilityUnavailable("process_run"),
            ));
        }
        let output = services
            .environment
            .processes()
            .run(
                ProcessRequest {
                    program,
                    args,
                    cwd: services.workspace.root.clone(),
                    environment: Default::default(),
                    clear_environment: !self.policy.inherit_environment,
                    timeout_ms: self.policy.timeout_ms,
                    max_output_bytes: self.policy.max_output_bytes,
                },
                ctx.cancel_token.clone(),
            )
            .await
            .map_err(map_environment_error)?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let content = serde_json::to_string(&ShellOutput {
            command: command.to_string(),
            success: output.status_code == Some(0),
            exit_code: output.status_code,
            stdout,
            stderr,
            stdout_truncated: output.stdout_truncated,
            stderr_truncated: output.stderr_truncated,
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

#[cfg(windows)]
fn shell_command(command: &str) -> (String, Vec<String>) {
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ],
    )
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> (String, Vec<String>) {
    (
        "sh".to_string(),
        vec!["-lc".to_string(), command.to_string()],
    )
}

fn map_environment_error(error: EnvironmentError) -> ToolError {
    match error {
        EnvironmentError::Timeout(timeout_ms) => ToolError::Timeout { timeout_ms },
        EnvironmentError::Cancelled => ToolError::ExecutionFailed {
            reason: "execution cancelled".to_string(),
        },
        EnvironmentError::CapabilityUnavailable(capability) => ToolError::PermissionDenied {
            reason: format!("execution capability unavailable: {capability}"),
        },
        EnvironmentError::StaleObservation => ToolError::InvalidInput {
            reason: "observation version is stale".to_string(),
        },
        EnvironmentError::NotFound => ToolError::ExecutionFailed {
            reason: "workspace file was not found".to_string(),
        },
        EnvironmentError::InvalidPath(reason) => ToolError::InvalidInput { reason },
        EnvironmentError::Boundary => ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        },
        EnvironmentError::Host(reason) => ToolError::ExecutionFailed { reason },
    }
}
