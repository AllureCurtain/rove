use std::path::PathBuf;

use rove_app_bootstrap::state_migration::{
    ConflictPolicy, DEFAULT_MAX_MIGRATION_BYTES, MigrationOptions, run_state_migration,
};
use rove_app_bootstrap::{AppConfig, AppConfigOverrides, WorkspaceStateLayout};
use rove_runtime::state::index::CleanupResult;
use rove_runtime::state::store::{RepairResult, StateStore};
use rove_runtime::workspace::Workspace;
use serde::Serialize;

use crate::cli::args::{CliStateConflictPolicy, StateCommand};

pub async fn run(cwd: Option<String>, command: StateCommand) -> anyhow::Result<()> {
    let cwd = cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    let config = AppConfig::load(&workspace.root, AppConfigOverrides::default())?;

    match command {
        StateCommand::Repair | StateCommand::Cleanup => {
            let state_store = StateStore::with_index_path(
                &config.state_dir(),
                config.sqlite_path(),
                config.state.sqlite_busy_timeout_ms,
            );
            match command {
                StateCommand::Repair => {
                    let result = state_store.repair_index().await?;
                    print!("{}", format_repair_result(&result));
                }
                StateCommand::Cleanup => {
                    let result = state_store.cleanup_expired().await?;
                    print!("{}", format_cleanup_result(&result));
                }
                _ => unreachable!("handled above"),
            }
        }
        StateCommand::Paths => {
            let report = build_state_paths_report(&workspace, &config)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        StateCommand::Migrate {
            apply,
            on_conflict,
            prune_legacy,
            max_bytes,
            data_root,
        } => {
            let options = MigrationOptions {
                workspace_root: workspace.root.clone(),
                data_root: data_root.or_else(|| {
                    config
                        .user_state_roots
                        .as_ref()
                        .map(|roots| roots.root().to_path_buf())
                }),
                on_conflict: match on_conflict {
                    Some(CliStateConflictPolicy::BackupTarget) => ConflictPolicy::BackupTarget,
                    Some(CliStateConflictPolicy::KeepTarget) | None => ConflictPolicy::KeepTarget,
                },
                max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_MIGRATION_BYTES),
                prune_legacy,
                apply,
            };
            let report = run_state_migration(&options)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            let unresolved = report
                .conflicts
                .iter()
                .filter(|conflict| conflict.resolution == "conflict_keep_target")
                .count();
            if unresolved > 0 {
                // Mirror `provider migrate`: a remaining conflict is a
                // non-zero exit with a typed code, never a silent PASS.
                let resolution = if report
                    .conflicts
                    .iter()
                    .any(|conflict| conflict.resolution == "conflict_keep_target")
                {
                    "rerun with --on-conflict backup-target to replace differing targets, or resolve them manually"
                } else {
                    "review the conflicts before pruning legacy state"
                };
                anyhow::bail!(
                    "state_migration_conflict: {} conflict(s) remain; {resolution}",
                    unresolved
                );
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct StatePathsReport {
    schema_version: i64,
    workspace: StatePathsWorkspace,
    user_roots: StatePathsUserRoots,
    resolved_paths: StatePathsResolved,
    legacy: StatePathsLegacy,
}

#[derive(Serialize)]
struct StatePathsWorkspace {
    root: String,
    kind: String,
    storage_key: String,
}

#[derive(Serialize)]
struct StatePathsUserRoots {
    data_root: String,
    data_root_source: String,
    config_root: String,
}

#[derive(Serialize)]
struct StatePathsResolved {
    state_dir: String,
    state_sqlite: String,
    product_sqlite: String,
    mcp_catalog: String,
    memory_durable_dir: String,
    memory_session_dir: String,
    runs_dir: String,
    tasks_base: String,
    project_config: String,
}

#[derive(Serialize)]
struct StatePathsLegacy {
    present: bool,
    dir: String,
    migration_receipt_present: bool,
}

fn build_state_paths_report(
    workspace: &Workspace,
    config: &AppConfig,
) -> anyhow::Result<StatePathsReport> {
    let roots = config.user_state_roots.as_ref().ok_or_else(|| {
        anyhow::anyhow!("state_paths_unavailable: user state roots are not pinned")
    })?;
    let layout = WorkspaceStateLayout::resolve(roots.root(), &workspace.root);
    let legacy_dir = workspace.root.join(".rove");
    let display = |path: &std::path::Path| path.to_string_lossy().replace('\\', "/");
    Ok(StatePathsReport {
        schema_version: 1,
        workspace: StatePathsWorkspace {
            root: display(&workspace.root),
            kind: match workspace.kind {
                rove_runtime::workspace::WorkspaceKind::Repo => "repo".to_string(),
                rove_runtime::workspace::WorkspaceKind::Folder => "folder".to_string(),
                rove_runtime::workspace::WorkspaceKind::Task => "task".to_string(),
            },
            storage_key: layout.storage_key.clone(),
        },
        user_roots: StatePathsUserRoots {
            data_root: display(roots.root()),
            data_root_source: roots.override_source().unwrap_or("platform").to_string(),
            config_root: display(
                config
                    .source_summary
                    .user_config_path
                    .parent()
                    .unwrap_or(std::path::Path::new("")),
            ),
        },
        resolved_paths: StatePathsResolved {
            state_dir: display(&config.state_dir()),
            state_sqlite: display(&config.sqlite_path()),
            product_sqlite: display(&config.product_sqlite_path()),
            mcp_catalog: display(&config.effective_mcp_config_path()),
            memory_durable_dir: display(&config.memory_durable_dir()),
            memory_session_dir: display(&config.memory_session_dir()),
            runs_dir: display(&layout.runs_dir),
            tasks_base: display(&layout.tasks_base),
            project_config: display(&config.source_summary.project_config_path),
        },
        legacy: StatePathsLegacy {
            present: legacy_dir.is_dir(),
            dir: display(&legacy_dir),
            migration_receipt_present: legacy_dir.join(".rove-migration-receipt.json").is_file(),
        },
    })
}

pub fn format_repair_result(result: &RepairResult) -> String {
    format!(
        "state repair complete: imported {} task state artifact(s), {} trace event(s), {} report artifact(s); skipped {} corrupted trace line(s)\n",
        result.task_state_count,
        result.event_count,
        result.report_count,
        result.corrupt_trace_line_count
    )
}

pub fn format_cleanup_result(result: &CleanupResult) -> String {
    format!(
        "state cleanup complete: removed {} job(s), {} run(s), {} task state row(s)\n",
        result.job_count, result.run_count, result.task_state_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_repair_result_reports_import_count() {
        let result = RepairResult {
            task_state_count: 2,
            event_count: 3,
            report_count: 1,
            corrupt_trace_line_count: 4,
        };

        let output = format_repair_result(&result);

        assert!(output.contains("imported 2 task state artifact(s)"));
        assert!(output.contains("3 trace event(s)"));
        assert!(output.contains("1 report artifact(s)"));
        assert!(output.contains("skipped 4 corrupted trace line(s)"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn format_cleanup_result_reports_removed_counts() {
        let result = CleanupResult {
            job_count: 2,
            run_count: 2,
            task_state_count: 1,
        };

        let output = format_cleanup_result(&result);

        assert!(output.contains("removed 2 job(s)"));
        assert!(output.contains("2 run(s)"));
        assert!(output.contains("1 task state row(s)"));
        assert!(output.ends_with('\n'));
    }
}
