//! Runtime-owned host authority for filesystem and process operations.
//!
//! Built-in tools depend on these ports instead of importing host APIs. The
//! local adapter is the only implementation that touches the process host;
//! deterministic tests can use the in-memory adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::boundary::{resolve_workspace_read_path, resolve_workspace_write_path};
use crate::workspace::{Workspace, WorkspaceKind};

const MAX_IN_MEMORY_FILES: usize = 4_096;
const MAX_FILE_MUTATION_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 512;
const MAX_OBSERVATION_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_ARTIFACT_PROJECTIONS: usize = 512;
const MAX_ARTIFACT_PROJECTION_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHECKPOINTS: usize = 32;
const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
const MAX_BACKGROUND_PROCESSES: usize = 64;
const MAX_DIRECTORY_MUTATION_ENTRIES: usize = 4_096;
const TRAVERSAL_SCAN_MULTIPLIER: usize = 8;
const STDIO_PROCESS_REAP_TIMEOUT: Duration = Duration::from_secs(5);
const STDIO_PROCESS_REAP_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionEnvironmentIdentity {
    pub adapter: String,
    pub workspace_kind: WorkspaceKind,
    /// A stable redacted identity. The canonical local path is never persisted
    /// in this structure or in resume diagnostics.
    pub workspace_digest: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ExecutionCapabilities {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub process_run: bool,
    pub process_stdio: bool,
    pub observations: bool,
    pub process_background: bool,
    pub process_pty: bool,
    pub workspace_checkpoints: bool,
    pub artifact_projection: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    #[error("invalid workspace path: {0}")]
    InvalidPath(String),
    #[error("workspace path is outside the execution boundary")]
    Boundary,
    #[error("workspace file was not found")]
    NotFound,
    #[error("execution capability is unavailable: {0}")]
    CapabilityUnavailable(&'static str),
    #[error("execution timed out after {0} ms")]
    Timeout(u64),
    #[error("execution was cancelled")]
    Cancelled,
    #[error("observation version is stale")]
    StaleObservation,
    #[error("execution resource was not found: {0}")]
    ResourceNotFound(&'static str),
    #[error("execution resource limit reached: {0}")]
    ResourceLimit(&'static str),
    #[error("execution conflict: {0}")]
    Conflict(String),
    #[error("host operation failed: {0}")]
    Host(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutation {
    pub before: Option<String>,
    pub operation: FileMutationOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMutationOperation {
    Create,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileEntry {
    pub relative_path: String,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFileRead {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub relative_path: String,
    pub kind: WorkspaceEntryKind,
    pub byte_len: usize,
}

/// Operator/model-selected traversal behavior. Ignored and hidden paths are
/// independent opt-ins; sensitive paths and links are never traversed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceTraversalOptions {
    pub recursive: bool,
    pub include_ignored: bool,
    pub include_hidden: bool,
}

/// Deterministic bounded traversal plus explicit exclusion/truncation facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceTraversal {
    pub entries: Vec<WorkspaceEntry>,
    pub scanned_entries: usize,
    pub ignored_entries: usize,
    pub hidden_entries: usize,
    pub sensitive_entries: usize,
    pub link_entries: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedFile {
    pub bytes: Vec<u8>,
    pub version: String,
}

#[async_trait]
pub trait WorkspaceFileSystem: Send + Sync {
    fn root(&self) -> &Path;
    async fn read_utf8(&self, raw_path: &str) -> Result<String, EnvironmentError>;
    async fn write_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError>;
    async fn create_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError>;
    async fn list_files(
        &self,
        raw_path: Option<&str>,
        max_files: usize,
    ) -> Result<Vec<WorkspaceFileEntry>, EnvironmentError>;
    async fn read_relative_bytes(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> Result<BoundedFileRead, EnvironmentError>;
    async fn read_versioned(
        &self,
        raw_path: &str,
        max_bytes: usize,
    ) -> Result<VersionedFile, EnvironmentError>;
    async fn path_kind(
        &self,
        raw_path: &str,
    ) -> Result<Option<WorkspaceEntryKind>, EnvironmentError>;
    async fn list_entries(
        &self,
        raw_path: Option<&str>,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<WorkspaceEntry>, EnvironmentError>;
    async fn traverse_entries(
        &self,
        raw_path: Option<&str>,
        options: WorkspaceTraversalOptions,
        max_entries: usize,
    ) -> Result<WorkspaceTraversal, EnvironmentError> {
        let entries = self
            .list_entries(raw_path, options.recursive, max_entries.saturating_add(1))
            .await?;
        let truncated = entries.len() > max_entries;
        Ok(WorkspaceTraversal {
            scanned_entries: entries.len(),
            entries: entries.into_iter().take(max_entries).collect(),
            truncated,
            ..WorkspaceTraversal::default()
        })
    }
    async fn delete_path(&self, raw_path: &str, recursive: bool) -> Result<(), EnvironmentError>;
    async fn move_path(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<(), EnvironmentError>;
}

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub clear_environment: bool,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundProcessStatus {
    Running,
    Exited,
    Terminated,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundProcessStarted {
    pub process_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundProcessOutput {
    pub process_id: String,
    pub status: BackgroundProcessStatus,
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_cursor: usize,
    pub stderr_cursor: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_has_more: bool,
    pub stderr_has_more: bool,
    pub output_complete: bool,
}

#[async_trait]
pub trait ProcessHost: Send + Sync {
    async fn run(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<ProcessOutput, EnvironmentError>;
    async fn spawn_stdio(
        &self,
        program: &str,
        args: &[String],
        environment: &[(String, String)],
    ) -> Result<StdioChild, EnvironmentError>;
    async fn spawn_background(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<BackgroundProcessStarted, EnvironmentError>;
    async fn poll_background(
        &self,
        process_id: &str,
        stdout_cursor: usize,
        stderr_cursor: usize,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessOutput, EnvironmentError>;
    async fn terminate_background(&self, process_id: &str) -> Result<(), EnvironmentError>;
}

pub struct StdioChild {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    child: Option<StdioProcessGuard>,
}

impl StdioChild {
    pub fn into_parts(mut self) -> (ChildStdin, ChildStdout, ChildStderr, StdioProcessGuard) {
        let child = self.child.take().expect("stdio child process is present");
        (self.stdin, self.stdout, self.stderr, child)
    }

    pub fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.kill();
        }
    }
}

pub struct StdioProcessGuard {
    child: Option<Child>,
}

impl StdioProcessGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for StdioProcessGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = child.start_kill();

        // `start_kill` only requests termination. On Unix the exited child
        // remains a zombie until its parent observes the status, so dropping
        // it immediately can leave registry-owned MCP processes visible and
        // consume a process-table entry. Reap independently of Tokio: registry
        // drop can happen while the runtime thread is blocked or shutting down.
        let _ = std::thread::Builder::new()
            .name("rove-stdio-reaper".to_string())
            .spawn(move || {
                let deadline = Instant::now() + STDIO_PROCESS_REAP_TIMEOUT;
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) | Err(_) => break,
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(STDIO_PROCESS_REAP_POLL_INTERVAL);
                        }
                        Ok(None) => {
                            let _ = child.start_kill();
                            break;
                        }
                    }
                }
            });
    }
}

#[async_trait]
pub trait ArtifactSink: Send + Sync {
    async fn put(&self, source: &str, bytes: &[u8]) -> Result<Option<String>, EnvironmentError>;
    async fn get(&self, artifact_ref: &str) -> Result<Vec<u8>, EnvironmentError>;
}

#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    fn identity(&self) -> &ExecutionEnvironmentIdentity;
    fn filesystem(&self) -> &dyn WorkspaceFileSystem;
    fn processes(&self) -> &dyn ProcessHost;
    fn artifacts(&self) -> Option<&dyn ArtifactSink>;
    fn observations(&self) -> &ObservationStore;
    fn checkpoints(&self) -> &WorkspaceCheckpointStore;
    fn capabilities(&self) -> &ExecutionCapabilities;
}

pub struct LocalExecutionEnvironment {
    identity: ExecutionEnvironmentIdentity,
    filesystem: LocalFileSystem,
    processes: LocalProcessHost,
    artifacts: TransientArtifactStore,
    observations: ObservationStore,
    checkpoints: WorkspaceCheckpointStore,
    capabilities: ExecutionCapabilities,
}

impl LocalExecutionEnvironment {
    pub fn new(workspace: &Workspace) -> Self {
        Self {
            identity: ExecutionEnvironmentIdentity {
                adapter: "local".to_string(),
                workspace_kind: workspace.kind.clone(),
                workspace_digest: rove_runtime_hash_workspace(workspace),
            },
            filesystem: LocalFileSystem {
                root: workspace.root.clone(),
            },
            processes: LocalProcessHost {
                root: workspace.root.clone(),
                background: Arc::new(RwLock::new(BTreeMap::new())),
            },
            artifacts: TransientArtifactStore::default(),
            observations: ObservationStore::default(),
            checkpoints: WorkspaceCheckpointStore::default(),
            capabilities: ExecutionCapabilities {
                filesystem_read: true,
                filesystem_write: true,
                process_run: true,
                process_stdio: true,
                observations: true,
                process_background: true,
                process_pty: false,
                workspace_checkpoints: true,
                artifact_projection: true,
            },
        }
    }
}

impl ExecutionEnvironment for LocalExecutionEnvironment {
    fn identity(&self) -> &ExecutionEnvironmentIdentity {
        &self.identity
    }

    fn filesystem(&self) -> &dyn WorkspaceFileSystem {
        &self.filesystem
    }

    fn processes(&self) -> &dyn ProcessHost {
        &self.processes
    }

    fn artifacts(&self) -> Option<&dyn ArtifactSink> {
        Some(&self.artifacts)
    }

    fn observations(&self) -> &ObservationStore {
        &self.observations
    }

    fn checkpoints(&self) -> &WorkspaceCheckpointStore {
        &self.checkpoints
    }

    fn capabilities(&self) -> &ExecutionCapabilities {
        &self.capabilities
    }
}

pub struct InMemoryExecutionEnvironment {
    identity: ExecutionEnvironmentIdentity,
    filesystem: InMemoryFileSystem,
    processes: InMemoryProcessHost,
    artifacts: TransientArtifactStore,
    observations: ObservationStore,
    checkpoints: WorkspaceCheckpointStore,
    capabilities: ExecutionCapabilities,
}

impl InMemoryExecutionEnvironment {
    pub fn new(workspace: &Workspace) -> Self {
        Self::with_capabilities(
            workspace,
            ExecutionCapabilities {
                filesystem_read: true,
                filesystem_write: true,
                process_run: true,
                process_stdio: false,
                observations: true,
                process_background: true,
                process_pty: false,
                workspace_checkpoints: true,
                artifact_projection: true,
            },
        )
    }

    pub fn with_capabilities(workspace: &Workspace, capabilities: ExecutionCapabilities) -> Self {
        Self {
            identity: ExecutionEnvironmentIdentity {
                adapter: "in_memory".to_string(),
                workspace_kind: workspace.kind.clone(),
                workspace_digest: rove_runtime_hash_workspace(workspace),
            },
            filesystem: InMemoryFileSystem::new(workspace.root.clone()),
            processes: InMemoryProcessHost::default(),
            artifacts: TransientArtifactStore::default(),
            observations: ObservationStore::default(),
            checkpoints: WorkspaceCheckpointStore::default(),
            capabilities,
        }
    }

    pub fn filesystem(&self) -> &InMemoryFileSystem {
        &self.filesystem
    }

    pub async fn seed_file(&self, path: impl Into<String>, content: impl Into<String>) {
        self.filesystem.seed(path.into(), content.into()).await;
    }

    pub fn processes(&self) -> &InMemoryProcessHost {
        &self.processes
    }
}

impl ExecutionEnvironment for InMemoryExecutionEnvironment {
    fn identity(&self) -> &ExecutionEnvironmentIdentity {
        &self.identity
    }

    fn filesystem(&self) -> &dyn WorkspaceFileSystem {
        &self.filesystem
    }

    fn processes(&self) -> &dyn ProcessHost {
        &self.processes
    }

    fn artifacts(&self) -> Option<&dyn ArtifactSink> {
        Some(&self.artifacts)
    }

    fn observations(&self) -> &ObservationStore {
        &self.observations
    }

    fn checkpoints(&self) -> &WorkspaceCheckpointStore {
        &self.checkpoints
    }

    fn capabilities(&self) -> &ExecutionCapabilities {
        &self.capabilities
    }
}

pub struct LocalFileSystem {
    root: PathBuf,
}

#[async_trait]
impl WorkspaceFileSystem for LocalFileSystem {
    fn root(&self) -> &Path {
        &self.root
    }

    async fn read_utf8(&self, raw_path: &str) -> Result<String, EnvironmentError> {
        let path = resolve_local_read_path(&self.root, raw_path)?;
        tokio::fs::read_to_string(path)
            .await
            .map_err(map_file_read_error)
    }

    async fn write_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError> {
        let path = resolve_workspace_write_path(&self.root, raw_path)
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        let before = match tokio::fs::metadata(&path).await {
            Ok(metadata) => {
                if metadata.len() > MAX_FILE_MUTATION_CAPTURE_BYTES {
                    return Err(EnvironmentError::ResourceLimit(
                        "file_mutation_capture_bytes",
                    ));
                }
                Some(
                    tokio::fs::read_to_string(&path)
                        .await
                        .map_err(|error| EnvironmentError::Host(error.to_string()))?,
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(EnvironmentError::Host(error.to_string())),
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        }
        tokio::fs::write(path, content)
            .await
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        Ok(FileMutation {
            operation: if before.is_some() {
                FileMutationOperation::Update
            } else {
                FileMutationOperation::Create
            },
            before,
        })
    }

    async fn create_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError> {
        let path = resolve_workspace_write_path(&self.root, raw_path)
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    EnvironmentError::Conflict("destination already exists".to_string())
                } else {
                    EnvironmentError::Host(error.to_string())
                }
            })?;
        // `write_all` alone only fills tokio's internal buffer. Without an
        // explicit flush the bytes are pushed out by a background task when the
        // handle drops, so a caller that reads the path right after this returns
        // can observe an empty file. Flush before reporting the mutation.
        if let Err(error) = async {
            file.write_all(content.as_bytes()).await?;
            file.flush().await
        }
        .await
        {
            drop(file);
            let _ = tokio::fs::remove_file(&path).await;
            return Err(EnvironmentError::Host(error.to_string()));
        }
        Ok(FileMutation {
            operation: FileMutationOperation::Create,
            before: None,
        })
    }

    async fn list_files(
        &self,
        raw_path: Option<&str>,
        max_files: usize,
    ) -> Result<Vec<WorkspaceFileEntry>, EnvironmentError> {
        Ok(self
            .list_entries(raw_path, true, max_files)
            .await?
            .into_iter()
            .filter(|entry| entry.kind == WorkspaceEntryKind::File)
            .take(max_files)
            .map(|entry| WorkspaceFileEntry {
                relative_path: entry.relative_path,
                byte_len: entry.byte_len,
            })
            .collect())
    }

    async fn read_relative_bytes(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> Result<BoundedFileRead, EnvironmentError> {
        let path = resolve_local_read_path(&self.root, relative_path)?;
        let file = tokio::fs::File::open(path)
            .await
            .map_err(map_file_read_error)?;
        let mut bytes = Vec::with_capacity(max_bytes.min(8 * 1024));
        file.take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let truncated = bytes.len() > max_bytes;
        bytes.truncate(max_bytes);
        Ok(BoundedFileRead { bytes, truncated })
    }

    async fn read_versioned(
        &self,
        raw_path: &str,
        max_bytes: usize,
    ) -> Result<VersionedFile, EnvironmentError> {
        let path = resolve_local_read_path(&self.root, raw_path)?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(map_file_read_error)?;
        if !metadata.is_file() {
            return Err(EnvironmentError::InvalidPath(
                "workspace path is not a file".to_string(),
            ));
        }
        if metadata.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
            return Err(EnvironmentError::ResourceLimit("versioned_file_bytes"));
        }
        let bytes = tokio::fs::read(path).await.map_err(map_file_read_error)?;
        Ok(VersionedFile {
            version: version_bytes(&bytes),
            bytes,
        })
    }

    async fn path_kind(
        &self,
        raw_path: &str,
    ) -> Result<Option<WorkspaceEntryKind>, EnvironmentError> {
        let path = match resolve_local_read_path(&self.root, raw_path) {
            Ok(path) => path,
            Err(EnvironmentError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(map_file_read_error)?;
        Ok(if metadata.is_file() {
            Some(WorkspaceEntryKind::File)
        } else if metadata.is_dir() {
            Some(WorkspaceEntryKind::Directory)
        } else {
            None
        })
    }

    async fn list_entries(
        &self,
        raw_path: Option<&str>,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<WorkspaceEntry>, EnvironmentError> {
        Ok(self
            .traverse_entries(
                raw_path,
                WorkspaceTraversalOptions {
                    recursive,
                    ..WorkspaceTraversalOptions::default()
                },
                max_entries,
            )
            .await?
            .entries)
    }

    async fn traverse_entries(
        &self,
        raw_path: Option<&str>,
        options: WorkspaceTraversalOptions,
        max_entries: usize,
    ) -> Result<WorkspaceTraversal, EnvironmentError> {
        let root = self.root.clone();
        let raw_path = raw_path.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            let canonical_root = root
                .canonicalize()
                .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
            let search_root = match raw_path.as_deref() {
                None | Some("") | Some(".") => canonical_root.clone(),
                Some(path) => resolve_local_read_path(&root, path)?,
            };
            if !search_root.starts_with(&canonical_root) {
                return Err(EnvironmentError::Boundary);
            }
            let search_metadata = std::fs::metadata(&search_root)
                .map_err(|error| EnvironmentError::Host(error.to_string()))?;
            if search_metadata.is_file() {
                let relative = relative_workspace_path(&canonical_root, &search_root)?;
                if is_sensitive_traversal_path(&relative) {
                    return Ok(WorkspaceTraversal {
                        scanned_entries: 1,
                        sensitive_entries: 1,
                        ..WorkspaceTraversal::default()
                    });
                }
                if !options.include_hidden && path_has_hidden_component(&relative) {
                    return Ok(WorkspaceTraversal {
                        scanned_entries: 1,
                        hidden_entries: 1,
                        ..WorkspaceTraversal::default()
                    });
                }
                if !options.include_ignored
                    && direct_path_is_ignored(&canonical_root, &search_root)?
                {
                    return Ok(WorkspaceTraversal {
                        scanned_entries: 1,
                        ignored_entries: 1,
                        ..WorkspaceTraversal::default()
                    });
                }
                return Ok(WorkspaceTraversal {
                    entries: vec![WorkspaceEntry {
                        relative_path: relative,
                        kind: WorkspaceEntryKind::File,
                        byte_len: search_metadata.len() as usize,
                    }],
                    scanned_entries: 1,
                    ..WorkspaceTraversal::default()
                });
            }
            if !search_metadata.is_dir() {
                return Err(EnvironmentError::InvalidPath(
                    "workspace path is not a file or directory".to_string(),
                ));
            }

            let mut traversal = WorkspaceTraversal::default();
            let max_depth = if options.recursive { None } else { Some(1) };
            let mut builder = ignore::WalkBuilder::new(&search_root);
            builder
                .follow_links(false)
                .hidden(!options.include_hidden)
                .ignore(!options.include_ignored)
                .git_ignore(!options.include_ignored)
                .git_exclude(!options.include_ignored)
                .git_global(false)
                .require_git(false)
                .parents(true)
                .max_depth(max_depth);
            builder.sort_by_file_path(|left, right| left.cmp(right));
            let scan_limit = max_entries
                .saturating_mul(TRAVERSAL_SCAN_MULTIPLIER)
                .max(max_entries.saturating_add(1))
                .max(1);
            for entry in builder.build() {
                let entry = entry.map_err(|error| EnvironmentError::Host(error.to_string()))?;
                if entry.depth() == 0 {
                    continue;
                }
                if traversal.scanned_entries >= scan_limit {
                    traversal.truncated = true;
                    break;
                }
                traversal.scanned_entries = traversal.scanned_entries.saturating_add(1);
                let file_type = entry.file_type();
                if file_type.is_some_and(|kind| kind.is_symlink())
                    || std::fs::symlink_metadata(entry.path())
                        .map(|metadata| {
                            crate::workspace::boundary::is_symlink_or_reparse(&metadata)
                        })
                        .unwrap_or(false)
                {
                    traversal.link_entries = traversal.link_entries.saturating_add(1);
                    continue;
                }
                let relative = relative_workspace_path(&canonical_root, entry.path())?;
                if is_sensitive_traversal_path(&relative) {
                    traversal.sensitive_entries = traversal.sensitive_entries.saturating_add(1);
                    continue;
                }
                let Some(file_type) = file_type else {
                    continue;
                };
                if !file_type.is_file() && !file_type.is_dir() {
                    continue;
                }
                let canonical = entry
                    .path()
                    .canonicalize()
                    .map_err(|error| EnvironmentError::Host(error.to_string()))?;
                if !canonical.starts_with(&canonical_root) {
                    continue;
                }
                let metadata = std::fs::metadata(&canonical)
                    .map_err(|error| EnvironmentError::Host(error.to_string()))?;
                traversal.entries.push(WorkspaceEntry {
                    relative_path: relative,
                    kind: if file_type.is_file() {
                        WorkspaceEntryKind::File
                    } else {
                        WorkspaceEntryKind::Directory
                    },
                    byte_len: if file_type.is_file() {
                        metadata.len() as usize
                    } else {
                        0
                    },
                });
                if traversal.entries.len() > max_entries {
                    traversal.truncated = true;
                    break;
                }
            }
            traversal
                .entries
                .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            traversal.entries.truncate(max_entries);
            traversal.sensitive_entries = traversal_sensitive_count(
                &canonical_root,
                &search_root,
                options.recursive,
                max_entries.saturating_mul(2).max(1),
            )?;
            let admitted = traversal
                .entries
                .iter()
                .map(|entry| entry.relative_path.clone())
                .collect::<BTreeSet<_>>();
            if !options.include_ignored {
                let ignored_visible = traversal_path_set(
                    &canonical_root,
                    &search_root,
                    options.recursive,
                    true,
                    options.include_hidden,
                    max_entries.saturating_mul(2).max(1),
                )?;
                traversal.ignored_entries = ignored_visible.difference(&admitted).count();
            }
            if !options.include_hidden {
                let hidden_visible = traversal_path_set(
                    &canonical_root,
                    &search_root,
                    options.recursive,
                    options.include_ignored,
                    true,
                    max_entries.saturating_mul(2).max(1),
                )?;
                traversal.hidden_entries = hidden_visible.difference(&admitted).count();
            }
            Ok(traversal)
        })
        .await
        .map_err(|error| EnvironmentError::Host(error.to_string()))?
    }

    async fn delete_path(&self, raw_path: &str, recursive: bool) -> Result<(), EnvironmentError> {
        let path = resolve_workspace_write_path(&self.root, raw_path)
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(map_file_read_error)?;
        if metadata.is_file() {
            tokio::fs::remove_file(path)
                .await
                .map_err(|error| EnvironmentError::Host(error.to_string()))
        } else if metadata.is_dir() {
            if recursive {
                let entries = self
                    .list_entries(
                        Some(raw_path),
                        true,
                        MAX_DIRECTORY_MUTATION_ENTRIES.saturating_add(1),
                    )
                    .await?;
                if entries.len() > MAX_DIRECTORY_MUTATION_ENTRIES {
                    return Err(EnvironmentError::ResourceLimit(
                        "directory_mutation_entries",
                    ));
                }
                tokio::fs::remove_dir_all(path)
                    .await
                    .map_err(|error| EnvironmentError::Host(error.to_string()))
            } else {
                tokio::fs::remove_dir(path)
                    .await
                    .map_err(|error| EnvironmentError::Conflict(error.to_string()))
            }
        } else {
            Err(EnvironmentError::InvalidPath(
                "workspace path is not a file or directory".to_string(),
            ))
        }
    }

    async fn move_path(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<(), EnvironmentError> {
        if overwrite {
            return Err(EnvironmentError::Conflict(
                "move overwrite requires a separately observed destination and is unsupported"
                    .to_string(),
            ));
        }
        let source = resolve_workspace_write_path(&self.root, from)
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        let destination = resolve_workspace_write_path(&self.root, to)
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        if tokio::fs::try_exists(&destination)
            .await
            .map_err(|error| EnvironmentError::Host(error.to_string()))?
        {
            return Err(EnvironmentError::Conflict(
                "destination already exists".to_string(),
            ));
        }
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        }
        tokio::fs::rename(source, destination)
            .await
            .map_err(|error| EnvironmentError::Host(error.to_string()))
    }
}

#[derive(Clone)]
pub struct InMemoryFileSystem {
    root: PathBuf,
    files: Arc<RwLock<BTreeMap<String, String>>>,
}

impl InMemoryFileSystem {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    async fn seed(&self, path: String, content: String) {
        self.files.write().await.insert(path, content);
    }

    fn normalize(path: &str) -> Result<String, EnvironmentError> {
        let path = Path::new(path);
        if path.is_absolute() {
            return Err(EnvironmentError::Boundary);
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::Normal(value) => normalized.push(value),
                std::path::Component::ParentDir => {
                    if !normalized.pop() {
                        return Err(EnvironmentError::Boundary);
                    }
                }
                _ => return Err(EnvironmentError::Boundary),
            }
        }
        let value = normalized.to_string_lossy().replace('\\', "/");
        if value.is_empty() {
            return Err(EnvironmentError::InvalidPath(
                "path must not be empty".to_string(),
            ));
        }
        Ok(value)
    }
}

#[async_trait]
impl WorkspaceFileSystem for InMemoryFileSystem {
    fn root(&self) -> &Path {
        &self.root
    }

    async fn read_utf8(&self, raw_path: &str) -> Result<String, EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        self.files
            .read()
            .await
            .get(&path)
            .cloned()
            .ok_or_else(|| EnvironmentError::Host("file not found".to_string()))
    }

    async fn write_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        let mut files = self.files.write().await;
        if files
            .get(&path)
            .is_some_and(|value| value.len() as u64 > MAX_FILE_MUTATION_CAPTURE_BYTES)
        {
            return Err(EnvironmentError::ResourceLimit(
                "file_mutation_capture_bytes",
            ));
        }
        if !files.contains_key(&path) && files.len() >= MAX_IN_MEMORY_FILES {
            return Err(EnvironmentError::Host(
                "in-memory file limit reached".to_string(),
            ));
        }
        let before = files.insert(path, content.to_string());
        Ok(FileMutation {
            operation: if before.is_some() {
                FileMutationOperation::Update
            } else {
                FileMutationOperation::Create
            },
            before,
        })
    }

    async fn create_utf8(
        &self,
        raw_path: &str,
        content: &str,
    ) -> Result<FileMutation, EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        let mut files = self.files.write().await;
        if files.contains_key(&path) {
            return Err(EnvironmentError::Conflict(
                "destination already exists".to_string(),
            ));
        }
        if files.len() >= MAX_IN_MEMORY_FILES {
            return Err(EnvironmentError::ResourceLimit("in_memory_files"));
        }
        files.insert(path, content.to_string());
        Ok(FileMutation {
            operation: FileMutationOperation::Create,
            before: None,
        })
    }

    async fn list_files(
        &self,
        raw_path: Option<&str>,
        max_files: usize,
    ) -> Result<Vec<WorkspaceFileEntry>, EnvironmentError> {
        let prefix = raw_path
            .filter(|value| !value.is_empty() && *value != ".")
            .map(Self::normalize)
            .transpose()?;
        Ok(self
            .files
            .read()
            .await
            .iter()
            .filter(|(path, _)| {
                prefix.as_ref().is_none_or(|prefix| {
                    path.as_str() == prefix
                        || path
                            .strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
            })
            .take(max_files)
            .map(|(path, content)| WorkspaceFileEntry {
                relative_path: path.clone(),
                byte_len: content.len(),
            })
            .collect())
    }

    async fn read_relative_bytes(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> Result<BoundedFileRead, EnvironmentError> {
        let path = Self::normalize(relative_path)?;
        let files = self.files.read().await;
        let content = files.get(&path).ok_or(EnvironmentError::NotFound)?;
        let bytes = content.as_bytes();
        let truncated = bytes.len() > max_bytes;
        Ok(BoundedFileRead {
            bytes: bytes[..bytes.len().min(max_bytes)].to_vec(),
            truncated,
        })
    }

    async fn read_versioned(
        &self,
        raw_path: &str,
        max_bytes: usize,
    ) -> Result<VersionedFile, EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        let files = self.files.read().await;
        let content = files.get(&path).ok_or(EnvironmentError::NotFound)?;
        if content.len() > max_bytes {
            return Err(EnvironmentError::ResourceLimit("versioned_file_bytes"));
        }
        Ok(VersionedFile {
            version: version_bytes(content.as_bytes()),
            bytes: content.as_bytes().to_vec(),
        })
    }

    async fn path_kind(
        &self,
        raw_path: &str,
    ) -> Result<Option<WorkspaceEntryKind>, EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        let files = self.files.read().await;
        if files.contains_key(&path) {
            return Ok(Some(WorkspaceEntryKind::File));
        }
        let prefix = format!("{path}/");
        Ok(files
            .keys()
            .any(|candidate| candidate.starts_with(&prefix))
            .then_some(WorkspaceEntryKind::Directory))
    }

    async fn list_entries(
        &self,
        raw_path: Option<&str>,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<WorkspaceEntry>, EnvironmentError> {
        let prefix = raw_path
            .filter(|value| !value.is_empty() && *value != ".")
            .map(Self::normalize)
            .transpose()?;
        let files = self.files.read().await;
        if let Some(path) = prefix.as_ref()
            && let Some(content) = files.get(path)
        {
            return Ok(vec![WorkspaceEntry {
                relative_path: path.clone(),
                kind: WorkspaceEntryKind::File,
                byte_len: content.len(),
            }]);
        }

        let mut entries = BTreeMap::<String, WorkspaceEntry>::new();
        for (path, content) in files.iter() {
            let relative_suffix = match prefix.as_ref() {
                Some(prefix) => match path.strip_prefix(&format!("{prefix}/")) {
                    Some(value) => value,
                    None => continue,
                },
                None => path.as_str(),
            };
            if relative_suffix.is_empty() {
                continue;
            }
            let components = relative_suffix.split('/').collect::<Vec<_>>();
            let directory_components = components.len().saturating_sub(1);
            let directory_limit = if recursive { directory_components } else { 1 };
            for index in 0..directory_limit.min(directory_components) {
                let suffix = components[..=index].join("/");
                let full = prefix
                    .as_ref()
                    .map(|prefix| format!("{prefix}/{suffix}"))
                    .unwrap_or(suffix);
                entries.entry(full.clone()).or_insert(WorkspaceEntry {
                    relative_path: full,
                    kind: WorkspaceEntryKind::Directory,
                    byte_len: 0,
                });
            }
            if recursive || components.len() == 1 {
                entries.insert(
                    path.clone(),
                    WorkspaceEntry {
                        relative_path: path.clone(),
                        kind: WorkspaceEntryKind::File,
                        byte_len: content.len(),
                    },
                );
            }
            if entries.len() > max_entries {
                break;
            }
        }
        Ok(entries.into_values().collect())
    }

    async fn traverse_entries(
        &self,
        raw_path: Option<&str>,
        options: WorkspaceTraversalOptions,
        max_entries: usize,
    ) -> Result<WorkspaceTraversal, EnvironmentError> {
        let entries = self
            .list_entries(raw_path, options.recursive, max_entries.saturating_add(1))
            .await?;
        let mut result = WorkspaceTraversal::default();
        for entry in entries {
            if is_sensitive_traversal_path(&entry.relative_path) {
                result.sensitive_entries += 1;
                continue;
            }
            if !options.include_hidden && path_has_hidden_component(&entry.relative_path) {
                result.hidden_entries += 1;
                continue;
            }
            result.scanned_entries += 1;
            if result.entries.len() == max_entries {
                result.truncated = true;
                break;
            }
            result.entries.push(entry);
        }
        Ok(result)
    }

    async fn delete_path(&self, raw_path: &str, recursive: bool) -> Result<(), EnvironmentError> {
        let path = Self::normalize(raw_path)?;
        let mut files = self.files.write().await;
        if files.remove(&path).is_some() {
            return Ok(());
        }
        let prefix = format!("{path}/");
        let matches = files
            .keys()
            .filter(|candidate| candidate.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(EnvironmentError::NotFound);
        }
        if !recursive {
            return Err(EnvironmentError::Conflict(
                "directory is not empty".to_string(),
            ));
        }
        if matches.len() > MAX_DIRECTORY_MUTATION_ENTRIES {
            return Err(EnvironmentError::ResourceLimit(
                "directory_mutation_entries",
            ));
        }
        for candidate in matches {
            files.remove(&candidate);
        }
        Ok(())
    }

    async fn move_path(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<(), EnvironmentError> {
        if overwrite {
            return Err(EnvironmentError::Conflict(
                "move overwrite requires a separately observed destination and is unsupported"
                    .to_string(),
            ));
        }
        let from = Self::normalize(from)?;
        let to = Self::normalize(to)?;
        let mut files = self.files.write().await;
        if files.contains_key(&to) {
            return Err(EnvironmentError::Conflict(
                "destination already exists".to_string(),
            ));
        }
        if let Some(content) = files.remove(&from) {
            files.insert(to, content);
            return Ok(());
        }
        let prefix = format!("{from}/");
        let candidates = files
            .keys()
            .filter(|candidate| candidate.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(EnvironmentError::NotFound);
        }
        if candidates.len() > MAX_DIRECTORY_MUTATION_ENTRIES {
            return Err(EnvironmentError::ResourceLimit(
                "directory_mutation_entries",
            ));
        }
        if files
            .keys()
            .any(|candidate| candidate == &to || candidate.starts_with(&format!("{to}/")))
        {
            return Err(EnvironmentError::Conflict(
                "directory destination already exists".to_string(),
            ));
        }
        for candidate in candidates {
            let suffix = candidate
                .strip_prefix(&from)
                .expect("candidate is source-prefixed");
            if let Some(content) = files.remove(&candidate) {
                files.insert(format!("{to}{suffix}"), content);
            }
        }
        Ok(())
    }
}

pub struct LocalProcessHost {
    root: PathBuf,
    background: Arc<RwLock<BTreeMap<String, Arc<BackgroundProcess>>>>,
}

#[derive(Debug, Default)]
struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
    done: bool,
}

struct BackgroundProcess {
    child: Mutex<Child>,
    stdout: Arc<RwLock<CapturedStream>>,
    stderr: Arc<RwLock<CapturedStream>>,
    terminal: RwLock<Option<(BackgroundProcessStatus, Option<i32>)>>,
}

#[async_trait]
impl ProcessHost for LocalProcessHost {
    async fn run(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<ProcessOutput, EnvironmentError> {
        if request.program.trim().is_empty() {
            return Err(EnvironmentError::Host(
                "process program must not be empty".to_string(),
            ));
        }
        if request.timeout_ms == 0 {
            return Err(EnvironmentError::Timeout(0));
        }
        let canonical_root = self
            .root
            .canonicalize()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let canonical_cwd = request
            .cwd
            .canonicalize()
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        if !canonical_cwd.starts_with(&canonical_root) {
            return Err(EnvironmentError::Boundary);
        }
        if cancel.is_cancelled() {
            return Err(EnvironmentError::Cancelled);
        }
        let mut process = Command::new(&request.program);
        process
            .args(&request.args)
            .current_dir(canonical_cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if request.clear_environment {
            process.env_clear();
        }
        process.envs(&request.environment);
        let mut child = process
            .spawn()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EnvironmentError::Host("process stdout unavailable".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| EnvironmentError::Host("process stderr unavailable".to_string()))?;
        let stdout_task = tokio::spawn(read_stream_bounded(stdout, request.max_output_bytes));
        let stderr_task = tokio::spawn(read_stream_bounded(stderr, request.max_output_bytes));
        let status = tokio::select! {
            _ = cancel.cancelled() => {
                terminate_child(&mut child).await;
                let _ = join_bounded_reader(stdout_task).await;
                let _ = join_bounded_reader(stderr_task).await;
                return Err(EnvironmentError::Cancelled);
            }
            result = timeout(Duration::from_millis(request.timeout_ms), child.wait()) => {
                match result {
                    Ok(status) => status.map_err(|error| EnvironmentError::Host(error.to_string()))?,
                    Err(_) => {
                        terminate_child(&mut child).await;
                        let _ = join_bounded_reader(stdout_task).await;
                        let _ = join_bounded_reader(stderr_task).await;
                        return Err(EnvironmentError::Timeout(request.timeout_ms));
                    }
                }
            }
        };
        let (stdout, stdout_truncated) = join_bounded_reader(stdout_task).await?;
        let (stderr, stderr_truncated) = join_bounded_reader(stderr_task).await?;
        Ok(ProcessOutput {
            status_code: status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    }

    async fn spawn_stdio(
        &self,
        program: &str,
        args: &[String],
        environment: &[(String, String)],
    ) -> Result<StdioChild, EnvironmentError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&self.root)
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EnvironmentError::Host("stdio stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EnvironmentError::Host("stdio stdout unavailable".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| EnvironmentError::Host("stdio stderr unavailable".to_string()))?;
        Ok(StdioChild {
            stdin,
            stdout,
            stderr,
            child: Some(StdioProcessGuard::new(child)),
        })
    }

    async fn spawn_background(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<BackgroundProcessStarted, EnvironmentError> {
        if request.program.trim().is_empty() {
            return Err(EnvironmentError::Host(
                "process program must not be empty".to_string(),
            ));
        }
        if request.timeout_ms == 0 {
            return Err(EnvironmentError::Timeout(0));
        }
        if cancel.is_cancelled() {
            return Err(EnvironmentError::Cancelled);
        }
        let canonical_root = self
            .root
            .canonicalize()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let canonical_cwd = request
            .cwd
            .canonicalize()
            .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
        if !canonical_cwd.starts_with(&canonical_root) {
            return Err(EnvironmentError::Boundary);
        }
        if self.background.read().await.len() >= MAX_BACKGROUND_PROCESSES {
            return Err(EnvironmentError::ResourceLimit("background_processes"));
        }

        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(canonical_cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if request.clear_environment {
            command.env_clear();
        }
        command.envs(&request.environment);
        let mut child = command
            .spawn()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EnvironmentError::Host("process stdout unavailable".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| EnvironmentError::Host("process stderr unavailable".to_string()))?;
        let stdout_capture = Arc::new(RwLock::new(CapturedStream::default()));
        let stderr_capture = Arc::new(RwLock::new(CapturedStream::default()));
        tokio::spawn(read_stream_progressive(
            stdout,
            request.max_output_bytes,
            stdout_capture.clone(),
        ));
        tokio::spawn(read_stream_progressive(
            stderr,
            request.max_output_bytes,
            stderr_capture.clone(),
        ));

        let process_id = ulid::Ulid::new().to_string();
        let process = Arc::new(BackgroundProcess {
            child: Mutex::new(child),
            stdout: stdout_capture,
            stderr: stderr_capture,
            terminal: RwLock::new(None),
        });
        self.background
            .write()
            .await
            .insert(process_id.clone(), process.clone());

        let weak_process = Arc::downgrade(&process);
        let timeout_ms = request.timeout_ms;
        tokio::spawn(async move {
            let terminal_status = tokio::select! {
                _ = cancel.cancelled() => BackgroundProcessStatus::Terminated,
                _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                    BackgroundProcessStatus::TimedOut
                }
            };
            let Some(process) = weak_process.upgrade() else {
                return;
            };
            let mut child = process.child.lock().await;
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.start_kill();
                let _ = child.wait().await;
                *process.terminal.write().await = Some((terminal_status, None));
            }
        });
        Ok(BackgroundProcessStarted { process_id })
    }

    async fn poll_background(
        &self,
        process_id: &str,
        stdout_cursor: usize,
        stderr_cursor: usize,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessOutput, EnvironmentError> {
        let process = self
            .background
            .read()
            .await
            .get(process_id)
            .cloned()
            .ok_or(EnvironmentError::ResourceNotFound("background_process"))?;
        if process.terminal.read().await.is_none() {
            let mut child = process.child.lock().await;
            if let Some(status) = child
                .try_wait()
                .map_err(|error| EnvironmentError::Host(error.to_string()))?
            {
                *process.terminal.write().await =
                    Some((BackgroundProcessStatus::Exited, status.code()));
            }
        }
        let terminal = process.terminal.read().await.clone();
        let stdout = process.stdout.read().await;
        let stderr = process.stderr.read().await;
        let (stdout_page, stdout_next) = process_page(
            &stdout.bytes,
            stdout_cursor,
            max_output_bytes,
            "stdout_cursor",
        )?;
        let (stderr_page, stderr_next) = process_page(
            &stderr.bytes,
            stderr_cursor,
            max_output_bytes,
            "stderr_cursor",
        )?;
        let stdout_has_more = stdout_next < stdout.bytes.len();
        let stderr_has_more = stderr_next < stderr.bytes.len();
        let output_complete = stdout.done && stderr.done;
        let result = BackgroundProcessOutput {
            process_id: process_id.to_string(),
            status: terminal
                .as_ref()
                .map(|(status, _)| status.clone())
                .unwrap_or(BackgroundProcessStatus::Running),
            status_code: terminal.as_ref().and_then(|(_, code)| *code),
            stdout: stdout_page,
            stderr: stderr_page,
            stdout_cursor: stdout_next,
            stderr_cursor: stderr_next,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            stdout_has_more,
            stderr_has_more,
            output_complete,
        };
        let should_release =
            terminal.is_some() && output_complete && !stdout_has_more && !stderr_has_more;
        drop(stdout);
        drop(stderr);
        if should_release {
            self.background.write().await.remove(process_id);
        }
        Ok(result)
    }

    async fn terminate_background(&self, process_id: &str) -> Result<(), EnvironmentError> {
        let process = self
            .background
            .read()
            .await
            .get(process_id)
            .cloned()
            .ok_or(EnvironmentError::ResourceNotFound("background_process"))?;
        let mut child = process.child.lock().await;
        if child
            .try_wait()
            .map_err(|error| EnvironmentError::Host(error.to_string()))?
            .is_none()
        {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        *process.terminal.write().await = Some((BackgroundProcessStatus::Terminated, None));
        drop(child);
        self.background.write().await.remove(process_id);
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryProcessHost {
    responses: RwLock<BTreeMap<String, ProcessOutput>>,
    delays: RwLock<BTreeMap<String, Duration>>,
    background: RwLock<BTreeMap<String, InMemoryBackgroundProcess>>,
}

#[derive(Debug, Clone)]
struct InMemoryBackgroundProcess {
    output: ProcessOutput,
    status: BackgroundProcessStatus,
}

impl InMemoryProcessHost {
    pub async fn set_response(&self, program: impl Into<String>, output: ProcessOutput) {
        self.responses.write().await.insert(program.into(), output);
    }

    pub async fn set_delay(&self, program: impl Into<String>, delay: Duration) {
        self.delays.write().await.insert(program.into(), delay);
    }
}

#[async_trait]
impl ProcessHost for InMemoryProcessHost {
    async fn run(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<ProcessOutput, EnvironmentError> {
        if cancel.is_cancelled() {
            return Err(EnvironmentError::Cancelled);
        }
        let delay = self
            .delays
            .read()
            .await
            .get(&request.program)
            .copied()
            .unwrap_or_default();
        tokio::select! {
            _ = cancel.cancelled() => return Err(EnvironmentError::Cancelled),
            result = timeout(
                Duration::from_millis(request.timeout_ms),
                tokio::time::sleep(delay),
            ) => {
                result.map_err(|_| EnvironmentError::Timeout(request.timeout_ms))?;
            }
        }
        let mut output = self
            .responses
            .read()
            .await
            .get(&request.program)
            .cloned()
            .ok_or_else(|| {
                EnvironmentError::Host("in-memory process is not configured".to_string())
            })?;
        let (stdout, stdout_truncated) = bounded_bytes(output.stdout, request.max_output_bytes);
        let (stderr, stderr_truncated) = bounded_bytes(output.stderr, request.max_output_bytes);
        output.stdout = stdout;
        output.stderr = stderr;
        output.stdout_truncated |= stdout_truncated;
        output.stderr_truncated |= stderr_truncated;
        Ok(output)
    }

    async fn spawn_stdio(
        &self,
        _program: &str,
        _args: &[String],
        _environment: &[(String, String)],
    ) -> Result<StdioChild, EnvironmentError> {
        Err(EnvironmentError::CapabilityUnavailable("process_stdio"))
    }

    async fn spawn_background(
        &self,
        request: ProcessRequest,
        cancel: CancellationToken,
    ) -> Result<BackgroundProcessStarted, EnvironmentError> {
        if cancel.is_cancelled() {
            return Err(EnvironmentError::Cancelled);
        }
        let mut processes = self.background.write().await;
        if processes.len() >= MAX_BACKGROUND_PROCESSES {
            return Err(EnvironmentError::ResourceLimit("background_processes"));
        }
        let mut output = self
            .responses
            .read()
            .await
            .get(&request.program)
            .cloned()
            .ok_or_else(|| {
                EnvironmentError::Host("in-memory process is not configured".to_string())
            })?;
        let (stdout, stdout_truncated) = bounded_bytes(output.stdout, request.max_output_bytes);
        let (stderr, stderr_truncated) = bounded_bytes(output.stderr, request.max_output_bytes);
        output.stdout = stdout;
        output.stderr = stderr;
        output.stdout_truncated |= stdout_truncated;
        output.stderr_truncated |= stderr_truncated;
        let process_id = ulid::Ulid::new().to_string();
        processes.insert(
            process_id.clone(),
            InMemoryBackgroundProcess {
                output,
                status: BackgroundProcessStatus::Exited,
            },
        );
        Ok(BackgroundProcessStarted { process_id })
    }

    async fn poll_background(
        &self,
        process_id: &str,
        stdout_cursor: usize,
        stderr_cursor: usize,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessOutput, EnvironmentError> {
        let mut processes = self.background.write().await;
        let process = processes
            .get(process_id)
            .cloned()
            .ok_or(EnvironmentError::ResourceNotFound("background_process"))?;
        let (stdout, stdout_next) = process_page(
            &process.output.stdout,
            stdout_cursor,
            max_output_bytes,
            "stdout_cursor",
        )?;
        let (stderr, stderr_next) = process_page(
            &process.output.stderr,
            stderr_cursor,
            max_output_bytes,
            "stderr_cursor",
        )?;
        let stdout_has_more = stdout_next < process.output.stdout.len();
        let stderr_has_more = stderr_next < process.output.stderr.len();
        let result = BackgroundProcessOutput {
            process_id: process_id.to_string(),
            status: process.status.clone(),
            status_code: process.output.status_code,
            stdout,
            stderr,
            stdout_cursor: stdout_next,
            stderr_cursor: stderr_next,
            stdout_truncated: process.output.stdout_truncated,
            stderr_truncated: process.output.stderr_truncated,
            stdout_has_more,
            stderr_has_more,
            output_complete: true,
        };
        if process.status != BackgroundProcessStatus::Running
            && !stdout_has_more
            && !stderr_has_more
        {
            processes.remove(process_id);
        }
        Ok(result)
    }

    async fn terminate_background(&self, process_id: &str) -> Result<(), EnvironmentError> {
        self.background
            .write()
            .await
            .remove(process_id)
            .map(|_| ())
            .ok_or(EnvironmentError::ResourceNotFound("background_process"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observation {
    pub id: String,
    pub source: String,
    pub start: usize,
    pub end: usize,
    pub byte_count: usize,
    pub digest: String,
    pub version: String,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

impl Observation {
    pub fn from_bytes(
        source: impl Into<String>,
        start: usize,
        bytes: &[u8],
        version: impl Into<String>,
        truncated: bool,
        artifact_ref: Option<String>,
    ) -> Self {
        use sha2::{Digest, Sha256};

        let source = source.into();
        let version = version.into();
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let end = start.saturating_add(bytes.len());
        let id = crate::context::prompt_metadata::stable_hash(&format!(
            "{source}|{start}|{end}|{digest}|{version}"
        ));
        Self {
            id,
            source,
            start,
            end,
            byte_count: bytes.len(),
            digest,
            version,
            truncated,
            artifact_ref,
        }
    }
}

#[derive(Debug, Clone)]
struct StoredObservation {
    metadata: Observation,
    payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct ObservationState {
    observations: BTreeMap<String, StoredObservation>,
    payload_bytes: usize,
}

#[derive(Clone, Default)]
pub struct ObservationStore {
    state: Arc<RwLock<ObservationState>>,
}

impl ObservationStore {
    pub async fn put(&self, observation: Observation) -> Result<(), EnvironmentError> {
        self.put_with_payload(observation, Vec::new()).await
    }

    pub async fn put_with_payload(
        &self,
        observation: Observation,
        payload: Vec<u8>,
    ) -> Result<(), EnvironmentError> {
        if payload.len() > MAX_OBSERVATION_PAYLOAD_BYTES {
            return Err(EnvironmentError::ResourceLimit("observation_payload_bytes"));
        }
        let mut state = self.state.write().await;
        if state.observations.len() >= MAX_OBSERVATIONS
            && !state.observations.contains_key(&observation.id)
        {
            return Err(EnvironmentError::ResourceLimit("observations"));
        }
        let replaced_bytes = state
            .observations
            .get(&observation.id)
            .map(|value| value.payload.len())
            .unwrap_or(0);
        let next_bytes = state
            .payload_bytes
            .saturating_sub(replaced_bytes)
            .saturating_add(payload.len());
        if next_bytes > MAX_OBSERVATION_PAYLOAD_BYTES {
            return Err(EnvironmentError::ResourceLimit("observation_payload_bytes"));
        }
        state.payload_bytes = next_bytes;
        state.observations.insert(
            observation.id.clone(),
            StoredObservation {
                metadata: observation,
                payload,
            },
        );
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<Observation> {
        self.state
            .read()
            .await
            .observations
            .get(id)
            .map(|stored| stored.metadata.clone())
    }

    pub async fn payload(&self, id: &str) -> Option<Vec<u8>> {
        self.state
            .read()
            .await
            .observations
            .get(id)
            .map(|stored| stored.payload.clone())
    }

    pub async fn require_version(
        &self,
        id: &str,
        version: &str,
    ) -> Result<Observation, EnvironmentError> {
        let observation = self
            .get(id)
            .await
            .ok_or_else(|| EnvironmentError::Host("observation not found".to_string()))?;
        if observation.version != version {
            return Err(EnvironmentError::StaleObservation);
        }
        Ok(observation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointFile {
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCheckpoint {
    pub id: String,
    pub files: BTreeMap<String, CheckpointFile>,
    pub byte_count: usize,
}

#[derive(Debug, Default)]
struct CheckpointState {
    checkpoints: BTreeMap<String, WorkspaceCheckpoint>,
    byte_count: usize,
}

#[derive(Clone, Default)]
pub struct WorkspaceCheckpointStore {
    state: Arc<RwLock<CheckpointState>>,
}

impl WorkspaceCheckpointStore {
    pub async fn put(
        &self,
        files: Vec<CheckpointFile>,
    ) -> Result<WorkspaceCheckpoint, EnvironmentError> {
        let byte_count = files
            .iter()
            .filter_map(|file| file.content.as_ref())
            .map(Vec::len)
            .sum::<usize>();
        if byte_count > MAX_CHECKPOINT_BYTES {
            return Err(EnvironmentError::ResourceLimit("checkpoint_bytes"));
        }
        let mut identity = String::from("workspace-checkpoint-v1\0");
        let mut mapped = BTreeMap::new();
        for file in files {
            identity.push_str(&file.path);
            identity.push('\0');
            identity.push_str(file.version.as_deref().unwrap_or("missing"));
            identity.push('\0');
            mapped.insert(file.path.clone(), file);
        }
        let id = crate::context::prompt_metadata::stable_hash(&identity);
        let checkpoint = WorkspaceCheckpoint {
            id: id.clone(),
            files: mapped,
            byte_count,
        };
        let mut state = self.state.write().await;
        if state.checkpoints.len() >= MAX_CHECKPOINTS && !state.checkpoints.contains_key(&id) {
            return Err(EnvironmentError::ResourceLimit("workspace_checkpoints"));
        }
        let replaced_bytes = state
            .checkpoints
            .get(&id)
            .map(|value| value.byte_count)
            .unwrap_or(0);
        let next_bytes = state
            .byte_count
            .saturating_sub(replaced_bytes)
            .saturating_add(byte_count);
        if next_bytes > MAX_CHECKPOINT_BYTES {
            return Err(EnvironmentError::ResourceLimit("checkpoint_bytes"));
        }
        state.byte_count = next_bytes;
        state.checkpoints.insert(id, checkpoint.clone());
        Ok(checkpoint)
    }

    pub async fn get(&self, id: &str) -> Result<WorkspaceCheckpoint, EnvironmentError> {
        self.state
            .read()
            .await
            .checkpoints
            .get(id)
            .cloned()
            .ok_or(EnvironmentError::ResourceNotFound("workspace_checkpoint"))
    }
}

#[derive(Debug, Default)]
struct ArtifactState {
    values: BTreeMap<String, Vec<u8>>,
    byte_count: usize,
}

#[derive(Clone, Default)]
pub struct TransientArtifactStore {
    state: Arc<RwLock<ArtifactState>>,
}

#[async_trait]
impl ArtifactSink for TransientArtifactStore {
    async fn put(&self, source: &str, bytes: &[u8]) -> Result<Option<String>, EnvironmentError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        if bytes.len() > MAX_ARTIFACT_PROJECTION_BYTES {
            return Err(EnvironmentError::ResourceLimit("artifact_projection_bytes"));
        }
        let digest = version_bytes(bytes);
        let artifact_ref = format!(
            "observation:{}",
            crate::context::prompt_metadata::stable_hash(&format!("{source}|{digest}"))
        );
        let mut state = self.state.write().await;
        if state.values.len() >= MAX_ARTIFACT_PROJECTIONS
            && !state.values.contains_key(&artifact_ref)
        {
            return Err(EnvironmentError::ResourceLimit("artifact_projections"));
        }
        let replaced_bytes = state.values.get(&artifact_ref).map(Vec::len).unwrap_or(0);
        let next_bytes = state
            .byte_count
            .saturating_sub(replaced_bytes)
            .saturating_add(bytes.len());
        if next_bytes > MAX_ARTIFACT_PROJECTION_BYTES {
            return Err(EnvironmentError::ResourceLimit("artifact_projection_bytes"));
        }
        state.byte_count = next_bytes;
        state.values.insert(artifact_ref.clone(), bytes.to_vec());
        Ok(Some(artifact_ref))
    }

    async fn get(&self, artifact_ref: &str) -> Result<Vec<u8>, EnvironmentError> {
        self.state
            .read()
            .await
            .values
            .get(artifact_ref)
            .cloned()
            .ok_or(EnvironmentError::ResourceNotFound("artifact_projection"))
    }
}

pub fn local_environment(workspace: &Workspace) -> Arc<dyn ExecutionEnvironment> {
    Arc::new(LocalExecutionEnvironment::new(workspace))
}

fn bounded_bytes(bytes: Vec<u8>, max: usize) -> (Vec<u8>, bool) {
    if bytes.len() <= max {
        return (bytes, false);
    }
    (bytes[..max].to_vec(), true)
}

fn map_file_read_error(error: std::io::Error) -> EnvironmentError {
    if error.kind() == std::io::ErrorKind::NotFound {
        EnvironmentError::NotFound
    } else {
        EnvironmentError::Host(error.to_string())
    }
}

fn resolve_local_read_path(root: &Path, raw_path: &str) -> Result<PathBuf, EnvironmentError> {
    match resolve_workspace_read_path(root, raw_path) {
        Ok(path) => Ok(path),
        Err(read_error) => {
            let candidate = resolve_workspace_write_path(root, raw_path)
                .map_err(|_| EnvironmentError::InvalidPath(read_error.to_string()))?;
            if candidate.exists() {
                Err(EnvironmentError::InvalidPath(read_error.to_string()))
            } else {
                Err(EnvironmentError::NotFound)
            }
        }
    }
}

async fn read_stream_bounded(
    mut reader: impl AsyncRead + Unpin,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), EnvironmentError> {
    let mut stored = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        if read == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(stored.len());
        let keep = read.min(remaining);
        stored.extend_from_slice(&chunk[..keep]);
        truncated |= keep < read;
    }
    Ok((stored, truncated))
}

async fn read_stream_progressive(
    mut reader: impl AsyncRead + Unpin,
    max_bytes: usize,
    capture: Arc<RwLock<CapturedStream>>,
) {
    let mut chunk = [0_u8; 8 * 1024];
    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        let mut capture = capture.write().await;
        let remaining = max_bytes.saturating_sub(capture.bytes.len());
        let keep = read.min(remaining);
        capture.bytes.extend_from_slice(&chunk[..keep]);
        capture.truncated |= keep < read;
    }
    capture.write().await.done = true;
}

fn process_page(
    bytes: &[u8],
    cursor: usize,
    max_bytes: usize,
    field: &str,
) -> Result<(Vec<u8>, usize), EnvironmentError> {
    if cursor > bytes.len() {
        return Err(EnvironmentError::Conflict(format!(
            "{field} is beyond retained process output"
        )));
    }
    let end = cursor.saturating_add(max_bytes).min(bytes.len());
    Ok((bytes[cursor..end].to_vec(), end))
}

async fn join_bounded_reader(
    task: tokio::task::JoinHandle<Result<(Vec<u8>, bool), EnvironmentError>>,
) -> Result<(Vec<u8>, bool), EnvironmentError> {
    task.await
        .map_err(|error| EnvironmentError::Host(error.to_string()))?
}

async fn terminate_child(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn version_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn relative_workspace_path(root: &Path, path: &Path) -> Result<String, EnvironmentError> {
    path.strip_prefix(root)
        .map_err(|_| EnvironmentError::Boundary)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn path_has_hidden_component(path: &str) -> bool {
    path.split('/')
        .any(|component| component.starts_with('.') && component != ".")
}

fn traversal_path_set(
    canonical_root: &Path,
    search_root: &Path,
    recursive: bool,
    include_ignored: bool,
    include_hidden: bool,
    max_entries: usize,
) -> Result<BTreeSet<String>, EnvironmentError> {
    let mut builder = ignore::WalkBuilder::new(search_root);
    builder
        .follow_links(false)
        .hidden(!include_hidden)
        .ignore(!include_ignored)
        .git_ignore(!include_ignored)
        .git_exclude(!include_ignored)
        .git_global(false)
        .require_git(false)
        .parents(true)
        .max_depth(if recursive { None } else { Some(1) });
    builder.sort_by_file_path(|left, right| left.cmp(right));
    let mut paths = BTreeSet::new();
    for entry in builder.build() {
        let entry = entry.map_err(|error| EnvironmentError::Host(error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| EnvironmentError::Host(error.to_string()))?;
        if crate::workspace::boundary::is_symlink_or_reparse(&metadata) {
            continue;
        }
        let relative = relative_workspace_path(canonical_root, entry.path())?;
        if is_sensitive_traversal_path(&relative) {
            continue;
        }
        paths.insert(relative);
        if paths.len() >= max_entries {
            break;
        }
    }
    Ok(paths)
}

fn traversal_sensitive_count(
    canonical_root: &Path,
    search_root: &Path,
    recursive: bool,
    max_entries: usize,
) -> Result<usize, EnvironmentError> {
    let mut builder = ignore::WalkBuilder::new(search_root);
    builder
        .follow_links(false)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .parents(true)
        .max_depth(if recursive { None } else { Some(1) });
    builder.sort_by_file_path(|left, right| left.cmp(right));
    let mut count = 0usize;
    for (index, entry) in builder.build().enumerate() {
        if index >= max_entries {
            break;
        }
        let entry = entry.map_err(|error| EnvironmentError::Host(error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = relative_workspace_path(canonical_root, entry.path())?;
        count += usize::from(is_sensitive_traversal_path(&relative));
    }
    Ok(count)
}

pub(crate) fn is_sensitive_traversal_path(path: &str) -> bool {
    path.split('/').any(|component| {
        let name = component.to_ascii_lowercase();
        name == ".git"
            || name == ".rove"
            || name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || matches!(
                name.as_str(),
                "credentials" | "credentials.json" | "id_rsa" | "id_ed25519"
            )
    })
}

fn direct_path_is_ignored(root: &Path, target: &Path) -> Result<bool, EnvironmentError> {
    let mut ignored = false;
    let mut ancestor = root.to_path_buf();
    ignored = match_ignore_file(root, ".gitignore", target, false, ignored)?;
    ignored = match_ignore_file(root, ".ignore", target, false, ignored)?;
    let relative_parent = target
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .unwrap_or_else(|| Path::new(""));
    for component in relative_parent.components() {
        ancestor.push(component.as_os_str());
        ignored = match_ignore_file(&ancestor, ".gitignore", target, false, ignored)?;
        ignored = match_ignore_file(&ancestor, ".ignore", target, false, ignored)?;
    }
    Ok(ignored)
}

fn match_ignore_file(
    root: &Path,
    file_name: &str,
    target: &Path,
    is_dir: bool,
    current: bool,
) -> Result<bool, EnvironmentError> {
    let path = root.join(file_name);
    if !path.is_file() {
        return Ok(current);
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    if let Some(error) = builder.add(&path) {
        return Err(EnvironmentError::Host(error.to_string()));
    }
    let matcher = builder
        .build()
        .map_err(|error| EnvironmentError::Host(error.to_string()))?;
    let matched = matcher.matched_path_or_any_parents(target, is_dir);
    Ok(if matched.is_ignore() {
        true
    } else if matched.is_whitelist() {
        false
    } else {
        current
    })
}

fn rove_runtime_hash_workspace(workspace: &Workspace) -> String {
    crate::context::prompt_metadata::workspace_fingerprint(workspace)
}

#[cfg(test)]
mod productization_traversal_tests {
    use super::*;

    #[tokio::test]
    async fn local_traversal_is_ignore_hidden_sensitive_and_sort_aware() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::create_dir_all(temp.path().join("ignored")).unwrap();
        std::fs::create_dir_all(temp.path().join("also-ignored")).unwrap();
        std::fs::create_dir_all(temp.path().join(".hidden")).unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(temp.path().join(".ignore"), "also-ignored/\n").unwrap();
        std::fs::write(temp.path().join("src/b.rs"), "b").unwrap();
        std::fs::write(temp.path().join("src/a.rs"), "a").unwrap();
        std::fs::write(temp.path().join("ignored/noise.rs"), "ignored").unwrap();
        std::fs::write(temp.path().join("also-ignored/noise.rs"), "ignored").unwrap();
        std::fs::write(temp.path().join(".hidden/note.rs"), "hidden").unwrap();
        std::fs::write(temp.path().join(".env"), "SECRET=value").unwrap();
        let filesystem = LocalFileSystem {
            root: temp.path().to_path_buf(),
        };

        let default = filesystem
            .traverse_entries(
                None,
                WorkspaceTraversalOptions {
                    recursive: true,
                    include_ignored: false,
                    include_hidden: false,
                },
                100,
            )
            .await
            .unwrap();
        let paths = default
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, {
            let mut sorted = paths.clone();
            sorted.sort();
            sorted
        });
        assert!(!paths.contains(&"ignored/noise.rs"));
        assert!(!paths.contains(&"also-ignored/noise.rs"));
        assert!(!paths.contains(&".hidden/note.rs"));
        assert!(!paths.contains(&".env"));
        assert!(default.ignored_entries > 0);
        assert!(default.hidden_entries > 0);
        assert!(default.sensitive_entries > 0);

        let opted_in = filesystem
            .traverse_entries(
                None,
                WorkspaceTraversalOptions {
                    recursive: true,
                    include_ignored: true,
                    include_hidden: true,
                },
                100,
            )
            .await
            .unwrap();
        let paths = opted_in
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"ignored/noise.rs"));
        assert!(paths.contains(&"also-ignored/noise.rs"));
        assert!(paths.contains(&".hidden/note.rs"));
        assert!(!paths.contains(&".env"));

        let direct_sensitive = filesystem
            .traverse_entries(
                Some(".env"),
                WorkspaceTraversalOptions {
                    recursive: true,
                    include_ignored: true,
                    include_hidden: true,
                },
                100,
            )
            .await
            .unwrap();
        assert!(direct_sensitive.entries.is_empty());
        assert_eq!(direct_sensitive.sensitive_entries, 1);

        let direct_ignored = filesystem
            .traverse_entries(
                Some("also-ignored/noise.rs"),
                WorkspaceTraversalOptions::default(),
                100,
            )
            .await
            .unwrap();
        assert!(direct_ignored.entries.is_empty());
        assert_eq!(direct_ignored.ignored_entries, 1);
    }

    #[tokio::test]
    async fn local_traversal_truncates_the_lexically_first_entries() {
        let temp = tempfile::TempDir::new().unwrap();
        for name in ["z.txt", "m.txt", "a.txt"] {
            std::fs::write(temp.path().join(name), name).unwrap();
        }
        let filesystem = LocalFileSystem {
            root: temp.path().to_path_buf(),
        };

        let traversal = filesystem
            .traverse_entries(None, WorkspaceTraversalOptions::default(), 2)
            .await
            .unwrap();
        assert_eq!(
            traversal
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "m.txt"]
        );
        assert!(traversal.truncated);
    }

    #[tokio::test]
    async fn local_traversal_bounds_scanning_when_every_entry_is_excluded() {
        let temp = tempfile::TempDir::new().unwrap();
        for index in 0..32 {
            std::fs::write(temp.path().join(format!(".env.{index:02}")), "secret").unwrap();
        }
        let filesystem = LocalFileSystem {
            root: temp.path().to_path_buf(),
        };

        let traversal = filesystem
            .traverse_entries(
                None,
                WorkspaceTraversalOptions {
                    recursive: false,
                    include_ignored: true,
                    include_hidden: true,
                },
                1,
            )
            .await
            .unwrap();
        assert!(traversal.entries.is_empty());
        assert!(traversal.truncated);
        assert!(traversal.scanned_entries <= TRAVERSAL_SCAN_MULTIPLIER);
    }
}
