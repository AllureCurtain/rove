use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::interfaces::tui::state::{InteractionKeyMode, InteractionModalView};

const MAX_MODAL_WIDTH: u16 = 88;
const MAX_APPROVAL_HEIGHT: u16 = 16;
const MAX_INPUT_HEIGHT: u16 = 14;
const NORMAL_MIN_WIDTH: u16 = 28;
const NORMAL_MIN_HEIGHT: u16 = 8;

pub(crate) fn render_modal(
    frame: &mut Frame<'_>,
    modal: &InteractionModalView,
    viewport: Rect,
    interaction_key_mode: InteractionKeyMode,
    approval_confirmation: bool,
) {
    let area = modal_area(viewport, modal);
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(Clear, area);
    if area.width < NORMAL_MIN_WIDTH || area.height < NORMAL_MIN_HEIGHT {
        render_compact(
            frame,
            modal,
            area,
            interaction_key_mode,
            approval_confirmation,
        );
        return;
    }

    match modal {
        InteractionModalView::Approval {
            name, args, reason, ..
        } => render_approval(
            frame,
            area,
            name,
            args,
            reason,
            interaction_key_mode,
            approval_confirmation,
        ),
        InteractionModalView::Input { prompt, draft, .. } => {
            render_input(frame, area, prompt, draft, interaction_key_mode);
        }
    }
}

pub(crate) fn modal_area(viewport: Rect, modal: &InteractionModalView) -> Rect {
    if viewport.width == 0 || viewport.height == 0 {
        return Rect::new(viewport.x, viewport.y, 0, 0);
    }

    let horizontal_margin = if viewport.width >= 12 { 4 } else { 0 };
    let vertical_margin = if viewport.height >= 8 { 2 } else { 0 };
    let max_height = match modal {
        InteractionModalView::Approval { .. } => MAX_APPROVAL_HEIGHT,
        InteractionModalView::Input { .. } => MAX_INPUT_HEIGHT,
    };
    let width = viewport
        .width
        .saturating_sub(horizontal_margin)
        .min(MAX_MODAL_WIDTH);
    let height = viewport
        .height
        .saturating_sub(vertical_margin)
        .min(max_height);
    let x = viewport
        .x
        .saturating_add(viewport.width.saturating_sub(width) / 2);
    let y = viewport
        .y
        .saturating_add(viewport.height.saturating_sub(height) / 2);

    Rect::new(x, y, width, height)
}

fn render_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    name: &str,
    args: &serde_json::Value,
    reason: &str,
    interaction_key_mode: InteractionKeyMode,
    approval_confirmation: bool,
) {
    let accent = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let block = modal_block(" Approval required ", accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (body, actions) = split_actions(inner);
    let tool_height = wrapped_height(&labeled_text("Tool", name, accent), body.width)
        .min(2)
        .min(body.height.saturating_sub(2))
        .max(1);
    let tool_area = take_top(body, tool_height);
    let after_tool = trim_top(body, tool_height);
    let reason_height = wrapped_height(&labeled_text("Reason", reason, accent), body.width)
        .min(3)
        .min(after_tool.height.saturating_sub(1))
        .max(1);
    let reason_area = take_top(after_tool, reason_height);
    let arguments_area = trim_top(after_tool, reason_height);

    frame.render_widget(
        Paragraph::new(labeled_text("Tool", name, accent)).wrap(Wrap { trim: false }),
        tool_area,
    );
    frame.render_widget(
        Paragraph::new(labeled_text("Reason", reason, accent)).wrap(Wrap { trim: false }),
        reason_area,
    );

    let arguments = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
    frame.render_widget(
        Paragraph::new(labeled_text("Arguments", &arguments, accent)).wrap(Wrap { trim: false }),
        arguments_area,
    );
    frame.render_widget(
        approval_actions(actions.width, interaction_key_mode, approval_confirmation),
        actions,
    );
}

fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &str,
    draft: &str,
    interaction_key_mode: InteractionKeyMode,
) {
    let accent = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let block = modal_block(" Input required ", accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (body, actions) = split_actions(inner);
    let prompt_height = wrapped_height(&labeled_text("Prompt", prompt, accent), body.width)
        .min(3)
        .min(body.height.saturating_sub(2))
        .max(1);
    let prompt_area = take_top(body, prompt_height);
    let after_prompt = trim_top(body, prompt_height);
    let response_label = take_top(after_prompt, 1);
    let draft_area = trim_top(after_prompt, 1);

    frame.render_widget(
        Paragraph::new(labeled_text("Prompt", prompt, accent)).wrap(Wrap { trim: false }),
        prompt_area,
    );
    frame.render_widget(
        Paragraph::new(Span::styled("Response", accent)),
        response_label,
    );
    frame.render_widget(draft_paragraph(draft, draft_area), draft_area);
    frame.render_widget(input_actions(actions.width, interaction_key_mode), actions);
}

fn render_compact(
    frame: &mut Frame<'_>,
    modal: &InteractionModalView,
    area: Rect,
    interaction_key_mode: InteractionKeyMode,
    approval_confirmation: bool,
) {
    match modal {
        InteractionModalView::Approval {
            name, args, reason, ..
        } => render_compact_approval(
            frame,
            area,
            name,
            args,
            reason,
            interaction_key_mode,
            approval_confirmation,
        ),
        InteractionModalView::Input { prompt, draft, .. } => {
            render_compact_input(frame, area, prompt, draft, interaction_key_mode);
        }
    }
}

fn render_compact_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    name: &str,
    args: &serde_json::Value,
    reason: &str,
    interaction_key_mode: InteractionKeyMode,
    approval_confirmation: bool,
) {
    let (body, actions) = split_actions(area);
    let accent = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    if body.height >= 3 {
        frame.render_widget(compact_line("Approval", name, accent), take_top(body, 1));
        let after_tool = trim_top(body, 1);
        frame.render_widget(
            compact_line("Reason", reason, accent),
            take_top(after_tool, 1),
        );
        frame.render_widget(
            Paragraph::new(labeled_text("Args", &args.to_string(), accent))
                .wrap(Wrap { trim: false }),
            trim_top(after_tool, 1),
        );
    } else if body.height > 0 {
        frame.render_widget(
            Paragraph::new(format!(
                "Approval: {name} | Reason: {reason} | Args: {args}"
            ))
            .wrap(Wrap { trim: false }),
            body,
        );
    }

    frame.render_widget(
        approval_actions(actions.width, interaction_key_mode, approval_confirmation),
        actions,
    );
}

fn render_compact_input(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &str,
    draft: &str,
    interaction_key_mode: InteractionKeyMode,
) {
    let (body, actions) = split_actions(area);
    let accent = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    if body.height >= 2 {
        frame.render_widget(compact_line("Input", prompt, accent), take_top(body, 1));
        let draft_area = trim_top(body, 1);
        frame.render_widget(draft_paragraph(draft, draft_area), draft_area);
    } else if body.height == 1 {
        frame.render_widget(draft_paragraph(draft, body), body);
    }

    frame.render_widget(input_actions(actions.width, interaction_key_mode), actions);
}

fn modal_block(title: &'static str, accent: Style) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(accent)
        .style(Style::default().bg(Color::Black))
        .title(title)
}

fn labeled_text(label: &'static str, value: &str, label_style: Style) -> Text<'static> {
    let mut parts = value.split('\n');
    let first = parts.next().unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{label}  "), label_style),
        Span::raw(first.to_string()),
    ])];
    let indent = " ".repeat(label.len() + 2);
    lines.extend(
        parts.map(|part| Line::from(vec![Span::raw(indent.clone()), Span::raw(part.to_string())])),
    );
    Text::from(lines)
}

fn draft_paragraph(draft: &str, area: Rect) -> Paragraph<'static> {
    let mut lines = draft
        .split('\n')
        .map(|line| Line::raw(line.to_string()))
        .collect::<Vec<_>>();
    if let Some(last_line) = lines.last_mut() {
        last_line.spans.push(Span::styled(
            " ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
    }
    let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    let scroll = if area.width == 0 || area.height == 0 {
        0
    } else {
        paragraph
            .line_count(area.width)
            .saturating_sub(usize::from(area.height))
    };
    paragraph.scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
}

fn compact_line(label: &'static str, value: &str, accent: Style) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled(format!("{label}: "), accent),
        Span::raw(value.to_string()),
    ]))
}

fn approval_actions(
    width: u16,
    interaction_key_mode: InteractionKeyMode,
    approval_confirmation: bool,
) -> Paragraph<'static> {
    let text = match interaction_key_mode {
        InteractionKeyMode::Direct => {
            if width >= 46 {
                "[Y] Approve once   [N / Esc] Reject once"
            } else if width >= 24 {
                "Y approve | N/Esc reject"
            } else if width >= 18 {
                "Y approve N/Esc no"
            } else if width >= 13 {
                "Y ok N/Esc no"
            } else {
                "Y/N"
            }
        }
        InteractionKeyMode::ConfirmWithFunctionKey => {
            if approval_confirmation {
                "[F8] Confirm approve   [N / Esc] Reject"
            } else if width >= 24 {
                "Y select | F8 confirm | N/Esc reject"
            } else {
                "Y then F8; N/Esc no"
            }
        }
        InteractionKeyMode::Unavailable => "interaction unavailable",
    };
    Paragraph::new(text).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

fn input_actions(width: u16, interaction_key_mode: InteractionKeyMode) -> Paragraph<'static> {
    let text = match interaction_key_mode {
        InteractionKeyMode::Direct => {
            if width >= 24 {
                "[Enter] Submit response"
            } else {
                "Enter submit"
            }
        }
        InteractionKeyMode::ConfirmWithFunctionKey => {
            if width >= 24 {
                "[F8] Submit response"
            } else {
                "F8 submit"
            }
        }
        InteractionKeyMode::Unavailable => "interaction unavailable",
    };
    Paragraph::new(text).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn wrapped_height(text: &Text<'_>, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    let lines = Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .line_count(width);
    u16::try_from(lines).unwrap_or(u16::MAX)
}

fn split_actions(area: Rect) -> (Rect, Rect) {
    if area.height == 0 {
        return (area, area);
    }
    let actions = Rect::new(
        area.x,
        area.y.saturating_add(area.height - 1),
        area.width,
        1,
    );
    let body = Rect::new(area.x, area.y, area.width, area.height - 1);
    (body, actions)
}

fn take_top(area: Rect, height: u16) -> Rect {
    Rect::new(area.x, area.y, area.width, height.min(area.height))
}

fn trim_top(area: Rect, height: u16) -> Rect {
    let removed = height.min(area.height);
    Rect::new(
        area.x,
        area.y.saturating_add(removed),
        area.width,
        area.height - removed,
    )
}
