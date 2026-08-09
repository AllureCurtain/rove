//! MCP client foundation shared by every transport.
//!
//! Layering, outermost first:
//!
//! - [`protocol`] — bounded message, version, and session vocabulary.
//! - [`dispatcher`] — one JSON-RPC correlation authority.
//! - [`http_safety`] — URL/redirect/response validation for HTTP transports.
//! - [`catalog`] — tool discovery, pagination, and atomic snapshots.
//!
//! A transport adapter moves frames. It never decides tool safety, chooses
//! retries, or projects a result. Those decisions belong to the local safety
//! path and the runtime, so a remote server cannot grant itself permission.

pub mod catalog;
pub mod client;
#[cfg(test)]
mod client_tests;
pub mod dispatcher;
#[cfg(test)]
pub mod fixture;
pub mod http_safety;
pub mod protocol;
pub mod result_mapping;
#[cfg(test)]
mod result_mapping_tests;
pub mod streamable_http;
pub mod transport;
