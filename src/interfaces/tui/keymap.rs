use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::state::{InteractionKeyMode, InteractionModalView, TuiOverlay};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: &'static str,
    pub action: &'static str,
    pub context: &'static str,
}

/// The help overlay renders this table directly. Keep entries next to the
/// keymap so help cannot drift into documenting unsupported commands.
pub const KEY_BINDINGS: &[KeyBinding] = &[
    KeyBinding {
        key: "Ctrl+Q",
        action: "Exit",
        context: "global",
    },
    KeyBinding {
        key: "Ctrl+C",
        action: "Cancel run / close overlay",
        context: "global",
    },
    KeyBinding {
        key: "Enter",
        action: "Submit prompt",
        context: "composer",
    },
    KeyBinding {
        key: "Tab",
        action: "Focus transcript/composer",
        context: "idle",
    },
    KeyBinding {
        key: "Up/Down",
        action: "Scroll transcript",
        context: "idle",
    },
    KeyBinding {
        key: "PageUp/PageDown",
        action: "Scroll transcript by page",
        context: "idle",
    },
    KeyBinding {
        key: "Ctrl+R",
        action: "Open/resume session picker",
        context: "idle",
    },
    KeyBinding {
        key: "Ctrl+T",
        action: "Open tool detail",
        context: "no interaction",
    },
    KeyBinding {
        key: "F1",
        action: "Open this help",
        context: "no interaction",
    },
    KeyBinding {
        key: "Esc",
        action: "Close overlay / reject approval",
        context: "overlay/modal",
    },
    KeyBinding {
        key: "Enter",
        action: "Select highlighted item",
        context: "overlay",
    },
    KeyBinding {
        key: "Up/Down",
        action: "Move overlay selection",
        context: "overlay",
    },
    KeyBinding {
        key: "Y/N",
        action: "Approve/reject tool",
        context: "approval",
    },
    KeyBinding {
        key: "F8",
        action: "Confirm approval / submit input",
        context: "Windows interaction",
    },
    KeyBinding {
        key: "Enter",
        action: "Submit input",
        context: "input",
    },
];

pub fn key_bindings() -> &'static [KeyBinding] {
    KEY_BINDINGS
}

pub fn map_key_event(event: KeyEvent) -> Option<TuiAction> {
    map_key_event_with_modal(event, None)
}

pub fn map_key_event_with_modal(
    event: KeyEvent,
    modal: Option<&InteractionModalView>,
) -> Option<TuiAction> {
    map_key_event_with_modal_mode(event, modal, InteractionKeyMode::Direct)
}

pub fn map_key_event_with_modal_mode(
    event: KeyEvent,
    modal: Option<&InteractionModalView>,
    interaction_key_mode: InteractionKeyMode,
) -> Option<TuiAction> {
    map_key_event_with_overlay_mode(event, modal, None, interaction_key_mode)
}

pub fn map_key_event_with_overlay_mode(
    event: KeyEvent,
    modal: Option<&InteractionModalView>,
    overlay: Option<&TuiOverlay>,
    interaction_key_mode: InteractionKeyMode,
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
        return map_modal_key_event(event, modal, interaction_key_mode);
    }

    if let Some(overlay) = overlay {
        return map_overlay_key_event(event, overlay);
    }

    match (event.code, event.modifiers) {
        (KeyCode::Char('r'), KeyModifiers::CONTROL)
        | (KeyCode::Char('R'), KeyModifiers::CONTROL) => Some(TuiAction::OpenSessionPicker),
        (KeyCode::Char('t'), KeyModifiers::CONTROL)
        | (KeyCode::Char('T'), KeyModifiers::CONTROL) => Some(TuiAction::OpenToolDetail),
        (KeyCode::F(1), KeyModifiers::NONE) => Some(TuiAction::OpenHelp),
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

fn map_overlay_key_event(event: KeyEvent, overlay: &TuiOverlay) -> Option<TuiAction> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    match (event.code, event.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => Some(TuiAction::CloseOverlay),
        (KeyCode::F(1), KeyModifiers::NONE) => Some(TuiAction::OpenHelp),
        (KeyCode::Char('t'), KeyModifiers::CONTROL)
        | (KeyCode::Char('T'), KeyModifiers::CONTROL)
            if matches!(overlay, TuiOverlay::ToolDetail(_)) =>
        {
            Some(TuiAction::OpenToolDetail)
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL)
        | (KeyCode::Char('R'), KeyModifiers::CONTROL)
            if matches!(overlay, TuiOverlay::SessionPicker(_)) =>
        {
            Some(TuiAction::OpenSessionPicker)
        }
        (KeyCode::Up, KeyModifiers::NONE) => Some(TuiAction::OverlayPrevious),
        (KeyCode::Down, KeyModifiers::NONE) => Some(TuiAction::OverlayNext),
        (KeyCode::PageUp, KeyModifiers::NONE) => Some(TuiAction::OverlayPageUp),
        (KeyCode::PageDown, KeyModifiers::NONE) => Some(TuiAction::OverlayPageDown),
        (KeyCode::Enter, _) => Some(TuiAction::ConfirmOverlay),
        _ => None,
    }
}

fn map_modal_key_event(
    event: KeyEvent,
    modal: &InteractionModalView,
    interaction_key_mode: InteractionKeyMode,
) -> Option<TuiAction> {
    if !interaction_key_mode.is_available() {
        return None;
    }

    match modal {
        InteractionModalView::Approval { call_id, .. } => {
            if event.kind != KeyEventKind::Press {
                return None;
            }

            match (event.code, event.modifiers) {
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if ch.eq_ignore_ascii_case(&'y') =>
                {
                    match interaction_key_mode {
                        InteractionKeyMode::Direct => {
                            Some(TuiAction::ApproveInteraction { call_id: *call_id })
                        }
                        InteractionKeyMode::ConfirmWithFunctionKey => {
                            Some(TuiAction::PrepareApproval { call_id: *call_id })
                        }
                        InteractionKeyMode::Unavailable => None,
                    }
                }
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if ch.eq_ignore_ascii_case(&'n') =>
                {
                    Some(TuiAction::RejectInteraction { call_id: *call_id })
                }
                (KeyCode::Esc, KeyModifiers::NONE) => {
                    Some(TuiAction::RejectInteraction { call_id: *call_id })
                }
                (KeyCode::F(8), KeyModifiers::NONE)
                    if interaction_key_mode == InteractionKeyMode::ConfirmWithFunctionKey =>
                {
                    Some(TuiAction::ApproveInteraction { call_id: *call_id })
                }
                _ => None,
            }
        }
        InteractionModalView::Input { input_id, .. } => {
            match (event.kind, event.code, event.modifiers) {
                (KeyEventKind::Press, KeyCode::Enter, _)
                    if interaction_key_mode == InteractionKeyMode::Direct =>
                {
                    Some(TuiAction::SubmitInteraction {
                        input_id: *input_id,
                    })
                }
                (KeyEventKind::Press, KeyCode::F(8), KeyModifiers::NONE)
                    if interaction_key_mode == InteractionKeyMode::ConfirmWithFunctionKey =>
                {
                    Some(TuiAction::SubmitInteraction {
                        input_id: *input_id,
                    })
                }
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
    use crate::interfaces::tui::state::{
        HelpState, InteractionKeyMode, InteractionModalView, TuiOverlay,
    };

    use super::{
        key_bindings, map_key_event, map_key_event_with_modal, map_key_event_with_modal_mode,
        map_key_event_with_overlay_mode,
    };

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
    fn function_key_mode_separates_text_from_approval_and_submission() {
        let call_id = CallId::new();
        let approval = approval_modal(call_id);
        assert_eq!(
            map_key_event_with_modal_mode(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                Some(&approval),
                InteractionKeyMode::ConfirmWithFunctionKey,
            ),
            Some(TuiAction::PrepareApproval { call_id })
        );
        assert_eq!(
            map_key_event_with_modal_mode(
                KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE),
                Some(&approval),
                InteractionKeyMode::ConfirmWithFunctionKey,
            ),
            Some(TuiAction::ApproveInteraction { call_id })
        );

        let input_id = CallId::new();
        let input = input_modal(input_id);
        assert_eq!(
            map_key_event_with_modal_mode(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                Some(&input),
                InteractionKeyMode::ConfirmWithFunctionKey,
            ),
            None
        );
        assert_eq!(
            map_key_event_with_modal_mode(
                KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE),
                Some(&input),
                InteractionKeyMode::ConfirmWithFunctionKey,
            ),
            Some(TuiAction::SubmitInteraction { input_id })
        );
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

    #[test]
    fn maps_navigation_overlays_and_help_from_the_documented_keymap() {
        assert_eq!(
            map_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(TuiAction::OpenSessionPicker)
        );
        assert_eq!(
            map_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            Some(TuiAction::OpenToolDetail)
        );
        assert_eq!(
            map_key_event(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
            Some(TuiAction::OpenHelp)
        );

        let overlay = TuiOverlay::Help(HelpState { scroll: 0 });
        assert_eq!(
            map_key_event_with_overlay_mode(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                None,
                Some(&overlay),
                InteractionKeyMode::Direct,
            ),
            Some(TuiAction::CloseOverlay)
        );
        assert_eq!(
            map_key_event_with_overlay_mode(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                None,
                Some(&overlay),
                InteractionKeyMode::Direct,
            ),
            Some(TuiAction::OverlayPageDown)
        );

        let keys = key_bindings()
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>();
        for documented in ["Ctrl+Q", "Ctrl+C", "Ctrl+R", "Ctrl+T", "F1", "Esc", "F8"] {
            assert!(keys.contains(&documented), "missing {documented} from help");
        }
    }
}
