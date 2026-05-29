use std::path::{Path, PathBuf};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::core::context::ContextManager;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::events::StreamEvent;
use crate::core::types::{ApprovalPolicy, JobId, RunId, SessionId, TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::models::fake::{FakeModelClient, FakeTurn};
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::{RunHandle, StateStore};
use crate::tools::{default_tool_registry_with_shell_policy, shell::ShellPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub name: String,
    pub tasks: Vec<BenchmarkTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub name: String,
    pub message: String,
    #[serde(default)]
    pub setup_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub turns: Vec<BenchmarkTurn>,
    #[serde(default)]
    pub resume_state: Option<BenchmarkResumeState>,
    #[serde(default)]
    pub expected_output_contains: Vec<String>,
    #[serde(default)]
    pub expected_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub expected_summary_contains: Vec<String>,
    #[serde(default)]
    pub requires_network: Option<bool>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResumeState {
    pub goal: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub suite: String,
    pub passed: bool,
    pub tasks: Vec<BenchmarkTaskReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTaskReport {
    pub name: String,
    pub outcome: BenchmarkOutcome,
    pub artifacts: BenchmarkArtifacts,
    pub output: Option<String>,
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

pub async fn load_benchmark_suite(path: impl AsRef<Path>) -> std::io::Result<BenchmarkSuite> {
    let bytes = tokio::fs::read(path).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

pub async fn run_benchmark_suite(
    suite: &BenchmarkSuite,
    output_root: impl AsRef<Path>,
) -> std::io::Result<BenchmarkReport> {
    let output_root = output_root.as_ref();
    tokio::fs::create_dir_all(output_root).await?;

    let mut tasks = Vec::new();
    for task in &suite.tasks {
        tasks.push(run_benchmark_task(task, output_root).await?);
    }

    Ok(BenchmarkReport {
        suite: suite.name.clone(),
        passed: tasks
            .iter()
            .all(|task| task.outcome == BenchmarkOutcome::Passed),
        tasks,
    })
}

async fn run_benchmark_task(
    task: &BenchmarkTask,
    output_root: &Path,
) -> std::io::Result<BenchmarkTaskReport> {
    let workspace_root = output_root.join(&task.name).join("workspace");
    let state_dir = output_root.join(&task.name).join(".rove");
    tokio::fs::create_dir_all(&workspace_root).await?;
    tokio::fs::create_dir_all(&state_dir).await?;
    write_files(&workspace_root, &task.setup_files).await?;

    let workspace = Workspace {
        root: workspace_root.clone(),
        kind: crate::core::workspace::WorkspaceKind::Folder,
        state_dir: state_dir.clone(),
    };
    let state_store = StateStore::new(&state_dir);
    state_store.index.initialize()?;

    let session_id = SessionId::new();
    let job_id = JobId::new();
    let resume_state = match &task.resume_state {
        Some(resume_state) => {
            let previous = build_resume_state(session_id, resume_state);
            state_store.write_task_state(&previous).await?;
            Some(previous)
        }
        None => None,
    };
    let run = state_store.start_run(session_id, job_id, RunId::new())?;
    let artifacts = artifacts_for_run(&run);
    let engine = benchmark_engine(&workspace, task);
    let (reason, output) = run_engine_collect_output(
        &engine,
        &state_store,
        run,
        task.message.clone(),
        resume_state,
    )
    .await;

    let mut failures = Vec::new();
    if reason != TerminationReason::Final {
        failures.push(format!("expected final termination, got {reason:?}"));
    }
    for expected in &task.expected_output_contains {
        if !output.as_deref().unwrap_or_default().contains(expected) {
            failures.push(format!("output did not contain {expected:?}"));
        }
    }
    for expected in &task.expected_files {
        let path = workspace_root.join(&expected.path);
        match tokio::fs::read_to_string(&path).await {
            Ok(actual) if actual == expected.content => {}
            Ok(actual) => failures.push(format!(
                "file {} content mismatch: expected {:?}, got {:?}",
                expected.path, expected.content, actual
            )),
            Err(err) => failures.push(format!("file {} missing: {err}", expected.path)),
        }
    }
    let task_state = tokio::fs::read_to_string(&artifacts.task_state_json)
        .await
        .unwrap_or_default();
    for expected in &task.expected_summary_contains {
        if !task_state.contains(expected) {
            failures.push(format!("task state did not contain {expected:?}"));
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

    Ok(BenchmarkTaskReport {
        name: task.name.clone(),
        outcome: if failures.is_empty() {
            BenchmarkOutcome::Passed
        } else {
            BenchmarkOutcome::Failed
        },
        artifacts,
        output,
        failures,
    })
}

fn benchmark_engine(workspace: &Workspace, task: &BenchmarkTask) -> Engine {
    let model = Box::new(FakeModelClient::with_turns(
        "benchmark fallback response".to_string(),
        task.turns.iter().cloned().map(Into::into).collect(),
    ));
    let registry = default_tool_registry_with_shell_policy(workspace, ShellPolicy::default());
    Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are the rove benchmark runner.".to_string()),
        EngineConfig {
            max_steps: 8,
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
) -> (TerminationReason, Option<String>) {
    let resume_state_for_recorder = resume_state.clone();
    let req = run.request(message.clone(), resume_state);
    let RunHandle {
        session_id,
        job_id,
        run_id,
        run_dir,
        trace_writer,
    } = run;
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        message,
        resume_state_for_recorder.as_ref(),
    );
    let mut stream =
        std::pin::pin!(engine.run_with_cancel(req, Some(trace_writer), CancellationToken::new()));
    let mut reason = TerminationReason::Error;
    let mut output = None;

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        if let StreamEvent::RunCompleted {
            reason: event_reason,
            output: event_output,
        } = event
        {
            reason = event_reason;
            output = event_output;
            break;
        }
    }
    recorder
        .finalize(state_store, engine.workspace(), engine.model_id(), &run_dir)
        .await;

    (reason, output)
}

fn build_resume_state(session_id: SessionId, resume_state: &BenchmarkResumeState) -> TaskState {
    let job_id = JobId::new();
    let run_id = RunId::new();
    TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: resume_state.goal.clone(),
        step: 1,
        history: vec![crate::core::types::Message::user(resume_state.goal.clone())],
        summary: Some(resume_state.summary.clone()),
        checkpoint: None,
        plan: None,
    }
}

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
