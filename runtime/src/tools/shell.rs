use std::path::PathBuf;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::environment::{BackgroundProcessStatus, EnvironmentError, Observation, ProcessRequest};
use crate::tools::coding::map_environment_error;
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};
use rove_core::{ToolCapability, ToolDescriptor};

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
    mode: String,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    observation_id: Option<String>,
    artifact_ref: Option<String>,
}

#[derive(Serialize)]
struct BackgroundStartOutput {
    command: String,
    mode: String,
    process_id: String,
    status: BackgroundProcessStatus,
    stdout_cursor: usize,
    stderr_cursor: usize,
}

#[derive(Serialize)]
struct BackgroundPollOutput {
    process_id: String,
    status: BackgroundProcessStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_cursor: usize,
    stderr_cursor: usize,
    stdout_truncated: bool,
    stderr_truncated: bool,
    stdout_has_more: bool,
    stderr_has_more: bool,
    output_complete: bool,
    observation_id: String,
    artifact_ref: Option<String>,
}

const SHELL_CONTEXT_PROJECTION_BYTES: usize = 16 * 1024;
const DEFAULT_POLL_BYTES: usize = 16 * 1024;
const MAX_POLL_BYTES: usize = 64 * 1024;

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
            description: "Run a shell command in the workspace, either in the foreground or as an explicitly identified background process.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 32768,
                        "description": "Command to execute with sh -lc"
                    },
                    "background": {
                        "type": "boolean",
                        "default": false,
                        "description": "Start the command in the background and return an opaque process ID"
                    },
                    "paths": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 4096
                        },
                        "description": "Workspace-relative paths the command will access, when path-scoped workspace instructions exist"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("execution.shell.run".to_string()),
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

        let background = args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (program, shell_args) = shell_command(command);
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().process_run {
            return Err(map_environment_error(
                EnvironmentError::CapabilityUnavailable("process_run"),
            ));
        }
        let request = ProcessRequest {
            program,
            args: shell_args,
            cwd: services.workspace.root.clone(),
            environment: Default::default(),
            clear_environment: !self.policy.inherit_environment,
            timeout_ms: self.policy.timeout_ms,
            max_output_bytes: self.policy.max_output_bytes,
        };
        if background {
            if !services.environment.capabilities().process_background {
                return Err(ToolError::PermissionDenied {
                    reason: "execution capability unavailable: process_background".to_string(),
                });
            }
            let started = services
                .environment
                .processes()
                .spawn_background(request, ctx.cancel_token.clone())
                .await
                .map_err(map_environment_error)?;
            return Ok(ToolOutput::text(
                serde_json::to_string(&BackgroundStartOutput {
                    command: command.to_string(),
                    mode: "background".to_string(),
                    process_id: started.process_id,
                    status: BackgroundProcessStatus::Running,
                    stdout_cursor: 0,
                    stderr_cursor: 0,
                })
                .map_err(|error| ToolError::ExecutionFailed {
                    reason: error.to_string(),
                })?,
            ));
        }
        let output = services
            .environment
            .processes()
            .run(request, ctx.cancel_token.clone())
            .await
            .map_err(map_environment_error)?;

        let mut retained = output.stdout.clone();
        retained.extend_from_slice(b"\n--- stderr ---\n");
        retained.extend_from_slice(&output.stderr);
        let source = format!("process:foreground:{}", ctx.call_id);
        let version = {
            use sha2::{Digest, Sha256};
            format!("sha256:{:x}", Sha256::digest(&retained))
        };
        let artifact_ref = if retained.len() > SHELL_CONTEXT_PROJECTION_BYTES
            && services.environment.capabilities().artifact_projection
        {
            match services.environment.artifacts() {
                Some(sink) => sink.put(&source, &retained).await.ok().flatten(),
                None => None,
            }
        } else {
            None
        };
        let projection_limit = if artifact_ref.is_some() {
            SHELL_CONTEXT_PROJECTION_BYTES / 2
        } else {
            self.policy.max_output_bytes
        };
        let stdout_bytes = &output.stdout[..output.stdout.len().min(projection_limit)];
        let stderr_bytes = &output.stderr[..output.stderr.len().min(projection_limit)];
        let stdout = String::from_utf8_lossy(stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(stderr_bytes).to_string();
        let observation = Observation::from_bytes(
            source,
            0,
            &retained,
            version,
            output.stdout_truncated
                || output.stderr_truncated
                || stdout_bytes.len() < output.stdout.len()
                || stderr_bytes.len() < output.stderr.len(),
            artifact_ref.clone(),
        );
        services
            .environment
            .observations()
            .put_with_payload(observation.clone(), retained)
            .await
            .map_err(map_environment_error)?;
        let content = serde_json::to_string(&ShellOutput {
            command: command.to_string(),
            mode: "foreground".to_string(),
            success: output.status_code == Some(0),
            exit_code: output.status_code,
            stdout,
            stderr,
            stdout_truncated: output.stdout_truncated || stdout_bytes.len() < output.stdout.len(),
            stderr_truncated: output.stderr_truncated || stderr_bytes.len() < output.stderr.len(),
            observation_id: Some(observation.id),
            artifact_ref,
        })
        .map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;

        Ok(ToolOutput::text(content))
    }
}

#[derive(Default)]
pub struct ShellOutputTool;

impl ShellOutputTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellOutputTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "shell_output".to_string(),
            description:
                "Read the next bounded stdout/stderr page for a Runtime-owned background process."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "minLength": 1, "maxLength": 128 },
                    "stdout_cursor": { "type": "integer", "minimum": 0, "maximum": 67108864, "default": 0 },
                    "stderr_cursor": { "type": "integer", "minimum": 0, "maximum": 67108864, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 65536, "default": 16384 }
                },
                "required": ["process_id"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("execution.shell.output".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let process_id = args
            .get("process_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: process_id".to_string(),
            })?;
        let stdout_cursor = cursor_arg(&args, "stdout_cursor")?;
        let stderr_cursor = cursor_arg(&args, "stderr_cursor")?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_POLL_BYTES as u64) as usize;
        if !(1..=MAX_POLL_BYTES).contains(&limit) {
            return Err(ToolError::InvalidInput {
                reason: "shell output limit must be between 1 and 65536 bytes".to_string(),
            });
        }
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().process_background {
            return Err(ToolError::PermissionDenied {
                reason: "execution capability unavailable: process_background".to_string(),
            });
        }
        let output = services
            .environment
            .processes()
            .poll_background(process_id, stdout_cursor, stderr_cursor, limit)
            .await
            .map_err(map_environment_error)?;
        let mut retained = output.stdout.clone();
        retained.extend_from_slice(b"\n--- stderr page ---\n");
        retained.extend_from_slice(&output.stderr);
        let source = format!("process:background:{process_id}");
        let version = format!("cursor:{}:{}", output.stdout_cursor, output.stderr_cursor);
        let artifact_ref =
            if retained.len() >= limit && services.environment.capabilities().artifact_projection {
                match services.environment.artifacts() {
                    Some(sink) => sink.put(&source, &retained).await.ok().flatten(),
                    None => None,
                }
            } else {
                None
            };
        let observation = Observation::from_bytes(
            source,
            stdout_cursor.saturating_add(stderr_cursor),
            &retained,
            version,
            output.stdout_truncated || output.stderr_truncated,
            artifact_ref.clone(),
        );
        services
            .environment
            .observations()
            .put_with_payload(observation.clone(), retained)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput::text(
            serde_json::to_string(&BackgroundPollOutput {
                process_id: output.process_id,
                status: output.status,
                exit_code: output.status_code,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                stdout_cursor: output.stdout_cursor,
                stderr_cursor: output.stderr_cursor,
                stdout_truncated: output.stdout_truncated,
                stderr_truncated: output.stderr_truncated,
                stdout_has_more: output.stdout_has_more,
                stderr_has_more: output.stderr_has_more,
                output_complete: output.output_complete,
                observation_id: observation.id,
                artifact_ref,
            })
            .map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?,
        ))
    }
}

#[derive(Default)]
pub struct ShellTerminateTool;

impl ShellTerminateTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellTerminateTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "shell_terminate".to_string(),
            description: "Terminate and wait for a Runtime-owned background process.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "minLength": 1, "maxLength": 128 }
                },
                "required": ["process_id"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("execution.shell.terminate".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let process_id = args
            .get("process_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: process_id".to_string(),
            })?;
        let services = runtime_tool_services(ctx)?;
        services
            .environment
            .processes()
            .terminate_background(process_id)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput::text(
            serde_json::json!({
                "process_id": process_id,
                "status": "terminated"
            })
            .to_string(),
        ))
    }
}

#[derive(Default)]
pub struct ShellPtyTool;

impl ShellPtyTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellPtyTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "run_shell_pty".to_string(),
            description: "Request a pseudo-terminal shell when the active Execution Environment supports PTY sessions.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "minLength": 1, "maxLength": 32768 }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("execution.shell.pty".to_string()),
            capability: Some(ToolCapability {
                status: "unsupported".to_string(),
                feature: Some("pty".to_string()),
                message: Some(
                    "the local Coding Tool V2 adapter does not provide PTY sessions".to_string(),
                ),
            }),
        }
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        Err(ToolError::PermissionDenied {
            reason: "execution capability unavailable: process_pty (typed unsupported)".to_string(),
        })
    }
}

fn cursor_arg(args: &Value, field: &str) -> Result<usize, ToolError> {
    args.get(field)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .try_into()
        .map_err(|_| ToolError::InvalidInput {
            reason: format!("{field} is too large"),
        })
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
