use std::path::PathBuf;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::environment::{
    EnvironmentError, FileMutationOperation, Observation, WorkspaceEntryKind,
};
use crate::tools::coding::{
    MAX_TOOL_CONTENT_BYTES, MAX_VERSIONED_FILE_BYTES, file_source, localized_diff,
    map_environment_error, normalize_tool_path, require_file_observation,
};
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolMutation, ToolMutationOperation, ToolOutput};

/// Read a UTF-8 file inside the workspace.
pub struct FsReadTool;

const DEFAULT_READ_BYTES: usize = 64 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Serialize)]
struct ReadFileOutput {
    path: String,
    content: String,
    offset: usize,
    end: usize,
    total_bytes: usize,
    version: String,
    observation_id: String,
    truncated: bool,
    continuation: Option<String>,
    artifact_ref: Option<String>,
}

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
            description: "Read a bounded UTF-8 range from a workspace file. Use offset/limit or continuation for structured observation metadata required by exact mutations; a complete small legacy path-only read remains plain text.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 4096,
                        "description": "Workspace-relative file path"
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 16777216,
                        "description": "Zero-based UTF-8 byte offset"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 262144,
                        "default": 65536,
                        "description": "Maximum bytes returned in this observation"
                    },
                    "continuation": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 512,
                        "description": "Continuation returned by an earlier read of the same unchanged file"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
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
        let file = services
            .environment
            .filesystem()
            .read_versioned(raw_path, MAX_VERSIONED_FILE_BYTES)
            .await
            .map_err(map_environment_error)?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_READ_BYTES as u64);
        let limit = usize::try_from(limit)
            .ok()
            .filter(|value| (1..=MAX_READ_BYTES).contains(value))
            .ok_or_else(|| ToolError::InvalidInput {
                reason: "read limit must be between 1 and 262144 bytes".to_string(),
            })?;
        let source = file_source(raw_path)?;
        let explicit_v2 = args.get("offset").is_some()
            || args.get("limit").is_some()
            || args.get("continuation").is_some();
        let offset = if let Some(token) = args.get("continuation").and_then(Value::as_str) {
            if args.get("offset").is_some() {
                return Err(ToolError::InvalidInput {
                    reason: "offset and continuation are mutually exclusive".to_string(),
                });
            }
            let observation_id =
                token
                    .strip_prefix("read:")
                    .ok_or_else(|| ToolError::InvalidInput {
                        reason: "invalid read continuation".to_string(),
                    })?;
            let observation = services
                .environment
                .observations()
                .require_version(observation_id, &file.version)
                .await
                .map_err(map_environment_error)?;
            if observation.source != source || !observation.truncated {
                return Err(ToolError::InvalidInput {
                    reason: "read continuation does not match this file".to_string(),
                });
            }
            observation.end
        } else {
            args.get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .try_into()
                .map_err(|_| ToolError::InvalidInput {
                    reason: "read offset is too large".to_string(),
                })?
        };
        if offset > file.bytes.len() {
            return Err(ToolError::InvalidInput {
                reason: "read offset exceeds file length".to_string(),
            });
        }
        let full = std::str::from_utf8(&file.bytes).map_err(|_| ToolError::InvalidInput {
            reason: "read_file requires a UTF-8 file".to_string(),
        })?;
        if !full.is_char_boundary(offset) {
            return Err(ToolError::InvalidInput {
                reason: "read offset must be on a UTF-8 character boundary".to_string(),
            });
        }
        let mut end = offset.saturating_add(limit).min(file.bytes.len());
        while end > offset && !full.is_char_boundary(end) {
            end -= 1;
        }
        if end == offset && offset < file.bytes.len() {
            return Err(ToolError::InvalidInput {
                reason: "read limit is too small for the next UTF-8 character".to_string(),
            });
        }
        let bytes = &file.bytes[offset..end];
        let truncated = end < file.bytes.len();
        let artifact_ref = if truncated && services.environment.capabilities().artifact_projection {
            match services.environment.artifacts() {
                Some(sink) => sink.put(&source, &file.bytes).await.ok().flatten(),
                None => None,
            }
        } else {
            None
        };
        let mut observation = Observation::from_bytes(
            source,
            offset,
            bytes,
            file.version.clone(),
            truncated,
            artifact_ref.clone(),
        );
        observation.end = end;
        services
            .environment
            .observations()
            .put_with_payload(observation.clone(), bytes.to_vec())
            .await
            .map_err(map_environment_error)?;
        let content = std::str::from_utf8(bytes)
            .expect("range boundaries were checked")
            .to_string();
        if !explicit_v2 && !truncated && offset == 0 {
            return Ok(ToolOutput::text(content));
        }
        Ok(ToolOutput::text(
            serde_json::to_string(&ReadFileOutput {
                path: normalize_tool_path(raw_path)?,
                content,
                offset,
                end,
                total_bytes: file.bytes.len(),
                version: file.version,
                observation_id: observation.id.clone(),
                truncated,
                continuation: truncated.then(|| format!("read:{}", observation.id)),
                artifact_ref,
            })
            .map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?,
        ))
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
            description: "Create a UTF-8 workspace file. Existing files require explicit mode=overwrite; optional observation/version rejects stale compatible overwrite.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 4096,
                        "description": "Workspace-relative file path"
                    },
                    "content": {
                        "type": "string",
                        "maxLength": 1048576,
                        "description": "File content to write"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["create", "overwrite"],
                        "default": "create",
                        "description": "Create-first by default; overwrite must be explicit"
                    },
                    "observation_id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 256
                    },
                    "version": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
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
        if content.len() > MAX_TOOL_CONTENT_BYTES {
            return Err(ToolError::InvalidInput {
                reason: "write content exceeds the tool content limit".to_string(),
            });
        }
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("create");
        let existing_kind = services
            .environment
            .filesystem()
            .path_kind(raw_path)
            .await
            .map_err(map_environment_error)?;
        if existing_kind == Some(WorkspaceEntryKind::Directory) {
            return Err(ToolError::InvalidInput {
                reason: "write target is a directory".to_string(),
            });
        }
        if existing_kind.is_some() && mode != "overwrite" {
            return Err(ToolError::InvalidInput {
                reason:
                    "write_file is create-first; use mode=overwrite explicitly for an existing file"
                        .to_string(),
            });
        }
        let observation_id = args.get("observation_id").and_then(Value::as_str);
        let version = args.get("version").and_then(Value::as_str);
        if observation_id.is_some() != version.is_some() {
            return Err(ToolError::InvalidInput {
                reason: "observation_id and version must be supplied together".to_string(),
            });
        }
        if let (Some(observation_id), Some(version)) = (observation_id, version) {
            let current = services
                .environment
                .filesystem()
                .read_versioned(raw_path, MAX_VERSIONED_FILE_BYTES)
                .await
                .map_err(map_environment_error)?;
            require_file_observation(services, raw_path, observation_id, version, &current).await?;
        }
        let mutation = if mode == "create" {
            services
                .environment
                .filesystem()
                .create_utf8(raw_path, content)
                .await
        } else {
            services
                .environment
                .filesystem()
                .write_utf8(raw_path, content)
                .await
        }
        .map_err(map_environment_error)?;
        let normalized_path = normalize_tool_path(raw_path)?;
        let operation = match mutation.operation {
            FileMutationOperation::Create => ToolMutationOperation::Create,
            FileMutationOperation::Update => ToolMutationOperation::Update,
        };
        let diff = localized_diff(
            &normalized_path,
            mutation.before.as_deref().unwrap_or(""),
            content,
        );
        let new_version = {
            use sha2::{Digest, Sha256};
            format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
        };
        Ok(ToolOutput {
            content: serde_json::json!({
                "path": normalized_path.clone(),
                "mode": mode,
                "version": new_version
            })
            .to_string(),
            mutations: vec![ToolMutation {
                path: normalized_path,
                operation,
                diff: Some(diff),
            }],
        })
    }
}
