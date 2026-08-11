use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::evidence::{V2CaseReport, V2EvidenceManifest, V2Metrics, V2OracleResult};
use super::oracles::{OracleInput, evaluate_oracles, hard_gate_aggregate};
use super::schema::{
    BENCHMARK_V2_SCHEMA_VERSION, BenchmarkScenarioV2, BenchmarkSuiteV2, FailureOutcome,
    FixtureLedgerEntry, FixtureTruth,
};
use crate::evidence::sanitize_path_component;
use crate::runner::{BenchmarkRunOptions, run_benchmark_task_with_options};
use crate::schema::{BenchmarkFile, BenchmarkOutcome};
use rove_runtime::agents::AgentSelector;
use rove_runtime::execution::{ExecutionPolicy, ExecutionStrategy, StrategySelectionSource};
use rove_runtime::types::{ApprovalDecision, ApprovalPolicy};

pub async fn load_benchmark_suite_v2(path: impl AsRef<Path>) -> std::io::Result<BenchmarkSuiteV2> {
    let path = path.as_ref();
    let bytes = tokio::fs::read(path).await?;
    let suite: BenchmarkSuiteV2 = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    suite
        .validate(path.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|errors| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, errors.join("; "))
        })?;
    Ok(suite)
}

/// Run the deterministic V2 matrix using the normal V1 Engine bridge for the
/// scripted task in each case. The fixture truth and request ledger are read
/// independently, so a model answer cannot manufacture the oracle input.
pub async fn run_benchmark_suite_v2(
    suite: &BenchmarkSuiteV2,
    suite_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    profile_override: Option<&str>,
) -> std::io::Result<V2EvidenceManifest> {
    let suite_root = suite_root.as_ref();
    suite.validate(suite_root).map_err(|errors| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, errors.join("; "))
    })?;
    let started_at = chrono::Utc::now().to_rfc3339();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let evidence_root = output_root.as_ref().join(format!(
        "{stamp}-{}-v2",
        sanitize_path_component(&suite.name)
    ));
    tokio::fs::create_dir_all(&evidence_root).await?;

    let profiles = profile_override
        .map(|profile| vec![profile.to_string()])
        .unwrap_or_else(|| {
            if suite.matrix.profiles.is_empty() {
                suite.profiles.clone()
            } else {
                suite.matrix.profiles.clone()
            }
        });
    let seeds = if suite.matrix.seeds.is_empty() {
        vec![0]
    } else {
        suite.matrix.seeds.clone()
    };
    let providers = if suite.matrix.provider_profiles.is_empty() {
        vec!["fake_contract".to_string()]
    } else {
        suite.matrix.provider_profiles.clone()
    };

    let mut cases = Vec::new();
    for scenario in &suite.scenarios {
        for profile in &profiles {
            for provider in &providers {
                for seed in &seeds {
                    cases.push(
                        run_case(
                            scenario,
                            profile,
                            provider,
                            *seed,
                            suite_root,
                            &evidence_root,
                        )
                        .await?,
                    );
                }
            }
        }
    }

    let git_commit = git_output(["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let git_dirty = git_output(["status", "--porcelain"]).is_some_and(|value| !value.is_empty());
    let raw_manifest = serde_json::json!({
        "schema_version": BENCHMARK_V2_SCHEMA_VERSION,
        "suite": suite.name,
        "cases": cases,
        "git_commit": git_commit,
        "git_dirty": git_dirty,
        "network_mode": "disabled",
        "redaction": "fixture_and_runtime_scan",
    });
    let package_hash = rove_runtime::prompt_metadata::stable_hash(
        &serde_json::to_string(&raw_manifest).unwrap_or_default(),
    );
    let finished_at = chrono::Utc::now().to_rfc3339();
    let manifest = V2EvidenceManifest {
        schema_version: BENCHMARK_V2_SCHEMA_VERSION,
        suite: suite.name.clone(),
        case_count: cases.len(),
        started_at,
        finished_at,
        git_commit: raw_manifest["git_commit"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        git_dirty,
        provider_profile: providers.join(","),
        network_mode: "disabled".to_string(),
        redaction: "fixture_and_runtime_scan".to_string(),
        package_hash,
    };
    tokio::fs::write(
        evidence_root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(std::io::Error::other)?,
    )
    .await?;
    tokio::fs::write(
        evidence_root.join("aggregate.json"),
        serde_json::to_vec_pretty(&cases).map_err(std::io::Error::other)?,
    )
    .await?;
    tokio::fs::write(
        evidence_root.join("DATA_PROVENANCE.md"),
        format!(
            "# Benchmark V2 Data Provenance\n\nSuite: `{}`\nSchema: `{}`\nNetwork: disabled\nProvider profiles: `{}`\nFixture truth is loaded independently from runtime trace and report output. Hard safety gates are never averaged into quality.\n",
            suite.name, BENCHMARK_V2_SCHEMA_VERSION, providers.join(", ")
        ),
    )
    .await?;
    tokio::fs::write(
        evidence_root.join("summary.md"),
        render_summary(&manifest, &cases),
    )
    .await?;

    let passed = cases.iter().all(|case| case.passed);
    if !passed {
        return Err(std::io::Error::other(format!(
            "benchmark V2 hard gate or quality failure; evidence: {}",
            evidence_root.display()
        )));
    }
    Ok(manifest)
}

async fn run_case(
    scenario: &BenchmarkScenarioV2,
    profile: &str,
    provider: &str,
    seed: u64,
    suite_root: &Path,
    evidence_root: &Path,
) -> std::io::Result<V2CaseReport> {
    let case_id = format!("{}@{}@{}@{}", scenario.id, profile, provider, seed);
    let case_path_hash = rove_runtime::prompt_metadata::stable_hash(&case_id);
    let case_path_hash = case_path_hash
        .strip_prefix("sha256:")
        .unwrap_or(&case_path_hash);
    let case_dir = evidence_root
        .join("scenarios")
        .join(format!("case-{}", &case_path_hash[..16]));
    tokio::fs::create_dir_all(&case_dir).await?;
    let fixture_path = bounded_fixture_path(suite_root, &scenario.fixture)?;
    let fixture_bytes = tokio::fs::read(&fixture_path).await?;
    let fixture_hash =
        rove_runtime::prompt_metadata::stable_hash(&String::from_utf8_lossy(&fixture_bytes));
    let truth: FixtureTruth =
        serde_json::from_slice(&fixture_bytes).map_err(std::io::Error::other)?;
    truth.validate(&scenario.id, seed).map_err(|errors| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, errors.join("; "))
    })?;

    let mut task = scenario.task.clone().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "V2 scenario task is required",
        )
    })?;
    // The full case identity remains in V2 evidence. The V1 bridge uses a
    // short internal task component so Windows run paths stay below legacy
    // path limits after the StateStore appends its run ULID.
    task.name = "case".to_string();
    materialize_fixture_inputs(suite_root, scenario, &truth, &mut task)?;
    apply_failure_schedule(scenario, &mut task)?;
    let options = benchmark_options(scenario, profile, task.max_steps)?;
    let raw_root = case_dir.join("raw");
    let task_report = run_benchmark_task_with_options(&task, &raw_root, &options).await?;
    let runtime_dir = case_dir.join("runtime");
    copy_tree(&task_report.artifacts.run_dir, &runtime_dir)?;
    let runtime_report_path = runtime_dir.join("report.json");
    let runtime_trace_path = runtime_dir.join("trace.jsonl");
    let report_text = tokio::fs::read_to_string(&runtime_report_path).await?;
    let report: Value = serde_json::from_str(&report_text).map_err(std::io::Error::other)?;
    let oracle_report = report_with_agent_output(&report);
    let trace_text = tokio::fs::read_to_string(&runtime_trace_path).await?;
    let trace = trace_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).map_err(std::io::Error::other))
        .collect::<Result<Vec<_>, _>>()?;

    let fixture_dir = case_dir.join("fixture");
    let fixture_ledger_path = fixture_dir.join("request-ledger.jsonl");
    if let Some(parent) = fixture_ledger_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let ledger = ledger_from_trace(&trace, &truth);
    let ledger_text = ledger
        .iter()
        .map(|entry| serde_json::to_string(entry).map_err(std::io::Error::other))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    tokio::fs::write(&fixture_ledger_path, format!("{ledger_text}\n")).await?;
    tokio::fs::write(fixture_dir.join("truth.json"), &fixture_bytes).await?;
    tokio::fs::write(
        case_dir.join("oracle-input.json"),
        serde_json::to_vec_pretty(&oracle_report).map_err(std::io::Error::other)?,
    )
    .await?;
    tokio::fs::write(
        case_dir.join("fixture-hash.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scenario_id": scenario.id,
            "seed": seed,
            "sha256": fixture_hash,
        }))
        .map_err(std::io::Error::other)?,
    )
    .await?;

    let oracle_results = evaluate_oracles(
        &scenario.oracles,
        OracleInput {
            report: &oracle_report,
            trace: &trace,
            truth: &truth,
            ledger: &ledger,
        },
    );
    let (hard_gate_passed, _) = hard_gate_aggregate(&oracle_results);
    let quality_passed = oracle_results
        .iter()
        .filter(|result| !result.hard)
        .all(|result| result.passed);
    let metrics = metrics_from_report(
        &report,
        hard_gate_passed,
        quality_passed,
        task_report.resumed,
        &oracle_results,
    );
    let passed =
        hard_gate_passed && quality_passed && task_report.outcome == BenchmarkOutcome::Passed;
    tokio::fs::write(
        case_dir.join("oracles.json"),
        serde_json::to_vec_pretty(&oracle_results).map_err(std::io::Error::other)?,
    )
    .await?;
    tokio::fs::write(
        case_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&metrics).map_err(std::io::Error::other)?,
    )
    .await?;

    Ok(V2CaseReport {
        case_id,
        scenario_id: scenario.id.clone(),
        profile: profile.to_string(),
        seed,
        provider_profile: provider.to_string(),
        passed,
        hard_gate_passed,
        fixture_hash,
        runtime_report: runtime_report_path,
        runtime_trace: runtime_trace_path,
        fixture_ledger: fixture_ledger_path,
        oracle_results,
        metrics,
    })
}

fn benchmark_options(
    scenario: &BenchmarkScenarioV2,
    matrix_profile: &str,
    task_max_steps: u32,
) -> std::io::Result<BenchmarkRunOptions> {
    let selector = AgentSelector::parse(&scenario.agent.definition)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let force_react = matrix_profile == "react_no_procedure";
    let strategy = if force_react || scenario.execution.strategy == "react" {
        ExecutionStrategy::React
    } else {
        ExecutionStrategy::PlanReact
    };
    let max_steps = scenario
        .execution
        .max_steps
        .or((task_max_steps > 0).then_some(task_max_steps))
        .unwrap_or(20);
    let mut policy = ExecutionPolicy::from_max_steps_and_plan_flag(
        max_steps,
        strategy == ExecutionStrategy::PlanReact,
    );
    policy.selection_source = StrategySelectionSource::Request;
    policy.budgets.max_model_turns = scenario.execution.max_model_turns;
    if let (Some(per_step), Some(global)) = (
        policy.budgets.max_model_turns_per_step,
        policy.budgets.max_model_turns,
    ) {
        policy.budgets.max_model_turns_per_step = Some(per_step.min(global));
    }
    policy.budgets.max_tool_calls = scenario.execution.max_tool_calls;
    policy.budgets.max_plan_revisions = scenario.execution.max_plan_revisions;
    if strategy == ExecutionStrategy::PlanReact {
        policy.budgets.max_plan_steps = scenario.execution.max_steps;
        policy.budgets.max_step_attempts = scenario.execution.max_steps.or(Some(max_steps));
    }
    policy.validate().map_err(std::io::Error::other)?;

    let (approval_policy, approval_decision) = match scenario.execution.approval.as_str() {
        "auto_approve_read_only" => (ApprovalPolicy::Ask, ApprovalDecision::Reject),
        "reject_all_mutation" => (ApprovalPolicy::Never, ApprovalDecision::Reject),
        "auto_approve_fixture" => (ApprovalPolicy::Auto, ApprovalDecision::Approve),
        unsupported => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unsupported V2 approval driver '{unsupported}'"),
            ));
        }
    };
    let no_procedure = force_react || scenario.agent.procedure_mode == "disabled";
    Ok(BenchmarkRunOptions {
        workspace_agent_authorized: scenario.agent.definition.starts_with("workspace:"),
        load_workspace_instructions: scenario.agent.definition.starts_with("workspace:"),
        allow_remediation_procedures: scenario.agent.procedure_mode == "remediation",
        max_procedure_selections: Some(if no_procedure { 0 } else { 2 }),
        agent_selector: Some(selector),
        execution_policy: Some(policy),
        approval_policy,
        approval_decision,
    })
}

fn apply_failure_schedule(
    scenario: &BenchmarkScenarioV2,
    task: &mut crate::schema::BenchmarkTask,
) -> std::io::Result<()> {
    for failure in &scenario.failures {
        match &failure.outcome {
            FailureOutcome::CancelRun
                if matches!(
                    failure.at.lifecycle.as_deref(),
                    Some("after_model_turn") | Some("after_step_model_turn")
                ) =>
            {
                task.cancel_resume_after_turns = Some(failure.at.occurrence as usize);
            }
            outcome => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    format!(
                        "failure schedule {:?} at {:?} is not implemented by the local direct runner",
                        outcome, failure.at
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn materialize_fixture_inputs(
    suite_root: &Path,
    scenario: &BenchmarkScenarioV2,
    truth: &FixtureTruth,
    task: &mut crate::schema::BenchmarkTask,
) -> std::io::Result<()> {
    let observation = serde_json::json!({
        "schema_version": truth.schema_version,
        "scenario_id": truth.scenario_id,
        "seed": truth.seed,
        "incident": truth.incident,
        "evidence": truth.evidence,
    });
    task.setup_files
        .retain(|file| file.path != "incident/evidence.json");
    task.setup_files.push(BenchmarkFile {
        path: "incident/evidence.json".to_string(),
        content: serde_json::to_string_pretty(&observation).map_err(std::io::Error::other)?,
    });

    let Some(agent_id) = scenario.agent.definition.strip_prefix("workspace:") else {
        return Ok(());
    };
    let source_root = bounded_fixture_path(&suite_root.join("agents"), agent_id)?;
    if !source_root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("workspace Agent package is missing for {agent_id}"),
        ));
    }
    let mut package_files = Vec::new();
    collect_agent_files(&source_root, &source_root, &mut package_files)?;
    for (relative, content) in package_files {
        let destination = format!(
            "agents/{agent_id}/{}",
            relative.to_string_lossy().replace('\\', "/")
        );
        task.setup_files.retain(|file| file.path != destination);
        task.setup_files.push(BenchmarkFile {
            path: destination,
            content,
        });
    }
    Ok(())
}

fn collect_agent_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<(PathBuf, String)>,
) -> std::io::Result<()> {
    const MAX_AGENT_FILES: usize = 64;
    const MAX_AGENT_FILE_BYTES: u64 = 256 * 1024;
    if output.len() >= MAX_AGENT_FILES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "benchmark Agent package exceeds the file-count bound",
        ));
    }
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "benchmark Agent package may not contain symlinks",
            ));
        }
        if file_type.is_dir() {
            collect_agent_files(root, &entry.path(), output)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if output.len() >= MAX_AGENT_FILES || entry.metadata()?.len() > MAX_AGENT_FILE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "benchmark Agent package exceeds a materialization bound",
            ));
        }
        let bytes = fs::read(entry.path())?;
        let content = String::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "benchmark Agent package files must be UTF-8",
            )
        })?;
        output.push((
            entry
                .path()
                .strip_prefix(root)
                .map_err(std::io::Error::other)?
                .to_path_buf(),
            content,
        ));
    }
    Ok(())
}

fn report_with_agent_output(report: &Value) -> Value {
    let mut report = report.clone();
    let parsed = report
        .get("output")
        .and_then(Value::as_str)
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .unwrap_or(Value::Null);
    if let Some(object) = report.as_object_mut() {
        object.insert("agent_output".to_string(), parsed);
    }
    report
}

fn ledger_from_trace(trace: &[Value], truth: &FixtureTruth) -> Vec<FixtureLedgerEntry> {
    let mut ledger = Vec::new();
    let mut pending = BTreeMap::<String, usize>::new();
    for event in trace {
        match event.get("type").and_then(Value::as_str) {
            Some("tool_call_started") => {
                let internal_id = json_scalar(event.get("call_id"));
                let call_id = event
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| internal_id.clone());
                let tool_name = event
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_tool")
                    .to_string();
                let capability_id = truth
                    .tool_capabilities
                    .get(&tool_name)
                    .cloned()
                    .unwrap_or_else(|| tool_name.clone());
                let index = ledger.len();
                ledger.push(FixtureLedgerEntry {
                    ordinal: (index + 1) as u64,
                    call_id,
                    tool_name,
                    capability_id,
                    arguments: event
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                    commit_point: "dispatched".to_string(),
                    response_status: "pending".to_string(),
                    mutation_key: None,
                    evidence_ids: Vec::new(),
                    session_id: None,
                });
                pending.insert(internal_id, index);
            }
            Some("tool_call_completed") => {
                let internal_id = json_scalar(event.get("call_id"));
                if let Some(index) = pending.remove(&internal_id)
                    && let Some(entry) = ledger.get_mut(index)
                {
                    let result = event.get("result").unwrap_or(&Value::Null);
                    entry.commit_point = "completed".to_string();
                    entry.response_status = result
                        .get("metadata")
                        .and_then(|metadata| metadata.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("ok")
                        .to_string();
                    entry.evidence_ids = evidence_ids_in(result);
                    entry.mutation_key = result
                        .get("mutations")
                        .and_then(Value::as_array)
                        .and_then(|mutations| mutations.first())
                        .and_then(mutation_key);
                }
            }
            Some("tool_call_failed") => {
                let internal_id = json_scalar(event.get("call_id"));
                if let Some(index) = pending.remove(&internal_id)
                    && let Some(entry) = ledger.get_mut(index)
                {
                    entry.commit_point = "failed".to_string();
                    let code = event
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or("tool_error");
                    entry.response_status = format!("error:{code}");
                }
            }
            _ => {}
        }
    }
    ledger
}

fn json_scalar(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| value.map(Value::to_string))
        .unwrap_or_else(|| "missing-call-id".to_string())
}

fn mutation_key(mutation: &Value) -> Option<String> {
    let path = mutation.get("path")?.as_str()?;
    let operation = mutation.get("operation")?.as_str()?;
    Some(format!("{operation}:{path}"))
}

fn evidence_ids_in(value: &Value) -> Vec<String> {
    let mut ids = BTreeSet::new();
    collect_evidence_tokens(value, &mut ids);
    ids.into_iter().collect()
}

fn collect_evidence_tokens(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            for token in text.split(|character: char| {
                !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
            }) {
                if token.starts_with("ev-") && token.len() <= 128 {
                    ids.insert(token.to_string());
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_evidence_tokens(item, ids);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_evidence_tokens(nested, ids);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn metrics_from_report(
    report: &Value,
    safety_passed: bool,
    quality_passed: bool,
    resumed: bool,
    results: &[V2OracleResult],
) -> V2Metrics {
    let usage = &report["execution_lifecycle"]["budget_usage"];
    V2Metrics {
        model_turns: usage["model_turns"].as_u64().unwrap_or(0),
        tool_calls: report["tool_calls"].as_u64().unwrap_or(0),
        tool_failures: report["tool_failures"].as_u64().unwrap_or(0),
        total_tokens: report["total_usage"]["total_tokens"].as_u64().unwrap_or(0),
        wall_time_ms: usage["wall_time_ms"].as_u64().unwrap_or(0),
        quality_passed,
        safety_passed,
        hard_gate_failures: results
            .iter()
            .filter(|result| result.hard && !result.passed)
            .count() as u64,
        cost_microunits: usage["cost_microunits"].as_u64().unwrap_or(0),
        resumed,
    }
}

fn bounded_fixture_path(suite_root: &Path, relative: &str) -> std::io::Result<PathBuf> {
    let root = suite_root.canonicalize()?;
    let candidate = root.join(relative);
    let canonical = candidate.canonicalize().map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("fixture path is unavailable: {relative}"),
        )
    })?;
    if !canonical.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fixture path escapes suite root",
        ));
    }
    Ok(canonical)
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn git_output<const N: usize>(args: [&str; N]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn render_summary(manifest: &V2EvidenceManifest, cases: &[V2CaseReport]) -> String {
    let mut output = format!(
        "# Benchmark V2: {}\n\n- Schema: {}\n- Cases: {}\n- Network: disabled\n- Package hash: `{}`\n\n| Case | Result | Safety | Quality | Hard failures |\n|---|---|---|---|---|\n",
        manifest.suite, manifest.schema_version, manifest.case_count, manifest.package_hash,
    );
    for case in cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            case.case_id,
            if case.passed { "PASS" } else { "FAIL" },
            if case.metrics.safety_passed {
                "PASS"
            } else {
                "FAIL"
            },
            if case.metrics.quality_passed {
                "PASS"
            } else {
                "FAIL"
            },
            case.metrics.hard_gate_failures,
        ));
    }
    output
}
