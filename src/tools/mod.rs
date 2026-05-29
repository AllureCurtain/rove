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
pub mod shell;
pub mod traits;

use crate::core::workspace::Workspace;
use crate::tools::echo::EchoTool;
use crate::tools::fs::{FsReadTool, FsWriteTool};
use crate::tools::mcp_proxy::register_mcp_tools_from_file;
use crate::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::request_input::RequestInputTool;
use crate::tools::shell::{ShellPolicy, ShellTool};

pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
    default_tool_registry_with_shell_policy(workspace, ShellPolicy::default())
}

pub fn default_tool_registry_with_shell_policy(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    registry.register(Box::new(SaveMemoryTool::new()));
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        shell_policy,
    )));
    registry
}

pub async fn runtime_tool_registry(
    workspace: &Workspace,
    shell_policy: ShellPolicy,
    mcp_config_path: impl Into<std::path::PathBuf>,
) -> anyhow::Result<ToolRegistry> {
    let mut registry = default_tool_registry_with_shell_policy(workspace, shell_policy);
    register_mcp_tools_from_file(&mut registry, mcp_config_path).await?;
    Ok(registry)
}
