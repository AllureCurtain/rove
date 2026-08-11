use std::path::Path;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use rove_app_bootstrap::tool_registry_with_shell_policy;
use rove_models::fake::{FakeModelClient, FakeTurn};
use rove_runtime::agents::validation::OperatorConstraints;
use rove_runtime::agents::{AgentActivationConfig, AgentSelector};
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig};
use rove_runtime::events::StreamEvent;
use rove_runtime::execution::ExecutionPolicy;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::store::{RunHandle, StateStore};
use rove_runtime::tools::shell::ShellPolicy;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, JobId, RunId, RunRequest, SessionId, TaskState,
    TerminationReason,
};
use rove_runtime::workspace::boundary::resolve_workspace_write_path;
use rove_runtime::workspace::{Workspace, WorkspaceKind};

use super::checks::run_check;
use super::evidence::{render_summary_md, sanitize_path_component};
use super::schema::{
    BenchmarkArtifacts, BenchmarkFile, BenchmarkOutcome, BenchmarkReport, BenchmarkResumeState,
    BenchmarkSuite, BenchmarkTask, BenchmarkTaskReport, BenchmarkTurn,
};

pub async fn load_benchmark_suite(path: impl AsRef<Path>) -> std::io::Result<BenchmarkSuite> {
    let bytes = tokio::fs::read(path).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

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
        tasks.push(
            run_benchmark_task_with_options(task, &evidence_root, &BenchmarkRunOptions::default())
                .await?,
        );
    }

    let finished_at = chrono::Utc::now().to_rfc3339();
    let passed_count = tasks
        .iter()
        .filter(|t| t.outcome == BenchmarkOutcome::Passed)
        .count();
    let failed_count = tasks.len() - passed_count;

    let report = BenchmarkReport {
        suite: suite.name.clone(),
        profile: profile.to_string(),
        passed: failed_count == 0,
        started_at,
        finished_at,
        total_tasks: tasks.len(),
        passed_tasks: passed_count,
        failed_tasks: failed_count,
        tasks,
        evidence_root: evidence_root.clone(),
    };

    tokio::fs::write(
        evidence_root.join("metrics.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .await?;
    tokio::fs::write(evidence_root.join("summary.md"), render_summary_md(&report)).await?;

    Ok(report)
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkRunOptions {
    pub agent_selector: Option<AgentSelector>,
    pub workspace_agent_authorized: bool,
    pub load_workspace_instructions: bool,
    pub allow_remediation_procedures: bool,
    pub max_procedure_selections: Option<u32>,
    pub execution_policy: Option<ExecutionPolicy>,
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
}

impl Default for BenchmarkRunOptions {
    fn default() -> Self {
        Self {
            agent_selector: None,
            workspace_agent_authorized: false,
            load_workspace_instructions: false,
            allow_remediation_procedures: false,
            max_procedure_selections: None,
            execution_policy: None,
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Reject,
        }
    }
}

pub(crate) async fn run_benchmark_task_with_options(
    task: &BenchmarkTask,
    evidence_root: &Path,
    options: &BenchmarkRunOptions,
) -> std::io::Result<BenchmarkTaskReport> {
    let task_dir = evidence_root
        .join("tasks")
        .join(sanitize_path_component(&task.name));
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

    let max_steps = if task.max_steps > 0 {
        task.max_steps
    } else {
        20
    };

    let (reason, output, steps, tool_calls, tool_failures, artifacts, resumed) =
        if let Some(cancel_after) = task.cancel_resume_after_turns {
            Box::pin(run_task_with_cancel_resume(
                task,
                &workspace,
                &state_dir,
                max_steps,
                cancel_after,
                options,
            ))
            .await?
        } else {
            let state_store = StateStore::new(&state_dir);
            state_store
                .index
                .initialize()
                .map_err(std::io::Error::other)?;

            let session_id = SessionId::new();
            let job_id = JobId::new();
            let run_id = RunId::new();
            let resume_task_state = task
                .resume_state
                .as_ref()
                .map(|rs| build_resume_state(session_id, job_id, run_id, rs));
            let run = state_store.start_run(session_id, job_id, run_id)?;
            let artifacts = artifacts_for_run(&run);
            let engine = benchmark_engine(
                &workspace,
                task.turns.iter().cloned().map(Into::into).collect(),
                max_steps,
                options,
            )?;
            let (reason, output, steps, tool_calls, tool_failures) =
                Box::pin(run_engine_collect_output(
                    &engine,
                    &state_store,
                    run,
                    task.message.clone(),
                    resume_task_state,
                    CancellationToken::new(),
                ))
                .await;
            (
                reason,
                output,
                steps,
                tool_calls,
                tool_failures,
                artifacts,
                false,
            )
        };

    let mut check_results = Vec::new();
    let mut failures = Vec::new();

    for expected in &task.expected_output_contains {
        if !output.as_deref().unwrap_or_default().contains(expected) {
            failures.push(format!("output did not contain {expected:?}"));
        }
    }
    for expected in &task.expected_files {
        let path = workspace_root.join(&expected.path);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            failures.push(format!("expected file missing: {}", expected.path));
        } else if !expected.content.is_empty() {
            match tokio::fs::read_to_string(&path).await {
                Ok(actual) if actual == expected.content => {}
                Ok(_) => {
                    failures.push(format!("expected file content mismatch: {}", expected.path))
                }
                Err(error) => failures.push(format!(
                    "failed to read expected file {}: {error}",
                    expected.path
                )),
            }
        }
    }
    if !task.expected_summary_contains.is_empty() {
        match tokio::fs::read_to_string(&artifacts.task_state_json).await {
            Ok(state_content) => {
                for expected in &task.expected_summary_contains {
                    if !state_content.contains(expected) {
                        failures.push(format!(
                            "task_state did not contain expected summary {expected:?}"
                        ));
                    }
                }
            }
            Err(e) => failures.push(format!("failed to read task_state for summary check: {e}")),
        }
    }
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
        failures.push(format!("expected final termination, got {reason:?}"));
    }

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
        termination_reason: format!("{reason:?}").to_lowercase(),
        steps,
        tool_calls,
        tool_failures,
        artifacts,
        output,
        check_results,
        failures,
        resumed,
    })
}

fn build_resume_state(
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    rs: &BenchmarkResumeState,
) -> TaskState {
    TaskState {
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
        agent_profile: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    }
}

async fn run_task_with_cancel_resume(
    task: &BenchmarkTask,
    workspace: &Workspace,
    state_dir: &Path,
    max_steps: u32,
    cancel_after_turns: usize,
    options: &BenchmarkRunOptions,
) -> std::io::Result<(
    TerminationReason,
    Option<String>,
    u32,
    u32,
    u32,
    BenchmarkArtifacts,
    bool,
)> {
    let all_turns: Vec<FakeTurn> = task.turns.iter().cloned().map(Into::into).collect();
    let mut split_at = cancel_after_turns
        .max(1)
        .min(all_turns.len().saturating_sub(1));
    while split_at > 1 {
        if matches!(all_turns[split_at - 1], FakeTurn::Text(_)) {
            split_at -= 1;
        } else {
            break;
        }
    }
    let turns_1: Vec<FakeTurn> = all_turns.iter().take(split_at).cloned().collect();
    let turns_2: Vec<FakeTurn> = all_turns.iter().skip(split_at).cloned().collect();

    let cancel_token = CancellationToken::new();
    let state_store = StateStore::new(state_dir);
    state_store
        .index
        .initialize()
        .map_err(std::io::Error::other)?;
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id_1 = RunId::new();
    let run1 = state_store.start_run(session_id, job_id, run_id_1)?;
    let run_dir_1 = run1.run_dir.clone();

    let engine1 = benchmark_engine(workspace, turns_1, max_steps, options)?;
    let req1 = run1.request(task.message.clone(), None);
    let RunHandle {
        session_id: s1,
        job_id: j1,
        run_id: r1,
        run_dir: _,
        trace_writer: tw1,
    } = &run1;
    let mut rec1 = RunArtifactRecorder::new(
        *s1,
        *j1,
        *r1,
        task.message.clone(),
        None,
        Some(engine1.runtime_identity()),
    );
    let mut stream1 =
        std::pin::pin!(engine1.run_with_cancel(req1, Some(tw1.clone()), cancel_token.clone()));
    rec1.set_runtime_identity(stream1.runtime_identity().clone());
    rec1.set_agent_profile(stream1.agent_profile().cloned());
    let mut llm_count = 0u32;
    let mut run1_tc = 0u32;
    let mut run1_tf = 0u32;
    let mut cancel_after_tool = false;
    while let Some(event) = stream1.next().await {
        rec1.record_event(&event, &state_store).await;
        match &event {
            StreamEvent::LlmMessage { tool_calls, .. } => {
                llm_count += 1;
                if llm_count >= split_at as u32 {
                    if tool_calls.is_empty() {
                        cancel_token.cancel();
                    } else {
                        cancel_after_tool = true;
                    }
                }
            }
            StreamEvent::ToolCallStarted { .. } => {
                run1_tc += 1;
            }
            StreamEvent::ToolCallFailed { .. } if cancel_after_tool => {
                run1_tf += 1;
                cancel_token.cancel();
            }
            StreamEvent::ToolCallFailed { .. } => {
                run1_tf += 1;
            }
            StreamEvent::ToolCallCompleted { .. } if cancel_after_tool => {
                cancel_token.cancel();
            }
            _ => {}
        }
        if matches!(event, StreamEvent::RunCompleted { .. }) {
            break;
        }
    }
    rec1.finalize(&state_store, workspace, engine1.model_id(), &run_dir_1)
        .await;

    let saved = match state_store.load_task_state(run_id_1).await {
        Ok(state) => state,
        Err(err) => {
            tracing::warn!("cancel-resume checkpoint load failed: {err}; falling back");
            let run = state_store.start_run(session_id, job_id, RunId::new())?;
            let artifacts = artifacts_for_run(&run);
            let engine = benchmark_engine(workspace, all_turns, max_steps, options)?;
            let (reason, output, steps, tool_calls, tool_failures) =
                Box::pin(run_engine_collect_output(
                    &engine,
                    &state_store,
                    run,
                    task.message.clone(),
                    None,
                    CancellationToken::new(),
                ))
                .await;
            return Ok((
                reason,
                output,
                steps,
                tool_calls,
                tool_failures,
                artifacts,
                false,
            ));
        }
    };

    let run_id_2 = RunId::new();
    let run2 = state_store.start_run(session_id, job_id, run_id_2)?;
    let run_dir_2 = run2.run_dir.clone();
    let engine2 = benchmark_engine(workspace, turns_2, max_steps, options)?;
    let saved_for_recorder = saved.clone();
    let req2 = run2.request(task.message.clone(), Some(saved));
    let RunHandle {
        session_id: s2,
        job_id: j2,
        run_id: r2,
        run_dir: rd2,
        trace_writer: tw2,
    } = &run2;
    let mut rec2 = RunArtifactRecorder::new(
        *s2,
        *j2,
        *r2,
        task.message.clone(),
        Some(&saved_for_recorder),
        Some(engine2.runtime_identity()),
    );
    let mut stream2 =
        std::pin::pin!(engine2.run_with_cancel(req2, Some(tw2.clone()), CancellationToken::new()));
    rec2.set_runtime_identity(stream2.runtime_identity().clone());
    rec2.set_agent_profile(stream2.agent_profile().cloned());
    let mut reason = TerminationReason::Error;
    let mut output = None;
    let mut total_steps = 0u32;
    let mut total_tc = 0u32;
    let mut total_tf = 0u32;
    while let Some(event) = stream2.next().await {
        rec2.record_event(&event, &state_store).await;
        match &event {
            StreamEvent::LlmMessage { .. } => total_steps += 1,
            StreamEvent::ToolCallStarted { .. } => total_tc += 1,
            StreamEvent::ToolCallFailed { .. } => total_tf += 1,
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
    rec2.finalize(&state_store, workspace, engine2.model_id(), rd2)
        .await;
    total_steps += llm_count;
    total_tc += run1_tc;
    total_tf += run1_tf;
    let _ = merge_run_reports(&run_dir_1, &run_dir_2).await;

    Ok((
        reason,
        output,
        total_steps,
        total_tc,
        total_tf,
        artifacts_for_run_dir(&run_dir_2),
        true,
    ))
}

fn benchmark_engine(
    workspace: &Workspace,
    turns: Vec<FakeTurn>,
    max_steps: u32,
    options: &BenchmarkRunOptions,
) -> std::io::Result<Engine> {
    let model = Box::new(FakeModelClient::with_turns(
        "benchmark fallback response".to_string(),
        turns,
    ));
    let policy = ShellPolicy {
        timeout_ms: 10_000,
        max_output_bytes: 64 * 1024,
        inherit_environment: true,
        denylist: Vec::new(),
    };
    let registry = tool_registry_with_shell_policy(workspace, policy);
    let mut engine = Engine::with_workspace_and_approval_decision(
        model,
        registry,
        ContextManager::new("You are the rove benchmark runner.".to_string()),
        EngineConfig::new(max_steps, false),
        workspace.clone(),
        options.approval_policy,
        options.approval_decision,
    );
    if let Some(policy) = options.execution_policy.clone() {
        engine = engine
            .with_execution_policy(policy)
            .map_err(std::io::Error::other)?;
    }
    if let Some(selector) = options.agent_selector.clone() {
        engine = engine
            .with_agent_activation(AgentActivationConfig {
                selector,
                workspace_source_authorized: options.workspace_agent_authorized,
                load_workspace_instructions: options.load_workspace_instructions,
                allow_remediation_procedures: options.allow_remediation_procedures,
                constraints: OperatorConstraints {
                    max_steps_cap: Some(max_steps),
                    max_tool_calls_cap: options
                        .execution_policy
                        .as_ref()
                        .and_then(|policy| policy.budgets.max_tool_calls),
                    max_procedure_selections_cap: options.max_procedure_selections,
                    ..OperatorConstraints::unconstrained()
                },
                context_tokens: Some(32_000),
            })
            .map_err(std::io::Error::other)?;
    }
    Ok(engine)
}

async fn run_engine_collect_output(
    engine: &Engine,
    state_store: &StateStore,
    run: RunHandle,
    message: String,
    resume_state: Option<TaskState>,
    cancel_token: CancellationToken,
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
    let mut stream = std::pin::pin!(engine.run_with_cancel(req, Some(trace_writer), cancel_token));
    recorder.set_runtime_identity(stream.runtime_identity().clone());
    recorder.set_agent_profile(stream.agent_profile().cloned());
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

fn artifacts_for_run(run: &RunHandle) -> BenchmarkArtifacts {
    artifacts_for_run_dir(&run.run_dir)
}

fn artifacts_for_run_dir(run_dir: &Path) -> BenchmarkArtifacts {
    BenchmarkArtifacts {
        run_dir: run_dir.to_path_buf(),
        trace_jsonl: run_dir.join("trace.jsonl"),
        task_state_json: run_dir.join("task_state.json"),
        report_json: run_dir.join("report.json"),
    }
}

async fn write_files(root: &Path, files: &[BenchmarkFile]) -> std::io::Result<()> {
    for file in files {
        let path = resolve_workspace_write_path(root, &file.path).map_err(std::io::Error::other)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, &file.content).await?;
    }
    Ok(())
}

async fn merge_run_reports(run1_dir: &Path, run2_dir: &Path) -> std::io::Result<()> {
    let r1_path = run1_dir.join("report.json");
    let r2_path = run2_dir.join("report.json");
    let Ok(r1_bytes) = tokio::fs::read(&r1_path).await else {
        return Ok(());
    };
    let Ok(r2_bytes) = tokio::fs::read(&r2_path).await else {
        return Ok(());
    };
    let Ok(r1): Result<serde_json::Value, _> = serde_json::from_slice(&r1_bytes) else {
        return Ok(());
    };
    let Ok(mut r2): Result<serde_json::Value, _> = serde_json::from_slice(&r2_bytes) else {
        return Ok(());
    };

    for field in ["steps", "tool_calls", "tool_failures"] {
        let v1 = r1.get(field).and_then(|v| v.as_u64()).unwrap_or(0);
        let v2 = r2.get(field).and_then(|v| v.as_u64()).unwrap_or(0);
        r2[field] = serde_json::json!(v1 + v2);
    }

    let mut mutations = r1
        .get("tool_mutations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(r2_muts) = r2.get("tool_mutations").and_then(|v| v.as_array()) {
        mutations.extend(r2_muts.iter().cloned());
    }
    r2["tool_mutations"] = serde_json::json!(mutations);

    tokio::fs::write(
        &r2_path,
        serde_json::to_string_pretty(&r2).map_err(std::io::Error::other)?,
    )
    .await?;

    let t1_path = run1_dir.join("trace.jsonl");
    let t2_path = run2_dir.join("trace.jsonl");
    if let (Ok(t1_content), Ok(t2_content)) = (
        tokio::fs::read_to_string(&t1_path).await,
        tokio::fs::read_to_string(&t2_path).await,
    ) {
        tokio::fs::write(&t2_path, format!("{t1_content}{t2_content}")).await?;
    }

    Ok(())
}

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
