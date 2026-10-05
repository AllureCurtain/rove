use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::terminal::view::ToolCallStatus;
use crate::tui::sanitize::{sanitize_display_text, sanitize_tool_text, truncate_display_text};
use crate::tui::state::{ProviderStatus, RunLifecycle, TuiFocus, TuiState};
use rove_runtime::types::TerminationReason;

use super::termination_label;

pub(crate) fn activity(state: &TuiState, bordered: bool) -> Paragraph<'static> {
    let (label, detail, style) = activity_content(state);
    let line = Line::from(vec![
        Span::styled(format!("{label}  "), style.add_modifier(Modifier::BOLD)),
        Span::styled(
            sanitize_activity_text(&detail),
            Style::default().fg(Color::White),
        ),
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

pub(crate) fn composer(state: &TuiState, area: Rect, bordered: bool) -> Paragraph<'static> {
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
        Span::raw(sanitize_composer_text(&state.composer))
    };
    let caret = if focused {
        Span::styled(
            " ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::REVERSED),
        )
    } else {
        Span::raw("")
    };
    let paragraph = Paragraph::new(Line::from(vec![Span::styled("> ", accent), content, caret]))
        .wrap(Wrap { trim: false });
    let border_space = u16::from(bordered) * 2;
    let inner_width = area.width.saturating_sub(border_space);
    let inner_height = area.height.saturating_sub(border_space);
    let scroll = paragraph
        .line_count(inner_width)
        .saturating_sub(usize::from(inner_height));
    let scroll = u16::try_from(scroll).unwrap_or(u16::MAX);
    let paragraph = paragraph.scroll((scroll, 0));

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
    let queued = state.eligible_message_count();
    let model = state
        .model_selection
        .as_ref()
        .map(|selection| selection.model.as_str())
        .unwrap_or("unconfigured");
    let provider = match state.provider_status {
        ProviderStatus::Ready => "ready",
        ProviderStatus::OnboardingRequired => "setup required",
        ProviderStatus::Testing => "testing",
        ProviderStatus::RecoverableError => "needs attention",
    };
    let content = if width >= 112 {
        let workspace = compact_tail(&state.workspace_root, 18);
        let workspace_kind = compact_tail(&state.workspace_kind, 10);
        let model = compact_tail(model, 22);
        let provider = compact_tail(provider, 8);
        let session = compact_tail(&state.session_id, 8);
        let run = state
            .run
            .run_id
            .map(|run_id| compact_tail(&run_id.to_string(), 8))
            .unwrap_or_else(|| "-".to_string());
        format!(
            " ws:{workspace} ({workspace_kind}) | model:{model} | p:{provider} | s:{session} | id:{run} | {run_status} | q:{queued}"
        )
    } else if width >= 72 {
        let workspace = compact_tail(&state.workspace_root, 20);
        let session = compact_tail(&state.session_id, 8);
        format!(" ws:{workspace} | p:{provider} | s:{session} | run:{run_status} | q:{queued}")
    } else if width >= 36 {
        let workspace = compact_tail(&state.workspace_root, usize::from(width / 3).max(12));
        format!(" ws:{workspace} | run:{run_status} | q:{queued} | focus:{focus}")
    } else {
        format!("run:{run_status} | q:{queued}")
    };

    Paragraph::new(Span::styled(content, status_style)).style(Style::default().bg(Color::Black))
}

fn compact_tail(value: &str, max_chars: usize) -> String {
    let sanitized = sanitize_display_text(value, 2048);
    let count = sanitized.chars().count();
    if count <= max_chars {
        return sanitized;
    }
    let keep = max_chars.saturating_sub(3);
    let tail = sanitized
        .chars()
        .skip(count.saturating_sub(keep))
        .collect::<String>();
    format!("...{tail}")
}

pub(crate) fn minimal_line(state: &TuiState) -> Paragraph<'static> {
    let (run_status, _) = run_status(state);
    let draft = if state.composer.is_empty() {
        "compose"
    } else {
        state.composer.as_str()
    };
    Paragraph::new(format!(
        "> {} | {run_status}",
        sanitize_composer_text(draft)
    ))
    .style(Style::default().fg(Color::Cyan))
}

fn sanitize_activity_text(value: &str) -> String {
    sanitize_tool_text(value, crate::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES)
}

fn sanitize_composer_text(value: &str) -> String {
    truncate_display_text(
        &sanitize_display_text(value, crate::tui::state::MAX_COMPOSER_BYTES),
        crate::tui::state::MAX_COMPOSER_BYTES,
    )
}

fn activity_content(state: &TuiState) -> (&'static str, String, Style) {
    let run = &state.run;
    if state.run_lifecycle == RunLifecycle::Cancelling {
        return (
            "Run",
            "cancellation requested".to_string(),
            Style::default().fg(Color::Yellow),
        );
    }
    if state.run_lifecycle == RunLifecycle::Completed
        && let Some(completed) = &run.completed
    {
        let style = match completed.reason {
            TerminationReason::Error => Style::default().fg(Color::Red),
            TerminationReason::Final => Style::default().fg(Color::Green),
            TerminationReason::Cancelled
            | TerminationReason::StepLimit
            | TerminationReason::TokenLimit
            | TerminationReason::TimeLimit => Style::default().fg(Color::Yellow),
        };
        return (
            "Done",
            termination_label(&completed.reason).to_string(),
            style,
        );
    }
    if let Some(notice) = &state.model_notice {
        return ("Model", notice.clone(), Style::default().fg(Color::Cyan));
    }
    if state.provider_status == ProviderStatus::OnboardingRequired {
        return (
            "Provider",
            "configuration required; use /provider".to_string(),
            Style::default().fg(Color::Yellow),
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
    if let Some(tool) = run.tool_calls.iter().rev().find(|tool| {
        matches!(
            tool.status,
            ToolCallStatus::Started | ToolCallStatus::WaitingApproval
        )
    }) {
        return (
            "Tool",
            format!("{} - {}", tool.name, tool_status(tool.status)),
            tool_style(tool.status),
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
    if state.run_lifecycle == RunLifecycle::Running {
        return (
            "Run",
            if run.run_id.is_some() {
                "waiting for events".to_string()
            } else {
                "starting".to_string()
            },
            Style::default().fg(Color::Cyan),
        );
    }
    if let Some(resume) = &state.active_resume {
        return (
            "Resume",
            format!(
                "{} - step {}",
                resume.goal.replace(['\n', '\t'], " "),
                resume.step
            ),
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
    if state.quit_confirmation {
        return ("confirm exit", Style::default().fg(Color::Yellow));
    }
    match state.run_lifecycle {
        RunLifecycle::Idle => ("idle", Style::default().fg(Color::DarkGray)),
        RunLifecycle::Cancelling => ("cancelling", Style::default().fg(Color::Yellow)),
        RunLifecycle::Running if !state.run.pending_approvals.is_empty() => {
            ("approval", Style::default().fg(Color::Yellow))
        }
        RunLifecycle::Running if !state.run.pending_inputs.is_empty() => {
            ("input", Style::default().fg(Color::Yellow))
        }
        RunLifecycle::Running => ("running", Style::default().fg(Color::Cyan)),
        RunLifecycle::Completed => match state.run.completed.as_ref().map(|view| &view.reason) {
            Some(TerminationReason::Final) => ("done", Style::default().fg(Color::Green)),
            Some(TerminationReason::Error) => ("error", Style::default().fg(Color::Red)),
            Some(TerminationReason::Cancelled) => ("cancelled", Style::default().fg(Color::Yellow)),
            Some(TerminationReason::StepLimit) => {
                ("step limit", Style::default().fg(Color::Yellow))
            }
            Some(TerminationReason::TokenLimit) => {
                ("token limit", Style::default().fg(Color::Yellow))
            }
            Some(TerminationReason::TimeLimit) => {
                ("time limit", Style::default().fg(Color::Yellow))
            }
            None => ("completed", Style::default().fg(Color::Yellow)),
        },
    }
}

fn tool_status(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Started => "running",
        ToolCallStatus::WaitingApproval => "approval required",
        ToolCallStatus::Completed => "done",
        ToolCallStatus::Failed => "failed",
        ToolCallStatus::Interrupted => "interrupted",
    }
}

fn tool_style(status: ToolCallStatus) -> Style {
    match status {
        ToolCallStatus::Started => Style::default().fg(Color::Cyan),
        ToolCallStatus::WaitingApproval => Style::default().fg(Color::Yellow),
        ToolCallStatus::Completed => Style::default().fg(Color::Green),
        ToolCallStatus::Failed => Style::default().fg(Color::Red),
        ToolCallStatus::Interrupted => Style::default().fg(Color::Yellow),
    }
}
