use std::io::{self, Stdout};
use std::ops::{Deref, DerefMut};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

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

    /// Restores terminal modes immediately. Drop remains a no-op after success
    /// or failure so every cleanup command is attempted at most once.
    pub fn restore(&mut self) -> io::Result<()> {
        self.lifecycle.restore()
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
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
}

struct CrosstermTerminalControl;

impl TerminalControl for CrosstermTerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Hide)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Show)
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
    cursor_hide_attempted: bool,
}

impl<C: TerminalControl> TerminalLifecycle<C> {
    fn enter(control: C) -> io::Result<Self> {
        let mut lifecycle = Self {
            control,
            raw_mode_attempted: false,
            alternate_screen_attempted: false,
            cursor_hide_attempted: false,
        };

        lifecycle.raw_mode_attempted = true;
        lifecycle.control.enable_raw_mode()?;
        lifecycle.alternate_screen_attempted = true;
        lifecycle.control.enter_alternate_screen()?;
        lifecycle.cursor_hide_attempted = true;
        lifecycle.control.hide_cursor()?;

        Ok(lifecycle)
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;

        if self.cursor_hide_attempted {
            self.cursor_hide_attempted = false;
            remember_first_error(&mut first_error, self.control.show_cursor());
        }
        if self.alternate_screen_attempted {
            self.alternate_screen_attempted = false;
            remember_first_error(&mut first_error, self.control.leave_alternate_screen());
        }
        if self.raw_mode_attempted {
            self.raw_mode_attempted = false;
            remember_first_error(&mut first_error, self.control.disable_raw_mode());
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

    use super::{TerminalControl, TerminalLifecycle};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Operation {
        EnableRawMode,
        EnterAlternateScreen,
        HideCursor,
        ShowCursor,
        LeaveAlternateScreen,
        DisableRawMode,
    }

    struct FakeTerminalControl {
        operations: Arc<Mutex<Vec<Operation>>>,
        fail_on: Option<Operation>,
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

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.record(Operation::HideCursor)
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.record(Operation::ShowCursor)
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
        })
        .unwrap();

        drop(lifecycle);

        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::HideCursor,
                Operation::ShowCursor,
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
        });

        assert!(result.is_err());
        assert_eq!(
            *operations.lock().unwrap(),
            vec![
                Operation::EnableRawMode,
                Operation::EnterAlternateScreen,
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]
        );
    }

    #[test]
    fn cleanup_continues_after_a_restore_error() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let mut lifecycle = TerminalLifecycle::enter(FakeTerminalControl {
            operations: Arc::clone(&operations),
            fail_on: None,
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
                Operation::HideCursor,
                Operation::ShowCursor,
                Operation::LeaveAlternateScreen,
                Operation::DisableRawMode,
            ]
        );
    }
}
