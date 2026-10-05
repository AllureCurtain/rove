//! Follow-up (successor) delivery: launch, requeue/redrive, drain, steer replay.

use super::*;

pub(crate) async fn prepare_followup_job_launch(
    state: &ApiState,
    product_session_id: &ProductSessionId,
    claim: ProductFollowupTurnClaim,
) -> Result<JobLaunch, ApiError> {
    let store = state.product_store()?;
    let request = CreateJobRequest {
        // The raw stored text: the model-facing composition (attachment
        // blocks, image projection, degradation) happens inside
        // `prepare_claimed_product_job_launch`, where the run's resolved
        // Provider config decides how an image is carried.
        message: claim.control.content.clone(),
        model: None,
        max_steps: None,
        agent: None,
        approval: None,
        resume: None,
        workspace: None,
        provider: None,
        product_session_id: Some(product_session_id.clone()),
    };
    let approval_policy = match resolve_product_job_approval_policy(state).await {
        Ok(policy) => policy,
        Err(error) => {
            requeue_failed_followup_start(
                state,
                product_session_id,
                &store,
                &claim.turn.claim_id,
                &claim.control.id,
                "approval policy resolution",
                error.code,
            )
            .await;
            return Err(error);
        }
    };
    prepare_claimed_product_job_launch(
        state,
        &request,
        product_session_id,
        store,
        claim.turn,
        approval_policy,
        Some(claim.control.id),
    )
    .await
}

/// The exact user message and image blocks one queued follow-up contributes to
/// the model request.
///
/// The stored `content` is the text the client sent and is never rewritten: the
/// transcript, the `MessageQueued` event, and the idempotency digest all keep
/// it. Only the *model request* gains the attachment blocks, and only when the
/// message actually references one — so a session with no attachments composes
/// to exactly its own text, byte for byte. Image references project content
/// blocks only when `images_supported` says the resolved Provider accepts them;
/// the rest become labelled text lines through the same renderer.
///
/// An attachment that cannot be resolved degrades to a labelled omission inside
/// the composed message. Nothing here fails the turn: a file that vanished
/// between send and launch must not strand a message the user already sent.
pub(crate) async fn compose_message_for_model(
    store: &dyn ProductStore,
    storage: Option<&product::attachments::AttachmentStorage>,
    product_session_id: &ProductSessionId,
    control_id: &ProductControlId,
    content: &str,
    images_supported: bool,
) -> (String, Vec<rove_models::ContentBlock>) {
    let Some(storage) = storage else {
        return (content.to_string(), Vec::new());
    };
    let Ok(message) = store.get_message(product_session_id, control_id).await else {
        return (content.to_string(), Vec::new());
    };
    if message.attachments.is_empty() {
        return (content.to_string(), Vec::new());
    }
    let attachments = product::attachments::resolve_message_attachments(
        storage,
        store,
        product_session_id,
        &message.attachments,
    )
    .await;
    let (mapped, blocks) =
        product::attachments::map_attachments_for_model(attachments, images_supported);
    (
        rove_runtime::conversation::compose_user_message(content, &mapped),
        blocks,
    )
}

/// Release a claimed successor whose start failed for a reason a retry cannot
/// clear: a Provider identity that no longer resumes, a corrupted runtime
/// state, a lost claim, or a run that could not be bound. Abandoning is what
/// makes the loss visible — the session moves to `needs_attention` and the
/// message keeps its reason until the user revokes or confirms it.
pub(crate) async fn abandon_failed_followup_start(
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    control_id: &ProductControlId,
    phase: &'static str,
) {
    if let Err(error) = store
        .abandon_followup_turn(claim_id, control_id, phase)
        .await
    {
        tracing::warn!(
            control_id = %control_id,
            phase = phase,
            "failed to abandon automatic follow-up turn: {error}"
        );
    }
}

/// Release a claimed successor whose start failed in a way a later attempt may
/// still clear, and make sure something will attempt it.
///
/// Requeueing alone leaves the session `idle` with a `pending` successor and
/// nothing that will ever claim it: no timer, no other drain call site. So a
/// requeued successor is always followed by one of two things — a delayed
/// re-drain of the same session, or, once `FOLLOWUP_START_MAX_ATTEMPTS` failed
/// starts have accumulated, the visible `needs_attention` state. Neither is
/// silent: every release logs the session, the control, the attempt number, and
/// the typed failure.
pub(crate) async fn requeue_failed_followup_start(
    state: &ApiState,
    product_session_id: &ProductSessionId,
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    control_id: &ProductControlId,
    phase: &'static str,
    failure: &str,
) {
    let attempt = state.record_followup_start_failure(control_id).await;
    if attempt >= FOLLOWUP_START_MAX_ATTEMPTS {
        let reason =
            format!("successor start failed {attempt} times; last failure: {phase} ({failure})");
        // The budget ends here whether or not the escalation itself commits. A
        // failed abandon leaves the claim in place for the store's own
        // stale-claim reconciliation, and a later start of this successor must
        // not begin a fresh budget as if nothing had failed.
        state.clear_followup_start_failures(control_id).await;
        tracing::warn!(
            product_session_id = %product_session_id,
            control_id = %control_id,
            attempts = attempt,
            phase = phase,
            failure = failure,
            "queued successor exhausted its start attempts; marking the session needs_attention"
        );
        if let Err(error) = store
            .abandon_followup_turn(claim_id, control_id, &reason)
            .await
        {
            tracing::warn!(
                product_session_id = %product_session_id,
                control_id = %control_id,
                "failed to abandon an exhausted follow-up start: {error}"
            );
            return;
        }
        state.notify_product_events();
        return;
    }

    if let Err(error) = store.requeue_followup_turn(claim_id, control_id).await {
        // Nothing was released, so there is nothing to re-drain and no reason to
        // hold the claim against a later start: forget the attempts as well.
        state.clear_followup_start_failures(control_id).await;
        tracing::warn!(
            product_session_id = %product_session_id,
            control_id = %control_id,
            attempt = attempt,
            phase = phase,
            failure = failure,
            "failed to requeue automatic follow-up turn: {error}"
        );
        return;
    }
    let delay = followup_redrive_backoff(attempt);
    tracing::warn!(
        product_session_id = %product_session_id,
        control_id = %control_id,
        attempt = attempt,
        max_attempts = FOLLOWUP_START_MAX_ATTEMPTS,
        next_attempt = attempt + 1,
        retry_in_ms = delay.as_millis() as u64,
        phase = phase,
        failure = failure,
        "requeued a successor whose start failed; re-scheduling its drain"
    );
    state.notify_product_events();
    schedule_followup_redrive(state, product_session_id, control_id, attempt, delay);
}

/// The delay before the re-drain that follows failed attempt `failed_attempt`:
/// exponential from `FOLLOWUP_REDRIVE_INITIAL_BACKOFF`, capped.
pub(crate) fn followup_redrive_backoff(failed_attempt: u32) -> Duration {
    let doublings = failed_attempt.saturating_sub(1).min(3);
    FOLLOWUP_REDRIVE_INITIAL_BACKOFF
        .saturating_mul(1 << doublings)
        .min(FOLLOWUP_REDRIVE_MAX_BACKOFF)
}

/// Re-drain one session after a bounded backoff.
///
/// This is a delay in front of the existing drain, not a second drain path: it
/// adds no guard of its own, because `drain_followup_for_session` still claims
/// the store's single per-session turn. An extra wake-up racing a user send or
/// boot recovery therefore claims nothing and is harmless.
pub(crate) fn schedule_followup_redrive(
    state: &ApiState,
    session_id: &ProductSessionId,
    control_id: &ProductControlId,
    failed_attempt: u32,
    delay: Duration,
) {
    if state.inner.shutdown_token.is_cancelled() || state.inner.job_starts.is_closed() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    let shutdown = state.inner.shutdown_token.clone();
    let session_id = session_id.clone();
    let control_id = control_id.clone();
    drop(job_starts.spawn(async move {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        tracing::info!(
            product_session_id = %session_id,
            control_id = %control_id,
            attempt = failed_attempt + 1,
            "re-draining a requeued successor after its backoff"
        );
        drain_followup_for_session(state, session_id).await;
    }));
}

pub(crate) async fn replay_pending_product_steers(record: &JobRecord) {
    let (Some(session_id), Some(store)) = (
        record.product_session_id.as_ref(),
        record.product_store.as_ref(),
    ) else {
        return;
    };
    let pending = match store
        .list_controls(session_id, Some(ProductControlStatus::Pending))
        .await
    {
        Ok(controls) => controls,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to load pending steer controls: {error}");
            return;
        }
    };
    let handle = record.control.lock().await.clone();
    let Some(handle) = handle else {
        return;
    };
    for control in pending
        .into_iter()
        .filter(|control| control.kind == ProductControlKind::Steer)
    {
        let unified = store.get_message(session_id, &control.id).await.ok();
        // A promoted message keeps its attachments, so re-delivering it after a
        // run start must project them exactly as the live-handle path does.
        // Otherwise the same message would reach the model with its attachment
        // blocks present or absent depending on which code path won the race.
        let attachments = match (unified.as_ref(), record.attachment_storage.as_ref()) {
            (Some(message), Some(storage)) if !message.attachments.is_empty() => {
                product::attachments::resolve_message_attachments(
                    storage,
                    store.as_ref(),
                    session_id,
                    &message.attachments,
                )
                .await
            }
            _ => Vec::new(),
        };
        // The replayed steer must project exactly what the live-handle path
        // would have delivered, including how the run's model carries images.
        let images_supported = record
            .product_model_config
            .as_ref()
            .map(|model| model_supports_images(&record.config, &model.model))
            .unwrap_or(false);
        let (attachments, blocks) =
            product::attachments::map_attachments_for_model(attachments, images_supported);
        let steer = if unified.is_some() {
            rove_runtime::engine::SteerMessage::for_message(control.id.as_str(), control.content)
                .with_attachments(attachments)
                .with_content_blocks(blocks)
        } else {
            rove_runtime::engine::SteerMessage::with_id(control.id.as_str(), control.content)
        };
        if !handle.try_send_steer(steer) {
            tracing::warn!(
                job_id = %record.job_id,
                control_id = %control.id,
                "pending steer could not be replayed into the bounded runtime channel"
            );
            // The persistent pending bound mirrors the runtime channel bound,
            // so this is only possible if a concurrently delivered control
            // filled the channel. Leave this and later controls pending; the
            // next safe-point/attachment pass will classify or deliver them.
            break;
        }
    }
}

/// Start a server-owned queued follow-up drain without nesting it inside the
/// just-completed run supervisor. The product-store CAS makes duplicate
/// scheduler wake-ups harmless.
pub(crate) fn schedule_followup_drain(state: &ApiState, session_id: ProductSessionId) {
    if state.inner.shutdown_token.is_cancelled() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        drain_followup_for_session(state, session_id).await;
    }));
}

/// Claim the next pending follow-up (if any) and start it as a fresh durable
/// product run. The store atomically claims both the control and session turn;
/// run-id reservation makes post-reservation interruption conservative.
pub(crate) async fn drain_followup_for_session(state: ApiState, session_id: ProductSessionId) {
    if state.inner.shutdown_token.is_cancelled() {
        return;
    }
    let Ok(store) = state.product_store() else {
        return;
    };
    let claimed = match store.claim_next_followup_turn(&session_id).await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                product_session_id = %session_id,
                "failed to claim pending follow-up turn: {error}"
            );
            return;
        }
    };
    let Some(claimed) = claimed else {
        return;
    };
    let control_id = claimed.control.id.clone();
    tracing::info!(
        product_session_id = %session_id,
        control_id = %control_id,
        "starting claimed follow-up as a new product run"
    );

    match prepare_followup_job_launch(&state, &session_id, claimed).await {
        Ok(launch) => {
            state.clear_followup_start_failures(&control_id).await;
            start_job_supervisor(state, launch).await
        }
        Err(error) => tracing::warn!(
            product_session_id = %session_id,
            control_id = %control_id,
            "failed to prepare follow-up job: {error:?}"
        ),
    }
}

/// Recover safe queued follow-ups once an API process is serving requests.
/// Stale automatic turn claims are handled synchronously when the store opens:
/// only claims without a reserved runtime run id return to `pending`.
pub(crate) async fn recover_pending_followup_drains(state: ApiState) {
    let Ok(store) = state.product_store() else {
        return;
    };
    let sessions = match store.list_idle_sessions_with_pending_followups().await {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!("failed to list queued follow-ups for recovery: {error}");
            return;
        }
    };
    for session_id in sessions {
        schedule_followup_drain(&state, session_id);
    }
}

/// Start a follow-up that was already claimed while its previous run was
/// finalizing. It deliberately bypasses the generic drain because the
/// exclusive store claim already exists.
pub(crate) fn schedule_claimed_followup_start(
    state: &ApiState,
    session_id: ProductSessionId,
    claim: ProductFollowupTurnClaim,
) {
    if state.inner.shutdown_token.is_cancelled() || state.inner.job_starts.is_closed() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        let control_id = claim.control.id.clone();
        match prepare_followup_job_launch(&state, &session_id, claim).await {
            Ok(launch) => start_job_supervisor(state, launch).await,
            Err(error) => tracing::warn!(
                product_session_id = %session_id,
                control_id = %control_id,
                "failed to prepare atomically claimed follow-up job: {error:?}"
            ),
        }
    }));
}
