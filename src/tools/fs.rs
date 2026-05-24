use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

fn resolve_workspace_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        return Err(ToolError::InvalidInput {
            reason: "absolute paths are not allowed".to_string(),
        });
    }

    let joined = root.join(path);
    let parent = joined.parent().unwrap_or(root);
    let canonical_parent = parent.canonicalize().map_err(|e| ToolError::InvalidInput {
        reason: format!("invalid path parent: {e}"),
    })?;

    if !canonical_parent.starts_with(root) {
        return Err(ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        });
    }

    Ok(joined)
}

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
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs_read".to_string(),
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
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_path = args
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: path".to_string(),
            })?;
        let path = resolve_workspace_path(&self.root, raw_path)?;
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    reason: e.to_string(),
                })?;
        Ok(ToolOutput { content })
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
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs_write".to_string(),
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
        let path = resolve_workspace_path(&self.root, raw_path)?;
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
        Ok(ToolOutput {
            content: format!("wrote {}", raw_path),
        })
    }
}
