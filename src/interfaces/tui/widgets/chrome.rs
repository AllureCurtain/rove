use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::core::types::TerminationReason;
use crate::interfaces::terminal::view::{RunViewState, ToolCallStatus};
use crate::interfaces::tui::state::{TuiFocus, TuiState};

use super::termination_label;

pub(crate) fn activity(run: &RunViewState, bordered: bool) -> Paragraph<'static> {
    let (label, detail, style) = activity_content(run);
    let line = Line::from(vec![
        Span::styled(format!("{label}  "), style.add_modifier(Modifier::BOLD)),
        Span::styled(detail, Style::default().fg(Color::White)),
    ]);
    let paragraph = Paragraph::new(line).wrap(Wrap { trim: false });

    if bordered {
        paragraph.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Activity "),
        )
    } else {
        paragraph
    }
}

pub(crate) fn composer(state: &TuiState, bordered: bool) -> Paragraph<'static> {
    let focused = state.focus == TuiFocus::Composer;
    let accent = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let content = if state.composer.is_empty() {
        Span::styled(
            "Type a prompt...".to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )
    } else {
        Span::raw(state.composer.clone())
    };
    let paragraph = Paragraph::new(Line::from(vec![Span::styled("> ", accent), content]))
        .wrap(Wrap { trim: false });

    if bordered {
        let title = if focused {
            " Composer [focused] "
        } else {
            " Composer "
        };
        paragraph.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent)
                .title(title),
        )
    } else {
        paragraph
    }
}

pub(crate) fn status_line(state: &TuiState, width: u16) -> Paragraph<'static> {
    let (run_status, status_style) = run_status(state);
    let focus = match state.focus {
        TuiFocus::Transcript => "transcript",
        TuiFocus::Composer => "composer",
    };
    let content = if width >= 72 {
        format!(" workspace: - | run: {run_status} | focus: {focus} | Tab focus | Ctrl+Q quit")
    } else if width >= 36 {
        format!(" ws:- | run:{run_status} | focus:{focus}")
    } else {
        format!("run:{run_status} | {focus}")
    };

    Paragraph::new(Span::styled(content, status_style)).style(Style::default().bg(Color::Black))
}

pub(crate) fn minimal_line(state: &TuiState) -> Paragraph<'static> {
    let (run_status, _) = run_status(state);
    let draft = if state.composer.is_empty() {
        "compose"
    } else {
        state.composer.as_str()
    };
    Paragraph::new(format!("> {draft} | {run_status}")).style(Style::default().fg(Color::Cyan))
}

fn activity_content(run: &RunViewState) -> (&'static str, String, Style) {
    if let Some(completed) = &run.completed {
        let style = match completed.reason {
            TerminationReason::Error | TerminationReason::Cancelled => {
                Style::default().fg(Color::Red)
            }
            _ => Style::default().fg(Color::Green),
        };
        return (
            "Done",
            termination_label(&completed.reason).to_string(),
            style,
        );
    }
    if let Some(approval) = run.pending_approvals.last() {
        return (
            "Approval",
            format!("{} - {}", approval.name, approval.reason),
            Style::default().fg(Color::Yellow),
        );
    }
    if let Some(input) = run.pending_inputs.last() {
        return (
            "Input",
            input.prompt.clone(),
            Style::default().fg(Color::Yellow),
        );
    }
    if let Some((status, message)) = &run.model_status {
        return (
            "Model",
            format!("{status} - {message}"),
            Style::default().fg(Color::Cyan),
        );
    }
    if let Some((index, step)) = &run.current_step {
        let total = run.plan.as_ref().map_or(0, |plan| plan.steps.len());
        return (
            "Plan",
            format!(
                "step {}/{} - {}",
                index + 1,
                total.max(index + 1),
                step.title
            ),
            Style::default().fg(Color::Magenta),
        );
    }
    if let Some(tool) = run.tool_calls.last() {
        return (
            "Tool",
            format!("{} - {}", tool.name, tool_status(tool.status)),
            tool_style(tool.status),
        );
    }
    if run.run_id.is_some() {
        return (
            "Run",
            "waiting for events".to_string(),
            Style::default().fg(Color::Cyan),
        );
    }
    (
        "Idle",
        "ready for a prompt".to_string(),
        Style::default().fg(Color::DarkGray),
    )
}

fn run_status(state: &TuiState) -> (&'static str, Style) {
    if let Some(completed) = &state.run.completed {
        return match completed.reason {
            TerminationReason::Error => ("error", Style::default().fg(Color::Red)),
            TerminationReason::Cancelled => ("cancelled", Style::default().fg(Color::Yellow)),
            _ => ("done", Style::default().fg(Color::Green)),
        };
    }
    if !state.run.pending_approvals.is_empty() {
        return ("approval", Style::default().fg(Color::Yellow));
    }
    if !state.run.pending_inputs.is_empty() {
        return ("input", Style::default().fg(Color::Yellow));
    }
    if state.run.run_id.is_some() {
        return ("running", Style::default().fg(Color::Cyan));
    }
    ("idle", Style::default().fg(Color::DarkGray))
}

fn tool_status(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Started => "running",
        ToolCallStatus::WaitingApproval => "approval required",
        ToolCallStatus::Completed => "done",
        ToolCallStatus::Failed => "failed",
    }
}

fn tool_style(status: ToolCallStatus) -> Style {
    match status {
        ToolCallStatus::Started => Style::default().fg(Color::Cyan),
        ToolCallStatus::WaitingApproval => Style::default().fg(Color::Yellow),
        ToolCallStatus::Completed => Style::default().fg(Color::Green),
        ToolCallStatus::Failed => Style::default().fg(Color::Red),
    }
}
