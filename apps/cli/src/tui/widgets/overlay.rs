use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::terminal::view::ToolCallStatus;
use crate::tui::keymap::key_bindings;
use crate::tui::state::{
    ModelPickerState, SessionPickerState, ToolDetailEntry, ToolDetailState, TuiOverlay,
};
use rove_runtime::conversation::MessageStatus;

const MAX_OVERLAY_WIDTH: u16 = 96;
const MAX_OVERLAY_HEIGHT: u16 = 26;

pub(crate) fn render_overlay(frame: &mut Frame<'_>, overlay: &TuiOverlay, viewport: Rect) {
    let area = overlay_area(viewport);
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(Clear, area);
    let accent = match overlay {
        TuiOverlay::SessionPicker(_) => Color::Cyan,
        TuiOverlay::ModelPicker(_) => Color::Green,
        TuiOverlay::ToolDetail(_) => Color::Blue,
        TuiOverlay::Help(_) => Color::Magenta,
        TuiOverlay::MessageQueue(_) => Color::Yellow,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(Color::Black))
        .title(overlay.title());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    match overlay {
        TuiOverlay::SessionPicker(picker) => render_session_picker(frame, picker, inner),
        TuiOverlay::ModelPicker(picker) => render_model_picker(frame, picker, inner),
        TuiOverlay::ToolDetail(detail) => render_tool_detail(frame, detail, inner),
        TuiOverlay::Help(help) => {
            let text = help_text();
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(inner.width)
                .saturating_sub(usize::from(inner.height));
            let max_scroll = u16::try_from(max_scroll).unwrap_or(u16::MAX);
            let paragraph = paragraph.scroll((help.scroll.min(max_scroll), 0));
            frame.render_widget(paragraph, inner);
        }
        TuiOverlay::MessageQueue(queue) => {
            let (body, footer) = split_footer(inner);
            let mut lines = Vec::new();
            if queue.messages.is_empty() {
                lines.push(Line::styled(
                    "No durable messages in this session.",
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                let visible = usize::from(body.height).max(1);
                let start = queue
                    .selected
                    .saturating_add(1)
                    .saturating_sub(visible)
                    .min(queue.messages.len().saturating_sub(visible));
                for (index, message) in queue.messages.iter().enumerate().skip(start).take(visible)
                {
                    let selected = index == queue.selected;
                    let marker = if selected { ">" } else { " " };
                    let status = match message.status {
                        MessageStatus::Queued => "queued",
                        MessageStatus::InterventionRequested => "promoted",
                        MessageStatus::AppliedCurrentRun => "applied",
                        MessageStatus::ClaimedSuccessor => "successor",
                        MessageStatus::NeedsAttention => "attention",
                        MessageStatus::Revoked => "revoked",
                    };
                    let style = if selected {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("{marker} {:<10} ", status), style),
                        Span::styled(
                            crate::tui::sanitize::sanitize_display_text(&message.content, 160),
                            style,
                        ),
                    ]));
                }
            }
            frame.render_widget(
                Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
                body,
            );
            frame.render_widget(
                Paragraph::new("Up/Down select  P promote  X revoke  Esc close")
                    .style(Style::default().fg(Color::Yellow)),
                footer,
            );
        }
    }
}

fn render_model_picker(frame: &mut Frame<'_>, picker: &ModelPickerState, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (body, footer) = split_footer(area);
    let mut lines = Vec::new();
    match picker {
        ModelPickerState::Loading { query } => lines.push(Line::styled(
            format!("Loading Provider catalog...  filter: {query}"),
            Style::default().fg(Color::Green),
        )),
        ModelPickerState::Ready {
            query,
            selected,
            error,
            persisting,
            ..
        } => {
            lines.push(Line::from(vec![
                Span::styled("Filter  ", Style::default().fg(Color::DarkGray)),
                Span::styled(query.clone(), Style::default().fg(Color::White)),
            ]));
            if let Some(error) = error {
                lines.push(Line::styled(
                    error.label(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
            }
            let visible_candidates = picker.visible_candidates();
            if visible_candidates.is_empty() && error.is_none() {
                lines.push(Line::styled(
                    "No matching Provider models.",
                    Style::default().fg(Color::DarkGray),
                ));
            }
            let reserved = 1 + usize::from(error.is_some());
            let visible = usize::from(body.height).saturating_sub(reserved).max(1);
            let start = selected
                .saturating_add(1)
                .saturating_sub(visible)
                .min(visible_candidates.len().saturating_sub(visible));
            for (index, candidate) in visible_candidates
                .iter()
                .enumerate()
                .skip(start)
                .take(visible)
            {
                let focused = index == *selected;
                let marker = if focused { ">" } else { " " };
                let current = if candidate.current { " current" } else { "" };
                let readiness = if candidate.credential_ready {
                    "ready"
                } else {
                    "credential missing"
                };
                let freshness = if candidate.inventory_fresh {
                    "inventory fresh"
                } else {
                    "configured"
                };
                let style = if focused {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(
                            "{marker} {}  {}  ",
                            candidate.selection.model, candidate.label
                        ),
                        style,
                    ),
                    Span::styled(
                        format!(
                            "{}  {}  {}{current}",
                            candidate.provider_type, readiness, freshness
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            if *persisting {
                lines.push(Line::styled(
                    "Saving selection...",
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        body,
    );
    frame.render_widget(
        Paragraph::new("Type filter  Up/Down select  Enter use next turn  Esc cancel")
            .style(Style::default().fg(Color::Green)),
        footer,
    );
}

pub(crate) fn overlay_area(viewport: Rect) -> Rect {
    if viewport.width == 0 || viewport.height == 0 {
        return Rect::new(viewport.x, viewport.y, 0, 0);
    }
    let horizontal_margin = if viewport.width >= 12 { 4 } else { 0 };
    let vertical_margin = if viewport.height >= 8 { 2 } else { 0 };
    let width = viewport
        .width
        .saturating_sub(horizontal_margin)
        .min(MAX_OVERLAY_WIDTH);
    let height = viewport
        .height
        .saturating_sub(vertical_margin)
        .min(MAX_OVERLAY_HEIGHT);
    Rect::new(
        viewport
            .x
            .saturating_add(viewport.width.saturating_sub(width) / 2),
        viewport
            .y
            .saturating_add(viewport.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn render_session_picker(frame: &mut Frame<'_>, picker: &SessionPickerState, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (body, footer) = split_footer(area);
    let mut lines = Vec::new();
    match picker {
        SessionPickerState::Loading => lines.push(Line::styled(
            "Loading resumable sessions...",
            Style::default().fg(Color::Cyan),
        )),
        SessionPickerState::Ready {
            candidates,
            selected,
            error,
            resolving,
        } => {
            if let Some(error) = error {
                lines.push(Line::styled(
                    error.label(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
            }
            if candidates.is_empty() {
                lines.push(Line::styled(
                    "No resumable task states found.",
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                let reserved = usize::from(error.is_some());
                let visible = usize::from(body.height).saturating_sub(reserved).max(1);
                let start = selected
                    .saturating_add(1)
                    .saturating_sub(visible)
                    .min(candidates.len().saturating_sub(visible));
                for (index, candidate) in candidates.iter().enumerate().skip(start).take(visible) {
                    let focused = index == *selected;
                    let marker = if focused { ">" } else { " " };
                    let style = if focused {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let run = candidate.run_id.to_string();
                    let short_run = run.get(..8).unwrap_or(run.as_str());
                    lines.push(Line::from(vec![
                        Span::styled(format!("{marker} {short_run}  "), style),
                        Span::styled(
                            format!("step {}  ", candidate.step),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(candidate.goal.replace(['\n', '\t'], " "), style),
                    ]));
                }
            }
            if let Some(run_id) = resolving {
                lines.push(Line::styled(
                    format!("Validating {}...", run_id),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        body,
    );
    frame.render_widget(
        Paragraph::new("Up/Down select  Enter resume  Esc cancel")
            .style(Style::default().fg(Color::Cyan)),
        footer,
    );
}

fn render_tool_detail(frame: &mut Frame<'_>, detail: &ToolDetailState, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (body, footer) = split_footer(area);
    let Some(entry) = detail.entries.get(detail.selected) else {
        frame.render_widget(
            Paragraph::new("No completed or failed tool calls yet.")
                .style(Style::default().fg(Color::DarkGray)),
            body,
        );
        frame.render_widget(
            Paragraph::new("Esc close").style(Style::default().fg(Color::Blue)),
            footer,
        );
        return;
    };

    let paragraph = Paragraph::new(tool_detail_text(
        entry,
        detail.selected,
        detail.entries.len(),
    ))
    .wrap(Wrap { trim: false });
    let max_scroll = if body.width == 0 || body.height == 0 {
        0
    } else {
        u16::try_from(
            paragraph
                .line_count(body.width)
                .saturating_sub(usize::from(body.height)),
        )
        .unwrap_or(u16::MAX)
    };
    let paragraph = paragraph.scroll((detail.scroll.min(max_scroll), 0));
    frame.render_widget(paragraph, body);
    frame.render_widget(
        Paragraph::new("Up/Down tool  PageUp/PageDown detail  Esc close")
            .style(Style::default().fg(Color::Blue)),
        footer,
    );
}

fn tool_detail_text(entry: &ToolDetailEntry, index: usize, total: usize) -> Text<'static> {
    let status = match entry.status {
        ToolCallStatus::Completed => "done",
        ToolCallStatus::Failed => "failed",
        ToolCallStatus::Started => "running",
        ToolCallStatus::WaitingApproval => "approval required",
        ToolCallStatus::Interrupted => "interrupted",
    };
    let status_color = if entry.status == ToolCallStatus::Completed {
        Color::Green
    } else {
        Color::Red
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("Tool {}/{}  ", index + 1, total),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(entry.name.clone(), Style::default().fg(Color::White)),
            Span::styled(format!(" [{status}]"), Style::default().fg(status_color)),
        ]),
        Line::styled(
            format!("Call  {}", entry.call_id),
            Style::default().fg(Color::DarkGray),
        ),
        Line::default(),
    ];
    push_section(&mut lines, "Arguments", &entry.args, Color::Cyan);
    if let Some(output) = &entry.output {
        lines.push(Line::default());
        push_section(&mut lines, "Output", output, Color::Green);
    }
    if let Some(error) = &entry.error {
        lines.push(Line::default());
        push_section(&mut lines, "Error", error, Color::Red);
    }
    Text::from(lines)
}

fn push_section(lines: &mut Vec<Line<'static>>, label: &str, value: &str, color: Color) {
    lines.push(Line::styled(
        label.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    if value.is_empty() {
        lines.push(Line::styled(
            "(empty)",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        lines.extend(value.lines().map(|line| Line::raw(line.to_string())));
    }
}

fn help_text() -> Text<'static> {
    let mut lines = vec![Line::styled(
        "Keyboard actions",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )];
    lines.extend(key_bindings().iter().map(|binding| {
        Line::from(vec![
            Span::styled(
                format!("{:<18}", binding.key),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{:<34}", binding.action)),
            Span::styled(binding.context, Style::default().fg(Color::DarkGray)),
        ])
    }));
    lines.push(Line::default());
    lines.push(Line::styled(
        "Slash commands: /model, /model current, /model <query>, /model reset",
        Style::default().fg(Color::Green),
    ));
    lines.push(Line::styled(
        "Esc closes this view. PageUp/PageDown scrolls.",
        Style::default().fg(Color::DarkGray),
    ));
    Text::from(lines)
}

fn split_footer(area: Rect) -> (Rect, Rect) {
    if area.height == 0 {
        return (area, area);
    }
    let footer = Rect::new(
        area.x,
        area.y.saturating_add(area.height - 1),
        area.width,
        1,
    );
    let body = Rect::new(area.x, area.y, area.width, area.height - 1);
    (body, footer)
}
