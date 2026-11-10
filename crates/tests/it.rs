//! Shared integration-test binary. Each module is one former test target;
//! run one with a filter, for example
//! `cargo test -p rove-integration-tests --test it e2e::`.

mod support;

#[path = "abort_salvage.rs"]
mod abort_salvage;
#[path = "artifact_compatibility.rs"]
mod artifact_compatibility;
#[path = "bench.rs"]
mod bench;
#[path = "cli_config.rs"]
mod cli_config;
#[path = "cli_repl.rs"]
mod cli_repl;
#[path = "cli_review.rs"]
mod cli_review;
#[path = "cli_sessions.rs"]
mod cli_sessions;
#[path = "code_hygiene.rs"]
mod code_hygiene;
#[path = "e2e.rs"]
mod e2e;
#[path = "embedding_contract.rs"]
mod embedding_contract;
#[path = "event_contract.rs"]
mod event_contract;
#[path = "history_resume.rs"]
mod history_resume;
#[path = "mcp.rs"]
mod mcp;
#[path = "mcp_streamable_http.rs"]
mod mcp_streamable_http;
#[path = "memory_layered.rs"]
mod memory_layered;
#[path = "memory_tool.rs"]
mod memory_tool;
#[path = "model_factory.rs"]
mod model_factory;
#[path = "provider_smoke.rs"]
mod provider_smoke;
#[path = "recovery.rs"]
mod recovery;
#[path = "request_input_tool.rs"]
mod request_input_tool;
#[path = "review.rs"]
mod review;
#[path = "stress.rs"]
mod stress;
#[path = "tool_safety.rs"]
mod tool_safety;
#[path = "workspace_architecture.rs"]
mod workspace_architecture;
#[path = "workspace_product.rs"]
mod workspace_product;
