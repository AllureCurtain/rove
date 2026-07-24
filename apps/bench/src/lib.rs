//! Benchmark harness for deterministic local long-task evaluation.
//!
//! Modules:
//! - [`schema`]: suite/task/check/report types and profile params
//! - [`checks`]: process-evidence validators
//! - [`runner`]: suite execution, cancel+resume, engine glue
//! - [`evidence`]: summary rendering and path helpers
//! - [`suites`]: built-in dataprep generator

mod checks;
mod evidence;
mod runner;
mod schema;
mod suites;

pub use runner::{load_benchmark_suite, run_benchmark_suite};
pub use schema::{
    BenchmarkArtifacts, BenchmarkCheck, BenchmarkFile, BenchmarkOutcome, BenchmarkReport,
    BenchmarkResumeState, BenchmarkSuite, BenchmarkTask, BenchmarkTaskReport, BenchmarkToolCall,
    BenchmarkTurn, CheckResult, ProfileParams, SuiteInfo, default_profile_params, shell_echo,
    stress_profile_params,
};
pub use suites::generate_dataprep_suite;

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // package lives at <workspace>/apps/bench
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn workspace_benchmarks_path(name: &str) -> PathBuf {
    workspace_root().join("benchmarks").join(name)
}

/// Suites exposed to CLI / HTTP / web-ui.
pub fn available_suites() -> Vec<SuiteInfo> {
    vec![
        SuiteInfo {
            name: "dataprep".to_string(),
            description: "Data preparation pipeline with multi-phase tasks, failure recovery, and rich artifacts.".to_string(),
            profiles: vec!["default".to_string(), "stress".to_string()],
        },
        SuiteInfo {
            name: "agent-smoke".to_string(),
            description: "Legacy smoke test suite (loaded from benchmarks/agent-smoke.json)."
                .to_string(),
            profiles: vec!["default".to_string()],
        },
    ]
}

/// Resolve a suite by name and profile.
pub fn resolve_suite(name: &str, profile: &str) -> std::io::Result<BenchmarkSuite> {
    match name {
        "dataprep" => {
            let params = match profile {
                "stress" => stress_profile_params(),
                _ => default_profile_params(),
            };
            Ok(generate_dataprep_suite(&params))
        }
        "agent-smoke" => {
            let path = workspace_benchmarks_path("agent-smoke.json");
            let bytes = std::fs::read(&path)?;
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)
        }
        other => {
            let path = workspace_benchmarks_path(&format!("{other}.json"));
            if path.exists() {
                let bytes = std::fs::read(&path)?;
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("unknown suite: {other}"),
                ))
            }
        }
    }
}
