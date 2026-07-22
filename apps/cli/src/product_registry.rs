use rove_app_bootstrap::registry as bootstrap_registry;
use rove_core::ToolRegistry;
use rove_runtime::tools::shell::ShellPolicy;
use rove_runtime::workspace::Workspace;

pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
    default_tool_registry_with_shell_policy(workspace, ShellPolicy::default())
}

pub fn default_tool_registry_with_shell_policy(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
) -> ToolRegistry {
    #[allow(unused_mut)]
    let mut registry =
        bootstrap_registry::default_tool_registry_with_shell_policy(workspace, shell_policy);
    #[cfg(feature = "rag")]
    replace_with_real_rag_tools(&mut registry, workspace);
    registry
}

pub async fn runtime_tool_registry(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<std::path::PathBuf>,
) -> anyhow::Result<ToolRegistry> {
    #[allow(unused_mut)]
    let mut registry =
        bootstrap_registry::runtime_tool_registry(workspace, shell_policy, mcp_config_path).await?;
    #[cfg(feature = "rag")]
    replace_with_real_rag_tools(&mut registry, workspace);
    Ok(registry)
}

#[cfg(feature = "rag")]
fn replace_with_real_rag_tools(registry: &mut ToolRegistry, workspace: &Workspace) {
    use crate::rag::RagRetrieveTool;
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
}
