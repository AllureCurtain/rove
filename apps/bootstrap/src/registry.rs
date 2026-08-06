use std::path::PathBuf;

use rove_core::{Tool, ToolRegistry};
use rove_runtime::tools::fs::{FsReadTool, FsWriteTool};
use rove_runtime::tools::mcp_proxy::register_mcp_tools_from_file;
use rove_runtime::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use rove_runtime::tools::request_input::RequestInputTool;
use rove_runtime::tools::search::SearchCodeTool;
use rove_runtime::tools::shell::{ShellPolicy, ShellTool};
use rove_runtime::workspace::Workspace;

use crate::config::AppConfig;

/// Build the default first-party tool registry exposed to models.
///
/// `echo` is intentionally omitted; keep it for tests only.
pub fn tool_registry(workspace: &Workspace) -> ToolRegistry {
    tool_registry_with_shell_policy(workspace, ShellPolicy::default())
}

/// Build the default first-party tool registry with a custom shell policy.
pub fn tool_registry_with_shell_policy(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(SearchCodeTool::new(workspace.root.clone())));
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    registry.register(Box::new(SaveMemoryTool::new()));
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        shell_policy,
    )));
    registry
}

/// Build the default first-party registry and register configured MCP tools.
pub async fn tool_registry_with_mcp(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<PathBuf>,
) -> anyhow::Result<ToolRegistry> {
    let mut registry = tool_registry_with_shell_policy(workspace, shell_policy);
    register_mcp_tools_from_file(&mut registry, mcp_config_path).await?;
    Ok(registry)
}

/// Build the registry allowed by the selected workspace's activation state.
///
/// Restricted workspaces keep the ordinary local tools, but repository-owned
/// MCP definitions are not read or spawned.
pub async fn tool_registry_for_config(
    workspace: &Workspace,
    config: &AppConfig,
) -> anyhow::Result<ToolRegistry> {
    if !config.project_capability_allowed(crate::project_trust::CAP_MCP_PROCESSES) {
        return Ok(tool_registry_with_shell_policy(
            workspace,
            config.shell_policy(),
        ));
    }
    tool_registry_with_mcp(
        workspace,
        config.shell_policy(),
        config.workspace_bounded_mcp_config_path()?,
    )
    .await
}

/// Helper for product surfaces that need to inject extra tools after defaults.
pub fn register_extra_tools(registry: &mut ToolRegistry, tools: Vec<Box<dyn Tool>>) {
    for tool in tools {
        registry.register(tool);
    }
}

#[cfg(test)]
mod tests {
    use crate::{AppConfig, ProjectActivationState};
    use rove_runtime::workspace::Workspace;

    use super::tool_registry_for_config;

    #[tokio::test]
    async fn restricted_workspace_does_not_read_or_spawn_mcp_configuration() {
        let temp = tempfile::TempDir::new().unwrap();
        let config_dir = temp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("mcp_servers.json"),
            r#"{"servers":[{"name":"blocked","enabled":true,"transport":"stdio","command":"rove-command-that-does-not-exist"}]}"#,
        )
        .unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        let mut config = AppConfig::default();
        config.rebase_to_workspace(&workspace.root);
        config.source_summary.project_activation = ProjectActivationState::Restricted;
        config.source_summary.project_activation_source = None;
        config.source_summary.trusted_workspace_roots.clear();

        let registry = tool_registry_for_config(&workspace, &config).await.unwrap();

        assert!(registry.has("run_shell"));
        assert!(!registry.has("mcp__blocked"));
    }
}
