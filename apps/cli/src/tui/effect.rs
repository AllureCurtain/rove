use crate::terminal::action::TerminalAction;
use rove_runtime::types::RunId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEffect {
    Dispatch(TerminalAction),
    LoadSessions,
    ResolveResume { run_id: RunId },
    Exit,
    ExitAfterRun,
}
