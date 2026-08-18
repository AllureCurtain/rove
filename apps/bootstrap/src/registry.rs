use std::path::{Path, PathBuf};
use std::sync::Arc;

use rove_core::{Tool, ToolRegistry};
use rove_runtime::environment::{ExecutionEnvironment, local_environment};
use rove_runtime::tools::coding::{
    DeletePathTool, EditFileTool, GlobPathsTool, ListDirectoryTool, MovePathTool,
    WorkspaceCheckpointTool, WorkspaceDiffTool, WorkspaceRewindTool,
};
use rove_runtime::tools::fs::{FsReadTool, FsWriteTool};
use rove_runtime::tools::history::ResolveToolArtifactTool;
use rove_runtime::tools::mcp_proxy::{
    McpServerConfig, register_mcp_tools_from_file_with_environment,
    register_mcp_tools_with_environment,
};
use rove_runtime::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use rove_runtime::tools::repository::RepositoryMapTool;
use rove_runtime::tools::request_input::RequestInputTool;
use rove_runtime::tools::search::SearchCodeTool;
use rove_runtime::tools::shell::{
    ShellOutputTool, ShellPolicy, ShellPtyTool, ShellTerminateTool, ShellTool,
};
use rove_runtime::workspace::Workspace;

use crate::config::AppConfig;
use crate::user_state::McpCatalogAuthority;

/// Upper bound for a bootstrap-read MCP catalog, matching the runtime's
/// `MAX_MCP_CONFIG_BYTES`.
const MAX_BOOTSTRAP_MCP_CATALOG_BYTES: u64 = 256 * 1024;

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
    registry.register(Box::new(EditFileTool::new()));
    registry.register(Box::new(DeletePathTool::new()));
    registry.register(Box::new(MovePathTool::new()));
    registry.register(Box::new(ListDirectoryTool::new()));
    registry.register(Box::new(GlobPathsTool::new()));
    registry.register(Box::new(WorkspaceCheckpointTool::new()));
    registry.register(Box::new(WorkspaceDiffTool::new()));
    registry.register(Box::new(WorkspaceRewindTool::new()));
    registry.register(Box::new(SearchCodeTool::new(workspace.root.clone())));
    registry.register(Box::new(RepositoryMapTool));
    registry.register(Box::new(ResolveToolArtifactTool));
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    registry.register(Box::new(SaveMemoryTool::new()));
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        shell_policy,
    )));
    registry.register(Box::new(ShellOutputTool::new()));
    registry.register(Box::new(ShellTerminateTool::new()));
    registry.register(Box::new(ShellPtyTool::new()));
    registry
}

/// Build the default first-party registry and register configured MCP tools.
pub async fn tool_registry_with_mcp(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<PathBuf>,
) -> anyhow::Result<ToolRegistry> {
    tool_registry_with_mcp_and_environment(
        workspace,
        shell_policy,
        mcp_config_path,
        local_environment(workspace),
    )
    .await
}

pub async fn tool_registry_with_mcp_and_environment(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<PathBuf>,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<ToolRegistry> {
    let mcp_config_path = mcp_config_path.into();
    let mcp_config_path = if mcp_config_path.is_absolute() {
        mcp_config_path
    } else {
        workspace.root.join(mcp_config_path)
    };
    let authority = McpCatalogAuthority::Workspace {
        path: mcp_config_path,
    };
    tool_registry_with_mcp_authority_and_environment(
        workspace,
        shell_policy,
        authority,
        environment,
    )
    .await
}

/// Build a registry from a path whose authority was resolved by AppConfig.
/// This is the only path that may read the user-state MCP catalog.
pub async fn tool_registry_with_mcp_authority_and_environment(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    authority: McpCatalogAuthority,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<ToolRegistry> {
    authority
        .validate(&workspace.root)
        .map_err(|error| anyhow::anyhow!("invalid MCP catalog authority: {error}"))?;
    let mut registry = tool_registry_with_shell_policy(workspace, shell_policy);
    if authority.is_user_state() {
        register_contract_mcp_catalog(&mut registry, authority.path(), environment).await?;
    } else {
        register_mcp_tools_from_file_with_environment(
            &mut registry,
            authority.path().to_path_buf(),
            environment,
        )
        .await?;
    }
    Ok(registry)
}

#[derive(serde::Deserialize)]
struct McpCatalogFile {
    #[serde(default)]
    servers: Vec<McpServerConfig>,
}

async fn register_contract_mcp_catalog(
    registry: &mut ToolRegistry,
    path: &Path,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<()> {
    if !environment.capabilities().filesystem_read {
        anyhow::bail!("execution capability unavailable: filesystem_read");
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            anyhow::bail!("could not inspect MCP catalog {}: {error}", path.display())
        }
    };
    if !metadata.is_file() {
        anyhow::bail!("MCP catalog is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_BOOTSTRAP_MCP_CATALOG_BYTES {
        anyhow::bail!("MCP config exceeds the supported size");
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_to_end(&mut file, &mut bytes))
        .map_err(|error| {
            anyhow::anyhow!("could not read MCP catalog {}: {error}", path.display())
        })?;
    if bytes.len() as u64 > MAX_BOOTSTRAP_MCP_CATALOG_BYTES {
        anyhow::bail!("MCP config exceeds the supported size");
    }
    let catalog: McpCatalogFile = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow::anyhow!("MCP catalog {} is invalid: {error}", path.display()))?;
    register_mcp_tools_with_environment(registry, catalog.servers, environment).await?;
    Ok(())
}

/// Build the registry allowed by the selected workspace's activation state.
///
/// Restricted workspaces keep the ordinary local tools, but repository-owned
/// MCP definitions are not read or spawned.
pub async fn tool_registry_for_config(
    workspace: &Workspace,
    config: &AppConfig,
) -> anyhow::Result<ToolRegistry> {
    tool_registry_for_config_with_environment(workspace, config, local_environment(workspace)).await
}

pub async fn tool_registry_for_config_with_environment(
    workspace: &Workspace,
    config: &AppConfig,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<ToolRegistry> {
    if !config.project_capability_allowed(crate::project_trust::CAP_MCP_PROCESSES) {
        return Ok(tool_registry_with_shell_policy(
            workspace,
            config.shell_policy(),
        ));
    }
    let authority = config
        .mcp_catalog_authority()
        .map_err(|error| anyhow::anyhow!("MCP catalog is not authorized: {error}"))?;
    tool_registry_with_mcp_authority_and_environment(
        workspace,
        config.shell_policy(),
        authority,
        environment,
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

    #[tokio::test]
    async fn trusted_workspace_without_mcp_configuration_keeps_builtin_tools() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        let mut config = AppConfig::default();
        config.rebase_to_workspace(&workspace.root);

        let registry = tool_registry_for_config(&workspace, &config).await.unwrap();

        assert!(registry.has("read_file"));
        assert!(registry.has("run_shell"));
    }
}
