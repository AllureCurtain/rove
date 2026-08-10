use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use figment::Figment;
use figment::providers::{Format, Toml};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use rove_runtime::agents::AgentDefinition;
use rove_runtime::agents::activation::MAX_WORKSPACE_PROCEDURE_ENTRIES;
use rove_runtime::agents::instructions::InstructionBundle;
use rove_runtime::agents::package::MAX_MANIFEST_BYTES;
use rove_runtime::context::prompt_metadata::stable_hash;
use rove_runtime::workspace::WorkspaceKind;
use rove_runtime::workspace::boundary::{
    is_symlink_or_reparse, resolve_workspace_read_path, resolve_workspace_read_path_without_links,
};

use crate::config::ProviderConfig;
use crate::provider::{ProviderAuthConfig, ProviderHeaderValue, SecretSource};

pub const TRUSTED_WORKSPACES_ENV: &str = "ROVE_TRUSTED_WORKSPACES";
pub const PROJECT_TRUST_STORE_ENV: &str = "ROVE_PROJECT_TRUST_STORE";
pub const PROJECT_TRUST_FILE_NAME: &str = "project-trust.sqlite";
pub const PROJECT_TRUST_LEGACY_FILE_NAME: &str = "project-trust.json";
pub const PROJECT_TRUST_SCHEMA_VERSION: u32 = 1;
pub const PROJECT_TRUST_INVALID_INPUT_CODE: &str = "project_trust_invalid_input";
pub const PROJECT_TRUST_UNAVAILABLE_CODE: &str = "project_trust_unavailable";
pub const PROJECT_TRUST_REQUIRED_CODE: &str = "project_trust_required";
const MAX_TRUST_INPUT_BYTES: usize = 256 * 1024;
/// Authority-source discovery is intentionally stricter than ordinary
/// workspace scanning. If these bounds are exceeded, the capability digest is
/// omitted and durable trust cannot activate workspace instructions/Agents.
const MAX_AUTHORITY_SOURCE_ENTRIES: usize = 16_384;
const MAX_AUTHORITY_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const UNAVAILABLE_CAPABILITY_DIGEST_PREFIX: &str = "unavailable:";

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
struct LegacyTrustFile {
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
        let effective_state = match resolution.state {
            ProjectActivationState::Unknown => ProjectActivationState::Restricted,
            state => state,
        };
        Self {
            state: effective_state,
            source: (effective_state == ProjectActivationState::Trusted)
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

/// Operator-owned durable trust repository. This SQLite database is the
/// canonical Project Trust authority shared by CLI, API, and bootstrap.
/// ProductStore v11 rows are imported once for compatibility, never used as a
/// second live authority.
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
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT canonical_root, workspace_kind, identity_digest, state,
                    capability_digests_json, granted_at, revoked_at, updated_at
             FROM project_trust_records ORDER BY canonical_root, workspace_kind",
        )?;
        let rows = statement.query_map([], project_trust_record_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(anyhow::Error::from)
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
        if decision == ProjectTrustDecision::Grant
            && let Some(capability) = capability_digests.iter().find_map(|(capability, digest)| {
                digest
                    .starts_with(UNAVAILABLE_CAPABILITY_DIGEST_PREFIX)
                    .then_some(capability)
            })
        {
            anyhow::bail!(
                "project trust capability '{capability}' cannot be granted because its source digest is unavailable"
            );
        }
        let canonical_root = canonical_directory(workspace_root)?;
        let identity_digest = workspace_identity_digest(&canonical_root, workspace_kind.clone());
        let canonical_root_text = canonical_root_key(&canonical_root);
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT canonical_root, workspace_kind, identity_digest, state,
                        capability_digests_json, granted_at, revoked_at, updated_at
                 FROM project_trust_records
                 WHERE canonical_root = ?1 AND workspace_kind = ?2",
                params![
                    &canonical_root_text,
                    workspace_kind_to_db(workspace_kind.clone())
                ],
                project_trust_record_from_row,
            )
            .optional()?;
        let now = now_rfc3339();
        let same_identity = existing
            .as_ref()
            .is_some_and(|record| record.identity_digest == identity_digest);
        let mut granted = if same_identity
            && existing
                .as_ref()
                .is_some_and(|record| record.state == ProjectActivationState::Trusted)
        {
            existing
                .as_ref()
                .map(|record| record.capability_digests.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let state = match decision {
            ProjectTrustDecision::Grant => {
                granted.extend(capability_digests);
                ProjectActivationState::Trusted
            }
            ProjectTrustDecision::Deny | ProjectTrustDecision::Revoke
                if !capability_digests.is_empty() =>
            {
                for capability in capability_digests.keys() {
                    granted.remove(capability);
                }
                if granted.is_empty() {
                    match decision {
                        ProjectTrustDecision::Deny => ProjectActivationState::Restricted,
                        ProjectTrustDecision::Revoke => ProjectActivationState::Revoked,
                        ProjectTrustDecision::Grant => unreachable!(),
                    }
                } else {
                    ProjectActivationState::Trusted
                }
            }
            ProjectTrustDecision::Deny => {
                granted.clear();
                ProjectActivationState::Restricted
            }
            ProjectTrustDecision::Revoke => {
                granted.clear();
                ProjectActivationState::Revoked
            }
        };
        let record = ProjectTrustRecord {
            canonical_root: canonical_root_text.clone(),
            workspace_kind: workspace_kind.clone(),
            identity_digest,
            state,
            capability_digests: granted,
            granted_at: if state == ProjectActivationState::Trusted {
                existing
                    .as_ref()
                    .and_then(|record| record.granted_at.clone())
                    .or_else(|| Some(now.clone()))
            } else {
                None
            },
            revoked_at: (state == ProjectActivationState::Revoked).then_some(now.clone()),
            updated_at: now,
        };
        let capability_digests = serde_json::to_string(&record.capability_digests)?;
        transaction.execute(
            "INSERT INTO project_trust_records(
                canonical_root, workspace_kind, identity_digest, state,
                capability_digests_json, granted_at, revoked_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(canonical_root, workspace_kind) DO UPDATE SET
                identity_digest = excluded.identity_digest,
                state = excluded.state,
                capability_digests_json = excluded.capability_digests_json,
                granted_at = excluded.granted_at,
                revoked_at = excluded.revoked_at,
                updated_at = excluded.updated_at",
            params![
                &canonical_root_text,
                workspace_kind_to_db(record.workspace_kind.clone()),
                &record.identity_digest,
                activation_state_to_db(record.state),
                &capability_digests,
                &record.granted_at,
                &record.revoked_at,
                &record.updated_at,
            ],
        )?;
        transaction.commit()?;
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

    /// Import v11 ProductStore rows without overwriting a canonical operator
    /// decision. This is intentionally one-way compatibility migration.
    pub fn import_product_store_snapshot(
        &self,
        product_store_path: &Path,
    ) -> anyhow::Result<usize> {
        if !product_store_path.exists() || product_store_path == self.path {
            return Ok(0);
        }
        let legacy = Connection::open(product_store_path)?;
        let table_exists = legacy
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'project_trust_records'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !table_exists {
            return Ok(0);
        }
        let mut statement = legacy.prepare(
            "SELECT canonical_root, workspace_kind, identity_digest, state,
                    capability_digests_json, granted_at, revoked_at, updated_at
             FROM project_trust_records",
        )?;
        let rows = statement.query_map([], project_trust_record_from_row)?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        let mut canonical = self.open()?;
        let transaction = canonical.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut imported = 0;
        for record in records {
            imported += insert_trust_record_if_missing(&transaction, &record)?;
        }
        transaction.commit()?;
        Ok(imported)
    }

    fn open(&self) -> anyhow::Result<Connection> {
        if self.path.exists() {
            migrate_legacy_json(&self.path, &self.path)?;
        } else if self
            .path
            .file_name()
            .is_some_and(|name| name == PROJECT_TRUST_FILE_NAME)
        {
            migrate_legacy_json(
                &self.path.with_file_name(PROJECT_TRUST_LEGACY_FILE_NAME),
                &self.path,
            )?;
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("project trust store has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        initialize_trust_schema(&connection)?;
        Ok(connection)
    }
}

fn initialize_trust_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_trust_records(
            canonical_root TEXT NOT NULL,
            workspace_kind TEXT NOT NULL CHECK(workspace_kind IN ('folder','repo','task')),
            identity_digest TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('unknown','restricted','trusted','revoked')),
            capability_digests_json TEXT NOT NULL,
            granted_at TEXT,
            revoked_at TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(canonical_root, workspace_kind)
        );
        CREATE INDEX IF NOT EXISTS idx_project_trust_state
            ON project_trust_records(state, updated_at DESC);",
    )
}

fn insert_trust_record_if_missing(
    connection: &Connection,
    record: &ProjectTrustRecord,
) -> anyhow::Result<usize> {
    Ok(connection.execute(
        "INSERT INTO project_trust_records(
            canonical_root, workspace_kind, identity_digest, state,
            capability_digests_json, granted_at, revoked_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(canonical_root, workspace_kind) DO NOTHING",
        params![
            &record.canonical_root,
            workspace_kind_to_db(record.workspace_kind.clone()),
            &record.identity_digest,
            activation_state_to_db(record.state),
            serde_json::to_string(&record.capability_digests)?,
            &record.granted_at,
            &record.revoked_at,
            &record.updated_at,
        ],
    )?)
}

fn project_trust_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectTrustRecord> {
    let workspace_kind = row.get::<_, String>(1)?;
    let state = row.get::<_, String>(3)?;
    Ok(ProjectTrustRecord {
        canonical_root: row.get(0)?,
        workspace_kind: workspace_kind_from_db(&workspace_kind)
            .map_err(|error| trust_row_conversion_error(1, error))?,
        identity_digest: row.get(2)?,
        state: activation_state_from_db(&state)
            .map_err(|error| trust_row_conversion_error(3, error))?,
        capability_digests: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        granted_at: row.get(5)?,
        revoked_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn trust_row_conversion_error(column: usize, error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn workspace_kind_to_db(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Folder => "folder",
        WorkspaceKind::Repo => "repo",
        WorkspaceKind::Task => "task",
    }
}

fn workspace_kind_from_db(value: &str) -> anyhow::Result<WorkspaceKind> {
    match value {
        "folder" => Ok(WorkspaceKind::Folder),
        "repo" => Ok(WorkspaceKind::Repo),
        "task" => Ok(WorkspaceKind::Task),
        _ => anyhow::bail!("invalid project trust workspace kind `{value}`"),
    }
}

fn activation_state_to_db(state: ProjectActivationState) -> &'static str {
    match state {
        ProjectActivationState::Unknown => "unknown",
        ProjectActivationState::Restricted => "restricted",
        ProjectActivationState::Trusted => "trusted",
        ProjectActivationState::Revoked => "revoked",
    }
}

fn activation_state_from_db(value: &str) -> anyhow::Result<ProjectActivationState> {
    match value {
        "unknown" => Ok(ProjectActivationState::Unknown),
        "restricted" => Ok(ProjectActivationState::Restricted),
        "trusted" => Ok(ProjectActivationState::Trusted),
        "revoked" => Ok(ProjectActivationState::Revoked),
        _ => anyhow::bail!("invalid project trust state `{value}`"),
    }
}

fn migrate_legacy_json(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(source)?;
    if bytes.len() > 512 * 1024 {
        anyhow::bail!("legacy project trust store exceeds the supported size");
    }
    if bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_none_or(|byte| *byte != b'{')
    {
        return Ok(());
    }
    let legacy: LegacyTrustFile = serde_json::from_slice(&bytes)?;
    if legacy.schema_version != PROJECT_TRUST_SCHEMA_VERSION {
        anyhow::bail!("unsupported legacy project trust schema version");
    }
    let backup = source.with_extension("json.legacy");
    if backup.exists() {
        anyhow::bail!(
            "legacy project trust backup already exists at {}",
            backup.display()
        );
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(source, &backup)?;
    let mut connection = Connection::open(destination)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    initialize_trust_schema(&connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for record in legacy.records {
        insert_trust_record_if_missing(&transaction, &record)?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn capability_digest_map(
    workspace_root: &Path,
    mcp_config: Option<&Path>,
    provider_selector: Option<&str>,
) -> BTreeMap<String, String> {
    let project_config = workspace_config_value(workspace_root);
    let mut result = BTreeMap::new();
    result.insert(
        CAP_PROJECT_CONFIGURATION.to_string(),
        digest_project_configuration(workspace_root, project_config.clone()),
    );
    result.insert(
        CAP_MCP_PROCESSES.to_string(),
        digest_mcp_configuration(workspace_root, mcp_config, project_config.as_ref()),
    );
    result.insert(
        CAP_PROVIDER_CREDENTIALS.to_string(),
        stable_hash(
            &provider_selector
                .map(str::to_string)
                .unwrap_or_else(|| provider_capability_selector_for_workspace(workspace_root)),
        ),
    );
    result.insert(
        CAP_WORKSPACE_INSTRUCTIONS.to_string(),
        digest_workspace_instruction_authority(workspace_root).unwrap_or_else(|| {
            format!("{UNAVAILABLE_CAPABILITY_DIGEST_PREFIX}workspace_instruction_authority")
        }),
    );
    result.insert(
        CAP_HOOKS_EXTENSIONS.to_string(),
        digest_hooks_extensions(project_config.as_ref()),
    );
    result.insert(
        CAP_EXTERNAL_PATHS.to_string(),
        digest_external_paths(project_config.as_ref()),
    );
    result
}

/// Bind Project Trust to every workspace-owned source that the Agent runtime
/// can admit as instructions or procedural guidance. This deliberately covers
/// the whole `agents/` tree (including currently unreferenced package files),
/// which is conservative: editing package documentation may require a new
/// confirmation, but editing ordinary application source does not.
///
/// `None` means the source set could not be enumerated completely within the
/// safety bounds. Callers omit the capability in that case, so an incomplete
/// digest can never become a durable grant.
fn digest_workspace_instruction_authority(workspace_root: &Path) -> Option<String> {
    let workspace_root = workspace_root.canonicalize().ok()?;
    let bundle = InstructionBundle::discover(&workspace_root).ok()?;
    let bundle_json = serde_json::to_string(&bundle).ok()?;
    let mut components =
        BTreeMap::from([("instruction_bundle".to_string(), stable_hash(&bundle_json))]);
    let mut budget = AuthorityDigestBudget::default();
    let agents_root = workspace_root.join("agents");

    match std::fs::symlink_metadata(&agents_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            components.insert("agents".to_string(), "missing".to_string());
        }
        Err(_) => return None,
        Ok(metadata) if !metadata.is_dir() || is_symlink_or_reparse(&metadata) => {
            components.insert("agents".to_string(), "refused".to_string());
        }
        Ok(_) => {
            let definitions =
                digest_agent_tree(&workspace_root, &agents_root, &mut components, &mut budget)?;
            digest_external_procedure_roots(
                &workspace_root,
                definitions,
                &mut components,
                &mut budget,
            )?;
        }
    }

    stable_hash(&serde_json::to_string(&components).ok()?).into()
}

#[derive(Default)]
struct AuthorityDigestBudget {
    entries: usize,
    bytes: usize,
}

fn digest_agent_tree(
    workspace_root: &Path,
    agents_root: &Path,
    components: &mut BTreeMap<String, String>,
    budget: &mut AuthorityDigestBudget,
) -> Option<Vec<AgentDefinition>> {
    let mut definitions = Vec::new();
    for entry in walkdir::WalkDir::new(agents_root)
        .follow_links(false)
        .sort_by_file_name()
    {
        budget.entries = budget.entries.checked_add(1)?;
        if budget.entries > MAX_AUTHORITY_SOURCE_ENTRIES {
            return None;
        }
        let entry = entry.ok()?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(path).ok()?;
        let relative = authority_relative_path(workspace_root, path)?;
        if is_symlink_or_reparse(&metadata) {
            components.insert(
                format!("agent_source:{relative}"),
                "linked_refused".to_string(),
            );
            continue;
        }
        if !metadata.is_file() {
            continue;
        }

        let bytes = read_authority_file(path, metadata.len(), budget)?;
        components.insert(
            format!("agent_source:{relative}"),
            authority_bytes_hash(&bytes, metadata.len()),
        );

        if is_direct_agent_manifest(&relative)
            && bytes.len() <= MAX_MANIFEST_BYTES
            && let Ok(text) = std::str::from_utf8(&bytes)
            && let Ok(definition) = toml::from_str::<AgentDefinition>(text)
        {
            definitions.push(definition);
        }
    }
    Some(definitions)
}

fn digest_external_procedure_roots(
    workspace_root: &Path,
    definitions: Vec<AgentDefinition>,
    components: &mut BTreeMap<String, String>,
    budget: &mut AuthorityDigestBudget,
) -> Option<()> {
    let roots = definitions
        .into_iter()
        .flat_map(|definition| definition.procedure_policy.roots)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<BTreeSet<_>>();

    for raw_root in roots {
        let component_prefix = format!("procedure_root:{raw_root}");
        let Ok(root) = resolve_workspace_read_path_without_links(workspace_root, &raw_root) else {
            components.insert(component_prefix, "unavailable".to_string());
            continue;
        };
        if !root.is_dir() {
            components.insert(component_prefix, "not_directory".to_string());
            continue;
        }

        let mut root_entries = 0usize;
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .max_depth(6)
            .sort_by_file_name()
        {
            root_entries += 1;
            if root_entries > MAX_WORKSPACE_PROCEDURE_ENTRIES {
                components.insert(
                    format!("{component_prefix}:limit"),
                    "entry_limit".to_string(),
                );
                break;
            }
            budget.entries = budget.entries.checked_add(1)?;
            if budget.entries > MAX_AUTHORITY_SOURCE_ENTRIES {
                return None;
            }
            let entry = entry.ok()?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(path).ok()?;
            let relative = authority_relative_path(workspace_root, path)?;
            if is_symlink_or_reparse(&metadata) {
                if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
                    components.insert(
                        format!("procedure_source:{relative}"),
                        "linked_refused".to_string(),
                    );
                }
                continue;
            }
            if !metadata.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("md")
            {
                continue;
            }
            let bytes = read_authority_file(path, metadata.len(), budget)?;
            components.insert(
                format!("procedure_source:{relative}"),
                authority_bytes_hash(&bytes, metadata.len()),
            );
        }
    }
    Some(())
}

fn read_authority_file(
    path: &Path,
    observed_len: u64,
    budget: &mut AuthorityDigestBudget,
) -> Option<Vec<u8>> {
    // Every Agent manifest, referenced prompt, and procedure is rejected by
    // Runtime above 64 KiB. Its content cannot become authority while it stays
    // oversized, so length is sufficient until it returns to the readable set.
    if observed_len > MAX_MANIFEST_BYTES as u64 {
        return Some(Vec::new());
    }
    let length = usize::try_from(observed_len).ok()?;
    budget.bytes = budget.bytes.checked_add(length)?;
    if budget.bytes > MAX_AUTHORITY_SOURCE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() == length).then_some(bytes)
}

fn authority_bytes_hash(bytes: &[u8], observed_len: u64) -> String {
    if observed_len > MAX_MANIFEST_BYTES as u64 {
        return format!("oversized:{observed_len}");
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn authority_relative_path(root: &Path, path: &Path) -> Option<String> {
    Some(
        path.strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

fn is_direct_agent_manifest(relative: &str) -> bool {
    let parts = relative.split('/').collect::<Vec<_>>();
    parts.len() == 3 && parts[0] == "agents" && parts[2] == "agent.toml"
}

/// Return a stable, redacted provider authority selector. Endpoint/profile and
/// credential source identifiers are included; literal secret values are only
/// represented by a one-way digest and are never persisted or logged.
pub fn provider_capability_selector_for_workspace(workspace_root: &Path) -> String {
    let config = bounded_workspace_text(workspace_root, ".rove/config.toml");
    let provider = config
        .as_deref()
        .and_then(|config| {
            Figment::new()
                .merge(Toml::string(config))
                .extract_inner::<ProviderConfig>("provider")
                .ok()
        })
        .unwrap_or_default();
    let active = provider.active.clone().unwrap_or_default();
    let profiles = provider
        .profiles
        .iter()
        .map(|(name, profile)| {
            format!(
                "{name}:{}:{}:{}:{}:{}:{}:{}",
                profile.provider_type,
                profile.base_url,
                profile.model,
                auth_selector(&profile.auth),
                profile
                    .headers
                    .iter()
                    .map(|(header, value)| format!("{header}={}", header_selector(value)))
                    .collect::<Vec<_>>()
                    .join(","),
                stable_hash(&serde_json::to_string(&profile.options).unwrap_or_default()),
                stable_hash(&serde_json::to_string(&profile.protocol_options).unwrap_or_default())
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let env_digest = digest_workspace_file(workspace_root, Some(&workspace_root.join(".env")))
        .unwrap_or_else(|| stable_hash("missing-project-env"));
    format!(
        "active={active};profiles={profiles};fallback={:?};model={};fallback_models={:?};options={};env={env_digest}",
        provider.fallback_profiles,
        provider.model,
        provider.fallback_models,
        stable_hash(&serde_json::to_string(&provider.options).unwrap_or_default()),
    )
}

fn auth_selector(auth: &ProviderAuthConfig) -> String {
    match auth {
        ProviderAuthConfig::None => "none".to_string(),
        ProviderAuthConfig::Bearer { secret } => format!("bearer:{}", secret_selector(secret)),
        ProviderAuthConfig::Header { header, secret } => {
            format!("header={header}:{}", secret_selector(secret))
        }
    }
}

fn header_selector(value: &ProviderHeaderValue) -> String {
    match value {
        ProviderHeaderValue::Literal(value) => format!("literal:{}", stable_hash(value)),
        ProviderHeaderValue::Env { env } => format!("env:{env}"),
        ProviderHeaderValue::File { file } => format!("file:{}", file.to_string_lossy()),
    }
}

fn secret_selector(secret: &SecretSource) -> String {
    match secret {
        SecretSource::Env { env } => format!("env:{env}"),
        SecretSource::File { file } => format!("file:{}", file.to_string_lossy()),
        SecretSource::Literal(value) => format!("literal:{}", stable_hash(value)),
    }
}

fn digest_project_configuration(
    workspace_root: &Path,
    mut project_config: Option<serde_json::Value>,
) -> String {
    if let Some(config) = project_config.as_mut() {
        remove_config_value(config, &["provider"]);
        remove_config_value(config, &["tool", "mcp_config_path"]);
        remove_config_value(config, &["hooks"]);
        remove_config_value(config, &["extensions"]);
        for path in EXTERNAL_PATH_CONFIG_KEYS {
            remove_config_value(config, path);
        }
    }
    let config = project_config
        .as_ref()
        .and_then(|config| serde_json::to_string(config).ok())
        .map(|config| stable_hash(&config))
        .unwrap_or_else(|| stable_hash("missing-or-invalid-project-config"));
    let env = digest_workspace_file(workspace_root, Some(&workspace_root.join(".env")))
        .unwrap_or_else(|| stable_hash("missing-project-env"));
    stable_hash(&format!("config={config};env={env}"))
}

const EXTERNAL_PATH_CONFIG_KEYS: &[&[&str]] = &[
    &["runtime", "system_prompt_path"],
    &["runtime", "planner_prompt_path"],
    &["memory", "session_dir"],
    &["memory", "durable_dir"],
    &["state", "state_dir"],
    &["state", "sqlite_path"],
    &["state", "allow_external_paths"],
];

fn workspace_config_value(workspace_root: &Path) -> Option<serde_json::Value> {
    let config = bounded_workspace_text(workspace_root, ".rove/config.toml")?;
    Figment::new()
        .merge(Toml::string(&config))
        .extract::<serde_json::Value>()
        .ok()
}

fn digest_mcp_configuration(
    workspace_root: &Path,
    explicit_path: Option<&Path>,
    project_config: Option<&serde_json::Value>,
) -> String {
    let configured = project_config
        .and_then(|config| config.pointer("/tool/mcp_config_path"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(".rove/mcp_servers.json");
    let path = explicit_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace_root.join(configured));
    let content = digest_workspace_file(workspace_root, Some(&path))
        .unwrap_or_else(|| stable_hash("missing-or-unreadable-mcp-config"));
    stable_hash(&format!(
        "path={};content={content}",
        stable_hash(configured)
    ))
}

fn digest_external_paths(project_config: Option<&serde_json::Value>) -> String {
    let selected = EXTERNAL_PATH_CONFIG_KEYS
        .iter()
        .filter_map(|path| {
            let pointer = format!("/{}", path.join("/"));
            project_config
                .and_then(|config| config.pointer(&pointer))
                .map(|value| (pointer, value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    stable_hash(&serde_json::to_string(&selected).unwrap_or_default())
}

fn digest_hooks_extensions(project_config: Option<&serde_json::Value>) -> String {
    let selected = ["hooks", "extensions"]
        .into_iter()
        .filter_map(|key| {
            project_config
                .and_then(|config| config.get(key))
                .map(|value| (key, value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    stable_hash(&serde_json::to_string(&selected).unwrap_or_default())
}

fn remove_config_value(value: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = value;
    for parent in parents {
        let Some(next) = current.get_mut(*parent) else {
            return;
        };
        current = next;
    }
    if let Some(object) = current.as_object_mut() {
        object.remove(*last);
    }
}

fn bounded_workspace_text(workspace_root: &Path, relative_path: &str) -> Option<String> {
    let path = resolve_workspace_read_path(workspace_root, relative_path).ok()?;
    let mut bytes = Vec::with_capacity(8 * 1024);
    std::fs::File::open(path)
        .ok()?
        .take((MAX_TRUST_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_TRUST_INPUT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
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
            state: ProjectActivationState::Unknown,
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
            if digest.starts_with(UNAVAILABLE_CAPABILITY_DIGEST_PREFIX) {
                if record.capability_digests.contains_key(capability) {
                    invalidated.push(capability.clone());
                }
                continue;
            }
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

    fn workspace_instruction_digest(root: &Path) -> BTreeMap<String, String> {
        let all = capability_digest_map(root, None, None);
        BTreeMap::from([(
            CAP_WORKSPACE_INSTRUCTIONS.to_string(),
            all.get(CAP_WORKSPACE_INSTRUCTIONS)
                .expect("workspace instruction capability remains discoverable")
                .clone(),
        )])
    }

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
    fn workspace_instruction_grant_tracks_nested_rules_and_agent_sources_only() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.sqlite"));
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join("apps/web")).unwrap();
        std::fs::create_dir_all(root.join("agents/ops/procedures")).unwrap();
        std::fs::create_dir_all(root.join("runbooks")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("AGENTS.md"), "Root rule.\n").unwrap();
        std::fs::write(root.join("apps/web/AGENTS.md"), "Web rule.\n").unwrap();
        std::fs::write(
            root.join("agents/ops/agent.toml"),
            r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Ops"
default_instructions_path = "instructions.md"

[procedure_policy]
roots = ["runbooks"]
max_selected = 2
"#,
        )
        .unwrap();
        std::fs::write(root.join("agents/ops/instructions.md"), "Inspect first.\n").unwrap();
        std::fs::write(
            root.join("agents/ops/procedures/local.md"),
            "local procedure source\n",
        )
        .unwrap();
        std::fs::write(root.join("runbooks/disk.md"), "external procedure source\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let grant = workspace_instruction_digest(&root);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                grant,
            )
            .unwrap();

        std::fs::write(
            root.join("src/main.rs"),
            "fn main() { println!(\"ok\"); }\n",
        )
        .unwrap();
        let ordinary_change = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert!(
            ordinary_change
                .granted_capabilities
                .contains(CAP_WORKSPACE_INSTRUCTIONS)
        );

        for (path, replacement) in [
            ("apps/web/AGENTS.md", "Changed web rule.\n"),
            (
                "agents/ops/instructions.md",
                "Changed Agent instructions.\n",
            ),
            (
                "agents/ops/procedures/local.md",
                "changed local procedure source\n",
            ),
            ("runbooks/disk.md", "changed external procedure source\n"),
        ] {
            store
                .decide(
                    &root,
                    WorkspaceKind::Folder,
                    ProjectTrustDecision::Grant,
                    workspace_instruction_digest(&root),
                )
                .unwrap();
            std::fs::write(root.join(path), replacement).unwrap();
            let changed = store
                .resolve(
                    &root,
                    WorkspaceKind::Folder,
                    &capability_digest_map(&root, None, None),
                )
                .unwrap();
            assert_eq!(
                changed.invalidated_capabilities,
                vec![CAP_WORKSPACE_INSTRUCTIONS.to_string()],
                "{path} must be bound to the workspace instruction grant"
            );
            assert!(
                !changed
                    .granted_capabilities
                    .contains(CAP_WORKSPACE_INSTRUCTIONS)
            );
        }
    }

    #[test]
    fn unavailable_authority_digest_cannot_be_granted_and_invalidates_an_old_grant() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.sqlite"));
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let stable = workspace_instruction_digest(&root);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                stable,
            )
            .unwrap();

        let unavailable = BTreeMap::from([(
            CAP_WORKSPACE_INSTRUCTIONS.to_string(),
            format!("{UNAVAILABLE_CAPABILITY_DIGEST_PREFIX}test"),
        )]);
        let resolved = store
            .resolve(&root, WorkspaceKind::Folder, &unavailable)
            .unwrap();
        assert_eq!(
            resolved.invalidated_capabilities,
            vec![CAP_WORKSPACE_INSTRUCTIONS.to_string()]
        );
        assert!(resolved.granted_capabilities.is_empty());

        let error = store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                unavailable,
            )
            .unwrap_err();
        assert!(error.to_string().contains("source digest is unavailable"));
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
        assert_eq!(resolved.state, ProjectActivationState::Unknown);
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
        assert_eq!(resolved.state, ProjectActivationState::Unknown);
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

    #[test]
    fn provider_endpoint_and_credential_selector_invalidate_only_provider_capability() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.sqlite"));
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join(".rove")).unwrap();
        let config = root.join(".rove/config.toml");
        write_provider_config(&config, "https://one.example.test/v1", "FIRST_API_KEY");
        let initial = capability_digest_map(&root, None, None);
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                initial,
            )
            .unwrap();

        write_provider_config(&config, "https://two.example.test/v1", "FIRST_API_KEY");
        let endpoint_changed = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(
            endpoint_changed.invalidated_capabilities,
            vec![CAP_PROVIDER_CREDENTIALS.to_string()]
        );
        assert!(
            endpoint_changed
                .granted_capabilities
                .contains(CAP_PROJECT_CONFIGURATION)
        );
        assert!(
            endpoint_changed
                .granted_capabilities
                .contains(CAP_MCP_PROCESSES)
        );

        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                capability_digest_map(&root, None, None),
            )
            .unwrap();
        write_provider_config(&config, "https://two.example.test/v1", "SECOND_API_KEY");
        let credential_changed = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(
            credential_changed.invalidated_capabilities,
            vec![CAP_PROVIDER_CREDENTIALS.to_string()]
        );
        assert!(
            credential_changed
                .granted_capabilities
                .contains(CAP_WORKSPACE_INSTRUCTIONS)
        );
    }

    #[test]
    fn workspace_env_change_invalidates_project_and_provider_but_not_other_capabilities() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.sqlite"));
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join(".rove")).unwrap();
        write_provider_config(
            &root.join(".rove/config.toml"),
            "https://api.example.test/v1",
            "PROJECT_API_KEY",
        );
        std::fs::write(root.join(".env"), "PROJECT_API_KEY=first\n").unwrap();
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                capability_digest_map(&root, None, None),
            )
            .unwrap();

        std::fs::write(root.join(".env"), "PROJECT_API_KEY=second\n").unwrap();
        let changed = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(
            changed.invalidated_capabilities,
            vec![
                CAP_PROJECT_CONFIGURATION.to_string(),
                CAP_PROVIDER_CREDENTIALS.to_string(),
            ]
        );
        assert!(changed.granted_capabilities.contains(CAP_MCP_PROCESSES));
        assert!(
            changed
                .granted_capabilities
                .contains(CAP_WORKSPACE_INSTRUCTIONS)
        );
        assert!(changed.granted_capabilities.contains(CAP_EXTERNAL_PATHS));
    }

    #[test]
    fn hook_and_external_path_selectors_have_independent_capability_digests() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ProjectTrustRepository::new(temp.path().join("operator/trust.sqlite"));
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join(".rove")).unwrap();
        let config = root.join(".rove/config.toml");
        std::fs::write(
            &config,
            "[runtime]\nmax_steps = 9\nsystem_prompt_path = \"first.md\"\n[hooks]\ncommand = \"first-hook\"\n",
        )
        .unwrap();
        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                capability_digest_map(&root, None, None),
            )
            .unwrap();

        std::fs::write(
            &config,
            "[runtime]\nmax_steps = 9\nsystem_prompt_path = \"second.md\"\n[hooks]\ncommand = \"first-hook\"\n",
        )
        .unwrap();
        let external_changed = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(
            external_changed.invalidated_capabilities,
            vec![CAP_EXTERNAL_PATHS.to_string()]
        );

        store
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Grant,
                capability_digest_map(&root, None, None),
            )
            .unwrap();
        std::fs::write(
            &config,
            "[runtime]\nmax_steps = 9\nsystem_prompt_path = \"second.md\"\n[hooks]\ncommand = \"second-hook\"\n",
        )
        .unwrap();
        let hook_changed = store
            .resolve(
                &root,
                WorkspaceKind::Folder,
                &capability_digest_map(&root, None, None),
            )
            .unwrap();
        assert_eq!(
            hook_changed.invalidated_capabilities,
            vec![CAP_HOOKS_EXTENSIONS.to_string()]
        );
        assert!(
            hook_changed
                .granted_capabilities
                .contains(CAP_PROJECT_CONFIGURATION)
        );
    }

    #[test]
    fn legacy_json_migrates_once_and_keeps_a_rollback_backup() {
        let temp = tempfile::TempDir::new().unwrap();
        let operator = temp.path().join("operator");
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(&operator).unwrap();
        std::fs::create_dir(&root).unwrap();
        let canonical_root = root.canonicalize().unwrap();
        let digests = capability_digest_map(&root, None, None);
        let record = ProjectTrustRecord {
            canonical_root: canonical_root_key(&canonical_root),
            workspace_kind: WorkspaceKind::Folder,
            identity_digest: workspace_identity_digest(&canonical_root, WorkspaceKind::Folder),
            state: ProjectActivationState::Trusted,
            capability_digests: digests.clone(),
            granted_at: Some("legacy-grant".to_string()),
            revoked_at: None,
            updated_at: "legacy-update".to_string(),
        };
        let legacy_path = operator.join(PROJECT_TRUST_LEGACY_FILE_NAME);
        std::fs::write(
            &legacy_path,
            serde_json::to_vec(&LegacyTrustFile {
                schema_version: PROJECT_TRUST_SCHEMA_VERSION,
                records: vec![record],
            })
            .unwrap(),
        )
        .unwrap();
        let repository = ProjectTrustRepository::new(operator.join(PROJECT_TRUST_FILE_NAME));

        let resolved = repository
            .resolve(&root, WorkspaceKind::Folder, &digests)
            .unwrap();

        assert_eq!(resolved.state, ProjectActivationState::Trusted);
        assert!(!legacy_path.exists());
        assert!(legacy_path.with_extension("json.legacy").exists());
        assert!(repository.path().exists());
    }

    #[test]
    fn product_store_snapshot_is_import_only_and_cannot_overwrite_canonical_decisions() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let digests = capability_digest_map(&root, None, None);
        let legacy_path = temp.path().join("product.sqlite");
        let legacy = Connection::open(&legacy_path).unwrap();
        initialize_trust_schema(&legacy).unwrap();
        let canonical_root = root.canonicalize().unwrap();
        let legacy_record = ProjectTrustRecord {
            canonical_root: canonical_root_key(&canonical_root),
            workspace_kind: WorkspaceKind::Folder,
            identity_digest: workspace_identity_digest(&canonical_root, WorkspaceKind::Folder),
            state: ProjectActivationState::Trusted,
            capability_digests: digests.clone(),
            granted_at: Some("legacy-grant".to_string()),
            revoked_at: None,
            updated_at: "legacy-update".to_string(),
        };
        assert_eq!(
            insert_trust_record_if_missing(&legacy, &legacy_record).unwrap(),
            1
        );
        drop(legacy);
        let repository = ProjectTrustRepository::new(temp.path().join("trust.sqlite"));

        assert_eq!(
            repository
                .import_product_store_snapshot(&legacy_path)
                .unwrap(),
            1
        );
        assert_eq!(
            repository
                .resolve(&root, WorkspaceKind::Folder, &digests)
                .unwrap()
                .state,
            ProjectActivationState::Trusted
        );
        repository
            .decide(
                &root,
                WorkspaceKind::Folder,
                ProjectTrustDecision::Deny,
                BTreeMap::new(),
            )
            .unwrap();
        assert_eq!(
            repository
                .import_product_store_snapshot(&legacy_path)
                .unwrap(),
            0
        );
        assert_eq!(
            repository
                .resolve(&root, WorkspaceKind::Folder, &digests)
                .unwrap()
                .state,
            ProjectActivationState::Restricted
        );
    }

    fn write_provider_config(path: &Path, endpoint: &str, api_key_env: &str) {
        std::fs::write(
            path,
            format!(
                r#"[runtime]
max_steps = 9

[provider]
active = "default"

[provider.profiles.default]
provider_type = "openai"
base_url = "{endpoint}"
model = "test-model"
auth = {{ style = "bearer", secret = {{ env = "{api_key_env}" }} }}
"#
            ),
        )
        .unwrap();
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
