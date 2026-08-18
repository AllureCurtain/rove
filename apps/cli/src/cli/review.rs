use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ModelSelection, ProviderCatalog, ProviderCatalogService,
    ProviderProfileId, RunModelSnapshot, build_review_engine, try_build_model_client,
};
use rove_models::fake::{FakeModelClient, FakeTurn};
use rove_runtime::review::{
    ReviewConclusion, ReviewResult, ReviewRuntimeEvidence, ReviewStats, ReviewTargetSpec,
    ReviewTargetSummary, ReviewUnchecked, apply_runtime_outcome, capture_target,
    finalize_result_with_evidence, resolve_external_state_root,
};
use rove_runtime::runtime_identity::workspace_fingerprint;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{JobId, RunId, RunMode, RunRequest, SessionId, TerminationReason};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

use super::args::ReviewFormat;

/// Run the CLI Review contract. Returns the documented process exit code.
pub async fn run(
    cwd: Option<PathBuf>,
    model: Option<String>,
    max_steps: Option<u32>,
    base: Option<String>,
    commit: Option<String>,
    format: ReviewFormat,
) -> anyhow::Result<i32> {
    let start = Instant::now();
    let cwd = cwd.unwrap_or(std::env::current_dir()?);
    let workspace = Workspace::detect(&cwd)?;
    let spec = match (base, commit) {
        (Some(revision), None) => ReviewTargetSpec::base(revision),
        (None, Some(revision)) => ReviewTargetSpec::commit(revision),
        (None, None) => ReviewTargetSpec::default(),
        (Some(_), Some(_)) => unreachable!("clap enforces conflicts"),
    };
    let review_id = format!("rev_{}", RunId::new());
    let session_id = SessionId::new();
    let run_id = RunId::new();
    let snapshot = match capture_target(&workspace, spec.clone()) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!("Review target is unavailable: {error}");
            let result = unavailable_target_result(
                &review_id,
                run_id,
                session_id,
                &workspace,
                spec,
                start.elapsed().as_millis() as u64,
            );
            render(&result, format)?;
            return Ok(2);
        }
    };
    let job_id = JobId::new();
    let state_root = resolve_external_state_root(
        &workspace,
        Some(
            &std::env::temp_dir()
                .join("rove-review-state")
                .join(&review_id),
        ),
    )?;
    std::fs::create_dir_all(&state_root)?;
    tokio::fs::write(
        state_root.join("target_snapshot.json"),
        serde_json::to_vec(&snapshot)?,
    )
    .await?;
    let mut review_workspace = workspace.clone();
    review_workspace.state_dir = state_root.clone();
    let state_store =
        StateStore::with_index_path(&state_root, state_root.join("state.sqlite"), 5_000);
    state_store.index.initialize()?;
    let run = state_store.start_run(session_id, job_id, run_id)?;

    let (model, run_model_snapshot) = match assemble_review_model(&workspace, model.as_deref()) {
        Ok(assembly) => assembly,
        Err(error) => {
            tracing::warn!("Review Provider is unavailable: {error}");
            let result = unavailable_result(
                &review_id,
                run_id,
                session_id,
                snapshot.clone(),
                start.elapsed().as_millis() as u64,
                ReviewConclusion::Unavailable,
                "provider_unavailable",
            );
            persist_result(&state_root, &result).await?;
            render(&result, format)?;
            return Ok(2);
        }
    };
    let (engine, submission_store) = match build_review_engine(
        model,
        &workspace,
        Arc::new(snapshot.clone()),
        &review_id,
        Some(&state_root),
        Some(run_model_snapshot),
        max_steps.unwrap_or(8),
    ) {
        Ok(assembly) => assembly,
        Err(error) => {
            tracing::warn!("Review engine assembly failed: {error}");
            let result = unavailable_result(
                &review_id,
                run_id,
                session_id,
                snapshot.clone(),
                start.elapsed().as_millis() as u64,
                ReviewConclusion::Error,
                "review_engine_unavailable",
            );
            persist_result(&state_root, &result).await?;
            render(&result, format)?;
            return Ok(3);
        }
    };
    let user_message = format!(
        "Review target {}. Inspect the immutable diff and submit findings.",
        snapshot.digest
    );
    let request = RunRequest {
        session_id,
        job_id,
        run_id,
        user_message: user_message.clone(),
        resume_state: None,
    };
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal_listener = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancel.cancel();
        }
    });
    let mut stream =
        engine.run_with_cancel(request, Some(run.trace_writer.clone()), cancel.clone());
    let stream_runtime_identity = stream.runtime_identity().clone();
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        user_message,
        None,
        Some(stream_runtime_identity.clone()),
    );
    if engine.run_mode() != RunMode::Review {
        recorder.set_agent_profile(stream.agent_profile().cloned());
    }
    let mut termination = TerminationReason::Error;
    while let Some(event) = stream.next().await {
        if let rove_runtime::events::StreamEvent::RunCompleted { reason, .. } = &event {
            termination = reason.clone();
        }
        // The Engine already protects its TraceWriter. Apply the same
        // projection to the artifact recorder so task state and report cannot
        // diverge from the trace when this CLI owns the supervisor.
        let persisted = if engine.run_mode() == RunMode::Review {
            event.redacted_for_review_persistence()
        } else {
            event
        };
        recorder.record_event(&persisted, &state_store).await;
    }
    recorder
        .finalize(
            &state_store,
            &review_workspace,
            engine.model_id(),
            &run.run_dir,
        )
        .await;
    let runtime_durable = run.run_dir.join("trace.jsonl").is_file()
        && run.run_dir.join("task_state.json").is_file()
        && run.run_dir.join("report.json").is_file();
    signal_listener.abort();
    let cancelled = cancel.is_cancelled();
    let stale = snapshot.is_stale(&workspace).unwrap_or(true);
    let mut result = finalize_result_with_evidence(
        &review_id,
        run_id.to_string(),
        session_id.to_string(),
        snapshot,
        submission_store.get(),
        stale,
        cancelled,
        start.elapsed().as_millis() as u64,
        ReviewRuntimeEvidence::from(&stream_runtime_identity),
    );
    apply_runtime_outcome(&mut result, &termination, runtime_durable);
    persist_result(&state_root, &result).await?;
    render(&result, format)?;
    let code = if cancelled {
        130
    } else {
        match result.conclusion {
            ReviewConclusion::Pass | ReviewConclusion::Findings => 0,
            ReviewConclusion::Partial
            | ReviewConclusion::Stale
            | ReviewConclusion::Unavailable
            | ReviewConclusion::Cancelled => 2,
            ReviewConclusion::Error => 3,
        }
    };
    Ok(code)
}

fn unavailable_result(
    review_id: &str,
    run_id: RunId,
    session_id: SessionId,
    snapshot: rove_runtime::review::ReviewTargetSnapshot,
    duration_ms: u64,
    conclusion: ReviewConclusion,
    warning: &str,
) -> rove_runtime::review::ReviewResult {
    let mut result = finalize_result_with_evidence(
        review_id,
        run_id.to_string(),
        session_id.to_string(),
        snapshot,
        None,
        false,
        false,
        duration_ms,
        ReviewRuntimeEvidence::default(),
    );
    result.conclusion = conclusion;
    result.warnings.push(warning.to_string());
    result.warnings.sort();
    result.warnings.dedup();
    result
}

fn unavailable_target_result(
    review_id: &str,
    run_id: RunId,
    session_id: SessionId,
    workspace: &Workspace,
    spec: ReviewTargetSpec,
    duration_ms: u64,
) -> ReviewResult {
    let workspace_digest = workspace_fingerprint(workspace);
    let digest = rove_runtime::context::stable_hash(&format!(
        "review-target-unavailable:{workspace_digest}:{}",
        serde_json::to_string(&spec).unwrap_or_default()
    ));
    ReviewResult {
        schema_version: rove_runtime::review::REVIEW_RESULT_SCHEMA_VERSION,
        review_id: review_id.to_string(),
        run_id: run_id.to_string(),
        session_id: session_id.to_string(),
        target: ReviewTargetSummary {
            schema_version: 1,
            spec,
            workspace_kind: workspace.kind.clone(),
            workspace_digest,
            resolved_base: None,
            captured_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            entries: 0,
            entries_truncated: 0,
            digest,
        },
        conclusion: ReviewConclusion::Unavailable,
        findings: Vec::new(),
        stats: ReviewStats {
            duration_ms,
            concurrency_limit: 1,
            ..ReviewStats::default()
        },
        unchecked: vec![ReviewUnchecked {
            reason: "review_target_unavailable".to_string(),
            paths: Vec::new(),
        }],
        model_snapshot: None,
        capability_snapshot_id: None,
        execution_environment: None,
        execution_capabilities: None,
        warnings: vec!["review_target_unavailable".to_string()],
    }
}

async fn persist_result(
    state_root: &std::path::Path,
    result: &rove_runtime::review::ReviewResult,
) -> anyhow::Result<()> {
    let result_json = serde_json::to_vec_pretty(result)?;
    tokio::fs::write(state_root.join("review.json"), result_json).await?;
    Ok(())
}

fn assemble_review_model(
    workspace: &Workspace,
    requested_model: Option<&str>,
) -> anyhow::Result<(Box<dyn rove_models::ModelClient>, RunModelSnapshot)> {
    let config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            model: requested_model.map(str::to_string),
            max_steps: None,
            agent_selector: None,
            api_bind_addr: None,
            // Review never activates project-owned configuration, hooks, or
            // MCP. Provider credentials come only from the user catalog.
            trust_project: false,
        },
    )?;
    let requested_fake = requested_model.is_some_and(|model| matches!(model, "fake" | "fake-raw"));
    let catalog_service = ProviderCatalogService::discover();
    let catalog = catalog_service.load()?;
    let programmatic_fake = requested_fake
        || (config.provider.model == "fake"
            && config
                .provider
                .profiles
                .values()
                .any(|profile| profile.provider_type == "fake")
            && catalog.profiles().is_empty());
    if programmatic_fake {
        return Ok((
            Box::new(review_fake_model()),
            RunModelSnapshot {
                profile_id: "programmatic-fake".to_string(),
                provider_type: "fake".to_string(),
                wire_protocol: "fake".to_string(),
                endpoint: String::new(),
                model: requested_model.unwrap_or("fake").to_string(),
                reasoning: "default".to_string(),
                catalog_revision: "programmatic".to_string(),
                safe_config_digest: rove_runtime::context::stable_hash("programmatic-fake"),
            },
        ));
    }

    let selection = selection_from_config_for_review(&config, &catalog)?;
    let snapshot = catalog.snapshot(&selection, &workspace.root)?;
    let profile = catalog.profile_config(&selection.profile_id)?.clone();
    profile.resolve(&catalog_service.paths().root, true, Some(&selection.model))?;
    let mut provider_config = config;
    provider_config.provider.active = Some(selection.profile_id.to_string());
    provider_config.provider.model = selection.model.clone();
    provider_config.provider.profiles = catalog.document().provider.profiles.clone();
    for profile in provider_config.provider.profiles.values_mut() {
        profile.rebase_secret_paths(&catalog_service.paths().root);
    }
    provider_config.provider.fallback_profiles =
        catalog.document().provider.fallback_profiles.clone();
    provider_config.source_summary.user_config_loaded = true;
    provider_config.source_summary.user_config_path = catalog_service.paths().config_file.clone();

    let model: Box<dyn rove_models::ModelClient> = if profile.provider_type == "fake" {
        Box::new(review_fake_model())
    } else {
        try_build_model_client(&provider_config, selection.model.clone()).map_err(|error| {
            anyhow::anyhow!("provider_unavailable: Review model is unavailable: {error}")
        })?
    };
    Ok((model, snapshot))
}

fn review_fake_model() -> FakeModelClient {
    FakeModelClient::with_turns(
        "Review complete".to_string(),
        vec![
            FakeTurn::ToolUse {
                id: "review-diff".to_string(),
                name: "review_target_diff".to_string(),
                args: serde_json::json!({}),
            },
            FakeTurn::ToolUse {
                id: "review-submit".to_string(),
                name: "review_submit_findings".to_string(),
                args: serde_json::json!({"findings": []}),
            },
        ],
    )
}

fn selection_from_config_for_review(
    config: &AppConfig,
    catalog: &ProviderCatalog,
) -> anyhow::Result<ModelSelection> {
    if let Some(active) = config.provider.active.as_deref() {
        let profile_id = ProviderProfileId::new(active.to_string())?;
        if catalog.profile_config(&profile_id).is_ok() {
            return Ok(ModelSelection {
                profile_id,
                model: config.provider.model.clone(),
                reasoning: catalog.document().model.reasoning.clone(),
                revision: catalog.revision().to_string(),
            });
        }
    }
    catalog.default_selection().map_err(anyhow::Error::from)
}

fn render(result: &rove_runtime::review::ReviewResult, format: ReviewFormat) -> anyhow::Result<()> {
    match format {
        ReviewFormat::Json => println!("{}", serde_json::to_string_pretty(result)?),
        ReviewFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "schema_version": result.schema_version,
                    "review_id": result.review_id,
                    "conclusion": result.conclusion,
                    "target_digest": result.target.digest
                }))?
            );
            for finding in &result.findings {
                println!("{}", serde_json::to_string(finding)?);
            }
        }
        ReviewFormat::Text => {
            println!(
                "Review {}: {:?} (target {})",
                result.review_id, result.conclusion, result.target.digest
            );
            if result.findings.is_empty() {
                println!("No findings.");
            } else {
                for finding in &result.findings {
                    println!(
                        "[{:?}] {}:{} {}",
                        finding.severity, finding.path, finding.location.start_line, finding.title
                    );
                }
            }
            if !result.unchecked.is_empty() {
                println!("Unchecked scope: {}", result.unchecked.len());
            }
        }
    }
    Ok(())
}
