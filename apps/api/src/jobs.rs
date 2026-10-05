//! `/jobs`, `/runs`, and `/providers` HTTP handlers and generic job launch.

use super::*;

#[utoipa::path(
    post,
    path = "/jobs",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    request_body = CreateJobRequest,
    responses(
        (status = 200, description = "Job created", body = CreateJobResponse, content_type = "application/json"),
        (status = 400, description = "Invalid job request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 409, description = "Resume or product-session conflict", body = ApiErrorResponse, content_type = "application/json"),
        (status = 503, description = "Product store unavailable", body = ApiErrorResponse, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = ApiErrorResponse, content_type = "application/json")
    )
)]
pub(crate) async fn create_job(
    State(state): State<ApiState>,
    body: Result<Json<CreateJobRequest>, JsonRejection>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    let req = json_body(body, "bad_request", INVALID_JOB_BODY_MESSAGE)?;
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    if req.product_session_id.is_some() && req.resume.is_some() {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionResumeConflict.as_str(),
            "product-session jobs resolve resume from the server binding; omit resume",
        ));
    }

    let response = start_tracked_job(state, req)
        .await
        .map_err(|_| ApiError::internal("job start task did not complete"))??;

    Ok(Json(response))
}

pub(crate) fn start_tracked_job(
    state: ApiState,
    req: CreateJobRequest,
) -> oneshot::Receiver<Result<CreateJobResponse, ApiError>> {
    let (response_tx, response_rx) = oneshot::channel();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        let result = prepare_and_start_job(state, req).await;
        let _ = response_tx.send(result);
    }));
    response_rx
}

pub(crate) async fn prepare_and_start_job(
    state: ApiState,
    req: CreateJobRequest,
) -> Result<CreateJobResponse, ApiError> {
    let launch = prepare_job_launch(&state, &req).await?;
    let record = Arc::clone(&launch.record);
    let response = CreateJobResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        resumed_from_run_id: record.resumed_from_run_id,
        workspace_activation: record.config.project_activation_state().into(),
    };
    start_job_supervisor(state, launch).await;

    Ok(response)
}

#[utoipa::path(
    post,
    path = "/providers/models",
    tag = docs::PROVIDERS_TAG,
    security(("BearerAuth" = [])),
    request_body = ProviderModelsRequest,
    responses(
        (status = 200, description = "Provider model catalog", body = ProviderModelsResponse, content_type = "application/json"),
        (status = 400, description = "Invalid provider profile or missing key env", body = serde_json::Value, content_type = "application/json"),
        (status = 429, description = "Provider rate limited the inventory request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 504, description = "Provider inventory request timed out", body = ApiErrorResponse, content_type = "application/json"),
        (status = 502, description = "Provider model inventory request failed", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn list_provider_models(
    State(_state): State<ApiState>,
    body: Result<Json<ProviderModelsRequest>, JsonRejection>,
) -> Result<Json<ProviderModelsResponse>, ApiError> {
    let req = json_body(body, "bad_request", INVALID_PROVIDER_BODY_MESSAGE)?;
    let profile = normalize_provider_profile(&req.provider)?;
    let key_env = provider_key_env(&profile);
    let inventory = provider_inventory(&profile, &key_env, req.models_endpoint.as_deref()).await?;
    Ok(Json(ProviderModelsResponse {
        provider: profile.name,
        provider_type: profile.provider_type,
        wire_protocol: profile.wire_protocol,
        api_base: profile.api_base,
        key_env,
        key_present: inventory.key_present,
        models_count: inventory.models.len(),
        models: inventory.models,
    }))
}

#[utoipa::path(
    post,
    path = "/providers/test",
    tag = docs::PROVIDERS_TAG,
    security(("BearerAuth" = [])),
    request_body = ProviderTestRequest,
    responses(
        (status = 200, description = "Provider inventory check result", body = ProviderTestResponse, content_type = "application/json"),
        (status = 400, description = "Invalid provider profile", body = serde_json::Value, content_type = "application/json"),
        (status = 429, description = "Provider rate limited the inventory request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 504, description = "Provider inventory request timed out", body = ApiErrorResponse, content_type = "application/json"),
        (status = 502, description = "Provider inventory request failed", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn test_provider(
    State(_state): State<ApiState>,
    body: Result<Json<ProviderTestRequest>, JsonRejection>,
) -> Result<Json<ProviderTestResponse>, ApiError> {
    let req = json_body(body, "bad_request", INVALID_PROVIDER_BODY_MESSAGE)?;
    let profile = normalize_provider_profile(&req.provider)?;
    let key_env = provider_key_env(&profile);
    let inventory = provider_inventory(&profile, &key_env, req.models_endpoint.as_deref()).await?;
    let model_present = req
        .model
        .as_ref()
        .map(|model| inventory.models.iter().any(|id| id == model));
    Ok(Json(ProviderTestResponse {
        status: "pass".to_string(),
        provider: profile.name,
        provider_type: Some(profile.provider_type),
        wire_protocol: Some(profile.wire_protocol),
        api_base: profile.api_base,
        key_env,
        key_present: inventory.key_present,
        model: req.model,
        model_present,
        models_count: inventory.models.len(),
    }))
}

#[utoipa::path(
    get,
    path = "/jobs/{job_id}/events",
    tag = docs::JOB_EVENTS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("after" = Option<u64>, Query, description = "Replay only events with seq greater than this value")
    ),
    responses(
        (status = 200, description = "Server-Sent Events stream. Each frame carries `seq` in the SSE `id:` field, the variant name in `event:`, and a `data:` body of `{\"v\": PROTOCOL_VERSION, ...StreamEvent}` — the protocol version first, then the event's own fields flattened alongside `type`.", body = JobStreamEvent, content_type = "text/event-stream"),
        (status = 400, description = "Invalid Last-Event-ID header", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load persisted events", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn job_events(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
    Query(query): Query<JobEventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let after = query
        .after
        .or(parse_last_event_id(&headers)?)
        .unwrap_or_default();

    let Some(record) = live_job(&state, job_id).await else {
        let replay_events = persisted_job_events(&state, job_id, after).await?;
        let stream = futures::stream::iter(replay_events)
            .filter_map(|event| futures::future::ready(sse_event(event).ok()))
            .map(Ok)
            .boxed();
        return Ok(Sse::new(stream).keep_alive(KeepAlive::default()));
    };

    let live_rx = record.tx.subscribe();
    let (existing, status) = persisted_or_live_events(&state, &record, after).await?;
    let stream = replay_and_live_job_event_stream(existing, status, live_rx, after)
        .filter_map(|event| futures::future::ready(sse_event(event).ok()))
        .map(Ok)
        .boxed();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(crate) fn replay_and_live_job_event_stream(
    existing: Vec<JobStreamEvent>,
    status: RunStatus,
    receiver: broadcast::Receiver<JobStreamEvent>,
    after: u64,
) -> futures::stream::BoxStream<'static, JobStreamEvent> {
    let live_terminal_published = is_terminal(&status);
    let replay_events: Vec<_> = existing
        .into_iter()
        .filter(|event| event.seq > after)
        .filter(|event| {
            live_terminal_published || !matches!(&event.event, StreamEvent::RunCompleted { .. })
        })
        .collect();
    let replay_has_terminal = replay_events
        .iter()
        .any(|event| matches!(&event.event, StreamEvent::RunCompleted { .. }));
    let replay_high_water = replay_events.last().map(|event| event.seq).unwrap_or(after);
    let replay = futures::stream::iter(replay_events);
    let live = if replay_has_terminal || live_terminal_published {
        futures::stream::empty().boxed()
    } else {
        live_job_event_stream(receiver, replay_high_water)
    };
    replay.chain(live).boxed()
}

pub(crate) fn live_job_event_stream(
    receiver: broadcast::Receiver<JobStreamEvent>,
    after: u64,
) -> futures::stream::BoxStream<'static, JobStreamEvent> {
    futures::stream::unfold(
        (receiver, false),
        move |(mut receiver, completed)| async move {
            if completed {
                return None;
            }
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let completed = matches!(&event.event, StreamEvent::RunCompleted { .. });
                        if event.seq > after {
                            return Some((event, (receiver, completed)));
                        }
                        if completed {
                            return None;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    )
    .boxed()
}

#[utoipa::path(
    get,
    path = "/jobs/{job_id}/state",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID")
    ),
    responses(
        (status = 200, description = "Current or persisted job state", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load persisted job state", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn job_state(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    if let Some(record) = live_job(&state, job_id).await {
        return Ok(Json(job_state_response(&record).await));
    }
    Ok(Json(persisted_job_state_response(&state, job_id).await?))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/cancel",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID")
    ),
    responses(
        (status = 200, description = "Job state after cancellation request", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Live job not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn cancel_job(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    let current_status = record.status.lock().await.clone();
    if is_terminal(&current_status) {
        wait_for_job_completion(&record).await;
        return Ok(Json(job_state_response(&record).await));
    }

    record.cancel_token.cancel();
    let state_store = state_store_for_record(&record);
    reject_pending_approvals(&record, &state_store.index).await;
    reject_pending_inputs(&record, &state_store.index).await;
    wait_for_job_completion(&record).await;

    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/approvals/{call_id}",
    tag = docs::APPROVALS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("call_id" = String, Path, description = "Tool call ULID")
    ),
    request_body = SubmitApprovalRequest,
    responses(
        (status = 200, description = "Job state after resolving approval", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job or pending approval not found", body = serde_json::Value, content_type = "application/json"),
        (status = 409, description = "Approval responder is no longer live", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn submit_approval(
    State(state): State<ApiState>,
    Path((job_id, call_id)): Path<(JobId, CallId)>,
    body: Result<Json<SubmitApprovalRequest>, JsonRejection>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let req = json_body(body, "bad_request", INVALID_APPROVAL_BODY_MESSAGE)?;
    let record = find_job(&state, job_id).await?;
    let pending = record
        .pending_approvals
        .lock()
        .await
        .remove(&call_id)
        .ok_or_else(|| ApiError::not_found("pending approval not found"))?;
    let index = state_store_for_record(&record).index;
    index
        .record_approval_decision_async(
            call_id,
            approval_status(req.decision).to_string(),
            "job_api".to_string(),
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to persist approval decision: {err}")))?;
    if pending.tx.send(req.decision).is_err() {
        if let Err(err) = index
            .record_approval_decision_async(
                call_id,
                "cancelled".to_string(),
                "job_responder_lost".to_string(),
            )
            .await
        {
            tracing::warn!(job_id = %job_id, call_id = %call_id, "failed to mark stale approval cancelled: {err}");
        }
        return Err(ApiError::conflict(
            "approval is no longer awaiting a response",
        ));
    }
    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/inputs/{input_id}",
    tag = docs::APPROVALS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("input_id" = String, Path, description = "Pending input ULID")
    ),
    request_body = SubmitInputRequest,
    responses(
        (status = 200, description = "Job state after answering input request", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job or pending input not found", body = serde_json::Value, content_type = "application/json"),
        (status = 409, description = "Input responder is no longer live", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn submit_input(
    State(state): State<ApiState>,
    Path((job_id, input_id)): Path<(JobId, CallId)>,
    body: Result<Json<SubmitInputRequest>, JsonRejection>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let req = json_body(body, "bad_request", INVALID_INPUT_BODY_MESSAGE)?;
    let record = find_job(&state, job_id).await?;
    let pending = record
        .pending_inputs
        .lock()
        .await
        .remove(&input_id)
        .ok_or_else(|| ApiError::not_found("pending input not found"))?;
    let index = state_store_for_record(&record).index;
    index
        .mark_pending_input_status_async(input_id, "answered".to_string())
        .await
        .map_err(|err| ApiError::internal(format!("failed to persist input answer: {err}")))?;
    if pending.tx.send(req.answer).is_err() {
        if let Err(err) = index
            .mark_pending_input_status_async(input_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %job_id, input_id = %input_id, "failed to mark stale input cancelled: {err}");
        }
        return Err(ApiError::conflict("input is no longer awaiting a response"));
    }
    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    get,
    path = "/runs",
    tag = docs::RUNS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("limit" = Option<usize>, Query, description = "Maximum number of run summaries to return")
    ),
    responses(
        (status = 200, description = "Recent run summaries", body = ListRunsResponse, content_type = "application/json"),
        (status = 500, description = "Failed to list runs", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn list_runs(
    State(state): State<ApiState>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<ListRunsResponse>, ApiError> {
    let state_store = state_store_for_api(&state);
    let records = state_store
        .index
        .list_run_records_async(query.limit.unwrap_or(50))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ListRunsResponse {
        runs: records
            .into_iter()
            .map(|record| RunSummaryResponse {
                run_id: record.run_id,
                session_id: record.session_id,
                job_id: record.job_id,
                status: run_status_from_index(&record.status),
                last_event_seq: record.last_event_seq,
                has_report: record.report_path.is_some(),
            })
            .collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/runs/{run_id}/report",
    tag = docs::RUNS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("run_id" = String, Path, description = "Run ULID")
    ),
    responses(
        (status = 200, description = "Persisted run report", body = serde_json::Value, content_type = "application/json"),
        (status = 404, description = "Run report not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load run report", body = serde_json::Value, content_type = "application/json")
    )
)]
pub(crate) async fn run_report(
    State(state): State<ApiState>,
    Path(run_id): Path<RunId>,
) -> Result<Json<rove_runtime::state::report::RunReport>, ApiError> {
    let state_store = state_store_for_api(&state);
    let report = state_store
        .load_report(run_id)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ApiError::not_found("run report not found"),
            _ => ApiError::internal(err),
        })?;
    Ok(Json(report))
}

pub(crate) async fn prepare_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
) -> Result<JobLaunch, ApiError> {
    match req.product_session_id.as_ref() {
        Some(product_session_id) => {
            let approval_policy = resolve_product_job_approval_policy(state).await?;
            prepare_product_job_launch(state, req, product_session_id, approval_policy).await
        }
        None => {
            let approval_policy = req.approval.unwrap_or(ApprovalPolicy::Ask);
            prepare_generic_job_launch(state, req, approval_policy).await
        }
    }
}

pub(crate) async fn resolve_product_job_approval_policy(
    state: &ApiState,
) -> Result<ApprovalPolicy, ApiError> {
    let store = state.product_store()?;
    let preference = store.get_preferences().await?.default_approval_policy;
    Ok(match preference {
        ProductApprovalPreference::Ask => ApprovalPolicy::Ask,
        ProductApprovalPreference::Auto => ApprovalPolicy::Auto,
        ProductApprovalPreference::Never => ApprovalPolicy::Never,
    })
}

pub(crate) async fn prepare_generic_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    approval_policy: ApprovalPolicy,
) -> Result<JobLaunch, ApiError> {
    let (workspace, config) = workspace_and_config_for_create_job(state, req)?;
    let state_store = state_store_for_parts(&workspace, &config);
    let resume_state = resolve_resume_state(&state_store, req.resume.as_deref())
        .await
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    if req.resume.is_some() && resume_state.is_none() {
        return Err(ApiError::bad_request(
            "nothing to resume in this workspace; hard resume requires durable task_state under the requested workspace root",
        ));
    }

    let mut resume_claim = claim_runtime_resume(&state_store, resume_state.as_ref(), false).await?;
    let session_id = resume_state
        .as_ref()
        .map(|task_state| task_state.session_id)
        .unwrap_or_else(SessionId::new);
    let job_id = resume_state
        .as_ref()
        .map(|task_state| task_state.job_id)
        .unwrap_or_else(JobId::new);
    let resumed_from_run_id = resume_state.as_ref().map(|task_state| task_state.run_id);
    let record = new_job_record(NewJobRecord {
        state,
        workspace,
        config,
        request: req,
        message_override: None,
        content_blocks: Vec::new(),
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id: None,
        product_store: None,
        attachment_storage: None,
        product_model_config: None,
        run_model_snapshot: None,
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
            tracing::warn!(job_id = %record.job_id, "failed to assemble job engine: {error}");
            return Err(ApiError::agent_engine_assembly(&error));
        }
    };
    let run = match state_store.start_run(record.session_id, record.job_id, record.run_id) {
        Ok(run) => run,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            return Err(ApiError::internal(error));
        }
    };

    Ok(JobLaunch {
        record,
        engine,
        run,
        product_turn: None,
        startup_events: Vec::new(),
    })
}

pub(crate) async fn prepare_product_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    product_session_id: &ProductSessionId,
    approval_policy: ApprovalPolicy,
) -> Result<JobLaunch, ApiError> {
    let store = state.product_store()?;
    let claim = store.claim_session_turn(product_session_id).await?;
    prepare_claimed_product_job_launch(
        state,
        req,
        product_session_id,
        store,
        claim,
        approval_policy,
        None,
    )
    .await
}
