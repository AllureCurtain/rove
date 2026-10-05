//! Product fork boundary verification and fork-source state projection.

use super::*;

/// Verify a fork request against the parent session's immutable product binding
/// and the canonical terminal runtime artifacts. The browser supplies only a
/// run id and, for edit-and-resend, a message-ledger sequence; every other
/// source identity and the terminal event sequence come from server-owned
/// state.
pub(crate) async fn verify_product_fork_boundary(
    state: &ApiState,
    parent_session_id: &ProductSessionId,
    fork_at_run_id: RunId,
    truncate_after_message_seq: Option<i64>,
) -> Result<VerifiedProductForkBoundary, ApiError> {
    let store = state.product_store()?;
    let context = store.get_session_context(parent_session_id).await?;
    if context.session.status == ProductSessionStatus::Running {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionActive,
            "a product session can only be forked after its active turn reaches a terminal boundary",
        )
        .into());
    }
    if context.session.status != ProductSessionStatus::Idle {
        return Err(fork_source_rejection(
            "only an idle product session with a final durable run can be forked",
        ));
    }
    let bindings = store.list_run_bindings(parent_session_id).await?;
    let Some(binding) = bindings
        .iter()
        .find(|binding| binding.runtime_run_id == fork_at_run_id)
    else {
        return Err(fork_source_rejection(
            "the requested runtime run is not bound to the parent product session",
        ));
    };
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;
    let (source_state, terminal_event_seq) = load_final_fork_source_state(
        &state_store,
        binding.runtime_session_id,
        binding.runtime_job_id,
        binding.runtime_run_id,
        None,
    )
    .await?;
    let truncate_after = match truncate_after_message_seq {
        None => None,
        Some(seq) => Some(
            verify_fork_truncation_target(
                &store,
                parent_session_id,
                fork_at_run_id,
                seq,
                &source_state,
            )
            .await?,
        ),
    };
    Ok(VerifiedProductForkBoundary {
        parent_product_session_id: context.session.id,
        parent_workspace_id: context.workspace.id,
        parent_title: context.session.title,
        source_runtime_session_id: binding.runtime_session_id,
        source_runtime_job_id: binding.runtime_job_id,
        source_runtime_run_id: binding.runtime_run_id,
        fork_at_event_seq: terminal_event_seq,
        truncate_after,
    })
}

/// Resolve the user message an edit-and-resend fork cuts at.
///
/// The sequence is a parent message-ledger sequence, which is also the
/// sequence the product message contract exposes to clients. It must name a
/// user message that the fork run itself delivered, and that message must
/// still resolve to a user entry in the loaded source session: anything else
/// would let a client cut the child's seed at content it never owned.
pub(crate) async fn verify_fork_truncation_target(
    store: &Arc<dyn ProductStore>,
    parent_session_id: &ProductSessionId,
    fork_at_run_id: RunId,
    seq: i64,
    source_state: &TaskState,
) -> Result<VerifiedForkTruncation, ApiError> {
    if seq < 1 {
        return Err(invalid_fork_truncation(
            "truncate_after_message_seq must be a positive message sequence",
        ));
    }
    let Some(message) = store.find_message_by_seq(parent_session_id, seq).await? else {
        // The ledger holds steer controls and unified user messages in one
        // sequence space, so this also covers a sequence that names an
        // assistant or tool-result event in a different (per-run) sequence
        // space: it is simply not a product message.
        return Err(invalid_fork_truncation(
            "no product user message in this session has that sequence",
        ));
    };
    let delivered_run = message.successor_run_id.or(message.run_id);
    if delivered_run.as_ref() != Some(&fork_at_run_id) {
        return Err(fork_source_rejection(
            "the truncation target message does not belong to the fork run",
        ));
    }
    resolve_fork_truncation_entry(source_state, &message, fork_at_run_id)?;
    Ok(VerifiedForkTruncation {
        message_seq: seq,
        message_id: message.id,
    })
}

pub(crate) fn invalid_fork_truncation(message: impl Into<String>) -> ApiError {
    ProductStoreError::new(ProductErrorCode::ProductInvalidInput, message).into()
}

/// The canonical session entry a truncation target cuts at.
///
/// `Session.entries` ids are deterministic per delivery: a message that
/// started this run is `user-{run_id}`, while a message promoted into the
/// already-running run is `message-{control_id}`. Resolving against the loaded
/// session — instead of trusting the ledger row — is what proves the cut lands
/// on a user turn that this run actually produced.
pub(crate) fn fork_truncation_entry_id(
    source_state: &TaskState,
    message_id: &ProductControlId,
    fork_at_run_id: RunId,
) -> Option<String> {
    let session = source_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.session.as_ref())?;
    let promoted = format!("message-{message_id}");
    if session.entries.iter().any(|entry| entry.id() == promoted) {
        return Some(promoted);
    }
    let trigger = format!("user-{fork_at_run_id}");
    session
        .entries
        .iter()
        .any(|entry| entry.id() == trigger)
        .then_some(trigger)
}

pub(crate) fn resolve_fork_truncation_entry(
    source_state: &TaskState,
    message: &ProductMessage,
    fork_at_run_id: RunId,
) -> Result<String, ApiError> {
    let Some(entry_id) = fork_truncation_entry_id(source_state, &message.id, fork_at_run_id) else {
        return Err(fork_source_rejection(
            "the truncation target message is not part of the fork source history",
        ));
    };
    let session = source_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.session.as_ref())
        .ok_or_else(|| {
            fork_source_rejection("the fork source has no canonical session history to cut")
        })?;
    let entry = session
        .entries
        .iter()
        .find(|entry| entry.id() == entry_id)
        .ok_or_else(|| {
            fork_source_rejection("the truncation target entry is missing from the source history")
        })?;
    if !matches!(entry, SessionEntry::User { .. }) {
        return Err(invalid_fork_truncation(
            "truncate_after_message_seq must name a user message",
        ));
    }
    Ok(entry_id)
}

pub(crate) async fn load_product_fork_resume(
    state_store: &StateStore,
    fork: &ProductFork,
) -> Result<TaskState, ApiError> {
    let (state, _) = load_final_fork_source_state(
        state_store,
        fork.source_runtime_session_id,
        fork.source_runtime_job_id,
        fork.source_runtime_run_id,
        Some(fork.fork_at_event_seq),
    )
    .await?;
    let state = project_product_follow_up_state(state)?;
    truncate_product_fork_state(state, fork)
}

/// Build the child's private starting state from the parent's terminal state.
///
/// Only the copy handed to the child is cut: the parent's `task_state.json`,
/// its `trace.jsonl`, and its session row are never written here. The cut
/// removes the target user message and everything after it, then re-derives the
/// legacy projections from the pruned canonical session so no second copy of
/// the removed content survives in the seed.
pub(crate) fn truncate_product_fork_state(
    state: TaskState,
    fork: &ProductFork,
) -> Result<TaskState, ApiError> {
    let (Some(message_seq), Some(message_id)) = (
        fork.truncate_after_message_seq,
        fork.truncate_after_message_id.as_ref(),
    ) else {
        if fork.truncate_after_message_seq.is_some() || fork.truncate_after_message_id.is_some() {
            return Err(fork_source_rejection(
                "the stored fork truncation is incomplete and cannot be applied",
            ));
        }
        return Ok(state);
    };
    let mut state = state;
    let entry_id = fork_truncation_entry_id(&state, message_id, fork.source_runtime_run_id)
        .ok_or_else(|| {
            fork_source_rejection(format!(
                "the truncation target message {message_seq} is not part of the fork source history"
            ))
        })?;
    // The inherited summary is the source run's terminal summary, and the cut
    // always lands inside that run: the summary describes the edited message and
    // the work that followed it. A fork child owns neither, so it goes with the
    // entries the cut removes instead of reaching the child as session memory.
    state.summary = None;
    let Some(checkpoint) = state.checkpoint.as_mut() else {
        return Err(fork_source_rejection(
            "the fork source has no canonical session history to cut",
        ));
    };
    let Some(session) = checkpoint.session.as_mut() else {
        return Err(fork_source_rejection(
            "the fork source has no canonical session history to cut",
        ));
    };
    let removed = session.truncate_from(&entry_id).map_err(|error| {
        fork_source_rejection(format!(
            "the fork truncation target cannot be removed from the source history: {error}"
        ))
    })?;
    if !matches!(removed, SessionEntry::User { .. }) {
        return Err(invalid_fork_truncation(
            "truncate_after_message_seq must name a user message",
        ));
    }
    // The cut can only land between turns, but a seed whose last message is an
    // unanswered tool call would not project. Closing it keeps the child's
    // first prompt well-formed and leaves the effect explicitly unknown.
    session.close_unresolved_tool_calls().map_err(|error| {
        fork_source_rejection(format!("the fork truncation left an invalid seed: {error}"))
    })?;
    let pruned = session.clone();
    checkpoint.preserved_tail = pruned
        .suffix(rove_runtime::session::CHECKPOINT_SESSION_TAIL_ENTRIES)
        .messages_for_compatibility_artifact()
        .map_err(|error| {
            fork_source_rejection(format!("the fork truncation left an invalid seed: {error}"))
        })?;
    checkpoint.history_pruned = true;
    checkpoint.summary = None;
    state.history = pruned
        .messages_for_compatibility_artifact()
        .map_err(|error| {
            fork_source_rejection(format!("the fork truncation left an invalid seed: {error}"))
        })?;
    Ok(state)
}

pub(crate) async fn load_final_fork_source_state(
    state_store: &StateStore,
    runtime_session_id: SessionId,
    runtime_job_id: JobId,
    runtime_run_id: RunId,
    expected_terminal_event_seq: Option<u64>,
) -> Result<(TaskState, u64), ApiError> {
    let job = state_store
        .index
        .job_record_async(runtime_job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source runtime job is missing"))?;
    if job.session_id != runtime_session_id || job.run_id != Some(runtime_run_id) {
        return Err(fork_source_rejection(
            "the fork source runtime job identity does not match its product binding",
        ));
    }
    let run = load_runtime_run_record(&state_store.index, runtime_run_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source runtime run is missing"))?;
    if run.session_id != runtime_session_id
        || run.job_id != runtime_job_id
        || run.run_id != runtime_run_id
        || run.status != "done"
        || run.task_state_path.is_none()
        || run.report_path.is_none()
        || run.last_event_seq == 0
    {
        return Err(fork_source_rejection(
            "the fork source runtime run is not a complete durable terminal boundary",
        ));
    }
    if expected_terminal_event_seq.is_some_and(|expected| expected != run.last_event_seq) {
        return Err(fork_source_rejection(
            "the fork source terminal event sequence no longer matches its stored boundary",
        ));
    }
    let task_state = state_store
        .load_task_state(runtime_run_id)
        .await
        .map_err(|_| {
            fork_source_rejection("the fork source task state is unavailable or corrupt")
        })?;
    if task_state.session_id != runtime_session_id
        || task_state.job_id != runtime_job_id
        || task_state.run_id != runtime_run_id
        || task_state
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.last_event_seq)
            != Some(run.last_event_seq)
    {
        return Err(fork_source_rejection(
            "the fork source task-state identity or terminal checkpoint is invalid",
        ));
    }
    let report = state_store
        .load_report(runtime_run_id)
        .await
        .map_err(|_| fork_source_rejection("the fork source report is unavailable or corrupt"))?;
    if report.session_id != runtime_session_id
        || report.job_id != runtime_job_id
        || report.run_id != runtime_run_id
        || report.status != "success"
        || report.termination_reason != TerminationReason::Final
    {
        return Err(fork_source_rejection(
            "the fork source report does not prove a final completed run",
        ));
    }
    let snapshot = state_store
        .index
        .run_event_snapshot_async(runtime_run_id, run.last_event_seq.saturating_sub(1), 1)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source canonical event is missing"))?;
    if snapshot.high_water_seq != run.last_event_seq
        || snapshot.has_more
        || snapshot.events.len() != 1
        || snapshot.events[0].seq != run.last_event_seq
    {
        return Err(fork_source_rejection(
            "the fork source canonical terminal event range is incomplete",
        ));
    }
    let terminal = serde_json::from_str::<StreamEvent>(&snapshot.events[0].event_json)
        .map_err(|_| fork_source_rejection("the fork source terminal event is corrupt"))?;
    if !matches!(
        terminal,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        }
    ) {
        return Err(fork_source_rejection(
            "the fork source does not end with a final canonical completion event",
        ));
    }
    Ok((task_state, run.last_event_seq))
}

pub(crate) fn fork_source_rejection(message: impl Into<String>) -> ApiError {
    ProductStoreError::new(ProductErrorCode::ProductForkSourceInvalid, message).into()
}

pub(crate) fn project_product_follow_up_state(mut state: TaskState) -> Result<TaskState, ApiError> {
    // A product follow-up is a new user turn in the same durable conversation,
    // not a replay of the previous turn's terminal execution decision.
    state.step = 0;
    state.plan = None;
    state.step_ledger = Default::default();
    state.history = close_product_follow_up_tool_rounds(state.history)?;
    if let Some(checkpoint) = state.checkpoint.as_mut() {
        checkpoint.last_step = 0;
        checkpoint.plan = None;
        checkpoint.step_ledger = Default::default();
        checkpoint.last_event_seq = None;
        checkpoint.preserved_tail =
            close_product_follow_up_tool_rounds(std::mem::take(&mut checkpoint.preserved_tail))?;
    }
    Ok(state)
}

pub(crate) const UNKNOWN_PRODUCT_TOOL_RESULT: &str = "The previous turn ended before a durable tool result was recorded. The tool effect is unknown; verify the current state before retrying this tool call.";

pub(crate) fn close_product_follow_up_tool_rounds(
    messages: Vec<Message>,
) -> Result<Vec<Message>, ApiError> {
    let mut closed = Vec::with_capacity(messages.len());
    let mut pending_tool_call_ids = Vec::new();
    let mut completed_tool_call_ids = HashSet::new();

    for message in messages {
        if message.role == Role::Tool {
            let Some(tool_call_id) = message.tool_call_id.as_ref() else {
                append_missing_product_tool_results(
                    &mut closed,
                    &pending_tool_call_ids,
                    &completed_tool_call_ids,
                );
                pending_tool_call_ids.clear();
                completed_tool_call_ids.clear();
                closed.push(message);
                continue;
            };
            if tool_call_id.trim().is_empty() {
                return Err(invalid_product_tool_history(
                    "the product session runtime history contains an empty tool result call id",
                ));
            }
            if pending_tool_call_ids.contains(tool_call_id)
                && completed_tool_call_ids.insert(tool_call_id.clone())
            {
                closed.push(message);
            }
            continue;
        }

        append_missing_product_tool_results(
            &mut closed,
            &pending_tool_call_ids,
            &completed_tool_call_ids,
        );
        pending_tool_call_ids.clear();
        completed_tool_call_ids.clear();

        if message.role == Role::Assistant && !message.tool_calls.is_empty() {
            let mut round_tool_call_ids = HashSet::new();
            for tool_call in &message.tool_calls {
                if tool_call.id.trim().is_empty() {
                    return Err(invalid_product_tool_history(
                        "the product session runtime history contains an empty assistant tool call id",
                    ));
                }
                if !round_tool_call_ids.insert(tool_call.id.clone()) {
                    return Err(invalid_product_tool_history(
                        "the product session runtime history contains duplicate assistant tool call ids",
                    ));
                }
                pending_tool_call_ids.push(tool_call.id.clone());
            }
        }
        closed.push(message);
    }

    append_missing_product_tool_results(
        &mut closed,
        &pending_tool_call_ids,
        &completed_tool_call_ids,
    );
    Ok(closed)
}

pub(crate) fn invalid_product_tool_history(message: &'static str) -> ApiError {
    ProductStoreError::new(ProductErrorCode::ProductSessionRuntimeStateCorrupt, message).into()
}

pub(crate) fn append_missing_product_tool_results(
    messages: &mut Vec<Message>,
    pending_tool_call_ids: &[String],
    completed_tool_call_ids: &HashSet<String>,
) {
    for tool_call_id in pending_tool_call_ids {
        if !completed_tool_call_ids.contains(tool_call_id) {
            messages.push(Message::tool(
                UNKNOWN_PRODUCT_TOOL_RESULT,
                Some(tool_call_id.clone()),
            ));
        }
    }
}
