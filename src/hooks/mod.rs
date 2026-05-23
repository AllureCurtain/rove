use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::core::types::{JobId, RunId, SessionId, TerminationReason, ToolContext, ToolResult};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;

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
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
    pub reason: TerminationReason,
    pub output: Option<String>,
    pub cancel_token: CancellationToken,
}

#[async_trait]
pub trait PostRunHook: Send + Sync {
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
            tokio::select! {
                biased;
                _ = ctx.cancel_token.cancelled() => {
                    tracing::warn!(
                        run_id = %ctx.run_id,
                        "post-run hook execution cancelled"
                    );
                    return;
                }
                result = hook.after_run(ctx) => {
                    if let Err(err) = result {
                        tracing::warn!(
                            run_id = %ctx.run_id,
                            "post-run hook failed: {err}"
                        );
                    }
                }
            }
        }
    }
}
