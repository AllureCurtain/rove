use crate::terminal::action::TerminalAction;
use crate::tui::action::TuiAction;
use crate::tui::effect::TuiEffect;
use crate::tui::state::{
    HelpState, InteractionKeyMode, InteractionModalView, MAX_COMPOSER_BYTES,
    MAX_INTERACTION_INPUT_BYTES, RunLifecycle, SessionPickerError, SessionPickerState, TuiFocus,
    TuiOverlay, TuiState,
};
use rove_runtime::types::CallId;

pub fn reduce(state: &mut TuiState, action: TuiAction) -> Vec<TuiEffect> {
    if !matches!(
        &action,
        TuiAction::Terminal(TerminalAction::Exit)
            | TuiAction::Tick
            | TuiAction::Resize { .. }
            | TuiAction::SessionsLoaded { .. }
            | TuiAction::SessionsLoadFailed { .. }
            | TuiAction::ResumeSelectionFailed { .. }
    ) {
        state.quit_confirmation = false;
    }

    match action {
        TuiAction::OpenInteraction(modal) => {
            if state.modal.is_none() {
                state.overlay = None;
                state.modal = Some(modal);
                state.approval_confirmation = None;
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
                state.approval_confirmation = None;
            }
            Vec::new()
        }
        TuiAction::PrepareApproval { call_id } => prepare_approval(state, call_id),
        TuiAction::ApproveInteraction { call_id } => resolve_approval(state, call_id, true),
        TuiAction::RejectInteraction { call_id } => resolve_approval(state, call_id, false),
        TuiAction::SubmitInteraction { input_id } => resolve_input(state, input_id),
        TuiAction::OpenSessionPicker => open_session_picker(state),
        TuiAction::OpenToolDetail => open_tool_detail(state),
        TuiAction::OpenHelp => open_help(state),
        TuiAction::CloseOverlay => {
            if state.modal.is_none() {
                state.overlay = None;
            }
            Vec::new()
        }
        TuiAction::OverlayNext => move_overlay(state, 1),
        TuiAction::OverlayPrevious => move_overlay(state, -1),
        TuiAction::OverlayPageUp => page_overlay(state, false),
        TuiAction::OverlayPageDown => page_overlay(state, true),
        TuiAction::ConfirmOverlay => confirm_overlay(state),
        TuiAction::SessionsLoaded { candidates } => {
            if matches!(state.overlay, Some(TuiOverlay::SessionPicker(_))) {
                state.overlay = Some(TuiOverlay::SessionPicker(SessionPickerState::ready(
                    candidates,
                )));
            }
            Vec::new()
        }
        TuiAction::SessionsLoadFailed { error } => {
            if let Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
                error: slot,
                resolving,
                ..
            })) = state.overlay.as_mut()
                && resolving.is_none()
            {
                *slot = Some(error);
            } else if matches!(state.overlay, Some(TuiOverlay::SessionPicker(_))) {
                state.overlay = Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
                    candidates: Vec::new(),
                    selected: 0,
                    error: Some(error),
                    resolving: None,
                }));
            }
            Vec::new()
        }
        TuiAction::ResumeSelectionSucceeded { run_id } => {
            if let Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
                candidates,
                selected,
                resolving: Some(current),
                ..
            })) = state.overlay.as_ref()
                && *current == run_id
            {
                state.active_resume = candidates.get(*selected).cloned();
                state.overlay = None;
            }
            Vec::new()
        }
        TuiAction::ResumeSelectionFailed { run_id, error } => {
            if let Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
                candidates,
                selected,
                resolving,
                error: slot,
            })) = state.overlay.as_mut()
                && *resolving == Some(run_id)
            {
                *resolving = None;
                *slot = Some(error);
                if matches!(
                    error,
                    SessionPickerError::Stale | SessionPickerError::Malformed
                ) {
                    if *selected < candidates.len() {
                        candidates.remove(*selected);
                    }
                    *selected = (*selected).min(candidates.len().saturating_sub(1));
                }
            }
            Vec::new()
        }
        TuiAction::Terminal(TerminalAction::Exit) => {
            if state.run_lifecycle.is_active() {
                if state.quit_confirmation {
                    state.quit_confirmation = false;
                    state.run_lifecycle = RunLifecycle::Cancelling;
                    state.modal = None;
                    state.approval_confirmation = None;
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
                state.approval_confirmation = None;
                state.overlay = None;
                state.should_quit = true;
                vec![TuiEffect::Exit]
            }
        }
        TuiAction::Terminal(TerminalAction::CancelRun) => cancel_or_clear(state),
        TuiAction::Terminal(TerminalAction::SubmitPrompt(message))
            if state.modal.is_none() && state.overlay.is_none() =>
        {
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
                && state.overlay.is_none()
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
            } else if state.modal.is_none()
                && state.overlay.is_none()
                && state.focus == TuiFocus::Composer
            {
                state.composer.pop();
            }
            Vec::new()
        }
        TuiAction::SubmitComposer => {
            if state.modal.is_some()
                || state.overlay.is_some()
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
            if state.modal.is_some() || state.overlay.is_some() {
                return Vec::new();
            }
            state.focus = match state.focus {
                TuiFocus::Transcript => TuiFocus::Composer,
                TuiFocus::Composer => TuiFocus::Transcript,
            };
            Vec::new()
        }
        TuiAction::ScrollUp(amount) => {
            if state.modal.is_none()
                && state.overlay.is_none()
                && state.focus == TuiFocus::Transcript
            {
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollDown(amount) => {
            if state.modal.is_none()
                && state.overlay.is_none()
                && state.focus == TuiFocus::Transcript
            {
                state.transcript_scroll.scroll_down(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageUp => {
            if state.modal.is_none()
                && state.overlay.is_none()
                && state.focus == TuiFocus::Transcript
            {
                let amount = state.transcript_scroll.page_size.max(1);
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageDown => {
            if state.modal.is_none()
                && state.overlay.is_none()
                && state.focus == TuiFocus::Transcript
            {
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
    if state.modal.is_none() && state.overlay.take().is_some() {
        return Vec::new();
    }
    state.modal = None;
    state.approval_confirmation = None;
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

fn prepare_approval(state: &mut TuiState, call_id: CallId) -> Vec<TuiEffect> {
    let matches = state.interaction_key_mode == InteractionKeyMode::ConfirmWithFunctionKey
        && state.modal.as_ref().is_some_and(|modal| {
            matches!(
                modal,
                InteractionModalView::Approval {
                    call_id: current,
                    ..
                } if *current == call_id
            )
        });
    if matches {
        state.approval_confirmation = Some(call_id);
    }
    Vec::new()
}

fn dispatch_prompt(state: &mut TuiState, message: String) -> Vec<TuiEffect> {
    if state.modal.is_some()
        || state.overlay.is_some()
        || !state.run_lifecycle.accepts_prompt()
        || message.trim().is_empty()
    {
        return Vec::new();
    }

    state.run_lifecycle = RunLifecycle::Running;
    vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message))]
}

fn open_session_picker(state: &mut TuiState) -> Vec<TuiEffect> {
    if state.modal.is_some() || state.run_lifecycle.is_active() {
        return Vec::new();
    }
    if matches!(state.overlay, Some(TuiOverlay::SessionPicker(_))) {
        state.overlay = None;
        return Vec::new();
    }
    state.overlay = Some(TuiOverlay::SessionPicker(SessionPickerState::loading()));
    vec![TuiEffect::LoadSessions]
}

fn open_tool_detail(state: &mut TuiState) -> Vec<TuiEffect> {
    if state.modal.is_some() {
        return Vec::new();
    }
    if matches!(state.overlay, Some(TuiOverlay::ToolDetail(_))) {
        state.overlay = None;
    } else {
        state.open_tool_detail();
    }
    Vec::new()
}

fn open_help(state: &mut TuiState) -> Vec<TuiEffect> {
    if state.modal.is_some() {
        return Vec::new();
    }
    if matches!(state.overlay, Some(TuiOverlay::Help(_))) {
        state.overlay = None;
    } else {
        state.overlay = Some(TuiOverlay::Help(HelpState { scroll: 0 }));
    }
    Vec::new()
}

fn move_overlay(state: &mut TuiState, delta: isize) -> Vec<TuiEffect> {
    let Some(overlay) = state.overlay.as_mut() else {
        return Vec::new();
    };
    match overlay {
        TuiOverlay::SessionPicker(picker) => picker.move_selection(delta),
        TuiOverlay::ToolDetail(detail) => detail.move_selection(delta),
        TuiOverlay::Help(help) => {
            help.scroll = if delta.is_negative() {
                help.scroll.saturating_sub(delta.unsigned_abs() as u16)
            } else {
                help.scroll
                    .saturating_add(u16::try_from(delta).unwrap_or(u16::MAX))
                    .min(crate::tui::state::MAX_HELP_LINES as u16)
            };
        }
    }
    Vec::new()
}

fn page_overlay(state: &mut TuiState, down: bool) -> Vec<TuiEffect> {
    let Some(overlay) = state.overlay.as_mut() else {
        return Vec::new();
    };
    match overlay {
        TuiOverlay::SessionPicker(picker) => picker.move_selection(if down { 8 } else { -8 }),
        TuiOverlay::ToolDetail(detail) => {
            detail.scroll = if down {
                detail
                    .scroll
                    .saturating_add(8)
                    .min(crate::tui::state::MAX_TOOL_DETAIL_TEXT_BYTES as u16)
            } else {
                detail.scroll.saturating_sub(8)
            };
        }
        TuiOverlay::Help(help) => {
            help.scroll = if down {
                help.scroll
                    .saturating_add(8)
                    .min(crate::tui::state::MAX_HELP_LINES as u16)
            } else {
                help.scroll.saturating_sub(8)
            };
        }
    }
    Vec::new()
}

fn confirm_overlay(state: &mut TuiState) -> Vec<TuiEffect> {
    if state.run_lifecycle.is_active() {
        if let Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
            error, resolving, ..
        })) = state.overlay.as_mut()
        {
            *resolving = None;
            *error = Some(SessionPickerError::Busy);
        }
        return Vec::new();
    }
    let Some(TuiOverlay::SessionPicker(picker)) = state.overlay.as_mut() else {
        return Vec::new();
    };
    let Some(run_id) = picker.selected_run_id() else {
        return Vec::new();
    };
    if let SessionPickerState::Ready {
        resolving, error, ..
    } = picker
    {
        *resolving = Some(run_id);
        *error = None;
    }
    vec![TuiEffect::ResolveResume { run_id }]
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

    if approve
        && match state.interaction_key_mode {
            InteractionKeyMode::Direct => false,
            InteractionKeyMode::ConfirmWithFunctionKey => {
                state.approval_confirmation != Some(call_id)
            }
            InteractionKeyMode::Unavailable => true,
        }
    {
        return Vec::new();
    }

    state.modal = None;
    state.approval_confirmation = None;
    let action = if approve {
        TerminalAction::ApproveTool { call_id }
    } else {
        TerminalAction::RejectTool { call_id }
    };
    vec![TuiEffect::Dispatch(action)]
}

fn resolve_input(state: &mut TuiState, input_id: CallId) -> Vec<TuiEffect> {
    if !state.interaction_key_mode.is_available() {
        return Vec::new();
    }
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
    state.approval_confirmation = None;
    vec![TuiEffect::Dispatch(TerminalAction::SubmitInput {
        input_id,
        answer,
    })]
}

#[cfg(test)]
mod tests {
    use crate::terminal::action::TerminalAction;
    use crate::tui::action::TuiAction;
    use crate::tui::effect::TuiEffect;
    use crate::tui::state::{
        InteractionKeyMode, InteractionModalKind, InteractionModalView,
        MAX_INTERACTION_INPUT_BYTES, ResumeCandidate, RunLifecycle, SessionPickerError, TuiFocus,
        TuiOverlay, TuiState,
    };
    use rove_runtime::types::{CallId, JobId, RunId, SessionId};

    use super::reduce;

    fn approval_modal(call_id: CallId) -> InteractionModalView {
        InteractionModalView::Approval {
            call_id,
            name: "write_file".to_string(),
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
            composer: "x".repeat(crate::tui::state::MAX_COMPOSER_BYTES - 1),
            ..TuiState::default()
        };

        reduce(&mut state, TuiAction::InsertChar('x'));
        reduce(&mut state, TuiAction::InsertChar('界'));

        assert_eq!(state.composer.len(), crate::tui::state::MAX_COMPOSER_BYTES);
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
    fn function_key_mode_requires_a_text_selection_before_approval() {
        let call_id = CallId::new();
        let mut state = TuiState {
            modal: Some(approval_modal(call_id)),
            interaction_key_mode: InteractionKeyMode::ConfirmWithFunctionKey,
            ..TuiState::default()
        };

        assert!(reduce(&mut state, TuiAction::ApproveInteraction { call_id }).is_empty());
        assert_eq!(state.approval_confirmation, None);
        assert!(state.modal.is_some());

        assert!(reduce(&mut state, TuiAction::PrepareApproval { call_id }).is_empty());
        assert_eq!(state.approval_confirmation, Some(call_id));
        assert_eq!(
            reduce(&mut state, TuiAction::ApproveInteraction { call_id }),
            vec![TuiEffect::Dispatch(TerminalAction::ApproveTool { call_id })]
        );
        assert!(state.modal.is_none());
        assert_eq!(state.approval_confirmation, None);
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

    fn candidate() -> ResumeCandidate {
        ResumeCandidate {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "resume me".to_string(),
            step: 2,
        }
    }

    #[test]
    fn session_picker_is_idle_only_and_loads_bounded_candidates() {
        let mut state = TuiState::default();
        assert_eq!(
            reduce(&mut state, TuiAction::OpenSessionPicker),
            vec![TuiEffect::LoadSessions]
        );
        let candidates = (0..(crate::tui::state::MAX_SESSION_CANDIDATES + 5))
            .map(|_| candidate())
            .collect();
        reduce(&mut state, TuiAction::SessionsLoaded { candidates });
        let Some(TuiOverlay::SessionPicker(picker)) = state.overlay.as_ref() else {
            panic!("expected session picker");
        };
        assert_eq!(
            picker.candidates().len(),
            crate::tui::state::MAX_SESSION_CANDIDATES
        );

        state.run_lifecycle = RunLifecycle::Running;
        assert!(reduce(&mut state, TuiAction::OpenSessionPicker).is_empty());
        assert!(matches!(state.overlay, Some(TuiOverlay::SessionPicker(_))));
    }

    #[test]
    fn picker_selection_is_cancelable_and_success_requires_matching_id() {
        let mut state = TuiState::default();
        reduce(&mut state, TuiAction::OpenSessionPicker);
        let selected = candidate();
        let run_id = selected.run_id;
        reduce(
            &mut state,
            TuiAction::SessionsLoaded {
                candidates: vec![selected.clone()],
            },
        );
        assert_eq!(
            reduce(&mut state, TuiAction::ConfirmOverlay),
            vec![TuiEffect::ResolveResume { run_id }]
        );
        reduce(
            &mut state,
            TuiAction::ResumeSelectionSucceeded {
                run_id: RunId::new(),
            },
        );
        assert!(state.overlay.is_some());
        reduce(&mut state, TuiAction::ResumeSelectionSucceeded { run_id });
        assert!(state.overlay.is_none());
        assert_eq!(
            state.active_resume.as_ref().map(|item| item.run_id),
            Some(run_id)
        );

        reduce(&mut state, TuiAction::OpenSessionPicker);
        assert!(matches!(state.overlay, Some(TuiOverlay::SessionPicker(_))));
        reduce(&mut state, TuiAction::CloseOverlay);
        assert!(state.overlay.is_none());
    }

    #[test]
    fn stale_and_malformed_resume_failures_remove_only_the_selected_entry() {
        let mut state = TuiState::default();
        let first = candidate();
        let second = candidate();
        let stale_id = first.run_id;
        reduce(&mut state, TuiAction::OpenSessionPicker);
        reduce(
            &mut state,
            TuiAction::SessionsLoaded {
                candidates: vec![first, second],
            },
        );
        reduce(&mut state, TuiAction::ConfirmOverlay);
        reduce(
            &mut state,
            TuiAction::ResumeSelectionFailed {
                run_id: stale_id,
                error: SessionPickerError::Stale,
            },
        );
        let Some(TuiOverlay::SessionPicker(picker)) = state.overlay.as_ref() else {
            panic!("expected picker after stale failure");
        };
        assert_eq!(picker.candidates().len(), 1);
        assert!(
            picker
                .candidates()
                .iter()
                .all(|item| item.run_id != stale_id)
        );
    }

    #[test]
    fn empty_or_busy_picker_confirmation_fails_closed() {
        let mut state = TuiState::default();
        reduce(&mut state, TuiAction::OpenSessionPicker);
        reduce(
            &mut state,
            TuiAction::SessionsLoaded {
                candidates: Vec::new(),
            },
        );
        assert!(reduce(&mut state, TuiAction::ConfirmOverlay).is_empty());

        let mut busy = TuiState {
            run_lifecycle: RunLifecycle::Running,
            overlay: Some(TuiOverlay::SessionPicker(
                crate::tui::state::SessionPickerState::ready(vec![candidate()]),
            )),
            ..TuiState::default()
        };
        assert!(reduce(&mut busy, TuiAction::ConfirmOverlay).is_empty());
        let Some(TuiOverlay::SessionPicker(picker)) = busy.overlay else {
            panic!("expected busy picker");
        };
        assert!(matches!(
            picker,
            crate::tui::state::SessionPickerState::Ready {
                error: Some(SessionPickerError::Busy),
                ..
            }
        ));
    }
}
