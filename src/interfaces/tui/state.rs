use crate::interfaces::terminal::view::{RunViewState, RunViewUpdate};

const MAX_TRANSCRIPT_HISTORY_RUNS: usize = 50;
pub const MAX_COMPOSER_BYTES: usize = 32 * 1024;

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
    pub quit_confirmation: bool,
    pub should_quit: bool,
}

impl TuiState {
    pub fn apply_run_update(&mut self, update: RunViewUpdate) {
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
            self.quit_confirmation = false;
        }
    }

    fn begin_run(&mut self, run_id: crate::core::types::RunId) {
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
            quit_confirmation: false,
            should_quit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::types::{CallId, JobId, RunId, TerminationReason};
    use crate::interfaces::terminal::view::RunViewUpdate;
    use crate::interfaces::tui::state::{RunLifecycle, TuiFocus, TuiState};

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
}
