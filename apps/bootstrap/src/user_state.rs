//! Shared cross-platform user state directory contract.
//!
//! This module owns the resolution of the Rove user data root and the
//! per-workspace state layout used by every first-party entry point
//! (CLI, API, embedded hosts, and the migration tooling). Runtime crates
//! keep receiving already-resolved absolute paths; home-directory
//! discovery stays in `rove-app-bootstrap`, next to
//! [`crate::user_config::UserConfigPaths`] and the operator trust store.
//!
//! Layout (see `docs/design/2026-08-16-user-state-directory-migration-design.md`):
//!
//! ```text
//! <data_root>/workspaces/<storage_key>/
//! ├── workspace.json            # identity marker
//! ├── state.sqlite
//! ├── mcp_servers.json
//! ├── runs/<run_id>/…
//! ├── memory/{MEMORY.md, topics/, sessions/}
//! ├── session-model-selections/
//! ├── circuit_breakers.json
//! ├── tasks/<name>/…
//! └── repl_history
//! ```
//!
//! Resolution is fail-closed: an explicit override must be absolute, and a
//! missing platform base is an error instead of a silent fallback.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rove_runtime::context::prompt_metadata::stable_hash;
use serde::{Deserialize, Serialize};

use crate::project_trust::canonical_root_key;

/// Environment variable overriding the user data root. Must be absolute.
pub const DATA_ROOT_ENV: &str = "ROVE_DATA_ROOT";

/// Marker file name inside a contract-managed workspace directory.
pub const WORKSPACE_MARKER_FILE: &str = "workspace.json";

/// Current schema version of `workspace.json` markers.
pub const WORKSPACE_MARKER_SCHEMA_VERSION: i64 = 1;

/// Legacy project-local state directory that migration imports from.
pub const LEGACY_STATE_DIR: &str = ".rove";

const STORAGE_KEY_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum UserStateError {
    #[error("the user data root is unavailable: {message}")]
    Unavailable { message: String },
    #[error("workspace marker mismatch at {path}: {message}")]
    MarkerMismatch { path: PathBuf, message: String },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The resolved user data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserStateRoots {
    root: PathBuf,
    /// Non-empty when the root came from an explicit environment override.
    source: &'static str,
}

impl UserStateRoots {
    /// Discover the data root from the environment or platform conventions.
    ///
    /// `ROVE_DATA_ROOT` must be an absolute path; relative values fail
    /// closed instead of resolving against an ambiguous working directory.
    pub fn discover() -> Result<Self, UserStateError> {
        Self::discover_with_override(None)
    }

    /// Discover with an explicit injection point for embedders and tests
    /// that must not depend on process-global environment state. The
    /// explicit root must be absolute.
    pub fn discover_with_override(injected: Option<PathBuf>) -> Result<Self, UserStateError> {
        let configured = match injected {
            Some(root) if root.is_absolute() => Some(root),
            Some(_) => {
                return Err(UserStateError::Unavailable {
                    message: "the injected data root must be an absolute path".to_string(),
                });
            }
            None => std::env::var_os(DATA_ROOT_ENV).map(PathBuf::from),
        };
        Self::discover_from(configured, platform_data_base())
    }

    /// Build an explicit data root (tests, embedders, migration tooling).
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: normalize_unresolved_root(&root),
            source: "",
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn override_source(&self) -> Option<&'static str> {
        (!self.source.is_empty()).then_some(self.source)
    }

    /// Resolve one workspace against this already-pinned root. Callers should
    /// retain the returned roots instead of rediscovering process environment
    /// variables while rebasing a request.
    pub fn workspace_layout(&self, workspace_root: &Path) -> WorkspaceStateLayout {
        WorkspaceStateLayout::resolve(&self.root, workspace_root)
    }

    fn discover_from(
        configured: Option<PathBuf>,
        platform_base: Option<PathBuf>,
    ) -> Result<Self, UserStateError> {
        if let Some(root) = configured {
            if !root.is_absolute() {
                return Err(UserStateError::Unavailable {
                    message: format!("{DATA_ROOT_ENV} must be an absolute path"),
                });
            }
            return Ok(Self {
                root: normalize_unresolved_root(&root),
                source: DATA_ROOT_ENV,
            });
        }
        let Some(base) = platform_base else {
            return Err(UserStateError::Unavailable {
                message: "no user data root is available because the platform data base and \
                          {DATA_ROOT_ENV} are unset"
                    .to_string(),
            });
        };
        if !base.is_absolute() {
            return Err(UserStateError::Unavailable {
                message: "the platform data base must resolve to an absolute path".to_string(),
            });
        }
        Ok(Self {
            root: normalize_unresolved_root(&base.join("rove")),
            source: "",
        })
    }

    /// Redact a path that lives under this data root for API/Web display.
    ///
    /// Paths outside the data root are returned unchanged (as display
    /// strings); callers still decide whether to show them at all.
    pub fn redact(&self, path: &Path) -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        match strip_prefix_platform(&normalize_unresolved_root(path), &self.root) {
            Some(relative) if relative.as_os_str().is_empty() => "<rove-data>".to_string(),
            Some(relative) => format!(
                "<rove-data>/{}",
                relative.to_string_lossy().replace('\\', "/")
            ),
            None => text,
        }
    }
}

fn platform_data_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
        })
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.as_os_str().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
}

/// Stable storage identity for a workspace root.
///
/// The key is derived from the canonicalized root path and the `.git`
/// filesystem fact only. It deliberately differs from the Project Trust
/// identity digest (which also binds filesystem identity such as inode or
/// creation time): storage needs "same workspace from any entry point maps
/// to one directory", while trust needs "detect when a directory was
/// replaced".
pub fn workspace_storage_key(workspace_root: &Path) -> String {
    let canonical = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| normalize_lexical(workspace_root));
    let kind = if canonical.join(".git").exists() {
        "repo"
    } else {
        "folder"
    };
    let key = canonical_root_key(&canonical);
    let digest = stable_hash(&format!("{key}|{kind}"));
    let hex = digest.trim_start_matches("sha256:").to_ascii_lowercase();
    hex.chars().take(STORAGE_KEY_LEN).collect()
}

/// The contract-resolved directory layout for one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStateLayout {
    pub data_root: PathBuf,
    pub storage_key: String,
    pub canonical_workspace_root: PathBuf,
    pub workspace_dir: PathBuf,
    pub state_sqlite: PathBuf,
    pub product_sqlite: PathBuf,
    pub mcp_catalog: PathBuf,
    pub memory_dir: PathBuf,
    pub memory_sessions_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub tasks_base: PathBuf,
}

impl WorkspaceStateLayout {
    pub fn resolve(data_root: &Path, workspace_root: &Path) -> Self {
        let canonical_workspace_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(workspace_root));
        let storage_key = workspace_storage_key(workspace_root);
        let workspace_dir = data_root.join("workspaces").join(&storage_key);
        let memory_dir = workspace_dir.join("memory");
        Self {
            data_root: data_root.to_path_buf(),
            storage_key,
            state_sqlite: workspace_dir.join("state.sqlite"),
            // ProductStore is API-global. It must not be duplicated under
            // every workspace runtime directory.
            product_sqlite: data_root.join("product.sqlite"),
            mcp_catalog: workspace_dir.join("mcp_servers.json"),
            memory_sessions_dir: memory_dir.join("sessions"),
            runs_dir: workspace_dir.join("runs"),
            tasks_base: workspace_dir.join("tasks"),
            workspace_dir,
            canonical_workspace_root,
            memory_dir,
        }
    }

    /// Path of the identity marker inside the workspace directory.
    pub fn marker_path(&self) -> PathBuf {
        self.workspace_dir.join(WORKSPACE_MARKER_FILE)
    }

    /// Verify an already-materialized workspace marker without creating any
    /// directories or files. Runtime import/resume code uses this at its
    /// storage boundary before accepting a user-state path as canonical.
    pub fn verify_marker(&self) -> Result<(), UserStateError> {
        verify_workspace_marker(&self.workspace_dir, &self.canonical_workspace_root)
    }

    /// Ensure the contract workspace directory exists (0700 on unix) and
    /// carries a matching identity marker.
    pub fn ensure(&self) -> Result<(), UserStateError> {
        if path_starts_with_platform(
            &normalize_unresolved_root(&self.data_root),
            &self.canonical_workspace_root,
        ) {
            return Err(UserStateError::Unavailable {
                message: "user data root must not be inside the selected workspace".to_string(),
            });
        }
        reject_existing_symlink(&self.data_root, "user data root")?;
        reject_existing_symlink(&self.data_root.join("workspaces"), "user workspaces root")?;
        reject_existing_symlink(&self.workspace_dir, "contract workspace directory")?;
        if let Some(parent) = self.workspace_dir.parent() {
            reject_existing_symlink(parent, "contract workspace parent")?;
        }
        std::fs::create_dir_all(&self.data_root)?;
        std::fs::create_dir_all(self.data_root.join("workspaces"))?;
        std::fs::create_dir_all(&self.workspace_dir)?;
        reject_existing_symlink(&self.data_root, "user data root")?;
        reject_existing_symlink(&self.data_root.join("workspaces"), "user workspaces root")?;
        reject_existing_symlink(&self.workspace_dir, "contract workspace directory")?;
        let canonical_root = self.data_root.canonicalize()?;
        let canonical_workspace = self.workspace_dir.canonicalize()?;
        if path_starts_with_platform(&canonical_root, &self.canonical_workspace_root) {
            return Err(UserStateError::Unavailable {
                message: "user data root must not be inside the selected workspace".to_string(),
            });
        }
        if !path_starts_with_platform(&canonical_workspace, &canonical_root) {
            return Err(UserStateError::Unavailable {
                message: "contract workspace directory escapes the user data root".to_string(),
            });
        }
        crate::user_config::harden_directory_permissions(&self.workspace_dir);
        self.write_or_verify_marker()
    }

    /// Write the marker on first use, or fail visibly when an existing
    /// marker describes a different workspace (hash-collision guard).
    fn write_or_verify_marker(&self) -> Result<(), UserStateError> {
        let marker = WorkspaceMarker {
            schema_version: WORKSPACE_MARKER_SCHEMA_VERSION,
            canonical_root: self
                .canonical_workspace_root
                .to_string_lossy()
                .replace('\\', "/"),
            storage_key: self.storage_key.clone(),
        };
        let path = self.marker_path();
        if let Ok(metadata) = std::fs::symlink_metadata(&path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(UserStateError::MarkerMismatch {
                path,
                message: "workspace marker must be a regular file".to_string(),
            });
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                let existing: WorkspaceMarker = serde_json::from_slice(&bytes).map_err(|err| {
                    UserStateError::MarkerMismatch {
                        path: path.clone(),
                        message: format!("unreadable marker: {err}"),
                    }
                })?;
                if existing.schema_version != WORKSPACE_MARKER_SCHEMA_VERSION
                    || existing.storage_key != marker.storage_key
                    || !same_marker_root(&existing.canonical_root, &marker.canonical_root)
                {
                    return Err(UserStateError::MarkerMismatch {
                        path,
                        message: format!(
                            "marker describes workspace {} (schema {}) but this workspace is {}",
                            existing.canonical_root, existing.schema_version, marker.canonical_root
                        ),
                    });
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default();
                let tmp = self.workspace_dir.join(format!(
                    ".{}.{}-{nonce}.tmp",
                    WORKSPACE_MARKER_FILE,
                    std::process::id()
                ));
                std::fs::write(&tmp, serde_json::to_vec_pretty(&marker)?)?;
                crate::user_config::harden_file_permissions(&tmp);
                match std::fs::hard_link(&tmp, &path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let _ = std::fs::remove_file(&tmp);
                        // Another process won the first-write race. Verify
                        // its marker below rather than accepting it blindly.
                        let bytes = std::fs::read(&path)?;
                        let existing: WorkspaceMarker = serde_json::from_slice(&bytes)?;
                        if existing.schema_version != WORKSPACE_MARKER_SCHEMA_VERSION
                            || existing.storage_key != marker.storage_key
                            || !same_marker_root(&existing.canonical_root, &marker.canonical_root)
                        {
                            return Err(UserStateError::MarkerMismatch {
                                path,
                                message: "concurrent marker describes another workspace"
                                    .to_string(),
                            });
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
                let _ = std::fs::remove_file(&tmp);
            }
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }
}

fn reject_existing_symlink(path: &Path, label: &str) -> Result<(), UserStateError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(UserStateError::Unavailable {
            message: format!("{label} must not be a symlink"),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Verify the identity marker for a contract workspace directory.
pub fn verify_workspace_marker(
    workspace_dir: &Path,
    workspace_root: &Path,
) -> Result<(), UserStateError> {
    let metadata = std::fs::symlink_metadata(workspace_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(UserStateError::Unavailable {
            message: "the contract workspace directory must be a real directory".to_string(),
        });
    }
    let canonical_workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| normalize_lexical(workspace_root));
    let expected = WorkspaceMarker {
        schema_version: WORKSPACE_MARKER_SCHEMA_VERSION,
        canonical_root: canonical_workspace_root
            .to_string_lossy()
            .replace('\\', "/"),
        storage_key: workspace_storage_key(workspace_root),
    };
    let marker_path = workspace_dir.join(WORKSPACE_MARKER_FILE);
    let marker_metadata = std::fs::symlink_metadata(&marker_path)?;
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Err(UserStateError::MarkerMismatch {
            path: marker_path,
            message: "workspace marker must be a regular file".to_string(),
        });
    }
    let bytes = std::fs::read(&marker_path)?;
    let actual: WorkspaceMarker =
        serde_json::from_slice(&bytes).map_err(|error| UserStateError::MarkerMismatch {
            path: marker_path.clone(),
            message: format!("unreadable marker: {error}"),
        })?;
    if actual != expected
        && !(same_marker_root(&actual.canonical_root, &expected.canonical_root)
            && actual.schema_version == expected.schema_version
            && actual.storage_key == expected.storage_key)
    {
        return Err(UserStateError::MarkerMismatch {
            path: marker_path,
            message: format!(
                "marker describes workspace {} (schema {}) but this workspace is {}",
                actual.canonical_root, actual.schema_version, expected.canonical_root
            ),
        });
    }
    Ok(())
}

/// Authority for an MCP catalog. A path alone is intentionally insufficient:
/// callers must carry whether the file belongs to the selected project or to
/// the pinned user-state workspace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpCatalogAuthority {
    Workspace {
        path: PathBuf,
    },
    UserState {
        path: PathBuf,
        workspace_dir: PathBuf,
    },
}

impl McpCatalogAuthority {
    pub fn path(&self) -> &Path {
        match self {
            Self::Workspace { path } | Self::UserState { path, .. } => path,
        }
    }

    pub fn is_user_state(&self) -> bool {
        matches!(self, Self::UserState { .. })
    }

    /// Revalidate the authority at the read/dispatch boundary. This rejects
    /// provider/MCP-supplied absolute paths and symlink escapes even if a
    /// caller constructed an authority from stale data.
    pub fn validate(&self, workspace_root: &Path) -> Result<(), UserStateError> {
        let (path, bound): (&Path, &Path) = match self {
            Self::Workspace { path } => (path.as_path(), workspace_root),
            Self::UserState {
                path,
                workspace_dir,
            } => (path.as_path(), workspace_dir.as_path()),
        };
        if matches!(self, Self::UserState { .. }) {
            match std::fs::symlink_metadata(bound) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(UserStateError::Unavailable {
                        message: "MCP catalog authority must be a real directory".to_string(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let canonical_bound = nearest_existing_canonical(bound)?;
        let canonical_parent = nearest_existing_canonical(path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "MCP catalog has no parent")
        })?)?;
        if !path_starts_with_platform(&canonical_parent, &canonical_bound) {
            return Err(UserStateError::Unavailable {
                message: "MCP catalog escapes its declared authority".to_string(),
            });
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(UserStateError::Unavailable {
                    message: "MCP catalog must not be a symlink".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

/// The catalog a default (unconfigured) MCP setup should read for a
/// workspace: the contract path when it already exists, otherwise the
/// legacy project-local file while it still exists, otherwise `None`.
///
/// The contract path always wins once it materializes (first Settings
/// write or migration), so deleting servers in Settings can never
/// resurrect a stale legacy catalog.
pub fn effective_default_mcp_catalog(workspace_root: &Path) -> Option<PathBuf> {
    let legacy = workspace_root
        .join(LEGACY_STATE_DIR)
        .join("mcp_servers.json");
    let contract = UserStateRoots::discover()
        .ok()
        .map(|roots| WorkspaceStateLayout::resolve(roots.root(), workspace_root).mcp_catalog);
    match contract {
        Some(contract) if contract.is_file() => Some(contract),
        _ => legacy.is_file().then_some(legacy),
    }
}

pub fn effective_default_mcp_authority(
    roots: &UserStateRoots,
    workspace_root: &Path,
) -> McpCatalogAuthority {
    let layout = roots.workspace_layout(workspace_root);
    if layout.mcp_catalog.is_file()
        || !workspace_root
            .join(LEGACY_STATE_DIR)
            .join("mcp_servers.json")
            .is_file()
    {
        McpCatalogAuthority::UserState {
            path: layout.mcp_catalog,
            workspace_dir: layout.workspace_dir,
        }
    } else {
        McpCatalogAuthority::Workspace {
            path: workspace_root
                .join(LEGACY_STATE_DIR)
                .join("mcp_servers.json"),
        }
    }
}

/// Discover and ensure the contract workspace directory for a workspace
/// root (directory creation, 0700 hardening, identity marker).
pub fn ensure_workspace_layout(
    workspace_root: &Path,
) -> Result<WorkspaceStateLayout, UserStateError> {
    let roots = UserStateRoots::discover()?;
    let layout = WorkspaceStateLayout::resolve(roots.root(), workspace_root);
    layout.ensure()?;
    Ok(layout)
}

/// The state directory run discovery should use for a workspace: the
/// contract directory once runtime state materializes, otherwise the legacy
/// project-local `.rove` layout. A marker or MCP-only directory must not hide
/// unmigrated legacy runs.
pub fn state_dir_for_run_discovery(workspace_root: &Path) -> PathBuf {
    if let Ok(roots) = UserStateRoots::discover() {
        let layout = WorkspaceStateLayout::resolve(roots.root(), workspace_root);
        if layout.workspace_dir.is_dir()
            && (layout.state_sqlite.is_file() || layout.runs_dir.is_dir())
        {
            return layout.workspace_dir;
        }
    }
    workspace_root.join(LEGACY_STATE_DIR)
}

fn normalize_unresolved_root(root: &Path) -> PathBuf {
    if root.exists() {
        return canonicalize_platform(root);
    }
    let mut ancestor = root.to_path_buf();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        if let Some(name) = ancestor.file_name() {
            missing.push(name.to_os_string());
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        ancestor = parent.to_path_buf();
    }
    let mut normalized = canonicalize_platform(&ancestor);
    for name in missing.iter().rev() {
        normalized.push(name);
    }
    normalized
}

fn canonicalize_platform(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if text
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("\\\\?\\UNC\\"))
        {
            return PathBuf::from(format!("\\\\{}", &text[8..]));
        }
        if let Some(stripped) = text.strip_prefix("\\\\?\\") {
            let stripped = PathBuf::from(stripped);
            if stripped.is_absolute() {
                return stripped;
            }
        }
    }
    canonical
}

pub(crate) fn path_starts_with_platform(path: &Path, base: &Path) -> bool {
    strip_prefix_platform(path, base).is_some()
}

fn strip_prefix_platform(path: &Path, base: &Path) -> Option<PathBuf> {
    let path = canonicalize_platform(path);
    let base = canonicalize_platform(base);

    #[cfg(windows)]
    {
        let mut path_components = path.components();
        for base_component in base.components() {
            let matches = path_components.next().is_some_and(|path_component| {
                path_component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&base_component.as_os_str().to_string_lossy())
            });
            if !matches {
                return None;
            }
        }
        let mut relative = PathBuf::new();
        for component in path_components {
            relative.push(component.as_os_str());
        }
        Some(relative)
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(base).ok().map(Path::to_path_buf)
    }
}

fn nearest_existing_canonical(path: &Path) -> Result<PathBuf, io::Error> {
    let mut current = path.to_path_buf();
    while !current.exists() {
        current = current
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing path ancestor"))?
            .to_path_buf();
    }
    current.canonicalize()
}

/// Marker document stored as `workspace.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceMarker {
    pub schema_version: i64,
    pub canonical_root: String,
    pub storage_key: String,
}

fn same_marker_root(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        let normalize = |value: &str| {
            let path = PathBuf::from(value);
            canonicalize_platform(&path)
                .to_string_lossy()
                .replace('\\', "/")
        };
        normalize(a).eq_ignore_ascii_case(&normalize(b))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Process-wide guard for tests that mutate `ROVE_DATA_ROOT`. Config,
/// trust, and migration tests share it so parallel environment edits stay
/// serial.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn platform_absolute(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\{}", path.replace('/', r"\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    #[test]
    fn data_root_override_must_be_absolute() {
        let error = UserStateRoots::discover_from(Some(PathBuf::from("relative")), None)
            .expect_err("relative override must fail closed");
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn data_root_fails_closed_without_platform_base() {
        let error =
            UserStateRoots::discover_from(None, None).expect_err("missing base must fail closed");
        assert!(error.to_string().contains("unavailable"));
    }

    #[test]
    fn data_root_joins_platform_base() {
        let base = platform_absolute("base/data");
        let roots = UserStateRoots::discover_from(None, Some(base.clone()))
            .expect("platform base resolves");
        assert_eq!(roots.root(), base.join("rove"));
        assert_eq!(roots.override_source(), None);
    }

    #[test]
    fn data_root_override_wins_without_platform_base() {
        let explicit = platform_absolute("explicit/rove");
        let roots = UserStateRoots::discover_from(Some(explicit.clone()), None)
            .expect("explicit override resolves");
        assert_eq!(roots.root(), explicit);
        assert_eq!(roots.override_source(), Some(DATA_ROOT_ENV));
    }

    #[test]
    fn redact_replaces_data_root_prefix() {
        let roots = UserStateRoots::from_root(platform_absolute("base/data/rove"));
        let inside = roots.root().join("workspaces").join("ab").join("memory");
        let display = roots.redact(&inside);
        assert_eq!(display, "<rove-data>/workspaces/ab/memory");
        assert_eq!(
            roots.redact(&platform_absolute("elsewhere/file.txt")),
            platform_absolute("elsewhere/file.txt")
                .to_string_lossy()
                .replace('\\', "/")
        );
    }

    #[test]
    fn storage_key_is_stable_and_isolated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let first = workspace_storage_key(&root);
        assert_eq!(first.len(), STORAGE_KEY_LEN);
        assert_eq!(workspace_storage_key(&root), first, "same root is stable");

        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert_ne!(
            workspace_storage_key(&other.canonicalize().unwrap()),
            first,
            "different roots must not collide"
        );
    }

    #[test]
    fn storage_key_is_stable_through_a_symlink_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        if !create_directory_symlink(&real, &link) {
            // Symlink creation needs privileges on some Windows setups; the
            // canonicalization behavior is covered on the other platforms.
            return;
        }

        assert_eq!(
            workspace_storage_key(&link),
            workspace_storage_key(&real.canonicalize().unwrap()),
            "symlinked entry canonicalizes to the same storage key"
        );
    }

    #[test]
    fn layout_paths_live_under_the_workspace_directory() {
        let layout =
            WorkspaceStateLayout::resolve(&platform_absolute("data"), &platform_absolute("ws"));
        assert!(layout.workspace_dir.ends_with(layout.storage_key.as_str()));
        assert_eq!(
            layout.state_sqlite,
            layout.workspace_dir.join("state.sqlite")
        );
        assert_eq!(
            layout.product_sqlite,
            layout.data_root.join("product.sqlite")
        );
        assert_eq!(
            layout.mcp_catalog,
            layout.workspace_dir.join("mcp_servers.json")
        );
        assert_eq!(
            layout.memory_sessions_dir,
            layout.memory_dir.join("sessions")
        );
        assert_eq!(layout.runs_dir, layout.workspace_dir.join("runs"));
    }

    #[test]
    fn marker_round_trip_and_mismatch_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_root = tmp.path().join("data");
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
        layout.ensure().expect("first ensure writes the marker");
        layout
            .ensure()
            .expect("second ensure verifies the same marker");

        // A marker describing a different root under the same storage key is
        // the hash-collision guard; it must fail visibly, not merge silently.
        let conflicting = WorkspaceMarker {
            schema_version: WORKSPACE_MARKER_SCHEMA_VERSION,
            canonical_root: platform_absolute("somewhere/else")
                .to_string_lossy()
                .replace('\\', "/"),
            storage_key: layout.storage_key.clone(),
        };
        std::fs::write(
            layout.marker_path(),
            serde_json::to_vec(&conflicting).unwrap(),
        )
        .unwrap();
        let error = layout.ensure().expect_err("conflicting marker must fail");
        match error {
            UserStateError::MarkerMismatch { .. } => {}
            other => panic!("expected marker mismatch, got {other:?}"),
        }
    }

    #[test]
    fn ensure_rejects_a_data_root_inside_the_workspace_without_creating_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let data_root = workspace.join("user-data");
        let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);

        let error = layout
            .ensure()
            .expect_err("nested user data root must fail closed");
        assert!(error.to_string().contains("must not be inside"));
        assert!(!data_root.exists());
    }

    #[test]
    fn verify_marker_rejects_tampered_content_without_recreating_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_root = tmp.path().join("data");
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
        layout.ensure().unwrap();

        let mut marker: WorkspaceMarker =
            serde_json::from_slice(&std::fs::read(layout.marker_path()).unwrap()).unwrap();
        marker.storage_key.push('0');
        std::fs::write(layout.marker_path(), serde_json::to_vec(&marker).unwrap()).unwrap();

        let error = layout
            .verify_marker()
            .expect_err("tampered identity marker must fail closed");
        assert!(matches!(error, UserStateError::MarkerMismatch { .. }));
    }

    #[test]
    fn ensure_rejects_a_symlinked_workspace_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_root = tmp.path().join("data");
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
        layout.ensure().unwrap();
        let marker_bytes = std::fs::read(layout.marker_path()).unwrap();
        let outside = tmp.path().join("outside-marker.json");
        std::fs::write(&outside, marker_bytes).unwrap();
        std::fs::remove_file(layout.marker_path()).unwrap();
        if !create_file_symlink(&outside, &layout.marker_path()) {
            return;
        }

        let error = layout
            .ensure()
            .expect_err("a symlinked identity marker must fail closed");
        assert!(matches!(error, UserStateError::MarkerMismatch { .. }));
    }

    #[test]
    fn dangling_mcp_catalog_symlink_is_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");
        let data_root = tmp.path().join("data");
        std::fs::create_dir_all(&workspace).unwrap();
        let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
        std::fs::create_dir_all(&layout.workspace_dir).unwrap();
        if !create_file_symlink(&tmp.path().join("missing-mcp.json"), &layout.mcp_catalog) {
            return;
        }

        let authority = McpCatalogAuthority::UserState {
            path: layout.mcp_catalog.clone(),
            workspace_dir: layout.workspace_dir.clone(),
        };
        let error = authority
            .validate(&workspace)
            .expect_err("dangling MCP symlink must fail closed");
        assert!(error.to_string().contains("must not be a symlink"));
    }

    #[cfg(windows)]
    #[test]
    fn extended_unc_paths_remain_absolute_when_normalized() {
        let normalized = canonicalize_platform(Path::new(r"\\?\UNC\server\share\workspace"));

        assert_eq!(normalized, PathBuf::from(r"\\server\share\workspace"));
        assert!(normalized.is_absolute());
        assert!(path_starts_with_platform(
            &normalized.join("state.sqlite"),
            Path::new(r"\\SERVER\SHARE\workspace")
        ));
    }
}
