//! Benchmark HTTP API handlers.
//!
//! Endpoints:
//!   GET  /bench/suites                 — list available benchmark suites
//!   POST /bench/runs                   — start a benchmark run
//!   GET  /bench/runs                   — list benchmark runs (newest first)
//!   GET  /bench/runs/{id}              — full result of one run
//!   GET  /bench/runs/{id}/tasks/{name} — single task result
//!   GET  /bench/runs/{id}/evidence/{*path} — read a file from the evidence package

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::bench::{
    BenchmarkOutcome, BenchmarkReport, available_suites, resolve_suite, run_benchmark_suite,
};
use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tokio::sync::RwLock;

use super::types::{
    BenchArtifactsResponse, BenchCheckResultResponse, BenchRunDetailResponse, BenchRunSummary,
    BenchSuiteInfoResponse, BenchTaskResultResponse, ListBenchRunsResponse,
    ListBenchSuitesResponse, StartBenchRunRequest, StartBenchRunResponse,
};
use super::{ApiError, ApiState};

const BENCH_OUTPUT_DIR: &str = "benchmarks/results";

/// In-memory tracking of benchmark runs started via the HTTP API.
#[derive(Debug, Default)]
pub(crate) struct BenchState {
    runs: RwLock<HashMap<String, Arc<BenchRunHandle>>>,
}

#[derive(Debug)]
struct BenchRunHandle {
    bench_run_id: String,
    suite: String,
    profile: String,
    result: RwLock<Option<BenchmarkReport>>,
    started_at: String,
}

#[utoipa::path(
    get,
    path = "/bench/suites",
    tag = "benchmark",
    responses(
        (status = 200, description = "Available benchmark suites", body = ListBenchSuitesResponse)
    )
)]
pub(crate) async fn list_bench_suites(
    State(_state): State<ApiState>,
) -> Json<ListBenchSuitesResponse> {
    let suites = available_suites()
        .into_iter()
        .map(|s| BenchSuiteInfoResponse {
            name: s.name,
            description: s.description,
            profiles: s.profiles,
        })
        .collect();
    Json(ListBenchSuitesResponse { suites })
}

#[utoipa::path(
    post,
    path = "/bench/runs",
    tag = "benchmark",
    request_body = StartBenchRunRequest,
    responses(
        (status = 200, description = "Benchmark run started", body = StartBenchRunResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub(crate) async fn start_bench_run(
    State(state): State<ApiState>,
    Json(req): Json<StartBenchRunRequest>,
) -> Result<Json<StartBenchRunResponse>, ApiError> {
    let bench_run_id = format!("bench_{}", chrono::Utc::now().format("%Y%m%dT%H%M%S%f"));
    let started_at = chrono::Utc::now().to_rfc3339();

    let suite = resolve_suite(&req.suite, &req.profile)
        .map_err(|e| ApiError::bad_request(format!("failed to resolve suite: {e}")))?;

    let output_dir = PathBuf::from(BENCH_OUTPUT_DIR);

    let handle = Arc::new(BenchRunHandle {
        bench_run_id: bench_run_id.clone(),
        suite: req.suite.clone(),
        profile: req.profile.clone(),
        result: RwLock::new(None),
        started_at: started_at.clone(),
    });

    state
        .inner
        .bench_runs
        .runs
        .write()
        .await
        .insert(bench_run_id.clone(), handle.clone());

    let bench_run_id_clone = bench_run_id.clone();
    let profile_clone = req.profile.clone();
    let suite_name = req.suite.clone();
    let profile_name = req.profile.clone();
    tokio::spawn(async move {
        match run_benchmark_suite(&suite, &output_dir, &profile_clone).await {
            Ok(report) => {
                *handle.result.write().await = Some(report);
            }
            Err(e) => {
                tracing::error!(bench_run_id = %bench_run_id_clone, "benchmark run failed: {e}");
            }
        }
    });

    Ok(Json(StartBenchRunResponse {
        bench_run_id,
        suite: suite_name,
        profile: profile_name,
        status: "running".to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/bench/runs",
    tag = "benchmark",
    responses(
        (status = 200, description = "Benchmark run history", body = ListBenchRunsResponse)
    )
)]
pub(crate) async fn list_bench_runs(State(state): State<ApiState>) -> Json<ListBenchRunsResponse> {
    let runs_map = state.inner.bench_runs.runs.read().await;
    let mut runs: Vec<BenchRunSummary> = runs_map
        .values()
        .map(|h| {
            let result = h.result.try_read().ok().and_then(|r| r.clone());
            BenchRunSummary {
                bench_run_id: h.bench_run_id.clone(),
                suite: h.suite.clone(),
                profile: h.profile.clone(),
                status: if result.is_some() {
                    if result.as_ref().unwrap().passed {
                        "passed".to_string()
                    } else {
                        "failed".to_string()
                    }
                } else {
                    "running".to_string()
                },
                total_tasks: result.as_ref().map(|r| r.total_tasks).unwrap_or(0),
                passed_tasks: result.as_ref().map(|r| r.passed_tasks).unwrap_or(0),
                failed_tasks: result.as_ref().map(|r| r.failed_tasks).unwrap_or(0),
                started_at: Some(h.started_at.clone()),
                finished_at: result.as_ref().map(|r| r.finished_at.clone()),
                evidence_root: result.map(|r| r.evidence_root.display().to_string()),
            }
        })
        .collect();
    // Sort newest first
    runs.sort_by(|a, b| b.bench_run_id.cmp(&a.bench_run_id));
    Json(ListBenchRunsResponse { runs })
}

#[utoipa::path(
    get,
    path = "/bench/runs/{bench_run_id}",
    tag = "benchmark",
    params(
        ("bench_run_id" = String, Path, description = "Benchmark run ID")
    ),
    responses(
        (status = 200, description = "Benchmark run detail", body = BenchRunDetailResponse),
        (status = 404, description = "Run not found")
    )
)]
pub(crate) async fn get_bench_run(
    State(state): State<ApiState>,
    AxumPath(bench_run_id): AxumPath<String>,
) -> Result<Json<BenchRunDetailResponse>, ApiError> {
    let runs_map = state.inner.bench_runs.runs.read().await;
    let handle = runs_map
        .get(&bench_run_id)
        .ok_or_else(|| ApiError::not_found("benchmark run not found"))?;

    let result_guard = handle.result.read().await;
    match result_guard.as_ref() {
        Some(report) => {
            let tasks = report.tasks.iter().map(task_to_response).collect();
            let summary_md = std::fs::read_to_string(report.evidence_root.join("summary.md")).ok();
            Ok(Json(BenchRunDetailResponse {
                bench_run_id: bench_run_id.clone(),
                suite: report.suite.clone(),
                profile: report.profile.clone(),
                status: if report.passed {
                    "passed".to_string()
                } else {
                    "failed".to_string()
                },
                started_at: Some(report.started_at.clone()),
                finished_at: Some(report.finished_at.clone()),
                total_tasks: report.total_tasks,
                passed_tasks: report.passed_tasks,
                failed_tasks: report.failed_tasks,
                evidence_root: Some(report.evidence_root.display().to_string()),
                summary_md,
                tasks,
            }))
        }
        None => Ok(Json(BenchRunDetailResponse {
            bench_run_id: handle.bench_run_id.clone(),
            suite: handle.suite.clone(),
            profile: handle.profile.clone(),
            status: "running".to_string(),
            started_at: Some(handle.started_at.clone()),
            finished_at: None,
            total_tasks: 0,
            passed_tasks: 0,
            failed_tasks: 0,
            evidence_root: None,
            summary_md: None,
            tasks: Vec::new(),
        })),
    }
}

#[utoipa::path(
    get,
    path = "/bench/runs/{bench_run_id}/tasks/{task_name}",
    tag = "benchmark",
    params(
        ("bench_run_id" = String, Path, description = "Benchmark run ID"),
        ("task_name" = String, Path, description = "Task name")
    ),
    responses(
        (status = 200, description = "Single task result", body = BenchTaskResultResponse),
        (status = 404, description = "Run or task not found")
    )
)]
pub(crate) async fn get_bench_task(
    State(state): State<ApiState>,
    AxumPath((bench_run_id, task_name)): AxumPath<(String, String)>,
) -> Result<Json<BenchTaskResultResponse>, ApiError> {
    let runs_map = state.inner.bench_runs.runs.read().await;
    let handle = runs_map
        .get(&bench_run_id)
        .ok_or_else(|| ApiError::not_found("benchmark run not found"))?;
    let result_guard = handle.result.read().await;
    let report = result_guard
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("benchmark run still in progress"))?;
    let task = report
        .tasks
        .iter()
        .find(|t| t.name == task_name)
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    Ok(Json(task_to_response(task)))
}

#[utoipa::path(
    get,
    path = "/bench/runs/{bench_run_id}/evidence/{*path}",
    tag = "benchmark",
    params(
        ("bench_run_id" = String, Path, description = "Benchmark run ID"),
        ("path" = String, Path, description = "Evidence file path relative to evidence root")
    ),
    responses(
        (status = 200, description = "Evidence file content", content_type = "application/octet-stream"),
        (status = 404, description = "File not found")
    )
)]
pub(crate) async fn get_bench_evidence(
    State(state): State<ApiState>,
    AxumPath((bench_run_id, path)): AxumPath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let runs_map = state.inner.bench_runs.runs.read().await;
    let handle = runs_map
        .get(&bench_run_id)
        .ok_or_else(|| ApiError::not_found("benchmark run not found"))?;
    let result_guard = handle.result.read().await;
    let report = result_guard
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("benchmark run still in progress"))?;

    let file_path = report.evidence_root.join(&path);
    // Safety: ensure path doesn't escape evidence root
    let canonical_root = report
        .evidence_root
        .canonicalize()
        .unwrap_or(report.evidence_root.clone());
    let canonical_file = file_path
        .canonicalize()
        .map_err(|_| ApiError::not_found("file not found"))?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err(ApiError::bad_request("path traversal detected"));
    }

    let content = tokio::fs::read(&canonical_file)
        .await
        .map_err(|_| ApiError::not_found("file not found"))?;

    let content_type = if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".md") {
        "text/markdown"
    } else if path.ends_with(".jsonl") {
        "application/x-ndjson"
    } else {
        "application/octet-stream"
    };

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, content_type)],
        content,
    ))
}

fn task_to_response(task: &crate::bench::BenchmarkTaskReport) -> BenchTaskResultResponse {
    BenchTaskResultResponse {
        name: task.name.clone(),
        outcome: match task.outcome {
            BenchmarkOutcome::Passed => "passed".to_string(),
            BenchmarkOutcome::Failed => "failed".to_string(),
        },
        termination_reason: task.termination_reason.clone(),
        steps: task.steps,
        tool_calls: task.tool_calls,
        tool_failures: task.tool_failures,
        artifacts: BenchArtifactsResponse {
            run_dir: task.artifacts.run_dir.display().to_string(),
            trace_jsonl: task.artifacts.trace_jsonl.display().to_string(),
            task_state_json: task.artifacts.task_state_json.display().to_string(),
            report_json: task.artifacts.report_json.display().to_string(),
        },
        output: task.output.clone(),
        check_results: task
            .check_results
            .iter()
            .map(|c| BenchCheckResultResponse {
                kind: c.kind.clone(),
                description: c.description.clone(),
                passed: c.passed,
                detail: c.detail.clone(),
            })
            .collect(),
        failures: task.failures.clone(),
    }
}
