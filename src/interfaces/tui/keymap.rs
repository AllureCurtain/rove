use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;

pub fn map_key_event(event: KeyEvent) -> Option<TuiAction> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    if event.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(ch) = event.code
    {
        if ch.eq_ignore_ascii_case(&'q') {
            return Some(TuiAction::Terminal(TerminalAction::Exit));
        }
        if ch.eq_ignore_ascii_case(&'c') {
            return Some(TuiAction::Terminal(TerminalAction::CancelRun));
        }
    }

    match (event.code, event.modifiers) {
        (KeyCode::Enter, _) => Some(TuiAction::SubmitComposer),
        (KeyCode::Backspace, _) => Some(TuiAction::Backspace),
        (KeyCode::Tab, _) => Some(TuiAction::FocusNext),
        (KeyCode::PageUp, _) => Some(TuiAction::ScrollUp(1)),
        (KeyCode::PageDown, _) => Some(TuiAction::ScrollDown(1)),
        (KeyCode::Up, _) => Some(TuiAction::ScrollUp(1)),
        (KeyCode::Down, _) => Some(TuiAction::ScrollDown(1)),
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Some(TuiAction::InsertChar(ch))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::tui::action::TuiAction;

    use super::map_key_event;

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
                TuiAction::ScrollUp(1),
            ),
            (
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                TuiAction::ScrollDown(1),
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
    }

    #[test]
    fn modified_non_command_characters_are_not_inserted() {
        let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);

        assert_eq!(map_key_event(event), None);
    }
}
