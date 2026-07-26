//! API-global product control plane.
//!
//! The coordinator owns this module root, public contracts, and route wiring.
//! Store and transcript workers implement only their dedicated submodules.

mod contracts;
pub(crate) mod migration;
pub(crate) mod routes;
pub(crate) mod store;
pub(crate) mod transcript;

pub use contracts::*;
