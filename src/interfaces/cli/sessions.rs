use std::path::PathBuf;

use crate::core::types::TaskState;
use crate::core::workspace::Workspace;
use crate::state::store::StateStore;

pub async fn run(cwd: Option<String>) -> anyhow::Result<()> {
    let cwd = cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    let state_store = StateStore::new(&workspace.state_dir);
    let states = state_store.list_task_states().await?;
    println!("{}", format_task_states(&states));
    Ok(())
}

pub fn format_task_states(states: &[TaskState]) -> String {
    if states.is_empty() {
        return "No resumable task states found.\n".to_string();
    }

    let mut output = String::from(
        "run_id                           session_id                       job_id                            step  goal\n",
    );
    for state in states {
        output.push_str(&format!(
            "{:<32} {:<32} {:<32} step {:<3} {}\n",
            state.run_id,
            state.session_id,
            state.job_id,
            state.step,
            compact(&state.goal, 80),
        ));
        if let Some(summary) = state.summary.as_deref() {
            output.push_str(&format!("  summary: {}\n", compact(summary, 100)));
        }
    }

    output
}

fn compact(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
