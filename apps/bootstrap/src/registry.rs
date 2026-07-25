use std::path::PathBuf;

use rove_core::{Tool, ToolRegistry};
use rove_runtime::tools::fs::{FsReadTool, FsWriteTool};
use rove_runtime::tools::mcp_proxy::register_mcp_tools_from_file;
use rove_runtime::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use rove_runtime::tools::request_input::RequestInputTool;
use rove_runtime::tools::search::SearchCodeTool;
use rove_runtime::tools::shell::{ShellPolicy, ShellTool};
use rove_runtime::workspace::Workspace;

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

/// Helper for product surfaces that need to inject extra tools after defaults.
pub fn register_extra_tools(registry: &mut ToolRegistry, tools: Vec<Box<dyn Tool>>) {
    for tool in tools {
        registry.register(tool);
    }
}
