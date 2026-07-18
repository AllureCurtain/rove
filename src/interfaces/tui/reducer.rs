use crate::core::types::CallId;
use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::effect::TuiEffect;
use crate::interfaces::tui::state::{
    InteractionModalView, MAX_COMPOSER_BYTES, MAX_INTERACTION_INPUT_BYTES, RunLifecycle, TuiFocus,
    TuiState,
};

pub fn reduce(state: &mut TuiState, action: TuiAction) -> Vec<TuiEffect> {
    if !matches!(
        &action,
        TuiAction::Terminal(TerminalAction::Exit) | TuiAction::Tick | TuiAction::Resize { .. }
    ) {
        state.quit_confirmation = false;
    }

    match action {
        TuiAction::OpenInteraction(modal) => {
            if state.modal.is_none() {
                state.modal = Some(modal);
            }
            Vec::new()
        }
        TuiAction::CloseInteraction { kind, request_id } => {
            if state
                .modal
                .as_ref()
                .is_some_and(|modal| modal.matches_request(kind, request_id))
            {
                state.modal = None;
            }
            Vec::new()
        }
        TuiAction::ApproveInteraction { call_id } => resolve_approval(state, call_id, true),
        TuiAction::RejectInteraction { call_id } => resolve_approval(state, call_id, false),
        TuiAction::SubmitInteraction { input_id } => resolve_input(state, input_id),
        TuiAction::Terminal(TerminalAction::Exit) => {
            if state.run_lifecycle.is_active() {
                if state.quit_confirmation {
                    state.quit_confirmation = false;
                    state.run_lifecycle = RunLifecycle::Cancelling;
                    state.modal = None;
                    vec![
                        TuiEffect::Dispatch(TerminalAction::CancelRun),
                        TuiEffect::ExitAfterRun,
                    ]
                } else {
                    state.quit_confirmation = true;
                    Vec::new()
                }
            } else {
                state.modal = None;
                state.should_quit = true;
                vec![TuiEffect::Exit]
            }
        }
        TuiAction::Terminal(TerminalAction::CancelRun) => cancel_or_clear(state),
        TuiAction::Terminal(TerminalAction::SubmitPrompt(message)) if state.modal.is_none() => {
            dispatch_prompt(state, message)
        }
        TuiAction::Terminal(TerminalAction::SubmitPrompt(_)) => Vec::new(),
        TuiAction::Terminal(
            TerminalAction::ApproveTool { .. }
            | TerminalAction::RejectTool { .. }
            | TerminalAction::SubmitInput { .. },
        ) => Vec::new(),
        TuiAction::Terminal(action) => vec![TuiEffect::Dispatch(action)],
        TuiAction::InsertChar(ch) => {
            if let Some(InteractionModalView::Input { draft, .. }) = state.modal.as_mut() {
                if draft.len().saturating_add(ch.len_utf8()) <= MAX_INTERACTION_INPUT_BYTES {
                    draft.push(ch);
                }
            } else if state.modal.is_none()
                && state.focus == TuiFocus::Composer
                && state.composer.len().saturating_add(ch.len_utf8()) <= MAX_COMPOSER_BYTES
            {
                state.composer.push(ch);
            }
            Vec::new()
        }
        TuiAction::Backspace => {
            if let Some(InteractionModalView::Input { draft, .. }) = state.modal.as_mut() {
                draft.pop();
            } else if state.modal.is_none() && state.focus == TuiFocus::Composer {
                state.composer.pop();
            }
            Vec::new()
        }
        TuiAction::SubmitComposer => {
            if state.modal.is_some()
                || state.focus != TuiFocus::Composer
                || !state.run_lifecycle.accepts_prompt()
            {
                return Vec::new();
            }
            let message = state.composer.trim().to_string();
            if message.is_empty() {
                Vec::new()
            } else {
                state.composer.clear();
                dispatch_prompt(state, message)
            }
        }
        TuiAction::FocusNext => {
            if state.modal.is_some() {
                return Vec::new();
            }
            state.focus = match state.focus {
                TuiFocus::Transcript => TuiFocus::Composer,
                TuiFocus::Composer => TuiFocus::Transcript,
            };
            Vec::new()
        }
        TuiAction::ScrollUp(amount) => {
            if state.modal.is_none() && state.focus == TuiFocus::Transcript {
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollDown(amount) => {
            if state.modal.is_none() && state.focus == TuiFocus::Transcript {
                state.transcript_scroll.scroll_down(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageUp => {
            if state.modal.is_none() && state.focus == TuiFocus::Transcript {
                let amount = state.transcript_scroll.page_size.max(1);
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageDown => {
            if state.modal.is_none() && state.focus == TuiFocus::Transcript {
                let amount = state.transcript_scroll.page_size.max(1);
                state.transcript_scroll.scroll_down(amount);
            }
            Vec::new()
        }
        TuiAction::SetTranscriptViewport {
            max_offset,
            page_size,
        } => {
            state.transcript_scroll.set_viewport(max_offset, page_size);
            Vec::new()
        }
        TuiAction::Resize { width, height } => {
            state.terminal_width = width;
            state.terminal_height = height;
            Vec::new()
        }
        TuiAction::Tick => Vec::new(),
    }
}

fn cancel_or_clear(state: &mut TuiState) -> Vec<TuiEffect> {
    state.modal = None;
    match state.run_lifecycle {
        RunLifecycle::Running => {
            state.run_lifecycle = RunLifecycle::Cancelling;
            vec![TuiEffect::Dispatch(TerminalAction::CancelRun)]
        }
        RunLifecycle::Cancelling => Vec::new(),
        RunLifecycle::Idle | RunLifecycle::Completed => {
            state.composer.clear();
            Vec::new()
        }
    }
}

fn dispatch_prompt(state: &mut TuiState, message: String) -> Vec<TuiEffect> {
    if state.modal.is_some() || !state.run_lifecycle.accepts_prompt() || message.trim().is_empty() {
        return Vec::new();
    }

    state.run_lifecycle = RunLifecycle::Running;
    vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message))]
}

fn resolve_approval(state: &mut TuiState, call_id: CallId, approve: bool) -> Vec<TuiEffect> {
    let matches = state.modal.as_ref().is_some_and(|modal| {
        matches!(
            modal,
            InteractionModalView::Approval {
                call_id: current,
                ..
            } if *current == call_id
        )
    });
    if !matches {
        return Vec::new();
    }

    state.modal = None;
    let action = if approve {
        TerminalAction::ApproveTool { call_id }
    } else {
        TerminalAction::RejectTool { call_id }
    };
    vec![TuiEffect::Dispatch(action)]
}

fn resolve_input(state: &mut TuiState, input_id: CallId) -> Vec<TuiEffect> {
    let Some(InteractionModalView::Input {
        input_id: current,
        draft,
        ..
    }) = state.modal.as_ref()
    else {
        return Vec::new();
    };
    if *current != input_id {
        return Vec::new();
    }

    let answer = draft.clone();
    state.modal = None;
    vec![TuiEffect::Dispatch(TerminalAction::SubmitInput {
        input_id,
        answer,
    })]
}

#[cfg(test)]
mod tests {
    use crate::core::types::CallId;
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::tui::action::TuiAction;
    use crate::interfaces::tui::effect::TuiEffect;
    use crate::interfaces::tui::state::{
        InteractionModalKind, InteractionModalView, MAX_INTERACTION_INPUT_BYTES, RunLifecycle,
        TuiFocus, TuiState,
    };

    use super::reduce;

    fn approval_modal(call_id: CallId) -> InteractionModalView {
        InteractionModalView::Approval {
            call_id,
            name: "fs_write".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        }
    }

    fn input_modal(input_id: CallId, draft: impl Into<String>) -> InteractionModalView {
        InteractionModalView::Input {
            input_id,
            prompt: "Which branch?".to_string(),
            draft: draft.into(),
        }
    }

    #[test]
    fn submitting_composer_produces_typed_terminal_effect() {
        let mut state = TuiState {
            composer: "  hello  ".to_string(),
            ..TuiState::default()
        };

        let effects = reduce(&mut state, TuiAction::SubmitComposer);

        assert_eq!(state.composer, "");
        assert_eq!(state.run_lifecycle, RunLifecycle::Running);
        assert_eq!(
            effects,
            vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(
                "hello".to_string()
            ))]
        );
    }

    #[test]
    fn composer_edits_unicode_without_splitting_characters() {
        let mut state = TuiState::default();

        reduce(&mut state, TuiAction::InsertChar('你'));
        reduce(&mut state, TuiAction::InsertChar('🙂'));
        reduce(&mut state, TuiAction::Backspace);

        assert_eq!(state.composer, "你");
        reduce(&mut state, TuiAction::Backspace);
        assert!(state.composer.is_empty());
        reduce(&mut state, TuiAction::Backspace);
        assert!(state.composer.is_empty());
    }

    #[test]
    fn composer_input_is_bounded_without_splitting_utf8() {
        let mut state = TuiState {
            composer: "x".repeat(crate::interfaces::tui::state::MAX_COMPOSER_BYTES - 1),
            ..TuiState::default()
        };

        reduce(&mut state, TuiAction::InsertChar('x'));
        reduce(&mut state, TuiAction::InsertChar('界'));

        assert_eq!(
            state.composer.len(),
            crate::interfaces::tui::state::MAX_COMPOSER_BYTES
        );
        assert!(state.composer.is_char_boundary(state.composer.len()));
    }

    #[test]
    fn composer_rejects_blank_or_busy_submissions() {
        let mut state = TuiState {
            composer: "   \t".to_string(),
            ..TuiState::default()
        };

        assert!(reduce(&mut state, TuiAction::SubmitComposer).is_empty());
        assert_eq!(state.run_lifecycle, RunLifecycle::Idle);
        assert_eq!(state.composer, "   \t");

        state.composer = "queued without queueing".to_string();
        state.run_lifecycle = RunLifecycle::Running;
        assert!(reduce(&mut state, TuiAction::SubmitComposer).is_empty());
        assert_eq!(state.composer, "queued without queueing");
    }

    #[test]
    fn focus_gates_composer_editing_and_transcript_scrolling() {
        let mut state = TuiState::default();
        reduce(
            &mut state,
            TuiAction::SetTranscriptViewport {
                max_offset: 10,
                page_size: 4,
            },
        );

        reduce(&mut state, TuiAction::ScrollUp(3));
        assert_eq!(state.transcript_scroll.offset, 0);

        reduce(&mut state, TuiAction::FocusNext);
        assert_eq!(state.focus, TuiFocus::Transcript);
        reduce(&mut state, TuiAction::InsertChar('x'));
        reduce(&mut state, TuiAction::Backspace);
        assert!(state.composer.is_empty());
        reduce(&mut state, TuiAction::ScrollUp(3));
        assert_eq!(state.transcript_scroll.offset, 3);

        reduce(&mut state, TuiAction::FocusNext);
        assert_eq!(state.focus, TuiFocus::Composer);
    }

    #[test]
    fn transcript_scrolling_is_bounded_and_clamped_when_content_shrinks() {
        let mut state = TuiState {
            focus: TuiFocus::Transcript,
            ..TuiState::default()
        };
        reduce(
            &mut state,
            TuiAction::SetTranscriptViewport {
                max_offset: 5,
                page_size: 4,
            },
        );

        reduce(&mut state, TuiAction::ScrollUp(u16::MAX));
        assert_eq!(state.transcript_scroll.offset, 5);
        reduce(&mut state, TuiAction::ScrollDown(2));
        assert_eq!(state.transcript_scroll.offset, 3);
        reduce(&mut state, TuiAction::ScrollDown(20));
        assert_eq!(state.transcript_scroll.offset, 0);

        reduce(&mut state, TuiAction::ScrollUp(5));
        reduce(
            &mut state,
            TuiAction::SetTranscriptViewport {
                max_offset: 2,
                page_size: 2,
            },
        );
        assert_eq!(state.transcript_scroll.offset, 2);
        assert_eq!(state.transcript_scroll.max_offset, 2);
    }

    #[test]
    fn cancellation_is_active_only_once_and_idle_clears_the_draft() {
        let mut state = TuiState {
            composer: "draft".to_string(),
            ..TuiState::default()
        };

        assert!(reduce(&mut state, TuiAction::Terminal(TerminalAction::CancelRun)).is_empty());
        assert!(state.composer.is_empty());
        assert_eq!(state.run_lifecycle, RunLifecycle::Idle);

        state.run_lifecycle = RunLifecycle::Running;
        assert_eq!(
            reduce(&mut state, TuiAction::Terminal(TerminalAction::CancelRun)),
            vec![TuiEffect::Dispatch(TerminalAction::CancelRun)]
        );
        assert_eq!(state.run_lifecycle, RunLifecycle::Cancelling);
        assert!(reduce(&mut state, TuiAction::Terminal(TerminalAction::CancelRun)).is_empty());
    }

    #[test]
    fn active_exit_requires_confirmation_and_cancels_before_exiting() {
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };

        assert!(reduce(&mut state, TuiAction::Terminal(TerminalAction::Exit)).is_empty());
        assert!(state.quit_confirmation);
        assert!(!state.should_quit);

        assert_eq!(
            reduce(&mut state, TuiAction::Terminal(TerminalAction::Exit)),
            vec![
                TuiEffect::Dispatch(TerminalAction::CancelRun),
                TuiEffect::ExitAfterRun,
            ]
        );
        assert_eq!(state.run_lifecycle, RunLifecycle::Cancelling);
        assert!(!state.quit_confirmation);
    }

    #[test]
    fn page_scrolling_uses_the_rendered_viewport_height() {
        let mut state = TuiState {
            focus: TuiFocus::Transcript,
            ..TuiState::default()
        };
        reduce(
            &mut state,
            TuiAction::SetTranscriptViewport {
                max_offset: 20,
                page_size: 6,
            },
        );

        reduce(&mut state, TuiAction::ScrollPageUp);
        assert_eq!(state.transcript_scroll.offset, 6);
        reduce(&mut state, TuiAction::ScrollPageDown);
        assert_eq!(state.transcript_scroll.offset, 0);
    }

    #[test]
    fn resize_records_even_minimal_terminal_dimensions() {
        let mut state = TuiState::default();

        let effects = reduce(
            &mut state,
            TuiAction::Resize {
                width: 1,
                height: 0,
            },
        );

        assert!(effects.is_empty());
        assert_eq!((state.terminal_width, state.terminal_height), (1, 0));
    }

    #[test]
    fn opening_an_interaction_never_overwrites_a_live_modal() {
        let approval_id = CallId::new();
        let input_id = CallId::new();
        let approval = approval_modal(approval_id);
        let mut state = TuiState::default();

        assert!(reduce(&mut state, TuiAction::OpenInteraction(approval.clone())).is_empty());
        assert!(
            reduce(
                &mut state,
                TuiAction::OpenInteraction(input_modal(input_id, "stale"))
            )
            .is_empty()
        );

        assert_eq!(state.modal, Some(approval));
    }

    #[test]
    fn close_requires_both_the_modal_variant_and_request_id() {
        let input_id = CallId::new();
        let mut state = TuiState {
            modal: Some(input_modal(input_id, "answer")),
            ..TuiState::default()
        };

        reduce(
            &mut state,
            TuiAction::CloseInteraction {
                kind: InteractionModalKind::Approval,
                request_id: input_id,
            },
        );
        assert!(state.modal.is_some());

        reduce(
            &mut state,
            TuiAction::CloseInteraction {
                kind: InteractionModalKind::Input,
                request_id: CallId::new(),
            },
        );
        assert!(state.modal.is_some());

        reduce(
            &mut state,
            TuiAction::CloseInteraction {
                kind: InteractionModalKind::Input,
                request_id: input_id,
            },
        );
        assert!(state.modal.is_none());
    }

    #[test]
    fn approval_resolution_is_typed_id_matched_and_single_use() {
        let call_id = CallId::new();
        let mut state = TuiState {
            modal: Some(approval_modal(call_id)),
            ..TuiState::default()
        };

        assert!(
            reduce(
                &mut state,
                TuiAction::SubmitInteraction { input_id: call_id }
            )
            .is_empty()
        );
        assert!(
            reduce(
                &mut state,
                TuiAction::ApproveInteraction {
                    call_id: CallId::new()
                }
            )
            .is_empty()
        );
        assert!(state.modal.is_some());

        assert_eq!(
            reduce(&mut state, TuiAction::ApproveInteraction { call_id }),
            vec![TuiEffect::Dispatch(TerminalAction::ApproveTool { call_id })]
        );
        assert!(state.modal.is_none());
        assert!(reduce(&mut state, TuiAction::ApproveInteraction { call_id }).is_empty());

        state.modal = Some(approval_modal(call_id));
        assert_eq!(
            reduce(&mut state, TuiAction::RejectInteraction { call_id }),
            vec![TuiEffect::Dispatch(TerminalAction::RejectTool { call_id })]
        );
        assert!(state.modal.is_none());
    }

    #[test]
    fn input_resolution_preserves_empty_and_whitespace_answers_exactly() {
        for answer in ["", "  \t  "] {
            let input_id = CallId::new();
            let mut state = TuiState {
                modal: Some(input_modal(input_id, answer)),
                ..TuiState::default()
            };

            assert_eq!(
                reduce(&mut state, TuiAction::SubmitInteraction { input_id }),
                vec![TuiEffect::Dispatch(TerminalAction::SubmitInput {
                    input_id,
                    answer: answer.to_string(),
                })]
            );
            assert!(state.modal.is_none());
        }
    }

    #[test]
    fn input_resolution_rejects_wrong_id_and_wrong_modal_variant() {
        let input_id = CallId::new();
        let mut state = TuiState {
            modal: Some(input_modal(input_id, "main")),
            ..TuiState::default()
        };

        assert!(
            reduce(
                &mut state,
                TuiAction::SubmitInteraction {
                    input_id: CallId::new()
                }
            )
            .is_empty()
        );
        assert!(
            reduce(
                &mut state,
                TuiAction::ApproveInteraction { call_id: input_id }
            )
            .is_empty()
        );
        assert_eq!(state.modal, Some(input_modal(input_id, "main")));
    }

    #[test]
    fn modal_input_edits_unicode_and_enforces_the_utf8_byte_limit() {
        let input_id = CallId::new();
        let mut state = TuiState {
            composer: "composer stays untouched".to_string(),
            modal: Some(input_modal(input_id, "你")),
            ..TuiState::default()
        };

        reduce(&mut state, TuiAction::InsertChar('🙂'));
        reduce(&mut state, TuiAction::Backspace);
        reduce(&mut state, TuiAction::InsertChar('界'));
        assert_eq!(state.modal, Some(input_modal(input_id, "你界")));
        assert_eq!(state.composer, "composer stays untouched");

        state.modal = Some(input_modal(
            input_id,
            "x".repeat(MAX_INTERACTION_INPUT_BYTES - 1),
        ));
        reduce(&mut state, TuiAction::InsertChar('x'));
        reduce(&mut state, TuiAction::InsertChar('界'));
        let InteractionModalView::Input { draft, .. } = state.modal.as_ref().unwrap() else {
            panic!("expected input modal");
        };
        assert_eq!(draft.len(), MAX_INTERACTION_INPUT_BYTES);
        assert!(draft.is_char_boundary(draft.len()));
    }

    #[test]
    fn modal_blocks_composer_focus_scroll_and_untyped_resolution() {
        let call_id = CallId::new();
        let mut state = TuiState {
            composer: "keep this draft".to_string(),
            focus: TuiFocus::Transcript,
            modal: Some(approval_modal(call_id)),
            ..TuiState::default()
        };
        state.transcript_scroll.set_viewport(10, 4);

        let actions = [
            TuiAction::InsertChar('y'),
            TuiAction::Backspace,
            TuiAction::SubmitComposer,
            TuiAction::FocusNext,
            TuiAction::ScrollUp(3),
            TuiAction::ScrollPageUp,
            TuiAction::Terminal(TerminalAction::ApproveTool { call_id }),
            TuiAction::Terminal(TerminalAction::SubmitInput {
                input_id: call_id,
                answer: "bypass".to_string(),
            }),
        ];
        for action in actions {
            assert!(reduce(&mut state, action).is_empty());
        }

        assert_eq!(state.composer, "keep this draft");
        assert_eq!(state.focus, TuiFocus::Transcript);
        assert_eq!(state.transcript_scroll.offset, 0);
        assert_eq!(state.modal, Some(approval_modal(call_id)));
    }

    #[test]
    fn global_cancel_and_confirmed_exit_clear_live_modals() {
        let call_id = CallId::new();
        let mut cancelled = TuiState {
            run_lifecycle: RunLifecycle::Running,
            modal: Some(approval_modal(call_id)),
            ..TuiState::default()
        };
        assert_eq!(
            reduce(
                &mut cancelled,
                TuiAction::Terminal(TerminalAction::CancelRun)
            ),
            vec![TuiEffect::Dispatch(TerminalAction::CancelRun)]
        );
        assert!(cancelled.modal.is_none());

        let input_id = CallId::new();
        let mut exiting = TuiState {
            run_lifecycle: RunLifecycle::Running,
            modal: Some(input_modal(input_id, "draft")),
            ..TuiState::default()
        };
        assert!(reduce(&mut exiting, TuiAction::Terminal(TerminalAction::Exit)).is_empty());
        assert!(exiting.modal.is_some());
        assert_eq!(
            reduce(&mut exiting, TuiAction::Terminal(TerminalAction::Exit)),
            vec![
                TuiEffect::Dispatch(TerminalAction::CancelRun),
                TuiEffect::ExitAfterRun,
            ]
        );
        assert!(exiting.modal.is_none());
    }
}
