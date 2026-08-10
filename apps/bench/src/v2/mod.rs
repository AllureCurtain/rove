//! Versioned deterministic Agent evaluation contracts.
//!
//! V2 is deliberately additive to the JSON V1 benchmark runner. It keeps
//! fixture truth and runtime evidence separate, evaluates hard safety gates
//! independently from quality, and writes a content-addressed evidence
//! package. YAML examples in the design are represented as JSON here so the
//! existing no-new-dependency toolchain remains reproducible.

mod evidence;
mod oracles;
mod runner;
pub mod schema;

pub use evidence::{V2CaseReport, V2EvidenceManifest, V2Metrics, V2OracleResult};
pub use oracles::{evaluate_oracles, hard_gate_aggregate};
pub use runner::{load_benchmark_suite_v2, run_benchmark_suite_v2};
pub use schema::*;
