use std::path::Path;

use crate::config::AppConfig;
use crate::core::types::TaskState;
use crate::core::workspace::{Workspace, WorkspaceKind};

pub struct ReplStatusView<'a> {
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub model_id: &'a str,
    pub active_resume_state: Option<&'a TaskState>,
}

pub fn format_repl_status(view: ReplStatusView<'_>) -> String {
    let workspace_kind = workspace_kind_label(&view.workspace.kind);
    let workspace_root = display_absolute_path(&view.workspace.root);
    let state = display_path(&view.workspace.state_dir, &view.workspace.root);
    let provider = provider_label(&view);
    let session = match view.active_resume_state {
        Some(state) => format!("resumed {}", short_id(state.run_id.to_string())),
        None => "new".to_string(),
    };

    format!(
        "\
rove
local-first agent runtime
workspace  {workspace_kind}  {workspace_root}
model      {model}
provider   {provider}
state      {state}  ·  session {session}

{commands}
",
        model = truncate_middle(view.model_id, 96),
        provider = provider,
        commands = command_hint_line(),
    )
}

pub fn format_repl_help() -> String {
    "\
Commands:
  /help             show this help
  /status           show workspace, model, provider, state, and session
  /exit, /quit      exit the REPL
  /clear            clear the terminal
  /sessions         list resumable task states
  /resume latest    resume the latest task state
  /resume <run_id>  resume a specific task state
"
    .to_string()
}

pub fn command_hint_line() -> &'static str {
    "/help  /sessions  /resume latest  /status  /clear  /exit"
}

pub fn short_id(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if value.len() <= 10 {
        value.to_string()
    } else {
        value.chars().take(10).collect()
    }
}

pub fn truncate_middle(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let left = (max_chars - 1) / 2;
    let right = max_chars - 1 - left;
    let prefix: String = value.chars().take(left).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(right)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn display_path(path: &Path, workspace_root: &Path) -> String {
    let display = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    clean_windows_verbatim_prefix(&display)
}

fn display_absolute_path(path: &Path) -> String {
    clean_windows_verbatim_prefix(&path.to_string_lossy())
}

fn clean_windows_verbatim_prefix(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = path.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        path.to_string()
    }
}

fn provider_label(view: &ReplStatusView<'_>) -> String {
    if view.model_id == "fake" {
        "fake".to_string()
    } else {
        view.config.provider.name.clone()
    }
}

fn workspace_kind_label(kind: &WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Folder => "folder",
        WorkspaceKind::Repo => "repo",
        WorkspaceKind::Task => "task",
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::config::AppConfig;
    use crate::core::workspace::Workspace;

    use super::{ReplStatusView, format_repl_help, format_repl_status, short_id};

    #[test]
    fn repl_status_includes_runtime_context_and_commands() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut config = AppConfig::default();
        config.provider.name = "openai-compatible".to_string();
        config.provider.model = "test-model".to_string();

        let output = format_repl_status(ReplStatusView {
            workspace: &workspace,
            config: &config,
            model_id: "test-model",
            active_resume_state: None,
        });

        assert!(output.contains("rove"));
        assert!(output.contains("local-first agent runtime"));
        assert!(output.contains("workspace"));
        assert!(output.contains("folder"));
        assert!(output.contains("model"));
        assert!(output.contains("test-model"));
        assert!(output.contains("provider"));
        assert!(output.contains("openai-compatible"));
        assert!(output.contains("state"));
        assert!(output.contains("session"));
        assert!(output.contains("new"));
        assert!(output.contains("/status"));
        assert!(output.contains("/resume latest"));
    }

    #[test]
    fn repl_status_cleans_windows_verbatim_paths_and_fake_provider() {
        let workspace = Workspace {
            root: std::path::PathBuf::from(r"\\?\C:\Users\AllureLove\repo"),
            kind: crate::core::workspace::WorkspaceKind::Repo,
            state_dir: std::path::PathBuf::from(r"\\?\C:\Users\AllureLove\repo\.rove"),
        };
        let config = AppConfig::default();

        let output = format_repl_status(ReplStatusView {
            workspace: &workspace,
            config: &config,
            model_id: "fake",
            active_resume_state: None,
        });

        assert!(!output.contains(r"\\?\"));
        assert!(output.contains(r"C:\Users\AllureLove\repo"));
        assert!(output.contains("provider   fake"));
    }

    #[test]
    fn repl_help_lists_status_command() {
        let output = format_repl_help();

        assert!(output.contains("/help"));
        assert!(output.contains("/status"));
        assert!(output.contains("/sessions"));
        assert!(output.contains("/resume latest"));
        assert!(output.contains("/exit"));
    }

    #[test]
    fn short_id_keeps_short_values_and_truncates_long_values() {
        assert_eq!(short_id("01ABC"), "01ABC");
        assert_eq!(short_id("01ARYZ6S41YYYYYYYYYYYYYYYY"), "01ARYZ6S41");
    }
}
