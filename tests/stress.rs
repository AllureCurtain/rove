//! Local, deterministic, credential-free stress and soak gate.
//!
//! The gate drives the real `rove-api` router (in-process, so no socket and no
//! network), the real runtime, real persisted state, and the local fake
//! provider. It therefore needs no provider key and no reachable endpoint, and
//! the same run twice produces the same classification.
//!
//! It is opt-in. Without `ROVE_LOCAL_STRESS=1` every phase is skipped, the
//! summary records `status: "skipped"`, and the test passes — the skip path is
//! the only thing a default `cargo test` proves. With the flag set the gate
//! runs four bounded phases:
//!
//! 1. `sequential`: N jobs one after another, each reaching a terminal success
//!    state with the exact expected answer.
//! 2. `concurrent`: M jobs in flight in one workspace, all terminal, no run
//!    lost, duplicated, or left `running`, plus the per-session
//!    single-active-turn claim.
//! 3. `soak`: K iterations with a short delay, asserting the durable facts stay
//!    consistent (run count grows exactly once per iteration, no orphaned
//!    `running` run survives, completed work is not replayed).
//! 4. `restart_recovery`: a second API state over the same state root while
//!    work is in flight, asserting that an interrupted job is never reported as
//!    success and that completed work is neither replayed nor rewritten.
//!
//! Everything is bounded: job counts, per-job timeout, in-flight ceiling, soak
//! delay, and one total wall clock for the whole gate. The first phase that
//! fails stops the phases after it, and the summary records exactly the phases
//! that ran. The run writes
//! `local-stress-summary.json` under `target/local-stress-gate/` (overridable
//! with `ROVE_LOCAL_STRESS_ARTIFACTS`) with real counts, per-phase records, and
//! a typed classification.
//!
//! A process-level restart of a built `rove-api` binary stays with the
//! credentialed `scripts/provider-integration.ps1` gate; this gate emulates the
//! stop in-process by abandoning the state that owns the in-flight run and
//! constructing a new one over the same root, which is exactly what the new
//! process' `mark_running_jobs_interrupted` startup recovery then sees.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
use rove_api::{ApiState, CreateJobResponse, JobStateResponse, router};
use rove_app_bootstrap::{AppConfig, ProjectTrustRepository};
use rove_runtime::events::StreamEvent;
use rove_runtime::types::RunStatus;
use rove_runtime::workspace::Workspace;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::ServiceExt;

const GATE: &str = "local-stress";
const GATE_ENV: &str = "ROVE_LOCAL_STRESS";
const ARTIFACTS_ENV: &str = "ROVE_LOCAL_STRESS_ARTIFACTS";
const SUMMARY_FILE: &str = "local-stress-summary.json";
const POLL_INTERVAL: Duration = Duration::from_millis(25);

const DEFAULT_SEQUENTIAL: usize = 6;
const DEFAULT_CONCURRENT: usize = 4;
const DEFAULT_CONCURRENCY: usize = 4;
const DEFAULT_SOAK_ITERATIONS: usize = 5;
const DEFAULT_SOAK_DELAY_MS: u64 = 50;
const DEFAULT_JOB_TIMEOUT_MS: u64 = 20_000;
const DEFAULT_TOTAL_TIMEOUT_MS: u64 = 180_000;

const MAX_SEQUENTIAL: usize = 50;
const MAX_CONCURRENT: usize = 16;
const MAX_CONCURRENCY: usize = 16;
const MAX_SOAK_ITERATIONS: usize = 50;
const MAX_SOAK_DELAY_MS: u64 = 5_000;
const MAX_JOB_TIMEOUT_MS: u64 = 120_000;
const MAX_TOTAL_TIMEOUT_MS: u64 = 900_000;

/// Classifications this gate can publish. A run is only ever `pass` or
/// `skipped` when the behavior it asserts was actually observed.
mod classification {
    pub const PASS: &str = "pass";
    pub const SKIPPED: &str = "skipped";
    pub const TIMEOUT: &str = "timeout";
    pub const ASSERTION_FAILURE: &str = "assertion_failure";
    pub const ROVE_RUNTIME_DEFECT: &str = "rove_runtime_defect";
    pub const GATE_SETUP_FAILURE: &str = "gate_setup_failure";
}

#[derive(Debug)]
struct GateFailure {
    phase: &'static str,
    classification: &'static str,
    message: String,
}

impl GateFailure {
    fn assertion(phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            phase,
            classification: classification::ASSERTION_FAILURE,
            message: message.into(),
        }
    }

    fn defect(phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            phase,
            classification: classification::ROVE_RUNTIME_DEFECT,
            message: message.into(),
        }
    }

    fn timeout(phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            phase,
            classification: classification::TIMEOUT,
            message: message.into(),
        }
    }

    fn setup(phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            phase,
            classification: classification::GATE_SETUP_FAILURE,
            message: message.into(),
        }
    }
}

type GateResult<T> = Result<T, GateFailure>;

fn require(condition: bool, failure: GateFailure) -> GateResult<()> {
    if condition { Ok(()) } else { Err(failure) }
}

// ─── Configuration ──────────────────────────────────────────────────────────

struct GateConfig {
    sequential: usize,
    concurrent: usize,
    concurrency: usize,
    soak_iterations: usize,
    soak_delay: Duration,
    job_timeout: Duration,
    total_timeout: Duration,
}

impl GateConfig {
    fn from_env() -> Option<Self> {
        if !truthy(std::env::var(GATE_ENV).ok()) {
            return None;
        }
        Some(Self {
            sequential: env_usize(
                "ROVE_LOCAL_STRESS_SEQUENTIAL",
                DEFAULT_SEQUENTIAL,
                MAX_SEQUENTIAL,
            ),
            concurrent: env_usize(
                "ROVE_LOCAL_STRESS_CONCURRENT",
                DEFAULT_CONCURRENT,
                MAX_CONCURRENT,
            ),
            concurrency: env_usize(
                "ROVE_LOCAL_STRESS_CONCURRENCY",
                DEFAULT_CONCURRENCY,
                MAX_CONCURRENCY,
            ),
            soak_iterations: env_usize(
                "ROVE_LOCAL_STRESS_SOAK_ITERATIONS",
                DEFAULT_SOAK_ITERATIONS,
                MAX_SOAK_ITERATIONS,
            ),
            soak_delay: Duration::from_millis(env_u64(
                "ROVE_LOCAL_STRESS_SOAK_DELAY_MS",
                DEFAULT_SOAK_DELAY_MS,
                MAX_SOAK_DELAY_MS,
            )),
            job_timeout: Duration::from_millis(env_u64(
                "ROVE_LOCAL_STRESS_JOB_TIMEOUT_MS",
                DEFAULT_JOB_TIMEOUT_MS,
                MAX_JOB_TIMEOUT_MS,
            )),
            total_timeout: Duration::from_millis(env_u64(
                "ROVE_LOCAL_STRESS_TOTAL_TIMEOUT_MS",
                DEFAULT_TOTAL_TIMEOUT_MS,
                MAX_TOTAL_TIMEOUT_MS,
            )),
        })
    }

    /// The number of jobs the concurrent phase actually starts before waiting
    /// for any of them: both bounds apply, so the ceiling is never exceeded.
    fn in_flight(&self) -> usize {
        self.concurrent.min(self.concurrency).max(1)
    }

    fn to_json(&self) -> Value {
        json!({
            "sequential": self.sequential,
            "concurrent_requested": self.concurrent,
            "concurrency_ceiling": self.concurrency,
            "concurrent_in_flight": self.in_flight(),
            "soak_iterations": self.soak_iterations,
            "soak_delay_ms": self.soak_delay.as_millis() as u64,
            "job_timeout_ms": self.job_timeout.as_millis() as u64,
            "total_timeout_ms": self.total_timeout.as_millis() as u64,
        })
    }
}

/// The repository's existing opt-in contract: `provider-integration.ps1`
/// accepts `1`, `true`, `yes`, or `on` for a gate switch, and the env-gated
/// test targets accept `1`. This reuses that contract instead of inventing one.
fn truthy(value: Option<String>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn env_usize(name: &str, default: usize, cap: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(1, cap))
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64, cap: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(1, cap))
        .unwrap_or(default)
}

fn artifacts_dir() -> PathBuf {
    if let Ok(path) = std::env::var(ARTIFACTS_ENV)
        && !path.trim().is_empty()
    {
        return PathBuf::from(path.trim());
    }
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.join("target").join("local-stress-gate")
}

fn write_summary(dir: &Path, summary: &Value) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        eprintln!("local-stress: could not create {}: {error}", dir.display());
        return;
    }
    let path = dir.join(SUMMARY_FILE);
    match serde_json::to_string_pretty(summary) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(()) => eprintln!("local-stress: summary written to {}", path.display()),
            Err(error) => eprintln!("local-stress: could not write {}: {error}", path.display()),
        },
        Err(error) => eprintln!("local-stress: could not render the summary: {error}"),
    }
}

// ─── Server assembly ────────────────────────────────────────────────────────

/// One API state over one isolated state root.
///
/// Every path it resolves lives under the gate's own temporary directory: the
/// server state root, the user catalog the provider profiles would come from,
/// the ProductStore, and the Project Trust authority. The operator's real rove
/// state is never read or written.
struct Server {
    app: Router,
    _state: ApiState,
}

impl Server {
    fn start(server_dir: &Path, workspace_dir: &Path) -> GateResult<Self> {
        const PHASE: &str = "setup";
        std::fs::create_dir_all(server_dir)
            .map_err(|error| GateFailure::setup(PHASE, format!("state root: {error}")))?;
        std::fs::create_dir_all(workspace_dir)
            .map_err(|error| GateFailure::setup(PHASE, format!("workspace root: {error}")))?;
        let mut config = AppConfig::default();
        config.provider.model = "fake".to_string();
        config.runtime.max_steps = 2;
        config.source_summary.user_config_path = server_dir.join("user-config/config.toml");
        let workspace = Workspace::detect(workspace_dir)
            .map_err(|error| GateFailure::setup(PHASE, format!("workspace detect: {error}")))?;
        let trust = Arc::new(ProjectTrustRepository::new(
            server_dir.join("project-trust.sqlite"),
        ));
        let state = ApiState::with_project_trust_repository(workspace, config, trust);
        let app = router(state.clone());
        Ok(Self { app, _state: state })
    }
}

// ─── HTTP helpers ───────────────────────────────────────────────────────────

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    phase: &'static str,
) -> GateResult<(StatusCode, Value)> {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let request = builder
        .body(body)
        .map_err(|error| GateFailure::assertion(phase, format!("{method} {uri}: {error}")))?;
    let response = app
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| GateFailure::assertion(phase, format!("{method} {uri}: {error}")))?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| GateFailure::assertion(phase, format!("{method} {uri}: {error}")))?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok((status, value))
}

async fn get_json(app: &Router, uri: &str, phase: &'static str) -> GateResult<Value> {
    let (status, value) = send(app, "GET", uri, None, phase).await?;
    require(
        status == StatusCode::OK,
        GateFailure::assertion(phase, format!("GET {uri} answered {status}: {value}")),
    )?;
    Ok(value)
}

async fn post_json(app: &Router, uri: &str, body: Value, phase: &'static str) -> GateResult<Value> {
    let (status, value) = send(app, "POST", uri, Some(body), phase).await?;
    require(
        status.is_success(),
        GateFailure::assertion(phase, format!("POST {uri} answered {status}: {value}")),
    )?;
    Ok(value)
}

async fn put_json(app: &Router, uri: &str, body: Value, phase: &'static str) -> GateResult<Value> {
    let (status, value) = send(app, "PUT", uri, Some(body), phase).await?;
    require(
        status.is_success(),
        GateFailure::assertion(phase, format!("PUT {uri} answered {status}: {value}")),
    )?;
    Ok(value)
}

async fn get_typed<T: DeserializeOwned>(
    app: &Router,
    uri: &str,
    phase: &'static str,
) -> GateResult<T> {
    let value = get_json(app, uri, phase).await?;
    serde_json::from_value(value).map_err(|error| {
        GateFailure::assertion(
            phase,
            format!("GET {uri} was not the expected type: {error}"),
        )
    })
}

async fn job_state(
    app: &Router,
    job_id: &str,
    phase: &'static str,
) -> GateResult<JobStateResponse> {
    get_typed(app, &format!("/jobs/{job_id}/state"), phase).await
}

/// Every run the state index currently knows, bounded by the gate's own run
/// count rather than by the API's default page.
async fn list_runs(app: &Router, phase: &'static str) -> GateResult<Vec<Value>> {
    let value = get_json(app, "/runs?limit=500", phase).await?;
    value["runs"].as_array().cloned().ok_or_else(|| {
        GateFailure::assertion(phase, format!("the run listing had no runs array: {value}"))
    })
}

fn find_run<'a>(runs: &'a [Value], run_id: &str) -> Option<&'a Value> {
    runs.iter().find(|run| run["run_id"] == run_id)
}

fn running_runs(runs: &[Value]) -> Vec<String> {
    runs.iter()
        .filter(|run| run["status"] == "running")
        .map(|run| run["run_id"].to_string())
        .collect()
}

fn status_text(status: &RunStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}"))
}

fn is_terminal(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Done | RunStatus::Error | RunStatus::Cancelled | RunStatus::Interrupted
    )
}

/// What is left of a shared burst budget, so a batch of bounded waits cannot
/// add up to an unbounded total.
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

// ─── Waits (every one bounded by the configured per-job timeout) ────────────

async fn wait_job_terminal(
    app: &Router,
    job_id: &str,
    timeout: Duration,
    phase: &'static str,
) -> GateResult<JobStateResponse> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = job_state(app, job_id, phase).await?;
        if is_terminal(&state.status) {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(GateFailure::timeout(
                phase,
                format!(
                    "job {job_id} did not reach a terminal state within {}ms; last status `{}`",
                    timeout.as_millis(),
                    status_text(&state.status)
                ),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_job_pending_approval(
    app: &Router,
    job_id: &str,
    timeout: Duration,
    phase: &'static str,
) -> GateResult<JobStateResponse> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = job_state(app, job_id, phase).await?;
        if !state.pending_approvals.is_empty() {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(GateFailure::timeout(
                phase,
                format!(
                    "job {job_id} did not ask for an approval within {}ms; last status `{}`",
                    timeout.as_millis(),
                    status_text(&state.status)
                ),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn product_session(
    app: &Router,
    workspace_id: &str,
    session_id: &str,
    phase: &'static str,
) -> GateResult<Value> {
    let listing = get_json(
        app,
        &format!("/product/sessions?workspace_id={workspace_id}"),
        phase,
    )
    .await?;
    listing["sessions"]
        .as_array()
        .and_then(|sessions| {
            sessions
                .iter()
                .find(|session| session["id"] == session_id)
                .cloned()
        })
        .ok_or_else(|| {
            GateFailure::assertion(
                phase,
                format!("product session {session_id} was not in the workspace listing"),
            )
        })
}

async fn wait_for_session_status(
    app: &Router,
    workspace_id: &str,
    session_id: &str,
    expected: &str,
    timeout: Duration,
    phase: &'static str,
) -> GateResult<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let session = product_session(app, workspace_id, session_id, phase).await?;
        if session["status"] == expected {
            return Ok(session);
        }
        if Instant::now() >= deadline {
            return Err(GateFailure::timeout(
                phase,
                format!(
                    "product session {session_id} did not reach `{expected}` within {}ms; last status `{}`",
                    timeout.as_millis(),
                    session["status"]
                ),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ─── Domain helpers ─────────────────────────────────────────────────────────

fn expected_answer(message: &str) -> String {
    // `apps/api` answers the `fake` model shortcut with `fake response: <message>`
    // for both generic and product turns, so the exact expected text is a
    // deterministic function of the request.
    format!("fake response: {message}")
}

async fn create_generic_job(
    app: &Router,
    message: &str,
    phase: &'static str,
) -> GateResult<CreateJobResponse> {
    let value = post_json(
        app,
        "/jobs",
        json!({
            "message": message,
            "model": "fake",
            "approval": "auto",
            "max_steps": 2
        }),
        phase,
    )
    .await?;
    serde_json::from_value(value).map_err(|error| {
        GateFailure::assertion(phase, format!("POST /jobs did not answer a job: {error}"))
    })
}

async fn create_product_workspace(
    app: &Router,
    root: &Path,
    phase: &'static str,
) -> GateResult<String> {
    let value = post_json(
        app,
        "/product/workspaces",
        json!({
            "root": root,
            "kind": "folder",
            "display_name": "Local stress gate workspace",
            "pinned": false
        }),
        phase,
    )
    .await?;
    value["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| GateFailure::assertion(phase, format!("workspace create: {value}")))
}

async fn create_product_session(
    app: &Router,
    workspace_id: &str,
    title: &str,
    phase: &'static str,
) -> GateResult<String> {
    let value = post_json(
        app,
        "/product/sessions",
        json!({ "workspace_id": workspace_id, "title": title }),
        phase,
    )
    .await?;
    value["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| GateFailure::assertion(phase, format!("session create: {value}")))
}

async fn configure_session_model(
    app: &Router,
    session_id: &str,
    model: &str,
    max_steps: u32,
    phase: &'static str,
) -> GateResult<()> {
    let current = get_json(
        app,
        &format!("/product/sessions/{session_id}/model-config"),
        phase,
    )
    .await?;
    let value = put_json(
        app,
        &format!("/product/sessions/{session_id}/model-config"),
        json!({
            "model": model,
            "reasoning": "default",
            "max_steps": max_steps,
            "expected_revision": current["revision"]
        }),
        phase,
    )
    .await?;
    require(
        value["model"] == model,
        GateFailure::assertion(
            phase,
            format!("session model config did not persist `{model}`: {value}"),
        ),
    )
}

async fn create_product_job(
    app: &Router,
    session_id: &str,
    message: &str,
    phase: &'static str,
) -> GateResult<CreateJobResponse> {
    let value = post_json(
        app,
        "/jobs",
        json!({ "message": message, "product_session_id": session_id }),
        phase,
    )
    .await?;
    serde_json::from_value(value).map_err(|error| {
        GateFailure::assertion(phase, format!("POST /jobs did not answer a job: {error}"))
    })
}

/// Assert one completed job's own durable answer, from both its canonical
/// events and its persisted report.
async fn assert_expected_answer(
    app: &Router,
    created: &CreateJobResponse,
    state: &JobStateResponse,
    message: &str,
    phase: &'static str,
) -> GateResult<()> {
    let expected = expected_answer(message);
    require(
        state.events.iter().any(|stored| {
            matches!(&stored.event, StreamEvent::LlmMessage { full, .. } if full == &expected)
        }),
        GateFailure::assertion(
            phase,
            format!(
                "run {} never published the exact expected answer `{expected}`",
                created.run_id
            ),
        ),
    )?;
    let report = get_json(app, &format!("/runs/{}/report", created.run_id), phase).await?;
    require(
        report["status"] == "success",
        GateFailure::defect(
            phase,
            format!(
                "run {} report status was {}",
                created.run_id, report["status"]
            ),
        ),
    )?;
    require(
        report["run_id"] == created.run_id.to_string()
            && report["job_id"] == created.job_id.to_string(),
        GateFailure::defect(
            phase,
            format!(
                "run {} report carried another identity: {report}",
                created.run_id
            ),
        ),
    )?;
    let output = report["output"].as_str().unwrap_or_default();
    require(
        output.contains(&expected),
        GateFailure::assertion(
            phase,
            format!(
                "run {} report did not carry `{expected}`: {output}",
                created.run_id
            ),
        ),
    )
}

// ─── Phases ─────────────────────────────────────────────────────────────────

async fn run_phase<F>(phases: &mut Vec<Value>, name: &'static str, body: F) -> GateResult<()>
where
    F: Future<Output = GateResult<Value>>,
{
    let started = Instant::now();
    let outcome = body.await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let record = match &outcome {
        Ok(fields) => {
            let mut record = json!({ "name": name, "status": "pass", "duration_ms": duration_ms });
            if let (Some(record), Some(fields)) = (record.as_object_mut(), fields.as_object()) {
                for (key, value) in fields {
                    record.insert(key.clone(), value.clone());
                }
            }
            record
        }
        Err(failure) => json!({
            "name": name,
            "status": "fail",
            "duration_ms": duration_ms,
            "phase": failure.phase,
            "classification": failure.classification,
            "message": failure.message,
        }),
    };
    phases.push(record);
    outcome.map(|_| ())
}

async fn run_gate(
    config: &GateConfig,
    server_dir: &Path,
    workspace_dir: &Path,
    phases: &mut Vec<Value>,
) -> GateResult<()> {
    let server = Server::start(server_dir, workspace_dir)?;
    let mut expected_runs = 0usize;
    run_phase(
        phases,
        "sequential",
        phase_sequential(&server.app, config, &mut expected_runs),
    )
    .await?;
    run_phase(
        phases,
        "concurrent",
        phase_concurrent(&server.app, config, workspace_dir, &mut expected_runs),
    )
    .await?;
    run_phase(
        phases,
        "soak",
        phase_soak(&server.app, config, &mut expected_runs),
    )
    .await?;
    run_phase(
        phases,
        "restart_recovery",
        phase_restart(
            &server.app,
            config,
            server_dir,
            workspace_dir,
            &mut expected_runs,
        ),
    )
    .await?;
    Ok(())
}

async fn phase_sequential(
    app: &Router,
    config: &GateConfig,
    expected_runs: &mut usize,
) -> GateResult<Value> {
    const PHASE: &str = "sequential";
    let mut run_ids = Vec::new();
    for index in 1..=config.sequential {
        let message = format!("local stress sequential {index}");
        let created = create_generic_job(app, &message, PHASE).await?;
        let state =
            wait_job_terminal(app, &created.job_id.to_string(), config.job_timeout, PHASE).await?;
        require(
            state.status == RunStatus::Done,
            GateFailure::assertion(
                PHASE,
                format!(
                    "sequential job {index} ended `{}`",
                    status_text(&state.status)
                ),
            ),
        )?;
        assert_expected_answer(app, &created, &state, &message, PHASE).await?;
        *expected_runs += 1;
        let runs = list_runs(app, PHASE).await?;
        require(
            runs.len() == *expected_runs,
            GateFailure::defect(
                PHASE,
                format!(
                    "after job {index} the index listed {} runs, expected {}",
                    runs.len(),
                    *expected_runs
                ),
            ),
        )?;
        run_ids.push(created.run_id.to_string());
    }
    Ok(json!({
        "jobs": config.sequential,
        "jobs_completed": run_ids.len(),
        "jobs_interrupted": 0,
        "runs_total": *expected_runs,
        "run_ids": run_ids,
    }))
}

async fn phase_concurrent(
    app: &Router,
    config: &GateConfig,
    workspace_dir: &Path,
    expected_runs: &mut usize,
) -> GateResult<Value> {
    const PHASE: &str = "concurrent";
    let in_flight = config.in_flight();
    let workspace_id = create_product_workspace(app, workspace_dir, PHASE).await?;
    let mut sessions = Vec::new();
    for index in 1..=in_flight {
        let session_id = create_product_session(
            app,
            &workspace_id,
            &format!("Local stress concurrent {index}"),
            PHASE,
        )
        .await?;
        configure_session_model(app, &session_id, "fake-raw", 1, PHASE).await?;
        sessions.push(session_id);
    }

    // Every job is a real mutation that stops at its approval boundary, so the
    // whole batch is genuinely in flight in one workspace at the same time:
    // submitted before any of them is waited on, each one parked with a side
    // effect that has not happened yet.
    let mut created = Vec::new();
    let mut targets = Vec::new();
    for (offset, session_id) in sessions.iter().enumerate() {
        let index = offset + 1;
        let file = format!("concurrent-turn-{index}.txt");
        let content = format!("concurrent turn {index}");
        let message = json!({
            "tool": "write_file",
            "args": { "path": file, "content": content }
        })
        .to_string();
        let job = create_product_job(app, session_id, &message, PHASE).await?;
        created.push(job);
        targets.push((workspace_dir.join(&file), content));
    }
    *expected_runs += created.len();
    let mut job_ids: Vec<String> = created.iter().map(|job| job.job_id.to_string()).collect();
    let mut run_ids: Vec<String> = created.iter().map(|job| job.run_id.to_string()).collect();
    job_ids.sort();
    job_ids.dedup();
    run_ids.sort();
    run_ids.dedup();
    require(
        job_ids.len() == created.len() && run_ids.len() == created.len(),
        GateFailure::defect(
            PHASE,
            "concurrent jobs did not receive distinct job and run identities",
        ),
    )?;

    // All of them are parked at once, and none of them has run its tool yet.
    let burst_deadline = Instant::now() + config.job_timeout;
    let mut pending = Vec::new();
    for job in &created {
        let state = wait_job_pending_approval(
            app,
            &job.job_id.to_string(),
            remaining(burst_deadline),
            PHASE,
        )
        .await?;
        require(
            state.status == RunStatus::Running,
            GateFailure::defect(
                PHASE,
                format!(
                    "a turn awaiting approval was reported as `{}`",
                    status_text(&state.status)
                ),
            ),
        )?;
        require(
            state.pending_approvals.len() == 1 && state.pending_approvals[0].name == "write_file",
            GateFailure::defect(
                PHASE,
                format!("job {} awaited an unexpected approval set", job.job_id),
            ),
        )?;
        require(
            state.resumed_from_run_id.is_none(),
            GateFailure::defect(
                PHASE,
                format!(
                    "fresh concurrent session {} resumed another run",
                    job.job_id
                ),
            ),
        )?;
        pending.push(state);
    }
    for (path, _) in &targets {
        require(
            !path.exists(),
            GateFailure::defect(PHASE, format!("{} ran before its approval", path.display())),
        )?;
    }
    for session_id in &sessions {
        let claimed = product_session(app, &workspace_id, session_id, PHASE).await?;
        require(
            claimed["status"] == "running",
            GateFailure::defect(
                PHASE,
                format!(
                    "a session with a turn in flight was `{}`",
                    claimed["status"]
                ),
            ),
        )?;
    }

    // The per-session single-active-turn claim: a second turn on a session whose
    // turn is in flight is refused, not queued or silently merged.
    let (refused_status, refused) = send(
        app,
        "POST",
        "/jobs",
        Some(json!({
            "message": "local stress second turn on a busy session",
            "product_session_id": sessions[0]
        })),
        PHASE,
    )
    .await?;
    require(
        refused_status == StatusCode::CONFLICT,
        GateFailure::defect(
            PHASE,
            format!(
                "a second turn on a session with an active turn answered {refused_status}, expected 409: {refused}"
            ),
        ),
    )?;
    require(
        refused["code"] == "product_session_active",
        GateFailure::defect(
            PHASE,
            format!("a refused second turn carried code `{}`", refused["code"]),
        ),
    )?;
    let runs = list_runs(app, PHASE).await?;
    require(
        runs.len() == *expected_runs,
        GateFailure::defect(
            PHASE,
            format!(
                "a refused second turn changed the run count: {} != {}",
                runs.len(),
                *expected_runs
            ),
        ),
    )?;

    // Releasing every claim lets the whole batch finish, so the refusal above is
    // a claim on one turn rather than an unreleasable session.
    for (job, state) in created.iter().zip(pending.iter()) {
        let call_id = state.pending_approvals[0].call_id.to_string();
        let (approve_status, approve) = send(
            app,
            "POST",
            &format!("/jobs/{}/approvals/{call_id}", job.job_id),
            Some(json!({ "decision": "approve" })),
            PHASE,
        )
        .await?;
        require(
            approve_status == StatusCode::OK,
            GateFailure::assertion(
                PHASE,
                format!(
                    "approving job {} answered {approve_status}: {approve}",
                    job.job_id
                ),
            ),
        )?;
    }
    for job in &created {
        let state =
            wait_job_terminal(app, &job.job_id.to_string(), config.job_timeout, PHASE).await?;
        require(
            state.status == RunStatus::Done,
            GateFailure::assertion(
                PHASE,
                format!(
                    "concurrent job {} ended `{}`",
                    job.job_id,
                    status_text(&state.status)
                ),
            ),
        )?;
    }
    for (path, content) in &targets {
        let written = std::fs::read_to_string(path).map_err(|error| {
            GateFailure::assertion(
                PHASE,
                format!(
                    "the approved write_file did not produce {}: {error}",
                    path.display()
                ),
            )
        })?;
        require(
            written == *content,
            GateFailure::assertion(
                PHASE,
                format!(
                    "{} carried `{written}` instead of `{content}`",
                    path.display()
                ),
            ),
        )?;
    }
    for job in &created {
        let report = get_json(app, &format!("/runs/{}/report", job.run_id), PHASE).await?;
        require(
            report["status"] == "success" && report["tool_calls"].as_u64() == Some(1),
            GateFailure::defect(
                PHASE,
                format!(
                    "concurrent run {} did not record exactly one successful tool call: {report}",
                    job.run_id
                ),
            ),
        )?;
    }
    for session_id in &sessions {
        wait_for_session_status(
            app,
            &workspace_id,
            session_id,
            "idle",
            config.job_timeout,
            PHASE,
        )
        .await?;
    }
    let runs = list_runs(app, PHASE).await?;
    require(
        runs.len() == *expected_runs,
        GateFailure::defect(
            PHASE,
            format!(
                "the index listed {} runs after {} concurrent jobs, expected {}",
                runs.len(),
                created.len(),
                *expected_runs
            ),
        ),
    )?;
    require(
        running_runs(&runs).is_empty(),
        GateFailure::defect(
            PHASE,
            format!(
                "runs were still `running` after every concurrent job settled: {:?}",
                running_runs(&runs)
            ),
        ),
    )?;
    for job in &created {
        require(
            find_run(&runs, &job.run_id.to_string()).is_some(),
            GateFailure::defect(
                PHASE,
                format!("concurrent run {} was lost from the index", job.run_id),
            ),
        )?;
    }

    Ok(json!({
        "jobs": created.len(),
        "jobs_completed": created.len(),
        "jobs_interrupted": 0,
        "concurrent_in_flight": in_flight,
        "jobs_parked_at_once": pending.len(),
        "single_active_turn": "refused with product_session_active",
        "side_effects_verified": targets.len(),
        "runs_total": *expected_runs,
        "run_ids": run_ids,
    }))
}

async fn phase_soak(
    app: &Router,
    config: &GateConfig,
    expected_runs: &mut usize,
) -> GateResult<Value> {
    const PHASE: &str = "soak";
    let mut tracked: Vec<(String, u64)> = Vec::new();
    let mut run_ids = Vec::new();
    for iteration in 1..=config.soak_iterations {
        let message = format!("local stress soak {iteration}");
        let created = create_generic_job(app, &message, PHASE).await?;
        let state =
            wait_job_terminal(app, &created.job_id.to_string(), config.job_timeout, PHASE).await?;
        require(
            state.status == RunStatus::Done,
            GateFailure::assertion(
                PHASE,
                format!(
                    "soak iteration {iteration} ended `{}`",
                    status_text(&state.status)
                ),
            ),
        )?;
        assert_expected_answer(app, &created, &state, &message, PHASE).await?;
        *expected_runs += 1;

        let runs = list_runs(app, PHASE).await?;
        require(
            runs.len() == *expected_runs,
            GateFailure::defect(
                PHASE,
                format!(
                    "after soak iteration {iteration} the index listed {} runs, expected {}",
                    runs.len(),
                    *expected_runs
                ),
            ),
        )?;
        let running = running_runs(&runs);
        require(
            running.is_empty(),
            GateFailure::defect(
                PHASE,
                format!("soak iteration {iteration} left orphaned `running` runs: {running:?}"),
            ),
        )?;
        // A completed run keeps its exact durable facts: same status, same last
        // event sequence. A replay would append events or rewrite the status.
        for (run_id, last_event_seq) in &tracked {
            let Some(record) = find_run(&runs, run_id) else {
                return Err(GateFailure::defect(
                    PHASE,
                    format!("soak iteration {iteration} lost completed run {run_id}"),
                ));
            };
            require(
                record["status"] == "done"
                    && record["last_event_seq"].as_u64() == Some(*last_event_seq),
                GateFailure::defect(
                    PHASE,
                    format!(
                        "completed run {run_id} was replayed or rewritten during soak iteration {iteration}: {record}"
                    ),
                ),
            )?;
        }
        let run_id = created.run_id.to_string();
        let last_event_seq = find_run(&runs, &run_id)
            .and_then(|record| record["last_event_seq"].as_u64())
            .ok_or_else(|| {
                GateFailure::defect(
                    PHASE,
                    format!("soak iteration {iteration} run {run_id} had no event sequence"),
                )
            })?;
        require(
            last_event_seq > 0,
            GateFailure::defect(
                PHASE,
                format!("soak iteration {iteration} run {run_id} recorded no events"),
            ),
        )?;
        tracked.push((run_id.clone(), last_event_seq));
        run_ids.push(run_id);
        if !config.soak_delay.is_zero() {
            tokio::time::sleep(config.soak_delay).await;
        }
    }
    Ok(json!({
        "iterations": config.soak_iterations,
        "delay_ms": config.soak_delay.as_millis() as u64,
        "jobs": config.soak_iterations,
        "jobs_completed": run_ids.len(),
        "jobs_interrupted": 0,
        "runs_total": *expected_runs,
        "run_ids": run_ids,
    }))
}

async fn phase_restart(
    app: &Router,
    config: &GateConfig,
    server_dir: &Path,
    workspace_dir: &Path,
    expected_runs: &mut usize,
) -> GateResult<Value> {
    const PHASE: &str = "restart_recovery";

    // Completed work to preserve across the restart.
    let baseline_message = "local stress restart baseline";
    let baseline = create_generic_job(app, baseline_message, PHASE).await?;
    let baseline_state =
        wait_job_terminal(app, &baseline.job_id.to_string(), config.job_timeout, PHASE).await?;
    require(
        baseline_state.status == RunStatus::Done,
        GateFailure::assertion(
            PHASE,
            format!(
                "the baseline job ended `{}`",
                status_text(&baseline_state.status)
            ),
        ),
    )?;
    assert_expected_answer(app, &baseline, &baseline_state, baseline_message, PHASE).await?;
    *expected_runs += 1;
    let baseline_report =
        get_json(app, &format!("/runs/{}/report", baseline.run_id), PHASE).await?;
    let baseline_output = baseline_report["output"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let before = list_runs(app, PHASE).await?;
    let baseline_seq = find_run(&before, &baseline.run_id.to_string())
        .and_then(|record| record["last_event_seq"].as_u64())
        .ok_or_else(|| GateFailure::defect(PHASE, "the baseline run had no event sequence"))?;

    // Work in flight at the moment the API stops: a turn parked on a destructive
    // tool approval. It has a durable `running` run and an unknown in-flight side
    // effect, which is exactly what recovery has to stay honest about.
    let workspace_id = create_product_workspace(app, workspace_dir, PHASE).await?;
    let session_id =
        create_product_session(app, &workspace_id, "Local stress restart recovery", PHASE).await?;
    configure_session_model(app, &session_id, "fake-raw", 1, PHASE).await?;
    let target = workspace_dir.join("restart-recovery-not-replayed.txt");
    let in_flight_message = json!({
        "tool": "write_file",
        "args": {
            "path": "restart-recovery-not-replayed.txt",
            "content": "must not be replayed after a restart"
        }
    })
    .to_string();
    let in_flight = create_product_job(app, &session_id, &in_flight_message, PHASE).await?;
    *expected_runs += 1;
    let pending = wait_job_pending_approval(
        app,
        &in_flight.job_id.to_string(),
        config.job_timeout,
        PHASE,
    )
    .await?;
    require(
        pending.status == RunStatus::Running,
        GateFailure::defect(
            PHASE,
            format!(
                "the in-flight job was `{}` while awaiting approval",
                status_text(&pending.status)
            ),
        ),
    )?;
    require(
        !target.exists(),
        GateFailure::defect(
            PHASE,
            format!("{} ran before its approval", target.display()),
        ),
    )?;
    let before_restart = list_runs(app, PHASE).await?;
    require(
        before_restart.len() == *expected_runs,
        GateFailure::defect(
            PHASE,
            format!(
                "before the restart the index listed {} runs, expected {}",
                before_restart.len(),
                *expected_runs
            ),
        ),
    )?;

    // The state that owns the in-flight run is abandoned without its graceful
    // drain — a killed process cannot drain anything — and a new state is built
    // over the same state root, which is what the restarted API process does.
    // The difference from a real kill is that the abandoned state's own SQLite
    // connections stay open for the rest of the test; it holds no write lock
    // because its only unfinished work is parked on an approval, and the first
    // thing the new state does is the same startup recovery a new process runs.
    let restarted = Server::start(server_dir, workspace_dir)?;
    let app = &restarted.app;

    let after = list_runs(app, PHASE).await?;
    require(
        after.len() == before_restart.len(),
        GateFailure::defect(
            PHASE,
            format!(
                "the restart changed the run count: {} != {} (completed work must not replay)",
                after.len(),
                before_restart.len()
            ),
        ),
    )?;

    let interrupted = find_run(&after, &in_flight.run_id.to_string()).ok_or_else(|| {
        GateFailure::defect(
            PHASE,
            format!("the restart lost in-flight run {}", in_flight.run_id),
        )
    })?;
    require(
        interrupted["status"] != "done" && interrupted["status"] != "success",
        GateFailure::defect(
            PHASE,
            format!("an interrupted run was reported as success after the restart: {interrupted}"),
        ),
    )?;
    require(
        interrupted["status"] == "interrupted",
        GateFailure::defect(
            PHASE,
            format!(
                "the restarted index reported the in-flight run as `{}`, expected `interrupted`",
                interrupted["status"]
            ),
        ),
    )?;
    require(
        interrupted["has_report"] == false,
        GateFailure::defect(
            PHASE,
            format!(
                "an interrupted run published a success report after the restart: {interrupted}"
            ),
        ),
    )?;
    let running = running_runs(&after);
    require(
        running.is_empty(),
        GateFailure::defect(
            PHASE,
            format!("orphaned `running` runs survived the restart: {running:?}"),
        ),
    )?;

    let interrupted_state =
        get_typed::<JobStateResponse>(app, &format!("/jobs/{}/state", in_flight.job_id), PHASE)
            .await?;
    require(
        interrupted_state.status == RunStatus::Interrupted,
        GateFailure::defect(
            PHASE,
            format!(
                "the restarted API reported the interrupted job as `{}`",
                status_text(&interrupted_state.status)
            ),
        ),
    )?;
    require(
        interrupted_state.pending_approvals.is_empty(),
        GateFailure::defect(
            PHASE,
            "an interrupted job still advertised a pending approval after the restart",
        ),
    )?;

    let (report_status, report) = send(
        app,
        "GET",
        &format!("/runs/{}/report", in_flight.run_id),
        None,
        PHASE,
    )
    .await?;
    require(
        report_status == StatusCode::NOT_FOUND,
        GateFailure::defect(
            PHASE,
            format!(
                "an interrupted run answered {report_status} for its report instead of 404: {report}"
            ),
        ),
    )?;

    let preserved = find_run(&after, &baseline.run_id.to_string()).ok_or_else(|| {
        GateFailure::defect(
            PHASE,
            format!("the restart lost completed run {}", baseline.run_id),
        )
    })?;
    require(
        preserved["status"] == "done" && preserved["last_event_seq"].as_u64() == Some(baseline_seq),
        GateFailure::defect(
            PHASE,
            format!(
                "completed run {} was rewritten by the restart: {preserved}",
                baseline.run_id
            ),
        ),
    )?;
    let after_report = get_json(app, &format!("/runs/{}/report", baseline.run_id), PHASE).await?;
    require(
        after_report["output"].as_str() == Some(baseline_output.as_str()),
        GateFailure::defect(
            PHASE,
            format!(
                "completed run {} was replayed by the restart",
                baseline.run_id
            ),
        ),
    )?;

    let session = product_session(app, &workspace_id, &session_id, PHASE).await?;
    require(
        session["status"] == "needs_attention",
        GateFailure::defect(
            PHASE,
            format!(
                "the interrupted session was reported as `{}` after the restart, expected `needs_attention`",
                session["status"]
            ),
        ),
    )?;
    let (refused_status, refused) = send(
        app,
        "POST",
        "/jobs",
        Some(json!({
            "message": "local stress turn after an interrupted restart",
            "product_session_id": session_id
        })),
        PHASE,
    )
    .await?;
    require(
        refused_status == StatusCode::CONFLICT,
        GateFailure::defect(
            PHASE,
            format!(
                "a session needing runtime recovery accepted a new turn with {refused_status}: {refused}"
            ),
        ),
    )?;
    require(
        refused["code"] == "product_session_runtime_state_missing",
        GateFailure::defect(
            PHASE,
            format!(
                "the refused recovery turn carried code `{}`",
                refused["code"]
            ),
        ),
    )?;
    let after_refusal = list_runs(app, PHASE).await?;
    require(
        after_refusal.len() == after.len(),
        GateFailure::defect(
            PHASE,
            format!(
                "a refused recovery turn changed the run count: {} != {}",
                after_refusal.len(),
                after.len()
            ),
        ),
    )?;

    Ok(json!({
        "jobs": 2,
        "jobs_completed": 1,
        "jobs_interrupted": 1,
        "interrupted_run_id": in_flight.run_id.to_string(),
        "interrupted_status": "interrupted",
        "interrupted_report": "404",
        "completed_runs_preserved": 1,
        "replayed_runs": 0,
        "session_after_restart": "needs_attention",
        "runs_total": after.len(),
        "restart_emulation": "abandoned_state_plus_new_state_over_the_same_root",
    }))
}

// ─── Entry point ────────────────────────────────────────────────────────────

/// The summary for a gate that produced no phases: the skip path, and the one
/// setup failure that happens before the gate can classify anything itself.
fn no_phase_summary(status: &str, gate_classification: &str, reason: String) -> Value {
    json!({
        "schema_version": 1,
        "gate": GATE,
        "status": status,
        "classification": gate_classification,
        "reason": reason,
        "env_var": GATE_ENV,
        "provider": "fake",
        "network_required": false,
        "credentials_required": false,
        "config": Value::Null,
        "phases": [],
        "totals": {
            "phases_run": 0,
            "phases_passed": 0,
            "jobs_started": 0,
            "jobs_completed": 0,
            "jobs_interrupted": 0,
            "failures": u64::from(status == "fail"),
            "skipped": u64::from(status == "skipped")
        }
    })
}

fn skipped_summary() -> Value {
    no_phase_summary(
        "skipped",
        classification::SKIPPED,
        format!("{GATE_ENV} is not set to a truthy value; this opt-in gate is off by default"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_deterministic_stress_and_soak_gate() {
    let artifacts = artifacts_dir();
    let Some(config) = GateConfig::from_env() else {
        write_summary(&artifacts, &skipped_summary());
        eprintln!(
            "local-stress gate skipped: set {GATE_ENV}=1 to run the deterministic fake-provider stress/soak gate"
        );
        return;
    };

    let root = match tempfile::TempDir::new() {
        Ok(root) => root,
        Err(error) => {
            let reason = format!("the gate could not create its temporary state root: {error}");
            write_summary(
                &artifacts,
                &no_phase_summary("fail", classification::GATE_SETUP_FAILURE, reason.clone()),
            );
            panic!(
                "local-stress gate fail ({}): {reason}",
                classification::GATE_SETUP_FAILURE
            );
        }
    };
    let server_dir = root.path().join("server");
    let workspace_dir = root.path().join("workspace");
    let mut phases: Vec<Value> = Vec::new();
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        config.total_timeout,
        run_gate(&config, &server_dir, &workspace_dir, &mut phases),
    )
    .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let (status, gate_classification, reason) = match outcome {
        Ok(Ok(())) => ("pass", classification::PASS.to_string(), None),
        Ok(Err(failure)) => (
            "fail",
            failure.classification.to_string(),
            Some(format!("{}: {}", failure.phase, failure.message)),
        ),
        Err(_) => (
            "fail",
            classification::TIMEOUT.to_string(),
            Some(format!(
                "the gate exceeded its total wall clock budget of {}ms",
                config.total_timeout.as_millis()
            )),
        ),
    };

    let phases_passed = phases
        .iter()
        .filter(|phase| phase["status"] == "pass")
        .count();
    let jobs_started: u64 = phases
        .iter()
        .filter_map(|phase| phase["jobs"].as_u64())
        .sum();
    // Both totals are summed from what each phase observed. A phase that failed
    // before it could count its jobs contributes to neither.
    let jobs_completed: u64 = phases
        .iter()
        .filter_map(|phase| phase["jobs_completed"].as_u64())
        .sum();
    let jobs_interrupted: u64 = phases
        .iter()
        .filter_map(|phase| phase["jobs_interrupted"].as_u64())
        .sum();
    let summary = json!({
        "schema_version": 1,
        "gate": GATE,
        "status": status,
        "classification": gate_classification,
        "reason": reason,
        "env_var": GATE_ENV,
        "provider": "fake",
        "network_required": false,
        "credentials_required": false,
        "duration_ms": elapsed_ms,
        "config": config.to_json(),
        "phases": phases,
        "totals": {
            "phases_run": phases.len(),
            "phases_passed": phases_passed,
            "jobs_started": jobs_started,
            "jobs_completed": jobs_completed,
            "jobs_interrupted": jobs_interrupted,
            "failures": u64::from(status == "fail"),
            "skipped": 0,
        }
    });
    write_summary(&artifacts, &summary);

    if let Some(reason) = reason {
        panic!("local-stress gate {status} ({gate_classification}): {reason}");
    }
    eprintln!(
        "local-stress gate pass: {} phases, {} jobs, {}ms",
        phases_passed, jobs_started, elapsed_ms
    );
}
