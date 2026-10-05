//! Job supervisor: event consumption, control reflection, terminal finalization.

use super::*;

/// Close a product turn that failed before or during startup.
///
/// `outcome` is what this attempt leaves behind: `Some(Failed)` when the caller
/// classified the attempt as a failure the user must see, and `None` when the
/// caller is undoing the attempt and restoring the session's previous status —
/// in that case the record of the last turn that actually ran must survive, so
/// a rejected retry cannot rewrite the session's history.
pub(crate) async fn finish_failed_product_start(
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    run_id: Option<RunId>,
    status: ProductSessionStatus,
    outcome: Option<ProductSessionOutcome>,
    phase: &'static str,
) {
    if let Err(error) = store
        .finish_session_turn_and_abandon_pending_controls(claim_id, run_id, status, outcome, phase)
        .await
    {
        tracing::warn!(
            phase = phase,
            "failed to classify controls after product start failure: {error}"
        );
    }
}

pub(crate) async fn finalize_prestarted_run(
    record: &JobRecord,
    engine: &Engine,
    run: RunHandle,
    message: &str,
) {
    let state_store = state_store_for_record(record);
    let terminal = StreamEvent::RunCompleted {
        reason: TerminationReason::Error,
        output: Some(message.to_string()),
    };
    let trace_persisted = append_trace_event(&run.trace_writer, record, &terminal).await;
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        record.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    recorder.record_event(&terminal, &state_store).await;
    recorder
        .finalize(
            &state_store,
            engine.workspace(),
            engine.model_id(),
            &run.run_dir,
        )
        .await;
    if !trace_persisted || !runtime_terminal_is_durable(&state_store, record, &terminal).await {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "prestarted runtime run did not reach a fully durable terminal state"
        );
    }
}

pub(crate) async fn start_job_supervisor(state: ApiState, launch: JobLaunch) {
    let JobLaunch {
        record,
        engine,
        run,
        product_turn,
        startup_events,
    } = launch;
    let record_for_task = Arc::clone(&record);
    let recovery_record = Arc::clone(&record);
    let recovery_product_turn = product_turn.clone();
    let completion = record.completion.clone();
    let state_for_supervisor = state.clone();
    let state_for_recovery = state.clone();
    // Register before spawning the stream. A control submitted after the
    // ProductStore turn claim but before `consume_job_stream` installs its
    // handle is either replayed from the durable queue or explicitly
    // rejected; it cannot be inserted after the one-time replay and become
    // stranded forever.
    state
        .inner
        .jobs
        .write()
        .await
        .insert(record.job_id, Arc::clone(&record));
    let handle = state.inner.supervisors.spawn(async move {
        let _completion_guard = JobCompletionGuard::new(completion);
        let outcome = AssertUnwindSafe(run_job_supervisor(
            state_for_supervisor,
            record_for_task,
            engine,
            run,
            product_turn,
            startup_events,
        ))
        .catch_unwind()
        .await;
        if outcome.is_err() {
            tracing::warn!(job_id = %recovery_record.job_id, "job supervisor panicked");
            let recovery = AssertUnwindSafe(recover_job_supervisor_panic(
                &state_for_recovery,
                &recovery_record,
                recovery_product_turn,
            ))
            .catch_unwind()
            .await;
            if recovery.is_err() {
                tracing::warn!(job_id = %recovery_record.job_id, "job supervisor recovery panicked");
                let terminal_published = {
                    let status = recovery_record.status.lock().await;
                    is_terminal(&status)
                };
                if !terminal_published {
                    append_job_event(
                        &recovery_record,
                        supervisor_failure_event("job supervisor failed"),
                    )
                    .await;
                }
            }
        }
    });
    *record.handle.lock().await = Some(handle);
}

pub(crate) async fn run_job_supervisor(
    state: ApiState,
    record: Arc<JobRecord>,
    engine: Engine,
    run: RunHandle,
    product_turn: Option<ProductTurnSupervisor>,
    startup_events: Vec<StreamEvent>,
) {
    let review_execution = state
        .inner
        .review_jobs
        .read()
        .await
        .get(&record.job_id)
        .cloned();
    let trust_monitor = if review_execution.is_some() {
        None
    } else {
        start_project_trust_monitor(&state, &record).await
    };
    let state_store = state_store_for_record(&record);
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        record.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    let stream_outcome = AssertUnwindSafe(consume_job_stream(
        &record,
        &engine,
        &run,
        &state_store,
        &mut recorder,
        startup_events,
    ))
    .catch_unwind()
    .await;
    stop_project_trust_monitor(trust_monitor).await;
    let (terminal, mut needs_attention, mut stream_trace_complete) = match stream_outcome {
        Ok(Some((terminal, trace_complete))) => (terminal, false, trace_complete),
        Ok(None) => (
            supervisor_failure_event("runtime stream ended without exactly one terminal event"),
            true,
            false,
        ),
        Err(_) => {
            tracing::warn!(job_id = %record.job_id, "job runtime panicked");
            (supervisor_failure_event("job runtime failed"), true, false)
        }
    };
    let terminal_status = terminal_run_status(&terminal);
    if matches!(terminal_status, RunStatus::Cancelled | RunStatus::Error) {
        reject_pending_approvals(&record, &state_store.index).await;
        reject_pending_inputs(&record, &state_store.index).await;
    }

    let mut final_candidate = matches!(
        &terminal,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        }
    ) && !needs_attention
        && stream_trace_complete;
    let _control_lifecycle = record.control_lifecycle_lock.lock().await;
    if final_candidate {
        match drop_unapplied_product_steers_before_terminal(
            product_turn.as_ref(),
            &record,
            &run.trace_writer,
            &mut recorder,
            &state_store,
        )
        .await
        {
            Ok(trace_complete) => {
                stream_trace_complete &= trace_complete;
                if !stream_trace_complete {
                    final_candidate = false;
                    needs_attention = true;
                }
            }
            Err(error) => {
                tracing::warn!(
                    job_id = %record.job_id,
                    run_id = %record.run_id,
                    "failed to close unapplied steers before product terminal: {error}"
                );
                final_candidate = false;
                needs_attention = true;
                stream_trace_complete = false;
            }
        }
    }
    if !final_candidate {
        finish_nonfinal_product_turn(
            product_turn.clone(),
            &terminal,
            needs_attention || !stream_trace_complete,
            &record,
            &run.trace_writer,
            &mut recorder,
            &state_store,
        )
        .await;
    }

    let public_terminal = if engine.run_mode() == RunMode::Review {
        terminal.redacted_for_review_persistence()
    } else {
        terminal.clone()
    };
    let trace_persisted = persist_terminal_and_finalize(
        &run.trace_writer,
        &record,
        public_terminal.clone(),
        &mut recorder,
        &state_store,
        &engine,
        &run,
    )
    .await;
    let runtime_durable = stream_trace_complete
        && trace_persisted
        && runtime_terminal_is_durable(&state_store, &record, &public_terminal).await;
    if !runtime_durable {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "runtime terminal artifacts are incomplete"
        );
    }

    let claimed_followup = if final_candidate && runtime_durable {
        finish_final_product_turn(product_turn.clone(), record.run_id).await
    } else if final_candidate {
        // The terminal trace is already incomplete, so do not append late
        // lifecycle events after it. Persist the conservative store state;
        // transcript projection will expose the durable-artifact failure.
        finish_product_turn_needs_attention(
            product_turn.clone(),
            Some(record.run_id),
            "run completed with incomplete terminal artifacts",
        )
        .await;
        None
    } else {
        None
    };

    // The turn boundary just wrote the durable directory facts (the session's
    // status/outcome and any successor claim), so wake the product event stream
    // instead of leaving it to its poll interval.
    state.notify_product_events();

    if let (Some(psid), Some(claim)) = (record.product_session_id.clone(), claimed_followup) {
        schedule_claimed_followup_start(&state, psid, claim);
    }

    if let Some(review_execution) = review_execution {
        finalize_review_execution(
            &record,
            &engine,
            &terminal,
            runtime_durable,
            review_execution,
        )
        .await;
        // Completed Review executions no longer need an in-memory snapshot or
        // submission store. Stale checks are rebuilt from the durable target
        // spec, so retaining this entry would only create unbounded growth.
        state.inner.review_jobs.write().await.remove(&record.job_id);
    }

    // The terminal event is the public lifecycle barrier. A client that sees
    // it must never observe the old product turn still claimed, so publish
    // only after ProductStore has released or atomically replaced that claim.
    publish_terminal_event(&record, public_terminal).await;
}

pub(crate) async fn finalize_review_execution(
    record: &JobRecord,
    engine: &Engine,
    terminal: &StreamEvent,
    runtime_durable: bool,
    execution: Arc<ReviewExecution>,
) {
    let cancelled = matches!(
        terminal,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Cancelled,
            ..
        }
    );
    let stale = execution
        .snapshot
        .is_stale(&record.workspace)
        .unwrap_or(true);
    let mut result = finalize_result_with_evidence(
        execution.review_id.to_string(),
        record.run_id.to_string(),
        record.session_id.to_string(),
        (*execution.snapshot).clone(),
        execution.submission_store.get(),
        stale,
        cancelled,
        execution.started_at.elapsed().as_millis() as u64,
        ReviewRuntimeEvidence::from(&engine.runtime_identity()),
    );
    let termination = match terminal {
        StreamEvent::RunCompleted { reason, .. } => reason.clone(),
        _ => TerminationReason::Error,
    };
    apply_runtime_outcome(&mut result, &termination, runtime_durable);
    match serde_json::to_vec_pretty(&result) {
        Ok(bytes) => {
            if let Err(error) =
                tokio::fs::write(execution.state_root.join("review.json"), bytes).await
            {
                tracing::warn!(
                    review_id = %execution.review_id,
                    "failed to persist Review result: {error}"
                );
            }
        }
        Err(error) => tracing::warn!(
            review_id = %execution.review_id,
            "failed to encode Review result: {error}"
        ),
    }
    if let Err(error) = execution
        .product_store
        .finalize_review(&execution.review_id, result)
        .await
    {
        tracing::warn!(
            review_id = %execution.review_id,
            "failed to finalize ProductStore Review projection: {error}"
        );
    }
}

pub(crate) struct ProjectTrustMonitor {
    pub(crate) stop: CancellationToken,
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

pub(crate) async fn start_project_trust_monitor(
    state: &ApiState,
    record: &Arc<JobRecord>,
) -> Option<ProjectTrustMonitor> {
    let authority = state.project_trust().ok()?;
    let provider_selector = if let (Some(store), Some(product_session_id)) =
        (&record.product_store, &record.product_session_id)
    {
        let catalog = state.provider_catalog().await.ok()?;
        match store.get_session_context(product_session_id).await {
            Ok(context) => product::trust::product_provider_capability_selector(
                store,
                &catalog,
                &context.workspace.id,
                &record.workspace.root,
            )
            .await
            .ok()?,
            Err(error) => {
                tracing::warn!(
                    job_id = %record.job_id,
                    "project trust monitor could not resolve the product session: {error}"
                );
                return None;
            }
        }
    } else {
        rove_app_bootstrap::provider_capability_selector_for_workspace(&record.workspace.root)
    };
    let digests = rove_app_bootstrap::capability_digest_map(
        &record.workspace.root,
        None,
        Some(&provider_selector),
    );
    let initial = match authority.resolve(
        &record.workspace.root,
        record.workspace.kind.clone(),
        &digests,
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            tracing::warn!(
                job_id = %record.job_id,
                "project trust monitor could not read its initial state: {error}"
            );
            return None;
        }
    };
    let initially_trusted = initial.state == ProjectActivationState::Trusted;
    let initially_granted = initial.granted_capabilities;
    let root = record.workspace.root.clone();
    let kind = record.workspace.kind.clone();
    let cancel = record.cancel_token.clone();
    let job_id = record.job_id;
    let stop = CancellationToken::new();
    let monitor_stop = stop.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The initial state was captured above; wait one interval before the
        // first comparison instead of performing an immediate duplicate read.
        interval.tick().await;
        loop {
            tokio::select! {
                _ = monitor_stop.cancelled() => break,
                _ = interval.tick() => {
                    let resolution = match authority.resolve(&root, kind.clone(), &digests) {
                        Ok(resolution) => resolution,
                        Err(error) => {
                            tracing::warn!(job_id = %job_id, "project trust monitor failed closed: {error}");
                            cancel.cancel();
                            break;
                        }
                    };
                    let revoked = resolution.state == ProjectActivationState::Revoked;
                    let trusted_authority_lost = initially_trusted
                        && (resolution.state != ProjectActivationState::Trusted
                            || !initially_granted.is_subset(&resolution.granted_capabilities));
                    if revoked || trusted_authority_lost {
                        tracing::warn!(job_id = %job_id, "project trust changed while the job was active; cancelling the run");
                        cancel.cancel();
                        break;
                    }
                }
            }
        }
    });
    Some(ProjectTrustMonitor { stop, handle })
}

pub(crate) async fn stop_project_trust_monitor(monitor: Option<ProjectTrustMonitor>) {
    let Some(ProjectTrustMonitor { stop, mut handle }) = monitor else {
        return;
    };
    stop.cancel();
    if tokio::time::timeout(std::time::Duration::from_secs(1), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
    }
}

pub(crate) async fn consume_job_stream(
    record: &JobRecord,
    engine: &Engine,
    run: &RunHandle,
    state_store: &StateStore,
    recorder: &mut RunArtifactRecorder,
    startup_events: Vec<StreamEvent>,
) -> Option<(StreamEvent, bool)> {
    let request = run.request(
        record.message.clone(),
        record.content_blocks.clone(),
        record.resume_state.clone(),
    );
    let mut stream =
        std::pin::pin!(engine.run_with_cancel(request, None, record.cancel_token.clone(),));
    recorder.set_runtime_identity(stream.runtime_identity().clone());
    if engine.run_mode() != RunMode::Review {
        // Review task state must not retain a workspace-owned Agent package
        // or instruction material, even if an embedding supplies one.
        recorder.set_agent_profile(stream.agent_profile().cloned());
    }
    // Capture the control handle so HTTP steer handlers can inject mid-run.
    {
        let _control_lifecycle = record.control_lifecycle_lock.lock().await;
        let handle = stream.control().clone();
        *record.control.lock().await = Some(handle);
        // A request can persist a steer after the product turn is claimed but
        // before this supervisor installs the in-memory handle. Replay the
        // bounded durable queue while holding the same lifecycle lock used by
        // route delivery, so no control can arrive between replay and the
        // live-handle handoff.
        replay_pending_product_steers(record).await;
    }
    let mut terminal = None;
    let mut protocol_invalid = false;
    let mut trace_complete = true;
    let mut saw_run_started = false;
    let mut startup_events = Some(startup_events);
    let review_mode = engine.run_mode() == RunMode::Review;
    while let Some(event) = stream.next().await {
        if matches!(&event, StreamEvent::RunCompleted { .. }) {
            if terminal.replace(event).is_some() {
                protocol_invalid = true;
            }
            continue;
        }
        if terminal.is_some() {
            protocol_invalid = true;
            continue;
        }
        if !saw_run_started {
            if !matches!(&event, StreamEvent::RunStarted { .. }) {
                protocol_invalid = true;
                continue;
            }
            saw_run_started = true;
        }
        let is_run_started = matches!(&event, StreamEvent::RunStarted { .. });
        let event = if review_mode {
            event.redacted_for_review_persistence()
        } else {
            event
        };
        // Reflect control lifecycle events back into ProductStore before the
        // canonical event is persisted and published.
        reflect_control_event(record, &event).await;
        trace_complete &= persist_record_and_publish_runtime_event(
            &run.trace_writer,
            record,
            event,
            recorder,
            state_store,
        )
        .await;
        if is_run_started {
            // Only expose the trace to concurrent API-originated control
            // events after its mandatory first event has been persisted.
            let _control_lifecycle = record.control_lifecycle_lock.lock().await;
            *record.control_event_trace.lock().await = Some(run.trace_writer.clone());
            let queued_product_events =
                std::mem::take(&mut *record.pending_product_events.lock().await);
            for queued_event in queued_product_events {
                let queued_event = if review_mode {
                    queued_event.redacted_for_review_persistence()
                } else {
                    queued_event
                };
                trace_complete &= persist_record_and_publish_runtime_event(
                    &run.trace_writer,
                    record,
                    queued_event,
                    recorder,
                    state_store,
                )
                .await;
            }
            for startup_event in startup_events.take().unwrap_or_default() {
                let startup_event = if review_mode {
                    startup_event.redacted_for_review_persistence()
                } else {
                    startup_event
                };
                // A claimed follow-up becomes applied only after its successor
                // has actually reached the durable run-start boundary. Keep the
                // ProductStore transition coupled to the same canonical event
                // path as ordinary stream events.
                reflect_control_event(record, &startup_event).await;
                trace_complete &= persist_record_and_publish_runtime_event(
                    &run.trace_writer,
                    record,
                    startup_event,
                    recorder,
                    state_store,
                )
                .await;
            }
        }
    }
    // Drop the control handle once the stream ends so further steers are rejected.
    {
        let _control_lifecycle = record.control_lifecycle_lock.lock().await;
        *record.control.lock().await = None;
    }
    if protocol_invalid || !saw_run_started {
        None
    } else {
        terminal.map(|terminal| (terminal, trace_complete))
    }
}

pub(crate) async fn reflect_control_event(record: &JobRecord, event: &StreamEvent) {
    let (Some(product_session_id), Some(store)) = (
        record.product_session_id.as_ref(),
        record.product_store.as_ref(),
    ) else {
        return;
    };
    match event {
        StreamEvent::SteerAccepted { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                tracing::warn!(control_id = %id, "steer_accepted id is not a product control id");
                return;
            };
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Accepted,
                    Some(&record.run_id),
                )
                .await
            {
                // Already accepted/applied is fine on replay.
                tracing::debug!(
                    control_id = %control_id,
                    "steer_accepted store transition: {error}"
                );
            }
        }
        StreamEvent::SteerApplied { id } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                tracing::warn!(control_id = %id, "steer_applied id is not a product control id");
                return;
            };
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await
            {
                // Replayed canonical events may encounter the already-applied
                // historical row. They must not mutate a completed fact.
                tracing::debug!(control_id = %control_id, "steer_applied store transition: {error}");
            }
        }
        StreamEvent::SteerDropped { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let pending = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Dropped,
                    Some(&record.run_id),
                )
                .await;
            if pending.is_err()
                && let Err(error) = store
                    .transition_control(
                        product_session_id,
                        &control_id,
                        ProductControlStatus::Accepted,
                        ProductControlStatus::Dropped,
                        Some(&record.run_id),
                    )
                    .await
            {
                tracing::debug!(
                    control_id = %control_id,
                    "steer_dropped store transition: {error}"
                );
            }
        }
        StreamEvent::FollowupDequeued { id } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            // Claim already moved pending→accepted; mark applied once the new run starts.
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await
            {
                tracing::debug!(
                    control_id = %control_id,
                    "followup_dequeued store transition: {error}"
                );
            }
        }
        StreamEvent::FollowupAbandoned { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Abandoned,
                    None,
                )
                .await;
            let _ = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Abandoned,
                    None,
                )
                .await;
        }
        StreamEvent::MessageInterventionRequested { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Accepted,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageAppliedCurrentRun { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageNeedsAttention { id, .. } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let pending = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Abandoned,
                    Some(&record.run_id),
                )
                .await;
            if pending.is_err() {
                let _ = store
                    .transition_control(
                        product_session_id,
                        &message_id,
                        ProductControlStatus::Accepted,
                        ProductControlStatus::Abandoned,
                        Some(&record.run_id),
                    )
                    .await;
            }
        }
        StreamEvent::MessageClaimedSuccessor { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageRevoked { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Revoked,
                    Some(&record.run_id),
                )
                .await;
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Abandoned,
                    ProductControlStatus::Revoked,
                    Some(&record.run_id),
                )
                .await;
        }
        _ => {}
    }
}

pub(crate) async fn finish_final_product_turn(
    product_turn: Option<ProductTurnSupervisor>,
    run_id: RunId,
) -> Option<ProductFollowupTurnClaim> {
    let product_turn = product_turn?;
    match product_turn
        .store
        .finish_session_turn_and_claim_followup(&product_turn.claim_id)
        .await
    {
        Ok(claim) => claim,
        Err(error) => {
            tracing::warn!("failed to atomically finish final product turn: {error}");
            finish_product_turn_needs_attention(
                Some(product_turn),
                Some(run_id),
                "final product-turn completion could not be committed",
            )
            .await;
            None
        }
    }
}

/// Close every steer which is still pending or only safe-point accepted before
/// the terminal fact is persisted. The runtime itself emits dropped events for
/// its in-memory channel; this covers the durable race window between that
/// channel closing and the product turn's terminal transaction.
pub(crate) async fn drop_unapplied_product_steers_before_terminal(
    product_turn: Option<&ProductTurnSupervisor>,
    record: &JobRecord,
    trace_writer: &TraceWriter,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) -> Result<bool, ProductStoreError> {
    let Some(product_turn) = product_turn else {
        return Ok(true);
    };
    let reason = "run completed before the steer reached a model turn";
    let dropped = product_turn
        .store
        .drop_unapplied_steers_for_turn(&product_turn.claim_id, record.run_id, reason)
        .await?;
    let mut trace_complete = true;
    for control in dropped {
        trace_complete &= persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::SteerDropped {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
    }
    Ok(trace_complete)
}

pub(crate) async fn finish_product_turn_needs_attention(
    product_turn: Option<ProductTurnSupervisor>,
    run_id: Option<RunId>,
    reason: &'static str,
) {
    let Some(product_turn) = product_turn else {
        return;
    };
    if let Err(error) = product_turn
        .store
        .finish_session_turn_and_abandon_pending_controls(
            &product_turn.claim_id,
            run_id,
            ProductSessionStatus::NeedsAttention,
            Some(ProductSessionOutcome::Failed),
            reason,
        )
        .await
    {
        tracing::warn!("failed to conservatively classify product turn controls: {error}");
    }
}

pub(crate) async fn finish_nonfinal_product_turn(
    product_turn: Option<ProductTurnSupervisor>,
    terminal: &StreamEvent,
    needs_attention: bool,
    record: &JobRecord,
    trace_writer: &TraceWriter,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) {
    let Some(product_turn) = product_turn else {
        return;
    };
    let terminal_status = terminal_run_status(terminal);
    let status = if needs_attention {
        ProductSessionStatus::NeedsAttention
    } else {
        match terminal_status {
            RunStatus::Error | RunStatus::Interrupted => ProductSessionStatus::Error,
            // Explicit cancellation is a known user decision. The existing
            // product continuation contract permits a later fresh turn from
            // its durable terminal state; queued follow-ups remain abandoned
            // and still require explicit confirmation.
            RunStatus::Cancelled => ProductSessionStatus::Idle,
            RunStatus::Done => ProductSessionStatus::NeedsAttention,
            RunStatus::Init | RunStatus::Running => ProductSessionStatus::NeedsAttention,
        }
    };
    let reason = match terminal_status {
        RunStatus::Done => "run completed without a final answer",
        RunStatus::Cancelled => "run cancelled",
        RunStatus::Error | RunStatus::Interrupted => "run did not complete normally",
        RunStatus::Init | RunStatus::Running => "run ended without a durable terminal",
    };
    // `Done` reaching this path means the run finished *without* a final answer,
    // so the turn did not succeed even though the run completed: the session is
    // deliberately moved to `needs_attention`, and the recorded outcome must
    // agree with that rather than claim a success the user never saw.
    let outcome = match terminal_status {
        RunStatus::Cancelled => ProductSessionOutcome::Cancelled,
        RunStatus::Done | RunStatus::Error | RunStatus::Interrupted => {
            ProductSessionOutcome::Failed
        }
        RunStatus::Init | RunStatus::Running => ProductSessionOutcome::Failed,
    };
    let finished = match product_turn
        .store
        .finish_session_turn_and_abandon_pending_controls(
            &product_turn.claim_id,
            Some(record.run_id),
            status,
            Some(outcome),
            reason,
        )
        .await
    {
        Ok(finished) => finished,
        Err(error) => {
            tracing::warn!("failed to atomically close non-final product turn: {error}");
            finish_product_turn_needs_attention(
                Some(product_turn),
                Some(record.run_id),
                "non-final product-turn completion could not be committed",
            )
            .await;
            return;
        }
    };
    for control in finished.dropped_steers {
        let _ = persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::SteerDropped {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
    }
    for control in finished.abandoned_followups {
        let _ = persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::FollowupAbandoned {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
    }
}

pub(crate) async fn recover_job_supervisor_panic(
    state: &ApiState,
    record: &JobRecord,
    product_turn: Option<ProductTurnSupervisor>,
) {
    let state_store = state_store_for_record(record);
    let terminal = match load_persisted_terminal_event(&state_store.index, record.run_id).await {
        Ok(Some(event)) => event,
        Ok(None) => supervisor_failure_event("job supervisor failed"),
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to inspect terminal event during supervisor recovery: {error}");
            supervisor_failure_event("job supervisor failed")
        }
    };
    reject_pending_approvals(record, &state_store.index).await;
    reject_pending_inputs(record, &state_store.index).await;
    let finish_outcome = AssertUnwindSafe(finish_product_turn_needs_attention(
        product_turn,
        Some(record.run_id),
        "job supervisor failed before control completion",
    ))
    .catch_unwind()
    .await;
    if finish_outcome.is_err() {
        tracing::warn!(job_id = %record.job_id, "product turn recovery panicked");
    }
    state.notify_product_events();
    let terminal_published = {
        let status = record.status.lock().await;
        is_terminal(&status)
    };
    if !terminal_published {
        append_job_event(record, terminal).await;
    }
}

pub(crate) async fn load_persisted_terminal_event(
    index: &StateIndex,
    run_id: RunId,
) -> std::io::Result<Option<StreamEvent>> {
    let Some(run) = load_runtime_run_record(index, run_id).await? else {
        return Ok(None);
    };
    if run.last_event_seq == 0 {
        return Ok(None);
    }
    let snapshot = index
        .run_event_snapshot_async(run_id, run.last_event_seq.saturating_sub(1), 1)
        .await?;
    let Some(event) = snapshot.and_then(|snapshot| snapshot.events.into_iter().next()) else {
        return Ok(None);
    };
    let event = serde_json::from_str::<StreamEvent>(&event.event_json)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if matches!(&event, StreamEvent::RunCompleted { .. }) {
        Ok(Some(event))
    } else {
        Ok(None)
    }
}

pub(crate) async fn runtime_terminal_is_durable(
    state_store: &StateStore,
    record: &JobRecord,
    terminal: &StreamEvent,
) -> bool {
    let run = match load_runtime_run_record(&state_store.index, record.run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to inspect finalized runtime run: {error}");
            return false;
        }
    };
    if run.session_id != record.session_id
        || run.job_id != record.job_id
        || run.run_id != record.run_id
        || run.status != run_status_label(&terminal_run_status(terminal))
        || run.task_state_path.is_none()
        || run.report_path.is_none()
        || run.last_event_seq == 0
    {
        return false;
    }
    let task_state = match state_store.load_task_state(record.run_id).await {
        Ok(task_state) => task_state,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized task state: {error}");
            return false;
        }
    };
    if task_state.session_id != record.session_id
        || task_state.job_id != record.job_id
        || task_state.run_id != record.run_id
        || task_state
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.last_event_seq)
            != Some(run.last_event_seq)
    {
        return false;
    }
    let report = match state_store.load_report(record.run_id).await {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized report: {error}");
            return false;
        }
    };
    if report.session_id != record.session_id
        || report.job_id != record.job_id
        || report.run_id != record.run_id
    {
        return false;
    }
    let persisted = match load_persisted_terminal_event(&state_store.index, record.run_id).await {
        Ok(Some(event)) => event,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized terminal event: {error}");
            return false;
        }
    };
    terminal_events_match(&persisted, terminal)
        && matches!(
            terminal,
            StreamEvent::RunCompleted { reason, .. }
                if termination_reasons_agree(&report.termination_reason, reason)
        )
}

/// Compare two termination reasons.
///
/// Reasons are a typed enum and the secret authority never rewrites one, so this
/// needs no normalization — unlike the output comparison next to it, which has
/// to account for the redaction the durable copy went through. Both reason
/// comparisons in `runtime_terminal_is_durable` go through this one function so
/// the report check and the event check cannot drift apart in intent.
pub(crate) fn termination_reasons_agree(
    left: &TerminationReason,
    right: &TerminationReason,
) -> bool {
    left == right
}

/// Compare the terminal event the supervisor saw with the one it reads back.
///
/// The durable copies are stored redacted — the trace line, the index row, and
/// the report all pass through the secret authority — so the comparison is made
/// in that same representation. A run whose final output happened to carry a
/// credential the runtime knows is still a run whose terminal fact was
/// persisted; without this, the durability check would report the redaction
/// itself as a lost terminal and move a genuinely successful session to
/// `needs_attention`.
///
/// # What this comparison can and cannot decide
///
/// The unredacted comparison is the primary path: identical values match, and
/// any divergence that redaction does not explain is a mismatch, so this is a
/// real equality check whenever no credential is involved.
///
/// Once a credential *is* involved the durable copy has forgotten which one it
/// held: redaction maps every registered credential to the same marker, and the
/// index row retains nothing else about it. Two terminals whose outputs differ
/// only in which registered credential they contain are therefore
/// indistinguishable here and are accepted as the same terminal. Deciding that
/// case would mean persisting a credential-derived fingerprint alongside the
/// run — a durable artifact derived from secret material, which is exactly what
/// this registry exists to avoid — so the ambiguity is registered as a residual
/// in design §14.3 rather than papered over. The check answers "was the terminal
/// fact persisted", which it does; it does not prove byte identity of an output
/// the durable copy deliberately no longer holds.
pub(crate) fn terminal_events_match(left: &StreamEvent, right: &StreamEvent) -> bool {
    let (
        StreamEvent::RunCompleted {
            reason: left_reason,
            output: left_output,
        },
        StreamEvent::RunCompleted {
            reason: right_reason,
            output: right_output,
        },
    ) = (left, right)
    else {
        return false;
    };
    if !termination_reasons_agree(left_reason, right_reason) {
        return false;
    }
    match (left_output, right_output) {
        (None, None) => true,
        (Some(durable), Some(live)) => {
            // Unredacted equality first: this is exact, and it is the whole
            // check whenever neither side carries a known credential.
            if durable == live {
                return true;
            }
            // Fall back only when the durable side really is a redacted copy,
            // and then require the live value's redaction to reproduce the
            // durable value *exactly*. A difference the authority does not
            // account for is still a mismatch, which is what keeps this from
            // becoming "two redactions that happen to agree".
            //
            // The marker conjunct is subsumed by the equality conjunct today:
            // [`SecretRegistry::redact_text`] writes only `KNOWN_SECRET_MARKER`,
            // so a durable value that differs from the live one and is
            // reproduced by that call necessarily contains the marker. It is
            // kept as an explicit precondition of the fallback — it is what the
            // gate is asking — and it starts deciding the moment `redact_text`
            // could change a value without writing that marker, for instance if
            // it grew the pattern backstop. The test in this module pins the
            // equality conjunct, which is the one that decides today.
            durable.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER)
                && rove_runtime::secrets::registry().redact_text(live) == *durable
        }
        _ => false,
    }
}

pub(crate) async fn load_runtime_run_record(
    index: &StateIndex,
    run_id: RunId,
) -> std::io::Result<Option<RunIndexRecord>> {
    let index = index.clone();
    tokio::task::spawn_blocking(move || index.run_record(run_id))
        .await
        .map_err(std::io::Error::other)?
}

pub(crate) fn supervisor_failure_event(message: &str) -> StreamEvent {
    StreamEvent::RunCompleted {
        reason: TerminationReason::Error,
        output: Some(message.to_string()),
    }
}

pub(crate) fn terminal_run_status(event: &StreamEvent) -> RunStatus {
    match event {
        StreamEvent::RunCompleted { reason, .. } => status_for_reason(reason),
        _ => RunStatus::Error,
    }
}

pub(crate) fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Init => "init",
        RunStatus::Running => "running",
        RunStatus::Done => "done",
        RunStatus::Error => "error",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
    }
}

pub(crate) async fn append_trace_event(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: &StreamEvent,
) -> bool {
    let _sequence = record.control_event_trace_lock.lock().await;
    append_trace_event_unlocked(trace_writer, record, event)
}

pub(crate) fn append_trace_event_unlocked(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: &StreamEvent,
) -> bool {
    if let Err(error) = trace_writer.append(event) {
        tracing::warn!(job_id = %record.job_id, run_id = %record.run_id, "failed to append runtime trace event: {error}");
        return false;
    }
    true
}

/// Persist one runtime-owned event, project it into the resumable artifacts,
/// then make it visible to live SSE consumers. The ordering ensures a client
/// never observes an event that cannot subsequently be replayed from the
/// canonical run index.
pub(crate) async fn persist_record_and_publish_runtime_event(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: StreamEvent,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) -> bool {
    if !append_trace_event(trace_writer, record, &event).await {
        return false;
    }
    recorder.record_event(&event, state_store).await;
    append_job_event(record, event).await;
    true
}

/// Persist the terminal event and finish the artifact projections before the
/// product supervisor settles its turn claim. Publication is deliberately a
/// separate final step: product clients treat the visible terminal event as
/// proof that their session is idle or has an atomically claimed successor.
pub(crate) async fn persist_terminal_and_finalize(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    terminal: StreamEvent,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
    engine: &Engine,
    run: &RunHandle,
) -> bool {
    let persisted = append_trace_event(trace_writer, record, &terminal).await;
    recorder.record_event(&terminal, state_store).await;
    recorder
        .finalize(
            state_store,
            engine.workspace(),
            engine.model_id(),
            &run.run_dir,
        )
        .await;
    persisted
}

pub(crate) async fn publish_terminal_event(record: &JobRecord, terminal: StreamEvent) {
    append_job_event(record, terminal).await;
    *record.control_event_trace.lock().await = None;
}

/// Queue an API-originated product control event until `run_started` is
/// durable, or persist and publish it immediately when the run trace is live.
/// Callers hold `control_lifecycle_lock`, which also serializes this decision
/// against terminal cleanup.
pub(crate) async fn queue_or_publish_product_control_event(record: &JobRecord, event: StreamEvent) {
    let persisted = {
        let _sequence = record.control_event_trace_lock.lock().await;
        let trace = record.control_event_trace.lock().await.clone();
        match trace {
            Some(trace) => append_trace_event_unlocked(&trace, record, &event),
            None => {
                record.pending_product_events.lock().await.push(event);
                return;
            }
        }
    };
    if persisted {
        append_job_event(record, event).await;
    } else {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "failed to persist API-originated product control event"
        );
    }
}
