//! API-global product control plane.
//!
//! This module owns public contracts and routes, the SQLite ProductStore,
//! browser migration coordination, and canonical-event transcript projection.

mod contracts;
pub(crate) mod migration;
pub(crate) mod routes;
pub(crate) mod store;
pub(crate) mod transcript;

pub use contracts::*;
