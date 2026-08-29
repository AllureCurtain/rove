use std::path::Path;

use rove_app_bootstrap::AppConfig;
use rove_runtime::types::{SessionId, TaskState};
use rove_runtime::workspace::{Workspace, WorkspaceKind};

pub struct ReplStatusView<'a> {
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub model_id: &'a str,
    pub session_id: SessionId,
    pub active_resume_state: Option<&'a TaskState>,
}

pub struct ReplWelcomeView<'a> {
    pub cwd: &'a Path,
    pub model_id: &'a str,
    pub session_label: &'a str,
    pub width: usize,
}

pub fn format_repl_welcome(view: ReplWelcomeView<'_>) -> String {
    let width = view.width.max(24);
    if width < 44 {
        return format_compact_welcome(view, width);
    }

    let block_width = width.min(56);
    let cwd = truncate_start(
        &display_absolute_path(view.cwd),
        block_width.saturating_sub(2),
    );
    let model_width = block_width.saturating_sub(24).max(8);
    let model = truncate_end(view.model_id, model_width);
    let session = truncate_end(view.session_label, block_width.saturating_sub(42).max(3));

    format!(
        "\
  R O V E
  local-first agent runtime

  {cwd}

  model   {model:<model_width$}  session  {session}
  mode    interactive{mode_pad}status   ready

  Type your task, or use /help for commands.
",
        mode_pad = " ".repeat(model_width.saturating_sub("interactive".len()) + 2),
    )
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
    let active_run = view
        .active_resume_state
        .map(|state| state.run_id.to_string())
        .unwrap_or_else(|| "none".to_string());
    let active_job = view
        .active_resume_state
        .map(|state| state.job_id.to_string())
        .unwrap_or_else(|| "none".to_string());
    let memory_paths = view.config.memory_paths();
    let session_memory_path = memory_paths
        .session_dir
        .join(format!("{}.md", view.session_id));
    let session_memory = display_path(&session_memory_path, &view.workspace.root);

    format!(
        "\
rove
local-first agent runtime
workspace  {workspace_kind}  {workspace_root}
model      {model}
provider   {provider}
state      {state}  ·  session {session}
session id {session_id}
active    run {active_run}  ·  job {active_job}
memory    {session_memory}

{commands}
",
        model = truncate_middle(view.model_id, 96),
        provider = provider,
        session_id = view.session_id,
        commands = command_hint_line(),
    )
}

pub fn format_repl_help() -> String {
    "\
Commands:
  /help             show this help
  /status           show workspace, model, provider, state, session, run, and memory
  /exit, /quit      exit the REPL
  /clear            clear the terminal
  /sessions         list resumable task states
  /compact          replace the active session history with a summary
  /resume latest    resume the latest task state
  /resume <run_id>  resume a specific task state
"
    .to_string()
}

pub fn command_hint_line() -> &'static str {
    "/help  /sessions  /compact  /resume latest  /status  /clear  /exit"
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

fn format_compact_welcome(view: ReplWelcomeView<'_>, width: usize) -> String {
    let content_width = width.saturating_sub(2).max(8);
    let cwd = truncate_start(&display_absolute_path(view.cwd), content_width);
    let model = truncate_end(view.model_id, content_width.saturating_sub(6).max(3));
    let session = truncate_end(view.session_label, content_width.saturating_sub(8).max(3));
    let hint = truncate_end("Type your task, or use /help.", content_width);

    format!(
        "\
R O V E
local-first agent runtime
{cwd}
model {model}
session {session}
mode interactive  status ready
{hint}
"
    )
}

fn truncate_start(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let tail_len = max_chars - 3;
    let tail: String = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}")
}

fn truncate_end(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let prefix: String = value.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
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
    } else if let Some(active) = view.config.provider.active.as_deref() {
        view.config
            .provider
            .profiles
            .get(active)
            .map(|profile| profile.provider_type.clone())
            .unwrap_or_else(|| active.to_string())
    } else {
        "unknown".to_string()
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

    use rove_app_bootstrap::AppConfig;
    use rove_runtime::workspace::Workspace;

    use rove_runtime::types::SessionId;

    use super::{
        ReplStatusView, ReplWelcomeView, format_repl_help, format_repl_status, format_repl_welcome,
        short_id,
    };

    #[test]
    fn repl_welcome_wide_layout_contains_compact_startup_context() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();

        let output = format_repl_welcome(ReplWelcomeView {
            cwd: workspace.root.as_path(),
            model_id: "qwen/qwen3-coder",
            session_label: "new",
            width: 80,
        });

        assert!(output.contains("R O V E"));
        assert!(output.contains("local-first agent runtime"));
        assert!(output.contains(tmp.path().file_name().unwrap().to_string_lossy().as_ref()));
        assert!(output.contains("model   qwen/qwen3-coder"));
        assert!(output.contains("session  new"));
        assert!(output.contains("mode    interactive"));
        assert!(output.contains("status   ready"));
        assert!(output.contains("Type your task, or use /help for commands."));
        assert!(!output.contains("provider"));
        assert!(!output.contains("session id"));
        assert!(!output.contains("memory"));
    }

    #[test]
    fn repl_welcome_truncates_long_windows_paths_from_the_start() {
        let cwd = std::path::Path::new(
            r"C:\Users\AllureLove\Documents\Projects\Deeply\Nested\Workspace\rove",
        );

        let output = format_repl_welcome(ReplWelcomeView {
            cwd,
            model_id: "test-model",
            session_label: "resumed 01ARYZ6S41",
            width: 56,
        });

        assert!(output.contains("..."));
        assert!(output.contains(r"Nested\Workspace\rove"));
        assert!(!output.contains(r"C:\Users\AllureLove\Documents\Projects\Deeply"));
    }

    #[test]
    fn repl_welcome_narrow_layout_is_compact_and_bounded() {
        let cwd = std::path::Path::new(r"C:\Users\AllureLove\repo\rove");

        let output = format_repl_welcome(ReplWelcomeView {
            cwd,
            model_id: "qwen/qwen3-coder-with-a-long-suffix",
            session_label: "new",
            width: 34,
        });

        assert!(output.contains("R O V E"));
        assert!(output.contains("mode interactive  status ready"));
        assert!(output.contains("model qwen/qwen3"));
        assert!(output.contains("session new"));
        assert!(output.contains("/help"));
        assert!(!output.contains("model   "));
        assert!(output.lines().all(|line| line.chars().count() <= 34));
    }

    #[test]
    fn repl_status_includes_runtime_context_and_commands() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut config = AppConfig::default();
        config.provider.active = Some("openai".to_string());
        config.provider.profiles.insert(
            "openai".to_string(),
            rove_app_bootstrap::ProviderProfileConfig {
                label: None,
                provider_type: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                model: "test-model".to_string(),
                auth: rove_app_bootstrap::ProviderAuthConfig::None,
                headers: Default::default(),
                options: Default::default(),
                protocol_options: serde_json::json!({}),
            },
        );
        config.provider.model = "test-model".to_string();
        let session_id = SessionId::new();

        let output = format_repl_status(ReplStatusView {
            workspace: &workspace,
            config: &config,
            model_id: "test-model",
            session_id,
            active_resume_state: None,
        });

        assert!(output.contains("rove"));
        assert!(output.contains("local-first agent runtime"));
        assert!(output.contains("workspace"));
        assert!(output.contains("folder"));
        assert!(output.contains("model"));
        assert!(output.contains("test-model"));
        assert!(output.contains("provider"));
        assert!(output.contains("openai"));
        assert!(output.contains("state"));
        assert!(output.contains("session"));
        assert!(output.contains("new"));
        assert!(output.contains(&session_id.to_string()));
        assert!(output.contains("active"));
        assert!(output.contains("memory"));
        assert!(output.contains("memory/sessions"));
        assert!(output.contains("/status"));
        assert!(output.contains("/resume latest"));
    }

    #[test]
    fn repl_status_cleans_windows_verbatim_paths_and_fake_provider() {
        let workspace = Workspace {
            root: std::path::PathBuf::from(r"\\?\C:\Users\AllureLove\repo"),
            kind: rove_runtime::workspace::WorkspaceKind::Repo,
            state_dir: std::path::PathBuf::from(r"\\?\C:\Users\AllureLove\repo\.rove"),
        };
        let config = AppConfig::default();

        let output = format_repl_status(ReplStatusView {
            workspace: &workspace,
            config: &config,
            model_id: "fake",
            session_id: SessionId::new(),
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
