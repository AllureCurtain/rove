//! Product Review runtime: start, stale check, cancel.

use super::*;

/// Start one product Review as a normal Runtime job without claiming the
/// conversation turn. The caller has already captured and persisted the
/// immutable target and won ProductStore idempotency.
pub(crate) async fn start_product_review_runtime(
    state: ApiState,
    review: ProductReview,
    product_workspace: ProductWorkspace,
    model_config: ProductSessionModelConfig,
    snapshot: Arc<ReviewTargetSnapshot>,
    state_root: PathBuf,
    max_steps: u32,
) -> Result<ProductReview, ApiError> {
    let workspace = open_product_workspace(&product_workspace)?;
    let (model, run_model_snapshot) =
        assemble_review_model(&state, &workspace, &model_config).await?;
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let request = CreateJobRequest {
        message: format!(
            "Review target {}. Inspect the immutable diff and submit one complete finding set.",
            snapshot.digest
        ),
        model: None,
        max_steps: Some(max_steps),
        agent: None,
        approval: Some(ApprovalPolicy::Never),
        resume: None,
        workspace: None,
        provider: None,
        product_session_id: None,
    };
    let mut review_workspace = workspace.clone();
    review_workspace.state_dir = state_root.clone();
    let mut config = state.inner.config.clone();
    config.source_summary.workspace_root = workspace.root.clone();
    config.source_summary.project_config_path = workspace.root.join(".rove/config.toml");
    config.source_summary.project_config_loaded = false;
    config.state.state_dir = state_root.clone();
    config.state.sqlite_path = state_root.join("state.sqlite");
    config.state.allow_external_paths = true;
    // A Review run retries under the same configured budget as any other run,
    // and runs the same silent-turn recovery policy. Review mode replaces model
    // text with the redaction marker unconditionally, so a Review turn is never
    // read as silent — not even a turn that produced no message at all.
    let provider_retry = config.runtime.recovery.retry_policy();
    let silent_turn_recovery = config.runtime.recovery.silent_turn_policy();
    let record = new_job_record(NewJobRecord {
        state: &state,
        workspace: review_workspace,
        config,
        request: &request,
        message_override: None,
        content_blocks: Vec::new(),
        session_id,
        job_id,
        resume_state: None,
        resumed_from_run_id: None,
        product_session_id: None,
        product_store: None,
        attachment_storage: None,
        product_model_config: None,
        run_model_snapshot: Some(run_model_snapshot.clone()),
    });
    let (engine, submission_store) = build_review_engine(
        model,
        &workspace,
        ReviewEngineOptions {
            snapshot: Arc::clone(&snapshot),
            review_id: review.id.to_string(),
            state_root: Some(&state_root),
            run_model_snapshot: Some(run_model_snapshot),
            provider_retry,
            silent_turn_recovery,
            max_steps,
        },
    )
    .map_err(|error| ApiError::internal(format!("review engine assembly failed: {error}")))?;
    std::fs::create_dir_all(&state_root)
        .map_err(|_| ApiError::internal("review state could not be created"))?;
    std::fs::write(
        state_root.join("target_snapshot.json"),
        serde_json::to_vec(&*snapshot)
            .map_err(|_| ApiError::internal("review target snapshot could not be encoded"))?,
    )
    .map_err(|_| ApiError::internal("review target snapshot could not be persisted"))?;
    let state_store = state_store_for_record(&record);
    state_store
        .index
        .initialize()
        .map_err(|_| ApiError::internal("review state index could not be initialized"))?;
    let run = state_store
        .start_run(record.session_id, record.job_id, record.run_id)
        .map_err(|_| ApiError::internal("review runtime run could not be started"))?;
    let store = state.product_store()?;
    let bound = store
        .bind_review_runtime(&review.id, record.session_id, record.job_id, record.run_id)
        .await?;
    state.inner.review_jobs.write().await.insert(
        record.job_id,
        Arc::new(ReviewExecution {
            review_id: review.id,
            snapshot,
            submission_store,
            product_store: store,
            started_at: Instant::now(),
            state_root,
        }),
    );
    start_job_supervisor(
        state,
        JobLaunch {
            record,
            engine,
            run,
            product_turn: None,
            startup_events: Vec::new(),
        },
    )
    .await;
    Ok(bound)
}

pub(crate) async fn assemble_review_model(
    state: &ApiState,
    workspace: &Workspace,
    model_config: &ProductSessionModelConfig,
) -> Result<(Box<dyn ModelClient>, RunModelSnapshot), ApiError> {
    if matches!(model_config.model.as_str(), "fake" | "fake-raw") {
        let snapshot = RunModelSnapshot {
            profile_id: "programmatic-fake".to_string(),
            provider_type: "fake".to_string(),
            wire_protocol: "fake".to_string(),
            endpoint: String::new(),
            model: model_config.model.clone(),
            reasoning: model_config.reasoning.as_str().to_string(),
            catalog_revision: "programmatic".to_string(),
            safe_config_digest: rove_runtime::context::stable_hash("programmatic-fake"),
        };
        let model = FakeModelClient::with_turns(
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
        );
        return Ok((Box::new(model), snapshot));
    }
    let profile_id = model_config.profile_id.as_ref().ok_or_else(|| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
            "product session has no Provider profile selection for Review",
        )
    })?;
    let catalog = state.provider_catalog().await?;
    let catalog_profile_id = product::provider_catalog::catalog_id(profile_id)?;
    let selection = rove_app_bootstrap::ModelSelection {
        profile_id: catalog_profile_id.clone(),
        model: model_config.model.clone(),
        reasoning: model_config.reasoning.as_str().to_string(),
        revision: catalog.revision().to_string(),
    };
    let snapshot = catalog
        .snapshot(&selection, &workspace.root)
        .map_err(product::provider_catalog::catalog_error)?;
    let profile = catalog
        .profile_config(&catalog_profile_id)
        .map_err(product::provider_catalog::catalog_error)?
        .clone();
    profile
        .resolve(&workspace.root, true, Some(&model_config.model))
        .map_err(|_| {
            ApiError::conflict_with_code(
                ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
                "the selected Provider credential is unavailable for Review",
            )
        })?;
    let mut config = state.inner.config.clone();
    config.provider.active = Some(catalog_profile_id.to_string());
    config.provider.profiles.clear();
    config
        .provider
        .profiles
        .insert(catalog_profile_id.to_string(), profile.clone());
    config.provider.fallback_profiles.clear();
    config.provider.fallback_models.clear();
    config.provider.model = model_config.model.clone();
    apply_product_reasoning(&mut config, &profile.provider_type, model_config.reasoning)?;
    let model = build_model_client_with_health(
        &config,
        model_config.model.clone(),
        state.inner.model_health.clone(),
    );
    Ok((model, snapshot))
}

pub(crate) async fn get_product_review_with_stale_check(
    state: &ApiState,
    review_id: &ProductReviewId,
) -> Result<ProductReview, ApiError> {
    let store = state.product_store()?;
    let review = store.get_review(review_id).await?;
    if !matches!(
        review.status,
        ProductReviewStatus::Pass
            | ProductReviewStatus::Findings
            | ProductReviewStatus::Partial
            | ProductReviewStatus::Stale
    ) {
        return Ok(review);
    }
    let product_context = store
        .get_session_context(&review.product_session_id)
        .await?;
    let workspace = open_product_workspace(&product_context.workspace)?;
    // Recompute from the durable target spec rather than relying on the
    // process-local ReviewExecution map. This keeps stale detection correct
    // after an API restart and treats an unavailable repository conservatively.
    let target_spec = review.target.spec.clone();
    let expected_digest = review.target.digest.clone();
    let current_digest = tokio::task::spawn_blocking(move || {
        capture_target(&workspace, target_spec).map(|snapshot| snapshot.digest)
    })
    .await
    .ok()
    .and_then(Result::ok);
    if current_digest.as_deref() != Some(expected_digest.as_str()) {
        return Ok(store.mark_review_needs_attention(review_id).await?);
    }
    Ok(review)
}

pub(crate) async fn cancel_product_review_runtime(
    state: &ApiState,
    review_id: &ProductReviewId,
) -> Result<ProductReview, ApiError> {
    let store = state.product_store()?;
    let current = store.get_review(review_id).await?;
    if current.status.is_terminal() {
        return Ok(current);
    }
    let has_live_execution = state
        .inner
        .review_jobs
        .read()
        .await
        .values()
        .any(|execution| &execution.review_id == review_id);
    if !has_live_execution {
        return Ok(store.cancel_review(review_id).await?);
    }
    let Some(job_id) = current.job_id else {
        return Ok(store.cancel_review(review_id).await?);
    };
    let Some(record) = live_job(state, job_id).await else {
        return Ok(store.cancel_review(review_id).await?);
    };
    record.cancel_token.cancel();
    wait_for_job_completion(&record).await;
    Ok(store.get_review(review_id).await?)
}
