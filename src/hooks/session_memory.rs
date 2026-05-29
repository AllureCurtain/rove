use async_trait::async_trait;

use crate::core::types::{TerminationReason, ToolMutationOperation};
use crate::hooks::{PostRunHook, PostRunHookContext};
use crate::memory::session::write_session_summary_to_dir_sync;

pub struct SessionMemoryHook;

#[async_trait]
impl PostRunHook for SessionMemoryHook {
    async fn after_run(&self, ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        if !matches!(&ctx.reason, TerminationReason::Final) {
            return Ok(());
        }

        let summary = deterministic_session_summary(ctx);
        write_session_summary_to_dir_sync(&ctx.memory_paths.session_dir, ctx.session_id, &summary)?;

        Ok(())
    }
}

fn deterministic_session_summary(ctx: &PostRunHookContext<'_>) -> String {
    let mut lines = Vec::new();
    lines.push("# Session Summary".to_string());
    lines.push(format!("- Goal: {}", one_line(&ctx.summary.goal)));
    lines.push(format!("- Status: {}", reason_label(&ctx.reason)));
    lines.push(format!(
        "- Output: {}",
        ctx.output
            .as_deref()
            .map(one_line)
            .filter(|output| !output.is_empty())
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "- Completed plan steps: {}",
        completed_plan_steps(ctx)
    ));
    lines.push(format!("- Tools used: {}", tools_used(ctx)));
    lines.push(format!("- Files changed: {}", files_changed(ctx)));
    lines.join("\n")
}

fn reason_label(reason: &TerminationReason) -> &'static str {
    match reason {
        TerminationReason::Final => "final",
        TerminationReason::StepLimit => "step_limit",
        TerminationReason::TokenLimit => "token_limit",
        TerminationReason::TimeLimit => "time_limit",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    }
}

fn completed_plan_steps(ctx: &PostRunHookContext<'_>) -> String {
    if ctx.summary.completed_plan_steps.is_empty() {
        return "none".to_string();
    }
    ctx.summary
        .completed_plan_steps
        .iter()
        .map(|step| format!("{} {}", one_line(&step.id), one_line(&step.title)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn tools_used(ctx: &PostRunHookContext<'_>) -> String {
    if ctx.summary.tools_used.is_empty() {
        "none".to_string()
    } else {
        ctx.summary.tools_used.join(", ")
    }
}

fn files_changed(ctx: &PostRunHookContext<'_>) -> String {
    if ctx.summary.tool_mutations.is_empty() {
        return "none".to_string();
    }
    ctx.summary
        .tool_mutations
        .iter()
        .map(|mutation| {
            format!(
                "{} ({})",
                one_line(&mutation.path),
                mutation_operation_label(&mutation.operation)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn mutation_operation_label(operation: &ToolMutationOperation) -> &'static str {
    match operation {
        ToolMutationOperation::Create => "create",
        ToolMutationOperation::Update => "update",
        ToolMutationOperation::Delete => "delete",
        ToolMutationOperation::Unknown => "unknown",
    }
}

fn one_line(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let joined = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 240;
    if joined.chars().count() <= MAX_CHARS {
        joined
    } else {
        joined.chars().take(MAX_CHARS).collect()
    }
}
