//! Behavior tests for the legacy `.rove/` -> user state contract
//! migration engine. These run as a separate test binary, so the
//! environment-mutating digest test cannot race the in-crate config
//! tests.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use fs2::FileExt;
use rove_app_bootstrap::state_migration::{
    ConflictPolicy, DEFAULT_MAX_MIGRATION_BYTES, MIGRATION_DIR, MIGRATION_JOURNAL_FILE,
    MIGRATION_LOCK_FILE, MigrationOptions, StateMigrationError, run_state_migration,
};
use rove_app_bootstrap::{WorkspaceStateLayout, capability_digest_map};
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::state::store::{StateStore, TASK_STATE_SCHEMA_VERSION};
use rove_runtime::types::{JobId, RunId, SessionId, TaskState};
use rusqlite::Connection;

fn options(workspace_root: &Path, data_root: &Path) -> MigrationOptions {
    MigrationOptions {
        workspace_root: workspace_root.to_path_buf(),
        data_root: Some(data_root.to_path_buf()),
        on_conflict: ConflictPolicy::KeepTarget,
        max_bytes: DEFAULT_MAX_MIGRATION_BYTES,
        prune_legacy: false,
        apply: false,
    }
}

fn write_legacy_layout(workspace_root: &Path) {
    let legacy = workspace_root.join(".rove");
    std::fs::create_dir_all(legacy.join("memory/topics")).unwrap();
    std::fs::create_dir_all(legacy.join("memory/sessions")).unwrap();
    std::fs::create_dir_all(legacy.join("runs/01J/run")).unwrap();
    std::fs::create_dir_all(legacy.join("session-model-selections")).unwrap();
    std::fs::create_dir_all(legacy.join("tasks/job-1")).unwrap();
    std::fs::write(legacy.join("config.toml"), "[state]\n").unwrap();
    std::fs::write(legacy.join("mcp_servers.json"), r#"{"servers":[]}"#).unwrap();
    std::fs::write(legacy.join("memory/MEMORY.md"), "# memory\n").unwrap();
    std::fs::write(legacy.join("memory/sessions/01S.md"), "summary\n").unwrap();
    std::fs::write(legacy.join("memory/.memory-index-1.tmp"), "residual temp\n").unwrap();
    std::fs::write(legacy.join("runs/01J/run/trace.jsonl"), "{}\n").unwrap();
    std::fs::write(
        legacy.join("runs/01J/run/task_state.json"),
        "{\"schema_version\":1}",
    )
    .unwrap();
    std::fs::write(legacy.join("session-model-selections/01SEL.json"), "{}").unwrap();
    std::fs::write(legacy.join("tasks/job-1/notes.md"), "task\n").unwrap();
    std::fs::write(legacy.join("repl_history"), "hello\n").unwrap();
}

fn write_sqlite(path: &Path, value: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "CREATE TABLE IF NOT EXISTS values_table (value TEXT NOT NULL)",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO values_table(value) VALUES (?1)", [value])
        .unwrap();
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

#[test]
fn fresh_workspace_reports_no_source_without_side_effects() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let data_root = tmp.path().join("data");

    let report = run_state_migration(&options(&workspace, &data_root)).unwrap();
    assert!(!report.source_present);
    assert!(report.files.is_empty());
    assert!(!data_root.exists(), "no contract dir for a dry-run");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let report = run_state_migration(&applied).unwrap();
    assert!(!report.source_present);
    assert!(
        !data_root.exists(),
        "apply without legacy state must not materialize contract directories"
    );
}

#[test]
fn dry_run_reports_classification_and_writes_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");

    let report = run_state_migration(&options(&workspace, &data_root)).unwrap();

    assert!(!report.applied);
    assert!(report.source_present);
    assert_eq!(report.journal.status, "none");
    let classes: Vec<&str> = report
        .files
        .iter()
        .map(|file| file.class.as_str())
        .collect();
    for expected in [
        "mcp_catalog",
        "memory",
        "run_artifact",
        "selection_store",
        "task_workspace",
        "repl_history",
    ] {
        assert!(
            classes.contains(&expected),
            "missing class {expected}: {classes:?}"
        );
    }
    assert!(
        report
            .skipped
            .iter()
            .any(|file| file.path == "config.toml"
                && file.reason == "project_config_stays_in_project")
    );
    assert!(
        report
            .risks
            .iter()
            .any(|risk| risk.code == "memory_replacement_temp_residual")
    );
    assert!(
        !data_root.exists(),
        "dry-run must not create the contract directory"
    );
}

#[test]
fn apply_copies_files_writes_receipts_and_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let report = run_state_migration(&applied).unwrap();
    assert!(report.applied);
    assert_eq!(report.copied as usize, report.files.len());
    assert_eq!(report.skipped_identical, 0);
    assert!(report.conflicts.is_empty());

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    assert_eq!(
        std::fs::read_to_string(layout.workspace_dir.join("mcp_servers.json")).unwrap(),
        r#"{"servers":[]}"#
    );
    assert!(
        layout
            .workspace_dir
            .join(".migration/journal.jsonl")
            .is_file()
    );
    assert!(
        layout
            .workspace_dir
            .join(".migration/migration.json")
            .is_file()
    );
    assert!(
        workspace
            .join(".rove")
            .join(".rove-migration-receipt.json")
            .is_file()
    );
    assert!(workspace.join(".rove/config.toml").is_file());
    assert!(
        !layout
            .workspace_dir
            .join("memory/.memory-index-1.tmp")
            .exists()
    );

    let first_sequences: Vec<u64> =
        std::fs::read_to_string(layout.workspace_dir.join(".migration/journal.jsonl"))
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|value| value.get("seq").and_then(|seq| seq.as_u64()))
            .collect();
    assert!(!first_sequences.is_empty());

    let second = run_state_migration(&applied).unwrap();
    assert_eq!(second.copied, 0);
    assert_eq!(second.skipped_identical as usize, second.files.len());
    assert!(second.conflicts.is_empty());
    let second_sequences: Vec<u64> =
        std::fs::read_to_string(layout.workspace_dir.join(".migration/journal.jsonl"))
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|value| value.get("seq").and_then(|seq| seq.as_u64()))
            .collect();
    assert!(
        second_sequences.iter().max().unwrap() > first_sequences.iter().max().unwrap(),
        "a resumed journal must continue its sequence"
    );
}

#[test]
fn interrupted_apply_resumes_without_duplicates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    run_state_migration(&applied).unwrap();

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    std::fs::remove_file(layout.workspace_dir.join("memory/MEMORY.md")).unwrap();

    let resumed = run_state_migration(&applied).unwrap();
    assert_eq!(resumed.copied, 1);
    assert_eq!(resumed.skipped_identical as usize + 1, resumed.files.len());
    assert!(layout.workspace_dir.join("memory/MEMORY.md").is_file());
}

#[test]
fn prepared_sqlite_journal_recovers_when_the_final_outcome_is_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(workspace.join(".rove")).unwrap();
    write_sqlite(&workspace.join(".rove/state.sqlite"), "source");
    let data_root = tmp.path().join("data");
    let mut applied = options(&workspace, &data_root);
    applied.apply = true;

    run_state_migration(&applied).unwrap();
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    let journal_path = layout
        .workspace_dir
        .join(MIGRATION_DIR)
        .join(MIGRATION_JOURNAL_FILE);
    let retained = std::fs::read_to_string(&journal_path)
        .unwrap()
        .lines()
        .filter(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value.get("path").and_then(|value| value.as_str()) != Some("state.sqlite")
                || value.get("outcome").and_then(|value| value.as_str()) == Some("prepared")
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&journal_path, format!("{retained}\n")).unwrap();

    let resumed = run_state_migration(&applied).unwrap();
    assert!(resumed.conflicts.is_empty());
    assert_eq!(resumed.copied, 0);
    assert_eq!(resumed.skipped_identical, 1);
    assert!(layout.state_sqlite.is_file());
}

#[test]
fn corrupt_journal_lines_do_not_block_a_partial_retry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");
    let mut applied = options(&workspace, &data_root);
    applied.apply = true;

    run_state_migration(&applied).unwrap();
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    let journal_path = layout
        .workspace_dir
        .join(MIGRATION_DIR)
        .join(MIGRATION_JOURNAL_FILE);
    writeln!(
        File::options().append(true).open(&journal_path).unwrap(),
        "{{not-json"
    )
    .unwrap();
    std::fs::remove_file(layout.workspace_dir.join("memory/MEMORY.md")).unwrap();

    let resumed = run_state_migration(&applied).unwrap();
    assert_eq!(resumed.copied, 1);
    assert!(layout.workspace_dir.join("memory/MEMORY.md").is_file());
}

#[test]
fn corrupt_sqlite_fails_visibly_and_keeps_the_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    let legacy = workspace.join(".rove");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("state.sqlite"), b"not a sqlite database").unwrap();
    let data_root = tmp.path().join("data");

    let plan = run_state_migration(&options(&workspace, &data_root)).unwrap();
    assert!(
        plan.risks
            .iter()
            .any(|risk| risk.code == "sqlite_schema_unreadable")
    );
    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let error = run_state_migration(&applied).expect_err("corrupt SQLite must fail");
    assert!(matches!(error, StateMigrationError::Sqlite(_)));

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    assert!(legacy.join("state.sqlite").is_file());
    assert!(!layout.state_sqlite.exists());
    assert!(
        !layout
            .workspace_dir
            .join("state.sqlite.rove-migrate.tmp")
            .exists()
    );
}

#[test]
fn differing_target_is_a_conflict_and_never_overwritten_by_default() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);

    std::fs::create_dir_all(layout.workspace_dir.join("memory")).unwrap();
    std::fs::write(layout.workspace_dir.join("memory/MEMORY.md"), "different\n").unwrap();

    let plan = run_state_migration(&options(&workspace, &data_root)).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].path, "memory/MEMORY.md");
    assert_eq!(plan.conflicts[0].resolution, "conflict_keep_target");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let report = run_state_migration(&applied).unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(
        std::fs::read_to_string(layout.workspace_dir.join("memory/MEMORY.md")).unwrap(),
        "different\n",
        "keep-target policy must not overwrite the target"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join(".rove/memory/MEMORY.md")).unwrap(),
        "# memory\n",
        "the source stays readable"
    );
    assert!(
        !workspace
            .join(".rove")
            .join(".rove-migration-receipt.json")
            .is_file()
    );

    let mut backup = options(&workspace, &data_root);
    backup.apply = true;
    backup.on_conflict = ConflictPolicy::BackupTarget;
    let report = run_state_migration(&backup).unwrap();
    assert_eq!(
        std::fs::read_to_string(layout.workspace_dir.join("memory/MEMORY.md")).unwrap(),
        "# memory\n"
    );
    assert_eq!(
        layout
            .workspace_dir
            .join(".migration/conflicts")
            .read_dir()
            .unwrap()
            .count(),
        1,
        "the old target must be backed up"
    );
    assert_eq!(report.journal.status, "complete");
}

#[test]
fn sqlite_state_index_migrates_into_a_usable_target() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let legacy = workspace.join(".rove");
    std::fs::create_dir_all(&legacy).unwrap();
    let source_index = rove_runtime::state::index::StateIndex::new(&legacy);
    source_index.initialize().unwrap();
    drop(source_index);
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let report = run_state_migration(&applied).unwrap();
    assert!(report.files.iter().any(|file| file.class == "state_sqlite"));

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    let target_index = rove_runtime::state::index::StateIndex::with_path(
        &layout.workspace_dir,
        layout.state_sqlite.clone(),
        5_000,
    );
    target_index.initialize().unwrap();
}

#[tokio::test]
async fn migrated_state_index_rebases_paths_and_resumes_after_prune() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let legacy = workspace.join(".rove");
    let source_store = StateStore::new(&legacy);
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let handle = source_store.start_run(session_id, job_id, run_id).unwrap();
    let state = TaskState {
        schema_version: TASK_STATE_SCHEMA_VERSION,
        session_id,
        job_id,
        run_id,
        goal: "resume after state migration".to_string(),
        step: 3,
        history: Vec::new(),
        summary: Some("durable migration checkpoint".to_string()),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        agent_profile: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    };
    source_store.write_task_state(&state).await.unwrap();
    let source_report = handle.run_dir.join("report.json");
    std::fs::write(&source_report, "{}").unwrap();
    source_store
        .record_report(
            run_id,
            source_report,
            "incomplete".to_string(),
            "step_limit".to_string(),
        )
        .await
        .unwrap();
    drop(handle);
    drop(source_store);

    let data_root = tmp.path().join("data");
    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    applied.prune_legacy = true;
    let report = run_state_migration(&applied).unwrap();
    assert_eq!(report.legacy_disposition, "pruned");

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    assert!(
        !legacy
            .join("runs")
            .join(run_id.to_string())
            .join("task_state.json")
            .exists(),
        "the target must not be able to read the pruned legacy artifact"
    );
    let target_store = StateStore::new(&layout.workspace_dir);
    let indexed_task_path = target_store.index.task_state_path(run_id).unwrap();
    let canonical_workspace_dir = layout.workspace_dir.canonicalize().unwrap();
    assert!(
        indexed_task_path
            .as_deref()
            .and_then(|path| path.canonicalize().ok())
            .is_some_and(|path| path.starts_with(&canonical_workspace_dir)),
        "migrated task-state path was not rebased: {indexed_task_path:?}"
    );
    let resumed = resolve_resume_state(&target_store, Some(&run_id.to_string()))
        .await
        .unwrap()
        .expect("the migrated run remains resumable");
    assert_eq!(resumed.run_id, run_id);
    assert_eq!(resumed.step, 3);
    assert_eq!(
        resumed.summary.as_deref(),
        Some("durable migration checkpoint")
    );

    let indexed = target_store
        .index
        .run_record(run_id)
        .unwrap()
        .expect("the migrated run remains indexed");
    assert!(
        indexed
            .run_dir
            .canonicalize()
            .is_ok_and(|path| path.starts_with(&canonical_workspace_dir))
    );
    assert!(
        indexed
            .task_state_path
            .as_deref()
            .and_then(|path| path.canonicalize().ok())
            .is_some_and(|path| path.starts_with(&canonical_workspace_dir))
    );
    assert!(
        indexed
            .report_path
            .as_deref()
            .and_then(|path| path.canonicalize().ok())
            .is_some_and(|path| path.starts_with(&canonical_workspace_dir))
    );
}

#[test]
fn prune_removes_migrated_files_but_never_project_configuration() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    run_state_migration(&applied).unwrap();

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    std::fs::write(layout.workspace_dir.join("repl_history"), "tampered\n").unwrap();
    let mut prune = applied.clone();
    prune.prune_legacy = true;
    let error = run_state_migration(&prune).unwrap_err();
    assert!(error.to_string().contains("refusing to prune"));

    std::fs::remove_file(layout.workspace_dir.join("repl_history")).unwrap();
    run_state_migration(&applied).unwrap();

    let report = run_state_migration(&prune).unwrap();
    assert_eq!(report.legacy_disposition, "pruned");
    let legacy = workspace.join(".rove");
    assert!(legacy.join("config.toml").is_file());
    assert!(!legacy.join("mcp_servers.json").exists());
    // Migrated memory content is gone; the transient replacement
    // marker is deliberately preserved (it may be mid-recovery).
    assert!(!legacy.join("memory/MEMORY.md").exists());
    assert!(!legacy.join("memory/sessions").exists());
    assert!(!legacy.join("runs").exists());
}

#[test]
fn apply_prune_is_a_safe_one_shot_operation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");

    let mut one_shot = options(&workspace, &data_root);
    one_shot.apply = true;
    one_shot.prune_legacy = true;
    let report = run_state_migration(&one_shot).unwrap();

    assert_eq!(report.legacy_disposition, "pruned");
    let legacy = workspace.join(".rove");
    assert!(legacy.join("config.toml").is_file());
    assert!(!legacy.join("mcp_servers.json").exists());
    assert!(!legacy.join("memory/MEMORY.md").exists());
    assert!(
        legacy.join(".rove-migration-receipt.json").is_file(),
        "the source receipt remains as the audit marker"
    );
}

#[test]
fn unknown_files_are_copied_but_report_partial_prune_and_keep_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let unknown = workspace.join(".rove/custom-extension.json");
    std::fs::write(&unknown, "project-owned extension\n").unwrap();
    let data_root = tmp.path().join("data");

    let mut one_shot = options(&workspace, &data_root);
    one_shot.apply = true;
    one_shot.prune_legacy = true;
    let report = run_state_migration(&one_shot).unwrap();

    assert_eq!(report.legacy_disposition, "partially_pruned");
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    assert_eq!(
        std::fs::read_to_string(layout.workspace_dir.join("custom-extension.json")).unwrap(),
        "project-owned extension\n"
    );
    assert!(
        unknown.is_file(),
        "unknown source files are never deleted automatically"
    );
    assert!(!workspace.join(".rove/mcp_servers.json").exists());
}

#[test]
fn replaced_sqlite_target_is_a_conflict_even_when_the_journal_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(workspace.join(".rove")).unwrap();
    write_sqlite(&workspace.join(".rove/state.sqlite"), "source");
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    run_state_migration(&applied).unwrap();

    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    std::fs::remove_file(&layout.state_sqlite).unwrap();
    write_sqlite(&layout.state_sqlite, "replacement");

    let report = run_state_migration(&applied).unwrap();
    assert!(
        report
            .conflicts
            .iter()
            .any(|conflict| conflict.path == "state.sqlite"
                && conflict.resolution == "conflict_keep_target")
    );
    let connection = Connection::open(&layout.state_sqlite).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM values_table", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "replacement");
}

#[test]
fn product_store_migration_uses_one_global_target_and_surfaces_conflicts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(workspace.join(".rove")).unwrap();
    write_sqlite(&workspace.join(".rove/product.sqlite"), "source");
    let data_root = tmp.path().join("data");

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    run_state_migration(&applied).unwrap();

    let global_target = data_root.join("product.sqlite");
    assert!(global_target.is_file());
    write_sqlite(
        &global_target.with_extension("replacement.sqlite"),
        "replacement",
    );
    std::fs::remove_file(&global_target).unwrap();
    std::fs::rename(
        global_target.with_extension("replacement.sqlite"),
        &global_target,
    )
    .unwrap();

    let report = run_state_migration(&applied).unwrap();
    assert!(
        report
            .conflicts
            .iter()
            .any(|conflict| conflict.path == "product.sqlite"
                && conflict.resolution == "conflict_keep_target")
    );
    let connection = Connection::open(&global_target).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM values_table", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "replacement");
    assert!(
        !WorkspaceStateLayout::resolve(&data_root, &workspace)
            .workspace_dir
            .join("product.sqlite")
            .exists(),
        "ProductStore is not duplicated under a workspace directory"
    );
}

#[test]
fn data_root_inside_workspace_is_rejected_before_source_inspection() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let error = run_state_migration(&options(&workspace, &workspace.join("user-data")))
        .expect_err("a contract data root must not be nested in the workspace");
    assert!(error.to_string().contains("inside workspace"));
}

#[test]
fn legacy_root_symlink_escape_is_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let legacy = workspace.join(".rove");
    if !create_directory_symlink(&outside, &legacy) {
        return;
    }

    let error = run_state_migration(&options(&workspace, &tmp.path().join("data")))
        .expect_err("a symlinked legacy root must fail closed");
    assert!(error.to_string().contains("real directory"));
}

#[test]
fn target_nested_symlink_escape_is_rejected_before_copy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    std::fs::create_dir_all(&layout.workspace_dir).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let memory = layout.workspace_dir.join("memory");
    if !create_directory_symlink(&outside, &memory) {
        return;
    }

    let error = run_state_migration(&options(&workspace, &data_root))
        .expect_err("a target parent symlink must not receive migrated bytes");
    assert!(error.to_string().contains("must not be a symlink"));
    assert!(!outside.join("MEMORY.md").exists());
}

#[test]
fn migration_metadata_directory_symlink_is_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    layout.ensure().unwrap();
    let outside = tmp.path().join("outside-migration");
    std::fs::create_dir_all(&outside).unwrap();
    if !create_directory_symlink(&outside, &layout.workspace_dir.join(MIGRATION_DIR)) {
        return;
    }

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let error = run_state_migration(&applied)
        .expect_err("migration metadata must not follow a directory symlink");
    assert!(error.to_string().contains("must be a real directory"));
    assert_eq!(outside.read_dir().unwrap().count(), 0);
}

#[test]
fn held_migration_lock_fails_typed_and_keeps_the_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data");
    let layout = WorkspaceStateLayout::resolve(&data_root, &workspace);
    layout.ensure().unwrap();
    let migration_dir = layout.workspace_dir.join(MIGRATION_DIR);
    std::fs::create_dir_all(&migration_dir).unwrap();
    let lock_path = migration_dir.join(MIGRATION_LOCK_FILE);
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock_exclusive().unwrap();

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let error = run_state_migration(&applied).expect_err("the held lock must time out");
    assert!(
        matches!(error, StateMigrationError::Locked(ref path) if path.ends_with(Path::new(MIGRATION_DIR).join(MIGRATION_LOCK_FILE))),
        "unexpected lock error: {error:?}; expected {}",
        lock_path.display()
    );
    FileExt::unlock(&lock).unwrap();
    assert!(workspace.join(".rove/memory/MEMORY.md").is_file());
    assert!(!layout.workspace_dir.join("memory/MEMORY.md").exists());
}

#[test]
fn non_directory_data_root_fails_without_touching_the_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write_legacy_layout(&workspace);
    let data_root = tmp.path().join("data-root-file");
    std::fs::write(&data_root, "occupied").unwrap();

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    let error = run_state_migration(&applied)
        .expect_err("a non-directory data root must fail before copying");
    assert!(matches!(error, StateMigrationError::Io(_)));
    assert_eq!(std::fs::read_to_string(&data_root).unwrap(), "occupied");
    assert!(workspace.join(".rove/memory/MEMORY.md").is_file());
}

#[test]
fn relative_injected_data_root_fails_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let error = run_state_migration(&options(&workspace, Path::new("relative"))).unwrap_err();
    assert!(error.to_string().contains("absolute"));
}

#[test]
fn mcp_processes_digest_survives_catalog_relocation() {
    // This binary is separate from the in-crate tests, so mutating
    // ROVE_DATA_ROOT here cannot race the config test suite.
    unsafe {
        std::env::remove_var(rove_app_bootstrap::user_state::DATA_ROOT_ENV);
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let legacy = workspace.join(".rove");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("mcp_servers.json"),
        r#"{"servers":[{"name":"demo"}]}"#,
    )
    .unwrap();
    let data_root = tmp.path().join("data");

    let before = capability_digest_map(&workspace, None, None);

    unsafe {
        std::env::set_var(rove_app_bootstrap::user_state::DATA_ROOT_ENV, &data_root);
    }
    let pre_migration = capability_digest_map(&workspace, None, None);
    assert_eq!(
        before, pre_migration,
        "before migration the digest still reads the legacy catalog"
    );

    let mut applied = options(&workspace, &data_root);
    applied.apply = true;
    run_state_migration(&applied).unwrap();

    let after = capability_digest_map(&workspace, None, None);
    unsafe {
        std::env::remove_var(rove_app_bootstrap::user_state::DATA_ROOT_ENV);
    }
    assert_eq!(
        before, after,
        "byte-identical relocation must not invalidate mcp_processes grants"
    );
}
