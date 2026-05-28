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
use crate::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::request_input::RequestInputTool;
use crate::tools::shell::ShellTool;

pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
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
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));
    registry
}
