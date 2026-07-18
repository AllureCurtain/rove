use crate::core::types::CallId;
use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::state::{InteractionModalKind, InteractionModalView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    Terminal(TerminalAction),
    OpenInteraction(InteractionModalView),
    CloseInteraction {
        kind: InteractionModalKind,
        request_id: CallId,
    },
    PrepareApproval {
        call_id: CallId,
    },
    ApproveInteraction {
        call_id: CallId,
    },
    RejectInteraction {
        call_id: CallId,
    },
    SubmitInteraction {
        input_id: CallId,
    },
    InsertChar(char),
    Backspace,
    SubmitComposer,
    FocusNext,
    ScrollUp(u16),
    ScrollDown(u16),
    ScrollPageUp,
    ScrollPageDown,
    SetTranscriptViewport {
        max_offset: u16,
        page_size: u16,
    },
    Resize {
        width: u16,
        height: u16,
    },
    Tick,
}
