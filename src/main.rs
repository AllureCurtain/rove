use std::path::PathBuf;

use clap::Parser;
use rove::config::AppConfig;
use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::types::ApprovalPolicy;
use rove::core::workspace::Workspace;
use rove::interfaces::cli::args::{Args, CliApprovalPolicy};
use rove::interfaces::cli::oneshot::run_oneshot;
use rove::models::fake::FakeModelClient;
use rove::models::openai::OpenAiClient;
use rove::models::traits::ModelClient;
use rove::state::store::StateStore;
use rove::tools::echo::EchoTool;
use rove::tools::fs::{FsReadTool, FsWriteTool};
use rove::tools::registry::ToolRegistry;
use rove::tools::shell::ShellTool;

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

    // Load config
    let config = AppConfig::from_env()?;

    // Detect workspace
    let cwd = args
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;

    tracing::info!(
        workspace_root = %workspace.root.display(),
        workspace_kind = ?workspace.kind,
        "Workspace detected"
    );

    // Build model client
    let model_id = args.model.unwrap_or(config.model.clone());
    let model: Box<dyn ModelClient> = if model_id == "fake" {
        Box::new(FakeModelClient::new(format!("fake response: {}", message)))
    } else {
        Box::new(OpenAiClient::new(
            config.api_base.clone(),
            config.api_key.clone(),
            model_id,
        ))
    };

    // Build tool registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    // Build context manager
    let system_prompt = config.load_system_prompt();
    let context_manager = ContextManager::new(system_prompt);

    // Build engine
    let engine_config = EngineConfig {
        max_steps: args.max_steps.unwrap_or(config.max_steps),
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
    );

    // Set up state store + trace
    let state_store = StateStore::new(&workspace.state_dir);
    let resume_state = match args.resume.as_deref() {
        Some("latest") => state_store.load_latest_task_state().await?,
        Some(other) => anyhow::bail!("unsupported --resume value: {other}; use --resume latest"),
        None => None,
    };
    let run_id = state_store.new_run();
    let run_dir = state_store.run_store.run_dir(&run_id);
    let trace_writer = state_store.run_store.create_trace(&run_id).ok();

    tracing::info!(%run_id, "Starting run");

    // Run
    run_oneshot(
        &engine,
        message,
        trace_writer,
        run_id,
        run_dir,
        resume_state,
        &state_store,
    )
    .await;

    Ok(())
}
