//! Pure text-editing and patch-application kernel for Rove's file tools.
//!
//! Codex alignment Phase 10. This crate deliberately contains **no IO and no
//! async runtime**: every entry point is a pure function from input content to
//! output content, so tool behavior is unit-testable without an agent loop, a
//! workspace, or a filesystem.
//!
//! The shape mirrors codex's `apply-patch` crate:
//!
//! ```text
//! (input_files, patch) -> Result<ApplyOutcome, ApplyError>
//! ```
//!
//! Callers in `rove-runtime` are responsible for reading the input files,
//! enforcing workspace boundaries and capabilities, and writing results back.
//! This crate only decides *what the new bytes should be*.
//!
//! # Error grading
//!
//! [`ApplyError`] separates failures that a model can productively retry with
//! a corrected patch ([`ApplyError::is_retryable`]) from failures that mean the
//! request is impossible as written. Context that could not be located is
//! retryable; a patch that would collide with an unrelated concurrent change is
//! not.

mod apply;
mod diff;
mod matching;
mod patch;

pub use apply::{ApplyError, ApplyOutcome, FileChange, FileChangeKind, apply_patch, replace_once};
pub use diff::{DIFF_CONTEXT_LINES, localized_diff, render_unified_diff};
pub use matching::{MatchConfidence, MatchOutcome, locate_context};
pub use patch::{FileOperation, Hunk, Patch, PatchParseError, parse_patch};

/// Largest rendered diff this crate will produce before truncating, in bytes.
pub const MAX_DIFF_BYTES: usize = 64 * 1024;
