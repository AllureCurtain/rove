use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::schema::BenchmarkTask;

pub const BENCHMARK_V2_SCHEMA_VERSION: u16 = 2;
pub const BENCHMARK_V2_KIND: &str = "agent_evaluation";
pub const MAX_V2_SCENARIOS: usize = 128;
pub const MAX_V2_ORACLES_PER_SCENARIO: usize = 64;
pub const MAX_V2_FIXTURE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuiteV2 {
    pub schema_version: u16,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub profiles: Vec<String>,
    pub scenarios: Vec<BenchmarkScenarioV2>,
    #[serde(default)]
    pub matrix: BenchmarkMatrix,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkScenarioV2 {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub fixture: String,
    #[serde(default)]
    pub agent: AgentTreatment,
    #[serde(default)]
    pub execution: ExecutionTreatment,
    #[serde(default)]
    pub transport: TransportTreatment,
    #[serde(default)]
    pub failures: Vec<FailureScheduleEntry>,
    #[serde(default)]
    pub oracles: Vec<BenchmarkOracle>,
    /// The scripted task is an ordinary public BenchmarkTask. It is optional
    /// for validation-only cases, but execution cases must provide one.
    #[serde(default)]
    pub task: Option<BenchmarkTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentTreatment {
    pub definition: String,
    pub profile: String,
    pub procedure_mode: String,
}

impl Default for AgentTreatment {
    fn default() -> Self {
        Self {
            definition: "builtin:legacy".to_string(),
            profile: "full".to_string(),
            procedure_mode: "eligible_retrieval".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionTreatment {
    pub strategy: String,
    pub max_model_turns: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_plan_revisions: Option<u32>,
    pub max_steps: Option<u32>,
    pub approval: String,
}

impl Default for ExecutionTreatment {
    fn default() -> Self {
        Self {
            strategy: "plan_react".to_string(),
            max_model_turns: Some(16),
            max_tool_calls: Some(20),
            max_plan_revisions: Some(3),
            max_steps: Some(8),
            approval: "reject_all_mutation".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TransportTreatment {
    pub kind: String,
    pub server_profile: String,
}

impl Default for TransportTreatment {
    fn default() -> Self {
        Self {
            kind: "direct".to_string(),
            server_profile: "local-deterministic".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureScheduleEntry {
    pub at: FailurePoint,
    pub outcome: FailureOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePoint {
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub lifecycle: Option<String>,
    pub occurrence: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureOutcome {
    JsonRpcError { code: i64 },
    Timeout,
    Disconnect,
    Partial,
    CancelRun,
    ResponseLossAfterCommit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchmarkOracle {
    JsonPathEquals {
        path: String,
        value: serde_json::Value,
        #[serde(default)]
        hard: bool,
    },
    JsonPathContains {
        path: String,
        value: serde_json::Value,
        #[serde(default)]
        hard: bool,
    },
    TraceHasEvent {
        event_type: String,
        #[serde(default)]
        hard: bool,
    },
    ReportSchemaValid {
        #[serde(default)]
        hard: bool,
    },
    ProcedureSelected {
        procedure_id: String,
        #[serde(default)]
        hard: bool,
    },
    ProcedureNotSelected {
        procedure_id: String,
        #[serde(default)]
        hard: bool,
    },
    CapabilityNotCalled {
        capability: String,
        #[serde(default)]
        hard: bool,
    },
    ToolCallCount {
        #[serde(default)]
        capability: Option<String>,
        #[serde(default)]
        min: Option<u64>,
        #[serde(default)]
        max: Option<u64>,
        #[serde(default)]
        hard: bool,
    },
    FixtureLedgerMatches {
        #[serde(default)]
        hard: bool,
    },
    EvidenceCitationsValid {
        #[serde(default)]
        hard: bool,
    },
    NoSecretPattern {
        #[serde(default)]
        hard: bool,
    },
    NoDuplicateExternalEffect {
        #[serde(default)]
        hard: bool,
    },
    TerminalStatus {
        allowed: Vec<String>,
        #[serde(default)]
        hard: bool,
    },
    OutputNotContains {
        text: String,
        #[serde(default)]
        hard: bool,
    },
}

impl BenchmarkOracle {
    pub fn is_hard(&self) -> bool {
        match self {
            Self::JsonPathEquals { hard, .. }
            | Self::JsonPathContains { hard, .. }
            | Self::TraceHasEvent { hard, .. }
            | Self::ReportSchemaValid { hard }
            | Self::ProcedureSelected { hard, .. }
            | Self::ProcedureNotSelected { hard, .. }
            | Self::CapabilityNotCalled { hard, .. }
            | Self::ToolCallCount { hard, .. }
            | Self::FixtureLedgerMatches { hard }
            | Self::EvidenceCitationsValid { hard }
            | Self::NoSecretPattern { hard }
            | Self::NoDuplicateExternalEffect { hard }
            | Self::TerminalStatus { hard, .. }
            | Self::OutputNotContains { hard, .. } => *hard,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchmarkMatrix {
    pub profiles: Vec<String>,
    pub seeds: Vec<u64>,
    pub provider_profiles: Vec<String>,
}

impl Default for BenchmarkMatrix {
    fn default() -> Self {
        Self {
            profiles: vec!["full".to_string()],
            seeds: vec![0],
            provider_profiles: vec!["fake_contract".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureTruth {
    pub schema_version: u16,
    pub scenario_id: String,
    pub seed: u64,
    pub incident: FixtureIncident,
    pub ground_truth: GroundTruth,
    #[serde(default)]
    pub evidence: Vec<FixtureEvidence>,
    #[serde(default)]
    pub allowed_capabilities: BTreeSet<String>,
    /// Authoritative fixture binding used to project observed tool names into
    /// stable capability identities. It is configuration, not a call ledger.
    #[serde(default)]
    pub tool_capabilities: BTreeMap<String, String>,
    #[serde(default)]
    pub forbidden_actions: BTreeSet<String>,
    #[serde(default)]
    pub request_ledger: Vec<FixtureLedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureIncident {
    pub service: String,
    pub environment: String,
    pub started_at: String,
    pub symptom: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruth {
    pub root_cause: String,
    #[serde(default)]
    pub acceptable_root_causes: BTreeSet<String>,
    #[serde(default)]
    pub decisive_evidence: BTreeSet<String>,
    #[serde(default)]
    pub prohibited_claims: Vec<String>,
    #[serde(default)]
    pub expected_terminal_status: Option<String>,
    #[serde(default)]
    pub expected_procedure_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEvidence {
    pub evidence_id: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub fields: serde_json::Value,
    #[serde(default)]
    pub supports: BTreeSet<String>,
    #[serde(default)]
    pub injection_payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureLedgerEntry {
    pub ordinal: u64,
    pub call_id: String,
    #[serde(default)]
    pub tool_name: String,
    pub capability_id: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub commit_point: String,
    #[serde(default)]
    pub response_status: String,
    #[serde(default)]
    pub mutation_key: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

impl BenchmarkSuiteV2 {
    pub fn validate(&self, suite_root: &Path) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.schema_version != BENCHMARK_V2_SCHEMA_VERSION {
            errors.push(format!(
                "schema_version must be {BENCHMARK_V2_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        if self.kind != BENCHMARK_V2_KIND {
            errors.push(format!("kind must be {BENCHMARK_V2_KIND}"));
        }
        if self.name.trim().is_empty() {
            errors.push("name must not be empty".to_string());
        }
        if self.name.len() > 128 {
            errors.push("name exceeds 128 bytes".to_string());
        }
        if self.scenarios.is_empty() {
            errors.push("scenarios must contain at least one scenario".to_string());
        }
        if self.scenarios.len() > MAX_V2_SCENARIOS {
            errors.push(format!(
                "scenarios exceeds the {MAX_V2_SCENARIOS} case bound"
            ));
        }
        let mut declared_profiles = BTreeSet::new();
        for profile in &self.profiles {
            if profile.trim().is_empty() || profile.len() > 64 {
                errors.push("profile names must be non-empty and at most 64 bytes".to_string());
            }
            if !declared_profiles.insert(profile.clone()) {
                errors.push(format!("duplicate profile '{profile}'"));
            }
        }
        let mut ids = BTreeSet::new();
        for scenario in &self.scenarios {
            if !ids.insert(scenario.id.clone()) {
                errors.push(format!("duplicate scenario id '{}'", scenario.id));
            }
            if scenario.id.trim().is_empty() {
                errors.push("scenario id must not be empty".to_string());
            }
            if !portable_component(&scenario.id) {
                errors.push(format!("scenario id '{}' is not portable", scenario.id));
            }
            if scenario.oracles.len() > MAX_V2_ORACLES_PER_SCENARIO {
                errors.push(format!(
                    "scenario {} exceeds the {} oracle bound",
                    scenario.id, MAX_V2_ORACLES_PER_SCENARIO
                ));
            }
            let fixture_relative = Path::new(&scenario.fixture);
            if fixture_relative.is_absolute()
                || fixture_relative.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                errors.push(format!(
                    "scenario {} fixture escapes suite root",
                    scenario.id
                ));
            } else {
                let fixture = suite_root.join(fixture_relative);
                match std::fs::metadata(&fixture) {
                    Ok(metadata) if metadata.is_file() => {
                        if metadata.len() > MAX_V2_FIXTURE_BYTES {
                            errors.push(format!(
                                "scenario {} fixture exceeds {} bytes",
                                scenario.id, MAX_V2_FIXTURE_BYTES
                            ));
                        }
                    }
                    _ => errors.push(format!("scenario {} fixture is missing", scenario.id)),
                }
            }
            if scenario.task.is_none() {
                errors.push(format!("scenario {} has no executable task", scenario.id));
            }
            if scenario
                .failures
                .iter()
                .any(|failure| failure.at.occurrence == 0)
            {
                errors.push(format!(
                    "scenario {} has a zero failure occurrence",
                    scenario.id
                ));
            }
            if !matches!(scenario.execution.strategy.as_str(), "react" | "plan_react") {
                errors.push(format!(
                    "scenario {} has unsupported execution strategy '{}'",
                    scenario.id, scenario.execution.strategy
                ));
            }
            if !matches!(
                scenario.execution.approval.as_str(),
                "auto_approve_read_only" | "reject_all_mutation" | "auto_approve_fixture"
            ) {
                errors.push(format!(
                    "scenario {} has unsupported approval driver '{}'",
                    scenario.id, scenario.execution.approval
                ));
            }
            if !matches!(scenario.transport.kind.as_str(), "direct") {
                errors.push(format!(
                    "scenario {} transport '{}' is not available in the local V2 runner",
                    scenario.id, scenario.transport.kind
                ));
            }
        }
        if self.matrix.seeds.is_empty() {
            errors.push("matrix.seeds must contain at least one seed".to_string());
        }
        if self.profiles.is_empty() {
            errors.push("profiles must contain at least one profile".to_string());
        }
        for profile in &self.matrix.profiles {
            if !declared_profiles.contains(profile) {
                errors.push(format!(
                    "matrix profile '{profile}' is not declared by the suite"
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl FixtureTruth {
    pub fn validate(&self, scenario_id: &str, seed: u64) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.schema_version != BENCHMARK_V2_SCHEMA_VERSION {
            errors.push(format!(
                "fixture schema_version must be {BENCHMARK_V2_SCHEMA_VERSION}"
            ));
        }
        if self.scenario_id != scenario_id || self.seed != seed {
            errors.push(format!(
                "fixture identity mismatch: expected {scenario_id}@{seed}, got {}@{}",
                self.scenario_id, self.seed
            ));
        }
        let mut evidence_ids = BTreeSet::new();
        for evidence in &self.evidence {
            if evidence.evidence_id.trim().is_empty() || evidence.evidence_id.len() > 128 {
                errors.push("fixture evidence IDs must be non-empty and bounded".to_string());
            }
            if !evidence_ids.insert(evidence.evidence_id.clone()) {
                errors.push(format!("duplicate evidence ID '{}'", evidence.evidence_id));
            }
        }
        for decisive in &self.ground_truth.decisive_evidence {
            if !evidence_ids.contains(decisive) {
                errors.push(format!("unknown decisive evidence ID '{decisive}'"));
            }
        }
        let mut ordinals = BTreeSet::new();
        let mut call_ids = BTreeSet::new();
        for entry in &self.request_ledger {
            if entry.ordinal == 0 || !ordinals.insert(entry.ordinal) {
                errors.push(format!(
                    "invalid or duplicate ledger ordinal {}",
                    entry.ordinal
                ));
            }
            if entry.call_id.trim().is_empty() || !call_ids.insert(entry.call_id.clone()) {
                errors.push(format!(
                    "invalid or duplicate ledger call_id '{}'",
                    entry.call_id
                ));
            }
            if !self.allowed_capabilities.contains(&entry.capability_id) {
                errors.push(format!(
                    "ledger capability '{}' is not allowed by the fixture",
                    entry.capability_id
                ));
            }
            for evidence_id in &entry.evidence_ids {
                if !evidence_ids.contains(evidence_id) {
                    errors.push(format!(
                        "ledger references unknown evidence ID '{evidence_id}'"
                    ));
                }
            }
        }
        for (tool, capability) in &self.tool_capabilities {
            if tool.trim().is_empty() || capability.trim().is_empty() {
                errors.push("tool capability bindings must be non-empty".to_string());
            }
            if !self.allowed_capabilities.contains(capability) {
                errors.push(format!(
                    "tool '{tool}' binds capability '{capability}' outside the fixture allow-list"
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn portable_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::{BenchmarkSuiteV2, FixtureTruth};

    #[test]
    fn suite_validation_rejects_fixture_escape_before_execution() {
        let temp = tempfile::TempDir::new().unwrap();
        let suite: BenchmarkSuiteV2 = serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "name": "escape-test",
            "kind": "agent_evaluation",
            "profiles": ["full"],
            "matrix": {"profiles": ["full"], "seeds": [0], "provider_profiles": ["fake_contract"]},
            "scenarios": [{
                "id": "case-1",
                "fixture": "../outside.json",
                "execution": {"strategy": "react", "approval": "reject_all_mutation"},
                "transport": {"kind": "direct"},
                "task": {"name": "case", "message": "test"}
            }]
        }))
        .unwrap();

        let errors = suite.validate(temp.path()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("escapes suite root"))
        );
    }

    #[test]
    fn fixture_validation_rejects_truth_ledger_with_unknown_evidence() {
        let truth: FixtureTruth = serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "scenario_id": "case-1",
            "seed": 0,
            "incident": {"service": "svc", "environment": "test", "started_at": "now", "symptom": "slow"},
            "ground_truth": {"root_cause": "cause", "decisive_evidence": ["ev-known"]},
            "evidence": [{"evidence_id": "ev-known", "kind": "metric", "source": "fixture"}],
            "allowed_capabilities": ["workspace.fs.read"],
            "tool_capabilities": {"read_file": "workspace.fs.read"},
            "request_ledger": [{
                "ordinal": 1,
                "call_id": "call-1",
                "tool_name": "read_file",
                "capability_id": "workspace.fs.read",
                "evidence_ids": ["ev-fabricated"]
            }]
        }))
        .unwrap();

        let errors = truth.validate("case-1", 0).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unknown evidence ID 'ev-fabricated'"))
        );
    }
}
