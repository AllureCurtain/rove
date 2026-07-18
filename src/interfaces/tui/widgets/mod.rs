//! Ratatui widgets owned by the TUI renderer worker.

mod chrome;
mod transcript;

pub(crate) use chrome::{activity, composer, minimal_line, status_line};
pub(crate) use transcript::{transcript, transcript_viewport};

use crate::core::types::TerminationReason;

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
