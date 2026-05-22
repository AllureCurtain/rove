use clap::{Parser, ValueEnum};

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

    /// Resume a previous task state. Use "latest" for the most recent snapshot.
    #[arg(long)]
    pub resume: Option<String>,

    /// Tool approval policy.
    #[arg(long, value_enum, default_value_t = CliApprovalPolicy::Ask)]
    pub approval: CliApprovalPolicy,

    /// Working directory (defaults to current directory).
    #[arg(short = 'C', long)]
    pub cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliApprovalPolicy {
    Ask,
    Auto,
    Never,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Args, CliApprovalPolicy};

    #[test]
    fn approval_defaults_to_ask() {
        let args = Args::parse_from(["rove", "inspect"]);
        assert!(matches!(args.approval, CliApprovalPolicy::Ask));
    }
}
