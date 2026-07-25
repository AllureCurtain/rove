use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::boundary::{resolve_workspace_read_path, resolve_workspace_write_path};
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolMutation, ToolMutationOperation, ToolOutput};

/// Read a UTF-8 file inside the workspace.
pub struct FsReadTool {
    root: PathBuf,
}

impl FsReadTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
            capability: None,
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_path = args
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: path".to_string(),
            })?;
        let path = resolve_workspace_read_path(&self.root, raw_path)?;
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    reason: e.to_string(),
                })?;
        Ok(ToolOutput::text(content))
    }
}

/// Write a UTF-8 file inside the workspace.
pub struct FsWriteTool {
    root: PathBuf,
}

impl FsWriteTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
            capability: None,
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
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
        let path = resolve_workspace_write_path(&self.root, raw_path)?;
        let before = match tokio::fs::read_to_string(&path).await {
            Ok(content) => Some(content),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(ToolError::ExecutionFailed {
                    reason: err.to_string(),
                });
            }
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    reason: e.to_string(),
                })?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                reason: e.to_string(),
            })?;
        let operation = if before.is_some() {
            ToolMutationOperation::Update
        } else {
            ToolMutationOperation::Create
        };
        let diff = unified_diff(raw_path, before.as_deref().unwrap_or(""), content);
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
