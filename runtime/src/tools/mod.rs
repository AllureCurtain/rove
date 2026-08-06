//! Built-in tools, tool executor pipeline, hooks, and input registration.

pub mod echo;
pub mod executor;
pub mod fs;
pub mod hooks;
pub mod mcp_config;
pub mod mcp_proxy;
pub mod memory;
pub mod request_input;
pub mod runtime_context;
pub mod search;
pub mod shell;
#[doc(hidden)]
pub mod tool_input;

pub use executor::Executor;
