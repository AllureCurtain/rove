use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use clap::Parser;
use rove_app_bootstrap::AppConfigOverrides;
use rove_cli::cli::args::{Args, Command};
use rove_cli::cli::config as cli_config;
use rove_cli::cli::exec::run_exec_with_cancel;
use rove_cli::cli::repl;
use rove_cli::cli::runtime::{CliRuntimeInteraction, CliRuntimeOptions, build_cli_runtime};
use rove_cli::cli::sessions;
use rove_cli::cli::state as cli_state;
use rove_cli::cli::trust as cli_trust;
use rove_cli::tui::app as tui_app;
use rove_cli::tui::providers::TuiInteractionBroker;
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::types::{RunId, TerminationReason};
use tokio_util::sync::CancellationToken;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.is_tui());

    if args.is_sync_fast_path() {
        return run_sync_fast_path(args);
    }

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async_main(args))
}

fn init_tracing(tui_mode: bool) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    if tui_mode {
        tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(env_filter)
            .init();
    }
}

fn run_sync_fast_path(args: Args) -> anyhow::Result<()> {
    match args.command {
        Some(Command::DumpConfig) => cli_config::run(
            args.cwd.clone().map(PathBuf::from),
            AppConfigOverrides {
                model: args.model.clone(),
                max_steps: args.max_steps,
                api_bind_addr: None,
                trust_project: args.trust_project,
            },
        ),
        _ => Ok(()),
    }
}

async fn async_main(args: Args) -> anyhow::Result<()> {
    match args.command.clone() {
        Some(Command::Sessions) => return sessions::run(args.cwd.clone()).await,
        Some(Command::State { command }) => return cli_state::run(args.cwd.clone(), command).await,
        Some(Command::Trust { command }) => return cli_trust::run(args.cwd.clone(), command),
        Some(Command::Tui) => {
            let (interaction, interaction_rx) = TuiInteractionBroker::default().into_parts();
            let runtime = build_runtime_with_interaction(
                &args,
                None,
                CliRuntimeInteraction::Providers {
                    input_provider: Some(interaction.input_provider),
                    approval_provider: Some(interaction.approval_provider),
                },
            )
            .await?;
            let resume_state =
                resolve_resume_state(&runtime.state_store, args.resume.as_deref()).await?;
            return tui_app::run(runtime, resume_state, interaction_rx).await;
        }
        Some(Command::Exec { message }) => {
            let message = join_message(message);
            let runtime = build_runtime(&args, Some(&message)).await?;
            return run_exec(args, runtime, message).await;
        }
        Some(Command::DumpConfig) => unreachable!("dump-config is handled before runtime startup"),
        None => {}
    }

    let message = args.message();
    let runtime = build_runtime(&args, message.as_ref()).await?;

    repl::run(runtime, message).await
}

async fn build_runtime(
    args: &Args,
    fake_message: Option<&String>,
) -> anyhow::Result<rove_cli::cli::runtime::CliRuntime> {
    build_runtime_with_interaction(args, fake_message, CliRuntimeInteraction::default()).await
}

async fn build_runtime_with_interaction(
    args: &Args,
    fake_message: Option<&String>,
    interaction: CliRuntimeInteraction,
) -> anyhow::Result<rove_cli::cli::runtime::CliRuntime> {
    build_cli_runtime(CliRuntimeOptions {
        cwd: args.cwd.clone().map(PathBuf::from),
        model: args.model.clone(),
        max_steps: args.max_steps,
        trust_project: args.trust_project,
        approval: args.approval,
        task_workspace: args.task_workspace.clone(),
        task_base: args.task_base.clone(),
        initial_fake_response: fake_message.map(|message| format!("fake response: {message}")),
        interaction,
    })
    .await
}

fn join_message(message: Vec<String>) -> String {
    message.join(" ").trim().to_string()
}

async fn run_exec(
    args: Args,
    runtime: rove_cli::cli::runtime::CliRuntime,
    message: String,
) -> anyhow::Result<()> {
    let resume_state = resolve_resume_state(&runtime.state_store, args.resume.as_deref()).await?;
    let run_id = RunId::new();
    let run_handle = runtime.state_store.start_run(
        resume_state
            .as_ref()
            .map(|state| state.session_id)
            .unwrap_or_default(),
        resume_state
            .as_ref()
            .map(|state| state.job_id)
            .unwrap_or_default(),
        run_id,
    )?;

    tracing::info!(%run_handle.run_id, "Starting exec run");

    let cli_cancel = CancellationToken::new();
    let signal_exit_code = spawn_cli_signal_listener(cli_cancel.clone());
    let termination = run_exec_with_cancel(
        &runtime.engine,
        message,
        run_handle,
        resume_state,
        &runtime.state_store,
        cli_cancel,
    )
    .await;
    if matches!(termination, TerminationReason::Cancelled) {
        std::process::exit(signal_exit_code.load(Ordering::SeqCst));
    }

    Ok(())
}

fn spawn_cli_signal_listener(cancel: CancellationToken) -> Arc<AtomicI32> {
    let exit_code = Arc::new(AtomicI32::new(130));
    let exit_code_for_task = exit_code.clone();
    tokio::spawn(async move {
        let code = wait_for_cli_signal().await;
        exit_code_for_task.store(code, Ordering::SeqCst);
        cancel.cancel();
    });
    exit_code
}

#[cfg(unix)]
async fn wait_for_cli_signal() -> i32 {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => 130,
                _ = terminate.recv() => 143,
            }
        }
        Err(err) => {
            tracing::warn!("failed to install SIGTERM handler: {err}");
            let _ = tokio::signal::ctrl_c().await;
            130
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_cli_signal() -> i32 {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::warn!("failed to listen for Ctrl+C: {err}");
    }
    130
}
