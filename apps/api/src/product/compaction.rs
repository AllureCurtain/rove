//! `POST /product/sessions/{session_id}/compact`.
//!
//! The API half of the CLI's `/compact`. It reuses `Engine::compact_resume_state`
//! instead of growing a second compaction, adds the state persistence the CLI
//! does not need (its snapshot is in-memory and the next prompt's run writes it),
//! and enforces the idleness the runtime does not.
//!
//! No run and no job are started: compaction edits state the session already
//! owns, so nothing executes, there is nothing to cancel, and no trace is
//! opened. `trace.jsonl` keeps the full history of the runs that produced the
//! compaction input.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use rove_runtime::compaction::ManualCompactionOutcome;
use rove_runtime::types::{PromptCheckpoint, PromptCompactionMode, RunId};
use tokio_util::sync::CancellationToken;

use crate::{ApiError, ApiErrorResponse, ApiState, docs};

use super::usage::compaction_mode_name;
use super::{
    PRODUCT_COMPACTION_SUMMARY_EXCERPT_CHARS, ProductErrorCode, ProductSessionCompaction,
    ProductSessionId, ProductSessionStatus, ProductStore, ProductStoreError, ProductTurnClaim,
    ProductTurnClaimId,
};

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/compact",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = String, Path, description = "Product session ULID")),
    responses(
        (status = 200, description = "Compaction facts for the session", body = ProductSessionCompaction),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 409, description = "The session is not idle, or its runtime state cannot be compacted", body = ApiErrorResponse),
        (status = 500, description = "The compacted state could not be persisted", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse),
    )
)]
pub(crate) async fn compact_product_session(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductSessionCompaction>, ApiError> {
    let store = state.product_store()?;
    // The exclusivity is the turn claim, not a status read: a compact racing a
    // turn loses the claim, and a compact racing another compact is refused the
    // same way instead of interleaving two rewrites of one snapshot.
    let claim = store.claim_session_turn(&session_id).await?;
    // The claim is owned by a guard rather than by the linear code below,
    // because that code does not run when this future is dropped. See
    // `CompactionClaim`.
    let mut claim_guard =
        CompactionClaim::new(store.clone(), claim.claim_id.clone(), claim.previous_status);
    if claim.previous_status != ProductSessionStatus::Idle {
        // Restore what the session was: this request never became a turn, so it
        // must not leave the session idle with someone else's work in flight.
        claim_guard.release().await;
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionActive.as_str(),
            "only an idle product session can be compacted",
        ));
    }

    let outcome = compact_claimed_session(&state, &store, &claim).await;
    // Every path this request can observe releases the claim, including failures
    // and cancellations. The one path it cannot observe — its own future being
    // dropped — is the guard's.
    claim_guard.release().await;
    outcome.map(Json)
}

/// The turn claim a compaction holds, released when it leaves scope.
///
/// Awaiting the release on every observable path is not enough: an aborted
/// request or a disconnected client drops the handler future between the claim
/// and the release, and nothing else recovers it. The claim row would outlive
/// the request with `status = 'running'`, every later turn and compaction would
/// answer 409 `product_session_active`, and the next API start would convert the
/// session to `needs_attention`, which `claim_session_turn` refuses as well — a
/// session that only an operator can free. `Drop` is the one hook that runs on
/// that path too, so the claim is owned by a value whose `Drop` releases it.
struct CompactionClaim {
    store: Arc<dyn ProductStore>,
    claim_id: ProductTurnClaimId,
    /// The status to restore if the guard has to release on its own.
    release_status: ProductSessionStatus,
    released: bool,
}

impl CompactionClaim {
    fn new(
        store: Arc<dyn ProductStore>,
        claim_id: ProductTurnClaimId,
        previous_status: ProductSessionStatus,
    ) -> Self {
        Self {
            store,
            claim_id,
            release_status: restorable_status(previous_status),
            released: false,
        }
    }

    /// Release the claim and restore the status this request inherited.
    async fn release(&mut self) {
        self.released = true;
        release_compaction_claim(&self.store, &self.claim_id, self.release_status).await;
    }
}

impl Drop for CompactionClaim {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // `Drop` cannot await, so the release is handed to a detached task
        // holding the owned store handle. Detached on purpose: the request that
        // was supposed to wait for the release no longer exists.
        let store = self.store.clone();
        let claim_id = self.claim_id.clone();
        let status = self.release_status;
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    release_compaction_claim(&store, &claim_id, status).await;
                });
            }
            Err(_) => tracing::warn!(
                %claim_id,
                "could not release the compaction turn claim: no runtime is available to run the release"
            ),
        }
    }
}

/// The status to restore when a compaction releases the claim it took.
///
/// `Running` is not restorable: `finish_session_turn` refuses it, because a
/// finished turn must not stay running, and the refusal used to leave both the
/// claim row and `status = 'running'` behind — the orphan state that makes a
/// session unclaimable. A compaction that inherited `running` therefore restores
/// `idle`: the turn it was told about is not there, so it frees the session
/// instead of restoring a status the store rejects.
fn restorable_status(previous_status: ProductSessionStatus) -> ProductSessionStatus {
    match previous_status {
        ProductSessionStatus::Running => ProductSessionStatus::Idle,
        other => other,
    }
}

async fn release_compaction_claim(
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    status: ProductSessionStatus,
) {
    if let Err(error) = store.finish_session_turn(claim_id, status).await {
        tracing::warn!(
            %claim_id,
            "failed to release the compaction turn claim: {error}"
        );
    }
}

async fn compact_claimed_session(
    state: &ApiState,
    store: &Arc<dyn ProductStore>,
    claim: &ProductTurnClaim,
) -> Result<ProductSessionCompaction, ApiError> {
    let product_session_id = claim.context.session.id.clone();
    // A session that has not run a turn yet has no prompt state to compact, and
    // neither has a fork child before its first turn. That is an answer.
    let Some(previous) = claim.previous_binding.as_ref() else {
        return Ok(nothing_to_compact(product_session_id));
    };

    let (workspace, config, run_model_snapshot) = crate::product_session_execution_config(
        state,
        &claim.context.workspace,
        store,
        &claim.model_config,
        // No request body carries a workspace hint on this route.
        None,
        true,
    )
    .await?;
    let state_store = crate::state_store_for_parts(&workspace, &config);
    // The state is loaded exactly the way a turn loads it. `project_product_follow_up_state`
    // is deliberately not applied: that projection resets the step, plan, and
    // ledger the *next* turn owns, and compaction must not decide those.
    let mut task_state = crate::load_product_resume_state(&state_store, previous).await?;
    // The same guard a turn applies: the snapshot is about to be projected
    // through the current engine's wire protocol, so a Provider identity that
    // changed underneath the session must fail closed rather than silently
    // re-project the history under different rules.
    crate::validate_product_resume_model(&task_state, &run_model_snapshot)?;
    let engine = crate::assemble_compaction_engine(
        state,
        &workspace,
        config,
        &run_model_snapshot,
        claim.model_config.max_steps,
    )
    .await
    .map_err(|error| ApiError::agent_engine_assembly(&error))?;

    let outcome = engine
        .compact_resume_state(&mut task_state, CancellationToken::new())
        .await
        .map_err(|error| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateCorrupt,
                format!(
                    "the product session's canonical history cannot be compacted safely: {error}"
                ),
            ))
        })?;

    if outcome.triggered {
        // Only a real rewrite is persisted. A call that produced no new summary
        // leaves `task_state.json` byte-identical rather than re-stamping it.
        state_store
            .write_task_state(&task_state)
            .await
            .map_err(|error| {
                tracing::warn!(
                    product_session_id = %product_session_id,
                    run_id = %previous.latest_run_id,
                    "failed to persist a manual compaction: {error}"
                );
                ApiError::from(ProductStoreError::new(
                    ProductErrorCode::ProductStorageFailure,
                    "the compacted product session state could not be persisted",
                ))
            })?;
    }

    Ok(compaction_facts(
        product_session_id,
        Some(previous.latest_run_id),
        task_state.checkpoint.as_ref(),
        &outcome,
        chrono::Utc::now(),
    ))
}

fn nothing_to_compact(product_session_id: ProductSessionId) -> ProductSessionCompaction {
    ProductSessionCompaction {
        product_session_id,
        runtime_run_id: None,
        triggered: false,
        mode: compaction_mode_name(&PromptCompactionMode::None).to_string(),
        degraded: false,
        consecutive_failures: 0,
        circuit_open: false,
        next_attempt_after: None,
        model: None,
        prompt_version: None,
        source_message_count: 0,
        summary: None,
        summary_truncated: false,
        token_estimate: 0,
        failure_code: None,
    }
}

/// Project the runtime's compaction outcome into the bounded HTTP response.
///
/// `circuit_open` comes from the outcome rather than from the persisted
/// `PromptCompactionState`, because the two answer different questions: the
/// persisted field is the reported state (`false` while compaction is switched
/// off), the outcome field is whether the session's failure count has reached
/// the threshold. They agree for every state this route writes.
///
/// A tripped breaker is not by itself a refusal: the automatic path is refused
/// only while the window the last failure armed is still open. So
/// `next_attempt_after` is reported only when it *gates* — the breaker is
/// tripped **and** the deadline is still ahead of `now` — which is what makes
/// `circuit_open: true` with a present window mean "refused until then" and
/// `circuit_open: true` with no window mean "tripped, and the next automatic
/// attempt may probe". A window armed below the threshold, or one that has
/// already elapsed, refuses nothing and is therefore not reported; this route
/// only reports the automatic path's state, it never changes it.
fn compaction_facts(
    product_session_id: ProductSessionId,
    runtime_run_id: Option<RunId>,
    checkpoint: Option<&PromptCheckpoint>,
    outcome: &ManualCompactionOutcome,
    now: chrono::DateTime<chrono::Utc>,
) -> ProductSessionCompaction {
    let (summary, summary_truncated) = outcome
        .summary
        .as_deref()
        .map(bounded_summary)
        .map(|(summary, truncated)| (Some(summary), truncated))
        .unwrap_or((None, false));
    ProductSessionCompaction {
        product_session_id,
        runtime_run_id,
        triggered: outcome.triggered,
        mode: compaction_mode_name(&outcome.state.mode).to_string(),
        degraded: outcome.state.degraded,
        consecutive_failures: outcome.state.consecutive_failures,
        circuit_open: outcome.breaker_open,
        next_attempt_after: gating_window(outcome, now),
        model: outcome.state.model.clone(),
        prompt_version: outcome.state.prompt_version.clone(),
        source_message_count: u64::try_from(outcome.state.source_message_count).unwrap_or(u64::MAX),
        summary,
        summary_truncated,
        token_estimate: checkpoint
            .map(|checkpoint| u64::try_from(checkpoint.token_estimate).unwrap_or(u64::MAX))
            .unwrap_or_default(),
        failure_code: outcome.failure_code.map(str::to_string),
    }
}

/// The deadline this answer may report, if it currently refuses the automatic
/// path.
///
/// The value is the session's own string, unchanged: the answer must be the same
/// one a later request reads back from the snapshot, so nothing here normalizes
/// or rewrites it. An unparsable deadline is not one this runtime wrote, and it
/// cannot gate anything it cannot compare, so it is not reported. The runtime's
/// own gate reads it the same way: as expired.
fn gating_window(
    outcome: &ManualCompactionOutcome,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    if !outcome.breaker_open {
        return None;
    }
    let deadline = outcome.state.next_attempt_after.as_deref()?;
    let parsed = chrono::DateTime::parse_from_rfc3339(deadline).ok()?;
    (parsed.with_timezone(&chrono::Utc) > now).then(|| deadline.to_string())
}

fn bounded_summary(summary: &str) -> (String, bool) {
    let mut excerpt: String = summary
        .chars()
        .take(PRODUCT_COMPACTION_SUMMARY_EXCERPT_CHARS)
        .collect();
    let truncated = excerpt.chars().count() < summary.chars().count();
    if truncated {
        excerpt.push('…');
    }
    (excerpt, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::store::open_product_store;
    use crate::product::{
        CreateProductSessionRequest, CreateProductWorkspaceRequest, ProductWorkspaceKind,
    };
    use rove_runtime::types::PromptCompactionState;

    fn outcome(summary: Option<&str>, breaker_open: bool) -> ManualCompactionOutcome {
        ManualCompactionOutcome {
            triggered: summary.is_some(),
            breaker_open,
            state: PromptCompactionState {
                mode: PromptCompactionMode::ModelGenerated,
                consecutive_failures: 2,
                ..PromptCompactionState::default()
            },
            summary: summary.map(str::to_string),
            failure_code: None,
        }
    }

    /// A fixed instant, so the projection's window test never depends on the
    /// wall clock. The route itself passes `Utc::now()`.
    fn fixed_now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2027-01-15T08:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn a_summary_excerpt_is_bounded_and_flagged() {
        let long = "x".repeat(PRODUCT_COMPACTION_SUMMARY_EXCERPT_CHARS + 50);
        let facts = compaction_facts(
            ProductSessionId::new(),
            None,
            None,
            &outcome(Some(&long), false),
            fixed_now(),
        );
        assert!(facts.summary_truncated);
        let summary = facts.summary.unwrap();
        assert_eq!(
            summary.chars().count(),
            PRODUCT_COMPACTION_SUMMARY_EXCERPT_CHARS + 1
        );

        let short = compaction_facts(
            ProductSessionId::new(),
            None,
            None,
            &outcome(Some("Compact summary"), false),
            fixed_now(),
        );
        assert!(!short.summary_truncated);
        assert_eq!(short.summary.as_deref(), Some("Compact summary"));
    }

    #[test]
    fn nothing_to_compact_reports_a_typed_empty_answer() {
        let answer = nothing_to_compact(ProductSessionId::new());
        assert!(!answer.triggered);
        assert_eq!(answer.mode, "none");
        assert!(answer.runtime_run_id.is_none());
        assert!(answer.summary.is_none());
        assert_eq!(answer.token_estimate, 0);
        assert!(
            answer.next_attempt_after.is_none(),
            "a session with no run has no breaker window to report"
        );
    }

    /// A window is reported only while it refuses the automatic path.
    ///
    /// That is the whole point of reporting it: `circuit_open: true` plus a
    /// deadline means "refused until then", so the deadline must never appear
    /// beside `circuit_open: false`, and an elapsed one must not appear beside a
    /// tripped breaker that the next automatic attempt may already probe.
    #[test]
    fn the_reported_window_is_only_the_one_that_gates() {
        let deadline = "2027-01-15T08:02:00+00:00".to_string();
        let gating = |breaker_open: bool, next: Option<&str>| {
            let mut state = outcome(Some("Compact summary"), breaker_open).state;
            state.next_attempt_after = next.map(str::to_string);
            compaction_facts(
                ProductSessionId::new(),
                None,
                None,
                &ManualCompactionOutcome {
                    triggered: true,
                    breaker_open,
                    state,
                    summary: Some("Compact summary".to_string()),
                    failure_code: None,
                },
                fixed_now(),
            )
        };

        let tripped = gating(true, Some(&deadline));
        assert!(tripped.circuit_open);
        assert_eq!(
            tripped.next_attempt_after.as_deref(),
            Some(deadline.as_str()),
            "a tripped breaker with a live window reports the refusal it causes"
        );

        let elapsed = gating(true, Some("2027-01-15T07:59:00+00:00"));
        assert!(elapsed.circuit_open);
        assert_eq!(
            elapsed.next_attempt_after, None,
            "an elapsed window refuses nothing, so the next automatic attempt may probe"
        );

        let inert = gating(false, Some(&deadline));
        assert!(!inert.circuit_open);
        assert_eq!(
            inert.next_attempt_after, None,
            "a window armed below the threshold is not a refusal and must not ship \
             beside an open circuit"
        );

        let cleared = gating(false, None);
        assert_eq!(
            cleared.next_attempt_after, None,
            "a successful probe leaves nothing to wait for"
        );
    }

    /// An unparsable deadline cannot gate anything the runtime can compare, so
    /// it is not reported as a refusal. The runtime's own gate reads it the same
    /// way: as expired.
    #[test]
    fn an_unparsable_window_is_not_reported_as_a_refusal() {
        let facts = {
            let mut state = outcome(Some("Compact summary"), true).state;
            state.next_attempt_after = Some("not a timestamp".to_string());
            compaction_facts(
                ProductSessionId::new(),
                None,
                None,
                &ManualCompactionOutcome {
                    triggered: true,
                    breaker_open: true,
                    state,
                    summary: Some("Compact summary".to_string()),
                    failure_code: None,
                },
                fixed_now(),
            )
        };
        assert!(facts.circuit_open);
        assert_eq!(facts.next_attempt_after, None);
    }

    #[test]
    fn the_breaker_fact_comes_from_the_outcome_not_the_persisted_state() {
        let mut state = outcome(Some("Compact summary"), true).state;
        state.circuit_open = false;
        let facts = compaction_facts(
            ProductSessionId::new(),
            None,
            None,
            &ManualCompactionOutcome {
                triggered: true,
                breaker_open: true,
                state,
                summary: Some("Compact summary".to_string()),
                failure_code: None,
            },
            fixed_now(),
        );
        assert!(facts.circuit_open);
        assert_eq!(facts.consecutive_failures, 2);
    }

    /// A claim the store refuses to release leaves the session unusable, so the
    /// guard must never ask for `Running`: `finish_session_turn` rejects it, and
    /// the rejected release would keep both the claim row and `running`.
    #[test]
    fn a_running_previous_status_is_restored_as_idle() {
        assert_eq!(
            restorable_status(ProductSessionStatus::Running),
            ProductSessionStatus::Idle
        );
        assert_eq!(
            restorable_status(ProductSessionStatus::Idle),
            ProductSessionStatus::Idle
        );
        assert_eq!(
            restorable_status(ProductSessionStatus::NeedsAttention),
            ProductSessionStatus::NeedsAttention
        );
    }

    /// The path no handler can await: the request future is dropped, so only
    /// `Drop` runs. Without the guard the claim row survives the request and the
    /// session answers 409 forever.
    #[tokio::test]
    async fn dropping_the_claim_guard_releases_the_session_turn_claim() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = open_product_store(temp.path().join("product.sqlite"), 5_000).unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let workspace = store
            .create_workspace(CreateProductWorkspaceRequest {
                root,
                kind: ProductWorkspaceKind::Folder,
                display_name: Some("Guard workspace".to_string()),
                pinned: false,
            })
            .await
            .unwrap();
        let session = store
            .create_session(CreateProductSessionRequest {
                workspace_id: workspace.id.clone(),
                title: Some("Guard session".to_string()),
            })
            .await
            .unwrap();

        let claim = store.claim_session_turn(&session.id).await.unwrap();
        assert_eq!(claim.previous_status, ProductSessionStatus::Idle);
        let guard =
            CompactionClaim::new(store.clone(), claim.claim_id.clone(), claim.previous_status);
        let blocked = store
            .claim_session_turn(&session.id)
            .await
            .expect_err("the fixture must hold the claim the guard releases");
        assert_eq!(blocked.code, ProductErrorCode::ProductSessionActive);

        drop(guard);

        // The release is a detached task, so the claim disappears asynchronously.
        let mut last = None;
        for _ in 0..200 {
            match store.claim_session_turn(&session.id).await {
                Ok(released) => {
                    assert_eq!(released.previous_status, ProductSessionStatus::Idle);
                    return;
                }
                Err(error) => last = Some(error),
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the dropped guard never released the claim: {last:?}");
    }
}
