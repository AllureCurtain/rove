use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use clap::Parser;
use rove::config::AppConfigOverrides;
use rove::core::types::{RunId, TerminationReason};
use rove::interfaces::cli::args::{Args, Command};
use rove::interfaces::cli::config as cli_config;
use rove::interfaces::cli::exec::run_exec_with_cancel;
use rove::interfaces::cli::index::{self as cli_index, IndexOptions};
use rove::interfaces::cli::repl;
use rove::interfaces::cli::runtime::{CliRuntimeOptions, build_cli_runtime};
use rove::interfaces::cli::sessions;
use rove::interfaces::cli::state as cli_state;
use rove::state::resume::resolve_resume_state;
use tokio_util::sync::CancellationToken;

fn main() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .init();

    let args = Args::parse();

    if args.is_sync_fast_path() {
        return run_sync_fast_path(args);
    }

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async_main(args))
}

fn run_sync_fast_path(args: Args) -> anyhow::Result<()> {
    match args.command {
        Some(Command::DumpConfig) => cli_config::run(
            args.cwd.clone().map(PathBuf::from),
            AppConfigOverrides {
                model: args.model.clone(),
                max_steps: args.max_steps,
                api_bind_addr: None,
            },
        ),
        _ => Ok(()),
    }
}

async fn async_main(args: Args) -> anyhow::Result<()> {
    match args.command.clone() {
        Some(Command::Index {
            path,
            deterministic,
            embedding_model,
        }) => {
            return cli_index::run(IndexOptions {
                cwd: path.or_else(|| args.cwd.clone().map(PathBuf::from)),
                deterministic,
                embedding_model,
                eval_query: None,
                eval_kind: None,
                eval_limit: 8,
            })
            .await;
        }
        Some(Command::Sessions) => return sessions::run(args.cwd.clone()).await,
        Some(Command::State { command }) => return cli_state::run(args.cwd.clone(), command).await,
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

    if let Some(message) = message {
        run_exec(args, runtime, message).await
    } else {
        repl::run(runtime).await
    }
}

async fn build_runtime(
    args: &Args,
    fake_message: Option<&String>,
) -> anyhow::Result<rove::interfaces::cli::runtime::CliRuntime> {
    build_cli_runtime(CliRuntimeOptions {
        cwd: args.cwd.clone().map(PathBuf::from),
        model: args.model.clone(),
        max_steps: args.max_steps,
        approval: args.approval,
        task_workspace: args.task_workspace.clone(),
        task_base: args.task_base.clone(),
        initial_fake_response: fake_message.map(|message| format!("fake response: {message}")),
    })
    .await
}

fn join_message(message: Vec<String>) -> String {
    message.join(" ").trim().to_string()
}

async fn run_exec(
    args: Args,
    runtime: rove::interfaces::cli::runtime::CliRuntime,
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
