use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use rove_runtime::context::prompt_metadata::stable_hash;
use rove_runtime::workspace::WorkspaceKind;
use rove_runtime::workspace::boundary::resolve_workspace_read_path;

pub const TRUSTED_WORKSPACES_ENV: &str = "ROVE_TRUSTED_WORKSPACES";
pub const PROJECT_TRUST_STORE_ENV: &str = "ROVE_PROJECT_TRUST_STORE";
pub const PROJECT_TRUST_FILE_NAME: &str = "project-trust.json";
pub const PROJECT_TRUST_SCHEMA_VERSION: u32 = 1;
const MAX_TRUST_INPUT_BYTES: usize = 256 * 1024;

pub const CAP_PROJECT_CONFIGURATION: &str = "project_configuration";
pub const CAP_WORKSPACE_INSTRUCTIONS: &str = "workspace_instructions";
pub const CAP_MCP_PROCESSES: &str = "mcp_processes";
pub const CAP_HOOKS_EXTENSIONS: &str = "hooks_extensions";
pub const CAP_PROVIDER_CREDENTIALS: &str = "provider_credentials";
pub const CAP_EXTERNAL_PATHS: &str = "external_paths";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivationState {
    Unknown,
    #[default]
    Restricted,
    Trusted,
    Revoked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivationSource {
    Programmatic,
    CommandLine,
    Environment,
    Durable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTrustDecision {
    Grant,
    Deny,
    Revoke,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTrustCapability {
    ProjectConfiguration,
    WorkspaceInstructions,
    McpProcesses,
    HooksExtensions,
    ProviderCredentials,
    ExternalPaths,
}

impl ProjectTrustCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectConfiguration => CAP_PROJECT_CONFIGURATION,
            Self::WorkspaceInstructions => CAP_WORKSPACE_INSTRUCTIONS,
            Self::McpProcesses => CAP_MCP_PROCESSES,
            Self::HooksExtensions => CAP_HOOKS_EXTENSIONS,
            Self::ProviderCredentials => CAP_PROVIDER_CREDENTIALS,
            Self::ExternalPaths => CAP_EXTERNAL_PATHS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectTrustRecord {
    pub canonical_root: String,
    pub workspace_kind: WorkspaceKind,
    pub identity_digest: String,
    pub state: ProjectActivationState,
    #[serde(default)]
    pub capability_digests: BTreeMap<String, String>,
    pub granted_at: Option<String>,
    pub revoked_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TrustFile {
    schema_version: u32,
    records: Vec<ProjectTrustRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustResolution {
    pub state: ProjectActivationState,
    pub identity_digest: String,
    pub invalidated_capabilities: Vec<String>,
    pub granted_capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectActivation {
    pub state: ProjectActivationState,
    pub source: Option<ProjectActivationSource>,
    pub trusted_workspace_roots: Vec<PathBuf>,
    pub granted_capabilities: BTreeSet<String>,
}

impl ProjectActivation {
    pub(crate) fn resolve(
        workspace_root: &Path,
        command_line_grant: bool,
        trusted_workspaces: Option<OsString>,
    ) -> anyhow::Result<Self> {
        let workspace_root = canonical_directory(workspace_root)?;
        let mut trusted_workspace_roots = Vec::new();
        let mut seen = HashSet::new();

        if let Some(raw) = trusted_workspaces {
            for path in std::env::split_paths(&raw) {
                if path.as_os_str().is_empty() {
                    continue;
                }
                let path = canonical_directory(&path).map_err(|error| {
                    anyhow::anyhow!(
                        "{TRUSTED_WORKSPACES_ENV} contains an invalid workspace path: {error}"
                    )
                })?;
                if seen.insert(path.clone()) {
                    trusted_workspace_roots.push(path);
                }
            }
        }

        if command_line_grant && seen.insert(workspace_root.clone()) {
            trusted_workspace_roots.push(workspace_root.clone());
        }

        let trusted = trusted_workspace_roots.contains(&workspace_root);
        let source = if command_line_grant {
            Some(ProjectActivationSource::CommandLine)
        } else if trusted {
            Some(ProjectActivationSource::Environment)
        } else {
            None
        };
        Ok(Self {
            state: if trusted {
                ProjectActivationState::Trusted
            } else {
                ProjectActivationState::Restricted
            },
            source,
            trusted_workspace_roots,
            granted_capabilities: all_capabilities(),
        })
    }

    pub(crate) fn programmatic() -> Self {
        Self {
            state: ProjectActivationState::Trusted,
            source: Some(ProjectActivationSource::Programmatic),
            trusted_workspace_roots: Vec::new(),
            granted_capabilities: all_capabilities(),
        }
    }

    pub(crate) fn durable(
        resolution: ProjectTrustResolution,
        trusted_workspace_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            state: resolution.state,
            source: (resolution.state == ProjectActivationState::Trusted)
                .then_some(ProjectActivationSource::Durable),
            trusted_workspace_roots,
            granted_capabilities: resolution.granted_capabilities,
        }
    }

    pub(crate) fn for_workspace(&self, workspace_root: &Path) -> Self {
        if self.source == Some(ProjectActivationSource::Programmatic) {
            return Self::programmatic();
        }
        let canonical = canonical_directory(workspace_root).ok();
        let trusted = canonical
            .as_ref()
            .is_some_and(|path| self.trusted_workspace_roots.contains(path));
        Self {
            state: if trusted {
                ProjectActivationState::Trusted
            } else {
                ProjectActivationState::Restricted
            },
            source: if trusted { self.source } else { None },
            trusted_workspace_roots: self.trusted_workspace_roots.clone(),
            granted_capabilities: if trusted {
                self.granted_capabilities.clone()
            } else {
                BTreeSet::new()
            },
        }
    }
}

/// Operator-owned durable trust repository. The file is intentionally opened
/// before project configuration and is written atomically through a sibling
/// temporary file. A project cannot grant itself trust by editing config.
#[derive(Debug, Clone)]
pub struct ProjectTrustRepository {
    path: PathBuf,
}

impl ProjectTrustRepository {
    /// Resolve the operator-owned trust store independently of the selected
    /// workspace, which must never be able to grant itself authority.
    pub fn operator_default() -> anyhow::Result<Self> {
        if let Some(path) = std::env::var_os(PROJECT_TRUST_STORE_ENV) {
            if path.is_empty() {
                anyhow::bail!("{PROJECT_TRUST_STORE_ENV} must not be empty");
            }
            return Ok(Self::new(path));
        }
        let base = operator_state_base().ok_or_else(|| {
            anyhow::anyhow!(
                "operator state directory is unavailable; set {PROJECT_TRUST_STORE_ENV}"
            )
        })?;
        Ok(Self::new(base.join("rove").join(PROJECT_TRUST_FILE_NAME)))
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> anyhow::Result<Vec<ProjectTrustRecord>> {
        let backup = self.path.with_extension("json.bak");
        let readable_path = if self.path.exists() {
            &self.path
        } else if backup.exists() {
            &backup
        } else {
            return Ok(Vec::new());
        };
        let bytes = std::fs::read(readable_path)?;
        if bytes.len() > 512 * 1024 {
            anyhow::bail!("project trust store exceeds the supported size");
        }
        let file: TrustFile = serde_json::from_slice(&bytes)?;
        if file.schema_version != PROJECT_TRUST_SCHEMA_VERSION {
            anyhow::bail!("unsupported project trust schema version");
        }
        Ok(file.records)
    }

    pub fn resolve(
        &self,
        workspace_root: &Path,
        workspace_kind: WorkspaceKind,
        capability_digests: &BTreeMap<String, String>,
    ) -> anyhow::Result<ProjectTrustResolution> {
        let canonical_root = canonical_directory(workspace_root)?;
        let identity_digest = workspace_identity_digest(&canonical_root, workspace_kind.clone());
        let canonical_text = canonical_root_key(&canonical_root);
        let record = self.load()?.into_iter().find(|record| {
            record.canonical_root == canonical_text && record.workspace_kind == workspace_kind
        });
        Ok(resolve_project_trust_record(
            record.as_ref(),
            identity_digest,
            capability_digests,
        ))
    }

    pub fn decide(
        &self,
        workspace_root: &Path,
        workspace_kind: WorkspaceKind,
        decision: ProjectTrustDecision,
        capability_digests: BTreeMap<String, String>,
    ) -> anyhow::Result<ProjectTrustRecord> {
        let canonical_root = canonical_directory(workspace_root)?;
        let identity_digest = workspace_identity_digest(&canonical_root, workspace_kind.clone());
        let now = now_rfc3339();
        let state = match decision {
            ProjectTrustDecision::Grant => ProjectActivationState::Trusted,
            ProjectTrustDecision::Deny => ProjectActivationState::Restricted,
            ProjectTrustDecision::Revoke => ProjectActivationState::Revoked,
        };
        let mut records = self.load()?;
        let canonical_root_text = canonical_root_key(&canonical_root);
        let record = ProjectTrustRecord {
            canonical_root: canonical_root_text.clone(),
            workspace_kind: workspace_kind.clone(),
            identity_digest,
            state,
            capability_digests: if state == ProjectActivationState::Trusted {
                capability_digests
            } else {
                BTreeMap::new()
            },
            granted_at: (state == ProjectActivationState::Trusted).then_some(now.clone()),
            revoked_at: (state == ProjectActivationState::Revoked).then_some(now.clone()),
            updated_at: now,
        };
        records.retain(|existing| {
            !(existing.canonical_root == canonical_root_text
                && existing.workspace_kind == workspace_kind)
        });
        records.push(record.clone());
        self.write(&records)?;
        Ok(record)
    }

    pub fn revoke(
        &self,
        workspace_root: &Path,
        workspace_kind: WorkspaceKind,
    ) -> anyhow::Result<ProjectTrustRecord> {
        self.decide(
            workspace_root,
            workspace_kind,
            ProjectTrustDecision::Revoke,
            BTreeMap::new(),
        )
    }

    fn write(&self, records: &[ProjectTrustRecord]) -> anyhow::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("project trust store has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(&TrustFile {
            schema_version: PROJECT_TRUST_SCHEMA_VERSION,
            records: records.to_vec(),
        })?;
        let temporary = self.path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(&temporary, bytes)?;
        if !self.path.exists() {
            std::fs::rename(&temporary, &self.path)?;
            return Ok(());
        }
        let backup = self.path.with_extension("json.bak");
        if backup.exists() {
            std::fs::remove_file(&backup)?;
        }
        std::fs::rename(&self.path, &backup)?;
        if let Err(error) = std::fs::rename(&temporary, &self.path) {
            let _ = std::fs::remove_file(&temporary);
            let _ = std::fs::rename(&backup, &self.path);
            return Err(error.into());
        }
        let _ = std::fs::remove_file(backup);
        Ok(())
    }
}

pub fn capability_digest_map(
    workspace_root: &Path,
    mcp_config: Option<&Path>,
    provider_selector: Option<&str>,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    result.insert(
        CAP_PROJECT_CONFIGURATION.to_string(),
        digest_workspace_file(
            workspace_root,
            Some(&workspace_root.join(".rove/config.toml")),
        )
        .unwrap_or_else(|| stable_hash("missing-project-config")),
    );
    result.insert(
        CAP_MCP_PROCESSES.to_string(),
        digest_workspace_file(workspace_root, mcp_config)
            .unwrap_or_else(|| stable_hash("missing-mcp-config")),
    );
    result.insert(
        CAP_PROVIDER_CREDENTIALS.to_string(),
        stable_hash(provider_selector.unwrap_or("provider-default")),
    );
    result.insert(
        CAP_WORKSPACE_INSTRUCTIONS.to_string(),
        digest_workspace_file(workspace_root, Some(&workspace_root.join("AGENTS.md")))
            .unwrap_or_else(|| stable_hash("missing-workspace-instructions")),
    );
    result.insert(
        CAP_HOOKS_EXTENSIONS.to_string(),
        stable_hash("hooks-not-configured"),
    );
    result.insert(
        CAP_EXTERNAL_PATHS.to_string(),
        stable_hash("external-paths-disabled"),
    );
    result
}

fn digest_workspace_file(workspace_root: &Path, path: Option<&Path>) -> Option<String> {
    let path = path?;
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace_root).ok()?
    } else {
        path
    };
    let raw_path = relative.to_string_lossy();
    let resolved = resolve_workspace_read_path(workspace_root, &raw_path).ok()?;
    let mut bytes = Vec::with_capacity(8 * 1024);
    std::fs::File::open(resolved)
        .ok()?
        .take((MAX_TRUST_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_TRUST_INPUT_BYTES {
        return Some(stable_hash(&format!("oversized-workspace-file:{raw_path}")));
    }
    Some(stable_hash(&String::from_utf8_lossy(&bytes)))
}

pub fn workspace_identity_digest(root: &Path, kind: WorkspaceKind) -> String {
    let metadata = std::fs::metadata(root).ok();
    let mut identity = format!("{}|{kind:?}", canonical_root_key(root));
    if let Some(metadata) = metadata {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            identity.push_str(&format!("|{}|{}", metadata.dev(), metadata.ino()));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            identity.push_str(&format!("|{}", metadata.creation_time()));
        }
        #[cfg(not(any(unix, windows)))]
        identity.push_str(&format!(
            "|{}",
            metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
    }
    stable_hash(&identity)
}

pub fn resolve_project_trust_record(
    record: Option<&ProjectTrustRecord>,
    identity_digest: String,
    capability_digests: &BTreeMap<String, String>,
) -> ProjectTrustResolution {
    let Some(record) = record else {
        return ProjectTrustResolution {
            state: ProjectActivationState::Restricted,
            identity_digest,
            invalidated_capabilities: Vec::new(),
            granted_capabilities: BTreeSet::new(),
        };
    };
    if record.identity_digest != identity_digest {
        return ProjectTrustResolution {
            state: ProjectActivationState::Unknown,
            identity_digest,
            invalidated_capabilities: all_capability_names(),
            granted_capabilities: BTreeSet::new(),
        };
    }
    let mut invalidated = Vec::new();
    let mut granted = BTreeSet::new();
    if record.state == ProjectActivationState::Trusted {
        for (capability, digest) in capability_digests {
            match record.capability_digests.get(capability) {
                Some(stored) if stored == digest => {
                    granted.insert(capability.clone());
                }
                Some(_) => invalidated.push(capability.clone()),
                None => {}
            }
        }
    }
    ProjectTrustResolution {
        state: record.state,
        identity_digest,
        invalidated_capabilities: invalidated,
        granted_capabilities: granted,
    }
}

pub fn all_capability_names() -> Vec<String> {
    [
        CAP_PROJECT_CONFIGURATION,
        CAP_WORKSPACE_INSTRUCTIONS,
        CAP_MCP_PROCESSES,
        CAP_HOOKS_EXTENSIONS,
        CAP_PROVIDER_CREDENTIALS,
        CAP_EXTERNAL_PATHS,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn all_capabilities() -> BTreeSet<String> {
    all_capability_names().into_iter().collect()
}

fn canonical_directory(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("{} is not a directory", canonical.display());
    }
    Ok(canonical)
}

pub fn canonical_root_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_lowercase();
    text
}

fn operator_state_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
    }
}

fn now_rfc3339() -> String {
    format!(
        "unix:{}",
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_grant_is_exact_root_and_digest_bound() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("trust.json"));
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let digests = capability_digest_map(&root, None, Some("OPENAI_API_KEY"));
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                digests.clone(),
            )
            .unwrap();
        let resolved = store
            .resolve(&root, WorkspaceKind::Folder, &digests)
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Trusted);
        assert_eq!(resolved.granted_capabilities.len(), digests.len());

        let mut changed = digests;
        changed.insert(CAP_MCP_PROCESSES.to_string(), stable_hash("changed"));
        let resolved = store
            .resolve(&root, WorkspaceKind::Folder, &changed)
            .unwrap();
        assert!(!resolved.granted_capabilities.contains(CAP_MCP_PROCESSES));
        assert!(
            resolved
                .invalidated_capabilities
                .iter()
                .any(|item| item == CAP_MCP_PROCESSES)
        );
    }

    #[test]
    fn replacement_identity_fails_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("trust.json"));
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let digests = capability_digest_map(&root, None, None);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                digests.clone(),
            )
            .unwrap();
        let old = workspace_identity_digest(&root, WorkspaceKind::Folder);
        let _ = std::fs::remove_dir(&root);
        std::fs::create_dir(&root).unwrap();
        let new = workspace_identity_digest(&root, WorkspaceKind::Folder);
        if old != new {
            let resolved = store
                .resolve(
                    &root,
                    WorkspaceKind::Folder,
                    &capability_digest_map(&root, None, None),
                )
                .unwrap();
            assert_eq!(resolved.state, ProjectActivationState::Unknown);
        }
    }

    #[test]
    fn revoke_replaces_the_durable_grant_and_blocks_every_capability() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let digests = capability_digest_map(&root, None, None);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                digests.clone(),
            )
            .unwrap();
        store.revoke(&root, WorkspaceKind::Folder).unwrap();

        let resolved = store
            .resolve(&root, WorkspaceKind::Folder, &digests)
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Revoked);
        assert!(resolved.granted_capabilities.is_empty());
        assert!(!store.path().starts_with(&root));
    }

    #[test]
    fn explicit_deny_persists_restricted_state_without_capabilities() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let digests = capability_digest_map(&root, None, None);

        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Deny,
                digests.clone(),
            )
            .unwrap();

        let resolved = store
            .resolve(&root, WorkspaceKind::Folder, &digests)
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Restricted);
        assert!(resolved.granted_capabilities.is_empty());
        assert!(resolved.invalidated_capabilities.is_empty());
    }

    #[test]
    fn nested_workspace_never_inherits_a_parent_grant() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let parent = temp.path().join("parent");
        let nested = parent.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let parent_digests = capability_digest_map(&parent, None, None);
        store
            .decide(
                &parent,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                parent_digests,
            )
            .unwrap();

        let resolved = store
            .resolve(
                &nested,
                WorkspaceKind::Folder,
                &capability_digest_map(&nested, None, None),
            )
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Restricted);
        assert!(resolved.granted_capabilities.is_empty());
    }

    #[test]
    fn canonical_symlink_alias_resolves_to_the_same_exact_root_when_supported() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let root = temp.path().join("workspace");
        let alias = temp.path().join("workspace-alias");
        std::fs::create_dir(&root).unwrap();
        if !create_directory_symlink(&root, &alias) {
            return;
        }
        let digests = capability_digest_map(&alias, None, None);
        store
            .decide(
                &alias,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                digests,
            )
            .unwrap();

        let resolved = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Trusted);
        assert_eq!(
            resolved.granted_capabilities.len(),
            all_capabilities().len()
        );
    }

    #[cfg(windows)]
    #[test]
    fn retargeted_windows_junction_does_not_reuse_the_original_grant() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let junction = temp.path().join("workspace-junction");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        if !create_directory_junction(&first, &junction) {
            return;
        }
        store
            .decide(
                &junction,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                capability_digest_map(&junction, None, None),
            )
            .unwrap();

        std::fs::remove_dir(&junction).unwrap();
        assert!(create_directory_junction(&second, &junction));
        let resolved = store
            .resolve(
                &junction,
                WorkspaceKind::Folder,
                &capability_digest_map(&junction, None, None),
            )
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Restricted);
        assert!(resolved.granted_capabilities.is_empty());
    }

    #[test]
    fn ordinary_workspace_changes_preserve_identity_and_invalidate_only_the_digest() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.json"));
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join(".rove")).unwrap();
        let project_config = root.join(".rove/config.toml");
        let mcp = root.join(".rove/mcp_servers.json");
        std::fs::write(&project_config, "[runtime]\nmax_steps = 8\n").unwrap();
        std::fs::write(&mcp, "[]").unwrap();
        let identity = workspace_identity_digest(&root, WorkspaceKind::Folder);
        let digests = capability_digest_map(&root, Some(&mcp), None);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                digests,
            )
            .unwrap();

        std::fs::write(&mcp, "[{\"name\":\"changed\"}]").unwrap();
        assert_eq!(
            workspace_identity_digest(&root, WorkspaceKind::Folder),
            identity
        );
        let resolved = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, Some(&mcp), None),
            )
            .unwrap();
        assert_eq!(resolved.state, ProjectActivationState::Trusted);
        assert_eq!(
            resolved.invalidated_capabilities,
            vec![CAP_MCP_PROCESSES.to_string()]
        );
        assert!(
            resolved
                .granted_capabilities
                .contains(CAP_PROJECT_CONFIGURATION)
        );

        std::fs::write(&mcp, "[]").unwrap();
        std::fs::write(&project_config, "[runtime]\nmax_steps = 9\n").unwrap();
        let resolved = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, Some(&mcp), None),
            )
            .unwrap();
        assert_eq!(
            resolved.invalidated_capabilities,
            vec![CAP_PROJECT_CONFIGURATION.to_string()]
        );
        assert!(resolved.granted_capabilities.contains(CAP_MCP_PROCESSES));
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(not(any(unix, windows)))]
    fn create_directory_symlink(_target: &Path, _link: &Path) -> bool {
        false
    }

    #[cfg(windows)]
    fn create_directory_junction(target: &Path, link: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|output| output.status.success())
    }
}
