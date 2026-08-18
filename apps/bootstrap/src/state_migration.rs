//! Idempotent migration of legacy project-local `.rove/` state into the
//! user state contract directory.
//!
//! The migration is copy-based: the legacy source stays readable until the
//! operator explicitly prunes it, every file is compared by content hash so
//! interrupted runs can resume without duplicating work, and conflicts are
//! never silently overwritten. `cleanup` and `repair` remain separate
//! actions; this module never grants Project Trust capabilities, opens
//! write transactions on legacy stores, starts providers or MCP servers,
//! or invokes models.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use rusqlite::OpenFlags;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::user_state::{
    LEGACY_STATE_DIR, UserStateRoots, WorkspaceStateLayout, path_starts_with_platform,
};
use rove_runtime::state::index::StateIndex;

pub const STATE_MIGRATION_REPORT_SCHEMA_VERSION: i64 = 1;
pub const LEGACY_MIGRATION_RECEIPT_FILE: &str = ".rove-migration-receipt.json";
pub const MIGRATION_DIR: &str = ".migration";
pub const MIGRATION_JOURNAL_FILE: &str = "journal.jsonl";
pub const MIGRATION_RECEIPT_FILE: &str = "migration.json";
pub const MIGRATION_LOCK_FILE: &str = "lock";

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY: Duration = Duration::from_millis(10);
const MAX_MIGRATION_ENTRIES: usize = 200_000;
const MAX_MIGRATION_DEPTH: usize = 48;
const MAX_MIGRATION_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_MAX_MIGRATION_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StateMigrationError {
    #[error("state_migration_data_root_unavailable: {0}")]
    DataRootUnavailable(String),
    #[error("state_migration_invalid_source: {0}")]
    InvalidSource(String),
    #[error("state_migration_locked: another migration holds {0}")]
    Locked(PathBuf),
    #[error("state_migration_bounds_exceeded: {0}")]
    BoundsExceeded(String),
    #[error("state_migration_sqlite: {0}")]
    Sqlite(String),
    #[error("state_migration_io: {0}")]
    Io(String),
    #[error("state_migration_conflict: {0}")]
    Conflict(String),
    #[error("state_migration_json: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<std::io::Error> for StateMigrationError {
    fn from(error: std::io::Error) -> Self {
        StateMigrationError::Io(error.to_string())
    }
}

/// How a file whose target already exists with different content is
/// handled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Keep the target file, leave the source readable, report the entry
    /// as a conflict. The default; nothing is overwritten.
    #[default]
    KeepTarget,
    /// Move the differing target file into the migration conflict
    /// directory, then copy the source.
    BackupTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationFileClass {
    McpCatalog,
    StateSqlite,
    ProductSqlite,
    Memory,
    RunArtifact,
    SelectionStore,
    HealthStore,
    TaskWorkspace,
    ReplHistory,
    Unknown,
}

impl MigrationFileClass {
    fn as_str(&self) -> &'static str {
        match self {
            MigrationFileClass::McpCatalog => "mcp_catalog",
            MigrationFileClass::StateSqlite => "state_sqlite",
            MigrationFileClass::ProductSqlite => "product_sqlite",
            MigrationFileClass::Memory => "memory",
            MigrationFileClass::RunArtifact => "run_artifact",
            MigrationFileClass::SelectionStore => "selection_store",
            MigrationFileClass::HealthStore => "health_store",
            MigrationFileClass::TaskWorkspace => "task_workspace",
            MigrationFileClass::ReplHistory => "repl_history",
            MigrationFileClass::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedFile {
    /// Path relative to the legacy `.rove/` directory.
    pub path: String,
    pub class: String,
    pub bytes: u64,
    /// SHA-256 of the source bytes at planning time. SQLite snapshots use
    /// this as the source identity; the target snapshot may have different
    /// SQLite page layout and is therefore verified through the journal.
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationConflict {
    pub path: String,
    pub reason: String,
    pub resolution: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationRisk {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationWorkspace {
    pub root: String,
    pub kind: String,
    pub storage_key: String,
    pub source_dir: String,
    pub target_dir: String,
    pub data_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationJournalStatus {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationReport {
    pub schema_version: i64,
    pub applied: bool,
    pub workspace: MigrationWorkspace,
    pub source_present: bool,
    pub source_receipt_present: bool,
    pub files: Vec<PlannedFile>,
    pub skipped: Vec<SkippedFile>,
    pub conflicts: Vec<MigrationConflict>,
    pub risks: Vec<MigrationRisk>,
    pub total_bytes: u64,
    pub copied: u64,
    pub skipped_identical: u64,
    pub journal: MigrationJournalStatus,
    pub legacy_disposition: String,
}

#[derive(Debug, Clone)]
pub struct MigrationOptions {
    pub workspace_root: PathBuf,
    /// Explicit data root override; defaults to
    /// [`crate::user_state::UserStateRoots::discover`].
    pub data_root: Option<PathBuf>,
    pub on_conflict: ConflictPolicy,
    pub max_bytes: u64,
    pub prune_legacy: bool,
    pub apply: bool,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            data_root: None,
            on_conflict: ConflictPolicy::KeepTarget,
            max_bytes: DEFAULT_MAX_MIGRATION_BYTES,
            prune_legacy: false,
            apply: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyMigrationReceipt {
    pub schema_version: i64,
    pub target_dir: String,
    pub inventory_digest: String,
    pub finished_unix: u64,
    #[serde(default)]
    pub legacy_disposition: String,
}

struct PlannedMigration {
    layout: WorkspaceStateLayout,
    workspace_kind: String,
    files: Vec<PlannedFile>,
    file_classes: Vec<MigrationFileClass>,
    skipped: Vec<SkippedFile>,
    conflicts: Vec<MigrationConflict>,
    risks: Vec<MigrationRisk>,
    total_bytes: u64,
    source_present: bool,
    source_receipt_present: bool,
}

/// Plan (dry-run) or execute the migration of one workspace's legacy
/// `.rove/` directory into the contract layout.
///
/// Planning performs no writes at all; applying writes only under the
/// contract workspace directory plus the legacy-side receipt file.
pub fn run_state_migration(
    options: &MigrationOptions,
) -> Result<MigrationReport, StateMigrationError> {
    let roots = match options.data_root.clone() {
        Some(root) if root.is_absolute() => UserStateRoots::from_root(root),
        Some(_) => {
            return Err(StateMigrationError::DataRootUnavailable(
                "the explicit data root must be an absolute path".to_string(),
            ));
        }
        _ => UserStateRoots::discover()
            .map_err(|error| StateMigrationError::DataRootUnavailable(error.to_string()))?,
    };
    let workspace_root = options.workspace_root.canonicalize().map_err(|error| {
        StateMigrationError::InvalidSource(format!(
            "could not canonicalize workspace root {}: {error}",
            options.workspace_root.display()
        ))
    })?;
    validate_data_root_boundary(roots.root(), &workspace_root)?;
    let layout = WorkspaceStateLayout::resolve(roots.root(), &workspace_root);
    let source_present = workspace_root.join(LEGACY_STATE_DIR).is_dir();
    if !options.apply || !source_present {
        let planned = plan_legacy_migration(
            &workspace_root,
            &layout,
            options.max_bytes,
            options.on_conflict,
        )?;
        // Dry-run stays read-only; a workspace without legacy state has
        // nothing to apply and must not materialize contract directories
        // or receipts.
        return Ok(report_from_plan(&planned, false, 0, 0, "not_applicable"));
    }

    layout.ensure().map_err(|error| {
        StateMigrationError::Io(format!(
            "could not prepare contract workspace directory: {error}"
        ))
    })?;
    let migration_dir = layout.workspace_dir.join(MIGRATION_DIR);
    ensure_real_directory(&migration_dir, "workspace migration directory")?;
    let _lock = acquire_migration_lock(&migration_dir)?;
    // ProductStore is API-global, so migrations from different legacy
    // workspaces must serialize before inspecting or replacing that target.
    let _product_lock = if workspace_root
        .join(LEGACY_STATE_DIR)
        .join("product.sqlite")
        .is_file()
    {
        let global_migration_dir = roots.root().join(MIGRATION_DIR);
        ensure_real_directory(&global_migration_dir, "global migration directory")?;
        Some(acquire_named_migration_lock(
            &global_migration_dir,
            "product.lock",
        )?)
    } else {
        None
    };

    // Inventory must be created while the lock is held. Otherwise two
    // concurrent apply processes can both plan against the same target and
    // produce a stale conflict/receipt report.
    let planned = plan_legacy_migration(
        &workspace_root,
        &layout,
        options.max_bytes,
        options.on_conflict,
    )?;
    if !planned.source_present {
        return Ok(report_from_plan(&planned, false, 0, 0, "not_applicable"));
    }

    let mut journal = MigrationJournal::append(&migration_dir)?;
    let mut copied = 0u64;
    let mut skipped_identical = 0u64;
    let mut conflicts = planned.conflicts.clone();
    for (index, file) in planned.files.iter().enumerate() {
        let class = planned.file_classes[index].clone();
        let source = workspace_root.join(LEGACY_STATE_DIR).join(&file.path);
        let target = migration_target_path(&layout, &file.path, &class);
        let outcome = migrate_one_file(MigrationFileContext {
            layout: &layout,
            source: &source,
            target: &target,
            relative: &file.path,
            class: &class,
            policy: options.on_conflict,
            migration_dir: &migration_dir,
            journal: &mut journal,
            planned_hash: &file.sha256,
            planned_bytes: file.bytes,
        })?;
        match outcome.as_str() {
            "copied" => copied += 1,
            "skipped_identical" => skipped_identical += 1,
            _ => {}
        }
        let target_hash = file_sha256(&target)?.unwrap_or_default();
        journal.entry(
            file.path.clone(),
            class,
            file.bytes,
            &file.sha256,
            &target_hash,
            &outcome,
        )?;
        if outcome.starts_with("conflict_")
            && !conflicts.iter().any(|conflict| conflict.path == file.path)
        {
            conflicts.push(MigrationConflict {
                path: file.path.clone(),
                reason: "target_differs".to_string(),
                resolution: outcome.clone(),
            });
        }
    }

    let unresolved_conflicts = conflicts
        .iter()
        .any(|conflict| conflict.resolution == "conflict_keep_target");

    let mut legacy_disposition = "kept".to_string();
    if unresolved_conflicts && options.prune_legacy {
        return Err(StateMigrationError::Conflict(format!(
            "refusing to prune legacy state while {} conflict(s) remain unresolved",
            conflicts.len()
        )));
    }

    if !unresolved_conflicts {
        // Write a complete receipt before pruning so `--apply --prune-legacy`
        // is a usable one-shot operation. The source remains untouched until
        // all target content has been revalidated by `prune_legacy_files`.
        write_target_receipt(&migration_dir, &planned, copied, skipped_identical, "kept")?;
        write_legacy_receipt(&workspace_root, &planned, "kept")?;
        if options.prune_legacy {
            let receipt = read_legacy_receipt(&workspace_root)?.ok_or_else(|| {
                StateMigrationError::Conflict(
                    "refusing to prune legacy state without a complete migration receipt"
                        .to_string(),
                )
            })?;
            if receipt.inventory_digest != inventory_digest(&planned) {
                return Err(StateMigrationError::Conflict(
                    "refusing to prune legacy state because the source inventory changed"
                        .to_string(),
                ));
            }
            let left_unknown = prune_legacy_files(&workspace_root, &planned, &layout)?;
            legacy_disposition = if left_unknown {
                "partially_pruned".to_string()
            } else {
                "pruned".to_string()
            };
            write_target_receipt(
                &migration_dir,
                &planned,
                copied,
                skipped_identical,
                &legacy_disposition,
            )?;
            write_legacy_receipt(&workspace_root, &planned, &legacy_disposition)?;
        }
    }

    let mut report = report_from_plan(
        &planned,
        true,
        copied,
        skipped_identical,
        &legacy_disposition,
    );
    // Apply outcomes, not the pre-lock dry-run snapshot, are authoritative.
    report.conflicts = conflicts;
    report.journal.status = if unresolved_conflicts {
        "in_progress".to_string()
    } else {
        "complete".to_string()
    };
    Ok(report)
}

fn plan_legacy_migration(
    workspace_root: &Path,
    layout: &WorkspaceStateLayout,
    max_bytes: u64,
    policy: ConflictPolicy,
) -> Result<PlannedMigration, StateMigrationError> {
    let legacy_dir = workspace_root.join(LEGACY_STATE_DIR);
    let mut files = Vec::new();
    let mut file_classes = Vec::new();
    let mut skipped = Vec::new();
    let mut conflicts = Vec::new();
    let mut risks = Vec::new();
    let mut total_bytes = 0u64;

    let source_present = legacy_dir.is_dir();
    let source_receipt_path = legacy_dir.join(LEGACY_MIGRATION_RECEIPT_FILE);
    let source_receipt_present = source_receipt_path.is_file();

    if !source_present {
        return Ok(PlannedMigration {
            layout: layout.clone(),
            workspace_kind: workspace_kind_string(workspace_root),
            files,
            file_classes,
            skipped,
            conflicts,
            risks,
            total_bytes,
            source_present,
            source_receipt_present,
        });
    }

    let legacy_metadata = fs::symlink_metadata(&legacy_dir).map_err(|error| {
        StateMigrationError::InvalidSource(format!(
            "could not inspect legacy state directory {}: {error}",
            legacy_dir.display()
        ))
    })?;
    if legacy_metadata.file_type().is_symlink() || !legacy_metadata.is_dir() {
        return Err(StateMigrationError::InvalidSource(
            "legacy .rove must be a real directory".to_string(),
        ));
    }
    let canonical_legacy = legacy_dir.canonicalize().map_err(|error| {
        StateMigrationError::InvalidSource(format!(
            "could not canonicalize legacy state directory {}: {error}",
            legacy_dir.display()
        ))
    })?;
    if !path_starts_with_platform(&canonical_legacy, workspace_root) {
        return Err(StateMigrationError::InvalidSource(
            "legacy .rove resolves outside the workspace root".to_string(),
        ));
    }

    let walker = WalkDir::new(&legacy_dir)
        .max_depth(MAX_MIGRATION_DEPTH)
        .follow_root_links(false)
        .follow_links(false)
        .sort_by_file_name();
    let mut entries = 0usize;
    for entry in walker {
        let entry = entry.map_err(|error| {
            StateMigrationError::InvalidSource(format!(
                "could not walk legacy state directory: {error}"
            ))
        })?;
        entries += 1;
        if entries > MAX_MIGRATION_ENTRIES {
            return Err(StateMigrationError::BoundsExceeded(format!(
                "legacy state directory exceeds {MAX_MIGRATION_ENTRIES} entries"
            )));
        }
        if entry.path() == legacy_dir {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&legacy_dir)
            .expect("walkdir yields paths under the root")
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) => {
                risks.push(MigrationRisk {
                    code: "source_metadata_unavailable".to_string(),
                    detail: format!("{relative}: {error}"),
                });
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            risks.push(MigrationRisk {
                code: "source_symlink_not_followed".to_string(),
                detail: relative,
            });
            continue;
        }
        match entry.path().canonicalize() {
            Ok(canonical_entry)
                if path_starts_with_platform(&canonical_entry, &canonical_legacy) => {}
            Ok(_) => {
                risks.push(MigrationRisk {
                    code: "source_reparse_escape_not_followed".to_string(),
                    detail: relative,
                });
                continue;
            }
            Err(error) => {
                risks.push(MigrationRisk {
                    code: "source_canonicalization_failed".to_string(),
                    detail: format!("{relative}: {error}"),
                });
                continue;
            }
        }
        if metadata.is_dir() {
            continue;
        }
        let bytes = metadata.len();
        match classify_relative_path(&relative) {
            Classify::Skip(reason) => skipped.push(SkippedFile {
                path: relative,
                reason: reason.to_string(),
            }),
            Classify::Risk(reason) => risks.push(MigrationRisk {
                code: reason.to_string(),
                detail: relative,
            }),
            Classify::Copy(class) => {
                total_bytes = total_bytes.saturating_add(bytes);
                if total_bytes > max_bytes {
                    return Err(StateMigrationError::BoundsExceeded(format!(
                        "legacy state exceeds the migration size budget of {max_bytes} bytes"
                    )));
                }
                if matches!(
                    class,
                    MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
                ) && let Some(risk) = sqlite_schema_risk(entry.path(), &class)
                {
                    risks.push(risk);
                }
                let journal_proves_same =
                    matches!(
                        class,
                        MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
                    ) && migration_journal_matches(layout, &relative, entry.path(), &class);
                if !journal_proves_same && target_conflicts(&legacy_dir, layout, &relative, &class)?
                {
                    conflicts.push(MigrationConflict {
                        path: relative.clone(),
                        reason: "target_differs".to_string(),
                        resolution: match policy {
                            ConflictPolicy::KeepTarget => "conflict_keep_target".to_string(),
                            ConflictPolicy::BackupTarget => "conflict_backup_target".to_string(),
                        },
                    });
                }
                files.push(PlannedFile {
                    path: relative,
                    class: class.as_str().to_string(),
                    bytes,
                    sha256: file_sha256(entry.path())?.ok_or_else(|| {
                        StateMigrationError::InvalidSource(format!(
                            "source disappeared while planning: {}",
                            entry.path().display()
                        ))
                    })?,
                });
                file_classes.push(class);
            }
        }
    }

    Ok(PlannedMigration {
        layout: layout.clone(),
        workspace_kind: workspace_kind_string(workspace_root),
        files,
        file_classes,
        skipped,
        conflicts,
        risks,
        total_bytes,
        source_present,
        source_receipt_present,
    })
}

enum Classify {
    Skip(&'static str),
    Risk(&'static str),
    Copy(MigrationFileClass),
}

fn classify_relative_path(relative: &str) -> Classify {
    let file_name = relative.rsplit('/').next().unwrap_or_default();
    let top = relative.split('/').next().unwrap_or_default();
    match relative {
        "config.toml" | "config.example.toml" => Classify::Skip("project_config_stays_in_project"),
        LEGACY_MIGRATION_RECEIPT_FILE => Classify::Skip("legacy_migration_receipt"),
        "state.sqlite" => Classify::Copy(MigrationFileClass::StateSqlite),
        "product.sqlite" => Classify::Copy(MigrationFileClass::ProductSqlite),
        "state.sqlite-wal" | "state.sqlite-shm" | "product.sqlite-wal" | "product.sqlite-shm" => {
            Classify::Skip("sqlite_wal_shadow_snapshot_instead")
        }
        "mcp_servers.json" => Classify::Copy(MigrationFileClass::McpCatalog),
        "circuit_breakers.json" => Classify::Copy(MigrationFileClass::HealthStore),
        "repl_history" => Classify::Copy(MigrationFileClass::ReplHistory),
        _ => match top {
            "memory" => {
                let replacement_marker = file_name.starts_with(".memory-index-")
                    || file_name.starts_with(".memory-topic-");
                let transient = file_name.ends_with(".tmp")
                    || file_name.ends_with(".bak")
                    || file_name.ends_with(".ready");
                if replacement_marker && transient {
                    Classify::Risk("memory_replacement_temp_residual")
                } else {
                    Classify::Copy(MigrationFileClass::Memory)
                }
            }
            "runs" => Classify::Copy(MigrationFileClass::RunArtifact),
            "session-model-selections" => Classify::Copy(MigrationFileClass::SelectionStore),
            "tasks" => Classify::Copy(MigrationFileClass::TaskWorkspace),
            _ => Classify::Copy(MigrationFileClass::Unknown),
        },
    }
}

fn workspace_kind_string(workspace_root: &Path) -> String {
    if workspace_root.join(".git").exists() {
        "repo".to_string()
    } else {
        "folder".to_string()
    }
}

/// True when the target already exists with different content, so a
/// conflict entry belongs in the plan (and dry-run report).
fn target_conflicts(
    legacy_dir: &Path,
    layout: &WorkspaceStateLayout,
    relative: &str,
    class: &MigrationFileClass,
) -> Result<bool, StateMigrationError> {
    let target = migration_target_path(layout, relative, class);
    validate_migration_target(layout, &target, class)?;
    let Some(target_hash) = file_sha256(&target)? else {
        return Ok(false);
    };
    let source = legacy_dir.join(relative);
    let Some(source_hash) = file_sha256(&source)? else {
        return Ok(false);
    };
    if matches!(
        class,
        MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
    ) {
        // A target created by a previous successful run is recognized from
        // its journal; without that proof, a differing SQLite byte layout is
        // conservatively reported as a conflict.
        return Ok(source_hash != target_hash);
    }
    Ok(source_hash != target_hash)
}

fn validate_migration_target(
    layout: &WorkspaceStateLayout,
    target: &Path,
    class: &MigrationFileClass,
) -> Result<(), StateMigrationError> {
    let bound = if matches!(class, MigrationFileClass::ProductSqlite) {
        &layout.data_root
    } else {
        &layout.workspace_dir
    };
    let canonical_bound = canonicalize_nearest_existing(bound).map_err(|error| {
        StateMigrationError::Io(format!(
            "could not resolve migration target boundary {}: {error}",
            bound.display()
        ))
    })?;
    let mut current = target.parent().ok_or_else(|| {
        StateMigrationError::Io(format!(
            "migration target has no parent: {}",
            target.display()
        ))
    })?;
    loop {
        match fs::symlink_metadata(current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(StateMigrationError::Io(format!(
                    "migration target parent must not be a symlink: {}",
                    current.display()
                )));
            }
            Ok(_) => {
                let canonical = current.canonicalize().map_err(|error| {
                    StateMigrationError::Io(format!(
                        "could not resolve migration target parent {}: {error}",
                        current.display()
                    ))
                })?;
                if !path_starts_with_platform(&canonical, &canonical_bound) {
                    return Err(StateMigrationError::Io(format!(
                        "migration target parent resolves outside its contract boundary: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(StateMigrationError::Io(format!(
                    "could not inspect migration target parent {}: {error}",
                    current.display()
                )));
            }
        }
        if !path_starts_with_platform(current, bound) {
            return Err(StateMigrationError::Io(format!(
                "migration target parent escapes its contract boundary: {}",
                current.display()
            )));
        }
        if current == bound {
            break;
        }
        current = current.parent().ok_or_else(|| {
            StateMigrationError::Io("migration target boundary has no parent".to_string())
        })?;
    }
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StateMigrationError::Io(format!(
            "migration target must not be a symlink: {}",
            target.display()
        ))),
        Ok(_) => {
            if !path_starts_with_platform(target, bound) {
                return Err(StateMigrationError::Io(format!(
                    "migration target escapes its contract boundary: {}",
                    target.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !path_starts_with_platform(target, bound) {
                return Err(StateMigrationError::Io(format!(
                    "migration target escapes its contract boundary: {}",
                    target.display()
                )));
            }
            Ok(())
        }
        Err(error) => Err(StateMigrationError::Io(format!(
            "could not inspect migration target {}: {error}",
            target.display()
        ))),
    }
}

fn canonicalize_nearest_existing(path: &Path) -> Result<PathBuf, std::io::Error> {
    let mut current = path.to_path_buf();
    while !current.exists() {
        current = current
            .parent()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no existing ancestor")
            })?
            .to_path_buf();
    }
    current.canonicalize()
}

fn validate_data_root_boundary(
    data_root: &Path,
    workspace_root: &Path,
) -> Result<(), StateMigrationError> {
    let canonical_data_root = canonicalize_nearest_existing(data_root).map_err(|error| {
        StateMigrationError::DataRootUnavailable(format!(
            "could not resolve data root {}: {error}",
            data_root.display()
        ))
    })?;
    if path_starts_with_platform(&canonical_data_root, workspace_root) {
        return Err(StateMigrationError::DataRootUnavailable(format!(
            "data root {} must not be inside workspace {}",
            data_root.display(),
            workspace_root.display()
        )));
    }
    Ok(())
}

fn migration_target_path(
    layout: &WorkspaceStateLayout,
    relative: &str,
    class: &MigrationFileClass,
) -> PathBuf {
    if matches!(class, MigrationFileClass::ProductSqlite) && relative == "product.sqlite" {
        layout.product_sqlite.clone()
    } else {
        layout.workspace_dir.join(relative)
    }
}

fn migration_journal_matches(
    layout: &WorkspaceStateLayout,
    relative: &str,
    source: &Path,
    class: &MigrationFileClass,
) -> bool {
    let Ok(source_hash) = file_sha256(source) else {
        return false;
    };
    let Some(source_hash) = source_hash else {
        return false;
    };
    let target = migration_target_path(layout, relative, class);
    let Ok(Some(target_hash)) = file_sha256(&target) else {
        return false;
    };
    let journal = layout
        .workspace_dir
        .join(MIGRATION_DIR)
        .join(MIGRATION_JOURNAL_FILE);
    let Ok(contents) = fs::read_to_string(journal) else {
        return false;
    };
    contents.lines().any(|line| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        value.get("path").and_then(|value| value.as_str()) == Some(relative)
            && value.get("source_sha256").and_then(|value| value.as_str())
                == Some(source_hash.as_str())
            && value.get("target_sha256").and_then(|value| value.as_str())
                == Some(target_hash.as_str())
            && matches!(
                value.get("outcome").and_then(|value| value.as_str()),
                Some("prepared")
                    | Some("copied")
                    | Some("skipped_identical")
                    | Some("conflict_backup_target")
            )
    })
}

/// Best-effort read-only peek at a legacy SQLite database for the plan
/// report. Never writes; failures degrade into a risk entry.
fn sqlite_schema_risk(path: &Path, class: &MigrationFileClass) -> Option<MigrationRisk> {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())
    .ok();
    let Some(connection) = connection else {
        return Some(MigrationRisk {
            code: "sqlite_unreadable".to_string(),
            detail: path.to_string_lossy().replace('\\', "/"),
        });
    };
    let table = if matches!(class, MigrationFileClass::ProductSqlite) {
        "product_schema_migrations"
    } else {
        "schema_migrations"
    };
    let version: Option<i64> = connection
        .query_row(&format!("SELECT MAX(version) FROM {table}"), [], |row| {
            row.get(0)
        })
        .ok();
    let Some(version) = version else {
        return Some(MigrationRisk {
            code: "sqlite_schema_unreadable".to_string(),
            detail: path.to_string_lossy().replace('\\', "/"),
        });
    };
    let current = if matches!(class, MigrationFileClass::ProductSqlite) {
        13
    } else {
        rove_runtime::state::index::CURRENT_SCHEMA_VERSION
    };
    if version > current {
        return Some(MigrationRisk {
            code: "sqlite_schema_newer_than_runtime".to_string(),
            detail: format!("{} schema max = {version}, supported = {current}", table),
        });
    }
    None
}

fn file_sha256(path: &Path) -> Result<Option<String>, StateMigrationError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(StateMigrationError::Io(format!(
                "could not open {}: {error}",
                path.display()
            )));
        }
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

struct MigrationFileContext<'a> {
    layout: &'a WorkspaceStateLayout,
    source: &'a Path,
    target: &'a Path,
    relative: &'a str,
    class: &'a MigrationFileClass,
    policy: ConflictPolicy,
    migration_dir: &'a Path,
    journal: &'a mut MigrationJournal,
    planned_hash: &'a str,
    planned_bytes: u64,
}

fn migrate_one_file(context: MigrationFileContext<'_>) -> Result<String, StateMigrationError> {
    let MigrationFileContext {
        layout,
        source,
        target,
        relative,
        class,
        policy,
        migration_dir,
        journal,
        planned_hash,
        planned_bytes,
    } = context;
    validate_migration_target(layout, target, class)?;
    let source_hash = file_sha256(source)?.ok_or_else(|| {
        StateMigrationError::InvalidSource(format!(
            "planned file disappeared: {}",
            source.display()
        ))
    })?;
    if source_hash != planned_hash {
        return Err(StateMigrationError::InvalidSource(format!(
            "source changed during migration: {}",
            source.display()
        )));
    }
    if let Some(target_hash) = file_sha256(target)? {
        if matches!(
            class,
            MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
        ) && journal.matches_snapshot(relative, planned_hash, &target_hash)
            && verify_sqlite_snapshot(target).is_ok()
        {
            return Ok("skipped_identical".to_string());
        }
        if target_hash == source_hash {
            return Ok("skipped_identical".to_string());
        }
        return match policy {
            ConflictPolicy::KeepTarget => Ok("conflict_keep_target".to_string()),
            ConflictPolicy::BackupTarget => {
                backup_target(target, migration_dir)?;
                // The target may have been replaced while the conflicting
                // file was moved aside. Re-check the parent boundary before
                // writing the replacement.
                validate_migration_target(layout, target, class)?;
                copy_source(CopySourceContext {
                    source,
                    target,
                    relative,
                    class,
                    expected_hash: &source_hash,
                    planned_bytes,
                    journal,
                })?;
                Ok("conflict_backup_target".to_string())
            }
        };
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    // Revalidate after materializing missing parents so a newly introduced
    // symlink cannot redirect the atomic replacement outside the contract.
    validate_migration_target(layout, target, class)?;
    copy_source(CopySourceContext {
        source,
        target,
        relative,
        class,
        expected_hash: &source_hash,
        planned_bytes,
        journal,
    })?;
    Ok("copied".to_string())
}

struct CopySourceContext<'a> {
    source: &'a Path,
    target: &'a Path,
    relative: &'a str,
    class: &'a MigrationFileClass,
    expected_hash: &'a str,
    planned_bytes: u64,
    journal: &'a mut MigrationJournal,
}

fn copy_source(context: CopySourceContext<'_>) -> Result<(), StateMigrationError> {
    let CopySourceContext {
        source,
        target,
        relative,
        class,
        expected_hash,
        planned_bytes,
        journal,
    } = context;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = target.file_name().ok_or_else(|| {
        StateMigrationError::Io(format!(
            "migration target has no file name: {}",
            target.display()
        ))
    })?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".rove-migrate.tmp");
    let tmp = target.with_file_name(temporary_name);
    match fs::symlink_metadata(&tmp) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(StateMigrationError::Io(format!(
                "migration temporary path must be a regular file: {}",
                tmp.display()
            )));
        }
        Ok(_) => fs::remove_file(&tmp)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // Always snapshot SQLite. Copying the main file while a WAL is present
    // can produce a database that opens but silently omits committed facts.
    let sqlite_snapshot = matches!(
        class,
        MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
    );
    let prepared_target_hash = (|| {
        if sqlite_snapshot {
            snapshot_sqlite(source, &tmp)?;
            if matches!(class, MigrationFileClass::StateSqlite) {
                let from_state_dir = source.parent().ok_or_else(|| {
                    StateMigrationError::Sqlite(format!(
                        "state source has no parent: {}",
                        source.display()
                    ))
                })?;
                let to_state_dir = target.parent().ok_or_else(|| {
                    StateMigrationError::Sqlite(format!(
                        "state target has no parent: {}",
                        target.display()
                    ))
                })?;
                StateIndex::with_path(to_state_dir, tmp.clone(), 5_000)
                    .rebase_artifact_paths(from_state_dir, to_state_dir)
                    .map_err(|error| {
                        StateMigrationError::Sqlite(format!(
                            "could not rebase state index artifact paths: {error}"
                        ))
                    })?;
            }
            verify_sqlite_snapshot(&tmp)?;
        } else {
            fs::copy(source, &tmp)?;
            let copied_hash = file_sha256(&tmp)?.ok_or_else(|| {
                StateMigrationError::Io(format!("snapshot vanished: {}", tmp.display()))
            })?;
            if copied_hash != expected_hash {
                return Err(StateMigrationError::Conflict(format!(
                    "copied bytes do not match the source hash for {}",
                    source.display()
                )));
            }
        }
        crate::user_config::harden_file_permissions(&tmp);
        File::options()
            .read(true)
            .write(true)
            .open(&tmp)?
            .sync_all()?;
        file_sha256(&tmp)?.ok_or_else(|| {
            StateMigrationError::Io(format!("prepared snapshot vanished: {}", tmp.display()))
        })
    })();
    let prepared_target_hash = match prepared_target_hash {
        Ok(hash) => hash,
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
    };
    if sqlite_snapshot {
        // A prepared entry closes the crash window between the atomic rename
        // and the final outcome entry. On restart, the target hash and this
        // identity prove that the snapshot was fully prepared.
        if let Err(error) = journal.entry(
            relative.to_string(),
            class.clone(),
            planned_bytes,
            expected_hash,
            &prepared_target_hash,
            "prepared",
        ) {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(StateMigrationError::Io(format!(
            "could not move {} into place: {error}",
            tmp.display()
        )));
    }
    Ok(())
}

/// A vacuumed snapshot is not byte-identical, so verify it opens as a
/// coherent database instead of comparing hashes.
fn verify_sqlite_snapshot(path: &Path) -> Result<(), StateMigrationError> {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        StateMigrationError::Sqlite(format!(
            "snapshot {} does not open: {error}",
            path.display()
        ))
    })?;
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| {
            StateMigrationError::Sqlite(format!(
                "snapshot {} failed integrity check: {error}",
                path.display()
            ))
        })?;
    if !result.eq_ignore_ascii_case("ok") {
        return Err(StateMigrationError::Sqlite(format!(
            "snapshot {} failed integrity check: {result}",
            path.display()
        )));
    }
    Ok(())
}

/// Consistent single-file snapshot of a live SQLite database.
///
/// Opens the source read-only and writes a vacuumed snapshot, which stays
/// correct even when the source has an active WAL. No write transaction
/// is opened on the source.
fn snapshot_sqlite(source: &Path, tmp: &Path) -> Result<(), StateMigrationError> {
    let connection = rusqlite::Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        StateMigrationError::Sqlite(format!(
            "could not open {} read-only: {error}",
            source.display()
        ))
    })?;
    connection
        .execute("VACUUM INTO ?1", [tmp.to_string_lossy().to_string()])
        .map_err(|error| {
            StateMigrationError::Sqlite(format!("could not snapshot {}: {error}", source.display()))
        })?;
    Ok(())
}

fn backup_target(target: &Path, migration_dir: &Path) -> Result<(), StateMigrationError> {
    let conflicts_dir = migration_dir.join("conflicts");
    ensure_real_directory(&conflicts_dir, "migration conflict directory")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let backup = conflicts_dir.join(format!("{stamp}-{file_name}"));
    let mut unique = backup.clone();
    let mut counter = 0u32;
    while unique.exists() {
        counter += 1;
        unique = conflicts_dir.join(format!("{stamp}-{counter}-{file_name}"));
    }
    fs::rename(target, unique).map_err(|error| {
        StateMigrationError::Io(format!(
            "could not back up conflicting target {}: {error}",
            target.display()
        ))
    })?;
    Ok(())
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<(), StateMigrationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(StateMigrationError::Io(format!(
                "{label} must be a real directory: {}",
                path.display()
            )));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::create_dir_all(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(StateMigrationError::Io(format!(
                "{label} must be a real directory: {}",
                path.display()
            )))
        }
        Ok(_) => {
            crate::user_config::harden_directory_permissions(path);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_regular_file_path(path: &Path, label: &str) -> Result<(), StateMigrationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(StateMigrationError::Io(format!(
                "{label} must be a regular file: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct MigrationLock {
    file: File,
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn acquire_migration_lock(migration_dir: &Path) -> Result<MigrationLock, StateMigrationError> {
    acquire_named_migration_lock(migration_dir, MIGRATION_LOCK_FILE)
}

fn acquire_named_migration_lock(
    migration_dir: &Path,
    file_name: &str,
) -> Result<MigrationLock, StateMigrationError> {
    let lock_path = migration_dir.join(file_name);
    ensure_regular_file_path(&lock_path, "migration lock")?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            StateMigrationError::Io(format!(
                "could not open migration lock {}: {error}",
                lock_path.display()
            ))
        })?;
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(MigrationLock { file }),
            Err(error) if is_lock_contention(&error) => {
                if Instant::now() >= deadline {
                    return Err(StateMigrationError::Locked(lock_path));
                }
                std::thread::sleep(LOCK_RETRY);
            }
            Err(error) => {
                return Err(StateMigrationError::Io(format!(
                    "could not acquire migration lock: {error}"
                )));
            }
        }
    }
}

fn is_lock_contention(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(32 | 33))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Debug, Clone)]
struct JournalIdentity {
    source_hash: String,
    target_hash: String,
}

struct MigrationJournal {
    file: Option<File>,
    seq: u64,
    identities: HashMap<String, JournalIdentity>,
}

impl MigrationJournal {
    fn append(migration_dir: &Path) -> Result<Self, StateMigrationError> {
        let path = migration_dir.join(MIGRATION_JOURNAL_FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(StateMigrationError::Io(format!(
                    "migration journal must be a regular file: {}",
                    path.display()
                )));
            }
            Ok(metadata) if metadata.len() > MAX_MIGRATION_JOURNAL_BYTES => {
                return Err(StateMigrationError::BoundsExceeded(format!(
                    "migration journal exceeds {MAX_MIGRATION_JOURNAL_BYTES} bytes"
                )));
            }
            Ok(_) | Err(_) => {}
        }
        let mut identities = HashMap::new();
        let mut seq = 0u64;
        if let Ok(contents) = fs::read_to_string(&path) {
            for line in contents.lines() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                seq = seq.max(
                    value
                        .get("seq")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0),
                );
                if let (Some(path), Some(hash)) = (
                    value.get("path").and_then(|value| value.as_str()),
                    value.get("source_sha256").and_then(|value| value.as_str()),
                ) && let Some(target_hash) =
                    value.get("target_sha256").and_then(|value| value.as_str())
                    && matches!(
                        value.get("outcome").and_then(|value| value.as_str()),
                        Some("prepared")
                            | Some("copied")
                            | Some("skipped_identical")
                            | Some("conflict_backup_target")
                    )
                {
                    identities.insert(
                        path.to_string(),
                        JournalIdentity {
                            source_hash: hash.to_string(),
                            target_hash: target_hash.to_string(),
                        },
                    );
                }
            }
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                StateMigrationError::Io(format!("could not open migration journal: {error}"))
            })?;
        Ok(Self {
            file: Some(file),
            seq,
            identities,
        })
    }

    fn matches_snapshot(&self, path: &str, source_hash: &str, target_hash: &str) -> bool {
        self.identities.get(path).is_some_and(|identity| {
            identity.source_hash == source_hash && identity.target_hash == target_hash
        })
    }

    /// Append one outcome line. The journal is part of the observable resume
    /// contract; a failed append aborts rather than claiming a durable result
    /// that cannot be audited.
    fn entry(
        &mut self,
        path: String,
        class: MigrationFileClass,
        bytes: u64,
        source_sha256: &str,
        target_sha256: &str,
        outcome: &str,
    ) -> Result<(), StateMigrationError> {
        use std::io::Write;
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        self.seq += 1;
        let line = serde_json::json!({
            "seq": self.seq,
            "path": path,
            "class": class.as_str(),
            "bytes": bytes,
            "source_sha256": source_sha256,
            "target_sha256": target_sha256,
            "outcome": outcome,
        });
        writeln!(file, "{line}").map_err(|error| {
            StateMigrationError::Io(format!("could not append migration journal: {error}"))
        })?;
        file.flush().map_err(|error| {
            StateMigrationError::Io(format!("could not flush migration journal: {error}"))
        })?;
        file.sync_data().map_err(|error| {
            StateMigrationError::Io(format!("could not sync migration journal: {error}"))
        })?;
        if matches!(
            outcome,
            "prepared" | "copied" | "skipped_identical" | "conflict_backup_target"
        ) {
            self.identities.insert(
                path,
                JournalIdentity {
                    source_hash: source_sha256.to_string(),
                    target_hash: target_sha256.to_string(),
                },
            );
        }
        Ok(())
    }
}

fn inventory_digest(planned: &PlannedMigration) -> String {
    let mut hasher = Sha256::new();
    for file in &planned.files {
        hasher.update(file.path.as_bytes());
        hasher.update(file.sha256.as_bytes());
        hasher.update(file.bytes.to_le_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn write_target_receipt(
    migration_dir: &Path,
    planned: &PlannedMigration,
    copied: u64,
    skipped_identical: u64,
    legacy_disposition: &str,
) -> Result<(), StateMigrationError> {
    let receipt = serde_json::json!({
        "schema_version": STATE_MIGRATION_REPORT_SCHEMA_VERSION,
        "finished_unix": unix_now(),
        "source_dir": planned
            .layout
            .canonical_workspace_root
            .join(LEGACY_STATE_DIR)
            .to_string_lossy()
            .replace('\\', "/"),
        "target_dir": planned
            .layout
            .workspace_dir
            .to_string_lossy()
            .replace('\\', "/"),
        "file_count": planned.files.len(),
        "total_bytes": planned.total_bytes,
        "copied": copied,
        "skipped_identical": skipped_identical,
        "inventory_digest": inventory_digest(planned),
        "legacy_disposition": legacy_disposition,
    });
    let path = migration_dir.join(MIGRATION_RECEIPT_FILE);
    let tmp = migration_dir.join(format!(
        ".{MIGRATION_RECEIPT_FILE}.{}.tmp",
        std::process::id()
    ));
    fs::write(&tmp, serde_json::to_vec_pretty(&receipt)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn write_legacy_receipt(
    workspace_root: &Path,
    planned: &PlannedMigration,
    legacy_disposition: &str,
) -> Result<(), StateMigrationError> {
    let receipt = LegacyMigrationReceipt {
        schema_version: STATE_MIGRATION_REPORT_SCHEMA_VERSION,
        target_dir: planned
            .layout
            .workspace_dir
            .to_string_lossy()
            .replace('\\', "/"),
        inventory_digest: inventory_digest(planned),
        finished_unix: unix_now(),
        legacy_disposition: legacy_disposition.to_string(),
    };
    let path = workspace_root
        .join(LEGACY_STATE_DIR)
        .join(LEGACY_MIGRATION_RECEIPT_FILE);
    let tmp = workspace_root.join(LEGACY_STATE_DIR).join(format!(
        ".{}.{}.tmp",
        LEGACY_MIGRATION_RECEIPT_FILE,
        std::process::id()
    ));
    fs::write(&tmp, serde_json::to_vec_pretty(&receipt)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Read the legacy-side migration receipt, when this workspace has already
/// been migrated.
pub fn read_legacy_receipt(
    workspace_root: &Path,
) -> Result<Option<LegacyMigrationReceipt>, StateMigrationError> {
    let path = workspace_root
        .join(LEGACY_STATE_DIR)
        .join(LEGACY_MIGRATION_RECEIPT_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(StateMigrationError::Io(format!(
                "could not read legacy migration receipt: {error}"
            )));
        }
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        StateMigrationError::InvalidSource(format!(
            "legacy migration receipt is unreadable: {error}"
        ))
    })
}

fn prune_legacy_files(
    workspace_root: &Path,
    planned: &PlannedMigration,
    layout: &WorkspaceStateLayout,
) -> Result<bool, StateMigrationError> {
    let legacy_dir = workspace_root.join(LEGACY_STATE_DIR);
    let migration_dir = layout.workspace_dir.join(MIGRATION_DIR);
    let mut left_unknown = false;
    for (index, file) in planned.files.iter().enumerate() {
        let class = &planned.file_classes[index];
        if matches!(class, MigrationFileClass::Unknown) {
            // Unknown files may be project-owned extensions. They can be
            // copied for preservation, but are never deleted automatically.
            left_unknown = true;
            continue;
        }
        let source = legacy_dir.join(&file.path);
        let target = migration_target_path(layout, &file.path, class);
        validate_migration_target(layout, &target, class)?;
        let source_hash = file_sha256(&source)?;
        let target_hash = file_sha256(&target)?;
        let byte_identical = matches!((source_hash, target_hash),
            (Some(source_hash), Some(target_hash)) if source_hash == target_hash);
        let snapshot_verified = matches!(
            class,
            MigrationFileClass::StateSqlite | MigrationFileClass::ProductSqlite
        ) && sqlite_snapshot_matches(
            &source,
            &target,
            &migration_dir,
            class,
            &legacy_dir,
            layout,
        )?;
        if byte_identical || snapshot_verified {
            fs::remove_file(&source).map_err(|error| {
                StateMigrationError::Io(format!("could not prune {}: {error}", source.display()))
            })?;
        } else {
            return Err(StateMigrationError::Conflict(format!(
                "refusing to prune {}: source and target digests disagree",
                file.path
            )));
        }
    }
    remove_empty_dirs(&legacy_dir)?;
    Ok(left_unknown)
}

fn sqlite_snapshot_matches(
    source: &Path,
    target: &Path,
    migration_dir: &Path,
    class: &MigrationFileClass,
    legacy_dir: &Path,
    layout: &WorkspaceStateLayout,
) -> Result<bool, StateMigrationError> {
    if !target.is_file() {
        return Ok(false);
    }
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("database.sqlite");
    let tmp = migration_dir.join(format!(".prune-{file_name}-{}.tmp", std::process::id()));
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    let result = (|| {
        snapshot_sqlite(source, &tmp)?;
        if matches!(class, MigrationFileClass::StateSqlite) {
            StateIndex::with_path(&layout.workspace_dir, tmp.clone(), 5_000)
                .rebase_artifact_paths(legacy_dir, &layout.workspace_dir)
                .map_err(|error| {
                    StateMigrationError::Sqlite(format!(
                        "could not verify rebased state index paths during prune: {error}"
                    ))
                })?;
        }
        let source_snapshot = file_sha256(&tmp)?.ok_or_else(|| {
            StateMigrationError::Sqlite("SQLite snapshot disappeared during prune".to_string())
        })?;
        let target_hash = file_sha256(target)?.unwrap_or_default();
        verify_sqlite_snapshot(target)?;
        Ok(source_snapshot == target_hash)
    })();
    let _ = fs::remove_file(&tmp);
    result
}

/// Remove directories left empty by pruning, deepest first. The legacy
/// `.rove` directory itself is kept: project configuration stays there.
fn remove_empty_dirs(root: &Path) -> Result<(), StateMigrationError> {
    let mut dirs: Vec<PathBuf> = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_dir())
        .map(|entry| entry.into_path())
        .collect();
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for dir in dirs {
        if dir != root
            && fs::read_dir(&dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(&dir);
        }
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn report_from_plan(
    planned: &PlannedMigration,
    applied: bool,
    copied: u64,
    skipped_identical: u64,
    legacy_disposition: &str,
) -> MigrationReport {
    MigrationReport {
        schema_version: STATE_MIGRATION_REPORT_SCHEMA_VERSION,
        applied,
        workspace: MigrationWorkspace {
            root: planned
                .layout
                .canonical_workspace_root
                .to_string_lossy()
                .replace('\\', "/"),
            kind: planned.workspace_kind.clone(),
            storage_key: planned.layout.storage_key.clone(),
            source_dir: planned
                .layout
                .canonical_workspace_root
                .join(LEGACY_STATE_DIR)
                .to_string_lossy()
                .replace('\\', "/"),
            target_dir: planned
                .layout
                .workspace_dir
                .to_string_lossy()
                .replace('\\', "/"),
            data_root: planned
                .layout
                .data_root
                .to_string_lossy()
                .replace('\\', "/"),
        },
        source_present: planned.source_present,
        source_receipt_present: planned.source_receipt_present,
        files: planned.files.clone(),
        skipped: planned.skipped.clone(),
        conflicts: planned.conflicts.clone(),
        risks: planned.risks.clone(),
        total_bytes: planned.total_bytes,
        copied,
        skipped_identical,
        journal: MigrationJournalStatus {
            path: planned
                .layout
                .workspace_dir
                .join(MIGRATION_DIR)
                .join(MIGRATION_JOURNAL_FILE)
                .to_string_lossy()
                .replace('\\', "/"),
            status: if applied {
                "complete".to_string()
            } else {
                "none".to_string()
            },
        },
        legacy_disposition: legacy_disposition.to_string(),
    }
}
