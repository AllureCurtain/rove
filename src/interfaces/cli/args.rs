use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

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

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliApprovalPolicy {
    Ask,
    Auto,
    Never,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Print the effective runtime configuration.
    DumpConfig,
    /// Index a workspace for RAG retrieval.
    Index {
        /// Workspace path to index. Defaults to the current working directory.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Use deterministic local embeddings instead of the OpenAI embedding API.
        #[arg(long)]
        deterministic: bool,

        /// OpenAI-compatible embedding model.
        #[arg(long)]
        embedding_model: Option<String>,
    },
    /// List resumable local task states.
    Sessions,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Args, CliApprovalPolicy, Command};

    #[test]
    fn approval_defaults_to_ask() {
        let args = Args::parse_from(["rove", "inspect"]);
        assert!(matches!(args.approval, CliApprovalPolicy::Ask));
    }

    #[test]
    fn sessions_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "sessions"]);

        assert!(args.message.is_none());
        assert!(matches!(args.command, Some(Command::Sessions)));
    }

    #[test]
    fn dump_config_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "dump-config"]);

        assert!(args.message.is_none());
        assert!(matches!(args.command, Some(Command::DumpConfig)));
    }

    #[test]
    fn index_subcommand_parses_options_without_message() {
        let args = Args::parse_from([
            "rove",
            "index",
            "src",
            "--deterministic",
            "--embedding-model",
            "text-embedding-3-large",
        ]);

        assert!(args.message.is_none());
        match args.command {
            Some(Command::Index {
                path,
                deterministic,
                embedding_model,
            }) => {
                assert_eq!(path, Some(PathBuf::from("src")));
                assert!(deterministic);
                assert_eq!(embedding_model.as_deref(), Some("text-embedding-3-large"));
            }
            other => panic!("expected index subcommand, got {other:?}"),
        }
    }
}
