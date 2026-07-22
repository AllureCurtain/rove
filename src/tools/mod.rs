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
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ShellPolicy;

pub use rove_app_bootstrap::registry::{
    default_tool_registry as bootstrap_default_tool_registry,
    default_tool_registry_with_shell_policy as bootstrap_default_tool_registry_with_shell_policy,
    runtime_tool_registry as bootstrap_runtime_tool_registry,
};

/// Product registry used by first-party surfaces.
///
/// Without the `rag` feature this re-exports the bootstrap registry, which already
/// includes disabled RAG stub tools. With `rag` enabled, real RAG tools replace the
/// stubs for root product assembly.
pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
    default_tool_registry_with_shell_policy(workspace, ShellPolicy::default())
}

pub fn default_tool_registry_with_shell_policy(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
) -> ToolRegistry {
    #[allow(unused_mut)]
    let mut registry = bootstrap_default_tool_registry_with_shell_policy(workspace, shell_policy);
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
        bootstrap_runtime_tool_registry(workspace, shell_policy, mcp_config_path).await?;
    #[cfg(feature = "rag")]
    replace_with_real_rag_tools(&mut registry, workspace);
    Ok(registry)
}

#[cfg(feature = "rag")]
fn replace_with_real_rag_tools(registry: &mut ToolRegistry, workspace: &Workspace) {
    use rove_cli::rag::RagRetrieveTool;
    // ToolRegistry keeps last registration for a name only if re-register is supported.
    // Prefer explicit re-register helpers if available; otherwise register again.
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
}
