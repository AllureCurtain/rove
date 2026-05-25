use std::path::PathBuf;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::workspace::Workspace;
use crate::interfaces::cli::args::StateCommand;
use crate::state::index::CleanupResult;
use crate::state::store::{RepairResult, StateStore};

pub async fn run(cwd: Option<String>, command: StateCommand) -> anyhow::Result<()> {
    let cwd = cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    let config = AppConfig::load(&workspace.root, AppConfigOverrides::default())?;
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
    }
    Ok(())
}

pub fn format_repair_result(result: &RepairResult) -> String {
    format!(
        "state repair complete: imported {} task state artifact(s)\n",
        result.task_state_count
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
        };

        let output = format_repair_result(&result);

        assert!(output.contains("imported 2 task state artifact(s)"));
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
