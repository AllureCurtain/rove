use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::core::events::StreamEvent;
use crate::core::types::{JobId, RunId, SessionId, TerminationReason, ToolContext, ToolResult};
use crate::core::types::{PlanStep, ToolMutation};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::memory::paths::MemoryPaths;

mod session_memory;

pub use session_memory::SessionMemoryHook;

#[async_trait]
pub trait PreToolHook: Send + Sync {
    async fn before_tool(
        &self,
        ctx: &ToolContext<'_>,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), ToolError>;
}

pub struct PostToolHookContext<'a> {
    pub tool_context: &'a ToolContext<'a>,
    pub name: &'a str,
    pub args: &'a serde_json::Value,
    pub result: &'a ToolResult,
}

#[async_trait]
pub trait PostToolHook: Send + Sync {
    async fn after_tool(&self, ctx: &PostToolHookContext<'_>) -> Result<(), ToolError>;
}

pub struct PostRunHookContext<'a> {
    pub workspace: &'a Workspace,
    pub memory_paths: &'a MemoryPaths,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub reason: TerminationReason,
    pub output: Option<String>,
    pub summary: RunSummary,
    pub cancel_token: CancellationToken,
}

#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    pub goal: String,
    pub completed_plan_steps: Vec<PlanStep>,
    pub tools_used: Vec<String>,
    pub tool_mutations: Vec<ToolMutation>,
}

impl RunSummary {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            completed_plan_steps: Vec::new(),
            tools_used: Vec::new(),
            tool_mutations: Vec::new(),
        }
    }

    pub fn record_event(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::ToolCallStarted { name, .. }
                if !self.tools_used.iter().any(|tool| tool == name) =>
            {
                self.tools_used.push(name.clone());
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                self.tool_mutations.extend(result.mutations.clone());
            }
            StreamEvent::PlanStepCompleted { step, .. } => {
                self.completed_plan_steps.push(step.clone());
            }
            _ => {}
        }
    }
}

#[async_trait]
pub trait PostRunHook: Send + Sync {
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn after_run(&self, ctx: &PostRunHookContext<'_>) -> anyhow::Result<()>;
}

#[derive(Clone, Default)]
pub struct HookRegistry {
    pre_tool: Vec<Arc<dyn PreToolHook>>,
    post_tool: Vec<Arc<dyn PostToolHook>>,
    post_run: Vec<Arc<dyn PostRunHook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_post_run_hooks() -> Self {
        Self::default().with_post_run(Box::new(SessionMemoryHook))
    }

    pub fn with_pre_tool(mut self, hook: Box<dyn PreToolHook>) -> Self {
        self.register_pre_tool(hook);
        self
    }

    pub fn with_post_tool(mut self, hook: Box<dyn PostToolHook>) -> Self {
        self.register_post_tool(hook);
        self
    }

    pub fn with_post_run(mut self, hook: Box<dyn PostRunHook>) -> Self {
        self.register_post_run(hook);
        self
    }

    pub fn register_pre_tool(&mut self, hook: Box<dyn PreToolHook>) {
        self.pre_tool.push(Arc::from(hook));
    }

    pub fn register_post_tool(&mut self, hook: Box<dyn PostToolHook>) {
        self.post_tool.push(Arc::from(hook));
    }

    pub fn register_post_run(&mut self, hook: Box<dyn PostRunHook>) {
        self.post_run.push(Arc::from(hook));
    }

    pub async fn run_pre_tool(
        &self,
        ctx: &ToolContext<'_>,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), ToolError> {
        for hook in &self.pre_tool {
            hook.before_tool(ctx, name, args).await?;
        }
        Ok(())
    }

    pub async fn run_post_tool(&self, ctx: &PostToolHookContext<'_>) {
        for hook in &self.post_tool {
            if let Err(err) = hook.after_tool(ctx).await {
                tracing::warn!("post-tool hook failed for {}: {}", ctx.name, err);
            }
        }
    }

    pub async fn run_post_run(&self, ctx: &PostRunHookContext<'_>) {
        for hook in &self.post_run {
            let timeout = hook.timeout();
            let hook_future = AssertUnwindSafe(hook.after_run(ctx)).catch_unwind();
            tokio::select! {
                biased;
                _ = ctx.cancel_token.cancelled() => {
                    tracing::warn!(
                        run_id = %ctx.run_id,
                        "post-run hook execution cancelled"
                    );
                    return;
                }
                result = tokio::time::timeout(timeout, hook_future) => {
                    match result {
                        Ok(Ok(Ok(()))) => {}
                        Ok(Ok(Err(err))) => {
                            tracing::warn!(
                                run_id = %ctx.run_id,
                                "post-run hook failed: {err}"
                            );
                        }
                        Ok(Err(_panic)) => {
                            tracing::error!(
                                run_id = %ctx.run_id,
                                "post-run hook panicked"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                run_id = %ctx.run_id,
                                timeout_ms = timeout.as_millis(),
                                "post-run hook timed out"
                            );
                        }
                    }
                }
            }
        }
    }
}
