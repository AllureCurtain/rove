use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::environment::{EnvironmentError, FileMutationOperation};
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolMutation, ToolMutationOperation, ToolOutput};

/// Read a UTF-8 file inside the workspace.
pub struct FsReadTool;

impl FsReadTool {
    pub fn new(_root: PathBuf) -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FsReadTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "read_file".to_string(),
            description: "Read a UTF-8 file from the current workspace.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path"
                    }
                },
                "required": ["path"]
            }),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("workspace.fs.read".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_path = args
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: path".to_string(),
            })?;
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().filesystem_read {
            return Err(map_environment_error(
                EnvironmentError::CapabilityUnavailable("filesystem_read"),
            ));
        }
        let content = services
            .environment
            .filesystem()
            .read_utf8(raw_path)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput::text(content))
    }
}

/// Write a UTF-8 file inside the workspace.
pub struct FsWriteTool;

impl FsWriteTool {
    pub fn new(_root: PathBuf) -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FsWriteTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "write_file".to_string(),
            description: "Write a UTF-8 file in the current workspace.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path"
                    },
                    "content": {
                        "type": "string",
                        "description": "File content to write"
                    }
                },
                "required": ["path", "content"]
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.fs.write".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_path = args
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: path".to_string(),
            })?;
        let content = args
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: content".to_string(),
            })?;
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().filesystem_write {
            return Err(map_environment_error(
                EnvironmentError::CapabilityUnavailable("filesystem_write"),
            ));
        }
        let mutation = services
            .environment
            .filesystem()
            .write_utf8(raw_path, content)
            .await
            .map_err(map_environment_error)?;
        let operation = match mutation.operation {
            FileMutationOperation::Create => ToolMutationOperation::Create,
            FileMutationOperation::Update => ToolMutationOperation::Update,
        };
        let diff = unified_diff(raw_path, mutation.before.as_deref().unwrap_or(""), content);
        Ok(ToolOutput {
            content: format!("wrote {}", raw_path),
            mutations: vec![ToolMutation {
                path: raw_path.to_string(),
                operation,
                diff: Some(diff),
            }],
        })
    }
}

fn map_environment_error(error: EnvironmentError) -> ToolError {
    match error {
        EnvironmentError::Boundary => ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        },
        EnvironmentError::InvalidPath(reason) if reason.contains("escapes workspace") => {
            ToolError::PermissionDenied { reason }
        }
        EnvironmentError::InvalidPath(reason) => ToolError::InvalidInput { reason },
        EnvironmentError::Cancelled => ToolError::ExecutionFailed {
            reason: "execution cancelled".to_string(),
        },
        EnvironmentError::Timeout(timeout_ms) => ToolError::Timeout { timeout_ms },
        EnvironmentError::CapabilityUnavailable(capability) => ToolError::PermissionDenied {
            reason: format!("execution capability unavailable: {capability}"),
        },
        EnvironmentError::StaleObservation => ToolError::InvalidInput {
            reason: "observation version is stale".to_string(),
        },
        EnvironmentError::NotFound => ToolError::ExecutionFailed {
            reason: "workspace file was not found".to_string(),
        },
        EnvironmentError::Host(reason) => ToolError::ExecutionFailed { reason },
    }
}

fn unified_diff(path: &str, before: &str, after: &str) -> String {
    let mut diff = format!("--- a/{path}\n+++ b/{path}\n@@\n");
    for line in before.lines() {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    if before.is_empty() {
        diff.push_str("-\n");
    }
    for line in after.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if after.is_empty() {
        diff.push_str("+\n");
    }
    diff
}
