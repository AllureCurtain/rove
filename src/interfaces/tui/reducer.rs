use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::effect::TuiEffect;
use crate::interfaces::tui::state::{MAX_COMPOSER_BYTES, RunLifecycle, TuiFocus, TuiState};

pub fn reduce(state: &mut TuiState, action: TuiAction) -> Vec<TuiEffect> {
    if !matches!(
        &action,
        TuiAction::Terminal(TerminalAction::Exit) | TuiAction::Tick | TuiAction::Resize { .. }
    ) {
        state.quit_confirmation = false;
    }

    match action {
        TuiAction::Terminal(TerminalAction::Exit) => {
            if state.run_lifecycle.is_active() {
                if state.quit_confirmation {
                    state.quit_confirmation = false;
                    state.run_lifecycle = RunLifecycle::Cancelling;
                    vec![
                        TuiEffect::Dispatch(TerminalAction::CancelRun),
                        TuiEffect::ExitAfterRun,
                    ]
                } else {
                    state.quit_confirmation = true;
                    Vec::new()
                }
            } else {
                state.should_quit = true;
                vec![TuiEffect::Exit]
            }
        }
        TuiAction::Terminal(TerminalAction::CancelRun) => cancel_or_clear(state),
        TuiAction::Terminal(TerminalAction::SubmitPrompt(message)) => {
            dispatch_prompt(state, message)
        }
        TuiAction::Terminal(action) => vec![TuiEffect::Dispatch(action)],
        TuiAction::InsertChar(ch) => {
            if state.focus == TuiFocus::Composer
                && state.composer.len().saturating_add(ch.len_utf8()) <= MAX_COMPOSER_BYTES
            {
                state.composer.push(ch);
            }
            Vec::new()
        }
        TuiAction::Backspace => {
            if state.focus == TuiFocus::Composer {
                state.composer.pop();
            }
            Vec::new()
        }
        TuiAction::SubmitComposer => {
            if state.focus != TuiFocus::Composer || !state.run_lifecycle.accepts_prompt() {
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
            state.focus = match state.focus {
                TuiFocus::Transcript => TuiFocus::Composer,
                TuiFocus::Composer => TuiFocus::Transcript,
            };
            Vec::new()
        }
        TuiAction::ScrollUp(amount) => {
            if state.focus == TuiFocus::Transcript {
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollDown(amount) => {
            if state.focus == TuiFocus::Transcript {
                state.transcript_scroll.scroll_down(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageUp => {
            if state.focus == TuiFocus::Transcript {
                let amount = state.transcript_scroll.page_size.max(1);
                state.transcript_scroll.scroll_up(amount);
            }
            Vec::new()
        }
        TuiAction::ScrollPageDown => {
            if state.focus == TuiFocus::Transcript {
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
    if !state.run_lifecycle.accepts_prompt() || message.trim().is_empty() {
        return Vec::new();
    }

    state.run_lifecycle = RunLifecycle::Running;
    vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message))]
}

#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::tui::action::TuiAction;
    use crate::interfaces::tui::effect::TuiEffect;
    use crate::interfaces::tui::state::{RunLifecycle, TuiFocus, TuiState};

    use super::reduce;

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
}
