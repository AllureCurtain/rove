use std::io::{self, Stdout};
use std::ops::{Deref, DerefMut};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::interfaces::tui::state::InteractionKeyMode;

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Owns both the Ratatui terminal and the terminal modes it requires.
///
/// Dropping this value restores every mode whose setup was attempted, including
/// when a later setup step failed.
pub struct TerminalSession {
    terminal: TuiTerminal,
    lifecycle: TerminalLifecycle<CrosstermTerminalControl>,
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        let lifecycle = TerminalLifecycle::enter(CrosstermTerminalControl)?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

        Ok(Self {
            terminal,
            lifecycle,
        })
    }

    /// Restores terminal modes immediately. Drop remains a no-op after a
    /// successful restore and retries only cleanup operations that failed.
    pub fn restore(&mut self) -> io::Result<()> {
        self.lifecycle.restore()
    }

    pub fn interaction_key_mode(&self) -> InteractionKeyMode {
        self.lifecycle.interaction_key_mode
    }
}

impl Deref for TerminalSession {
    type Target = TuiTerminal;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for TerminalSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

trait TerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn enable_bracketed_paste(&mut self) -> io::Result<()>;
    fn keyboard_event_type_support(&mut self) -> io::Result<KeyboardEventTypeSupport>;
    fn push_keyboard_enhancement(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn pop_keyboard_enhancement(&mut self) -> io::Result<()>;
    fn disable_bracketed_paste(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum KeyboardEventTypeSupport {
    Native,
    Enhancement,
    #[default]
    Unavailable,
}

struct CrosstermTerminalControl;

impl TerminalControl for CrosstermTerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn enable_bracketed_paste(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnableBracketedPaste)
    }

    fn keyboard_event_type_support(&mut self) -> io::Result<KeyboardEventTypeSupport> {
        #[cfg(windows)]
        {
            Ok(KeyboardEventTypeSupport::Native)
        }
        #[cfg(not(windows))]
        {
            match crossterm::terminal::supports_keyboard_enhancement() {
                Ok(true) => Ok(KeyboardEventTypeSupport::Enhancement),
                Ok(false) => Ok(KeyboardEventTypeSupport::Unavailable),
                Err(error) => {
                    tracing::debug!(%error, "keyboard enhancement probe unavailable");
                    Ok(KeyboardEventTypeSupport::Unavailable)
                }
            }
        }
    }

    fn push_keyboard_enhancement(&mut self) -> io::Result<()> {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Hide)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Show)
    }

    fn pop_keyboard_enhancement(&mut self) -> io::Result<()> {
        execute!(io::stdout(), PopKeyboardEnhancementFlags)
    }

    fn disable_bracketed_paste(&mut self) -> io::Result<()> {
        execute!(io::stdout(), DisableBracketedPaste)
    }

    fn leave_alternate_screen(&mut self) -> io::Result<()> {
        execute!(io::stdout(), LeaveAlternateScreen)
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }
}

struct TerminalLifecycle<C: TerminalControl> {
    control: C,
    raw_mode_attempted: bool,
    alternate_screen_attempted: bool,
    bracketed_paste_attempted: bool,
    keyboard_enhancement_attempted: bool,
    interaction_key_mode: InteractionKeyMode,
    cursor_hide_attempted: bool,
}

impl<C: TerminalControl> TerminalLifecycle<C> {
    fn enter(control: C) -> io::Result<Self> {
        let mut lifecycle = Self {
            control,
            raw_mode_attempted: false,
            alternate_screen_attempted: false,
            bracketed_paste_attempted: false,
            keyboard_enhancement_attempted: false,
            interaction_key_mode: InteractionKeyMode::Unavailable,
            cursor_hide_attempted: false,
        };

        lifecycle.raw_mode_attempted = true;
        lifecycle.control.enable_raw_mode()?;
        lifecycle.alternate_screen_attempted = true;
        lifecycle.control.enter_alternate_screen()?;
        lifecycle.bracketed_paste_attempted = true;
        lifecycle.control.enable_bracketed_paste()?;
        let keyboard_support = lifecycle.control.keyboard_event_type_support()?;
        if keyboard_support == KeyboardEventTypeSupport::Enhancement {
            lifecycle.keyboard_enhancement_attempted = true;
            lifecycle.control.push_keyboard_enhancement()?;
        }
        lifecycle.interaction_key_mode = match keyboard_support {
            KeyboardEventTypeSupport::Native => InteractionKeyMode::ConfirmWithFunctionKey,
            KeyboardEventTypeSupport::Enhancement => InteractionKeyMode::Direct,
            KeyboardEventTypeSupport::Unavailable => InteractionKeyMode::Unavailable,
        };
        lifecycle.cursor_hide_attempted = true;
        lifecycle.control.hide_cursor()?;

        Ok(lifecycle)
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;

        if self.cursor_hide_attempted {
            let result = self.control.show_cursor();
            if result.is_ok() {
                self.cursor_hide_attempted = false;
            }
            remember_first_error(&mut first_error, result);
        }
        if self.keyboard_enhancement_attempted {
            let result = self.control.pop_keyboard_enhancement();
            if result.is_ok() {
                self.keyboard_enhancement_attempted = false;
                self.interaction_key_mode = InteractionKeyMode::Unavailable;
            }
            remember_first_error(&mut first_error, result);
        }
        if self.bracketed_paste_attempted {
            let result = self.control.disable_bracketed_paste();
            if result.is_ok() {
                self.bracketed_paste_attempted = false;
            }
            remember_first_error(&mut first_error, result);
        }
        if self.alternate_screen_attempted {
            let result = self.control.leave_alternate_screen();
            if result.is_ok() {
                self.alternate_screen_attempted = false;
            }
            remember_first_error(&mut first_error, result);
        }
        if self.raw_mode_attempted {
            let result = self.control.disable_raw_mode();
            if result.is_ok() {
                self.raw_mode_attempted = false;
            }
            remember_first_error(&mut first_error, result);
        }

        if !self.raw_mode_attempted {
            self.interaction_key_mode = InteractionKeyMode::Unavailable;
        }

        first_error.map_or(Ok(()), Err)
    }
}

impl<C: TerminalControl> Drop for TerminalLifecycle<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn remember_first_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if first_error.is_none()
        && let Err(error) = result
    {
        *first_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use crate::interfaces::tui::state::InteractionKeyMode;

    use super::{KeyboardEventTypeSupport, TerminalControl, TerminalLifecycle};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Operation {
        EnableRawMode,
        EnterAlternateScreen,
        EnableBracketedPaste,
        PushKeyboardEnhancement,
        HideCursor,
        ShowCursor,
        PopKeyboardEnhancement,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        DisableRawMode,
    }

    struct FakeTerminalControl {
        operations: Arc<Mutex<Vec<Operation>>>,
        fail_on: Option<Operation>,
        keyboard_support: KeyboardEventTypeSupport,
    }

    impl FakeTerminalControl {
        fn record(&self, operation: Operation) -> io::Result<()> {
            self.operations.lock().unwrap().push(operation);
            if self.fail_on == Some(operation) {
                Err(io::Error::other("injected terminal failure"))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalControl for FakeTerminalControl {
        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.record(Operation::EnableRawMode)
        }

        fn enter_alternate_screen(&mut self) -> io::Result<()> {
            self.record(Operation::EnterAlternateScreen)
        }

        fn enable_bracketed_paste(&mut self) -> io::Result<()> {
            self.record(Operation::EnableBracketedPaste)
        }

        fn keyboard_event_type_support(&mut self) -> io::Result<KeyboardEventTypeSupport> {
            Ok(self.keyboard_support)
        }

        fn push_keyboard_enhancement(&mut self) -> io::Result<()> {
            self.record(Operation::PushKeyboardEnhancement)
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.record(Operation::HideCursor)
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.record(Operation::ShowCursor)
        }

        fn pop_keyboard_enhancement(&mut self) -> io::Result<()> {
            self.record(Operation::PopKeyboardEnhancement)
        }

        fn disable_bracketed_paste(&mut self) -> io::Result<()> {
            self.record(Operation::DisableBracketedPaste)
        }

        fn leave_alternate_screen(&mut self) -> io::Result<()> {
            self.record(Operation::LeaveAlternateScreen)
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.record(Operation::DisableRawMode)
        }
    }

    #[test]
    fn drop_restores_terminal_in_reverse_setup_order() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let lifecycle = TerminalLifecycle::enter(FakeTerminalControl {
            operations: Arc::clone(&operations),
            fail_on: None,
            keyboard_support: KeyboardEventTypeSupport::Enhancement,
        })
        .unwrap();

        drop(lifecycle);

        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::EnableBracketedPaste,
                Operation::PushKeyboardEnhancement,
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::PopKeyboardEnhancement,
                Operation::DisableBracketedPaste,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]
        );
    }

    #[test]
    fn partial_setup_failure_restores_every_attempted_mode() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let result = TerminalLifecycle::enter(FakeTerminalControl {
            operations: Arc::clone(&operations),
            fail_on: Some(Operation::HideCursor),
            keyboard_support: KeyboardEventTypeSupport::Enhancement,
        });

        assert!(result.is_err());
        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::EnableBracketedPaste,
                Operation::PushKeyboardEnhancement,
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::PopKeyboardEnhancement,
                Operation::DisableBracketedPaste,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]
        );
    }

    #[test]
    fn interaction_key_mode_matches_terminal_event_support() {
        for (support, mode, uses_enhancement) in [
            (
                KeyboardEventTypeSupport::Native,
                InteractionKeyMode::ConfirmWithFunctionKey,
                false,
            ),
            (
                KeyboardEventTypeSupport::Enhancement,
                InteractionKeyMode::Direct,
                true,
            ),
            (
                KeyboardEventTypeSupport::Unavailable,
                InteractionKeyMode::Unavailable,
                false,
            ),
        ] {
            let operations = Arc::new(Mutex::new(Vec::new()));
            let lifecycle = TerminalLifecycle::enter(FakeTerminalControl {
                operations: Arc::clone(&operations),
                fail_on: None,
                keyboard_support: support,
            })
            .unwrap();

            assert_eq!(lifecycle.interaction_key_mode, mode);
            drop(lifecycle);
            let mut expected = vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::EnableBracketedPaste,
            ];
            if uses_enhancement {
                expected.push(Operation::PushKeyboardEnhancement);
            }
            expected.extend([Operation::HideCursor, Operation::ShowCursor]);
            if uses_enhancement {
                expected.push(Operation::PopKeyboardEnhancement);
            }
            expected.extend([
                Operation::DisableBracketedPaste,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]);
            assert_eq!(*operations.lock().unwrap(), expected);
        }
    }

    #[test]
    fn cleanup_continues_after_a_restore_error() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let mut lifecycle = TerminalLifecycle::enter(FakeTerminalControl {
            operations: Arc::clone(&operations),
            fail_on: None,
            keyboard_support: KeyboardEventTypeSupport::Enhancement,
        })
        .unwrap();
        lifecycle.control.fail_on = Some(Operation::ShowCursor);

        assert!(lifecycle.restore().is_err());
        drop(lifecycle);

        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::EnableBracketedPaste,
                Operation::PushKeyboardEnhancement,
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::PopKeyboardEnhancement,
                Operation::DisableBracketedPaste,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
                Operation::ShowCursor,
            ]
        );
    }

    #[test]
    fn unwinding_restores_terminal_modes() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let operations = Arc::clone(&operations);
            move || {
                let _lifecycle = TerminalLifecycle::enter(FakeTerminalControl {
                    operations,
                    fail_on: None,
                    keyboard_support: KeyboardEventTypeSupport::Enhancement,
                })
                .unwrap();
                panic!("injected panic");
            }
        }));

        assert!(result.is_err());
        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::EnableBracketedPaste,
                Operation::PushKeyboardEnhancement,
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::PopKeyboardEnhancement,
                Operation::DisableBracketedPaste,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]
        );
    }
}
