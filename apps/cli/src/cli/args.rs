use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// rove — a local-first, stateful, observable agent runtime
#[derive(Parser, Debug)]
#[command(name = "rove", version, about)]
pub struct Args {
    /// The task or question to give the agent.
    #[arg(value_name = "MESSAGE", num_args = 1..)]
    pub message: Vec<String>,

    /// Model to use (overrides ROVE_MODEL env var).
    #[arg(short, long, global = true)]
    pub model: Option<String>,

    /// Maximum steps for this run.
    #[arg(long, global = true)]
    pub max_steps: Option<u32>,

    /// Fully qualified Agent selector, for example `workspace:ops`.
    #[arg(long, global = true, value_name = "SOURCE:ID")]
    pub agent: Option<String>,

    /// Resume a previous task state. Use "latest" for the most recent snapshot.
    #[arg(long, global = true)]
    pub resume: Option<String>,

    /// Tool approval policy.
    #[arg(long, value_enum, default_value_t = CliApprovalPolicy::Ask, global = true)]
    pub approval: CliApprovalPolicy,

    /// Working directory (defaults to current directory).
    #[arg(short = 'C', long, global = true)]
    pub cwd: Option<String>,

    /// Explicitly allow this run to load project config and activate workspace MCP servers.
    #[arg(long, global = true)]
    pub trust_project: bool,

    /// Create or use an isolated standalone task workspace by name.
    #[arg(long, global = true)]
    pub task_workspace: Option<String>,

    /// Base directory for task workspaces. Defaults to <state_dir>/tasks.
    #[arg(long, global = true)]
    pub task_base: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Args {
    pub fn is_sync_fast_path(&self) -> bool {
        matches!(
            self.command,
            Some(Command::DumpConfig | Command::Provider { .. })
        )
    }

    pub fn is_tui(&self) -> bool {
        matches!(self.command, Some(Command::Tui))
    }

    pub fn message(&self) -> Option<String> {
        let message = self.message.join(" ").trim().to_string();
        if message.is_empty() {
            None
        } else {
            Some(message)
        }
    }
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
    /// Start the full-screen terminal interface.
    Tui,
    /// Run a prompt non-interactively and exit.
    Exec {
        /// The task or question to give the agent.
        #[arg(value_name = "MESSAGE", num_args = 1.., required = true)]
        message: Vec<String>,
    },
    /// List resumable local task states.
    Sessions,
    /// Maintain the local state index.
    State {
        #[command(subcommand)]
        command: StateCommand,
    },
    /// Query or change durable Project Trust for the selected workspace.
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
    /// Inspect or migrate legacy Provider configuration.
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
}

#[derive(Clone, Debug, Subcommand)]
pub enum ProviderCommand {
    /// Import legacy Provider profiles into the user catalog (dry-run by default).
    Migrate {
        /// Apply the migration. Without this flag no files or databases are changed.
        #[arg(long)]
        apply: bool,
        /// Rewrite the trusted workspace config to selection-only Provider fields.
        #[arg(long, requires = "apply")]
        rewrite_workspace_config: bool,
        /// Legacy API ProductStore to inspect. Defaults to <cwd>/.rove/product.sqlite.
        #[arg(long, value_name = "PATH")]
        product_store: Option<PathBuf>,
        /// Resolve a conflict as SOURCE:PROFILE=NEW_PROFILE (repeatable).
        #[arg(long, value_name = "SOURCE:PROFILE=NEW_PROFILE")]
        rename: Vec<String>,
    },
}

#[derive(Clone, Debug, Subcommand)]
pub enum StateCommand {
    /// Rebuild missing SQLite index rows from local artifacts.
    Repair,
    /// Remove expired state rows and run artifacts.
    Cleanup,
}

#[derive(Clone, Debug, Subcommand)]
pub enum TrustCommand {
    /// Query durable trust without applying temporary --trust-project grants.
    Query {
        #[arg(long, value_enum)]
        capability: Vec<CliProjectTrustCapability>,
    },
    /// Persist grants for all or selected capabilities.
    Grant {
        #[arg(long, value_enum)]
        capability: Vec<CliProjectTrustCapability>,
    },
    /// Persist a denial for all or selected capabilities.
    Deny {
        #[arg(long, value_enum)]
        capability: Vec<CliProjectTrustCapability>,
    },
    /// Revoke all or selected durable capability grants.
    Revoke {
        #[arg(long, value_enum)]
        capability: Vec<CliProjectTrustCapability>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum CliProjectTrustCapability {
    #[value(name = "project_configuration", alias = "project-configuration")]
    ProjectConfiguration,
    #[value(name = "workspace_instructions", alias = "workspace-instructions")]
    WorkspaceInstructions,
    #[value(name = "mcp_processes", alias = "mcp-processes")]
    McpProcesses,
    #[value(name = "hooks_extensions", alias = "hooks-extensions")]
    HooksExtensions,
    #[value(name = "provider_credentials", alias = "provider-credentials")]
    ProviderCredentials,
    #[value(name = "external_paths", alias = "external-paths")]
    ExternalPaths,
}

impl CliProjectTrustCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectConfiguration => "project_configuration",
            Self::WorkspaceInstructions => "workspace_instructions",
            Self::McpProcesses => "mcp_processes",
            Self::HooksExtensions => "hooks_extensions",
            Self::ProviderCredentials => "provider_credentials",
            Self::ExternalPaths => "external_paths",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{
        Args, CliApprovalPolicy, CliProjectTrustCapability, Command, ProviderCommand, TrustCommand,
    };

    #[test]
    fn approval_defaults_to_ask() {
        let args = Args::parse_from(["rove", "inspect"]);
        assert!(matches!(args.approval, CliApprovalPolicy::Ask));
    }

    #[test]
    fn qualified_agent_selector_parses_as_a_global_option() {
        let args = Args::parse_from(["rove", "exec", "--agent", "workspace:ops", "inspect"]);

        assert_eq!(args.agent.as_deref(), Some("workspace:ops"));
    }

    #[test]
    fn trust_project_is_explicit_and_global() {
        let default = Args::parse_from(["rove", "inspect"]);
        let trusted = Args::parse_from(["rove", "exec", "--trust-project", "inspect"]);

        assert!(!default.trust_project);
        assert!(trusted.trust_project);
    }

    #[test]
    fn sessions_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "sessions"]);

        assert!(args.message().is_none());
        assert!(matches!(args.command, Some(Command::Sessions)));
    }

    #[test]
    fn dump_config_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "dump-config"]);

        assert!(args.message().is_none());
        assert!(matches!(args.command, Some(Command::DumpConfig)));
    }

    #[test]
    fn dump_config_is_a_sync_fast_path() {
        let args = Args::parse_from(["rove", "dump-config"]);

        assert!(args.is_sync_fast_path());
    }

    #[test]
    fn provider_migrate_is_dry_run_by_default_and_accepts_explicit_apply() {
        let dry_run = Args::parse_from(["rove", "provider", "migrate"]);
        assert!(matches!(
            dry_run.command,
            Some(Command::Provider {
                command: ProviderCommand::Migrate { apply: false, .. }
            })
        ));
        assert!(dry_run.is_sync_fast_path());

        let apply = Args::parse_from([
            "rove",
            "--trust-project",
            "provider",
            "migrate",
            "--apply",
            "--rewrite-workspace-config",
        ]);
        assert!(matches!(
            apply.command,
            Some(Command::Provider {
                command: ProviderCommand::Migrate {
                    apply: true,
                    rewrite_workspace_config: true,
                    ..
                }
            })
        ));
    }

    #[test]
    fn no_args_enters_async_cli_path() {
        let args = Args::parse_from(["rove"]);

        assert!(args.message().is_none());
        assert!(args.command.is_none());
        assert!(!args.is_sync_fast_path());
    }

    #[test]
    fn tui_subcommand_accepts_global_runtime_options() {
        let args = Args::parse_from(["rove", "tui", "--model", "fake", "--approval", "never"]);

        assert!(args.is_tui());
        assert!(matches!(args.command, Some(Command::Tui)));
        assert_eq!(args.model.as_deref(), Some("fake"));
        assert!(matches!(args.approval, CliApprovalPolicy::Never));
        assert!(args.message().is_none());
    }

    #[test]
    fn quoted_task_parses_as_initial_prompt() {
        let args = Args::parse_from(["rove", "analyze this project"]);

        assert_eq!(args.message().as_deref(), Some("analyze this project"));
        assert!(args.command.is_none());
    }

    #[test]
    fn unquoted_multi_word_task_parses_as_initial_prompt() {
        let args = Args::try_parse_from(["rove", "analyze", "this", "project"]).unwrap();

        assert_eq!(args.message().as_deref(), Some("analyze this project"));
        assert!(args.command.is_none());
    }

    #[test]
    fn exec_subcommand_parses_noninteractive_message() {
        let args = Args::parse_from(["rove", "exec", "analyze this project"]);

        assert!(args.message().is_none());
        match args.command {
            Some(Command::Exec { message }) => {
                assert_eq!(message, vec!["analyze this project".to_string()]);
            }
            other => panic!("expected exec subcommand, got {other:?}"),
        }
    }

    #[test]
    fn exec_subcommand_joins_unquoted_multi_word_message() {
        let args = Args::parse_from(["rove", "exec", "analyze", "this", "project"]);

        assert!(args.message().is_none());
        match args.command {
            Some(Command::Exec { message }) => {
                assert_eq!(message.join(" "), "analyze this project".to_string());
            }
            other => panic!("expected exec subcommand, got {other:?}"),
        }
    }

    #[test]
    fn exec_subcommand_requires_message() {
        let err = Args::try_parse_from(["rove", "exec"]).unwrap_err();

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn runtime_flags_parse_after_exec_subcommand() {
        let args = Args::parse_from([
            "rove",
            "exec",
            "--model",
            "fake",
            "--approval",
            "never",
            "hello",
        ]);

        assert_eq!(args.model.as_deref(), Some("fake"));
        assert!(matches!(args.approval, CliApprovalPolicy::Never));
        match args.command {
            Some(Command::Exec { message }) => assert_eq!(message, vec!["hello".to_string()]),
            other => panic!("expected exec subcommand, got {other:?}"),
        }
    }

    #[test]
    fn state_repair_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "state", "repair"]);

        assert!(args.message().is_none());
        assert!(matches!(
            args.command,
            Some(Command::State {
                command: super::StateCommand::Repair
            })
        ));
    }

    #[test]
    fn state_cleanup_subcommand_parses_without_message() {
        let args = Args::parse_from(["rove", "state", "cleanup"]);

        assert!(args.message().is_none());
        assert!(matches!(
            args.command,
            Some(Command::State {
                command: super::StateCommand::Cleanup
            })
        ));
    }

    #[test]
    fn task_workspace_options_parse_for_standalone_runs() {
        let args = Args::parse_from([
            "rove",
            "--task-workspace",
            "standalone",
            "--task-base",
            "D:/rove-tasks",
            "do work",
        ]);

        assert_eq!(args.task_workspace.as_deref(), Some("standalone"));
        assert_eq!(args.task_base, Some(PathBuf::from("D:/rove-tasks")));
    }

    #[test]
    fn trust_subcommands_parse_capability_scoped_operations() {
        let args = Args::parse_from([
            "rove",
            "trust",
            "grant",
            "--capability",
            "provider_credentials",
            "--capability",
            "mcp-processes",
        ]);

        assert!(matches!(
            args.command,
            Some(Command::Trust {
                command: TrustCommand::Grant { capability }
            }) if capability == vec![
                CliProjectTrustCapability::ProviderCredentials,
                CliProjectTrustCapability::McpProcesses,
            ]
        ));
        assert!(!args.trust_project);
    }
}
