use ratatui::Frame;
use ratatui::layout::Rect;

use crate::interfaces::tui::state::TuiState;
use crate::interfaces::tui::widgets::{activity, composer, minimal_line, status_line, transcript};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RenderLayout {
    transcript: Option<Rect>,
    activity: Option<Rect>,
    composer: Option<Rect>,
    status: Option<Rect>,
    minimal: Option<Rect>,
}

pub fn render(frame: &mut Frame<'_>, state: &TuiState) {
    let layout = render_layout(frame.area());

    if let Some(area) = layout.minimal {
        frame.render_widget(minimal_line(state), area);
        return;
    }

    if let Some(area) = layout.transcript {
        let bordered = area.width >= 24 && area.height >= 3;
        frame.render_widget(transcript(&state.run, state.focus, bordered), area);
    }
    if let Some(area) = layout.activity {
        let bordered = area.width >= 24 && area.height >= 3;
        frame.render_widget(activity(&state.run, bordered), area);
    }
    if let Some(area) = layout.composer {
        let bordered = area.width >= 24 && area.height >= 3;
        frame.render_widget(composer(state, bordered), area);
    }
    if let Some(area) = layout.status {
        frame.render_widget(status_line(state, area.width), area);
    }
}

fn render_layout(area: Rect) -> RenderLayout {
    if area.width == 0 || area.height == 0 {
        return RenderLayout::default();
    }
    if area.height == 1 {
        return RenderLayout {
            minimal: Some(area),
            ..RenderLayout::default()
        };
    }

    let status = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let content_height = area.height - 1;
    let composer_height = match area.height {
        0..=4 => 1,
        5..=17 => 3,
        _ => 4,
    }
    .min(content_height);
    let composer_y = status.y - composer_height;
    let composer = Rect::new(area.x, composer_y, area.width, composer_height);
    let upper_height = composer_y - area.y;

    let (transcript, activity) = if upper_height >= 4 {
        let activity = Rect::new(area.x, composer_y - 3, area.width, 3);
        let transcript_height = upper_height - 3;
        let transcript = (transcript_height > 0).then_some(Rect::new(
            area.x,
            area.y,
            area.width,
            transcript_height,
        ));
        (transcript, Some(activity))
    } else {
        let transcript =
            (upper_height > 0).then_some(Rect::new(area.x, area.y, area.width, upper_height));
        (transcript, None)
    };

    RenderLayout {
        transcript,
        activity,
        composer: Some(composer),
        status: Some(status),
        minimal: None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use crate::core::types::{CallId, PlanStep, RunId, TaskPlan, TerminationReason};
    use crate::errors::ToolError;
    use crate::interfaces::terminal::view::{
        PendingApprovalView, PendingInputView, RunCompletionView, ToolCallStatus, ToolCallView,
    };
    use crate::interfaces::tui::state::TuiState;

    fn populated_state() -> TuiState {
        let completed_call = CallId::new();
        let failed_call = CallId::new();
        let approval_call = CallId::new();
        let input_id = CallId::new();
        let mut state = TuiState {
            composer: "follow up".to_string(),
            ..TuiState::default()
        };
        state.run.run_id = Some(RunId::new());
        state.run.user_message = Some("Inspect the workspace".to_string());
        state.run.assistant_text = "I found the relevant files.".to_string();
        state.run.plan = Some(TaskPlan {
            goal: "Render the run".to_string(),
            steps: vec![
                PlanStep {
                    id: "read".to_string(),
                    title: "Read state".to_string(),
                    done: true,
                },
                PlanStep {
                    id: "draw".to_string(),
                    title: "Draw widgets".to_string(),
                    done: false,
                },
            ],
            current_step: 1,
        });
        state.run.tool_calls = vec![
            ToolCallView {
                call_id: completed_call,
                name: "fs_read".to_string(),
                args: serde_json::json!({"path": "README.md"}),
                status: ToolCallStatus::Completed,
                output: Some("read 42 lines".to_string()),
                error: None,
            },
            ToolCallView {
                call_id: failed_call,
                name: "shell".to_string(),
                args: serde_json::json!({"command": "false"}),
                status: ToolCallStatus::Failed,
                output: None,
                error: Some(ToolError::ExecutionFailed {
                    reason: "command failed".to_string(),
                }),
            },
            ToolCallView {
                call_id: approval_call,
                name: "fs_write".to_string(),
                args: serde_json::json!({"path": "out.txt"}),
                status: ToolCallStatus::WaitingApproval,
                output: None,
                error: None,
            },
        ];
        state.run.failed_steps.push((
            1,
            PlanStep {
                id: "draw".to_string(),
                title: "Draw widgets".to_string(),
                done: false,
            },
            "snapshot mismatch".to_string(),
        ));
        state.run.pending_approvals.push(PendingApprovalView {
            call_id: approval_call,
            name: "fs_write".to_string(),
            args: serde_json::json!({"path": "out.txt"}),
            reason: "writes a file".to_string(),
        });
        state.run.pending_inputs.push(PendingInputView {
            input_id,
            prompt: "Which branch?".to_string(),
        });
        state.run.completed = Some(RunCompletionView {
            reason: TerminationReason::Final,
            output: Some("Renderer ready".to_string()),
        });
        state
    }

    fn draw(width: u16, height: u16, state: &TuiState) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::render(frame, state)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn row_text(buffer: &Buffer, width: u16, y: u16) -> String {
        let start = usize::from(y) * usize::from(width);
        let end = start + usize::from(width);
        buffer.content()[start..end]
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn rect_text(buffer: &Buffer, width: u16, area: Rect) -> String {
        (area.y..area.y + area.height)
            .map(|y| row_text(buffer, width, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_complete_run_projection_at_120_by_40() {
        let state = populated_state();
        let buffer = draw(120, 40, &state);
        let rendered = buffer_text(&buffer);

        assert!(rendered.contains("You"));
        assert!(rendered.contains("Inspect the workspace"));
        assert!(rendered.contains("Assistant"));
        assert!(rendered.contains("I found the relevant files."));
        assert!(rendered.contains("Plan  1/2 complete"));
        assert!(rendered.contains("[x] Read state"));
        assert!(rendered.contains("[>] Draw widgets"));
        assert!(rendered.contains("Tool  fs_read [done]"));
        assert!(rendered.contains("Tool execution failed: command failed"));
        assert!(rendered.contains("Approval required"));
        assert!(rendered.contains("Input required"));
        assert!(rendered.contains("Completed  final"));
        assert!(rendered.contains("Renderer ready"));
        assert!(rendered.contains("Composer [focused]"));
        assert!(row_text(&buffer, 120, 39).contains("workspace: - | run: done"));
    }

    #[test]
    fn wraps_and_clips_long_content_inside_60_by_20_panels() {
        let mut state = TuiState {
            composer: format!("draft {} COMPOSER_TAIL", "x".repeat(500)),
            ..TuiState::default()
        };
        state.run.run_id = Some(RunId::new());
        state.run.assistant_text = "overflow ".repeat(500);
        let buffer = draw(60, 20, &state);
        let layout = super::render_layout(Rect::new(0, 0, 60, 20));
        let transcript = layout.transcript.unwrap();
        let transcript_rows = rect_text(&buffer, 60, transcript);

        assert!(transcript_rows.matches("overflow").count() > 2);
        assert!(!rect_text(&buffer, 60, layout.activity.unwrap()).contains("overflow"));
        assert!(!rect_text(&buffer, 60, layout.composer.unwrap()).contains("overflow"));
        assert!(!row_text(&buffer, 60, layout.status.unwrap().y).contains("overflow"));
        assert!(rect_text(&buffer, 60, layout.composer.unwrap()).contains("Composer [focused]"));
        assert!(rect_text(&buffer, 60, layout.composer.unwrap()).contains("draft"));
        assert!(!buffer_text(&buffer).contains("COMPOSER_TAIL"));
        assert!(row_text(&buffer, 60, 19).contains("run:running"));
    }

    #[test]
    fn keeps_all_four_regions_visible_at_40_by_12() {
        let state = TuiState {
            composer: "hello".to_string(),
            ..TuiState::default()
        };
        let buffer = draw(40, 12, &state);
        let rendered = buffer_text(&buffer);

        assert!(rendered.contains("Transcript"));
        assert!(rendered.contains("Activity"));
        assert!(rendered.contains("Composer [focused]"));
        assert!(rendered.contains("hello"));
        assert!(row_text(&buffer, 40, 11).contains("ws:- | run:idle | focus:composer"));
    }

    #[test]
    fn compact_layout_keeps_composer_and_status_separate_and_tiny_layout_does_not_panic() {
        let state = TuiState {
            composer: "tiny draft".to_string(),
            ..TuiState::default()
        };
        let buffer = draw(18, 4, &state);

        assert!(row_text(&buffer, 18, 2).contains("tiny draft"));
        assert!(row_text(&buffer, 18, 3).contains("run:idle"));

        let tiny = draw(1, 1, &state);
        assert_eq!(tiny.content().len(), 1);
    }
}
