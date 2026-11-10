//! Canonical content hashing (line-ending / order stable).
//!
//! The pure hash functions live in `rove-foundation::hashing`; re-exported
//! here so `rove_agents::hashing::*` resolves at the path runtime code has
//! always used.

pub use rove_foundation::hashing::*;
