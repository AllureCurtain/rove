use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2EvidenceManifest {
    pub schema_version: u16,
    pub suite: String,
    pub case_count: usize,
    pub started_at: String,
    pub finished_at: String,
    pub git_commit: String,
    pub git_dirty: bool,
    pub provider_profile: String,
    pub network_mode: String,
    pub redaction: String,
    pub package_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2OracleResult {
    pub kind: String,
    pub hard: bool,
    pub passed: bool,
    pub detail: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    pub evaluator_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct V2Metrics {
    pub model_turns: u64,
    pub tool_calls: u64,
    pub tool_failures: u64,
    pub total_tokens: u64,
    pub wall_time_ms: u64,
    pub quality_passed: bool,
    pub safety_passed: bool,
    pub hard_gate_failures: u64,
    pub cost_microunits: u64,
    pub resumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2CaseReport {
    pub case_id: String,
    pub scenario_id: String,
    pub profile: String,
    pub seed: u64,
    pub provider_profile: String,
    pub passed: bool,
    pub hard_gate_passed: bool,
    pub fixture_hash: String,
    pub runtime_report: PathBuf,
    pub runtime_trace: PathBuf,
    pub fixture_ledger: PathBuf,
    pub oracle_results: Vec<V2OracleResult>,
    pub metrics: V2Metrics,
}
