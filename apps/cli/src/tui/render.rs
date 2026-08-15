use ratatui::Frame;
use ratatui::layout::Rect;

use crate::tui::state::TuiState;
use crate::tui::widgets::{
    activity, composer, minimal_line, render_modal, render_overlay, status_line, transcript,
    transcript_viewport,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RenderLayout {
    transcript: Option<Rect>,
    activity: Option<Rect>,
    composer: Option<Rect>,
    status: Option<Rect>,
    minimal: Option<Rect>,
}

pub fn render(frame: &mut Frame<'_>, state: &TuiState) {
    let frame_area = frame.area();
    let layout = render_layout(frame_area);

    if let Some(area) = layout.minimal {
        frame.render_widget(minimal_line(state), area);
    } else {
        if let Some(area) = layout.transcript {
            let bordered = area.width >= 24 && area.height >= 3;
            frame.render_widget(transcript(state, area, bordered), area);
        }
        if let Some(area) = layout.activity {
            let bordered = area.width >= 24 && area.height >= 3;
            frame.render_widget(activity(state, bordered), area);
        }
        if let Some(area) = layout.composer {
            let bordered = area.width >= 24 && area.height >= 3;
            frame.render_widget(composer(state, area, bordered), area);
        }
        if let Some(area) = layout.status {
            frame.render_widget(status_line(state, area.width), area);
        }
    }

    if let Some(overlay) = &state.overlay {
        render_overlay(frame, overlay, frame_area);
    }

    if let Some(modal) = &state.modal {
        render_modal(
            frame,
            modal,
            frame_area,
            state.interaction_key_mode,
            state.approval_confirmation == Some(modal.request_id()),
        );
    }
}

pub fn sync_viewport(state: &mut TuiState, area: Rect) {
    state.terminal_width = area.width;
    state.terminal_height = area.height;
    let layout = render_layout(area);
    let (max_offset, page_size) = layout.transcript.map_or((0, 1), |transcript_area| {
        let bordered = transcript_area.width >= 24 && transcript_area.height >= 3;
        transcript_viewport(state, transcript_area, bordered)
    });
    state.transcript_scroll.set_viewport(max_offset, page_size);
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

    use crate::terminal::view::{
        PendingApprovalView, PendingInputView, RunCompletionView, RunViewState, ToolCallStatus,
        ToolCallView,
    };
    use crate::tui::state::{
        InteractionKeyMode, InteractionModalView, ModelCandidate, ModelPickerState,
        ResumeCandidate, RunLifecycle, SessionPickerState, TuiFocus, TuiOverlay, TuiState,
    };
    use crate::tui::widgets::modal_area;
    use rove_app_bootstrap::{ModelSelection, ProviderProfileId};
    use rove_core::ToolError;
    use rove_runtime::types::{
        CallId, JobId, PlanStep, RunId, SessionId, TaskPlan, TerminationReason,
    };

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
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "README.md"}),
                status: ToolCallStatus::Completed,
                output: Some("read 42 lines".to_string()),
                error: None,
            },
            ToolCallView {
                call_id: failed_call,
                name: "run_shell".to_string(),
                args: serde_json::json!({"command": "false"}),
                status: ToolCallStatus::Failed,
                output: None,
                error: Some(ToolError::ExecutionFailed {
                    reason: "command failed".to_string(),
                }),
            },
            ToolCallView {
                call_id: approval_call,
                name: "write_file".to_string(),
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
            name: "write_file".to_string(),
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
        state.run_lifecycle = RunLifecycle::Completed;
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

    fn outside_rect_text(buffer: &Buffer, width: u16, height: u16, area: Rect) -> String {
        (0..height)
            .map(|y| {
                (0..width)
                    .filter(|x| {
                        *x < area.x
                            || *x >= area.x.saturating_add(area.width)
                            || y < area.y
                            || y >= area.y.saturating_add(area.height)
                    })
                    .map(|x| {
                        let index = usize::from(y) * usize::from(width) + usize::from(x);
                        buffer.content()[index].symbol()
                    })
                    .collect::<String>()
            })
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
        assert!(rendered.contains("Tool  read_file [done]"));
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
        state.run_lifecycle = RunLifecycle::Running;
        let buffer = draw(60, 20, &state);
        let layout = super::render_layout(Rect::new(0, 0, 60, 20));
        let transcript = layout.transcript.unwrap();
        let transcript_rows = rect_text(&buffer, 60, transcript);

        assert!(transcript_rows.matches("overflow").count() > 2);
        assert!(!rect_text(&buffer, 60, layout.activity.unwrap()).contains("overflow"));
        assert!(!rect_text(&buffer, 60, layout.composer.unwrap()).contains("overflow"));
        assert!(!row_text(&buffer, 60, layout.status.unwrap().y).contains("overflow"));
        assert!(rect_text(&buffer, 60, layout.composer.unwrap()).contains("Composer [focused]"));
        assert!(rect_text(&buffer, 60, layout.composer.unwrap()).contains("COMPOSER_TAIL"));
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
        assert!(row_text(&buffer, 40, 11).contains("ws:- | run:idle | q:0 | focus:composer"));
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

    #[test]
    fn transcript_follows_the_wrapped_tail_and_can_scroll_back_to_the_head() {
        let mut state = TuiState {
            focus: TuiFocus::Transcript,
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        state.run.run_id = Some(RunId::new());
        state.run.assistant_text = format!(
            "HEAD_MARKER\n{}\nTAIL_MARKER",
            (0..40)
                .map(|index| format!("wrapped line {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        super::sync_viewport(&mut state, Rect::new(0, 0, 40, 12));

        assert!(state.transcript_scroll.max_offset > 0);
        let bottom = buffer_text(&draw(40, 12, &state));
        assert!(bottom.contains("TAIL_MARKER"));
        assert!(!bottom.contains("HEAD_MARKER"));

        state
            .transcript_scroll
            .scroll_up(state.transcript_scroll.max_offset);
        let top = buffer_text(&draw(40, 12, &state));
        assert!(top.contains("HEAD_MARKER"));
        assert!(!top.contains("TAIL_MARKER"));
    }

    #[test]
    fn transcript_renders_archived_runs_and_deduplicates_final_output() {
        let archived = RunViewState {
            run_id: Some(RunId::new()),
            user_message: Some("FIRST_PROMPT".to_string()),
            assistant_text: "FIRST_ANSWER".to_string(),
            completed: Some(RunCompletionView {
                reason: TerminationReason::Final,
                output: Some("FIRST_ANSWER".to_string()),
            }),
            ..RunViewState::default()
        };
        let mut state = TuiState {
            run_history: vec![archived],
            run_lifecycle: RunLifecycle::Completed,
            ..TuiState::default()
        };
        state.run.run_id = Some(RunId::new());
        state.run.user_message = Some("SECOND_PROMPT".to_string());
        state.run.assistant_text = "UNIQUE_FINAL".to_string();
        state.run.completed = Some(RunCompletionView {
            reason: TerminationReason::Final,
            output: Some("UNIQUE_FINAL".to_string()),
        });

        let rendered = buffer_text(&draw(100, 30, &state));
        assert!(rendered.contains("FIRST_PROMPT"));
        assert!(rendered.contains("FIRST_ANSWER"));
        assert!(rendered.contains("SECOND_PROMPT"));
        assert_eq!(rendered.matches("UNIQUE_FINAL").count(), 1);
    }

    #[test]
    fn timeline_status_between_stream_and_final_does_not_duplicate_the_answer() {
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Completed,
            ..TuiState::default()
        };
        state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::AssistantDelta {
                delta: "UNIQUE_TIMELINE_ANSWER".to_string(),
            });
        state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::ModelStatus {
                status: "working".to_string(),
                message: "checking".to_string(),
            });
        state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::LlmMessage {
                full: "UNIQUE_TIMELINE_ANSWER".to_string(),
                usage: Default::default(),
                tool_call_count: 0,
            });
        state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("UNIQUE_TIMELINE_ANSWER".to_string()),
            });

        let rendered = buffer_text(&draw(100, 24, &state));
        assert_eq!(rendered.matches("UNIQUE_TIMELINE_ANSWER").count(), 1);
    }

    #[test]
    fn render_boundaries_redact_split_secrets_and_modal_context() {
        let mut transcript_state = TuiState {
            composer: "Discuss password and secret handling".to_string(),
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        transcript_state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::AssistantDelta {
                delta: "api_".to_string(),
            });
        transcript_state
            .run
            .apply_update(crate::terminal::view::RunViewUpdate::AssistantDelta {
                delta: "key=CANARY_SPLIT_SECRET".to_string(),
            });
        let transcript = buffer_text(&draw(120, 30, &transcript_state));
        assert!(!transcript.contains("CANARY_SPLIT_SECRET"));
        assert!(transcript.contains("redacted sensitive output"));
        assert!(transcript.contains("Discuss password and secret handling"));

        let approval_state = TuiState {
            modal: Some(InteractionModalView::Approval {
                call_id: CallId::new(),
                name: "write_file".to_string(),
                args: serde_json::json!({
                    "authorization": "CANARY_AUTH_VALUE",
                    "nested": {"api_token": "CANARY_TOKEN_VALUE"}
                }),
                reason: "password=CANARY_REASON_VALUE".to_string(),
            }),
            ..TuiState::default()
        };
        let approval = buffer_text(&draw(120, 40, &approval_state));
        for canary in [
            "CANARY_AUTH_VALUE",
            "CANARY_TOKEN_VALUE",
            "CANARY_REASON_VALUE",
        ] {
            assert!(!approval.contains(canary));
        }
        assert!(approval.contains("redacted"));

        let input_state = TuiState {
            modal: Some(InteractionModalView::Input {
                input_id: CallId::new(),
                prompt: "access_token: CANARY_PROMPT_VALUE".to_string(),
                draft: "ordinary secret discussion".to_string(),
            }),
            ..TuiState::default()
        };
        let input = buffer_text(&draw(120, 40, &input_state));
        assert!(!input.contains("CANARY_PROMPT_VALUE"));
        assert!(input.contains("ordinary secret discussion"));
    }

    #[test]
    fn lifecycle_drives_pre_start_cancelling_and_limited_statuses() {
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        assert!(row_text(&draw(40, 12, &state), 40, 11).contains("run:running"));

        state.run_lifecycle = RunLifecycle::Cancelling;
        assert!(row_text(&draw(40, 12, &state), 40, 11).contains("run:cancelling"));

        state.run_lifecycle = RunLifecycle::Completed;
        state.run.completed = Some(RunCompletionView {
            reason: TerminationReason::StepLimit,
            output: None,
        });
        assert!(row_text(&draw(40, 12, &state), 40, 11).contains("run:step limit"));
    }

    #[test]
    fn active_tool_is_not_hidden_by_plan_or_model_status() {
        let mut state = populated_state();
        state.run_lifecycle = RunLifecycle::Running;
        state.run.completed = None;
        state.run.pending_approvals.clear();
        state.run.pending_inputs.clear();
        state.run.model_status = Some(("thinking".to_string(), "stale".to_string()));
        state.run.current_step = Some((
            1,
            PlanStep {
                id: "draw".to_string(),
                title: "Draw widgets".to_string(),
                done: false,
            },
        ));
        let tool = state.run.tool_calls.last_mut().unwrap();
        tool.status = ToolCallStatus::Started;

        let buffer = draw(80, 20, &state);
        let activity = super::render_layout(Rect::new(0, 0, 80, 20))
            .activity
            .unwrap();
        let rendered = rect_text(&buffer, 80, activity);
        assert!(rendered.contains("Tool"));
        assert!(rendered.contains("write_file"));
        assert!(!rendered.contains("stale"));
    }

    #[test]
    fn approval_modal_renders_complete_decision_context_at_120_by_40() {
        let modal = InteractionModalView::Approval {
            call_id: CallId::new(),
            name: "WRITE_MARKER_工具写入".to_string(),
            args: serde_json::json!({
                "path": "输出/ARG_MARKER.txt",
                "content": "多语言内容"
            }),
            reason: "REASON_MARKER 修改工作区中的文件，需要明确确认。".to_string(),
        };
        let area = modal_area(Rect::new(0, 0, 120, 40), &modal);
        let state = TuiState {
            modal: Some(modal),
            ..TuiState::default()
        };
        let buffer = draw(120, 40, &state);
        let rendered = rect_text(&buffer, 120, area);

        assert_eq!((area.width, area.height), (88, 16));
        assert!(rendered.contains("Approval required"));
        assert!(rendered.contains("Tool"));
        assert!(rendered.contains("WRITE_MARKER"));
        assert!(rendered.contains("Reason"));
        assert!(rendered.contains("REASON_MARKER"));
        assert!(rendered.contains("Arguments"));
        assert!(rendered.contains("ARG_MARKER"));
        assert!(rendered.contains("[Y] Approve once"));
        assert!(rendered.contains("[N / Esc] Reject once"));
    }

    #[test]
    fn function_key_mode_exposes_the_non_text_confirmation_boundary() {
        let call_id = CallId::new();
        let approval = InteractionModalView::Approval {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        };
        let state = TuiState {
            modal: Some(approval.clone()),
            interaction_key_mode: InteractionKeyMode::ConfirmWithFunctionKey,
            ..TuiState::default()
        };
        let rendered = buffer_text(&draw(80, 20, &state));
        assert!(rendered.contains("Y select"));
        assert!(rendered.contains("F8 confirm"));
        assert!(!rendered.contains("[Y] Approve once"));

        let confirmed = TuiState {
            modal: Some(approval),
            interaction_key_mode: InteractionKeyMode::ConfirmWithFunctionKey,
            approval_confirmation: Some(call_id),
            ..TuiState::default()
        };
        assert!(buffer_text(&draw(80, 20, &confirmed)).contains("[F8] Confirm approve"));

        let input = TuiState {
            modal: Some(InteractionModalView::Input {
                input_id: CallId::new(),
                prompt: "answer".to_string(),
                draft: String::new(),
            }),
            interaction_key_mode: InteractionKeyMode::ConfirmWithFunctionKey,
            ..TuiState::default()
        };
        let input_rendered = buffer_text(&draw(80, 20, &input));
        assert!(input_rendered.contains("[F8] Submit response"));
        assert!(!input_rendered.contains("[Enter] Submit response"));
    }

    #[test]
    fn approval_modal_clears_its_overlay_and_does_not_leak_at_60_by_20() {
        let modal = InteractionModalView::Approval {
            call_id: CallId::new(),
            name: "MODAL_ONLY_MARKER_删除工具".to_string(),
            args: serde_json::json!({"target": "临时文件", "recursive": true}),
            reason: format!("危险操作 REASON_TAIL {}", "理由很长 ".repeat(80)),
        };
        let viewport = Rect::new(0, 0, 60, 20);
        let area = modal_area(viewport, &modal);
        let mut state = TuiState {
            modal: Some(modal),
            ..TuiState::default()
        };
        state.run.assistant_text =
            "BACKGROUND_MARKER\nBACKGROUND_MARKER\nBACKGROUND_MARKER".to_string();

        let buffer = draw(60, 20, &state);
        let inside = rect_text(&buffer, 60, area);
        let outside = outside_rect_text(&buffer, 60, 20, area);

        assert!(inside.contains("MODAL_ONLY_MARKER"));
        assert!(!inside.contains("BACKGROUND_MARKER"));
        assert!(!outside.contains("MODAL_ONLY_MARKER"));
        assert!(outside.contains("BACKGROUND_MARKER"));
    }

    #[test]
    fn input_modal_keeps_long_unicode_draft_tail_visible_at_40_by_12() {
        let draft = format!(
            "HEAD_MARKER\n{}\nTAIL_MARKER_最终回答",
            (0..30)
                .map(|index| format!("第 {index} 行 response 内容"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let modal = InteractionModalView::Input {
            input_id: CallId::new(),
            prompt: format!(
                "PROMPT_MARKER 请准确输入分支名称。{}",
                "这是较长的 Unicode 提示。".repeat(12)
            ),
            draft,
        };
        let area = modal_area(Rect::new(0, 0, 40, 12), &modal);
        let state = TuiState {
            modal: Some(modal),
            ..TuiState::default()
        };
        let buffer = draw(40, 12, &state);
        let rendered = rect_text(&buffer, 40, area);

        assert_eq!((area.width, area.height), (36, 10));
        assert!(rendered.contains("Input required"));
        assert!(rendered.contains("Prompt"));
        assert!(rendered.contains("PROMPT_MARKER"));
        assert!(rendered.contains("Response"));
        assert!(rendered.contains("TAIL_MARKER"));
        assert!(!rendered.contains("HEAD_MARKER"));
        assert!(rendered.contains("[Enter] Submit response"));
    }

    #[test]
    fn modal_layout_is_bounded_and_tiny_viewports_do_not_panic() {
        let approval = InteractionModalView::Approval {
            call_id: CallId::new(),
            name: "write_file".to_string(),
            args: serde_json::json!({"path": "out.txt"}),
            reason: "writes a file".to_string(),
        };
        let input = InteractionModalView::Input {
            input_id: CallId::new(),
            prompt: "Tiny prompt".to_string(),
            draft: "HEAD\nmiddle\nTAIL".to_string(),
        };

        assert_eq!(
            modal_area(Rect::new(3, 4, 0, 0), &approval),
            Rect::new(3, 4, 0, 0)
        );
        for (width, height) in [(1, 1), (2, 2), (8, 3), (18, 4)] {
            for modal in [&approval, &input] {
                let state = TuiState {
                    modal: Some(modal.clone()),
                    ..TuiState::default()
                };
                let buffer = draw(width, height, &state);
                assert_eq!(
                    buffer.content().len(),
                    usize::from(width) * usize::from(height)
                );
                let area = modal_area(Rect::new(0, 0, width, height), modal);
                assert!(area.x.saturating_add(area.width) <= width);
                assert!(area.y.saturating_add(area.height) <= height);
            }
        }

        let approval_state = TuiState {
            modal: Some(approval),
            ..TuiState::default()
        };
        let compact_approval = buffer_text(&draw(18, 4, &approval_state));
        assert!(compact_approval.contains("Approval"));
        assert!(compact_approval.contains("Reason"));
        assert!(compact_approval.contains("Args"));
        assert!(compact_approval.contains("Y ok N/Esc no"));

        let input_state = TuiState {
            modal: Some(input),
            ..TuiState::default()
        };
        let compact_input = buffer_text(&draw(18, 4, &input_state));
        assert!(compact_input.contains("Input"));
        assert!(compact_input.contains("TAIL"));
        assert!(!compact_input.contains("HEAD"));
        assert!(compact_input.contains("Enter submit"));
    }

    #[test]
    fn navigation_overlays_are_bounded_and_show_safe_help_or_empty_states() {
        let candidate = ResumeCandidate {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "resume target".to_string(),
            step: 3,
        };
        let picker = TuiState {
            overlay: Some(TuiOverlay::SessionPicker(SessionPickerState::ready(vec![
                candidate,
            ]))),
            ..TuiState::default()
        };
        let rendered = buffer_text(&draw(80, 20, &picker));
        assert!(rendered.contains("Resume session"));
        assert!(rendered.contains("resume target"));
        assert!(rendered.contains("Enter resume"));

        let models = TuiState {
            overlay: Some(TuiOverlay::ModelPicker(ModelPickerState::ready(
                vec![ModelCandidate {
                    selection: ModelSelection {
                        profile_id: ProviderProfileId::new("local").unwrap(),
                        model: "模型-alpha".to_string(),
                        reasoning: "default".to_string(),
                        revision: "sha256:catalog".to_string(),
                    },
                    label: "本地模型".to_string(),
                    provider_type: "ollama".to_string(),
                    credential_ready: true,
                    inventory_fresh: false,
                    current: true,
                }],
                "模型".to_string(),
            ))),
            ..TuiState::default()
        };
        let rendered = buffer_text(&draw(40, 10, &models));
        assert!(rendered.contains("Select model"));
        assert!(rendered.contains("alpha"));
        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains("credential="));

        let empty_tools = TuiState {
            overlay: Some(TuiOverlay::ToolDetail(crate::tui::state::ToolDetailState {
                entries: Vec::new(),
                selected: 0,
                scroll: 0,
            })),
            ..TuiState::default()
        };
        let rendered = buffer_text(&draw(40, 10, &empty_tools));
        assert!(rendered.contains("No completed"));

        let help = TuiState {
            overlay: Some(TuiOverlay::Help(crate::tui::state::HelpState { scroll: 0 })),
            ..TuiState::default()
        };
        let rendered = buffer_text(&draw(100, 30, &help));
        assert!(rendered.contains("Ctrl+R"));
        assert!(rendered.contains("Open/resume session picker"));
        assert!(rendered.contains("F8"));

        for (width, height) in [(1, 1), (2, 2), (8, 3), (18, 4)] {
            let _ = draw(width, height, &picker);
            let _ = draw(width, height, &empty_tools);
            let _ = draw(width, height, &help);
            let _ = draw(width, height, &models);
        }
    }
}
