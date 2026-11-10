//! Built-in tools, tool executor pipeline, hooks, and input registration.
//!
//! Extracted from `rove-runtime` (the historic `runtime/src/tools/` tree);
//! `rove_runtime::tools::*` and the flat `rove_runtime::{executor, hooks,
//! tool_input}` paths resolve unchanged through re-export shims.

pub mod coding;
pub mod delegate;
pub mod echo;
pub mod executor;
pub mod fs;
pub mod history;
pub mod hooks;
pub mod mcp;
pub mod mcp_config;
pub mod mcp_proxy;
pub mod memory;
pub mod repository;
pub mod request_input;
pub mod review;
pub mod runtime_context;
pub mod search;
pub mod shell;
#[doc(hidden)]
pub mod tool_input;

pub use executor::Executor;
