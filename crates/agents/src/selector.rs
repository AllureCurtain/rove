//! Agent source selectors (`builtin:legacy`, `workspace:ops-diagnostic`).
//!
//! The selector data types live in `rove-foundation::selector`; re-exported
//! here so `rove_agents::selector::*` resolves at the path runtime code has
//! always used.

pub use rove_foundation::selector::*;
