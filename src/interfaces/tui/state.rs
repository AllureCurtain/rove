use crate::interfaces::terminal::view::{RunViewState, RunViewUpdate};

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
}

impl TranscriptScroll {
    pub fn set_max_offset(&mut self, max_offset: u16) {
        self.max_offset = max_offset;
        self.offset = self.offset.min(max_offset);
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

#[derive(Debug)]
pub struct TuiState {
    pub run: RunViewState,
    pub run_lifecycle: RunLifecycle,
    pub composer: String,
    pub focus: TuiFocus,
    pub transcript_scroll: TranscriptScroll,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub should_quit: bool,
}

impl TuiState {
    pub fn apply_run_update(&mut self, update: RunViewUpdate) {
        let run_started = matches!(&update, RunViewUpdate::RunStarted { .. });
        let run_completed = matches!(&update, RunViewUpdate::RunCompleted { .. });

        if run_started {
            let cancellation_requested = self.run_lifecycle == RunLifecycle::Cancelling;
            self.clear_run_local_ui();
            self.run_lifecycle = if cancellation_requested {
                RunLifecycle::Cancelling
            } else {
                RunLifecycle::Running
            };
        }

        self.run.apply_update(update);

        if run_completed {
            self.run_lifecycle = RunLifecycle::Completed;
        }
    }

    pub fn clear_run_local_ui(&mut self) {
        self.run = RunViewState::default();
        self.transcript_scroll.reset();
    }
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            run: RunViewState::default(),
            run_lifecycle: RunLifecycle::Idle,
            composer: String::new(),
            focus: TuiFocus::Composer,
            transcript_scroll: TranscriptScroll::default(),
            terminal_width: 80,
            terminal_height: 24,
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
}
