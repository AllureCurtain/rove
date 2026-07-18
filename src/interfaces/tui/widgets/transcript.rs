use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::core::types::TerminationReason;
use crate::interfaces::terminal::view::{RunViewState, ToolCallStatus};
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
    if let Some(message) = &run.user_message {
        push_labeled_text(
            lines,
            "You",
            message,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    if !run.assistant_text.is_empty() {
        push_labeled_text(
            lines,
            "Assistant",
            &run.assistant_text,
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
            Span::raw(plan.goal.clone()),
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
                Span::raw(step.title.clone()),
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
            Span::raw(tool.name.clone()),
            Span::styled(format!(" [{status}]"), style),
        ]));
        if let Some(output) = &tool.output {
            push_labeled_text(
                lines,
                "  Output",
                output,
                Style::default().fg(Color::DarkGray),
            );
        }
        if let Some(error) = &tool.error {
            push_labeled_text(
                lines,
                "  Failure",
                &error.to_string(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            );
        }
    }

    for (index, step, reason) in &run.failed_steps {
        push_labeled_text(
            lines,
            "Failure",
            &format!("step {} - {}: {reason}", index + 1, step.title),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }
    for approval in &run.pending_approvals {
        push_labeled_text(
            lines,
            "Approval required",
            &format!("{} - {}", approval.name, approval.reason),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    for input in &run.pending_inputs {
        push_labeled_text(
            lines,
            "Input required",
            &input.prompt,
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
                || !run.assistant_text.ends_with(output))
        {
            push_labeled_text(lines, "  Output", output, Style::default().fg(Color::White));
        }
    }
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
