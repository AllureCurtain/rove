//! Canonical-event transcript projection lane.
//!
//! C0 foundation intentionally leaves the implementation to the bounded
//! transcript worker. This module must project runtime facts rather than write
//! or persist a second chat history.

mod reader;
mod validation;

pub(crate) use reader::CanonicalProductTranscriptReader;
