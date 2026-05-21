use clap::Parser;

/// rove — a local-first, stateful, observable agent runtime
#[derive(Parser, Debug)]
#[command(name = "rove", version, about)]
pub struct Args {
    /// The task or question to give the agent.
    #[arg()]
    pub message: Option<String>,

    /// Model to use (overrides ROVE_MODEL env var).
    #[arg(short, long)]
    pub model: Option<String>,

    /// Maximum steps for this run.
    #[arg(long)]
    pub max_steps: Option<u32>,

    /// Working directory (defaults to current directory).
    #[arg(short = 'C', long)]
    pub cwd: Option<String>,
}
