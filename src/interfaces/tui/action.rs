use crate::interfaces::terminal::action::TerminalAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    Terminal(TerminalAction),
    InsertChar(char),
    Backspace,
    SubmitComposer,
    FocusNext,
    ScrollUp(u16),
    ScrollDown(u16),
    Resize { width: u16, height: u16 },
    Tick,
}
