pub mod echo;
pub mod fs;
pub mod mcp_proxy;
pub mod memory;
#[cfg(feature = "rag")]
pub mod rag;
#[cfg(not(feature = "rag"))]
#[path = "rag_stub.rs"]
pub mod rag;
pub mod registry;
pub mod request_input;
pub mod runtime_context;
pub mod shell;
pub mod traits;

use crate::core::workspace::Workspace;
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ShellPolicy;

pub use rove_app_bootstrap::registry::{
    default_tool_registry as bootstrap_default_tool_registry,
    default_tool_registry_with_shell_policy as bootstrap_default_tool_registry_with_shell_policy,
    runtime_tool_registry as bootstrap_runtime_tool_registry,
};

/// Product registry used by first-party surfaces, including optional RAG tools.
pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
    default_tool_registry_with_shell_policy(workspace, ShellPolicy::default())
}

pub fn default_tool_registry_with_shell_policy(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
) -> ToolRegistry {
    let mut registry = bootstrap_default_tool_registry_with_shell_policy(workspace, shell_policy);
    register_optional_rag_tools(&mut registry, workspace);
    registry
}

pub async fn runtime_tool_registry(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<std::path::PathBuf>,
) -> anyhow::Result<ToolRegistry> {
    let mut registry =
        bootstrap_runtime_tool_registry(workspace, shell_policy, mcp_config_path).await?;
    register_optional_rag_tools(&mut registry, workspace);
    Ok(registry)
}

fn register_optional_rag_tools(registry: &mut ToolRegistry, workspace: &Workspace) {
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
}
