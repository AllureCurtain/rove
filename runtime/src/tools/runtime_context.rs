use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::environment::{ExecutionEnvironment, local_environment};
use crate::memory::paths::MemoryPaths;
use crate::state::tool_artifacts::ToolArtifactStore;
use crate::types::{ApprovalPolicy, RunMode, UserInputProvider};
use crate::workspace::Workspace;
use rove_core::{CallId, ToolContext, ToolError};

/// Runtime-owned services attached to an invocation without coupling
/// `rove-core` to workspace, memory, approval, or interface types.
#[derive(Clone)]
pub struct RuntimeToolServices {
    pub workspace: Workspace,
    pub environment: Arc<dyn ExecutionEnvironment>,
    pub memory_paths: MemoryPaths,
    pub approval_policy: ApprovalPolicy,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    /// Durable Tool Artifact authority for the current run.
    ///
    /// `None` in an embedding with no run directory. A tool that needs to
    /// retain a payload must refuse it rather than inline it when this is
    /// absent, so a missing store degrades safely instead of leaking bytes
    /// into a prompt. This is the durable authority and is deliberately
    /// distinct from `environment.artifacts()`, which is a process-local
    /// Coding Tool projection store.
    pub tool_artifacts: Option<Arc<ToolArtifactStore>>,
    /// Immutable host-selected execution profile. Tools cannot change it.
    #[allow(dead_code)]
    pub run_mode: RunMode,
}

pub fn runtime_tool_context<'a>(
    call_id: CallId,
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    approval_policy: ApprovalPolicy,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    cancel_token: CancellationToken,
) -> ToolContext<'a> {
    runtime_tool_context_with_environment(
        call_id,
        workspace,
        memory_paths,
        approval_policy,
        input_provider,
        cancel_token,
        local_environment(workspace),
    )
}

pub fn runtime_tool_context_with_environment<'a>(
    call_id: CallId,
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    approval_policy: ApprovalPolicy,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    cancel_token: CancellationToken,
    environment: Arc<dyn ExecutionEnvironment>,
) -> ToolContext<'a> {
    runtime_tool_context_with_artifacts(
        call_id,
        workspace,
        memory_paths,
        approval_policy,
        input_provider,
        cancel_token,
        environment,
        None,
    )
}

/// Builds a context with an explicit execution profile while preserving the
/// historic helper signatures used by embedded callers.
#[allow(clippy::too_many_arguments)]
pub fn runtime_tool_context_with_mode_and_artifacts<'a>(
    call_id: CallId,
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    approval_policy: ApprovalPolicy,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    cancel_token: CancellationToken,
    environment: Arc<dyn ExecutionEnvironment>,
    tool_artifacts: Option<Arc<ToolArtifactStore>>,
    run_mode: RunMode,
) -> ToolContext<'a> {
    ToolContext::new(call_id, cancel_token).with_extension(Arc::new(RuntimeToolServices {
        workspace: workspace.clone(),
        environment,
        memory_paths,
        approval_policy,
        input_provider,
        tool_artifacts,
        run_mode,
    }))
}

/// Builds an invocation context that can also retain durable Tool Artifacts.
///
/// Separate from [`runtime_tool_context_with_environment`] so an embedding
/// without a run directory keeps compiling and simply has no artifact
/// authority, rather than being handed a store that writes somewhere
/// arbitrary.
#[allow(clippy::too_many_arguments)]
pub fn runtime_tool_context_with_artifacts<'a>(
    call_id: CallId,
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    approval_policy: ApprovalPolicy,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    cancel_token: CancellationToken,
    environment: Arc<dyn ExecutionEnvironment>,
    tool_artifacts: Option<Arc<ToolArtifactStore>>,
) -> ToolContext<'a> {
    runtime_tool_context_with_mode_and_artifacts(
        call_id,
        workspace,
        memory_paths,
        approval_policy,
        input_provider,
        cancel_token,
        environment,
        tool_artifacts,
        RunMode::Normal,
    )
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
