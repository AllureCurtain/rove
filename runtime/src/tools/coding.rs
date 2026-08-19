use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::environment::{
    CheckpointFile, EnvironmentError, Observation, VersionedFile, WorkspaceEntryKind,
    WorkspaceTraversal, WorkspaceTraversalOptions,
};
use crate::tools::runtime_context::{RuntimeToolServices, runtime_tool_services};
use rove_core::{
    Tool, ToolContext, ToolDescriptor, ToolError, ToolMutation, ToolMutationOperation, ToolOutput,
};

pub(crate) const MAX_VERSIONED_FILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TOOL_CONTENT_BYTES: usize = 1024 * 1024;
const DEFAULT_PAGE_ENTRIES: usize = 200;
const MAX_PAGE_ENTRIES: usize = 500;
const MAX_DISCOVERY_ENTRIES: usize = 10_000;
const MAX_DISCOVERY_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CHECKPOINT_FILES: usize = 512;
const MAX_CHECKPOINT_CONTENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REWIND_FILES: usize = 64;
const MAX_DIFF_BYTES: usize = 64 * 1024;
const DIFF_CONTEXT_LINES: usize = 3;

#[derive(Default)]
pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "edit_file".to_string(),
            description: "Replace one exact, uniquely observed text occurrence in a workspace file. observation_id and version must come from an explicit offset/limit read_file result for this same path; artifact hashes and search/list observations are invalid. The observation and version must still match.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "minLength": 1, "maxLength": 4096 },
                    "old_text": { "type": "string", "minLength": 1, "maxLength": 262144 },
                    "new_text": { "type": "string", "maxLength": 1048576 },
                    "observation_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                    "version": { "type": "string", "minLength": 1, "maxLength": 128 }
                },
                "required": ["path", "old_text", "new_text", "observation_id", "version"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.fs.edit_exact".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let path = required_string(&args, "path")?;
        let old_text = required_string(&args, "old_text")?;
        let new_text = required_string(&args, "new_text")?;
        let observation_id = required_string(&args, "observation_id")?;
        let version = required_string(&args, "version")?;
        let services = writable_services(ctx)?;
        let current = services
            .environment
            .filesystem()
            .read_versioned(path, MAX_VERSIONED_FILE_BYTES)
            .await
            .map_err(map_environment_error)?;
        let observed =
            require_file_observation(services, path, observation_id, version, &current).await?;
        let observed_payload = services
            .environment
            .observations()
            .payload(&observed.id)
            .await
            .ok_or_else(|| ToolError::InvalidInput {
                reason: "observation payload is unavailable".to_string(),
            })?;
        if !String::from_utf8_lossy(&observed_payload).contains(old_text) {
            return Err(ToolError::InvalidInput {
                reason: "old_text was not present in the observed range".to_string(),
            });
        }
        let before = String::from_utf8(current.bytes).map_err(|_| ToolError::InvalidInput {
            reason: "edit_file requires a UTF-8 file".to_string(),
        })?;
        let occurrences = before.match_indices(old_text).count();
        if occurrences != 1 {
            return Err(ToolError::InvalidInput {
                reason: format!(
                    "old_text must occur exactly once in the current file; found {occurrences}"
                ),
            });
        }
        let after = before.replacen(old_text, new_text, 1);
        if after.len() > MAX_VERSIONED_FILE_BYTES {
            return Err(ToolError::InvalidInput {
                reason: "edited file exceeds the versioned file limit".to_string(),
            });
        }
        services
            .environment
            .filesystem()
            .write_utf8(path, &after)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput {
            content: serde_json::json!({
                "path": normalize_tool_path(path)?,
                "replacements": 1,
                "previous_version": version,
                "version": version_bytes(after.as_bytes())
            })
            .to_string(),
            mutations: vec![ToolMutation {
                path: normalize_tool_path(path)?,
                operation: ToolMutationOperation::Update,
                diff: Some(localized_diff(path, &before, &after)),
            }],
            envelope: None,
        })
    }
}

#[derive(Default)]
pub struct DeletePathTool;

impl DeletePathTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DeletePathTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "delete_path".to_string(),
            description:
                "Delete an observed workspace file or a completely observed bounded directory."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "minLength": 1, "maxLength": 4096 },
                    "observation_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                    "version": { "type": "string", "minLength": 1, "maxLength": 128 },
                    "recursive": { "type": "boolean", "default": false }
                },
                "required": ["path", "observation_id", "version"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.fs.delete_observed".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let path = required_string(&args, "path")?;
        let observation_id = required_string(&args, "observation_id")?;
        let version = required_string(&args, "version")?;
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let services = writable_services(ctx)?;
        let normalized = normalize_tool_path(path)?;
        reject_workspace_root_mutation(&normalized)?;
        let kind = services
            .environment
            .filesystem()
            .path_kind(path)
            .await
            .map_err(map_environment_error)?
            .ok_or_else(|| ToolError::ExecutionFailed {
                reason: "workspace path was not found".to_string(),
            })?;
        let diff = match kind {
            WorkspaceEntryKind::File => {
                let current = services
                    .environment
                    .filesystem()
                    .read_versioned(path, MAX_VERSIONED_FILE_BYTES)
                    .await
                    .map_err(map_environment_error)?;
                require_file_observation(services, path, observation_id, version, &current).await?;
                String::from_utf8(current.bytes)
                    .ok()
                    .map(|before| localized_diff(path, &before, ""))
            }
            WorkspaceEntryKind::Directory => {
                let (current_entries, current_version) =
                    directory_entries_and_version(services, path, true, false, false).await?;
                let observation = services
                    .environment
                    .observations()
                    .require_version(observation_id, version)
                    .await
                    .map_err(map_environment_error)?;
                if observation.source != directory_source(path, true, false, false)?
                    || observation.version != current_version
                    || observation.truncated
                    || observation.start != 0
                    || observation.end != current_entries.entries.len()
                    || !recursive
                {
                    return Err(ToolError::InvalidInput {
                        reason: "recursive directory delete requires a complete current directory observation".to_string(),
                    });
                }
                None
            }
        };
        services
            .environment
            .filesystem()
            .delete_path(path, recursive)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput {
            content: serde_json::json!({ "path": normalized, "deleted": true }).to_string(),
            mutations: vec![ToolMutation {
                path: normalized,
                operation: ToolMutationOperation::Delete,
                diff,
            }],
            envelope: None,
        })
    }
}

#[derive(Default)]
pub struct MovePathTool;

impl MovePathTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for MovePathTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "move_path".to_string(),
            description: "Move or rename an observed workspace file or completely observed bounded directory.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "minLength": 1, "maxLength": 4096 },
                    "to": { "type": "string", "minLength": 1, "maxLength": 4096 },
                    "observation_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                    "version": { "type": "string", "minLength": 1, "maxLength": 128 }
                },
                "required": ["from", "to", "observation_id", "version"],
                "additionalProperties": false
            }),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.fs.move_observed".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let from = required_string(&args, "from")?;
        let to = required_string(&args, "to")?;
        let observation_id = required_string(&args, "observation_id")?;
        let version = required_string(&args, "version")?;
        let services = writable_services(ctx)?;
        let normalized_from = normalize_tool_path(from)?;
        let normalized_to = normalize_tool_path(to)?;
        reject_workspace_root_mutation(&normalized_from)?;
        reject_workspace_root_mutation(&normalized_to)?;
        let comparison_from = path_comparison_key(&normalized_from);
        let comparison_to = path_comparison_key(&normalized_to);
        if comparison_to == comparison_from
            || comparison_to.starts_with(&format!("{comparison_from}/"))
        {
            return Err(ToolError::InvalidInput {
                reason: "move destination must differ from and not be nested under its source"
                    .to_string(),
            });
        }
        let kind = services
            .environment
            .filesystem()
            .path_kind(from)
            .await
            .map_err(map_environment_error)?
            .ok_or_else(|| ToolError::ExecutionFailed {
                reason: "workspace source was not found".to_string(),
            })?;
        match kind {
            WorkspaceEntryKind::File => {
                let current = services
                    .environment
                    .filesystem()
                    .read_versioned(from, MAX_VERSIONED_FILE_BYTES)
                    .await
                    .map_err(map_environment_error)?;
                require_file_observation(services, from, observation_id, version, &current).await?;
            }
            WorkspaceEntryKind::Directory => {
                let (current_entries, current_version) =
                    directory_entries_and_version(services, from, true, false, false).await?;
                let observation = services
                    .environment
                    .observations()
                    .require_version(observation_id, version)
                    .await
                    .map_err(map_environment_error)?;
                if observation.source != directory_source(from, true, false, false)?
                    || observation.version != current_version
                    || observation.truncated
                    || observation.start != 0
                    || observation.end != current_entries.entries.len()
                {
                    return Err(ToolError::InvalidInput {
                        reason: "directory move requires a complete current directory observation"
                            .to_string(),
                    });
                }
            }
        }
        services
            .environment
            .filesystem()
            .move_path(from, to, false)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput {
            content: serde_json::json!({
                "from": normalized_from,
                "to": normalized_to,
                "moved": true
            })
            .to_string(),
            mutations: vec![
                ToolMutation {
                    path: normalized_from,
                    operation: ToolMutationOperation::Delete,
                    diff: None,
                },
                ToolMutation {
                    path: normalized_to,
                    operation: ToolMutationOperation::Create,
                    diff: None,
                },
            ],
            envelope: None,
        })
    }
}

#[derive(Default)]
pub struct ListDirectoryTool;

impl ListDirectoryTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Serialize)]
struct DirectoryPage {
    path: String,
    recursive: bool,
    entries: Vec<DirectoryPageEntry>,
    total_entries: usize,
    scanned_entries: usize,
    ignored_entries: usize,
    hidden_entries: usize,
    sensitive_entries: usize,
    link_entries: usize,
    output_bytes: usize,
    scan_truncated: bool,
    output_truncated: bool,
    observation_id: String,
    version: String,
    truncated: bool,
    continuation: Option<String>,
    artifact_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DirectoryPageEntry {
    path: String,
    kind: WorkspaceEntryKind,
    byte_len: usize,
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_directory".to_string(),
            description: "List workspace directory entries in deterministic lexical pages and return a versioned observation.".to_string(),
            parameters: discovery_schema(false),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("workspace.fs.list".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let limit = page_limit(&args)?;
        let continuation = args.get("continuation").and_then(Value::as_str);
        let include_ignored = args
            .get("include_ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include_hidden = args
            .get("include_hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let services = readable_services(ctx)?;
        let (traversal, version) = directory_entries_and_version(
            services,
            path,
            recursive,
            include_ignored,
            include_hidden,
        )
        .await?;
        let source = directory_source(path, recursive, include_ignored, include_hidden)?;
        let start = continuation_start(
            services,
            continuation,
            "list",
            &source,
            &version,
            traversal.entries.len(),
        )
        .await?;
        directory_page_output(
            services,
            DirectoryPageRequest {
                path,
                recursive,
                traversal,
                version,
                source,
                start,
                limit,
            },
        )
        .await
    }
}

#[derive(Default)]
pub struct GlobPathsTool;

impl GlobPathsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GlobPathsTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "glob_paths".to_string(),
            description:
                "Match workspace paths using a deterministic bounded glob page with continuation."
                    .to_string(),
            parameters: discovery_schema(true),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("workspace.search.glob".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let pattern = required_string(&args, "pattern")?;
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let limit = page_limit(&args)?;
        let continuation = args.get("continuation").and_then(Value::as_str);
        let include_ignored = args
            .get("include_ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include_hidden = args
            .get("include_hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let services = readable_services(ctx)?;
        let (mut traversal, catalog_version) =
            directory_entries_and_version(services, path, true, include_ignored, include_hidden)
                .await?;
        let matcher = compile_workspace_glob(pattern)?;
        traversal.entries = traversal
            .entries
            .into_iter()
            .filter(|entry| matcher.is_match(&entry.relative_path))
            .collect::<Vec<_>>();
        let source = format!(
            "glob:{}|{}|ignored:{}|hidden:{}",
            normalize_tool_path(path)?,
            pattern,
            include_ignored,
            include_hidden
        );
        let start = continuation_start(
            services,
            continuation,
            "glob",
            &source,
            &catalog_version,
            traversal.entries.len(),
        )
        .await?;
        directory_page_output(
            services,
            DirectoryPageRequest {
                path,
                recursive: true,
                traversal,
                version: catalog_version,
                source,
                start,
                limit,
            },
        )
        .await
    }
}

#[derive(Default)]
pub struct WorkspaceCheckpointTool;

impl WorkspaceCheckpointTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WorkspaceCheckpointTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "workspace_checkpoint".to_string(),
            description:
                "Capture a bounded process-local checkpoint for explicit workspace file paths."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1, "maxLength": 4096 },
                        "maxItems": 64
                    }
                },
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("workspace.checkpoint.create".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let services = readable_services(ctx)?;
        if !services.environment.capabilities().workspace_checkpoints {
            return Err(capability_unavailable("workspace_checkpoints"));
        }
        let requested = optional_paths(&args, "paths")?;
        let paths = checkpoint_paths(services, requested.as_deref()).await?;
        let mut files = Vec::with_capacity(paths.len());
        let mut checkpoint_bytes = 0usize;
        for path in paths {
            match services
                .environment
                .filesystem()
                .path_kind(&path)
                .await
                .map_err(map_environment_error)?
            {
                Some(WorkspaceEntryKind::File) => {
                    let remaining = MAX_CHECKPOINT_CONTENT_BYTES.saturating_sub(checkpoint_bytes);
                    let file = services
                        .environment
                        .filesystem()
                        .read_versioned(&path, remaining)
                        .await
                        .map_err(|error| match error {
                            EnvironmentError::ResourceLimit("versioned_file_bytes") => {
                                ToolError::InvalidInput {
                                    reason: "checkpoint content byte limit exceeded".to_string(),
                                }
                            }
                            other => map_environment_error(other),
                        })?;
                    checkpoint_bytes = checkpoint_bytes.saturating_add(file.bytes.len());
                    files.push(CheckpointFile {
                        path,
                        content: Some(file.bytes),
                        version: Some(file.version),
                    });
                }
                Some(WorkspaceEntryKind::Directory) => {
                    return Err(ToolError::InvalidInput {
                        reason: "checkpoint paths must resolve to files; list a directory first and pass explicit files".to_string(),
                    });
                }
                None => files.push(CheckpointFile {
                    path,
                    content: None,
                    version: None,
                }),
            }
        }
        let checkpoint = services
            .environment
            .checkpoints()
            .put(files)
            .await
            .map_err(map_environment_error)?;
        Ok(ToolOutput::text(
            serde_json::json!({
                "checkpoint_id": checkpoint.id,
                "file_count": checkpoint.files.len(),
                "byte_count": checkpoint.byte_count,
                "durable": false
            })
            .to_string(),
        ))
    }
}

#[derive(Default)]
pub struct WorkspaceDiffTool;

impl WorkspaceDiffTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WorkspaceDiffTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "workspace_diff".to_string(),
            description:
                "Return localized bounded diffs for files in a process-local workspace checkpoint."
                    .to_string(),
            parameters: checkpoint_selection_schema(64),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("workspace.checkpoint.diff".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let checkpoint_id = required_string(&args, "checkpoint_id")?;
        let services = readable_services(ctx)?;
        let checkpoint = services
            .environment
            .checkpoints()
            .get(checkpoint_id)
            .await
            .map_err(map_environment_error)?;
        let selected =
            selected_checkpoint_paths(&checkpoint.files, optional_paths(&args, "paths")?)?;
        let mut diffs = Vec::new();
        let mut truncated = false;
        let mut output_bytes = 0usize;
        for path in selected {
            let baseline = checkpoint
                .files
                .get(&path)
                .expect("selected checkpoint path exists");
            let current = read_optional_file(services, &path).await?;
            if baseline.content == current.as_ref().map(|file| file.bytes.clone()) {
                continue;
            }
            let diff = localized_bytes_diff(
                &path,
                baseline.content.as_deref(),
                current.as_ref().map(|file| file.bytes.as_slice()),
            );
            output_bytes = output_bytes.saturating_add(diff.len());
            if output_bytes > MAX_DIFF_BYTES {
                truncated = true;
                break;
            }
            diffs.push(serde_json::json!({ "path": path, "diff": diff }));
        }
        Ok(ToolOutput::text(
            serde_json::json!({
                "checkpoint_id": checkpoint.id,
                "changed_count": diffs.len(),
                "diffs": diffs,
                "truncated": truncated
            })
            .to_string(),
        ))
    }
}

#[derive(Default)]
pub struct WorkspaceRewindTool;

struct RewindAction {
    path: String,
    baseline: Option<String>,
    current: Option<Vec<u8>>,
}

impl WorkspaceRewindTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WorkspaceRewindTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "workspace_rewind".to_string(),
            description: "Restore explicitly selected files from a bounded process-local workspace checkpoint.".to_string(),
            parameters: checkpoint_selection_schema(MAX_REWIND_FILES),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.checkpoint.rewind".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let checkpoint_id = required_string(&args, "checkpoint_id")?;
        let services = writable_services(ctx)?;
        let checkpoint = services
            .environment
            .checkpoints()
            .get(checkpoint_id)
            .await
            .map_err(map_environment_error)?;
        let requested = optional_paths(&args, "paths")?.ok_or_else(|| ToolError::InvalidInput {
            reason: "workspace_rewind requires explicit paths".to_string(),
        })?;
        let selected = selected_checkpoint_paths(&checkpoint.files, Some(requested))?;
        if selected.len() > MAX_REWIND_FILES {
            return Err(ToolError::InvalidInput {
                reason: "rewind path limit exceeded".to_string(),
            });
        }
        let mut actions = Vec::with_capacity(selected.len());
        for path in selected {
            let baseline = checkpoint
                .files
                .get(&path)
                .expect("selected checkpoint path exists");
            let current = read_optional_file(services, &path).await?;
            let baseline = baseline
                .content
                .as_deref()
                .map(|bytes| {
                    std::str::from_utf8(bytes).map(str::to_string).map_err(|_| {
                        ToolError::InvalidInput {
                            reason: format!("checkpoint file is not UTF-8: {path}"),
                        }
                    })
                })
                .transpose()?;
            actions.push(RewindAction {
                path,
                baseline,
                current: current.map(|file| file.bytes),
            });
        }

        let mut mutations = Vec::new();
        for action in actions {
            match action.baseline {
                Some(content) => {
                    services
                        .environment
                        .filesystem()
                        .write_utf8(&action.path, &content)
                        .await
                        .map_err(map_environment_error)?;
                    mutations.push(ToolMutation {
                        path: action.path.clone(),
                        operation: if action.current.is_some() {
                            ToolMutationOperation::Update
                        } else {
                            ToolMutationOperation::Create
                        },
                        diff: Some(localized_bytes_diff(
                            &action.path,
                            action.current.as_deref(),
                            Some(content.as_bytes()),
                        )),
                    });
                }
                None if action.current.is_some() => {
                    services
                        .environment
                        .filesystem()
                        .delete_path(&action.path, false)
                        .await
                        .map_err(map_environment_error)?;
                    mutations.push(ToolMutation {
                        path: action.path.clone(),
                        operation: ToolMutationOperation::Delete,
                        diff: Some(localized_bytes_diff(
                            &action.path,
                            action.current.as_deref(),
                            None,
                        )),
                    });
                }
                None => {}
            }
        }
        Ok(ToolOutput {
            content: serde_json::json!({
                "checkpoint_id": checkpoint.id,
                "rewound_count": mutations.len()
            })
            .to_string(),
            mutations,
            envelope: None,
        })
    }
}

struct DirectoryPageRequest<'a> {
    path: &'a str,
    recursive: bool,
    traversal: WorkspaceTraversal,
    version: String,
    source: String,
    start: usize,
    limit: usize,
}

async fn directory_page_output(
    services: &RuntimeToolServices,
    request: DirectoryPageRequest<'_>,
) -> Result<ToolOutput, ToolError> {
    let DirectoryPageRequest {
        path,
        recursive,
        traversal,
        version,
        source,
        start,
        limit,
    } = request;
    let requested_end = start.saturating_add(limit).min(traversal.entries.len());
    let mut page_entries = traversal.entries[start..requested_end]
        .iter()
        .map(|entry| DirectoryPageEntry {
            path: entry.relative_path.clone(),
            kind: entry.kind,
            byte_len: entry.byte_len,
        })
        .collect::<Vec<_>>();
    let mut output_truncated = false;
    loop {
        let end = start + page_entries.len();
        let candidate = DirectoryPage {
            path: normalize_tool_path(path)?,
            recursive,
            entries: page_entries.clone(),
            total_entries: traversal.entries.len(),
            scanned_entries: traversal.scanned_entries,
            ignored_entries: traversal.ignored_entries,
            hidden_entries: traversal.hidden_entries,
            sensitive_entries: traversal.sensitive_entries,
            link_entries: traversal.link_entries,
            output_bytes: MAX_DISCOVERY_OUTPUT_BYTES,
            scan_truncated: traversal.truncated,
            output_truncated,
            observation_id: format!("sha256:{}", "0".repeat(64)),
            version: version.clone(),
            truncated: end < traversal.entries.len() || traversal.truncated || output_truncated,
            continuation: (end < traversal.entries.len())
                .then(|| format!("page:sha256:{}:{end}", "0".repeat(64))),
            artifact_ref: Some("x".repeat(512)),
        };
        if serde_json::to_vec(&candidate)
            .map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?
            .len()
            <= MAX_DISCOVERY_OUTPUT_BYTES
        {
            break;
        }
        if page_entries.pop().is_none() {
            return Err(ToolError::InvalidInput {
                reason: "discovery metadata exceeds the output byte limit".to_string(),
            });
        }
        output_truncated = true;
    }
    let end = start + page_entries.len();
    let payload =
        serde_json::to_vec(&page_entries).map_err(|error| ToolError::ExecutionFailed {
            reason: error.to_string(),
        })?;
    let page_truncated = end < traversal.entries.len();
    let truncated = page_truncated || traversal.truncated || output_truncated;
    let artifact_ref = if truncated {
        match services.environment.artifacts() {
            Some(sink) => sink.put(&source, &payload).await.ok().flatten(),
            None => None,
        }
    } else {
        None
    };
    let mut observation = Observation::from_bytes(
        source.clone(),
        start,
        &payload,
        version.clone(),
        truncated,
        artifact_ref.clone(),
    );
    observation.start = start;
    observation.end = end;
    services
        .environment
        .observations()
        .put_with_payload(observation.clone(), payload)
        .await
        .map_err(map_environment_error)?;
    let continuation = page_truncated.then(|| format!("page:{}:{end}", observation.id));
    let result = DirectoryPage {
        path: normalize_tool_path(path)?,
        recursive,
        entries: page_entries,
        total_entries: traversal.entries.len(),
        scanned_entries: traversal.scanned_entries,
        ignored_entries: traversal.ignored_entries,
        hidden_entries: traversal.hidden_entries,
        sensitive_entries: traversal.sensitive_entries,
        link_entries: traversal.link_entries,
        output_bytes: 0,
        scan_truncated: traversal.truncated,
        output_truncated,
        observation_id: observation.id,
        version,
        truncated,
        continuation,
        artifact_ref,
    };
    let (result, encoded) = exact_directory_output(result)?;
    debug_assert_eq!(result.output_bytes, encoded.len());
    debug_assert!(encoded.len() <= MAX_DISCOVERY_OUTPUT_BYTES);
    Ok(ToolOutput::text(encoded))
}

fn exact_directory_output(mut result: DirectoryPage) -> Result<(DirectoryPage, String), ToolError> {
    for _ in 0..4 {
        let encoded =
            serde_json::to_string(&result).map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?;
        if result.output_bytes == encoded.len() {
            return Ok((result, encoded));
        }
        result.output_bytes = encoded.len();
    }
    let encoded = serde_json::to_string(&result).map_err(|error| ToolError::ExecutionFailed {
        reason: error.to_string(),
    })?;
    Ok((result, encoded))
}

async fn continuation_start(
    services: &RuntimeToolServices,
    continuation: Option<&str>,
    _kind: &str,
    source: &str,
    version: &str,
    total: usize,
) -> Result<usize, ToolError> {
    let Some(token) = continuation else {
        return Ok(0);
    };
    let Some(rest) = token.strip_prefix("page:") else {
        return Err(ToolError::InvalidInput {
            reason: "invalid continuation token".to_string(),
        });
    };
    let Some((observation_id, raw_start)) = rest.rsplit_once(':') else {
        return Err(ToolError::InvalidInput {
            reason: "invalid continuation token".to_string(),
        });
    };
    let start = raw_start
        .parse::<usize>()
        .map_err(|_| ToolError::InvalidInput {
            reason: "invalid continuation cursor".to_string(),
        })?;
    if start > total {
        return Err(ToolError::InvalidInput {
            reason: "continuation cursor exceeds current result set".to_string(),
        });
    }
    let observation = services
        .environment
        .observations()
        .require_version(observation_id, version)
        .await
        .map_err(map_environment_error)?;
    if observation.source != source || observation.end != start || !observation.truncated {
        return Err(ToolError::InvalidInput {
            reason: "continuation does not match the current discovery request".to_string(),
        });
    }
    Ok(start)
}

async fn directory_entries_and_version(
    services: &RuntimeToolServices,
    path: &str,
    recursive: bool,
    include_ignored: bool,
    include_hidden: bool,
) -> Result<(WorkspaceTraversal, String), ToolError> {
    let traversal = services
        .environment
        .filesystem()
        .traverse_entries(
            Some(path),
            WorkspaceTraversalOptions {
                recursive,
                include_ignored,
                include_hidden,
            },
            MAX_DISCOVERY_ENTRIES,
        )
        .await
        .map_err(map_environment_error)?;
    let identity = traversal
        .entries
        .iter()
        .map(|entry| {
            format!(
                "{}|{:?}|{}",
                entry.relative_path, entry.kind, entry.byte_len
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((traversal, version_bytes(identity.as_bytes())))
}

async fn checkpoint_paths(
    services: &RuntimeToolServices,
    requested: Option<&[String]>,
) -> Result<Vec<String>, ToolError> {
    let mut paths = BTreeSet::new();
    if let Some(requested) = requested {
        for path in requested {
            paths.insert(normalize_tool_path(path)?);
        }
    } else {
        let entries = services
            .environment
            .filesystem()
            .list_entries(None, true, MAX_CHECKPOINT_FILES.saturating_add(1))
            .await
            .map_err(map_environment_error)?;
        for entry in entries {
            if entry.kind == WorkspaceEntryKind::File {
                paths.insert(entry.relative_path);
            }
        }
    }
    if paths.len() > MAX_CHECKPOINT_FILES {
        return Err(ToolError::InvalidInput {
            reason: "checkpoint file limit exceeded".to_string(),
        });
    }
    Ok(paths.into_iter().collect())
}

fn selected_checkpoint_paths(
    files: &BTreeMap<String, CheckpointFile>,
    requested: Option<Vec<String>>,
) -> Result<Vec<String>, ToolError> {
    let selected = match requested {
        Some(paths) => {
            let mut selected = Vec::with_capacity(paths.len());
            let mut seen = BTreeSet::new();
            for path in paths {
                let normalized = normalize_tool_path(&path)?;
                if !seen.insert(normalized.clone()) {
                    return Err(ToolError::InvalidInput {
                        reason: format!("checkpoint path is duplicated: {normalized}"),
                    });
                }
                selected.push(normalized);
            }
            selected
        }
        None => files.keys().cloned().collect(),
    };
    for path in &selected {
        if !files.contains_key(path) {
            return Err(ToolError::InvalidInput {
                reason: format!("path is not present in checkpoint: {path}"),
            });
        }
    }
    Ok(selected)
}

async fn read_optional_file(
    services: &RuntimeToolServices,
    path: &str,
) -> Result<Option<VersionedFile>, ToolError> {
    match services
        .environment
        .filesystem()
        .path_kind(path)
        .await
        .map_err(map_environment_error)?
    {
        Some(WorkspaceEntryKind::File) => services
            .environment
            .filesystem()
            .read_versioned(path, MAX_VERSIONED_FILE_BYTES)
            .await
            .map(Some)
            .map_err(map_environment_error),
        Some(WorkspaceEntryKind::Directory) => Err(ToolError::InvalidInput {
            reason: format!("checkpoint path became a directory: {path}"),
        }),
        None => Ok(None),
    }
}

pub(crate) async fn require_file_observation(
    services: &RuntimeToolServices,
    path: &str,
    observation_id: &str,
    version: &str,
    current: &VersionedFile,
) -> Result<Observation, ToolError> {
    let observation = services
        .environment
        .observations()
        .require_version(observation_id, version)
        .await
        .map_err(map_environment_error)?;
    if observation.source != file_source(path)? {
        return Err(ToolError::InvalidInput {
            reason: "observation source does not match the mutation path".to_string(),
        });
    }
    if current.version != version {
        return Err(ToolError::InvalidInput {
            reason: "observation version is stale".to_string(),
        });
    }
    Ok(observation)
}

pub(crate) fn file_source(path: &str) -> Result<String, ToolError> {
    Ok(format!("file:{}", normalize_tool_path(path)?))
}

fn directory_source(
    path: &str,
    recursive: bool,
    include_ignored: bool,
    include_hidden: bool,
) -> Result<String, ToolError> {
    Ok(format!(
        "directory:{}|recursive:{recursive}|ignored:{include_ignored}|hidden:{include_hidden}",
        normalize_tool_path(path)?,
    ))
}

pub(crate) fn normalize_tool_path(raw: &str) -> Result<String, ToolError> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ToolError::PermissionDenied {
            reason: "absolute paths are outside the workspace tool contract".to_string(),
        });
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ToolError::PermissionDenied {
                        reason: "path escapes workspace".to_string(),
                    });
                }
            }
            _ => {
                return Err(ToolError::PermissionDenied {
                    reason: "path escapes workspace".to_string(),
                });
            }
        }
    }
    let value = normalized.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        if raw == "." {
            Ok(".".to_string())
        } else {
            Err(ToolError::InvalidInput {
                reason: "path must not be empty".to_string(),
            })
        }
    } else {
        Ok(value)
    }
}

fn reject_workspace_root_mutation(path: &str) -> Result<(), ToolError> {
    if path == "." {
        return Err(ToolError::PermissionDenied {
            reason: "workspace root mutation is not allowed".to_string(),
        });
    }
    Ok(())
}

fn path_comparison_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}

pub(crate) fn localized_diff(path: &str, before: &str, after: &str) -> String {
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();
    let prefix = before_lines
        .iter()
        .zip(after_lines.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = before_lines[prefix..]
        .iter()
        .rev()
        .zip(after_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let before_start = prefix.saturating_sub(DIFF_CONTEXT_LINES);
    let after_start = before_start;
    let before_end = before_lines
        .len()
        .saturating_sub(suffix)
        .saturating_add(DIFF_CONTEXT_LINES)
        .min(before_lines.len());
    let after_end = after_lines
        .len()
        .saturating_sub(suffix)
        .saturating_add(DIFF_CONTEXT_LINES)
        .min(after_lines.len());
    let mut diff = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        before_start + 1,
        before_end.saturating_sub(before_start),
        after_start + 1,
        after_end.saturating_sub(after_start)
    );
    let context_prefix_end = prefix.min(before_end);
    for line in &before_lines[before_start..context_prefix_end] {
        push_diff_line(&mut diff, ' ', line);
    }
    for line in &before_lines[prefix..before_lines.len().saturating_sub(suffix)] {
        push_diff_line(&mut diff, '-', line);
        if diff.len() >= MAX_DIFF_BYTES {
            return truncate_utf8(diff, MAX_DIFF_BYTES, "\n... diff truncated\n");
        }
    }
    for line in &after_lines[prefix..after_lines.len().saturating_sub(suffix)] {
        push_diff_line(&mut diff, '+', line);
        if diff.len() >= MAX_DIFF_BYTES {
            return truncate_utf8(diff, MAX_DIFF_BYTES, "\n... diff truncated\n");
        }
    }
    let suffix_start = before_lines.len().saturating_sub(suffix);
    for line in &before_lines[suffix_start..before_end] {
        push_diff_line(&mut diff, ' ', line);
    }
    truncate_utf8(diff, MAX_DIFF_BYTES, "\n... diff truncated\n")
}

fn localized_bytes_diff(path: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> String {
    match (
        before.and_then(|bytes| std::str::from_utf8(bytes).ok()),
        after.and_then(|bytes| std::str::from_utf8(bytes).ok()),
    ) {
        (Some(before), Some(after)) => localized_diff(path, before, after),
        (None, Some(after)) => localized_diff(path, "", after),
        (Some(before), None) => localized_diff(path, before, ""),
        (None, None) => format!("binary or unavailable change: {path}"),
    }
}

fn push_diff_line(diff: &mut String, prefix: char, line: &str) {
    diff.push(prefix);
    diff.push_str(line);
    diff.push('\n');
}

fn truncate_utf8(mut value: String, max_bytes: usize, suffix: &str) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let target = max_bytes.saturating_sub(suffix.len());
    let mut end = target.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(suffix);
    value
}

fn version_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn compile_workspace_glob(pattern: &str) -> Result<globset::GlobMatcher, ToolError> {
    if pattern.is_empty() || pattern.contains('\0') {
        return Err(ToolError::InvalidInput {
            reason: "glob pattern must be non-empty and contain no NUL bytes".to_string(),
        });
    }
    let normalized = pattern.replace('\\', "/");
    if is_absolute_workspace_glob(&normalized)
        || normalized.split('/').any(|component| component == "..")
    {
        return Err(ToolError::PermissionDenied {
            reason: "glob pattern escapes workspace".to_string(),
        });
    }
    globset::GlobBuilder::new(&normalized)
        .literal_separator(true)
        .backslash_escape(false)
        .case_insensitive(cfg!(windows))
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| ToolError::InvalidInput {
            reason: format!("invalid glob: {error}"),
        })
}

fn is_absolute_workspace_glob(pattern: &str) -> bool {
    if Path::new(pattern).is_absolute() || pattern.starts_with("//") {
        return true;
    }

    let bytes = pattern.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn discovery_schema(include_pattern: bool) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "path".to_string(),
        serde_json::json!({ "type": "string", "minLength": 1, "maxLength": 4096, "default": "." }),
    );
    properties.insert(
        "recursive".to_string(),
        serde_json::json!({ "type": "boolean", "default": false }),
    );
    properties.insert(
        "include_ignored".to_string(),
        serde_json::json!({ "type": "boolean", "default": false }),
    );
    properties.insert(
        "include_hidden".to_string(),
        serde_json::json!({ "type": "boolean", "default": false }),
    );
    properties.insert(
        "limit".to_string(),
        serde_json::json!({ "type": "integer", "minimum": 1, "maximum": 500, "default": 200 }),
    );
    properties.insert(
        "continuation".to_string(),
        serde_json::json!({ "type": "string", "minLength": 1, "maxLength": 512 }),
    );
    if include_pattern {
        properties.insert(
            "pattern".to_string(),
            serde_json::json!({ "type": "string", "minLength": 1, "maxLength": 4096 }),
        );
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": if include_pattern { vec!["pattern"] } else { Vec::<&str>::new() },
        "additionalProperties": false
    })
}

fn checkpoint_selection_schema(max_items: usize) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "checkpoint_id": { "type": "string", "minLength": 1, "maxLength": 256 },
            "paths": {
                "type": "array",
                "items": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "maxItems": max_items
            }
        },
        "required": ["checkpoint_id"],
        "additionalProperties": false
    })
}

fn page_limit(args: &Value) -> Result<usize, ToolError> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_PAGE_ENTRIES as u64);
    usize::try_from(limit)
        .ok()
        .filter(|limit| (1..=MAX_PAGE_ENTRIES).contains(limit))
        .ok_or_else(|| ToolError::InvalidInput {
            reason: "page limit must be between 1 and 500".to_string(),
        })
}

fn optional_paths(args: &Value, field: &str) -> Result<Option<Vec<String>>, ToolError> {
    args.get(field)
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| ToolError::InvalidArgs {
                    reason: format!("Argument {field} must be array"),
                })?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| ToolError::InvalidArgs {
                            reason: format!("Argument {field} entries must be strings"),
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
}

fn required_string<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs {
            reason: format!("Missing required argument: {field}"),
        })
}

fn readable_services<'a>(ctx: &'a ToolContext<'_>) -> Result<&'a RuntimeToolServices, ToolError> {
    let services = runtime_tool_services(ctx)?;
    if !services.environment.capabilities().filesystem_read {
        return Err(capability_unavailable("filesystem_read"));
    }
    if !services.environment.capabilities().observations {
        return Err(capability_unavailable("observations"));
    }
    Ok(services)
}

fn writable_services<'a>(ctx: &'a ToolContext<'_>) -> Result<&'a RuntimeToolServices, ToolError> {
    let services = readable_services(ctx)?;
    if !services.environment.capabilities().filesystem_write {
        return Err(capability_unavailable("filesystem_write"));
    }
    Ok(services)
}

fn capability_unavailable(capability: &str) -> ToolError {
    ToolError::PermissionDenied {
        reason: format!("execution capability unavailable: {capability}"),
    }
}

pub(crate) fn map_environment_error(error: EnvironmentError) -> ToolError {
    match error {
        EnvironmentError::Boundary => ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        },
        EnvironmentError::InvalidPath(reason) if reason.contains("escapes workspace") => {
            ToolError::PermissionDenied { reason }
        }
        EnvironmentError::InvalidPath(reason) | EnvironmentError::Conflict(reason) => {
            ToolError::InvalidInput { reason }
        }
        EnvironmentError::Cancelled => ToolError::ExecutionFailed {
            reason: "execution cancelled".to_string(),
        },
        EnvironmentError::Timeout(timeout_ms) => ToolError::Timeout { timeout_ms },
        EnvironmentError::CapabilityUnavailable(capability) => capability_unavailable(capability),
        EnvironmentError::StaleObservation => ToolError::InvalidInput {
            reason: "observation version is stale".to_string(),
        },
        EnvironmentError::NotFound => ToolError::ExecutionFailed {
            reason: "workspace file was not found".to_string(),
        },
        EnvironmentError::ResourceNotFound(resource) => ToolError::ExecutionFailed {
            reason: format!("execution resource was not found: {resource}"),
        },
        EnvironmentError::ResourceLimit(resource) => ToolError::InvalidInput {
            reason: format!("execution resource limit reached: {resource}"),
        },
        EnvironmentError::Host(reason) => ToolError::ExecutionFailed { reason },
    }
}

#[cfg(test)]
mod productization_tests {
    use super::*;

    #[test]
    fn edit_schema_requires_a_same_file_structured_read_observation() {
        let description = EditFileTool::new().schema().description;

        assert!(description.contains("explicit offset/limit read_file"));
        assert!(description.contains("same path"));
        assert!(description.contains("search/list observations are invalid"));
    }

    #[test]
    fn maintained_globs_cover_recursive_braces_and_character_classes() {
        let recursive = compile_workspace_glob("**/*.rs").unwrap();
        let braces = compile_workspace_glob("src/*.{rs,toml}").unwrap();
        let class = compile_workspace_glob("tests/case[0-9].txt").unwrap();
        assert!(recursive.is_match("runtime/src/lib.rs"));
        assert!(braces.is_match("src/main.rs"));
        assert!(braces.is_match("src/config.toml"));
        assert!(class.is_match("tests/case7.txt"));
        assert!(!class.is_match("tests/casex.txt"));
    }

    #[test]
    fn glob_escape_and_absolute_patterns_fail_closed() {
        assert!(matches!(
            compile_workspace_glob("../*.rs"),
            Err(ToolError::PermissionDenied { .. })
        ));
        assert!(matches!(
            compile_workspace_glob("C:/secret/*"),
            Err(ToolError::PermissionDenied { .. })
        ));
        assert!(matches!(
            compile_workspace_glob("\\\\server\\share\\*.rs"),
            Err(ToolError::PermissionDenied { .. })
        ));
    }
}
