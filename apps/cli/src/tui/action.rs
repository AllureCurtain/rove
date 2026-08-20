use crate::terminal::action::TerminalAction;
use crate::tui::state::ProviderOnboardingFailure;
use crate::tui::state::{
    InteractionModalKind, InteractionModalView, ResumeCandidate, SessionPickerError,
};
use crate::tui::state::{ModelCandidate, ModelPickerError};
use rove_app_bootstrap::ModelSelection;
use rove_runtime::conversation::{ConversationMessage, MessageDomainError};
use rove_runtime::types::{CallId, RunId};

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
    OpenMessageQueue,
    OpenProviderOnboarding,
    CloseOverlay,
    OverlayNext,
    OverlayPrevious,
    OverlayPageUp,
    OverlayPageDown,
    ConfirmOverlay,
    ModelsLoaded {
        candidates: Vec<ModelCandidate>,
        query: String,
        auto_select: bool,
    },
    ModelsLoadFailed {
        error: ModelPickerError,
    },
    ModelSelectionPersisted {
        selection: ModelSelection,
        revision: u64,
    },
    ModelSelectionFailed {
        error: ModelPickerError,
    },
    ProviderOnboardingSucceeded {
        selection: ModelSelection,
        health: String,
    },
    ProviderOnboardingFailed {
        error: ProviderOnboardingFailure,
    },
    ProviderReloaded {
        selection: ModelSelection,
    },
    ProviderReloadFailed {
        error: ProviderOnboardingFailure,
    },
    ProviderProbeSucceeded {
        health: String,
    },
    ProviderProbeFailed {
        error: ProviderOnboardingFailure,
    },
    PromoteSelectedMessage,
    RevokeSelectedMessage,
    MessagesLoaded {
        messages: Vec<ConversationMessage>,
    },
    MessageUpdated {
        message: ConversationMessage,
    },
    MessageOperationFailed {
        error: MessageDomainError,
    },
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
