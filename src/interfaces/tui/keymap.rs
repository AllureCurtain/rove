use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::state::InteractionModalView;

pub fn map_key_event(event: KeyEvent) -> Option<TuiAction> {
    map_key_event_with_modal(event, None)
}

pub fn map_key_event_with_modal(
    event: KeyEvent,
    modal: Option<&InteractionModalView>,
) -> Option<TuiAction> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    if event.kind == KeyEventKind::Press
        && event.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(ch) = event.code
    {
        if ch.eq_ignore_ascii_case(&'q') {
            return Some(TuiAction::Terminal(TerminalAction::Exit));
        }
        if ch.eq_ignore_ascii_case(&'c') {
            return Some(TuiAction::Terminal(TerminalAction::CancelRun));
        }
    }

    if let Some(modal) = modal {
        return map_modal_key_event(event, modal);
    }

    match (event.code, event.modifiers) {
        (KeyCode::Enter, _) => Some(TuiAction::SubmitComposer),
        (KeyCode::Backspace, _) => Some(TuiAction::Backspace),
        (KeyCode::Tab, _) => Some(TuiAction::FocusNext),
        (KeyCode::PageUp, _) => Some(TuiAction::ScrollPageUp),
        (KeyCode::PageDown, _) => Some(TuiAction::ScrollPageDown),
        (KeyCode::Up, _) => Some(TuiAction::ScrollUp(1)),
        (KeyCode::Down, _) => Some(TuiAction::ScrollDown(1)),
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Some(TuiAction::InsertChar(ch))
        }
        _ => None,
    }
}

fn map_modal_key_event(event: KeyEvent, modal: &InteractionModalView) -> Option<TuiAction> {
    match modal {
        InteractionModalView::Approval { call_id, .. } => {
            if event.kind != KeyEventKind::Press {
                return None;
            }

            match (event.code, event.modifiers) {
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if ch.eq_ignore_ascii_case(&'y') =>
                {
                    Some(TuiAction::ApproveInteraction { call_id: *call_id })
                }
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if ch.eq_ignore_ascii_case(&'n') =>
                {
                    Some(TuiAction::RejectInteraction { call_id: *call_id })
                }
                (KeyCode::Esc, KeyModifiers::NONE) => {
                    Some(TuiAction::RejectInteraction { call_id: *call_id })
                }
                _ => None,
            }
        }
        InteractionModalView::Input { input_id, .. } => {
            match (event.kind, event.code, event.modifiers) {
                (KeyEventKind::Press, KeyCode::Enter, _) => Some(TuiAction::SubmitInteraction {
                    input_id: *input_id,
                }),
                (
                    KeyEventKind::Press | KeyEventKind::Repeat,
                    KeyCode::Backspace,
                    KeyModifiers::NONE,
                ) => Some(TuiAction::Backspace),
                (
                    KeyEventKind::Press | KeyEventKind::Repeat,
                    KeyCode::Char(ch),
                    KeyModifiers::NONE | KeyModifiers::SHIFT,
                ) => Some(TuiAction::InsertChar(ch)),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use crate::core::types::CallId;
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::tui::action::TuiAction;
    use crate::interfaces::tui::state::InteractionModalView;

    use super::{map_key_event, map_key_event_with_modal};

    fn approval_modal(call_id: CallId) -> InteractionModalView {
        InteractionModalView::Approval {
            call_id,
            name: "fs_write".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        }
    }

    fn input_modal(input_id: CallId) -> InteractionModalView {
        InteractionModalView::Input {
            input_id,
            prompt: "Which branch?".to_string(),
            draft: String::new(),
        }
    }

    #[test]
    fn maps_global_commands_navigation_and_printable_characters() {
        let cases = [
            (
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                TuiAction::Terminal(TerminalAction::Exit),
            ),
            (
                KeyEvent::new(KeyCode::Char('C'), KeyModifiers::CONTROL),
                TuiAction::Terminal(TerminalAction::CancelRun),
            ),
            (
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                TuiAction::SubmitComposer,
            ),
            (
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                TuiAction::FocusNext,
            ),
            (
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                TuiAction::Backspace,
            ),
            (
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
                TuiAction::ScrollPageUp,
            ),
            (
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                TuiAction::ScrollPageDown,
            ),
            (
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                TuiAction::ScrollUp(1),
            ),
            (
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                TuiAction::ScrollDown(1),
            ),
            (
                KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
                TuiAction::InsertChar('界'),
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(map_key_event(event), Some(expected));
        }
    }

    #[test]
    fn ignores_key_release_but_accepts_repeat() {
        let released = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        let repeated =
            KeyEvent::new_with_kind(KeyCode::Backspace, KeyModifiers::NONE, KeyEventKind::Repeat);

        assert_eq!(map_key_event(released), None);
        assert_eq!(map_key_event(repeated), Some(TuiAction::Backspace));

        let repeated_quit = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            KeyEventKind::Repeat,
        );
        assert_eq!(map_key_event(repeated_quit), None);
    }

    #[test]
    fn modified_non_command_characters_are_not_inserted() {
        let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);

        assert_eq!(map_key_event(event), None);
    }

    #[test]
    fn approval_modal_maps_only_explicit_press_decisions() {
        let call_id = CallId::new();
        let modal = approval_modal(call_id);
        let cases = [
            (
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                Some(TuiAction::ApproveInteraction { call_id }),
            ),
            (
                KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT),
                Some(TuiAction::ApproveInteraction { call_id }),
            ),
            (
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                Some(TuiAction::RejectInteraction { call_id }),
            ),
            (
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                Some(TuiAction::RejectInteraction { call_id }),
            ),
            (KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), None),
        ];

        for (event, expected) in cases {
            assert_eq!(map_key_event_with_modal(event, Some(&modal)), expected);
        }
    }

    #[test]
    fn approval_modal_never_authorizes_from_repeat_release_or_modified_keys() {
        let call_id = CallId::new();
        let modal = approval_modal(call_id);
        let ignored = [
            KeyEvent::new_with_kind(KeyCode::Char('y'), KeyModifiers::NONE, KeyEventKind::Repeat),
            KeyEvent::new_with_kind(
                KeyCode::Char('Y'),
                KeyModifiers::SHIFT,
                KeyEventKind::Release,
            ),
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
        ];

        for event in ignored {
            assert_eq!(map_key_event_with_modal(event, Some(&modal)), None);
        }
    }

    #[test]
    fn input_modal_submits_on_press_and_accepts_repeat_editing() {
        let input_id = CallId::new();
        let modal = input_modal(input_id);
        let cases = [
            (
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                Some(TuiAction::SubmitInteraction { input_id }),
            ),
            (
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                Some(TuiAction::SubmitInteraction { input_id }),
            ),
            (
                KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
                Some(TuiAction::InsertChar('界')),
            ),
            (
                KeyEvent::new_with_kind(
                    KeyCode::Char('x'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ),
                Some(TuiAction::InsertChar('x')),
            ),
            (
                KeyEvent::new_with_kind(
                    KeyCode::Backspace,
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ),
                Some(TuiAction::Backspace),
            ),
            (KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), None),
            (KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), None),
            (KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), None),
        ];

        for (event, expected) in cases {
            assert_eq!(map_key_event_with_modal(event, Some(&modal)), expected);
        }

        let repeated_enter =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat);
        assert_eq!(map_key_event_with_modal(repeated_enter, Some(&modal)), None);
    }

    #[test]
    fn global_cancel_and_exit_remain_available_over_modals() {
        let approval = approval_modal(CallId::new());
        let input = input_modal(CallId::new());

        for modal in [&approval, &input] {
            assert_eq!(
                map_key_event_with_modal(
                    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    Some(modal),
                ),
                Some(TuiAction::Terminal(TerminalAction::CancelRun))
            );
            assert_eq!(
                map_key_event_with_modal(
                    KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                    Some(modal),
                ),
                Some(TuiAction::Terminal(TerminalAction::Exit))
            );
        }
    }
}
