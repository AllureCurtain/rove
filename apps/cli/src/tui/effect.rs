use crate::terminal::action::TerminalAction;
use rove_app_bootstrap::ModelSelection;
use rove_runtime::types::RunId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEffect {
    Dispatch(TerminalAction),
    LoadSessions,
    ResolveResume {
        run_id: RunId,
    },
    LoadModels {
        query: String,
        auto_select: bool,
    },
    PersistModel {
        selection: ModelSelection,
        expected_revision: u64,
    },
    ResetModel {
        expected_revision: u64,
    },
    Exit,
    ExitAfterRun,
}
