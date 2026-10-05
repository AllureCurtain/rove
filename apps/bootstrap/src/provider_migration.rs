//! Explicit legacy Provider migration into the user-owned catalog.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::ProviderConfig;
use crate::provider::{ProviderAuthConfig, ProviderProfileConfig, SecretSource};
use crate::provider_catalog::{ProviderCatalogError, ProviderCatalogService, ProviderProfileId};
use crate::user_config::UserConfigDocument;

pub const PROVIDER_MIGRATION_RECEIPT_SCHEMA_VERSION: u16 = 1;
const MAX_LEGACY_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_PRODUCT_STORE_BYTES: u64 = 1024 * 1024 * 1024;
const ENV_PROFILES: &str = "ROVE_PROVIDER_PROFILES";
const ENV_ACTIVE: &str = "ROVE_PROVIDER_ACTIVE";
const ENV_MODEL: &str = "ROVE_MODEL";

#[derive(Debug, Clone)]
pub struct ProviderMigrationOptions {
    pub workspace_root: PathBuf,
    pub trusted_workspace: bool,
    pub product_store_path: Option<PathBuf>,
    /// Keys use `SOURCE:PROFILE`, for example `workspace:team`.
    pub renames: BTreeMap<String, String>,
    pub apply: bool,
    pub rewrite_workspace_config: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMigrationSource {
    Workspace,
    Environment,
    ProductStore,
}

impl ProviderMigrationSource {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "workspace" => Some(Self::Workspace),
            "environment" | "env" => Some(Self::Environment),
            "product_store" | "product-store" => Some(Self::ProductStore),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Environment => "environment",
            Self::ProductStore => "product_store",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMigrationOutcome {
    Import,
    Rename,
    Merge,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMigrationAction {
    pub source: ProviderMigrationSource,
    pub source_profile_id: String,
    pub catalog_profile_id: String,
    pub outcome: ProviderMigrationOutcome,
    pub safe_identity_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMigrationConflict {
    pub source: ProviderMigrationSource,
    pub source_profile_id: String,
    pub requested_catalog_profile_id: String,
    pub existing_safe_identity_digest: String,
    pub incoming_safe_identity_digest: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMigrationReport {
    pub schema_version: u16,
    pub applied: bool,
    pub catalog_revision_before: String,
    pub catalog_revision_after: String,
    pub source_digests: BTreeMap<String, String>,
    pub actions: Vec<ProviderMigrationAction>,
    pub conflicts: Vec<ProviderMigrationConflict>,
    pub workspace_rewritten: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderMigrationError {
    #[error("provider_migration_invalid_source: {0}")]
    InvalidSource(String),
    #[error("provider_migration_conflict: unresolved Provider profile conflicts")]
    Conflict,
    #[error("provider_migration_revision_conflict: the user catalog changed during migration")]
    RevisionConflict,
    #[error(
        "provider_migration_workspace_confirmation_required: workspace rewrite requires trusted workspace confirmation"
    )]
    WorkspaceConfirmationRequired,
    #[error("provider_migration_io: {0}")]
    Io(String),
    #[error("provider_migration_storage: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
struct LegacyProfile {
    source: ProviderMigrationSource,
    source_id: String,
    profile: ProviderProfileConfig,
    safe_digest: String,
}

#[derive(Debug, Default)]
struct LegacySource {
    profiles: Vec<LegacyProfile>,
    selection: Option<LegacySelection>,
    digest: Option<String>,
    raw_digest: Option<String>,
}

#[derive(Debug, Clone)]
struct LegacySelection {
    profile_id: String,
    model: Option<String>,
}

pub fn run_provider_migration(
    catalog_service: &ProviderCatalogService,
    options: ProviderMigrationOptions,
) -> Result<ProviderMigrationReport, ProviderMigrationError> {
    if options.rewrite_workspace_config && (!options.apply || !options.trusted_workspace) {
        return Err(ProviderMigrationError::WorkspaceConfirmationRequired);
    }
    let workspace_root = canonical_workspace(&options.workspace_root)?;
    let workspace_path = workspace_root.join(".rove/config.toml");
    let workspace =
        read_workspace_source(&workspace_root, &workspace_path, options.trusted_workspace)?;
    let environment = read_environment_source(&workspace_root)?;
    let product_store = options
        .product_store_path
        .as_deref()
        .map(read_product_store_source)
        .transpose()?
        .unwrap_or_default();

    let catalog = catalog_service.load().map_err(map_catalog_error)?;
    let before = catalog.revision().to_string();
    let mut document = catalog.document().clone();
    let mut safe_identities = document
        .provider
        .profiles
        .iter()
        .map(|(id, profile)| {
            safe_profile_digest(profile, &workspace_root).map(|digest| (id.clone(), digest))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let original_ids = document
        .provider
        .profiles
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut actions = Vec::new();
    let mut conflicts = Vec::new();
    let mut mappings = BTreeMap::new();
    let sources = [&workspace, &environment, &product_store];

    for source in sources {
        for incoming in &source.profiles {
            let source_key = format!("{}:{}", incoming.source.as_str(), incoming.source_id);
            let requested = options
                .renames
                .get(&source_key)
                .cloned()
                .unwrap_or_else(|| incoming.source_id.clone());
            ProviderProfileId::new(requested.clone()).map_err(map_catalog_error)?;

            if let Some((same_id, _)) = safe_identities
                .iter()
                .find(|(_, digest)| **digest == incoming.safe_digest)
            {
                let catalog_id = same_id.clone();
                mappings.insert(source_key, catalog_id.clone());
                actions.push(ProviderMigrationAction {
                    source: incoming.source,
                    source_profile_id: incoming.source_id.clone(),
                    catalog_profile_id: catalog_id,
                    outcome: ProviderMigrationOutcome::Merge,
                    safe_identity_digest: incoming.safe_digest.clone(),
                });
                continue;
            }

            if let Some(existing) = safe_identities.get(&requested) {
                conflicts.push(ProviderMigrationConflict {
                    source: incoming.source,
                    source_profile_id: incoming.source_id.clone(),
                    requested_catalog_profile_id: requested,
                    existing_safe_identity_digest: existing.clone(),
                    incoming_safe_identity_digest: incoming.safe_digest.clone(),
                    reason: "profile id names a different Provider identity; choose a new id"
                        .to_string(),
                });
                continue;
            }

            document
                .provider
                .profiles
                .insert(requested.clone(), incoming.profile.clone());
            safe_identities.insert(requested.clone(), incoming.safe_digest.clone());
            mappings.insert(source_key, requested.clone());
            actions.push(ProviderMigrationAction {
                source: incoming.source,
                source_profile_id: incoming.source_id.clone(),
                catalog_profile_id: requested.clone(),
                outcome: if requested == incoming.source_id {
                    ProviderMigrationOutcome::Import
                } else {
                    ProviderMigrationOutcome::Rename
                },
                safe_identity_digest: incoming.safe_digest.clone(),
            });
        }
    }

    apply_default_selection(
        &mut document,
        [&workspace, &environment, &product_store],
        &mappings,
    );
    document
        .validate()
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    let planned_revision = document.revision();
    let mut source_digests = BTreeMap::new();
    for (source, loaded) in [
        (ProviderMigrationSource::Workspace, &workspace),
        (ProviderMigrationSource::Environment, &environment),
        (ProviderMigrationSource::ProductStore, &product_store),
    ] {
        if let Some(digest) = &loaded.digest {
            source_digests.insert(source.as_str().to_string(), digest.clone());
        }
    }

    if !options.apply || !conflicts.is_empty() {
        return Ok(ProviderMigrationReport {
            schema_version: PROVIDER_MIGRATION_RECEIPT_SCHEMA_VERSION,
            applied: false,
            catalog_revision_before: before,
            catalog_revision_after: planned_revision,
            source_digests,
            actions,
            conflicts,
            workspace_rewritten: false,
            receipt_path: None,
        });
    }

    let changed = document
        .provider
        .profiles
        .keys()
        .any(|id| !original_ids.contains(id))
        || planned_revision != before;
    let after = if changed {
        catalog_service
            .replace(&before, &document)
            .map_err(map_catalog_error)?
            .revision()
            .to_string()
    } else {
        before.clone()
    };
    if let Some(path) = options.product_store_path.as_deref() {
        apply_product_store_mappings(path, &actions, &after)?;
    }
    let workspace_rewritten = if options.rewrite_workspace_config && workspace.digest.is_some() {
        rewrite_workspace_selection(&workspace_path, &workspace, &mappings)?;
        true
    } else {
        false
    };
    let mut report = ProviderMigrationReport {
        schema_version: PROVIDER_MIGRATION_RECEIPT_SCHEMA_VERSION,
        applied: true,
        catalog_revision_before: before,
        catalog_revision_after: after,
        source_digests,
        actions,
        conflicts,
        workspace_rewritten,
        receipt_path: None,
    };
    let receipt_path = write_receipt(catalog_service, &report)?;
    report.receipt_path = Some(receipt_path.display().to_string());
    Ok(report)
}

fn canonical_workspace(path: &Path) -> Result<PathBuf, ProviderMigrationError> {
    path.canonicalize()
        .map_err(|_| ProviderMigrationError::InvalidSource("workspace is unavailable".to_string()))
}

fn read_workspace_source(
    workspace_root: &Path,
    path: &Path,
    trusted: bool,
) -> Result<LegacySource, ProviderMigrationError> {
    if !trusted || !path.exists() {
        return Ok(LegacySource::default());
    }
    reject_symlink(path, "workspace Provider configuration")?;
    let canonical = path.canonicalize().map_err(|_| {
        ProviderMigrationError::InvalidSource("workspace config is unavailable".to_string())
    })?;
    if !canonical.starts_with(workspace_root) {
        return Err(ProviderMigrationError::InvalidSource(
            "workspace config resolves outside the workspace".to_string(),
        ));
    }
    let bytes = read_bounded(
        path,
        MAX_LEGACY_CONFIG_BYTES,
        "workspace Provider configuration",
    )?;
    let value: toml::Value = toml::from_str(&String::from_utf8_lossy(&bytes))
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    let provider: ProviderConfig = value
        .get("provider")
        .cloned()
        .map(toml::Value::try_into)
        .transpose()
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?
        .unwrap_or_default();
    let mut source = legacy_source(
        ProviderMigrationSource::Workspace,
        provider.profiles,
        provider.active,
        (!provider.model.trim().is_empty()).then_some(provider.model),
        workspace_root,
    )?;
    source.raw_digest = Some(hash_bytes(&bytes));
    Ok(source)
}

fn read_environment_source(workspace_root: &Path) -> Result<LegacySource, ProviderMigrationError> {
    let Some(raw) = std::env::var_os(ENV_PROFILES) else {
        return Ok(LegacySource::default());
    };
    let raw = raw.to_string_lossy();
    if raw.len() > MAX_LEGACY_CONFIG_BYTES as usize {
        return Err(ProviderMigrationError::InvalidSource(
            "environment Provider profiles exceed the size limit".to_string(),
        ));
    }
    let profiles: BTreeMap<String, ProviderProfileConfig> = serde_json::from_str(&raw)
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    let active = std::env::var(ENV_ACTIVE)
        .ok()
        .filter(|value| !value.is_empty());
    let model = std::env::var(ENV_MODEL)
        .ok()
        .filter(|value| !value.is_empty());
    legacy_source(
        ProviderMigrationSource::Environment,
        profiles,
        active,
        model,
        workspace_root,
    )
}

fn read_product_store_source(path: &Path) -> Result<LegacySource, ProviderMigrationError> {
    if !path.exists() {
        return Ok(LegacySource::default());
    }
    reject_symlink(path, "ProductStore")?;
    let metadata = fs::metadata(path)
        .map_err(|_| ProviderMigrationError::Storage("ProductStore metadata failed".to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_PRODUCT_STORE_BYTES {
        return Err(ProviderMigrationError::Storage(
            "ProductStore is not a bounded regular file".to_string(),
        ));
    }
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(storage_error)?;
    if !table_exists(&connection, "product_provider_profiles")? {
        return Ok(LegacySource::default());
    }
    let required = [
        "profile_id",
        "label",
        "provider_type",
        "api_base",
        "api_key_env",
        "default_model",
    ];
    let columns = table_columns(&connection, "product_provider_profiles")?;
    if required.iter().any(|name| !columns.contains(*name)) {
        return Err(ProviderMigrationError::Storage(
            "ProductStore Provider schema is incomplete".to_string(),
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT profile_id, label, provider_type, api_base, api_key_env, default_model FROM product_provider_profiles ORDER BY profile_id LIMIT 129",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(storage_error)?;
    let mut profiles = BTreeMap::new();
    for row in rows {
        let (id, label, provider_type, base_url, api_key_env, model) =
            row.map_err(storage_error)?;
        if profiles.len() >= 128 {
            return Err(ProviderMigrationError::Storage(
                "ProductStore Provider profile limit exceeded".to_string(),
            ));
        }
        let auth = match (provider_type.as_str(), api_key_env) {
            ("anthropic", Some(env)) => ProviderAuthConfig::Header {
                header: "x-api-key".to_string(),
                secret: SecretSource::Env { env },
            },
            (_, Some(env)) => ProviderAuthConfig::Bearer {
                secret: SecretSource::Env { env },
            },
            (_, None) => ProviderAuthConfig::None,
        };
        profiles.insert(
            id,
            ProviderProfileConfig {
                label: Some(label),
                provider_type,
                base_url,
                model: model.unwrap_or_else(|| "default".to_string()),
                auth,
                headers: BTreeMap::new(),
                options: Default::default(),
                protocol_options: serde_json::json!({}),
            },
        );
    }
    let selection = read_product_store_selection(&connection)?;
    let mut source = legacy_source(
        ProviderMigrationSource::ProductStore,
        profiles,
        selection.as_ref().map(|value| value.profile_id.clone()),
        selection.and_then(|value| value.model),
        Path::new("."),
    )?;
    // Include current session selections and immutable run mappings in the
    // source digest without changing historical snapshot rows.
    let mapping_digest = product_store_mapping_digest(&connection)?;
    source.digest = Some(hash_json(&serde_json::json!({
        "profiles": source.digest,
        "session_and_run_mappings": mapping_digest,
    }))?);
    Ok(source)
}

fn legacy_source(
    source: ProviderMigrationSource,
    profiles: BTreeMap<String, ProviderProfileConfig>,
    active: Option<String>,
    model: Option<String>,
    workspace_root: &Path,
) -> Result<LegacySource, ProviderMigrationError> {
    let mut loaded = Vec::with_capacity(profiles.len());
    for (id, profile) in profiles {
        ProviderProfileId::new(id.clone()).map_err(map_catalog_error)?;
        profile
            .validate(workspace_root, true)
            .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
        let safe_digest = safe_profile_digest(&profile, workspace_root)?;
        loaded.push(LegacyProfile {
            source,
            source_id: id,
            profile,
            safe_digest,
        });
    }
    let digest = (!loaded.is_empty()).then(|| {
        hash_json(
            &loaded
                .iter()
                .map(|profile| (&profile.source_id, &profile.safe_digest))
                .collect::<Vec<_>>(),
        )
    });
    Ok(LegacySource {
        profiles: loaded,
        selection: active.map(|profile_id| LegacySelection { profile_id, model }),
        digest: digest.transpose()?,
        raw_digest: None,
    })
}

fn safe_profile_digest(
    profile: &ProviderProfileConfig,
    workspace_root: &Path,
) -> Result<String, ProviderMigrationError> {
    profile
        .validate(workspace_root, true)
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    hash_json(profile)
}

fn hash_json(value: &impl Serialize) -> Result<String, ProviderMigrationError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    let mut digest = Sha256::new();
    digest.update(encoded);
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn apply_default_selection<'a>(
    document: &mut UserConfigDocument,
    sources: impl IntoIterator<Item = &'a LegacySource>,
    mappings: &BTreeMap<String, String>,
) {
    if document.model.default_profile.is_some() {
        return;
    }
    for source in sources {
        let Some(selection) = &source.selection else {
            continue;
        };
        let key = format!(
            "{}:{}",
            source
                .profiles
                .first()
                .map(|profile| profile.source.as_str())
                .unwrap_or_default(),
            selection.profile_id
        );
        if let Some(profile_id) = mappings.get(&key) {
            document.model.default_profile = Some(profile_id.clone());
            document.model.default_model = selection.model.clone().or_else(|| {
                document
                    .provider
                    .profiles
                    .get(profile_id)
                    .map(|profile| profile.model.clone())
            });
            break;
        }
    }
}

fn read_product_store_selection(
    connection: &Connection,
) -> Result<Option<LegacySelection>, ProviderMigrationError> {
    if table_exists(connection, "product_preferences")? {
        let columns = table_columns(connection, "product_preferences")?;
        if columns.contains("provider_profile_id") && columns.contains("provider_model") {
            let selection = connection
                .query_row(
                    "SELECT provider_profile_id, provider_model FROM product_preferences WHERE singleton = 1",
                    [],
                    |row| {
                        Ok(LegacySelection {
                            profile_id: row.get(0)?,
                            model: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(storage_error)?;
            if selection.is_some() {
                return Ok(selection);
            }
        }
    }
    if table_exists(connection, "product_session_model_configs")? {
        let columns = table_columns(connection, "product_session_model_configs")?;
        if columns.contains("profile_id") && columns.contains("model") {
            return connection
                .query_row(
                    "SELECT profile_id, model FROM product_session_model_configs WHERE profile_id IS NOT NULL ORDER BY updated_at DESC LIMIT 1",
                    [],
                    |row| {
                        Ok(LegacySelection {
                            profile_id: row.get(0)?,
                            model: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(storage_error);
        }
    }
    Ok(None)
}

fn product_store_mapping_digest(connection: &Connection) -> Result<String, ProviderMigrationError> {
    let mut rows = Vec::new();
    for (table, id_column) in [
        ("product_preferences", "singleton"),
        ("product_session_model_configs", "product_session_id"),
        ("product_session_run_models", "runtime_run_id"),
    ] {
        if !table_exists(connection, table)? {
            continue;
        }
        let columns = table_columns(connection, table)?;
        if !columns.contains(id_column) || !columns.contains("profile_id") {
            continue;
        }
        let sql = format!(
            "SELECT CAST({id_column} AS TEXT), profile_id FROM {table} WHERE profile_id IS NOT NULL ORDER BY {id_column} LIMIT 4097"
        );
        let mut statement = connection.prepare(&sql).map_err(storage_error)?;
        let values = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage_error)?;
        for value in values {
            if rows.len() >= 4096 {
                return Err(ProviderMigrationError::Storage(
                    "ProductStore Provider mapping limit exceeded".to_string(),
                ));
            }
            let (id, profile_id) = value.map_err(storage_error)?;
            rows.push((table.to_string(), id, profile_id));
        }
    }
    hash_json(&rows)
}

fn apply_product_store_mappings(
    path: &Path,
    actions: &[ProviderMigrationAction],
    catalog_revision: &str,
) -> Result<(), ProviderMigrationError> {
    if !path.exists() {
        return Ok(());
    }
    reject_symlink(path, "ProductStore")?;
    let mut connection = Connection::open(path).map_err(storage_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(storage_error)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS product_provider_profile_catalog_mappings (
                source TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                catalog_profile_id TEXT NOT NULL,
                source_digest TEXT NOT NULL,
                migrated_at TEXT NOT NULL,
                PRIMARY KEY(source, source_profile_id)
            );",
        )
        .map_err(storage_error)?;
    let migrated_at = timestamp();
    for action in actions
        .iter()
        .filter(|action| action.source == ProviderMigrationSource::ProductStore)
    {
        transaction
            .execute(
                "INSERT INTO product_provider_profile_catalog_mappings(
                    source, source_profile_id, catalog_profile_id, source_digest, migrated_at
                 ) VALUES ('product_store_v11', ?1, ?2, ?3, ?4)
                 ON CONFLICT(source, source_profile_id) DO UPDATE SET
                    catalog_profile_id = excluded.catalog_profile_id,
                    source_digest = excluded.source_digest,
                    migrated_at = excluded.migrated_at",
                params![
                    action.source_profile_id,
                    action.catalog_profile_id,
                    action.safe_identity_digest,
                    migrated_at,
                ],
            )
            .map_err(storage_error)?;
    }
    transaction
        .execute(
            "INSERT INTO product_provider_profile_catalog_mappings(
                source, source_profile_id, catalog_profile_id, source_digest, migrated_at
             ) VALUES ('user_catalog_revision', 'singleton', 'singleton', ?1, ?2)
             ON CONFLICT(source, source_profile_id) DO UPDATE SET
                source_digest = excluded.source_digest,
                migrated_at = excluded.migrated_at",
            params![catalog_revision, migrated_at],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn rewrite_workspace_selection(
    path: &Path,
    source: &LegacySource,
    mappings: &BTreeMap<String, String>,
) -> Result<(), ProviderMigrationError> {
    reject_symlink(path, "workspace Provider configuration")?;
    let bytes = read_bounded(
        path,
        MAX_LEGACY_CONFIG_BYTES,
        "workspace Provider configuration",
    )?;
    let Some(expected_digest) = source.raw_digest.as_deref() else {
        return Ok(());
    };
    if hash_bytes(&bytes) != expected_digest {
        return Err(ProviderMigrationError::RevisionConflict);
    }
    let mut value: toml::Value = toml::from_str(&String::from_utf8_lossy(&bytes))
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    if let Some(provider) = value
        .get_mut("provider")
        .and_then(toml::Value::as_table_mut)
    {
        let active = provider
            .get("active")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        for field in [
            "profiles",
            "fallback_profiles",
            "fallback_models",
            "options",
            "base_url",
            "auth",
            "headers",
            "protocol_options",
            "wire_protocol",
        ] {
            provider.remove(field);
        }
        if let Some(active) = active {
            let key = format!("workspace:{active}");
            if let Some(mapped) = mappings.get(&key) {
                provider.insert("active".to_string(), toml::Value::String(mapped.clone()));
            }
        }
    }
    let encoded = toml::to_string_pretty(&value)
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    // Re-read immediately before replacement so an editor cannot be silently
    // overwritten after the migration plan was assembled.
    let latest = read_bounded(
        path,
        MAX_LEGACY_CONFIG_BYTES,
        "workspace Provider configuration",
    )?;
    if hash_bytes(&latest) != expected_digest {
        return Err(ProviderMigrationError::RevisionConflict);
    }
    atomic_write(path, encoded.as_bytes())
}

fn write_receipt(
    service: &ProviderCatalogService,
    report: &ProviderMigrationReport,
) -> Result<PathBuf, ProviderMigrationError> {
    let digest = hash_json(&serde_json::json!({
        "schema_version": report.schema_version,
        "source_digests": report.source_digests,
        "mappings": report.actions.iter().map(|action| serde_json::json!({
            "source": action.source,
            "source_profile_id": action.source_profile_id,
            "catalog_profile_id": action.catalog_profile_id,
            "safe_identity_digest": action.safe_identity_digest,
        })).collect::<Vec<_>>(),
        "catalog_revision_after": report.catalog_revision_after,
    }))?;
    let short = digest.trim_start_matches("sha256:");
    let directory = service.paths().root.join("migrations");
    fs::create_dir_all(&directory).map_err(|_| {
        ProviderMigrationError::Io("could not create receipt directory".to_string())
    })?;
    restrict_directory_permissions(&directory)?;
    let path = directory.join(format!("provider-{}.json", &short[..16]));
    let encoded = serde_json::to_vec_pretty(report)
        .map_err(|error| ProviderMigrationError::InvalidSource(error.to_string()))?;
    atomic_write(&path, &encoded)?;
    restrict_file_permissions(&path)?;
    Ok(path)
}

fn read_bounded(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, ProviderMigrationError> {
    let metadata = fs::metadata(path)
        .map_err(|_| ProviderMigrationError::Io(format!("{label} metadata failed")))?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(ProviderMigrationError::InvalidSource(format!(
            "{label} is not a bounded regular file"
        )));
    }
    fs::read(path).map_err(|_| ProviderMigrationError::Io(format!("could not read {label}")))
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), ProviderMigrationError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ProviderMigrationError::Io(format!("{label} metadata failed")))?;
    if metadata.file_type().is_symlink() {
        return Err(ProviderMigrationError::InvalidSource(format!(
            "{label} must not be a symbolic link"
        )));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProviderMigrationError> {
    let parent = path
        .parent()
        .ok_or_else(|| ProviderMigrationError::Io("output path has no parent".to_string()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| ProviderMigrationError::Io("could not create temporary output".to_string()))?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| ProviderMigrationError::Io("could not flush temporary output".to_string()))?;
    temp.persist(path).map_err(|_| {
        ProviderMigrationError::Io("could not atomically replace output".to_string())
    })?;
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, ProviderMigrationError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(true),
        )
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(storage_error)
}

fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<BTreeSet<String>, ProviderMigrationError> {
    let sql = match table {
        "product_provider_profiles" => "PRAGMA table_info(product_provider_profiles)",
        "product_preferences" => "PRAGMA table_info(product_preferences)",
        "product_session_model_configs" => "PRAGMA table_info(product_session_model_configs)",
        "product_session_run_models" => "PRAGMA table_info(product_session_run_models)",
        _ => {
            return Err(ProviderMigrationError::Storage(
                "unknown ProductStore table".to_string(),
            ));
        }
    };
    let mut statement = connection.prepare(sql).map_err(storage_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage_error)?;
    rows.collect::<Result<BTreeSet<_>, _>>()
        .map_err(storage_error)
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("sha256:{:x}", digest.finalize())
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{seconds}")
}

fn storage_error(_error: rusqlite::Error) -> ProviderMigrationError {
    ProviderMigrationError::Storage("ProductStore operation failed".to_string())
}

fn map_catalog_error(error: ProviderCatalogError) -> ProviderMigrationError {
    match error {
        ProviderCatalogError::RevisionConflict | ProviderCatalogError::Busy => {
            ProviderMigrationError::RevisionConflict
        }
        other => ProviderMigrationError::InvalidSource(other.to_string()),
    }
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<(), ProviderMigrationError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| ProviderMigrationError::Io("could not restrict receipt directory".to_string()))
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) -> Result<(), ProviderMigrationError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<(), ProviderMigrationError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| ProviderMigrationError::Io("could not restrict migration receipt".to_string()))
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<(), ProviderMigrationError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_config::{UserConfigLoader, UserConfigPaths, UserConfigWriter};

    fn service(temp: &tempfile::TempDir) -> ProviderCatalogService {
        ProviderCatalogService::new(UserConfigPaths::from_root(temp.path().join("user")))
    }

    fn options(workspace: &Path) -> ProviderMigrationOptions {
        ProviderMigrationOptions {
            workspace_root: workspace.to_path_buf(),
            trusted_workspace: true,
            product_store_path: None,
            renames: BTreeMap::new(),
            apply: false,
            rewrite_workspace_config: false,
        }
    }

    fn write_workspace_profile(workspace: &Path, endpoint: &str) {
        fs::create_dir_all(workspace.join(".rove")).unwrap();
        fs::write(
            workspace.join(".rove/config.toml"),
            format!(
                "[provider]\nactive = 'legacy'\nmodel = 'legacy-model'\n[provider.profiles.legacy]\nprovider_type = 'openai'\nbase_url = '{endpoint}'\nmodel = 'legacy-model'\nauth = {{ style = 'bearer', secret = {{ env = 'LEGACY_KEY_REF' }} }}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn dry_run_is_redacted_and_does_not_write_catalog_or_workspace() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace_profile(&workspace, "https://legacy.example.test/v1");
        let before = fs::read(workspace.join(".rove/config.toml")).unwrap();

        let report = run_provider_migration(&service(&temp), options(&workspace)).unwrap();

        assert!(!report.applied);
        assert_eq!(report.actions[0].outcome, ProviderMigrationOutcome::Import);
        assert!(!temp.path().join("user/config.toml").exists());
        assert_eq!(
            fs::read(workspace.join(".rove/config.toml")).unwrap(),
            before
        );
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("LEGACY_KEY_REF"));
        assert!(!encoded.contains("legacy.example.test"));
    }

    #[test]
    fn conflict_requires_explicit_rename_and_apply_is_idempotent() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace_profile(&workspace, "https://legacy.example.test/v1");
        let paths = UserConfigPaths::from_root(temp.path().join("user"));
        let existing = UserConfigDocument::from_toml(
            "schema_version = 1\n[model]\ndefault_profile = 'legacy'\n[provider.profiles.legacy]\nprovider_type = 'ollama'\nbase_url = 'http://localhost:11434'\nmodel = 'local'",
        )
        .unwrap();
        UserConfigWriter::new(paths.clone())
            .update(None, &existing)
            .unwrap();
        let catalog = ProviderCatalogService::new(paths.clone());

        let conflict = run_provider_migration(&catalog, options(&workspace)).unwrap();
        assert_eq!(conflict.conflicts.len(), 1);

        let mut renamed = options(&workspace);
        renamed.apply = true;
        renamed.renames.insert(
            "workspace:legacy".to_string(),
            "legacy-imported".to_string(),
        );
        let first = run_provider_migration(&catalog, renamed.clone()).unwrap();
        assert!(first.applied);
        assert_eq!(first.actions[0].outcome, ProviderMigrationOutcome::Rename);
        let second = run_provider_migration(&catalog, renamed).unwrap();
        assert!(second.applied);
        assert_eq!(second.actions[0].outcome, ProviderMigrationOutcome::Merge);
        assert_eq!(first.receipt_path, second.receipt_path);
        let receipt = fs::read_to_string(first.receipt_path.unwrap()).unwrap();
        assert!(!receipt.contains("LEGACY_KEY_REF"));
        assert!(!receipt.contains("legacy.example.test"));
        assert_eq!(
            UserConfigLoader::new(paths)
                .load()
                .unwrap()
                .provider
                .profiles
                .len(),
            2
        );
    }

    #[test]
    fn workspace_is_rewritten_only_with_explicit_confirmation() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace_profile(&workspace, "https://legacy.example.test/v1");
        let catalog = service(&temp);
        let mut apply = options(&workspace);
        apply.apply = true;
        run_provider_migration(&catalog, apply).unwrap();
        assert!(
            fs::read_to_string(workspace.join(".rove/config.toml"))
                .unwrap()
                .contains("provider.profiles")
        );

        let mut rewrite = options(&workspace);
        rewrite.apply = true;
        rewrite.rewrite_workspace_config = true;
        let report = run_provider_migration(&catalog, rewrite).unwrap();
        assert!(report.workspace_rewritten);
        let text = fs::read_to_string(workspace.join(".rove/config.toml")).unwrap();
        assert!(!text.contains("profiles"));
        assert!(text.contains("active = \"legacy\""));
    }

    #[test]
    fn product_store_apply_preserves_legacy_rows_and_writes_mapping() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let database = temp.path().join("product.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE product_provider_profiles(
                    profile_id TEXT PRIMARY KEY, label TEXT NOT NULL,
                    provider_type TEXT NOT NULL, api_base TEXT NOT NULL,
                    api_key_env TEXT, default_model TEXT,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                 );
                 INSERT INTO product_provider_profiles VALUES(
                    'stored', 'Stored', 'ollama', 'http://localhost:11434',
                    NULL, 'local', 'before', 'before'
                 );",
            )
            .unwrap();
        drop(connection);
        let mut migrate = options(&workspace);
        migrate.apply = true;
        migrate.product_store_path = Some(database.clone());

        let report = run_provider_migration(&service(&temp), migrate).unwrap();

        assert!(report.applied);
        let connection = Connection::open(database).unwrap();
        let legacy_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM product_provider_profiles",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mapping: String = connection
            .query_row(
                "SELECT catalog_profile_id FROM product_provider_profile_catalog_mappings WHERE source = 'product_store_v11' AND source_profile_id = 'stored'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_count, 1);
        assert_eq!(mapping, "stored");
    }
}
