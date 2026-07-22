use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::core::types::TerminationReason;
use crate::interfaces::terminal::view::{
    RunTimelineEntryKind, RunTimelinePlanStepStatus, RunTimelineToolStatus, RunViewState,
    ToolCallStatus,
};
use crate::interfaces::tui::sanitize::{
    sanitize_display_text, sanitize_tool_text, truncate_display_text,
};
use crate::interfaces::tui::state::{TuiFocus, TuiState};

use super::termination_label;

pub(crate) fn transcript(state: &TuiState, area: Rect, bordered: bool) -> Paragraph<'static> {
    let (max_offset, _) = transcript_viewport(state, area, bordered);
    let scroll = max_offset.saturating_sub(state.transcript_scroll.offset.min(max_offset));
    let paragraph = Paragraph::new(transcript_text(state))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    if bordered {
        let focused = state.focus == TuiFocus::Transcript;
        let border_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let title = if focused {
            " Transcript [focused] "
        } else {
            " Transcript "
        };
        paragraph.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        )
    } else {
        paragraph
    }
}

pub(crate) fn transcript_viewport(state: &TuiState, area: Rect, bordered: bool) -> (u16, u16) {
    let border_space = u16::from(bordered) * 2;
    let inner_width = area.width.saturating_sub(border_space);
    let inner_height = area.height.saturating_sub(border_space);
    let line_count = Paragraph::new(transcript_text(state))
        .wrap(Wrap { trim: false })
        .line_count(inner_width);
    let max_offset = line_count.saturating_sub(usize::from(inner_height));
    let max_offset = u16::try_from(max_offset).unwrap_or(u16::MAX);
    let page_size = inner_height.saturating_sub(1).max(1);
    (max_offset, page_size)
}

fn transcript_text(state: &TuiState) -> Text<'static> {
    let mut lines = Vec::new();

    for run in &state.run_history {
        push_run_lines(&mut lines, run);
        lines.push(Line::default());
    }
    push_run_lines(&mut lines, &state.run);

    if lines.is_empty() {
        lines.push(Line::styled(
            "No run yet. Type a prompt below.",
            Style::default().fg(Color::DarkGray),
        ));
    }

    Text::from(lines)
}

fn push_run_lines(lines: &mut Vec<Line<'static>>, run: &RunViewState) {
    if run.timeline_entries().is_empty() {
        push_legacy_run_lines(lines, run);
    } else {
        push_timeline_run_lines(lines, run);
    }
}

fn push_timeline_run_lines(lines: &mut Vec<Line<'static>>, run: &RunViewState) {
    let mut pending_assistant = String::new();
    let mut streamed_turn = String::new();
    let mut last_assistant = String::new();

    for entry in run.timeline_entries() {
        match &entry.kind {
            RunTimelineEntryKind::Assistant {
                text,
                final_message: false,
            } => {
                pending_assistant.push_str(text);
                streamed_turn.push_str(text);
            }
            RunTimelineEntryKind::Assistant {
                text,
                final_message: true,
            } => {
                flush_assistant(lines, &mut pending_assistant);
                if !assistant_text_equivalent(&streamed_turn, text) {
                    push_assistant(lines, text);
                    last_assistant = text.clone();
                } else {
                    last_assistant = streamed_turn.clone();
                }
                streamed_turn.clear();
            }
            kind => {
                flush_assistant(lines, &mut pending_assistant);
                push_timeline_kind(lines, kind, &streamed_turn, &last_assistant);
                if matches!(kind, RunTimelineEntryKind::Completion { .. }) {
                    streamed_turn.clear();
                } else if timeline_turn_boundary(kind) && !streamed_turn.is_empty() {
                    last_assistant = std::mem::take(&mut streamed_turn);
                }
            }
        }
    }
    flush_assistant(lines, &mut pending_assistant);
}

fn timeline_turn_boundary(kind: &RunTimelineEntryKind) -> bool {
    matches!(
        kind,
        RunTimelineEntryKind::Plan { .. }
            | RunTimelineEntryKind::PlanDecision { .. }
            | RunTimelineEntryKind::PlanRevision { .. }
            | RunTimelineEntryKind::PlanStep { .. }
            | RunTimelineEntryKind::Tool { .. }
            | RunTimelineEntryKind::Approval { .. }
            | RunTimelineEntryKind::Input { .. }
            | RunTimelineEntryKind::Compaction { .. }
            | RunTimelineEntryKind::Memory { .. }
    )
}

fn flush_assistant(lines: &mut Vec<Line<'static>>, pending: &mut String) {
    if pending.is_empty() {
        return;
    }
    push_assistant(lines, pending);
    pending.clear();
}

fn push_assistant(lines: &mut Vec<Line<'static>>, text: &str) {
    push_labeled_text(
        lines,
        "Assistant",
        &sanitize_legacy_text(text),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
}

fn assistant_text_equivalent(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && left == right
}

fn push_timeline_kind(
    lines: &mut Vec<Line<'static>>,
    kind: &RunTimelineEntryKind,
    streamed_turn: &str,
    last_assistant: &str,
) {
    match kind {
        RunTimelineEntryKind::User { message } => push_labeled_text(
            lines,
            "You",
            &sanitize_user_text(message),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        RunTimelineEntryKind::Assistant { .. } => {}
        RunTimelineEntryKind::ModelStatus { status, message } => push_labeled_text(
            lines,
            "Status",
            &sanitize_legacy_text(&format!("{status}: {message}")),
            Style::default().fg(Color::DarkGray),
        ),
        RunTimelineEntryKind::Plan { goal, step_count } => push_labeled_text(
            lines,
            "Plan",
            &sanitize_legacy_text(&format!("{goal} ({step_count} steps)")),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        RunTimelineEntryKind::PlanDecision { kind, summary } => push_labeled_text(
            lines,
            "Plan decision",
            &sanitize_legacy_text(&format!("{} - {summary}", plan_decision_label(*kind))),
            Style::default().fg(Color::Magenta),
        ),
        RunTimelineEntryKind::PlanRevision {
            revision,
            step_count,
            superseded_step_count,
        } => push_labeled_text(
            lines,
            "Plan revised",
            &format!(
                "revision {revision}: {step_count} remaining steps, {superseded_step_count} superseded"
            ),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        RunTimelineEntryKind::PlanStep {
            index,
            title,
            status,
            reason,
            ..
        } => {
            let (label, style) = timeline_plan_status(*status);
            let detail = reason
                .as_ref()
                .map(|reason| format!("{title} [{label}] - {reason}"))
                .unwrap_or_else(|| format!("{title} [{label}]"));
            push_labeled_text(
                lines,
                &format!("Step {}", index + 1),
                &sanitize_legacy_text(&detail),
                style,
            );
        }
        RunTimelineEntryKind::Tool {
            name,
            status,
            error_code,
            ..
        } => {
            let (label, style) = timeline_tool_status(*status);
            let detail = error_code
                .as_ref()
                .map(|code| format!("{name} [{label}: {code}]"))
                .unwrap_or_else(|| format!("{name} [{label}]"));
            push_labeled_text(
                lines,
                "Tool",
                &sanitize_legacy_text(&detail),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            );
            if matches!(status, RunTimelineToolStatus::Failed) {
                lines.push(Line::styled("  tool failed", style));
            }
        }
        RunTimelineEntryKind::Approval {
            tool_name, reason, ..
        } => push_labeled_text(
            lines,
            "Approval required",
            &sanitize_legacy_text(&format!("{tool_name} - {reason}")),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        RunTimelineEntryKind::Input { prompt, .. } => push_labeled_text(
            lines,
            "Input required",
            &sanitize_legacy_text(prompt),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        RunTimelineEntryKind::Compaction {
            mode,
            source_message_count,
            degraded,
            summary_available,
        } => push_labeled_text(
            lines,
            "Context",
            &format!(
                "{mode:?} compaction of {source_message_count} messages (summary: {summary_available}, degraded: {degraded})"
            ),
            Style::default().fg(Color::DarkGray),
        ),
        RunTimelineEntryKind::Memory { note_count } => push_labeled_text(
            lines,
            "Memory",
            &format!("flushed {note_count} notes"),
            Style::default().fg(Color::DarkGray),
        ),
        RunTimelineEntryKind::Completion { reason, output } => {
            let style = completion_style(reason);
            lines.push(Line::from(vec![
                Span::styled("Completed  ", style),
                Span::raw(termination_label(reason)),
            ]));
            if let Some(output) = output {
                let shown = if streamed_turn.trim().is_empty() {
                    last_assistant
                } else {
                    streamed_turn
                };
                if !assistant_text_equivalent(shown, output)
                    && !shown.trim().ends_with(output.trim())
                {
                    push_labeled_text(
                        lines,
                        "  Output",
                        &sanitize_legacy_text(output),
                        Style::default().fg(Color::White),
                    );
                }
            }
        }
    }
}

fn plan_decision_label(kind: crate::core::execution::PlanDecisionKind) -> &'static str {
    match kind {
        crate::core::execution::PlanDecisionKind::Continue => "continue",
        crate::core::execution::PlanDecisionKind::ReplaceRemaining => "replace remaining",
        crate::core::execution::PlanDecisionKind::Finish => "finish",
    }
}

fn timeline_plan_status(status: RunTimelinePlanStepStatus) -> (&'static str, Style) {
    match status {
        RunTimelinePlanStepStatus::Started => ("running", Style::default().fg(Color::Magenta)),
        RunTimelinePlanStepStatus::Completed => ("done", Style::default().fg(Color::Green)),
        RunTimelinePlanStepStatus::Failed => ("failed", Style::default().fg(Color::Red)),
    }
}

fn timeline_tool_status(status: RunTimelineToolStatus) -> (&'static str, Style) {
    match status {
        RunTimelineToolStatus::Started => ("running", Style::default().fg(Color::Cyan)),
        RunTimelineToolStatus::Completed => ("done", Style::default().fg(Color::Green)),
        RunTimelineToolStatus::Failed => ("failed", Style::default().fg(Color::Red)),
    }
}

fn completion_style(reason: &TerminationReason) -> Style {
    match reason {
        TerminationReason::Error => Style::default().fg(Color::Red),
        TerminationReason::Final => Style::default().fg(Color::Green),
        TerminationReason::Cancelled
        | TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => Style::default().fg(Color::Yellow),
    }
    .add_modifier(Modifier::BOLD)
}

fn push_legacy_run_lines(lines: &mut Vec<Line<'static>>, run: &RunViewState) {
    if let Some(message) = &run.user_message {
        push_labeled_text(
            lines,
            "You",
            &sanitize_user_text(message),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    if !run.assistant_text.is_empty() {
        push_labeled_text(
            lines,
            "Assistant",
            &sanitize_legacy_text(&run.assistant_text),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(plan) = &run.plan {
        let complete = plan.steps.iter().filter(|step| step.done).count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("Plan  {complete}/{} complete  ", plan.steps.len()),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(sanitize_legacy_text(&plan.goal)),
        ]));
        for (index, step) in plan.steps.iter().enumerate() {
            let marker = if step.done {
                "[x]"
            } else if index == plan.current_step {
                "[>]"
            } else {
                "[ ]"
            };
            let style = if step.done {
                Style::default().fg(Color::Green)
            } else if index == plan.current_step {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {marker} "), style),
                Span::raw(sanitize_legacy_text(&step.title)),
            ]));
        }
    }

    for tool in &run.tool_calls {
        let (status, style) = tool_status(tool.status);
        lines.push(Line::from(vec![
            Span::styled(
                "Tool  ",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(sanitize_legacy_text(&tool.name)),
            Span::styled(format!(" [{status}]"), style),
        ]));
        if let Some(output) = &tool.output {
            push_labeled_text(
                lines,
                "  Output",
                &sanitize_tool_text(
                    output,
                    crate::interfaces::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES,
                ),
                Style::default().fg(Color::DarkGray),
            );
        }
        if let Some(error) = &tool.error {
            push_labeled_text(
                lines,
                "  Failure",
                &sanitize_tool_text(
                    &error.to_string(),
                    crate::interfaces::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES,
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            );
        }
    }

    for (index, step, reason) in &run.failed_steps {
        push_labeled_text(
            lines,
            "Failure",
            &sanitize_legacy_text(&format!("step {} - {}: {reason}", index + 1, step.title)),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }
    for approval in &run.pending_approvals {
        push_labeled_text(
            lines,
            "Approval required",
            &sanitize_legacy_text(&format!("{} - {}", approval.name, approval.reason)),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    for input in &run.pending_inputs {
        push_labeled_text(
            lines,
            "Input required",
            &sanitize_legacy_text(&input.prompt),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(completed) = &run.completed {
        let completion_style = match completed.reason {
            TerminationReason::Error => Style::default().fg(Color::Red),
            TerminationReason::Final => Style::default().fg(Color::Green),
            TerminationReason::Cancelled
            | TerminationReason::StepLimit
            | TerminationReason::TokenLimit
            | TerminationReason::TimeLimit => Style::default().fg(Color::Yellow),
        }
        .add_modifier(Modifier::BOLD);
        lines.push(Line::from(vec![
            Span::styled("Completed  ", completion_style),
            Span::raw(termination_label(&completed.reason)),
        ]));
        if let Some(output) = &completed.output
            && (!matches!(completed.reason, TerminationReason::Final)
                || !sanitize_legacy_text(&run.assistant_text)
                    .ends_with(&sanitize_legacy_text(output)))
        {
            push_labeled_text(
                lines,
                "  Output",
                &sanitize_legacy_text(output),
                Style::default().fg(Color::White),
            );
        }
    }
}

fn sanitize_legacy_text(value: &str) -> String {
    sanitize_tool_text(
        value,
        crate::interfaces::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES,
    )
}

fn sanitize_user_text(value: &str) -> String {
    truncate_display_text(
        &sanitize_display_text(
            value,
            crate::interfaces::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES,
        ),
        crate::interfaces::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES,
    )
}

fn push_labeled_text(lines: &mut Vec<Line<'static>>, label: &str, text: &str, label_style: Style) {
    let mut parts = text.lines();
    let first = parts.next().unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(format!("{label}  "), label_style),
        Span::raw(first.to_string()),
    ]));
    let indent = " ".repeat(label.chars().count() + 2);
    for part in parts {
        lines.push(Line::from(vec![
            Span::raw(indent.clone()),
            Span::raw(part.to_string()),
        ]));
    }
}

fn tool_status(status: ToolCallStatus) -> (&'static str, Style) {
    match status {
        ToolCallStatus::Started => ("running", Style::default().fg(Color::Cyan)),
        ToolCallStatus::WaitingApproval => {
            ("approval required", Style::default().fg(Color::Yellow))
        }
        ToolCallStatus::Completed => ("done", Style::default().fg(Color::Green)),
        ToolCallStatus::Failed => ("failed", Style::default().fg(Color::Red)),
        ToolCallStatus::Interrupted => ("interrupted", Style::default().fg(Color::Yellow)),
    }
}
