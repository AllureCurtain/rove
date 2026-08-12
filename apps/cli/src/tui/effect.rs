use crate::terminal::action::TerminalAction;
use rove_runtime::conversation::SessionDeliveryState;
use rove_runtime::types::RunId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEffect {
    Dispatch(TerminalAction),
    LoadSessions,
    ResolveResume {
        run_id: RunId,
    },
    SendMessage {
        content: String,
        session_state: SessionDeliveryState,
        target_run_id: Option<RunId>,
    },
    LoadMessages,
    PromoteMessage {
        message_id: String,
    },
    RevokeMessage {
        message_id: String,
    },
    Exit,
    ExitAfterRun,
}
