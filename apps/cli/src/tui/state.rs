use crate::terminal::view::{RunViewState, RunViewUpdate, ToolCallStatus};
use rove_runtime::conversation::{ConversationMessage, MessageStatus};
use rove_runtime::types::{CallId, RunId, SessionId, TaskState};

use super::sanitize::{
    sanitize_display_text, sanitize_json_value, sanitize_tool_text, truncate_display_text,
};

const MAX_TRANSCRIPT_HISTORY_RUNS: usize = 50;
pub const MAX_COMPOSER_BYTES: usize = 32 * 1024;
pub const MAX_INTERACTION_INPUT_BYTES: usize = 32 * 1024;
pub const MAX_SESSION_CANDIDATES: usize = 64;
pub const MAX_SESSION_GOAL_CHARS: usize = 160;
pub const MAX_TOOL_DETAIL_ITEMS: usize = 64;
pub const MAX_TOOL_DETAIL_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_HELP_LINES: usize = 64;
pub const MAX_VISIBLE_MESSAGES: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunLifecycle {
    #[default]
    Idle,
    Running,
    Cancelling,
    Completed,
}

impl RunLifecycle {
    pub fn accepts_prompt(self) -> bool {
        matches!(self, Self::Idle | Self::Completed)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

/// Transcript offset measured from the newest content at the bottom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptScroll {
    pub offset: u16,
    pub max_offset: u16,
    pub page_size: u16,
}

impl TranscriptScroll {
    pub fn set_max_offset(&mut self, max_offset: u16) {
        if self.offset > 0 && max_offset > self.max_offset {
            self.offset = self
                .offset
                .saturating_add(max_offset.saturating_sub(self.max_offset));
        }
        self.max_offset = max_offset;
        self.offset = self.offset.min(max_offset);
    }

    pub fn set_viewport(&mut self, max_offset: u16, page_size: u16) {
        self.set_max_offset(max_offset);
        self.page_size = page_size.max(1);
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.offset = self.offset.saturating_add(amount).min(self.max_offset);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.offset = self.offset.saturating_sub(amount);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TuiFocus {
    Transcript,
    #[default]
    Composer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionModalKind {
    Approval,
    Input,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InteractionKeyMode {
    #[default]
    Direct,
    ConfirmWithFunctionKey,
    Unavailable,
}

impl InteractionKeyMode {
    pub fn is_available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionModalView {
    Approval {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
        reason: String,
    },
    Input {
        input_id: CallId,
        prompt: String,
        draft: String,
    },
}

/// A bounded, renderer-safe summary of a persisted task state.
///
/// The full `TaskState` remains owned by the runtime/app. Keeping only the
/// identity and a short, sanitized goal in TUI state prevents an overlay from
/// accidentally retaining history, checkpoints, or provider data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeCandidate {
    pub session_id: SessionId,
    pub job_id: rove_runtime::types::JobId,
    pub run_id: RunId,
    pub goal: String,
    pub step: u32,
}

impl ResumeCandidate {
    pub fn from_task_state(state: &TaskState) -> Self {
        Self {
            session_id: state.session_id,
            job_id: state.job_id,
            run_id: state.run_id,
            goal: {
                let goal =
                    sanitize_tool_text(&state.goal, MAX_SESSION_GOAL_CHARS.saturating_mul(4));
                let goal = sanitize_display_text(&goal, MAX_SESSION_GOAL_CHARS);
                if goal.trim().is_empty() {
                    "(untitled)".to_string()
                } else {
                    goal
                }
            },
            step: state.step,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPickerError {
    LoadFailed,
    Malformed,
    Stale,
    Busy,
}

impl SessionPickerError {
    pub fn label(self) -> &'static str {
        match self {
            Self::LoadFailed => "Unable to load sessions",
            Self::Malformed => "A session has invalid state data",
            Self::Stale => "That session is no longer available",
            Self::Busy => "Cannot resume while a run is active",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPickerState {
    Loading,
    Ready {
        candidates: Vec<ResumeCandidate>,
        selected: usize,
        error: Option<SessionPickerError>,
        resolving: Option<RunId>,
    },
}

impl SessionPickerState {
    pub fn loading() -> Self {
        Self::Loading
    }

    pub fn ready(candidates: Vec<ResumeCandidate>) -> Self {
        let mut candidates = candidates;
        candidates.truncate(MAX_SESSION_CANDIDATES);
        Self::Ready {
            candidates,
            selected: 0,
            error: None,
            resolving: None,
        }
    }

    pub fn candidates(&self) -> &[ResumeCandidate] {
        match self {
            Self::Loading => &[],
            Self::Ready { candidates, .. } => candidates,
        }
    }

    pub fn selected_candidate(&self) -> Option<&ResumeCandidate> {
        match self {
            Self::Ready {
                candidates,
                selected,
                resolving,
                ..
            } if resolving.is_none() => candidates.get(*selected),
            _ => None,
        }
    }

    pub fn is_resolving(&self) -> bool {
        matches!(
            self,
            Self::Ready {
                resolving: Some(_),
                ..
            }
        )
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let Self::Ready {
            candidates,
            selected,
            resolving,
            ..
        } = self
        else {
            return;
        };
        if resolving.is_some() || candidates.is_empty() {
            return;
        }
        let last = candidates.len().saturating_sub(1);
        *selected = if delta.is_negative() {
            selected.saturating_sub(delta.unsigned_abs()).min(last)
        } else {
            selected.saturating_add(delta as usize).min(last)
        };
    }

    pub(crate) fn selected_run_id(&self) -> Option<RunId> {
        self.selected_candidate().map(|candidate| candidate.run_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDetailEntry {
    pub call_id: CallId,
    pub name: String,
    pub status: ToolCallStatus,
    pub args: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

impl ToolDetailEntry {
    fn from_tool(tool: &crate::terminal::view::ToolCallView) -> Self {
        let args = serde_json::to_string_pretty(&sanitize_json_value(&tool.args, 0))
            .unwrap_or_else(|_| "<unavailable>".to_string());
        Self {
            call_id: tool.call_id,
            name: sanitize_display_text(&sanitize_tool_text(&tool.name, 480), 120),
            status: tool.status,
            args: truncate_display_text(&args, MAX_TOOL_DETAIL_TEXT_BYTES),
            output: tool
                .output
                .as_deref()
                .map(|text| sanitize_tool_text(text, MAX_TOOL_DETAIL_TEXT_BYTES)),
            error: tool
                .error
                .as_ref()
                .map(|error| sanitize_tool_text(&error.to_string(), MAX_TOOL_DETAIL_TEXT_BYTES)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDetailState {
    pub entries: Vec<ToolDetailEntry>,
    pub selected: usize,
    pub scroll: u16,
}

impl ToolDetailState {
    fn from_state(state: &TuiState) -> Self {
        let mut entries = Vec::new();
        'runs: for run in state
            .run_history
            .iter()
            .chain(std::iter::once(&state.run))
            .rev()
        {
            for tool in run.tool_calls.iter().rev() {
                if matches!(
                    tool.status,
                    ToolCallStatus::Completed | ToolCallStatus::Failed
                ) {
                    entries.push(ToolDetailEntry::from_tool(tool));
                    if entries.len() == MAX_TOOL_DETAIL_ITEMS {
                        break 'runs;
                    }
                }
            }
        }
        entries.reverse();
        let selected = entries.len().saturating_sub(1);
        Self {
            entries,
            selected,
            scroll: 0,
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        self.selected = if delta.is_negative() {
            self.selected.saturating_sub(delta.unsigned_abs()).min(last)
        } else {
            self.selected.saturating_add(delta as usize).min(last)
        };
        self.scroll = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpState {
    pub scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQueueState {
    pub messages: Vec<ConversationMessage>,
    pub selected: usize,
}

impl MessageQueueState {
    pub fn new(mut messages: Vec<ConversationMessage>) -> Self {
        if messages.len() > MAX_VISIBLE_MESSAGES {
            messages.drain(..messages.len() - MAX_VISIBLE_MESSAGES);
        }
        let selected = messages.len().saturating_sub(1);
        Self { messages, selected }
    }

    pub fn selected(&self) -> Option<&ConversationMessage> {
        self.messages.get(self.selected)
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.messages.is_empty() {
            return;
        }
        let last = self.messages.len() - 1;
        self.selected = if delta.is_negative() {
            self.selected.saturating_sub(delta.unsigned_abs()).min(last)
        } else {
            self.selected.saturating_add(delta as usize).min(last)
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiOverlay {
    SessionPicker(SessionPickerState),
    ToolDetail(ToolDetailState),
    Help(HelpState),
    MessageQueue(MessageQueueState),
}

impl TuiOverlay {
    pub fn title(&self) -> &'static str {
        match self {
            Self::SessionPicker(_) => " Resume session ",
            Self::ToolDetail(_) => " Tool detail ",
            Self::Help(_) => " Help ",
            Self::MessageQueue(_) => " Message delivery ",
        }
    }
}

impl InteractionModalView {
    pub fn kind(&self) -> InteractionModalKind {
        match self {
            Self::Approval { .. } => InteractionModalKind::Approval,
            Self::Input { .. } => InteractionModalKind::Input,
        }
    }

    pub fn request_id(&self) -> CallId {
        match self {
            Self::Approval { call_id, .. } => *call_id,
            Self::Input { input_id, .. } => *input_id,
        }
    }

    pub fn matches_request(&self, kind: InteractionModalKind, request_id: CallId) -> bool {
        self.kind() == kind && self.request_id() == request_id
    }
}

#[derive(Debug, Clone)]
pub struct TuiState {
    pub run_history: Vec<RunViewState>,
    pub run: RunViewState,
    pub run_lifecycle: RunLifecycle,
    pub composer: String,
    pub focus: TuiFocus,
    pub transcript_scroll: TranscriptScroll,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub modal: Option<InteractionModalView>,
    pub interaction_key_mode: InteractionKeyMode,
    pub approval_confirmation: Option<CallId>,
    pub quit_confirmation: bool,
    pub should_quit: bool,
    pub overlay: Option<TuiOverlay>,
    pub active_resume: Option<ResumeCandidate>,
    pub messages: Vec<ConversationMessage>,
    pub message_error: Option<String>,
}

impl TuiState {
    pub fn apply_run_update(&mut self, update: RunViewUpdate) {
        if let RunViewUpdate::MessageDelivery {
            id,
            content,
            status,
            reason,
        } = &update
        {
            let mapped_status = match status {
                crate::terminal::view::MessageDeliveryStatus::Queued => MessageStatus::Queued,
                crate::terminal::view::MessageDeliveryStatus::InterventionRequested => {
                    MessageStatus::InterventionRequested
                }
                crate::terminal::view::MessageDeliveryStatus::AppliedCurrentRun => {
                    MessageStatus::AppliedCurrentRun
                }
                crate::terminal::view::MessageDeliveryStatus::ClaimedSuccessor => {
                    MessageStatus::ClaimedSuccessor
                }
                crate::terminal::view::MessageDeliveryStatus::NeedsAttention => {
                    MessageStatus::NeedsAttention
                }
                crate::terminal::view::MessageDeliveryStatus::Revoked => MessageStatus::Revoked,
            };
            if let Some(existing) = self.messages.iter_mut().find(|message| message.id == *id) {
                existing.status = mapped_status;
                existing.reason = reason.clone();
                if let Some(content) = content {
                    existing.content = content.clone();
                }
            } else if let Some(content) = content {
                self.messages.push(ConversationMessage {
                    id: id.clone(),
                    session_id: self
                        .active_resume
                        .as_ref()
                        .map(|resume| resume.session_id.to_string())
                        .unwrap_or_default(),
                    content: content.clone(),
                    requested_delivery: rove_runtime::conversation::MessageDelivery::Successor,
                    actual_delivery: None,
                    status: mapped_status,
                    sequence: self.messages.len() as i64 + 1,
                    target_run_id: self.run.run_id,
                    reason: reason.clone(),
                });
            }
        }
        let starting_run_id = match &update {
            RunViewUpdate::RunStarted { run_id, .. } => Some(*run_id),
            _ => None,
        };
        let run_completed = matches!(&update, RunViewUpdate::RunCompleted { .. });

        if let Some(run_id) = starting_run_id {
            let cancellation_requested = self.run_lifecycle == RunLifecycle::Cancelling;
            self.begin_run(run_id);
            self.run_lifecycle = if cancellation_requested {
                RunLifecycle::Cancelling
            } else {
                RunLifecycle::Running
            };
            self.quit_confirmation = false;
        }

        self.run.apply_update(update);

        if run_completed {
            self.run_lifecycle = RunLifecycle::Completed;
            self.modal = None;
            self.approval_confirmation = None;
            self.quit_confirmation = false;
        }
    }

    fn begin_run(&mut self, run_id: rove_runtime::types::RunId) {
        if self.run.run_id.is_some_and(|current| current != run_id) {
            let completed_run = std::mem::take(&mut self.run);
            self.run_history.push(completed_run);
            if self.run_history.len() > MAX_TRANSCRIPT_HISTORY_RUNS {
                self.run_history.remove(0);
            }
        } else if self.run.run_id.is_none() {
            self.run = RunViewState::default();
        }
        self.transcript_scroll.reset();
    }

    pub fn clear_run_local_ui(&mut self) {
        self.run = RunViewState::default();
        self.transcript_scroll.reset();
        self.modal = None;
        self.approval_confirmation = None;
    }

    pub fn open_tool_detail(&mut self) {
        self.overlay = Some(TuiOverlay::ToolDetail(ToolDetailState::from_state(self)));
    }

    pub fn replace_messages(&mut self, mut messages: Vec<ConversationMessage>) {
        if messages.len() > MAX_VISIBLE_MESSAGES {
            messages.drain(..messages.len() - MAX_VISIBLE_MESSAGES);
        }
        self.messages = messages;
        if let Some(TuiOverlay::MessageQueue(queue)) = self.overlay.as_mut() {
            *queue = MessageQueueState::new(self.messages.clone());
        }
    }

    pub fn upsert_message(&mut self, message: ConversationMessage) {
        if let Some(existing) = self.messages.iter_mut().find(|item| item.id == message.id) {
            *existing = message;
        } else {
            self.messages.push(message);
            if self.messages.len() > MAX_VISIBLE_MESSAGES {
                self.messages.remove(0);
            }
        }
        if let Some(TuiOverlay::MessageQueue(queue)) = self.overlay.as_mut() {
            *queue = MessageQueueState::new(self.messages.clone());
        }
    }

    pub fn eligible_message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| {
                matches!(
                    message.status,
                    MessageStatus::Queued | MessageStatus::NeedsAttention
                )
            })
            .count()
    }

    pub fn can_accept_resume(&self, run_id: RunId) -> bool {
        matches!(
            &self.overlay,
            Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
                resolving: Some(current),
                ..
            })) if *current == run_id
        )
    }
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            run_history: Vec::new(),
            run: RunViewState::default(),
            run_lifecycle: RunLifecycle::Idle,
            composer: String::new(),
            focus: TuiFocus::Composer,
            transcript_scroll: TranscriptScroll::default(),
            terminal_width: 80,
            terminal_height: 24,
            modal: None,
            interaction_key_mode: InteractionKeyMode::Direct,
            approval_confirmation: None,
            quit_confirmation: false,
            should_quit: false,
            overlay: None,
            active_resume: None,
            messages: Vec::new(),
            message_error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal::view::{RunViewUpdate, ToolCallStatus, ToolCallView};
    use crate::tui::state::{
        InteractionModalKind, InteractionModalView, RunLifecycle, TuiFocus, TuiOverlay, TuiState,
    };
    use rove_core::ToolError;
    use rove_runtime::types::{CallId, JobId, RunId, TerminationReason};

    #[test]
    fn modal_views_expose_stable_kind_and_request_id_without_live_responders() {
        let approval_id = CallId::new();
        let input_id = CallId::new();
        let approval = InteractionModalView::Approval {
            call_id: approval_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        };
        let input = InteractionModalView::Input {
            input_id,
            prompt: "Which branch?".to_string(),
            draft: String::new(),
        };

        assert_eq!(approval.kind(), InteractionModalKind::Approval);
        assert_eq!(approval.request_id(), approval_id);
        assert_eq!(input.kind(), InteractionModalKind::Input);
        assert_eq!(input.request_id(), input_id);
    }

    #[test]
    fn run_updates_track_lifecycle_and_clear_stale_run_local_ui() {
        let mut state = TuiState {
            composer: "next prompt".to_string(),
            focus: TuiFocus::Transcript,
            terminal_width: 120,
            terminal_height: 40,
            ..TuiState::default()
        };
        let first_run = RunId::new();

        state.apply_run_update(RunViewUpdate::RunStarted {
            run_id: first_run,
            job_id: JobId::new(),
            user_message: "first".to_string(),
        });
        state.apply_run_update(RunViewUpdate::AssistantDelta {
            delta: "stale answer".to_string(),
        });
        state.apply_run_update(RunViewUpdate::InputNeeded {
            input_id: CallId::new(),
            prompt: "stale input".to_string(),
        });
        state.transcript_scroll.set_max_offset(20);
        state.transcript_scroll.scroll_up(8);
        state.apply_run_update(RunViewUpdate::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("done".to_string()),
        });

        assert_eq!(state.run_lifecycle, RunLifecycle::Completed);

        let second_run = RunId::new();
        state.apply_run_update(RunViewUpdate::RunStarted {
            run_id: second_run,
            job_id: JobId::new(),
            user_message: "second".to_string(),
        });

        assert_eq!(state.run_lifecycle, RunLifecycle::Running);
        assert_eq!(state.run.run_id, Some(second_run));
        assert_eq!(state.run.user_message.as_deref(), Some("second"));
        assert!(state.run.assistant_text.is_empty());
        assert!(state.run.pending_inputs.is_empty());
        assert!(state.run.completed.is_none());
        assert_eq!(state.run_history.len(), 1);
        assert_eq!(state.run_history[0].user_message.as_deref(), Some("first"));
        assert_eq!(state.run_history[0].assistant_text, "stale answer");
        assert_eq!(state.transcript_scroll.offset, 0);
        assert_eq!(state.transcript_scroll.max_offset, 0);
        assert_eq!(state.composer, "next prompt");
        assert_eq!(state.focus, TuiFocus::Transcript);
        assert_eq!((state.terminal_width, state.terminal_height), (120, 40));
    }

    #[test]
    fn run_start_preserves_an_early_cancellation_request() {
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Cancelling,
            ..TuiState::default()
        };

        state.apply_run_update(RunViewUpdate::RunStarted {
            run_id: RunId::new(),
            job_id: JobId::new(),
            user_message: "cancel me".to_string(),
        });

        assert_eq!(state.run_lifecycle, RunLifecycle::Cancelling);
    }

    #[test]
    fn scrolling_up_keeps_the_same_content_anchored_while_output_grows() {
        let mut scroll = super::TranscriptScroll::default();
        scroll.set_max_offset(10);
        scroll.scroll_up(3);

        scroll.set_max_offset(14);

        assert_eq!(scroll.offset, 7);
        assert_eq!(scroll.max_offset, 14);

        scroll.set_max_offset(2);
        assert_eq!(scroll.offset, 2);
    }

    #[test]
    fn tool_detail_keeps_only_completed_or_failed_bounded_safe_entries() {
        let mut state = TuiState::default();
        state.run.tool_calls = vec![
            ToolCallView {
                call_id: CallId::new(),
                name: "running".to_string(),
                args: serde_json::json!({"password": "do-not-show"}),
                status: ToolCallStatus::Started,
                output: Some("Bearer do-not-show".to_string()),
                error: None,
            },
            ToolCallView {
                call_id: CallId::new(),
                name: "completed".to_string(),
                args: serde_json::json!({"nested": {"api_token": "do-not-show"}}),
                status: ToolCallStatus::Completed,
                output: Some("password=do-not-show\nvisible output".to_string()),
                error: None,
            },
            ToolCallView {
                call_id: CallId::new(),
                name: "failed".to_string(),
                args: serde_json::json!({"path": "safe.txt"}),
                status: ToolCallStatus::Failed,
                output: None,
                error: Some(ToolError::ExecutionFailed {
                    reason: "private_key=do-not-show".to_string(),
                }),
            },
        ];

        state.open_tool_detail();

        let Some(TuiOverlay::ToolDetail(detail)) = state.overlay else {
            panic!("expected tool detail");
        };
        assert_eq!(detail.entries.len(), 2);
        let rendered = format!("{detail:?}");
        assert!(!rendered.contains("do-not-show"));
        assert!(rendered.contains("visible output"));
        assert!(rendered.contains("[redacted]"));
    }
}
