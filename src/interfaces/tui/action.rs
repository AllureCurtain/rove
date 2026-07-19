use crate::core::types::{CallId, RunId};
use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::state::{
    InteractionModalKind, InteractionModalView, ResumeCandidate, SessionPickerError,
};

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
    OpenSessionPicker,
    OpenToolDetail,
    OpenHelp,
    CloseOverlay,
    OverlayNext,
    OverlayPrevious,
    OverlayPageUp,
    OverlayPageDown,
    ConfirmOverlay,
    SessionsLoaded {
        candidates: Vec<ResumeCandidate>,
    },
    SessionsLoadFailed {
        error: SessionPickerError,
    },
    ResumeSelectionSucceeded {
        run_id: RunId,
    },
    ResumeSelectionFailed {
        run_id: RunId,
        error: SessionPickerError,
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
