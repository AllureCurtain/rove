use async_trait::async_trait;

use crate::core::types::TerminationReason;
use crate::hooks::{PostRunHook, PostRunHookContext};
use crate::memory::session::write_session_summary_sync;

pub struct SessionMemoryHook;

#[async_trait]
impl PostRunHook for SessionMemoryHook {
    async fn after_run(&self, ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        if !matches!(&ctx.reason, TerminationReason::Final) {
            return Ok(());
        }

        if let Some(output) = ctx.output.as_deref() {
            write_session_summary_sync(ctx.workspace, ctx.session_id, output)?;
        }

        Ok(())
    }
}
