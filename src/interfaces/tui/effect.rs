use crate::core::types::RunId;
use crate::interfaces::terminal::action::TerminalAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEffect {
    Dispatch(TerminalAction),
    LoadSessions,
    ResolveResume { run_id: RunId },
    Exit,
    ExitAfterRun,
}
