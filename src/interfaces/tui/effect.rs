use crate::interfaces::terminal::action::TerminalAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEffect {
    Dispatch(TerminalAction),
    Exit,
}
