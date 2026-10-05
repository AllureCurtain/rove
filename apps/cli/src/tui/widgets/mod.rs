//! Ratatui widgets owned by the TUI renderer worker.

mod chrome;
mod modal;
mod overlay;
mod transcript;

pub(crate) use chrome::{activity, composer, minimal_line, status_line};
#[cfg(test)]
pub(crate) use modal::modal_area;
pub(crate) use modal::render_modal;
pub(crate) use overlay::render_overlay;
pub(crate) use transcript::{transcript, transcript_viewport};

use rove_runtime::types::TerminationReason;

fn termination_label(reason: &TerminationReason) -> &'static str {
    match reason {
        TerminationReason::Final => "final",
        TerminationReason::StepLimit => "step limit",
        TerminationReason::TokenLimit => "token limit",
        TerminationReason::TimeLimit => "time limit",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    }
}
