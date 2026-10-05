//! Job records, approval/input providers, job state and SSE projections.

use super::*;

pub(crate) async fn live_job(state: &ApiState, job_id: JobId) -> Option<Arc<JobRecord>> {
    state.inner.jobs.read().await.get(&job_id).cloned()
}

pub(crate) async fn wait_for_job_completion(record: &JobRecord) {
    let mut completion = record.completion.subscribe();
    while !*completion.borrow_and_update() {
        if completion.changed().await.is_err() {
            tracing::warn!(job_id = %record.job_id, "job completion signal closed unexpectedly");
            return;
        }
    }
}

pub(crate) async fn drain_job_supervisors(state: &ApiState) {
    state.inner.job_starts.close();
    state.inner.job_starts.wait().await;
    state.inner.supervisors.close();
    state.inner.supervisors.wait().await;

    let records: Vec<_> = state.inner.jobs.read().await.values().cloned().collect();
    for record in records {
        let handle = record.handle.lock().await.take();
        let Some(handle) = handle else {
            continue;
        };
        if let Err(error) = handle.await {
            tracing::warn!(job_id = %record.job_id, "job supervisor join failed during shutdown: {error}");
        }
    }
}

pub(crate) async fn find_job(state: &ApiState, job_id: JobId) -> Result<Arc<JobRecord>, ApiError> {
    live_job(state, job_id)
        .await
        .ok_or_else(|| ApiError::not_found("job not found"))
}

pub(crate) struct ApiApprovalProvider {
    pub(crate) record: Arc<JobRecord>,
    pub(crate) index: StateIndex,
}

#[async_trait]
impl ToolApprovalProvider for ApiApprovalProvider {
    async fn begin_approval(
        &self,
        request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        let (tx, rx) = oneshot::channel();
        let record = Arc::clone(&self.record);
        let index = self.index.clone();
        let job_id = record.job_id;
        let call_id = request.call_id;
        let registration = tokio::spawn(async move {
            if let Err(err) = index
                .record_pending_approval_async(
                    call_id,
                    record.job_id,
                    record.run_id,
                    request.name.clone(),
                    request.args.to_string(),
                    request.reason.clone(),
                )
                .await
            {
                tracing::warn!(
                    job_id = %record.job_id,
                    call_id = %call_id,
                    "failed to persist pending approval: {err}"
                );
                return Err(format!("failed to persist pending approval: {err}"));
            }

            let mut pending = record.pending_approvals.lock().await;
            if record.cancel_token.is_cancelled() {
                drop(pending);
                if let Err(err) = index
                    .record_approval_decision_async(
                        call_id,
                        "cancelled".to_string(),
                        "job_cancel".to_string(),
                    )
                    .await
                {
                    tracing::warn!(job_id = %record.job_id, call_id = %call_id, "failed to mark cancelled approval registration: {err}");
                }
                return Err("approval request cancelled during registration".to_string());
            }
            pending.insert(call_id, PendingApproval { request, tx });
            Ok(())
        });

        match registration.await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ToolError::ExecutionFailed { reason }),
            Err(err) => {
                tracing::warn!(job_id = %job_id, call_id = %call_id, "approval registration task failed: {err}");
                return Err(ToolError::ExecutionFailed {
                    reason: format!("approval registration task failed for job {job_id}: {err}"),
                });
            }
        }
        Ok(PendingToolApproval::new(async move {
            rx.await.unwrap_or(ApprovalDecision::Reject)
        }))
    }
}

pub(crate) struct ApiInputProvider {
    pub(crate) record: Arc<JobRecord>,
    pub(crate) index: StateIndex,
}

#[async_trait]
impl UserInputProvider for ApiInputProvider {
    async fn begin_input(
        &self,
        input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        let (tx, rx) = oneshot::channel();
        let prompt = request.prompt.clone();
        let record = Arc::clone(&self.record);
        let index = self.index.clone();
        let job_id = record.job_id;
        let registration = tokio::spawn(async move {
            if let Err(err) = index
                .record_pending_input_async(input_id, record.job_id, record.run_id, prompt)
                .await
            {
                tracing::warn!(
                    job_id = %record.job_id,
                    input_id = %input_id,
                    "failed to persist pending input: {err}"
                );
                return Err(format!("failed to persist pending input: {err}"));
            }

            let mut pending = record.pending_inputs.lock().await;
            if record.cancel_token.is_cancelled() {
                drop(pending);
                if let Err(err) = index
                    .mark_pending_input_status_async(input_id, "cancelled".to_string())
                    .await
                {
                    tracing::warn!(job_id = %record.job_id, input_id = %input_id, "failed to mark cancelled input registration: {err}");
                }
                return Err("input request cancelled during registration".to_string());
            }
            pending.insert(input_id, PendingInput { request, tx });
            Ok(())
        });

        match registration.await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ToolError::ExecutionFailed { reason }),
            Err(err) => {
                return Err(ToolError::ExecutionFailed {
                    reason: format!("input registration task failed for job {job_id}: {err}"),
                });
            }
        }
        Ok(PendingUserInput::new(async move {
            rx.await.map_err(|_| ToolError::ExecutionFailed {
                reason: "input request cancelled".to_string(),
            })
        }))
    }
}

/// Builds the typed job-state projection from the events a caller is about to
/// send.
///
/// The live route and the persisted route both construct their response here,
/// so the payload has exactly one construction site and `answer_aborted` is
/// always derived from the same slice the response carries rather than from a
/// separate read of the run index. `JobStateResponse::answer_aborted` documents
/// when an absent marker is conclusive: only for a settled listing, which needs
/// a terminal `status` and a contiguous `events` list.
pub(crate) fn job_state_from_events(
    job_id: JobId,
    run_id: RunId,
    resumed_from_run_id: Option<RunId>,
    status: RunStatus,
    events: Vec<JobStreamEvent>,
    pending_approvals: Vec<PendingApprovalResponse>,
    pending_inputs: Vec<PendingInputResponse>,
) -> JobStateResponse {
    JobStateResponse {
        job_id,
        run_id,
        resumed_from_run_id,
        status,
        event_count: events.len(),
        answer_aborted: crate::types::answer_aborted(&events),
        events,
        pending_approvals,
        pending_inputs,
    }
}

pub(crate) async fn job_state_response(record: &JobRecord) -> JobStateResponse {
    let events = record.events.lock().await.clone();
    job_state_from_events(
        record.job_id,
        record.run_id,
        record.resumed_from_run_id,
        record.status.lock().await.clone(),
        events,
        pending_approvals_response(record).await,
        pending_inputs_response(record).await,
    )
}

pub(crate) async fn persisted_job_state_response(
    state: &ApiState,
    job_id: JobId,
) -> Result<JobStateResponse, ApiError> {
    let state_store = state_store_for_api(state);
    let job = state_store
        .index
        .job_record_async(job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("job not found"))?;
    let run_id = job
        .run_id
        .ok_or_else(|| ApiError::not_found("job run not found"))?;
    let events = persisted_events_for_run(&state_store, run_id, 0).await?;
    Ok(job_state_from_events(
        job.job_id,
        run_id,
        None,
        run_status_from_index(&job.status),
        events,
        Vec::new(),
        Vec::new(),
    ))
}

pub(crate) async fn persisted_job_events(
    state: &ApiState,
    job_id: JobId,
    after: u64,
) -> Result<Vec<JobStreamEvent>, ApiError> {
    let state_store = state_store_for_api(state);
    let job = state_store
        .index
        .job_record_async(job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("job not found"))?;
    let run_id = job
        .run_id
        .ok_or_else(|| ApiError::not_found("job run not found"))?;
    persisted_events_for_run(&state_store, run_id, after).await
}

pub(crate) async fn persisted_or_live_events(
    _state: &ApiState,
    record: &JobRecord,
    after: u64,
) -> Result<(Vec<JobStreamEvent>, RunStatus), ApiError> {
    let state_store = state_store_for_record(record);
    let mut merged = persisted_events_for_run(&state_store, record.run_id, after)
        .await?
        .into_iter()
        .map(|event| (event.seq, event))
        .collect::<BTreeMap<_, _>>();
    // Terminal publication takes these locks in the same order. Holding the
    // event lock through the status snapshot makes the replay/live handoff
    // atomic with respect to a newly published terminal event.
    let events = record.events.lock().await;
    let status = record.status.lock().await.clone();
    for event in events.iter().cloned() {
        if event.seq > after {
            merged.insert(event.seq, event);
        }
    }
    Ok((merged.into_values().collect(), status))
}

pub(crate) async fn persisted_events_for_run(
    state_store: &StateStore,
    run_id: RunId,
    after: u64,
) -> Result<Vec<JobStreamEvent>, ApiError> {
    let records = state_store
        .index
        .event_records_async(run_id)
        .await
        .map_err(ApiError::internal)?;
    records
        .into_iter()
        .filter(|record| record.seq > after)
        .map(|record| {
            let event = serde_json::from_str::<StreamEvent>(&record.event_json)
                .map_err(ApiError::internal)?;
            Ok(JobStreamEvent {
                seq: record.seq,
                event,
            })
        })
        .collect()
}

pub(crate) async fn append_job_event(record: &JobRecord, event: StreamEvent) -> JobStreamEvent {
    let stored = {
        let mut events = record.events.lock().await;
        if let StreamEvent::RunCompleted { reason, .. } = &event {
            *record.status.lock().await = status_for_reason(reason);
        }
        let seq = events.last().map(|event| event.seq + 1).unwrap_or(1);
        let stored = JobStreamEvent { seq, event };
        events.push(stored.clone());
        stored
    };
    let _ = record.tx.send(stored.clone());
    stored
}

pub(crate) async fn pending_approvals_response(record: &JobRecord) -> Vec<PendingApprovalResponse> {
    record
        .pending_approvals
        .lock()
        .await
        .values()
        .map(|pending| PendingApprovalResponse {
            call_id: pending.request.call_id,
            name: pending.request.name.clone(),
            args: pending.request.args.clone(),
            reason: pending.request.reason.clone(),
        })
        .collect()
}

pub(crate) async fn pending_inputs_response(record: &JobRecord) -> Vec<PendingInputResponse> {
    record
        .pending_inputs
        .lock()
        .await
        .iter()
        .map(|(input_id, pending)| PendingInputResponse {
            input_id: *input_id,
            prompt: pending.request.prompt.clone(),
        })
        .collect()
}

pub(crate) async fn reject_pending_approvals(record: &JobRecord, index: &StateIndex) {
    let pending = std::mem::take(&mut *record.pending_approvals.lock().await);
    for (call_id, approval) in pending {
        if let Err(err) = index
            .record_approval_decision_async(
                call_id,
                "cancelled".to_string(),
                "job_cancel".to_string(),
            )
            .await
        {
            tracing::warn!(job_id = %record.job_id, call_id = %call_id, "failed to mark pending approval cancelled: {err}");
        }
        let _ = approval.tx.send(ApprovalDecision::Reject);
    }
}

pub(crate) async fn reject_pending_inputs(record: &JobRecord, index: &StateIndex) {
    let pending = std::mem::take(&mut *record.pending_inputs.lock().await);
    for (input_id, _) in pending {
        if let Err(err) = index
            .mark_pending_input_status_async(input_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %record.job_id, input_id = %input_id, "failed to mark pending input cancelled: {err}");
        }
    }
}

pub(crate) fn approval_status(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approve => "approved",
        ApprovalDecision::Reject => "rejected",
    }
}

pub(crate) fn sse_event(event: JobStreamEvent) -> Result<Event, serde_json::Error> {
    let name = event.event.event_name();
    // Every frame carries the protocol version as its first field. The payload
    // is flattened, so a client written before versioning still finds `type`
    // and the event fields exactly where they were.
    let versioned = rove_protocol::Versioned::now(&event.event);
    // One redaction point for the whole SSE surface, live and replayed alike:
    // the frame is built here from an in-memory event that has not been through
    // the trace writer, so redacting only on write would leave the live stream
    // unredacted. A frame that fails to serialize still fails; a credential in
    // one that succeeds never reaches the wire.
    let data =
        rove_runtime::secrets::registry().redact_json_text(&serde_json::to_string(&versioned)?);
    Ok(Event::default()
        .id(event.seq.to_string())
        .event(name)
        .data(data))
}

pub(crate) fn parse_last_event_id(headers: &HeaderMap) -> Result<Option<u64>, ApiError> {
    let Some(value) = headers.get("last-event-id") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("Last-Event-ID must be a valid integer"))?;
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ApiError::bad_request("Last-Event-ID must be a valid integer"))
}

pub(crate) fn is_terminal(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Done | RunStatus::Error | RunStatus::Cancelled | RunStatus::Interrupted
    )
}

pub(crate) fn status_for_reason(reason: &TerminationReason) -> RunStatus {
    match reason {
        TerminationReason::Final
        | TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => RunStatus::Done,
        TerminationReason::Error => RunStatus::Error,
        TerminationReason::Cancelled => RunStatus::Cancelled,
    }
}

pub(crate) fn run_status_from_index(status: &str) -> RunStatus {
    match status {
        "init" => RunStatus::Init,
        "running" => RunStatus::Running,
        "done" => RunStatus::Done,
        "error" => RunStatus::Error,
        "cancelled" => RunStatus::Cancelled,
        "interrupted" => RunStatus::Interrupted,
        _ => RunStatus::Error,
    }
}
