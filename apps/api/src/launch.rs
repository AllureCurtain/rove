//! Claimed product-turn launch and product resume-state loading.

use super::*;

pub(crate) async fn prepare_claimed_product_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    product_session_id: &ProductSessionId,
    store: Arc<dyn ProductStore>,
    claim: ProductTurnClaim,
    approval_policy: ApprovalPolicy,
    followup_control_id: Option<ProductControlId>,
) -> Result<JobLaunch, ApiError> {
    let claim_id = claim.claim_id.clone();
    let previous_product_status = claim.previous_status;
    let product_model_config = claim.model_config.clone();
    let owning_context = claim.context.clone();

    let (workspace, config, run_model_snapshot) = match workspace_and_config_for_product_job(
        state,
        req,
        &claim.context.workspace,
        &store,
        &product_model_config,
        claim.previous_binding.is_some(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            let provider_resume_failure = matches!(
                error.code,
                "provider_unavailable_for_resume" | "provider_changed_for_resume"
            );
            if let Some(control_id) = &followup_control_id {
                if provider_resume_failure {
                    abandon_failed_followup_start(
                        &store,
                        &claim_id,
                        control_id,
                        "workspace validation",
                    )
                    .await;
                } else {
                    requeue_failed_followup_start(
                        state,
                        product_session_id,
                        &store,
                        &claim_id,
                        control_id,
                        "workspace validation",
                        error.code,
                    )
                    .await;
                }
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    None,
                    if provider_resume_failure {
                        ProductSessionStatus::NeedsAttention
                    } else {
                        previous_product_status
                    },
                    // A Provider that cannot resume is a failure the user has to
                    // resolve; any other validation failure leaves the session as
                    // it was, which is also where the last outcome stays.
                    provider_resume_failure.then_some(ProductSessionOutcome::Failed),
                    "workspace validation",
                )
                .await;
            }
            return Err(error);
        }
    };
    // The image capability is resolved from the same profile-resolved config
    // the run will dispatch through, before anything is composed: a model
    // without image support composes the labelled "not sent" line instead of
    // carrying bytes the provider would refuse.
    let images_supported = model_supports_images(&config, &product_model_config.model);
    let (launch_message, launch_blocks) = match &followup_control_id {
        Some(control_id) => {
            compose_message_for_model(
                store.as_ref(),
                Some(&state.attachment_storage()),
                product_session_id,
                control_id,
                &req.message,
                images_supported,
            )
            .await
        }
        None => (req.message.clone(), Vec::new()),
    };
    let state_store = state_store_for_parts(&workspace, &config);
    // A fork's first child turn starts with the source run's verified prompt
    // state, but must never resume the source job. The fresh runtime session
    // and job identities keep cancellation, controls, and later follow-ups
    // isolated from the parent. Subsequent child turns use their own binding.
    let (resume_state, mut resume_claim, fork_bootstrap) = match claim.previous_binding.as_ref() {
        Some(previous) => {
            match load_and_claim_product_resume(&state_store, previous, &run_model_snapshot).await {
                Ok((resume_state, resume_claim)) => (Some(resume_state), Some(resume_claim), false),
                Err(error) => {
                    if let Some(control_id) = &followup_control_id {
                        abandon_failed_followup_start(
                            &store,
                            &claim_id,
                            control_id,
                            "exact runtime resume validation",
                        )
                        .await;
                    } else {
                        finish_failed_product_start(
                            &store,
                            &claim_id,
                            None,
                            ProductSessionStatus::NeedsAttention,
                            Some(ProductSessionOutcome::Failed),
                            "exact runtime resume validation",
                        )
                        .await;
                    }
                    return Err(error);
                }
            }
        }
        None => match claim.context.fork.as_ref() {
            Some(fork) => match load_product_fork_resume(&state_store, &fork.fork).await {
                Ok(resume_state) => (Some(resume_state), None, true),
                Err(error) => {
                    if let Some(control_id) = &followup_control_id {
                        abandon_failed_followup_start(
                            &store,
                            &claim_id,
                            control_id,
                            "fork source resume validation",
                        )
                        .await;
                    } else {
                        finish_failed_product_start(
                            &store,
                            &claim_id,
                            None,
                            ProductSessionStatus::NeedsAttention,
                            Some(ProductSessionOutcome::Failed),
                            "fork source resume validation",
                        )
                        .await;
                    }
                    return Err(error);
                }
            },
            None => (None, None, false),
        },
    };
    let session_id = if fork_bootstrap {
        SessionId::new()
    } else {
        resume_state
            .as_ref()
            .map(|task_state| task_state.session_id)
            .unwrap_or_else(SessionId::new)
    };
    let job_id = if fork_bootstrap {
        JobId::new()
    } else {
        resume_state
            .as_ref()
            .map(|task_state| task_state.job_id)
            .unwrap_or_else(JobId::new)
    };
    // Fork bootstrap reuses the verified source task state only as a history
    // seed. It is a new runtime lineage, so the child binding must not be
    // classified as a normal resume of the parent run.
    let resumed_from_run_id = if fork_bootstrap {
        None
    } else {
        resume_state.as_ref().map(|task_state| task_state.run_id)
    };
    let record = new_job_record(NewJobRecord {
        state,
        workspace,
        config,
        request: req,
        message_override: Some(launch_message),
        content_blocks: launch_blocks,
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id: Some(product_session_id.clone()),
        product_store: Some(store.clone()),
        attachment_storage: Some(state.attachment_storage()),
        product_model_config: Some(product_model_config),
        run_model_snapshot: Some(run_model_snapshot),
    });
    let engine = match assemble_job_engine(
        state,
        &record.message,
        req,
        Arc::clone(&record),
        approval_policy,
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            let failure = ApiError::agent_engine_assembly(&error);
            if let Some(control_id) = &followup_control_id {
                requeue_failed_followup_start(
                    state,
                    product_session_id,
                    &store,
                    &claim_id,
                    control_id,
                    "engine assembly",
                    failure.code,
                )
                .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    None,
                    previous_product_status,
                    // Nothing ran: the engine could not be assembled, so the
                    // session is restored and the last outcome is untouched.
                    None,
                    "engine assembly",
                )
                .await;
            }
            tracing::warn!(job_id = %record.job_id, "failed to assemble product job engine: {error}");
            return Err(failure);
        }
    };

    if let Some(control_id) = &followup_control_id
        && let Err(error) = store
            .reserve_followup_run(&claim_id, control_id, record.run_id)
            .await
    {
        release_runtime_resume_claim(&state_store, resume_claim.take()).await;
        abandon_failed_followup_start(&store, &claim_id, control_id, "runtime run reservation")
            .await;
        return Err(error.into());
    }
    let run = match state_store.start_run(record.session_id, record.job_id, record.run_id) {
        Ok(run) => run,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            if let Some(control_id) = &followup_control_id {
                abandon_failed_followup_start(&store, &claim_id, control_id, "runtime run start")
                    .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    Some(record.run_id),
                    ProductSessionStatus::NeedsAttention,
                    Some(ProductSessionOutcome::Failed),
                    "runtime run start",
                )
                .await;
            }
            tracing::warn!(product_session_id = %product_session_id, "failed to start product runtime run: {error}");
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime store could not start a run",
            )
            .into());
        }
    };
    let _ = resume_claim.take();

    let committed_binding = match store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim_id.clone(),
            product_session_id: product_session_id.clone(),
            runtime_session_id: record.session_id,
            runtime_job_id: record.job_id,
            runtime_run_id: record.run_id,
            resumed_from_run_id: record.resumed_from_run_id,
            followup_control_id: followup_control_id.clone(),
            model_config: record.product_model_config.clone().ok_or_else(|| {
                ProductStoreError::new(
                    ProductErrorCode::ProductBindingCorrupt,
                    "product run is missing its claimed model configuration",
                )
            })?,
            run_model_snapshot: record.run_model_snapshot.clone(),
        })
        .await
    {
        Ok(binding) => binding,
        Err(error) => {
            finalize_prestarted_run(
                &record,
                &engine,
                run,
                "product run binding was not committed",
            )
            .await;
            if let Some(control_id) = &followup_control_id {
                abandon_failed_followup_start(
                    &store,
                    &claim_id,
                    control_id,
                    "runtime binding commit",
                )
                .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    Some(record.run_id),
                    ProductSessionStatus::NeedsAttention,
                    Some(ProductSessionOutcome::Failed),
                    "runtime binding commit",
                )
                .await;
            }
            return Err(error.into());
        }
    };

    // The binding now exists in the catalog, so record
    // it in the run directory too. Written after the commit and never before —
    // a sidecar for a binding that failed would resurrect a session that never
    // owned this run. A write failure is logged, not propagated: the run is
    // already bound and running, and losing durability of the catalog is a
    // smaller harm than failing a turn the user asked for.
    if let Err(error) = product::ownership::write_ownership(
        &run.run_dir,
        &product::ownership::ProductRunOwnership {
            product_session_id: product_session_id.clone(),
            workspace_id: owning_context.workspace.id.clone(),
            workspace_root: owning_context.workspace.canonical_root.clone(),
            workspace_kind: owning_context.workspace.kind,
            workspace_display_name: owning_context.workspace.display_name.clone(),
            session_title: owning_context.session.title.clone(),
            ordinal: committed_binding.ordinal,
            runtime_session_id: record.session_id,
            runtime_job_id: record.job_id,
            runtime_run_id: record.run_id,
            resumed_from_run_id: record.resumed_from_run_id,
            parent_session_id: owning_context.session.parent_session_id.clone(),
            fork_point_run_id: owning_context.session.fork_point_run_id,
            fork_point_seq: owning_context.session.fork_point_seq,
            session_created_at: owning_context.session.created_at.clone(),
            bound_at: committed_binding.bound_at.clone(),
        },
    ) {
        tracing::warn!(
            product_session_id = %product_session_id,
            run_id = %record.run_id,
            "failed to record product ownership in the run directory: {error}"
        );
    }

    Ok(JobLaunch {
        record,
        engine,
        run,
        product_turn: Some(ProductTurnSupervisor {
            store: store.clone(),
            claim_id,
        }),
        startup_events: match followup_control_id {
            Some(control_id) => match store.get_message(product_session_id, &control_id).await {
                Ok(message) => vec![
                    StreamEvent::MessageQueued {
                        id: control_id.to_string(),
                        content: message.content,
                    },
                    StreamEvent::MessageClaimedSuccessor {
                        id: control_id.to_string(),
                    },
                ],
                Err(_) => vec![StreamEvent::FollowupDequeued {
                    id: control_id.to_string(),
                }],
            },
            None => Vec::new(),
        },
    })
}

pub(crate) struct NewJobRecord<'a> {
    pub(crate) state: &'a ApiState,
    pub(crate) workspace: Workspace,
    pub(crate) config: AppConfig,
    pub(crate) request: &'a CreateJobRequest,
    /// The composed model-facing message, when a product message's attachments
    /// had to be composed against the run's resolved Provider config. `None`
    /// keeps the request's own text.
    pub(crate) message_override: Option<String>,
    pub(crate) content_blocks: Vec<rove_models::ContentBlock>,
    pub(crate) session_id: SessionId,
    pub(crate) job_id: JobId,
    pub(crate) resume_state: Option<TaskState>,
    pub(crate) resumed_from_run_id: Option<RunId>,
    pub(crate) product_session_id: Option<ProductSessionId>,
    pub(crate) product_store: Option<Arc<dyn ProductStore>>,
    pub(crate) attachment_storage: Option<product::attachments::AttachmentStorage>,
    pub(crate) product_model_config: Option<ProductSessionModelConfig>,
    pub(crate) run_model_snapshot: Option<RunModelSnapshot>,
}

pub(crate) fn new_job_record(input: NewJobRecord<'_>) -> Arc<JobRecord> {
    let NewJobRecord {
        state,
        workspace,
        config,
        request,
        message_override,
        content_blocks,
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id,
        product_store,
        attachment_storage,
        product_model_config,
        run_model_snapshot,
    } = input;
    let run_id = RunId::new();
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    Arc::new(JobRecord {
        session_id,
        job_id,
        run_id,
        workspace,
        config,
        message: message_override.unwrap_or_else(|| request.message.clone()),
        content_blocks,
        resumed_from_run_id,
        resume_state,
        product_session_id,
        product_store,
        attachment_storage,
        product_model_config,
        run_model_snapshot,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        control: Mutex::new(None),
        control_event_trace: Mutex::new(None),
        control_event_trace_lock: Mutex::new(()),
        control_lifecycle_lock: Mutex::new(()),
        pending_product_events: Mutex::new(Vec::new()),
        completion,
        cancel_token: state.inner.shutdown_token.child_token(),
    })
}

pub(crate) async fn claim_runtime_resume(
    state_store: &StateStore,
    resume_state: Option<&TaskState>,
    product: bool,
) -> Result<Option<ResumeJobClaim>, ApiError> {
    let Some(resume_state) = resume_state else {
        return Ok(None);
    };
    let claim = state_store
        .index
        .claim_job_for_resume_async(resume_state.job_id, resume_state.run_id)
        .await
        .map_err(ApiError::internal)?;
    match claim {
        Some(claim) => Ok(Some(claim)),
        None if product => Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionResumeConflict.as_str(),
            "the product session runtime run is active, stale, or no longer the job's latest terminal run",
        )),
        None => Err(ApiError::conflict(
            "cannot resume a job unless the requested run is its latest terminal run",
        )),
    }
}

pub(crate) async fn release_runtime_resume_claim(
    state_store: &StateStore,
    claim: Option<ResumeJobClaim>,
) {
    let Some(claim) = claim else {
        return;
    };
    match state_store
        .index
        .release_job_resume_claim_async(claim)
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!("runtime resume claim was no longer releasable"),
        Err(error) => tracing::warn!("failed to release runtime resume claim: {error}"),
    }
}

pub(crate) async fn load_and_claim_product_resume(
    state_store: &StateStore,
    previous: &ProductRuntimeBinding,
    current_run_model: &RunModelSnapshot,
) -> Result<(TaskState, ResumeJobClaim), ApiError> {
    let resume_state = load_product_resume_state(state_store, previous).await?;
    validate_product_resume_model(&resume_state, current_run_model)?;
    let resume_state = project_product_follow_up_state(resume_state)?;
    let Some(claim) = claim_runtime_resume(state_store, Some(&resume_state), true).await? else {
        return Err(ApiError::internal(
            "product resume validation did not acquire a runtime claim",
        ));
    };
    Ok((resume_state, claim))
}

/// Load the exact durable task state a product session's latest run left.
///
/// The catalog binding, the runtime job, the run record, and the snapshot all
/// have to describe the same run before the snapshot is trusted. This is the
/// read half of a product resume; the turn path additionally validates the
/// pinned model and claims the runtime job, while manual compaction only needs
/// the state it is about to rewrite.
pub(crate) async fn load_product_resume_state(
    state_store: &StateStore,
    previous: &ProductRuntimeBinding,
) -> Result<TaskState, ApiError> {
    let job = state_store
        .index
        .job_record_async(previous.latest_job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime job is missing",
            ))
        })?;
    if job.session_id != previous.runtime_session_id || job.run_id != Some(previous.latest_run_id) {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime job identity does not match its binding",
        )
        .into());
    }
    let run = load_runtime_run_record(&state_store.index, previous.latest_run_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime run is missing",
            ))
        })?;
    if run.session_id != previous.runtime_session_id
        || run.job_id != previous.latest_job_id
        || run.run_id != previous.latest_run_id
    {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime run identity does not match its binding",
        )
        .into());
    }
    // Loading task state may lazily repair its index projection. Validate the
    // existing indexed identities first so product resume fails closed on drift.
    let resume_state = state_store
        .load_task_state(previous.latest_run_id)
        .await
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                ProductErrorCode::ProductSessionRuntimeStateMissing
            } else {
                ProductErrorCode::ProductSessionRuntimeStateCorrupt
            };
            ApiError::from(ProductStoreError::new(
                code,
                "the product session's exact runtime task state is unavailable or invalid",
            ))
        })?;
    if resume_state.session_id != previous.runtime_session_id
        || resume_state.job_id != previous.latest_job_id
        || resume_state.run_id != previous.latest_run_id
    {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime task-state identity does not match its binding",
        )
        .into());
    }
    Ok(resume_state)
}

pub(crate) fn validate_product_resume_model(
    resume_state: &TaskState,
    current: &RunModelSnapshot,
) -> Result<(), ApiError> {
    let saved = resume_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
        .or(resume_state.runtime_identity.as_ref())
        .and_then(|identity| identity.run_model.as_ref());
    let Some(saved) = saved else {
        return Ok(());
    };

    let selection_changed = saved.profile_id != current.profile_id
        || saved.model != current.model
        || saved.reasoning != current.reasoning;
    if selection_changed {
        return Ok(());
    }
    let compatible = saved.provider_type == current.provider_type
        && saved.wire_protocol == current.wire_protocol
        && saved.endpoint == current.endpoint
        && saved.safe_config_digest == current.safe_config_digest;
    if compatible {
        Ok(())
    } else {
        Err(ApiError::conflict_with_code(
            ProductErrorCode::ProviderChangedForResume.as_str(),
            "the selected Provider changed since the previous run; start a new session or restore the original Provider identity",
        ))
    }
}
