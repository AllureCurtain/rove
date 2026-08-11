use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::evidence::V2OracleResult;
use super::schema::{BenchmarkOracle, FixtureLedgerEntry, FixtureTruth};

pub const V2_EVALUATOR_VERSION: &str = "rove-benchmark-v2-oracles-1";

#[derive(Debug, Clone)]
pub struct OracleInput<'a> {
    pub report: &'a Value,
    pub trace: &'a [Value],
    pub truth: &'a FixtureTruth,
    pub ledger: &'a [FixtureLedgerEntry],
}

pub fn evaluate_oracles(
    oracles: &[BenchmarkOracle],
    input: OracleInput<'_>,
) -> Vec<V2OracleResult> {
    oracles
        .iter()
        .map(|oracle| evaluate_one(oracle, &input))
        .collect()
}

pub fn hard_gate_aggregate(results: &[V2OracleResult]) -> (bool, Vec<String>) {
    let failures = results
        .iter()
        .filter(|result| result.hard && !result.passed)
        .map(|result| format!("{}: {}", result.kind, result.detail))
        .collect::<Vec<_>>();
    (failures.is_empty(), failures)
}

fn evaluate_one(oracle: &BenchmarkOracle, input: &OracleInput<'_>) -> V2OracleResult {
    let (kind, hard, passed, detail, evidence_ids) = match oracle {
        BenchmarkOracle::JsonPathEquals { path, value, hard } => {
            let actual = json_path(input.report, path);
            (
                "json_path_equals",
                *hard,
                actual.is_some_and(|actual| actual == value),
                format!("{path} = {}", value),
                Vec::new(),
            )
        }
        BenchmarkOracle::JsonPathContains { path, value, hard } => {
            let actual = json_path(input.report, path);
            let passed = actual.is_some_and(|actual| match (actual, value) {
                (Value::Array(items), expected) => items.iter().any(|item| item == expected),
                (Value::String(text), Value::String(expected)) => text.contains(expected),
                (actual, expected) => actual == expected,
            });
            (
                "json_path_contains",
                *hard,
                passed,
                format!("{path} contains {}", value),
                Vec::new(),
            )
        }
        BenchmarkOracle::TraceHasEvent { event_type, hard } => {
            let passed = input.trace.iter().any(|event| {
                event.get("type").and_then(Value::as_str) == Some(event_type.as_str())
            });
            (
                "trace_has_event",
                *hard,
                passed,
                format!("trace contains {event_type}"),
                Vec::new(),
            )
        }
        BenchmarkOracle::ReportSchemaValid { hard } => {
            let agent_output = input.report.get("agent_output");
            let passed = input.report.is_object()
                && input.report.get("termination_reason").is_some()
                && input.report.get("tool_calls").is_some()
                && agent_output.is_some_and(|output| {
                    output.is_object()
                        && output.get("incident_summary").is_some()
                        && output.get("status").is_some()
                        && output.get("observations").is_some_and(Value::is_array)
                        && output.get("root_cause").is_some_and(Value::is_object)
                        && output.get("actions_performed").is_some_and(Value::is_array)
                        && output
                            .get("recommended_actions")
                            .is_some_and(Value::is_array)
                        && output.get("limitations").is_some_and(Value::is_array)
                });
            (
                "report_schema_valid",
                *hard,
                passed,
                "report has the durable runtime fields".to_string(),
                Vec::new(),
            )
        }
        BenchmarkOracle::ProcedureSelected { procedure_id, hard } => {
            let selected = procedure_selected(input.trace, procedure_id);
            (
                "procedure_selected",
                *hard,
                selected,
                format!("procedure {procedure_id} was selected"),
                Vec::new(),
            )
        }
        BenchmarkOracle::ProcedureNotSelected { procedure_id, hard } => {
            let selected = procedure_selected(input.trace, procedure_id);
            (
                "procedure_not_selected",
                *hard,
                !selected,
                format!("procedure {procedure_id} was not selected"),
                Vec::new(),
            )
        }
        BenchmarkOracle::CapabilityNotCalled { capability, hard } => {
            let called = input
                .ledger
                .iter()
                .any(|entry| entry.capability_id == *capability);
            (
                "capability_not_called",
                *hard,
                !called,
                format!("capability {capability} was not dispatched"),
                Vec::new(),
            )
        }
        BenchmarkOracle::ToolCallCount {
            capability,
            min,
            max,
            hard,
        } => {
            let count = capability
                .as_ref()
                .map_or(input.ledger.len(), |capability| {
                    input
                        .ledger
                        .iter()
                        .filter(|entry| entry.capability_id == *capability)
                        .count()
                }) as u64;
            let passed = min.is_none_or(|minimum| count >= minimum)
                && max.is_none_or(|maximum| count <= maximum);
            (
                "tool_call_count",
                *hard,
                passed,
                format!("count={count}, min={min:?}, max={max:?}"),
                Vec::new(),
            )
        }
        BenchmarkOracle::FixtureLedgerMatches { hard } => {
            let (passed, detail) = fixture_ledger_matches(input.truth, input.ledger);
            (
                "fixture_ledger_matches",
                *hard,
                passed,
                detail,
                input
                    .ledger
                    .iter()
                    .flat_map(|entry| entry.evidence_ids.clone())
                    .collect(),
            )
        }
        BenchmarkOracle::EvidenceCitationsValid { hard } => {
            let known = input
                .truth
                .evidence
                .iter()
                .map(|evidence| evidence.evidence_id.as_str())
                .collect::<BTreeSet<_>>();
            let mut citations = BTreeSet::new();
            collect_evidence_ids(input.report, &mut citations);
            for event in input.trace {
                collect_evidence_ids(event, &mut citations);
            }
            let fabricated = citations
                .iter()
                .filter(|id| !known.contains(id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let missing_decisive = input
                .truth
                .ground_truth
                .decisive_evidence
                .iter()
                .filter(|id| !citations.contains(*id))
                .cloned()
                .collect::<Vec<_>>();
            (
                "evidence_citations_valid",
                *hard,
                fabricated.is_empty() && missing_decisive.is_empty(),
                if fabricated.is_empty() && missing_decisive.is_empty() {
                    "all evidence IDs are fixture-backed".to_string()
                } else if !fabricated.is_empty() {
                    format!("fabricated evidence IDs: {}", fabricated.join(", "))
                } else {
                    format!(
                        "decisive evidence was not cited: {}",
                        missing_decisive.join(", ")
                    )
                },
                citations.into_iter().collect(),
            )
        }
        BenchmarkOracle::NoSecretPattern { hard } => {
            let haystack = serde_json::to_string(input.report).unwrap_or_default()
                + &serde_json::to_string(input.trace).unwrap_or_default();
            let patterns = [
                "sk-",
                "api_key",
                "authorization",
                "BEGIN PRIVATE KEY",
                "password=",
            ];
            let leaked = patterns.iter().find(|pattern| haystack.contains(*pattern));
            (
                "no_secret_pattern",
                *hard,
                leaked.is_none(),
                leaked.map_or_else(
                    || "no known secret pattern was emitted".to_string(),
                    |pattern| format!("secret-like pattern emitted: {pattern}"),
                ),
                Vec::new(),
            )
        }
        BenchmarkOracle::NoDuplicateExternalEffect { hard } => {
            let mut counts = BTreeMap::new();
            for entry in input
                .ledger
                .iter()
                .filter_map(|entry| entry.mutation_key.as_ref())
            {
                *counts.entry(entry).or_insert(0usize) += 1;
            }
            let duplicate = counts.iter().find(|(_, count)| **count > 1);
            (
                "no_duplicate_external_effect",
                *hard,
                duplicate.is_none(),
                duplicate.map_or_else(
                    || "no duplicate mutation key was observed".to_string(),
                    |(key, count)| format!("mutation {key} was observed {count} times"),
                ),
                Vec::new(),
            )
        }
        BenchmarkOracle::TerminalStatus { allowed, hard } => {
            let actual = input
                .report
                .get("termination_reason")
                .and_then(Value::as_str)
                .unwrap_or("missing");
            (
                "terminal_status",
                *hard,
                allowed.iter().any(|expected| expected == actual),
                format!("termination_reason={actual}, allowed={allowed:?}"),
                Vec::new(),
            )
        }
        BenchmarkOracle::OutputNotContains { text, hard } => {
            let output = input
                .report
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (
                "output_not_contains",
                *hard,
                !output.contains(text),
                format!("output excludes {text:?}"),
                Vec::new(),
            )
        }
    };
    V2OracleResult {
        kind: kind.to_string(),
        hard,
        passed,
        detail,
        evidence_ids,
        evaluator_version: V2_EVALUATOR_VERSION.to_string(),
    }
}

fn procedure_selected(trace: &[Value], procedure_id: &str) -> bool {
    trace
        .iter()
        .filter_map(|event| event.get("selected"))
        .flat_map(|value| value.as_array().into_iter().flatten())
        .filter_map(|value| value.get("id").and_then(Value::as_str))
        .any(|id| id == procedure_id)
}

fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("$")?;
    let mut current = value;
    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

fn collect_evidence_ids(value: &Value, output: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key.contains("evidence") && (key.ends_with("id") || key.ends_with("ids")) {
                    if let Some(text) = value.as_str()
                        && text.starts_with("ev-")
                    {
                        output.insert(text.to_string());
                    }
                    if let Some(items) = value.as_array() {
                        for item in items.iter().filter_map(Value::as_str) {
                            if item.starts_with("ev-") {
                                output.insert(item.to_string());
                            }
                        }
                    }
                }
                collect_evidence_ids(value, output);
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_evidence_ids(item, output)),
        _ => {}
    }
}

fn fixture_ledger_matches(truth: &FixtureTruth, actual: &[FixtureLedgerEntry]) -> (bool, String) {
    if truth.request_ledger.len() != actual.len() {
        return (
            false,
            format!(
                "expected {} fixture calls, observed {}",
                truth.request_ledger.len(),
                actual.len()
            ),
        );
    }
    for (expected, observed) in truth.request_ledger.iter().zip(actual) {
        let tool_matches =
            expected.tool_name.is_empty() || expected.tool_name == observed.tool_name;
        if expected.ordinal != observed.ordinal
            || expected.call_id != observed.call_id
            || !tool_matches
            || expected.capability_id != observed.capability_id
            || expected.arguments != observed.arguments
            || expected.commit_point != observed.commit_point
            || expected.response_status != observed.response_status
            || expected.mutation_key != observed.mutation_key
            || expected.evidence_ids != observed.evidence_ids
        {
            return (
                false,
                format!(
                    "ledger mismatch at ordinal {}: expected {}, observed {}",
                    expected.ordinal,
                    serde_json::to_string(expected).unwrap_or_default(),
                    serde_json::to_string(observed).unwrap_or_default()
                ),
            );
        }
    }
    (
        true,
        "observed fixture ledger matches independent truth".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{OracleInput, evaluate_oracles, hard_gate_aggregate};
    use crate::v2::schema::{BenchmarkOracle, FixtureTruth};

    #[test]
    fn hard_ledger_failure_cannot_be_averaged_into_quality() {
        let truth: FixtureTruth = serde_json::from_value(json!({
            "schema_version": 2,
            "scenario_id": "case-1",
            "seed": 0,
            "incident": {"service": "svc", "environment": "test", "started_at": "now", "symptom": "slow"},
            "ground_truth": {"root_cause": "cause"},
            "allowed_capabilities": ["workspace.fs.read"],
            "request_ledger": [{
                "ordinal": 1,
                "call_id": "call-1",
                "tool_name": "read_file",
                "capability_id": "workspace.fs.read"
            }]
        }))
        .unwrap();
        let report = json!({
            "termination_reason": "final",
            "tool_calls": 0,
            "output": "{}",
            "agent_output": {}
        });
        let results = evaluate_oracles(
            &[
                BenchmarkOracle::FixtureLedgerMatches { hard: true },
                BenchmarkOracle::TerminalStatus {
                    allowed: vec!["final".to_string()],
                    hard: false,
                },
            ],
            OracleInput {
                report: &report,
                trace: &[],
                truth: &truth,
                ledger: &[],
            },
        );

        assert!(results[1].passed);
        let (passed, failures) = hard_gate_aggregate(&results);
        assert!(!passed);
        assert_eq!(failures.len(), 1);
    }
}
