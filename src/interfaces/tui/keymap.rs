use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;

pub fn map_key_event(event: KeyEvent) -> Option<TuiAction> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    match (event.code, event.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
            Some(TuiAction::Terminal(TerminalAction::Exit))
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            Some(TuiAction::Terminal(TerminalAction::CancelRun))
        }
        (KeyCode::Enter, _) => Some(TuiAction::SubmitComposer),
        (KeyCode::Backspace, _) => Some(TuiAction::Backspace),
        (KeyCode::Tab, _) => Some(TuiAction::FocusNext),
        (KeyCode::PageUp, _) => Some(TuiAction::ScrollUp(1)),
        (KeyCode::PageDown, _) => Some(TuiAction::ScrollDown(1)),
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Some(TuiAction::InsertChar(ch))
        }
        _ => None,
    }
}
