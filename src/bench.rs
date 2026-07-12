use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::core::context::ContextManager;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::events::StreamEvent;
use crate::core::types::{ApprovalPolicy, JobId, RunId, SessionId, TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::core::workspace::WorkspaceKind;
use crate::models::fake::{FakeModelClient, FakeTurn};
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::{RunHandle, StateStore};
use crate::tools::{default_tool_registry_with_shell_policy, shell::ShellPolicy};

// ─── Schema types ────────────────────────────────────────────────────────────

/// A benchmark suite definition, either loaded from JSON or generated in code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<BenchmarkTask>,
}

/// Resume state from a benchmark JSON (subset of TaskState fields).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchmarkResumeState {
    #[serde(default)]
    pub goal: String,
    /// Accept either a bare string (`"summary": "..."`) or an optional field.
    #[serde(default, deserialize_with = "deserialize_optional_stringish")]
    pub summary: Option<String>,
}

fn deserialize_optional_stringish<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s),
        Some(other) => Some(other.to_string()),
    })
}

/// A single benchmark task within a suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub name: String,
    pub message: String,
    #[serde(default)]
    pub setup_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub turns: Vec<BenchmarkTurn>,
    #[serde(default)]
    pub max_steps: u32,
    #[serde(default)]
    pub checks: Vec<BenchmarkCheck>,
    #[serde(default)]
    pub expected_output_contains: Vec<String>,
    #[serde(default)]
    pub expected_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub expected_summary_contains: Vec<String>,
    #[serde(default)]
    pub requires_network: Option<bool>,
    #[serde(default)]
    pub resume_state: Option<BenchmarkResumeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BenchmarkTurn {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolBatch {
        calls: Vec<BenchmarkToolCall>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// A validation check run after task completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchmarkCheck {
    /// Assert that a file or directory exists in the workspace.
    FileExists {
        path: String,
        #[serde(default)]
        description: String,
    },
    /// Assert that a file exists and its content contains a substring.
    FileContentContains {
        path: String,
        substring: String,
        #[serde(default)]
        description: String,
    },
    /// Assert that the trace JSONL contains at least one event of the given type.
    TraceHasEvent {
        event_type: String,
        #[serde(default)]
        description: String,
    },
    /// Run a local shell command and assert exit code 0 (and optionally stdout contains).
    CommandOracle {
        command: String,
        #[serde(default)]
        workdir: Option<String>,
        #[serde(default)]
        expected_stdout_contains: Option<String>,
        #[serde(default)]
        description: String,
    },
}

// ─── Result types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub suite: String,
    pub profile: String,
    pub passed: bool,
    pub started_at: String,
    pub finished_at: String,
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub failed_tasks: usize,
    pub tasks: Vec<BenchmarkTaskReport>,
    pub evidence_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTaskReport {
    pub name: String,
    pub outcome: BenchmarkOutcome,
    pub termination_reason: String,
    pub steps: u32,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub artifacts: BenchmarkArtifacts,
    pub output: Option<String>,
    pub check_results: Vec<CheckResult>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkOutcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArtifacts {
    pub run_dir: PathBuf,
    pub trace_jsonl: PathBuf,
    pub task_state_json: PathBuf,
    pub report_json: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: String,
    pub description: String,
    pub passed: bool,
    pub detail: String,
}

// ─── Profile parameters for scalable suites ─────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProfileParams {
    pub task_count: usize,
    pub input_files_per_task: usize,
    pub max_steps: u32,
    pub include_failure_recovery: bool,
}

pub fn default_profile_params() -> ProfileParams {
    ProfileParams {
        task_count: 4,
        input_files_per_task: 2,
        max_steps: 20,
        include_failure_recovery: true,
    }
}

pub fn stress_profile_params() -> ProfileParams {
    ProfileParams {
        task_count: 14,
        input_files_per_task: 3,
        max_steps: 30,
        include_failure_recovery: true,
    }
}

// ─── Cross-platform command helpers ─────────────────────────────────────────

/// Returns a shell echo command that works in both sh and PowerShell.
fn shell_echo(msg: &str) -> String {
    format!("echo {}", msg)
}

// ─── JSON loading ────────────────────────────────────────────────────────────

pub async fn load_benchmark_suite(path: impl AsRef<Path>) -> std::io::Result<BenchmarkSuite> {
    let bytes = tokio::fs::read(path).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

// ─── Built-in dataprep suite generator ──────────────────────────────────────

/// Generate the "dataprep" suite for a given profile.
///
/// This suite simulates a data preparation pipeline:
/// Phase 1: Read input data files
/// Phase 2: Generate intermediate summary files
/// Phase 3: Encounter and recover from a failure
/// Phase 4: Aggregate results via shell and write final report
pub fn generate_dataprep_suite(params: &ProfileParams) -> BenchmarkSuite {
    let mut tasks = Vec::new();
    let datasets = vec!["sales", "users", "events", "logs", "metrics", "inventory"];

    for i in 0..params.task_count {
        let dataset = datasets[i % datasets.len()];
        let task = build_dataprep_task(i, dataset, params);
        tasks.push(task);
    }

    BenchmarkSuite {
        name: "dataprep".to_string(),
        description: "Data preparation pipeline: read inputs, write intermediates, recover from failure, produce final report.".to_string(),
        tasks,
    }
}

fn build_dataprep_task(idx: usize, dataset: &str, params: &ProfileParams) -> BenchmarkTask {
    let task_name = format!("dataprep_{:03}_{}", idx, dataset);
    let num_inputs = params.input_files_per_task;

    // Build setup files (input data)
    let mut setup_files = Vec::new();
    for j in 0..num_inputs {
        let filename = format!("inputs/{}-part{}.csv", dataset, j);
        let header = format!("id,{},value,timestamp\n", dataset);
        let mut rows = String::new();
        for r in 0..5 {
            rows.push_str(&format!(
                "{},{}_{},{},2024-01-{:02}T10:00:00Z\n",
                r,
                dataset,
                j,
                r * 10 + j,
                r + 1
            ));
        }
        setup_files.push(BenchmarkFile {
            path: filename,
            content: header + &rows,
        });
    }
    // Add a JSON manifest
    setup_files.push(BenchmarkFile {
        path: format!("inputs/{}-manifest.json", dataset),
        content: serde_json::to_string_pretty(&serde_json::json!({
            "dataset": dataset,
            "parts": num_inputs,
            "format": "csv",
            "version": "1.0"
        }))
        .unwrap(),
    });

    // Build turns simulating the agent's actions
    let mut turns = Vec::new();

    // Phase 1: Read inputs (read first file)
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{}_read0", idx),
        name: "fs_read".to_string(),
        args: serde_json::json!({ "path": format!("inputs/{}-part0.csv", dataset) }),
    });

    // Phase 2: Write intermediate summary
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{}_write_summary", idx),
        name: "fs_write".to_string(),
        args: serde_json::json!({
            "path": format!("intermediates/{}-summary.json", dataset),
            "content": serde_json::json!({
                "dataset": dataset,
                "rows_processed": 5 * num_inputs,
                "columns": ["id", dataset, "value", "timestamp"],
                "status": "summarized"
            }).to_string()
        }),
    });

    // Phase 3: Attempt to read a file that doesn't exist (recoverable failure)
    if params.include_failure_recovery {
        turns.push(BenchmarkTurn::ToolUse {
            id: format!("call_{}_fail_read", idx),
            name: "fs_read".to_string(),
            args: serde_json::json!({ "path": format!("inputs/{}-MISSING.csv", dataset) }),
        });
        // After failure, agent reads the manifest to understand the structure
        turns.push(BenchmarkTurn::ToolUse {
            id: format!("call_{}_read_manifest", idx),
            name: "fs_read".to_string(),
            args: serde_json::json!({ "path": format!("inputs/{}-manifest.json", dataset) }),
        });
    }

    // Phase 4: Use shell to run a cross-platform echo (aggregate phase marker)
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{}_shell", idx),
        name: "shell".to_string(),
        args: serde_json::json!({ "command": shell_echo(&format!("aggregate_phase_{}", dataset)) }),
    });

    // Phase 4: Write final report artifact
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{}_write_report", idx),
        name: "fs_write".to_string(),
        args: serde_json::json!({
            "path": format!("outputs/{}-final-report.json", dataset),
            "content": serde_json::json!({
                "dataset": dataset,
                "task_index": idx,
                "phases_completed": ["read", "summarize", "recover", "aggregate"],
                "total_input_files": num_inputs + 1,
                "intermediates_generated": 1,
                "failures_encountered": if params.include_failure_recovery { 1 } else { 0 },
                "final_status": "complete",
                "artifacts": [
                    format!("intermediates/{}-summary.json", dataset),
                    format!("outputs/{}-final-report.json", dataset)
                ]
            }).to_string()
        }),
    });

    // Write an evidence file
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{}_write_evidence", idx),
        name: "fs_write".to_string(),
        args: serde_json::json!({
            "path": format!("outputs/{}-evidence.md", dataset),
            "content": format!(
                "# Evidence for {}\n\n- Dataset: {}\n- Task index: {}\n- Input files read: {}\n- Failures recovered: {}\n- Status: PASS\n",
                dataset,
                dataset,
                idx,
                num_inputs,
                if params.include_failure_recovery { 1 } else { 0 }
            )
        }),
    });

    // Final text response
    turns.push(BenchmarkTurn::Text {
        text: format!(
            "Data preparation for {} complete. Processed {} input files, generated summary and final report. Encountered and recovered from 1 missing file error. Artifacts in outputs/ directory.",
            dataset,
            num_inputs + 1
        ),
    });

    // Build checks
    let mut checks = Vec::new();

    // Check 1: File exists - intermediate summary
    checks.push(BenchmarkCheck::FileExists {
        path: format!("intermediates/{}-summary.json", dataset),
        description: "Intermediate summary file exists".to_string(),
    });

    // Check 2: File content contains - final report has "complete" status
    checks.push(BenchmarkCheck::FileContentContains {
        path: format!("outputs/{}-final-report.json", dataset),
        substring: "\"final_status\":\"complete\"".to_string(),
        description: "Final report contains complete status".to_string(),
    });

    // Check 3: Trace has a tool_call_failed event (from the missing file)
    if params.include_failure_recovery {
        checks.push(BenchmarkCheck::TraceHasEvent {
            event_type: "tool_call_failed".to_string(),
            description: "Trace records the recoverable tool failure".to_string(),
        });
    }

    // Check 4: Trace has run_completed event
    checks.push(BenchmarkCheck::TraceHasEvent {
        event_type: "run_completed".to_string(),
        description: "Trace records run completion".to_string(),
    });

    // Check 5: Command oracle - run cross-platform echo and verify stdout
    checks.push(BenchmarkCheck::CommandOracle {
        command: shell_echo("oracle_verification_passed"),
        workdir: None,
        expected_stdout_contains: Some("oracle_verification_passed".to_string()),
        description: "Shell command executes and produces expected stdout".to_string(),
    });

    // Check 6: File exists - evidence markdown
    checks.push(BenchmarkCheck::FileExists {
        path: format!("outputs/{}-evidence.md", dataset),
        description: "Evidence markdown file exists".to_string(),
    });

    BenchmarkTask {
        name: task_name,
        message: format!(
            "Process the {} dataset: read inputs from inputs/, create intermediate summaries, handle any errors gracefully, and produce a final report in outputs/.",
            dataset
        ),
        setup_files,
        turns,
        max_steps: params.max_steps,
        checks,
        expected_output_contains: vec![dataset.to_string(), "complete".to_string()],
        expected_files: vec![BenchmarkFile {
            path: format!("outputs/{}-final-report.json", dataset),
            content: String::new(),
        }],
        expected_summary_contains: Vec::new(),
        requires_network: Some(false),
        resume_state: None,
    }
}

// ─── Runner ──────────────────────────────────────────────────────────────────

pub async fn run_benchmark_suite(
    suite: &BenchmarkSuite,
    output_root: impl AsRef<Path>,
    profile: &str,
) -> std::io::Result<BenchmarkReport> {
    let output_root = output_root.as_ref();
    let started_at = chrono::Utc::now().to_rfc3339();
    let run_stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let evidence_root = output_root.join(format!(
        "{}-{}-{}",
        run_stamp,
        sanitize_path_component(&suite.name),
        sanitize_path_component(profile)
    ));
    tokio::fs::create_dir_all(&evidence_root).await?;

    let mut tasks = Vec::new();
    for task in &suite.tasks {
        let task_report = run_benchmark_task(task, &evidence_root).await?;
        tasks.push(task_report);
    }

    let finished_at = chrono::Utc::now().to_rfc3339();
    let passed_count = tasks
        .iter()
        .filter(|t| t.outcome == BenchmarkOutcome::Passed)
        .count();
    let failed_count = tasks.len() - passed_count;
    let passed = failed_count == 0;

    let report = BenchmarkReport {
        suite: suite.name.clone(),
        profile: profile.to_string(),
        passed,
        started_at: started_at.clone(),
        finished_at: finished_at.clone(),
        total_tasks: tasks.len(),
        passed_tasks: passed_count,
        failed_tasks: failed_count,
        tasks,
        evidence_root: evidence_root.clone(),
    };

    // Write machine-readable metrics
    let metrics_path = evidence_root.join("metrics.json");
    tokio::fs::write(
        &metrics_path,
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .await?;

    // Write human-readable summary
    let summary_md = render_summary_md(&report);
    tokio::fs::write(evidence_root.join("summary.md"), &summary_md).await?;

    Ok(report)
}

async fn run_benchmark_task(
    task: &BenchmarkTask,
    evidence_root: &Path,
) -> std::io::Result<BenchmarkTaskReport> {
    let task_dir = evidence_root.join("tasks").join(&task.name);
    let workspace_root = task_dir.join("workspace");
    let state_dir = task_dir.join(".rove");
    tokio::fs::create_dir_all(&workspace_root).await?;
    tokio::fs::create_dir_all(&state_dir).await?;
    write_files(&workspace_root, &task.setup_files).await?;

    let workspace = Workspace {
        root: workspace_root.clone(),
        kind: WorkspaceKind::Task,
        state_dir: state_dir.clone(),
    };
    let state_store = StateStore::new(&state_dir);
    state_store
        .index
        .initialize()
        .map_err(std::io::Error::other)?;

    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();

    // Build resume_state if the task specifies one
    let resume_task_state = task.resume_state.as_ref().map(|rs| TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: rs.goal.clone(),
        step: 0,
        history: Vec::new(),
        summary: rs.summary.clone(),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
    });

    let run = state_store.start_run(session_id, job_id, run_id)?;
    let artifacts = artifacts_for_run(&run);

    let max_steps = if task.max_steps > 0 {
        task.max_steps
    } else {
        20
    };
    let engine = benchmark_engine(&workspace, task, max_steps);
    let (reason, output, steps, tool_calls, tool_failures) = run_engine_collect_output(
        &engine,
        &state_store,
        run,
        task.message.clone(),
        resume_task_state,
    )
    .await;

    // Run all checks
    let mut check_results = Vec::new();
    let mut failures = Vec::new();

    // Legacy checks (expected_output_contains)
    for expected in &task.expected_output_contains {
        if !output.as_deref().unwrap_or_default().contains(expected) {
            failures.push(format!("output did not contain {:?}", expected));
        }
    }

    // Legacy expected_files (existence check only, content via new checks)
    for expected in &task.expected_files {
        let path = workspace_root.join(&expected.path);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            failures.push(format!("expected file missing: {}", expected.path));
        }
    }

    // Legacy expected_summary_contains (check task_state.json)
    if !task.expected_summary_contains.is_empty() {
        match tokio::fs::read_to_string(&artifacts.task_state_json).await {
            Ok(state_content) => {
                for expected in &task.expected_summary_contains {
                    if !state_content.contains(expected) {
                        failures.push(format!(
                            "task_state did not contain expected summary {:?}",
                            expected
                        ));
                    }
                }
            }
            Err(e) => {
                failures.push(format!(
                    "failed to read task_state for summary check: {}",
                    e
                ));
            }
        }
    }

    // Core artifact existence checks
    for artifact in [
        &artifacts.trace_jsonl,
        &artifacts.task_state_json,
        &artifacts.report_json,
    ] {
        if !tokio::fs::try_exists(artifact).await.unwrap_or(false) {
            failures.push(format!("artifact missing: {}", artifact.display()));
        }
    }

    if reason != TerminationReason::Final {
        failures.push(format!("expected final termination, got {:?}", reason));
    }

    // Run new typed checks
    for check in &task.checks {
        let result = run_check(check, &workspace_root, &artifacts).await;
        if !result.passed {
            failures.push(format!(
                "[{}] {}: {}",
                result.kind, result.description, result.detail
            ));
        }
        check_results.push(result);
    }

    Ok(BenchmarkTaskReport {
        name: task.name.clone(),
        outcome: if failures.is_empty() {
            BenchmarkOutcome::Passed
        } else {
            BenchmarkOutcome::Failed
        },
        termination_reason: format!("{:?}", reason).to_lowercase(),
        steps,
        tool_calls,
        tool_failures,
        artifacts,
        output,
        check_results,
        failures,
    })
}

async fn run_check(
    check: &BenchmarkCheck,
    workspace_root: &Path,
    artifacts: &BenchmarkArtifacts,
) -> CheckResult {
    match check {
        BenchmarkCheck::FileExists { path, description } => {
            let full = workspace_root.join(path);
            let exists = tokio::fs::try_exists(&full).await.unwrap_or(false);
            CheckResult {
                kind: "file_exists".to_string(),
                description: description.clone(),
                passed: exists,
                detail: if exists {
                    format!("{} exists", path)
                } else {
                    format!("{} does not exist", path)
                },
            }
        }
        BenchmarkCheck::FileContentContains {
            path,
            substring,
            description,
        } => {
            let full = workspace_root.join(path);
            match tokio::fs::read_to_string(&full).await {
                Ok(content) => {
                    let contains = content.contains(substring);
                    CheckResult {
                        kind: "file_content_contains".to_string(),
                        description: description.clone(),
                        passed: contains,
                        detail: if contains {
                            format!("{} contains expected substring", path)
                        } else {
                            format!("{} does not contain {:?}", path, substring)
                        },
                    }
                }
                Err(e) => CheckResult {
                    kind: "file_content_contains".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to read {}: {}", path, e),
                },
            }
        }
        BenchmarkCheck::TraceHasEvent {
            event_type,
            description,
        } => {
            match tokio::fs::read_to_string(&artifacts.trace_jsonl).await {
                Ok(content) => {
                    // StreamEvent uses #[serde(tag = "type", rename_all = "snake_case")]
                    let needle = format!("\"type\":\"{}\"", event_type);
                    let found = content.lines().any(|line| line.contains(&needle));
                    CheckResult {
                        kind: "trace_has_event".to_string(),
                        description: description.clone(),
                        passed: found,
                        detail: if found {
                            format!("trace contains {} event", event_type)
                        } else {
                            format!("trace does not contain {} event", event_type)
                        },
                    }
                }
                Err(e) => CheckResult {
                    kind: "trace_has_event".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to read trace: {}", e),
                },
            }
        }
        BenchmarkCheck::CommandOracle {
            command,
            workdir,
            expected_stdout_contains,
            description,
        } => {
            let exec_dir = if let Some(wd) = workdir {
                workspace_root.join(wd)
            } else {
                workspace_root.to_path_buf()
            };
            // Match the shell tool's platform behavior:
            // Unix: sh -lc <command>
            // Windows: powershell -NoProfile -NonInteractive -Command <command>
            #[cfg(target_os = "windows")]
            let output = SysCommand::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", command])
                .current_dir(&exec_dir)
                .output();
            #[cfg(not(target_os = "windows"))]
            let output = SysCommand::new("sh")
                .args(["-lc", command])
                .current_dir(&exec_dir)
                .output();

            match output {
                Ok(result) => {
                    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
                    let exit_ok = result.status.success();
                    let stdout_ok = match expected_stdout_contains {
                        Some(expected) => stdout.contains(expected),
                        None => true,
                    };
                    let passed = exit_ok && stdout_ok;
                    let mut detail_parts = Vec::new();
                    if !exit_ok {
                        detail_parts.push(format!("exit code {:?}", result.status.code()));
                    }
                    if !stdout_ok {
                        detail_parts.push("stdout did not contain expected text".to_string());
                    }
                    if !stderr.is_empty() {
                        detail_parts.push(format!("stderr: {}", stderr.trim()));
                    }
                    if passed {
                        detail_parts.push(format!("command succeeded: {}", stdout.trim()));
                    }
                    CheckResult {
                        kind: "command_oracle".to_string(),
                        description: description.clone(),
                        passed,
                        detail: detail_parts.join("; "),
                    }
                }
                Err(e) => CheckResult {
                    kind: "command_oracle".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to execute command: {}", e),
                },
            }
        }
    }
}

// ─── Engine setup ────────────────────────────────────────────────────────────

fn benchmark_engine(workspace: &Workspace, task: &BenchmarkTask, max_steps: u32) -> Engine {
    let model = Box::new(FakeModelClient::with_turns(
        "benchmark fallback response".to_string(),
        task.turns.iter().cloned().map(Into::into).collect(),
    ));
    // Allow shell commands with no denylist for benchmark purposes
    let policy = ShellPolicy {
        timeout_ms: 10_000,
        max_output_bytes: 64 * 1024,
        inherit_environment: true,
        denylist: Vec::new(),
    };
    let registry = default_tool_registry_with_shell_policy(workspace, policy);
    Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are the rove benchmark runner.".to_string()),
        EngineConfig {
            max_steps,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
}

async fn run_engine_collect_output(
    engine: &Engine,
    state_store: &StateStore,
    run: RunHandle,
    message: String,
    resume_state: Option<TaskState>,
) -> (TerminationReason, Option<String>, u32, u32, u32) {
    let RunHandle {
        session_id,
        job_id,
        run_id,
        run_dir,
        trace_writer,
    } = run;

    let req = RunRequest {
        session_id,
        job_id,
        run_id,
        user_message: message,
        resume_state,
    };

    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        req.user_message.clone(),
        req.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    let mut stream =
        std::pin::pin!(engine.run_with_cancel(req, Some(trace_writer), CancellationToken::new()));
    let mut reason = TerminationReason::Error;
    let mut output = None;
    let mut steps = 0u32;
    let mut tool_calls = 0u32;
    let mut tool_failures = 0u32;

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        match &event {
            StreamEvent::LlmMessage { .. } => steps += 1,
            StreamEvent::ToolCallStarted { .. } => tool_calls += 1,
            StreamEvent::ToolCallFailed { .. } => tool_failures += 1,
            StreamEvent::RunCompleted {
                reason: event_reason,
                output: event_output,
            } => {
                reason = event_reason.clone();
                output = event_output.clone();
                break;
            }
            _ => {}
        }
    }
    recorder
        .finalize(state_store, engine.workspace(), engine.model_id(), &run_dir)
        .await;

    (reason, output, steps, tool_calls, tool_failures)
}

use crate::core::types::RunRequest;

fn artifacts_for_run(run: &RunHandle) -> BenchmarkArtifacts {
    BenchmarkArtifacts {
        run_dir: run.run_dir.clone(),
        trace_jsonl: run.run_dir.join("trace.jsonl"),
        task_state_json: run.run_dir.join("task_state.json"),
        report_json: run.run_dir.join("report.json"),
    }
}

async fn write_files(root: &Path, files: &[BenchmarkFile]) -> std::io::Result<()> {
    for file in files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, &file.content).await?;
    }
    Ok(())
}

// ─── Summary rendering ──────────────────────────────────────────────────────

fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "run".to_string()
    } else {
        out
    }
}

fn render_summary_md(report: &BenchmarkReport) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Benchmark Run: {}\n\n", report.suite));
    md.push_str(&format!("- **Profile**: {}\n", report.profile));
    md.push_str(&format!("- **Started**: {}\n", report.started_at));
    md.push_str(&format!("- **Finished**: {}\n", report.finished_at));
    md.push_str(&format!(
        "- **Result**: {} / {} passed\n\n",
        report.passed_tasks, report.total_tasks
    ));
    md.push_str("## Tasks\n\n");
    md.push_str("| Task | Outcome | Steps | Tool Calls | Failures | Checks Passed |\n");
    md.push_str("|------|---------|-------|------------|----------|---------------|\n");
    for task in &report.tasks {
        let checks_passed = task.check_results.iter().filter(|c| c.passed).count();
        let checks_total = task.check_results.len();
        let outcome = match task.outcome {
            BenchmarkOutcome::Passed => "PASS",
            BenchmarkOutcome::Failed => "FAIL",
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {}/{} |\n",
            task.name,
            outcome,
            task.steps,
            task.tool_calls,
            task.tool_failures,
            checks_passed,
            checks_total
        ));
    }
    md.push_str("\n## Failed Checks\n\n");
    let mut any_failures = false;
    for task in &report.tasks {
        for check in &task.check_results {
            if !check.passed {
                any_failures = true;
                md.push_str(&format!(
                    "- **{}** [{}]: {}\n",
                    task.name, check.kind, check.detail
                ));
            }
        }
        for failure in &task.failures {
            any_failures = true;
            md.push_str(&format!("- **{}**: {}\n", task.name, failure));
        }
    }
    if !any_failures {
        md.push_str("All checks passed.\n");
    }
    md.push_str(&format!(
        "\n## Evidence Location\n\n`{}`\n",
        report.evidence_root.display()
    ));
    md
}

// ─── Suite registry ─────────────────────────────────────────────────────────

pub fn available_suites() -> Vec<SuiteInfo> {
    vec![
        SuiteInfo {
            name: "dataprep".to_string(),
            description: "Data preparation pipeline with multi-phase tasks, failure recovery, and rich artifacts.".to_string(),
            profiles: vec!["default".to_string(), "stress".to_string()],
        },
        SuiteInfo {
            name: "agent-smoke".to_string(),
            description: "Legacy smoke test suite (loaded from benchmarks/agent-smoke.json).".to_string(),
            profiles: vec!["default".to_string()],
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteInfo {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
}

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
            let path = PathBuf::from("benchmarks/agent-smoke.json");
            let bytes = std::fs::read(&path)?;
            let suite: BenchmarkSuite =
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            Ok(suite)
        }
        other => {
            let path = PathBuf::from(format!("benchmarks/{}.json", other));
            if path.exists() {
                let bytes = std::fs::read(&path)?;
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("unknown suite: {}", other),
                ))
            }
        }
    }
}

// ─── Conversions ─────────────────────────────────────────────────────────────

impl From<BenchmarkTurn> for FakeTurn {
    fn from(turn: BenchmarkTurn) -> Self {
        match turn {
            BenchmarkTurn::Text { text } => FakeTurn::Text(text),
            BenchmarkTurn::ToolUse { id, name, args } => FakeTurn::ToolUse { id, name, args },
            BenchmarkTurn::ToolBatch { calls } => FakeTurn::ToolBatch(
                calls
                    .into_iter()
                    .map(|call| (call.id, call.name, call.args))
                    .collect(),
            ),
        }
    }
}
