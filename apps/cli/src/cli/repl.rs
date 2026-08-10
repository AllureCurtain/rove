use std::path::Path;

use crate::cli::render::{
    CliRunRenderContext, CliRunRenderMode, CliRunRenderOptions, render_run_events,
};
use crate::cli::runtime::CliRuntime;
use crate::cli::sessions;
use crate::cli::ui::{
    ReplStatusView, ReplWelcomeView, format_repl_help, format_repl_status, format_repl_welcome,
};
use crate::terminal::action::TerminalAction;
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::types::TerminationReason;
use rove_runtime::types::{JobId, RunId, SessionId, TaskState};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Status,
    Exit,
    Clear,
    Sessions,
    ResumeLatest,
    ResumeRun(String),
    Unknown(String),
}

impl SlashCommand {
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        let mut parts = trimmed.split_whitespace();
        let command = parts.next().unwrap_or_default();
        match command {
            "/help" => Self::Help,
            "/status" => Self::Status,
            "/exit" | "/quit" => Self::Exit,
            "/clear" => Self::Clear,
            "/sessions" => Self::Sessions,
            "/resume" => match parts.next() {
                Some("latest") => Self::ResumeLatest,
                Some(run_id) if !run_id.is_empty() => Self::ResumeRun(run_id.to_string()),
                _ => Self::Unknown("/resume".to_string()),
            },
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl SlashCommand {
    pub fn to_action(&self) -> TerminalAction {
        match self {
            Self::Help => TerminalAction::Help,
            Self::Status => TerminalAction::ShowStatus,
            Self::Exit => TerminalAction::Exit,
            Self::Clear => TerminalAction::Clear,
            Self::Sessions => TerminalAction::ShowSessions,
            Self::ResumeLatest => TerminalAction::ResumeLatest,
            Self::ResumeRun(run_id) => TerminalAction::ResumeRun(run_id.clone()),
            Self::Unknown(command) => TerminalAction::Unknown(command.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplState {
    session_id: SessionId,
    active_resume_state: Option<TaskState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplRunIdentity {
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
}

impl ReplState {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            active_resume_state: None,
        }
    }

    pub fn with_active_resume_state(mut self, active_resume_state: Option<TaskState>) -> Self {
        self.active_resume_state = active_resume_state;
        self
    }

    pub fn active_resume_state(&self) -> Option<&TaskState> {
        self.active_resume_state.as_ref()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn set_active_resume_state(&mut self, active_resume_state: Option<TaskState>) {
        self.active_resume_state = active_resume_state;
    }

    pub fn next_run_identity(&self) -> ReplRunIdentity {
        ReplRunIdentity {
            session_id: self.session_id,
            job_id: self
                .active_resume_state
                .as_ref()
                .map(|state| state.job_id)
                .unwrap_or_default(),
            run_id: RunId::new(),
        }
    }
}

pub async fn run(runtime: CliRuntime, initial_prompt: Option<String>) -> anyhow::Result<()> {
    let mut state = ReplState::new(SessionId::new());
    let session_label = repl_session_label(state.active_resume_state());
    eprintln!(
        "{}",
        format_repl_welcome(ReplWelcomeView {
            cwd: &runtime.workspace.root,
            model_id: runtime.engine.model_id(),
            session_label: &session_label,
            width: repl_welcome_width(),
        })
    );

    let history_path = runtime.workspace.state_dir.join("repl_history");
    let mut editor = DefaultEditor::new()?;
    load_history(&mut editor, &history_path);

    if let Some(initial_prompt) = initial_prompt {
        let input = initial_prompt.trim();
        if !input.is_empty() {
            if let Err(err) = editor.add_history_entry(input) {
                eprintln!("warning: failed to record REPL history: {err}");
            }
            run_prompt(input.to_string(), &runtime, &mut state).await?;
            save_history(&mut editor, &history_path);
        }
    }

    loop {
        match editor.readline("rove> ") {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }
                if input.starts_with('/') {
                    if handle_slash_command(input, &runtime, &mut state, &mut editor, &history_path)
                        .await?
                    {
                        save_history(&mut editor, &history_path);
                        return Ok(());
                    }
                    save_history(&mut editor, &history_path);
                    continue;
                }
                if let Err(err) = editor.add_history_entry(input) {
                    eprintln!("warning: failed to record REPL history: {err}");
                }
                run_prompt(input.to_string(), &runtime, &mut state).await?;
                save_history(&mut editor, &history_path);
            }
            Err(ReadlineError::Interrupted) => {
                eprintln!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                save_history(&mut editor, &history_path);
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn repl_session_label(active_resume_state: Option<&TaskState>) -> String {
    match active_resume_state {
        Some(state) => format!(
            "resumed {}",
            crate::cli::ui::short_id(state.run_id.to_string())
        ),
        None => "new".to_string(),
    }
}

fn repl_welcome_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

async fn run_prompt(
    message: String,
    runtime: &CliRuntime,
    state: &mut ReplState,
) -> anyhow::Result<()> {
    let identity = state.next_run_identity();
    let run_handle =
        runtime
            .state_store
            .start_run(identity.session_id, identity.job_id, identity.run_id)?;
    tracing::info!(%run_handle.run_id, "Starting REPL run");

    let resume_state = state.active_resume_state().cloned();
    let req = run_handle.request(message.clone(), resume_state.clone());
    let trace_writer = run_handle.trace_writer.clone();
    let run_cancel = CancellationToken::new();
    let signal_task = spawn_repl_run_signal_listener(run_cancel.clone());
    let stream = runtime
        .engine
        .run_with_cancel(req, Some(trace_writer), run_cancel);
    let runtime_identity = Some(stream.runtime_identity().clone());
    let agent_profile = stream.agent_profile().cloned();
    let termination = render_run_events(
        stream,
        CliRunRenderContext {
            message,
            run: run_handle,
            resume_state,
            state_store: &runtime.state_store,
            workspace: &runtime.workspace,
            model_id: runtime.engine.model_id(),
            runtime_identity,
            agent_profile,
        },
        CliRunRenderOptions {
            mode: CliRunRenderMode::ReplCompact,
            ..CliRunRenderOptions::default()
        },
    )
    .await;
    signal_task.abort();

    if !matches!(termination, TerminationReason::Cancelled)
        && let Ok(latest) = runtime.state_store.load_task_state(identity.run_id).await
    {
        state.set_active_resume_state(Some(latest));
    }
    Ok(())
}

async fn handle_slash_command(
    input: &str,
    runtime: &CliRuntime,
    state: &mut ReplState,
    editor: &mut DefaultEditor,
    history_path: &Path,
) -> anyhow::Result<bool> {
    match SlashCommand::parse(input) {
        SlashCommand::Help => {
            eprint!("{}", format_repl_help());
        }
        SlashCommand::Status => {
            eprintln!(
                "{}",
                format_repl_status(ReplStatusView {
                    workspace: &runtime.workspace,
                    config: &runtime.config,
                    model_id: runtime.engine.model_id(),
                    session_id: state.session_id(),
                    active_resume_state: state.active_resume_state(),
                })
            );
        }
        SlashCommand::Exit => return Ok(true),
        SlashCommand::Clear => clear_screen(),
        SlashCommand::Sessions => {
            let states = runtime.state_store.list_task_states().await?;
            print!("{}", sessions::format_task_states(&states));
        }
        SlashCommand::ResumeLatest => {
            match resolve_resume_state(&runtime.state_store, Some("latest")).await {
                Ok(Some(resume_state)) => {
                    eprintln!("resumed latest run {}", resume_state.run_id);
                    state.set_active_resume_state(Some(resume_state));
                }
                Ok(None) => eprintln!("no task state found to resume"),
                Err(err) => eprintln!("resume failed: {err}"),
            }
        }
        SlashCommand::ResumeRun(run_id) => {
            match resolve_resume_state(&runtime.state_store, Some(&run_id)).await {
                Ok(Some(resume_state)) => {
                    eprintln!("resumed run {}", resume_state.run_id);
                    state.set_active_resume_state(Some(resume_state));
                }
                Ok(None) => eprintln!("no task state found to resume"),
                Err(err) => eprintln!("resume failed: {err}"),
            }
        }
        SlashCommand::Unknown(command) => {
            eprintln!("unknown command `{command}`; type /help for commands");
        }
    }
    save_history(editor, history_path);
    Ok(false)
}

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn load_history(editor: &mut DefaultEditor, history_path: &Path) {
    if !history_path.exists() {
        if let Some(parent) = history_path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!("warning: failed to create REPL history directory: {err}");
        }
        if let Err(err) = std::fs::write(history_path, "") {
            eprintln!("warning: failed to create REPL history: {err}");
        }
    }
    if let Err(err) = editor.load_history(history_path) {
        eprintln!("warning: failed to load REPL history: {err}");
    }
}

fn save_history(editor: &mut DefaultEditor, history_path: &Path) {
    if let Err(err) = editor.save_history(history_path) {
        eprintln!("warning: failed to save REPL history: {err}");
    }
}

fn spawn_repl_run_signal_listener(cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            match signal(SignalKind::terminate()) {
                Ok(mut terminate) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => cancel.cancel(),
                        _ = terminate.recv() => std::process::exit(143),
                    }
                }
                Err(err) => {
                    tracing::warn!("failed to install SIGTERM handler: {err}");
                    let _ = tokio::signal::ctrl_c().await;
                    cancel.cancel();
                }
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                tracing::warn!("failed to listen for Ctrl+C: {err}");
            }
            cancel.cancel();
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::terminal::action::TerminalAction;
    use rove_runtime::types::{JobId, RunId, SessionId, TaskState};

    use super::{ReplState, SlashCommand};

    #[test]
    fn slash_command_parser_recognizes_first_pass_commands() {
        assert_eq!(SlashCommand::parse("/help"), SlashCommand::Help);
        assert_eq!(SlashCommand::parse("/status"), SlashCommand::Status);
        assert_eq!(SlashCommand::parse("/exit"), SlashCommand::Exit);
        assert_eq!(SlashCommand::parse("/quit"), SlashCommand::Exit);
        assert_eq!(SlashCommand::parse("/clear"), SlashCommand::Clear);
        assert_eq!(SlashCommand::parse("/sessions"), SlashCommand::Sessions);
        assert_eq!(
            SlashCommand::parse("/resume latest"),
            SlashCommand::ResumeLatest
        );
        assert_eq!(
            SlashCommand::parse("/resume 01ARYZ6S41YYYYYYYYYYYYYYYY"),
            SlashCommand::ResumeRun("01ARYZ6S41YYYYYYYYYYYYYYYY".to_string())
        );
        assert_eq!(
            SlashCommand::parse("/model gpt"),
            SlashCommand::Unknown("/model".to_string())
        );
    }

    #[test]
    fn slash_commands_convert_to_terminal_actions() {
        assert_eq!(
            SlashCommand::parse("/help").to_action(),
            TerminalAction::Help
        );
        assert_eq!(
            SlashCommand::parse("/status").to_action(),
            TerminalAction::ShowStatus
        );
        assert_eq!(
            SlashCommand::parse("/clear").to_action(),
            TerminalAction::Clear
        );
        assert_eq!(
            SlashCommand::parse("/sessions").to_action(),
            TerminalAction::ShowSessions
        );
        assert_eq!(
            SlashCommand::parse("/resume latest").to_action(),
            TerminalAction::ResumeLatest
        );
        assert_eq!(
            SlashCommand::parse("/resume 01ARYZ6S41").to_action(),
            TerminalAction::ResumeRun("01ARYZ6S41".to_string())
        );
        assert_eq!(
            SlashCommand::parse("/exit").to_action(),
            TerminalAction::Exit
        );
        assert_eq!(
            SlashCommand::parse("/model gpt").to_action(),
            TerminalAction::Unknown("/model".to_string())
        );
    }

    #[test]
    fn repl_state_uses_previous_task_identity_for_follow_up() {
        let session_id = SessionId::new();
        let first = ReplState::new(session_id);
        let first_identity = first.next_run_identity();
        let completed = task_state(session_id, first_identity.job_id, first_identity.run_id);
        let resumed = first.with_active_resume_state(Some(completed.clone()));
        let next_identity = resumed.next_run_identity();

        assert_eq!(next_identity.session_id, session_id);
        assert_eq!(next_identity.job_id, completed.job_id);
        assert_ne!(next_identity.run_id, completed.run_id);
    }

    fn task_state(session_id: SessionId, job_id: JobId, run_id: RunId) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id,
            job_id,
            run_id,
            goal: "hello".to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: None,
            plan: None,
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        }
    }
}
