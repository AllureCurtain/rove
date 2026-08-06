//! Runtime-owned host authority for filesystem and process operations.
//!
//! Built-in tools depend on these ports instead of importing host APIs. The
//! local adapter is the only implementation that touches the process host;
//! deterministic tests can use the in-memory adapter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::boundary::{resolve_workspace_read_path, resolve_workspace_write_path};
use crate::workspace::{Workspace, WorkspaceKind};

const MAX_IN_MEMORY_FILES: usize = 4_096;
const MAX_OBSERVATIONS: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionEnvironmentIdentity {
    pub adapter: String,
    pub workspace_kind: WorkspaceKind,
    /// A stable redacted identity. The canonical local path is never persisted
    /// in this structure or in resume diagnostics.
    pub workspace_digest: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ExecutionCapabilities {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub process_run: bool,
    pub process_stdio: bool,
    pub observations: bool,
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

#[async_trait]
pub trait WorkspaceFileSystem: Send + Sync {
    fn root(&self) -> &Path;
    async fn read_utf8(&self, raw_path: &str) -> Result<String, EnvironmentError>;
    async fn write_utf8(
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
        self.kill();
    }
}

pub trait ArtifactSink: Send + Sync {}

#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    fn identity(&self) -> &ExecutionEnvironmentIdentity;
    fn filesystem(&self) -> &dyn WorkspaceFileSystem;
    fn processes(&self) -> &dyn ProcessHost;
    fn artifacts(&self) -> Option<&dyn ArtifactSink>;
    fn capabilities(&self) -> &ExecutionCapabilities;
}

pub struct LocalExecutionEnvironment {
    identity: ExecutionEnvironmentIdentity,
    filesystem: LocalFileSystem,
    processes: LocalProcessHost,
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
            },
            capabilities: ExecutionCapabilities {
                filesystem_read: true,
                filesystem_write: true,
                process_run: true,
                process_stdio: true,
                observations: true,
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
        None
    }

    fn capabilities(&self) -> &ExecutionCapabilities {
        &self.capabilities
    }
}

pub struct InMemoryExecutionEnvironment {
    identity: ExecutionEnvironmentIdentity,
    filesystem: InMemoryFileSystem,
    processes: InMemoryProcessHost,
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
        None
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
        let before = match tokio::fs::read_to_string(&path).await {
            Ok(value) => Some(value),
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

    async fn list_files(
        &self,
        raw_path: Option<&str>,
        max_files: usize,
    ) -> Result<Vec<WorkspaceFileEntry>, EnvironmentError> {
        let root = self.root.clone();
        let raw_path = raw_path.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            let search_root = match raw_path.as_deref() {
                None | Some("") | Some(".") => root
                    .canonicalize()
                    .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?,
                Some(path) => resolve_workspace_read_path(&root, path)
                    .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?,
            };
            let canonical_root = root
                .canonicalize()
                .map_err(|error| EnvironmentError::InvalidPath(error.to_string()))?;
            if !search_root.starts_with(&canonical_root) {
                return Err(EnvironmentError::Boundary);
            }
            let mut entries = Vec::new();
            for entry in walkdir::WalkDir::new(search_root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || !matches!(
                            entry.file_name().to_string_lossy().as_ref(),
                            ".git"
                                | "target"
                                | "node_modules"
                                | ".next"
                                | "dist"
                                | "build"
                                | "__pycache__"
                                | ".rove"
                        )
                })
            {
                if entries.len() >= max_files {
                    break;
                }
                let entry = match entry {
                    Ok(value) if value.file_type().is_file() => value,
                    _ => continue,
                };
                let canonical = match entry.path().canonicalize() {
                    Ok(value) if value.starts_with(&canonical_root) => value,
                    _ => continue,
                };
                let relative = canonical
                    .strip_prefix(&canonical_root)
                    .map_err(|_| EnvironmentError::Boundary)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let byte_len = std::fs::metadata(&canonical)
                    .map_err(|error| EnvironmentError::Host(error.to_string()))?
                    .len() as usize;
                entries.push(WorkspaceFileEntry {
                    relative_path: relative,
                    byte_len,
                });
            }
            Ok(entries)
        })
        .await
        .map_err(|error| EnvironmentError::Host(error.to_string()))?
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
}

pub struct LocalProcessHost {
    root: PathBuf,
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
}

#[derive(Default)]
pub struct InMemoryProcessHost {
    responses: RwLock<BTreeMap<String, ProcessOutput>>,
    delays: RwLock<BTreeMap<String, Duration>>,
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

#[derive(Clone, Default)]
pub struct ObservationStore {
    observations: Arc<RwLock<BTreeMap<String, Observation>>>,
}

impl ObservationStore {
    pub async fn put(&self, observation: Observation) -> Result<(), EnvironmentError> {
        let mut values = self.observations.write().await;
        if values.len() >= MAX_OBSERVATIONS && !values.contains_key(&observation.id) {
            return Err(EnvironmentError::Host(
                "observation limit reached".to_string(),
            ));
        }
        values.insert(observation.id.clone(), observation);
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<Observation> {
        self.observations.read().await.get(id).cloned()
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

fn rove_runtime_hash_workspace(workspace: &Workspace) -> String {
    crate::context::prompt_metadata::workspace_fingerprint(workspace)
}
