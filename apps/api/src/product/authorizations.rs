//! Durable per-session authorization history (plan P4, decision side).
//!
//! The request side is durable in the runtime StateIndex
//! (`pending_approvals`); schema v5 adds `decided_via` so the decision side is
//! durable too. This endpoint projects both, plus the terminal tool event the
//! decision led to when the run's event log holds one. Fields that were never
//! recorded (pre-v5 `decided_via`, decisions that never reached execution)
//! stay `null` instead of being reconstructed by guessing.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::ProductSessionId;

/// Default page size; matches the pending panel's "recent records" framing.
const DEFAULT_AUTHORIZATIONS_LIMIT: usize = 50;
/// Hard upper bound; the UI asks for one bounded page, not a full export.
pub(crate) const MAX_AUTHORIZATIONS_LIMIT: usize = 200;
/// Terminal tool events scanned for outcome attribution across the session's
/// runs. Large enough to cover every approval row the page can return.
const MAX_OUTCOME_SCAN: usize = 2_000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuthorizationsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The terminal tool event one decided call reached, when the run's event
/// log records one. Payload is deliberately not projected here; the artifact
/// and transcript surfaces already carry it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductAuthorizationOutcome {
    /// `tool_call_completed` or `tool_call_failed`.
    pub event: String,
    pub seq: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductAuthorizationRecord {
    pub call_id: String,
    pub job_id: String,
    pub run_id: String,
    pub tool: String,
    #[schema(value_type = Object)]
    pub args: serde_json::Value,
    pub reason: String,
    /// `pending`, `approved`, `rejected`, `cancelled`, or `interrupted`.
    pub status: String,
    /// Which surface recorded the decision (`job_api`, `job_cancel`,
    /// `job_responder_lost`); `null` when no decision surface was recorded
    /// (still pending, interrupted at startup, or pre-schema-v5 rows).
    pub decided_via: Option<String>,
    pub requested_at: String,
    /// Last durable update; for decided rows this is the decision time.
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProductAuthorizationOutcome>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductAuthorizationsResponse {
    pub session_id: ProductSessionId,
    pub authorizations: Vec<ProductAuthorizationRecord>,
    /// True when the limit cut the page; there is no cursor because the
    /// panel only claims to show recent history.
    pub truncated: bool,
}

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/authorizations",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Server-owned product session id"),
        AuthorizationsQuery
    ),
    responses(
        (status = 200, description = "Bounded approval request + decision history across the session's runs", body = ProductAuthorizationsResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Runtime state read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_session_authorizations(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    Query(query): Query<AuthorizationsQuery>,
) -> Result<Json<ProductAuthorizationsResponse>, ApiError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_AUTHORIZATIONS_LIMIT)
        .clamp(1, MAX_AUTHORIZATIONS_LIMIT);
    let store = state.product_store()?;
    let context = store.get_session_context(&session_id).await?;
    let bindings = store.list_run_bindings(&session_id).await?;
    let run_ids: Vec<rove_runtime::types::RunId> = bindings
        .iter()
        .map(|binding| binding.runtime_run_id)
        .collect();
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;

    // Fetch one extra row to know whether the page was cut, without a count
    // query or a cursor.
    let mut entries = state_store
        .index
        .approvals_for_runs_async(run_ids.clone(), limit + 1)
        .await
        .map_err(ApiError::internal)?;
    let truncated = entries.len() > limit;
    entries.truncate(limit);

    let outcome_by_call_id = if entries.iter().any(|entry| entry.status != "pending") {
        let outcome_records = state_store
            .index
            .tool_outcome_records_for_runs_async(run_ids, MAX_OUTCOME_SCAN)
            .await
            .map_err(ApiError::internal)?;
        correlate_outcomes(outcome_records)
    } else {
        HashMap::new()
    };

    let authorizations = entries
        .into_iter()
        .map(|entry| {
            let args = serde_json::from_str(&entry.args_json)
                .unwrap_or(serde_json::Value::String(entry.args_json.clone()));
            let outcome = outcome_by_call_id.get(&entry.call_id).map(|(event, seq)| {
                ProductAuthorizationOutcome {
                    event: event.clone(),
                    seq: *seq,
                }
            });
            ProductAuthorizationRecord {
                call_id: entry.call_id,
                job_id: entry.job_id,
                run_id: entry.run_id,
                tool: entry.tool,
                args,
                reason: entry.reason,
                status: entry.status,
                decided_via: entry.decided_via,
                requested_at: entry.requested_at,
                updated_at: entry.updated_at,
                outcome,
            }
        })
        .collect();

    Ok(Json(ProductAuthorizationsResponse {
        session_id,
        authorizations,
        truncated,
    }))
}

/// Map `call_id` → terminal tool event. Event JSON that fails to parse is
/// skipped: a corrupted event must not hide the decision history itself.
fn correlate_outcomes(
    outcome_records: Vec<rove_runtime::state::index::EventIndexRecord>,
) -> HashMap<String, (String, u64)> {
    let mut by_call_id = HashMap::new();
    for record in outcome_records {
        let Ok(event) =
            serde_json::from_str::<rove_runtime::events::StreamEvent>(&record.event_json)
        else {
            continue;
        };
        let call_id = match &event {
            rove_runtime::events::StreamEvent::ToolCallCompleted { call_id, .. }
            | rove_runtime::events::StreamEvent::ToolCallFailed { call_id, .. } => {
                call_id.to_string()
            }
            _ => continue,
        };
        by_call_id.insert(call_id, (record.event_name, record.seq));
    }
    by_call_id
}
