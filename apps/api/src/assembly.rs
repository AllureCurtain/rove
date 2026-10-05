//! Engine assembly, workspace/config resolution, background maintenance tasks.

use super::*;

pub(crate) async fn assemble_job_engine(
    state: &ApiState,
    message: &str,
    req: &CreateJobRequest,
    record: Arc<JobRecord>,
    approval_policy: ApprovalPolicy,
) -> anyhow::Result<Engine> {
    let mut config = record.config.clone();
    if record.product_session_id.is_some() {
        config.pinned_mcp_catalog = Some(config.workspace_bounded_mcp_config_path()?);
    }
    let model_id = record
        .product_model_config
        .as_ref()
        .map(|model_config| model_config.model.clone())
        .or_else(|| req.model.clone())
        .unwrap_or_else(|| config.provider.model.clone());
    let model: Box<dyn ModelClient> = match model_id.as_str() {
        "fake" => Box::new(FakeModelClient::new(format!("fake response: {message}"))),
        "fake-raw" => Box::new(FakeModelClient::with_compatibility_text(
            message.to_string(),
        )),
        _ => build_model_client_with_health(&config, model_id, state.inner.model_health.clone()),
    };

    let workspace = record.workspace.clone();
    let state_store = state_store_for_record(&record);
    let input_provider: Arc<dyn UserInputProvider> = Arc::new(ApiInputProvider {
        record: record.clone(),
        index: state_store.index.clone(),
    });
    let approval_provider = (approval_policy == ApprovalPolicy::Ask).then(|| {
        Arc::new(ApiApprovalProvider {
            record: record.clone(),
            index: state_store.index.clone(),
        }) as Arc<dyn ToolApprovalProvider>
    });

    let engine = build_engine(EngineOptions {
        model,
        workspace: &workspace,
        config: &config,
        max_steps: record
            .product_model_config
            .as_ref()
            .map(|model_config| model_config.max_steps)
            .or(req.max_steps)
            .unwrap_or(config.runtime.max_steps),
        agent_selector: req.agent.clone(),
        approval_policy,
        input_provider: Some(input_provider),
        approval_provider,
        environment: None,
        run_model_snapshot: record.run_model_snapshot.clone(),
    })
    .await?;
    let mcp_config_path = config.workspace_bounded_mcp_config_path()?;
    state.inner.mcp_health.write().await.insert(
        mcp_config_path,
        engine.runtime_identity().mcp_servers.clone(),
    );
    Ok(engine)
}

/// Assemble the Engine `POST /product/sessions/{id}/compact` compacts with.
///
/// Compaction runs no tools, dispatches no model turn, and starts no run, so
/// this build reuses the shared product assembly but omits the approval/input
/// providers: nothing on this path can be authorized to do anything. The
/// registry is still built the ordinary way so the context manager, budget, and
/// compaction threshold match what a turn would use.
pub(crate) async fn assemble_compaction_engine(
    state: &ApiState,
    workspace: &Workspace,
    mut config: AppConfig,
    run_model_snapshot: &RunModelSnapshot,
    max_steps: u32,
) -> anyhow::Result<Engine> {
    let mcp_config_path = config.workspace_bounded_mcp_config_path()?;
    config.pinned_mcp_catalog = Some(mcp_config_path);
    let model_id = run_model_snapshot.model.clone();
    let model: Box<dyn ModelClient> = match model_id.as_str() {
        // The fake profiles answer with one fixed script. A summary call wants
        // exactly that: plain text, never a compatibility tool-call parse.
        "fake" | "fake-raw" => Box::new(FakeModelClient::new(
            FAKE_COMPACTION_SUMMARY_SCRIPT.to_string(),
        )),
        _ => build_model_client_with_health(&config, model_id, state.inner.model_health.clone()),
    };
    build_engine(EngineOptions {
        model,
        workspace,
        config: &config,
        max_steps,
        agent_selector: None,
        // No tool call can run here, so no approval can be requested. `Never`
        // states that rather than inheriting a policy this path cannot honour.
        approval_policy: ApprovalPolicy::Never,
        input_provider: None,
        approval_provider: None,
        environment: None,
        run_model_snapshot: Some(run_model_snapshot.clone()),
    })
    .await
}

/// Scripted summary the `fake`/`fake-raw` profile answers a compaction with.
///
/// Deliberately not the turn script: a test can then tell a summary produced by
/// the compaction call apart from ordinary model output.
pub(crate) const FAKE_COMPACTION_SUMMARY_SCRIPT: &str =
    "Goal: manual compaction of the product session history";

pub(crate) fn state_store_for_api(state: &ApiState) -> StateStore {
    state_store_for_parts(&state.inner.workspace, &state.inner.config)
}

pub(crate) fn state_store_for_record(record: &JobRecord) -> StateStore {
    state_store_for_parts(&record.workspace, &record.config)
}

pub(crate) fn workspace_and_config_for_create_job(
    state: &ApiState,
    req: &CreateJobRequest,
) -> Result<(Workspace, AppConfig), ApiError> {
    let (workspace, mut config) = workspace_for_create_job(state, req.workspace.as_ref())?;
    if let Some(profile) = &req.provider {
        apply_provider_profile(&mut config, profile, req.model.as_deref())?;
    }
    Ok((workspace, config))
}

pub(crate) async fn workspace_and_config_for_product_job(
    state: &ApiState,
    req: &CreateJobRequest,
    product_workspace: &ProductWorkspace,
    store: &Arc<dyn ProductStore>,
    model_config: &ProductSessionModelConfig,
    resume_expected: bool,
) -> Result<(Workspace, AppConfig, RunModelSnapshot), ApiError> {
    if req.model.is_some()
        || req.max_steps.is_some()
        || req.approval.is_some()
        || req.provider.is_some()
        || req.resume.is_some()
    {
        return Err(ApiError::bad_request(
            "product job requests must leave model, reasoning, approval, provider, max_steps, and resume to the server",
        ));
    }
    product_session_execution_config(
        state,
        product_workspace,
        store,
        model_config,
        req.workspace.as_ref(),
        resume_expected,
    )
    .await
}

/// Resolve the workspace, configuration, and immutable model snapshot that every
/// product-session runtime action runs against.
///
/// Shared by a turn and by manual compaction so both open the same root, apply
/// the same trust resolution, and pin the same Provider selection. The hint is
/// the optional workspace echo a job request carries; compaction has no request
/// body and passes `None`.
pub(crate) async fn product_session_execution_config(
    state: &ApiState,
    product_workspace: &ProductWorkspace,
    store: &Arc<dyn ProductStore>,
    model_config: &ProductSessionModelConfig,
    workspace_hint: Option<&CreateJobWorkspace>,
    resume_expected: bool,
) -> Result<(Workspace, AppConfig, RunModelSnapshot), ApiError> {
    let workspace = open_product_workspace(product_workspace)?;
    if let Some(requested) = workspace_hint {
        validate_product_workspace_hint(requested, product_workspace, &workspace)?;
    }
    let (workspace, mut config) = rebased_workspace_config(state, workspace)?;
    let authority = state.project_trust()?;
    let catalog = state.provider_catalog().await.map_err(|error| {
        if resume_expected {
            ApiError::conflict_with_code(
                ProductErrorCode::ProviderUnavailableForResume.as_str(),
                "the Provider catalog required to resume this session is unavailable",
            )
        } else {
            error
        }
    })?;
    let provider_selector = product::trust::product_provider_capability_selector(
        store,
        &catalog,
        &product_workspace.id,
        &workspace.root,
    )
    .await?;
    let trust = product::trust::resolve_product_workspace_trust(
        &authority,
        &workspace.root,
        workspace.kind.clone(),
        &provider_selector,
    )
    .await?;
    if trust.state == ProjectActivationState::Revoked {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProjectTrustRequired.as_str(),
            "project trust was revoked for this workspace",
        ));
    }
    config.apply_project_trust_resolution(trust);
    if let Some(profile_id) = model_config.profile_id.as_ref() {
        let provider_identity = store.get_provider_profile(profile_id).await?;
        if provider_identity.provider_type != ProductProviderType::Fake
            && !config.project_capability_allowed(rove_app_bootstrap::CAP_PROVIDER_CREDENTIALS)
        {
            return Err(ApiError::conflict_with_code(
                ProductErrorCode::ProjectTrustRequired.as_str(),
                "project trust must grant provider_credentials before using the selected Provider",
            ));
        }
    }
    let Some(profile_id) = model_config.profile_id.as_ref() else {
        if matches!(model_config.model.as_str(), "fake" | "fake-raw")
            && config
                .provider
                .profiles
                .values()
                .any(|profile| profile.provider_type == "fake")
        {
            config.provider.model = model_config.model.clone();
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
            return Ok((workspace, config, snapshot));
        }
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
            "product session has no Provider profile selection; configure ~/.rove/config.toml and select a profile",
        ));
    };
    let catalog_profile_id = product::provider_catalog::catalog_id(profile_id)?;
    let selection = rove_app_bootstrap::ModelSelection {
        profile_id: catalog_profile_id.clone(),
        model: model_config.model.clone(),
        reasoning: model_config.reasoning.as_str().to_string(),
        revision: catalog.revision().to_string(),
    };
    let run_model_snapshot = catalog
        .snapshot(&selection, &workspace.root)
        .map_err(|error| {
            if resume_expected {
                ApiError::conflict_with_code(
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the Provider profile required to resume this session is unavailable",
                )
            } else {
                product::provider_catalog::catalog_error(error)
            }
        })?;
    let profile = catalog
        .profile_config(&catalog_profile_id)
        .map_err(|error| {
            if resume_expected {
                ApiError::conflict_with_code(
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the Provider profile required to resume this session is unavailable",
                )
            } else {
                product::provider_catalog::catalog_error(error)
            }
        })?
        .clone();
    profile
        .resolve(&workspace.root, true, Some(&model_config.model))
        .map_err(|_| {
            let (code, message) = if resume_expected {
                (
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the credential required to resume this session is unavailable",
                )
            } else {
                (
                    ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
                    "the selected Provider credential is unavailable",
                )
            };
            ApiError::conflict_with_code(code, message)
        })?;
    config.provider.active = Some(catalog_profile_id.to_string());
    config.provider.profiles.clear();
    config
        .provider
        .profiles
        .insert(catalog_profile_id.to_string(), profile.clone());
    config.provider.fallback_profiles.clear();
    config.provider.fallback_models.clear();
    config.provider.model = model_config.model.clone();
    let provider_type = profile.provider_type;
    apply_product_reasoning(&mut config, &provider_type, model_config.reasoning)?;
    Ok((workspace, config, run_model_snapshot))
}

pub(crate) fn apply_product_reasoning(
    config: &mut AppConfig,
    provider_type: &str,
    reasoning: ProductReasoningPreference,
) -> Result<(), ApiError> {
    if reasoning == ProductReasoningPreference::Default {
        return Ok(());
    }
    if provider_type != "openai-responses" {
        return Err(ApiError::bad_request(
            "the selected provider does not support reasoning controls; choose default reasoning",
        ));
    }
    let active = config
        .provider
        .active
        .clone()
        .ok_or_else(|| ApiError::bad_request("the selected provider has no active protocol"))?;
    let profile =
        config.provider.profiles.get_mut(&active).ok_or_else(|| {
            ApiError::bad_request("the selected provider protocol is unavailable")
        })?;
    profile.protocol_options = serde_json::json!({
        "reasoning_effort": reasoning.as_str(),
    });
    Ok(())
}

pub(crate) fn open_product_workspace(
    product_workspace: &ProductWorkspace,
) -> Result<Workspace, ApiError> {
    let workspace = match product_workspace.kind {
        ProductWorkspaceKind::Folder => Workspace::open_folder(&product_workspace.canonical_root),
        ProductWorkspaceKind::Repo => Workspace::open_repo(&product_workspace.canonical_root),
    }
    .map_err(|error| {
        tracing::warn!(product_workspace_id = %product_workspace.id, "failed to open catalog workspace: {error}");
        ApiError::from(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateMissing,
            "the product session workspace is unavailable",
        ))
    })?;
    if workspace.root != product_workspace.canonical_root {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductBindingCorrupt,
            "the product workspace canonical root no longer matches its catalog binding",
        )
        .into());
    }
    Ok(workspace)
}

pub(crate) fn validate_product_workspace_hint(
    requested: &CreateJobWorkspace,
    product_workspace: &ProductWorkspace,
    server_workspace: &Workspace,
) -> Result<(), ApiError> {
    let kind_matches = matches!(
        (requested.kind, product_workspace.kind),
        (CreateJobWorkspaceKind::Folder, ProductWorkspaceKind::Folder)
            | (CreateJobWorkspaceKind::Repo, ProductWorkspaceKind::Repo)
    );
    if !kind_matches
        || requested.name.is_some()
        || requested.base.is_some()
        || requested.root.is_none()
    {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not match the product session workspace",
        ));
    }
    let root = requested.root.as_ref().ok_or_else(|| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not include the product session workspace root",
        )
    })?;
    let hinted_workspace = match requested.kind {
        CreateJobWorkspaceKind::Folder => Workspace::open_folder(root),
        CreateJobWorkspaceKind::Repo => Workspace::open_repo(root),
        CreateJobWorkspaceKind::Task => unreachable!("task cannot match a product workspace"),
    }
    .map_err(|_| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not resolve to the product session workspace",
        )
    })?;
    if hinted_workspace.root != server_workspace.root {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not match the product session workspace",
        ));
    }
    Ok(())
}

pub(crate) fn workspace_for_create_job(
    state: &ApiState,
    requested: Option<&CreateJobWorkspace>,
) -> Result<(Workspace, AppConfig), ApiError> {
    let Some(requested) = requested else {
        return Ok((state.inner.workspace.clone(), state.inner.config.clone()));
    };

    match requested.kind {
        CreateJobWorkspaceKind::Task => {
            if requested.root.is_some() {
                return Err(ApiError::bad_request(
                    "task workspace uses name/base, not root",
                ));
            }
            let name = requested
                .name
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("task workspace name is required"))?;
            let base = requested
                .base
                .clone()
                .unwrap_or_else(|| state.inner.config.state_dir().join("tasks"));
            let workspace = Workspace::task(&base, name)
                .map_err(|err| ApiError::bad_request(err.to_string()))?;
            rebased_workspace_config(state, workspace)
        }
        CreateJobWorkspaceKind::Folder | CreateJobWorkspaceKind::Repo => {
            if requested.name.is_some() || requested.base.is_some() {
                return Err(ApiError::bad_request(
                    "folder/repo workspace uses root, not name/base",
                ));
            }
            let root = requested
                .root
                .as_ref()
                .ok_or_else(|| ApiError::bad_request("folder/repo workspace root is required"))?;
            let workspace = match requested.kind {
                CreateJobWorkspaceKind::Folder => Workspace::open_folder(root),
                CreateJobWorkspaceKind::Repo => Workspace::open_repo(root),
                CreateJobWorkspaceKind::Task => unreachable!("task handled above"),
            }
            .map_err(|err| ApiError::bad_request(err.to_string()))?;
            rebased_workspace_config(state, workspace)
        }
    }
}

pub(crate) fn rebased_workspace_config(
    state: &ApiState,
    mut workspace: Workspace,
) -> Result<(Workspace, AppConfig), ApiError> {
    let mut config = state.inner.config.clone();
    config.rebase_to_workspace(&workspace.root);
    workspace.state_dir = config.state_dir();
    workspace
        .ensure_state_dir()
        .map_err(|err| ApiError::internal(err.to_string()))?;
    if config.state_dir_is_contract_managed() {
        config
            .ensure_contract_layout()
            .map_err(|err| ApiError::internal(err.to_string()))?;
    }
    Ok((workspace, config))
}

pub(crate) fn state_store_for_parts(workspace: &Workspace, config: &AppConfig) -> StateStore {
    StateStore::with_index_path(
        &workspace.state_dir,
        config.sqlite_path(),
        config.state.sqlite_busy_timeout_ms,
    )
}

/// How long a startup backfill may run before the API stops waiting on it.
///
/// The bound exists for the pathological case — an index locked by another
/// process, or a rebuild over a run history far larger than anything we test.
/// Import is idempotent and per-artifact, so abandoning a partial rebuild is
/// safe: the next boot resumes from whatever landed.
pub(crate) const STATE_INDEX_BACKFILL_TIMEOUT: Duration = Duration::from_secs(60);

/// Put back product sessions whose runs survived but whose catalog rows did not.
///
/// The product half of startup recovery. Shares the
/// index backfill's shape — off the boot path, bounded, warn-on-failure — for
/// the same reason: a session list that is briefly incomplete is recoverable,
/// an API that refuses to start is not.
pub(crate) fn spawn_product_ownership_recovery(
    store: Arc<dyn ProductStore>,
    workspace: &Workspace,
    config: &AppConfig,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!(
            "no async runtime at API construction; skipping product ownership recovery"
        );
        return;
    };
    let runs_dirs = product::ownership::candidate_runs_dirs(
        config.user_state_roots.as_ref().map(|roots| roots.root()),
        &workspace.state_dir,
    );
    if runs_dirs.is_empty() {
        return;
    }
    handle.spawn(async move {
        let sweep = product::ownership::recover_product_ownership(&store, &runs_dirs);
        match tokio::time::timeout(STATE_INDEX_BACKFILL_TIMEOUT, sweep).await {
            Ok(summary) if summary.sessions_recovered > 0 || summary.sessions_failed > 0 => {
                tracing::info!(
                    records_found = summary.records_found,
                    sessions_found = summary.sessions_found,
                    sessions_recovered = summary.sessions_recovered,
                    runs_recovered = summary.runs_recovered,
                    sessions_failed = summary.sessions_failed,
                    "recovered product sessions from run directories"
                );
            }
            Ok(summary) => tracing::debug!(
                records_found = summary.records_found,
                sessions_found = summary.sessions_found,
                "product catalog already covers every run on disk"
            ),
            Err(_) => tracing::warn!(
                timeout_secs = STATE_INDEX_BACKFILL_TIMEOUT.as_secs(),
                "product ownership recovery did not finish in time; the session list may be \
                 incomplete until the next start"
            ),
        }
    });
}

/// Reclaim unreferenced attachment payloads, off the boot path.
///
/// The same shape as [`spawn_product_ownership_recovery`] — fire-and-forget,
/// bounded per run, warn-on-failure — for the same reason: a payload that
/// lingers costs disk, while an API that refuses to start costs the product. The
/// first run waits [`ATTACHMENT_CLEANUP_FIRST_DELAY_SECONDS`] so a start never
/// queues behind a filesystem walk.
///
/// Each run is bounded by [`product::attachments::AttachmentCleanupLimits`], so
/// the work is proportional to the limits rather than to the data root. A run
/// that hits a bound is not "incomplete" in a way that needs resuming: the next
/// run reads the same ordered sets and continues past what this one finished,
/// and nothing is deleted that a durable row still names.
///
/// The task is cancelled by the state's shutdown token, so a test or a clean
/// shutdown does not leave an interval running.
pub(crate) fn spawn_attachment_cleanup(
    store: Arc<dyn ProductStore>,
    storage: product::attachments::AttachmentStorage,
    shutdown: CancellationToken,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!("no async runtime at API construction; skipping attachment cleanup");
        return;
    };
    handle.spawn(async move {
        let start = tokio::time::Instant::now()
            + Duration::from_secs(product::attachments::ATTACHMENT_CLEANUP_FIRST_DELAY_SECONDS);
        let period = Duration::from_secs(product::attachments::ATTACHMENT_CLEANUP_INTERVAL_SECONDS);
        let mut ticker = tokio::time::interval_at(start, period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if shutdown.is_cancelled() {
                return;
            }
            let outcome = product::attachments::run_attachment_cleanup(
                store.as_ref(),
                &storage,
                product::attachments::AttachmentCleanupLimits::default(),
            )
            .await;
            // Counts only. An attachment's display name, its bytes, and its path
            // never reach a log line.
            tracing::debug!(
                expired_rows = outcome.expired_rows,
                removed_expired_payloads = outcome.removed_expired_payloads,
                removed_orphan_payloads = outcome.removed_orphan_payloads,
                reclaimed_partials = outcome.reclaimed_partials,
                failures = outcome.failures,
                "attachment cleanup run finished"
            );
        }
    });
}

/// Heal the runtime index from the run directories, off the boot path.
///
/// The filesystem is the record and the index is a
/// rebuildable cache, so a deleted or truncated `state.sqlite` has to recover
/// on its own rather than wait for someone to notice and run `rove repair`.
/// Deliberately fire-and-forget: a failed or slow rebuild degrades history
/// lookups, and serving requests without history beats refusing to boot.
pub(crate) fn spawn_state_index_backfill(workspace: &Workspace, config: &AppConfig) {
    // The constructor is synchronous and is also called from tests that never
    // enter a runtime, so the spawn is conditional rather than assumed.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!("no async runtime at API construction; skipping state index backfill");
        return;
    };
    let state_dir = workspace.state_dir.clone();
    let db_path = config.sqlite_path();
    let busy_timeout_ms = config.state.sqlite_busy_timeout_ms;
    handle.spawn(async move {
        let store = StateStore::with_index_path(&state_dir, db_path, busy_timeout_ms);
        match tokio::time::timeout(STATE_INDEX_BACKFILL_TIMEOUT, store.backfill_missing_runs())
            .await
        {
            Ok(Ok(result)) => match result.repair {
                Some(repair) => tracing::info!(
                    runs_on_disk = result.runs_on_disk,
                    runs_missing = result.runs_missing,
                    task_states = repair.task_state_count,
                    events = repair.event_count,
                    reports = repair.report_count,
                    corrupt_trace_lines = repair.corrupt_trace_line_count,
                    "rebuilt state index entries missing for runs on disk"
                ),
                None => tracing::debug!(
                    runs_on_disk = result.runs_on_disk,
                    "state index already covers every run on disk"
                ),
            },
            Ok(Err(err)) => {
                tracing::warn!("state index backfill failed: {err}");
            }
            Err(_) => tracing::warn!(
                timeout_secs = STATE_INDEX_BACKFILL_TIMEOUT.as_secs(),
                "state index backfill did not finish in time; history may be incomplete \
                 until the next start"
            ),
        }
    });
}
