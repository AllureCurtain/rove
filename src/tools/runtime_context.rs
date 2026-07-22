use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::core::types::{ApprovalPolicy, CallId, ToolContext, UserInputProvider};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::memory::paths::MemoryPaths;

/// Runtime-owned services attached to an invocation without coupling
/// `rove-core` to workspace, memory, approval, or interface types.
#[derive(Clone)]
pub struct RuntimeToolServices {
    pub workspace: Workspace,
    pub memory_paths: MemoryPaths,
    pub approval_policy: ApprovalPolicy,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
}

pub fn runtime_tool_context<'a>(
    call_id: CallId,
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    approval_policy: ApprovalPolicy,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    cancel_token: CancellationToken,
) -> ToolContext<'a> {
    ToolContext::new(call_id, cancel_token).with_extension(Arc::new(RuntimeToolServices {
        workspace: workspace.clone(),
        memory_paths,
        approval_policy,
        input_provider,
    }))
}

pub fn runtime_tool_services<'a>(
    context: &'a ToolContext<'_>,
) -> Result<&'a RuntimeToolServices, ToolError> {
    context
        .extension::<RuntimeToolServices>()
        .ok_or_else(|| ToolError::ExecutionFailed {
            reason: "runtime tool services are not available in this embedding".to_string(),
        })
}
