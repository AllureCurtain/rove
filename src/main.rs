use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use clap::Parser;
use rove::config::{AppConfig, AppConfigOverrides};
use rove::core::context::{ContextBudget, ContextManager};
use rove::core::engine::{Engine, EngineConfig};
use rove::core::types::{ApprovalPolicy, RunId, TerminationReason};
use rove::core::workspace::Workspace;
use rove::interfaces::cli::approval::stdin_approval_provider;
use rove::interfaces::cli::args::{Args, CliApprovalPolicy, Command};
use rove::interfaces::cli::config as cli_config;
use rove::interfaces::cli::index::{self as cli_index, IndexOptions};
use rove::interfaces::cli::input::stdin_input_provider;
use rove::interfaces::cli::oneshot::{resolve_resume_state, run_oneshot_with_cancel};
use rove::interfaces::cli::sessions;
use rove::interfaces::cli::state as cli_state;
use rove::models::factory::build_model_client;
use rove::models::fake::FakeModelClient;
use rove::models::traits::ModelClient;
use rove::state::store::StateStore;
use rove::tools::echo::EchoTool;
use rove::tools::fs::{FsReadTool, FsWriteTool};
use rove::tools::mcp_proxy::register_mcp_tools_from_file;
use rove::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use rove::tools::rag::RagRetrieveTool;
use rove::tools::registry::ToolRegistry;
use rove::tools::request_input::RequestInputTool;
use rove::tools::shell::ShellTool;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rove=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    match args.command {
        Some(Command::DumpConfig) => {
            return cli_config::run(
                args.cwd.clone().map(PathBuf::from),
                AppConfigOverrides {
                    model: args.model.clone(),
                    max_steps: args.max_steps,
                    api_bind_addr: None,
                },
            );
        }
        Some(Command::Index {
            path,
            deterministic,
            embedding_model,
        }) => {
            return cli_index::run(IndexOptions {
                cwd: path.or_else(|| args.cwd.map(PathBuf::from)),
                deterministic,
                embedding_model,
                eval_query: None,
                eval_kind: None,
                eval_limit: 8,
            })
            .await;
        }
        Some(Command::Sessions) => return sessions::run(args.cwd).await,
        Some(Command::State { command }) => return cli_state::run(args.cwd, command).await,
        None => {}
    }

    // Fast path: no message = show help
    let message = match args.message {
        Some(msg) => msg,
        None => {
            eprintln!("rove — a local-first agent runtime");
            eprintln!();
            eprintln!("Usage: rove \"<your task>\"");
            eprintln!();
            eprintln!("Examples:");
            eprintln!("  rove \"echo hello\"");
            eprintln!("  rove \"find all TODO comments in this project\"");
            return Ok(());
        }
    };

    // Detect workspace
    let cwd = args
        .cwd
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;

    // Load config after workspace detection so `.rove/config.toml` is scoped to the project root.
    let config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            model: args.model.clone(),
            max_steps: args.max_steps,
            api_bind_addr: None,
        },
    )?;

    // Ensure state root exists after config path validation.
    let configured_state_dir = config.state_dir();
    if configured_state_dir != workspace.state_dir {
        std::fs::create_dir_all(&configured_state_dir)?;
    }
    let workspace = Workspace {
        state_dir: configured_state_dir,
        ..workspace
    };
    workspace.ensure_state_dir()?;

    tracing::info!(
        workspace_root = %workspace.root.display(),
        workspace_kind = ?workspace.kind,
        "Workspace detected"
    );

    // Build model client
    let model_id = config.provider.model.clone();
    let model: Box<dyn ModelClient> = if model_id == "fake" {
        Box::new(FakeModelClient::new(format!("fake response: {}", message)))
    } else {
        build_model_client(&config, model_id)
    };

    // Build tool registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ReadMemoryTopicTool::new(workspace.root.clone())));
    registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
    registry.register(Box::new(UpdateMemoryIndexTool::new(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));
    let mcp_config_path = config.resolve_path(&config.tool.mcp_config_path);
    let mcp_tool_count = register_mcp_tools_from_file(&mut registry, mcp_config_path).await?;
    if mcp_tool_count > 0 {
        tracing::info!(mcp_tool_count, "Registered MCP tools");
    }

    // Build context manager
    let system_prompt = config.load_system_prompt();
    let context_manager = ContextManager::with_token_budget(
        system_prompt,
        ContextBudget {
            soft_limit_tokens: config.runtime.context_soft_limit_tokens,
            hard_limit_tokens: config.runtime.context_hard_limit_tokens,
            reserved_tokens: config.runtime.context_reserved_tokens,
        },
    );

    // Build engine
    let engine_config = EngineConfig {
        max_steps: config.runtime.max_steps,
        plan_enabled: true,
    };
    let approval_policy = match args.approval {
        CliApprovalPolicy::Ask => ApprovalPolicy::Ask,
        CliApprovalPolicy::Auto => ApprovalPolicy::Auto,
        CliApprovalPolicy::Never => ApprovalPolicy::Never,
    };
    let engine = Engine::with_workspace(
        model,
        registry,
        context_manager,
        engine_config,
        workspace.clone(),
        approval_policy,
    )
    .with_memory_recall_limit(config.memory.recall_limit)
    .with_input_provider(stdin_input_provider());
    let engine = if approval_policy == ApprovalPolicy::Ask {
        engine.with_approval_provider(stdin_approval_provider())
    } else {
        engine
    };

    // Set up state store + trace
    let state_store = StateStore::with_index_path(
        &workspace.state_dir,
        config.sqlite_path(),
        config.state.sqlite_busy_timeout_ms,
    );
    let resume_state = resolve_resume_state(&state_store, args.resume.as_deref()).await?;
    let run_id = RunId::new();
    let run_handle = state_store.start_run(
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

    tracing::info!(%run_handle.run_id, "Starting run");

    // Run
    let cli_cancel = CancellationToken::new();
    let signal_exit_code = spawn_cli_signal_listener(cli_cancel.clone());
    let termination = run_oneshot_with_cancel(
        &engine,
        message,
        run_handle,
        resume_state,
        &state_store,
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
