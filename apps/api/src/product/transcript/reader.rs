use std::io::ErrorKind;
use std::sync::Arc;

use async_trait::async_trait;
use rove_runtime::events::StreamEvent;
use rove_runtime::state::index::RunEventSnapshot;
use rove_runtime::state::report::RunReport;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::RunStatus;
use tokio::io::AsyncReadExt;

use crate::product::{
    ProductErrorCode, ProductRuntimeStateResolver, ProductSessionContext, ProductSessionId,
    ProductSessionRunBinding, ProductStore, ProductStoreError, ProductTranscriptFallback,
    ProductTranscriptFallbackSource, ProductTranscriptPartialReason,
    ProductTranscriptPartialReasonCode, ProductTranscriptReader, ProductTranscriptResponse,
    ProductTranscriptRunSegment, ProductTranscriptStatus, ProductWorkspace,
};
use crate::types::JobStreamEvent;

use super::validation::{
    binding_prefix_len, is_live_status, is_returnable_runtime_error, latest_binding_matches,
    parse_run_status, push_reason, reason_for_binding, report_identity_matches,
    run_identity_matches, runtime_chain_prefix_len, runtime_read_reason,
    terminal_consistency_issue, truncate_utf8, validated_report_status,
};

const CATALOG_SNAPSHOT_ATTEMPTS: usize = 3;
const MAX_TRANSCRIPT_RUNS: usize = 256;
const MAX_EVENTS_PER_RUN: usize = 2_000;
const MAX_TOTAL_EVENTS: usize = 10_000;
const MAX_EVENT_JSON_BYTES: usize = 1_048_576;
const MAX_TOTAL_EVENT_JSON_BYTES: usize = 16 * 1_048_576;
const MAX_REPORT_BYTES: usize = 256 * 1_024;
const MAX_TOTAL_REPORT_BYTES: usize = 2 * 1_048_576;
const MAX_FALLBACK_SUMMARY_BYTES: usize = 8 * 1_024;

/// Read-only projection of a product session over canonical runtime events.
///
/// The reader keeps only product-to-runtime mappings in `ProductStore`. Event
/// facts are always read from the selected workspace's runtime `StateStore`.
#[derive(Clone)]
pub(crate) struct CanonicalProductTranscriptReader {
    store: Arc<dyn ProductStore>,
    runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver>,
}

impl CanonicalProductTranscriptReader {
    pub(crate) fn new(
        store: Arc<dyn ProductStore>,
        runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver>,
    ) -> Self {
        Self {
            store,
            runtime_state_resolver,
        }
    }

    async fn catalog_snapshot(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<CatalogSnapshot, ProductStoreError> {
        // The store contract exposes context and bindings separately. Retry a
        // bounded number of times if a turn commits between those two reads.
        let mut last = None;
        for _ in 0..CATALOG_SNAPSHOT_ATTEMPTS {
            let context = self.store.get_session_context(session_id).await?;
            let bindings = self.store.list_run_bindings(session_id).await?;
            let latest_consistent =
                latest_binding_matches(context.session.runtime_binding.as_ref(), bindings.last());
            let snapshot = CatalogSnapshot {
                context,
                bindings,
                latest_consistent,
            };
            if latest_consistent {
                return Ok(snapshot);
            }
            last = Some(snapshot);
        }

        last.ok_or_else(ProductStoreError::unavailable)
    }

    async fn project_run(
        &self,
        workspace: &ProductWorkspace,
        state_store: &StateStore,
        binding: &ProductSessionRunBinding,
        startup_race_possible: bool,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> RunProjection {
        if budget.events_remaining == 0 {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    binding,
                    None,
                    None,
                ),
            );
            return RunProjection::stop(None);
        }

        let limit = MAX_EVENTS_PER_RUN.min(budget.events_remaining);
        let snapshot = match state_store
            .index
            .run_event_snapshot_async(binding.runtime_run_id, 0, limit)
            .await
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::RuntimeRunMissing,
                        binding,
                        None,
                        None,
                    ),
                );
                if !startup_race_possible {
                    self.classify_trace_availability(state_store, binding, true, reasons)
                        .await;
                }
                let fallback = self
                    .load_report_fallback(workspace, state_store, binding, None, budget, reasons)
                    .await;
                return RunProjection::continue_with(
                    fallback.map(|fallback| fallback_segment(binding, fallback, 0)),
                );
            }
            Err(error) => {
                push_reason(
                    reasons,
                    reason_for_binding(runtime_read_reason(error.kind()), binding, None, None),
                );
                let fallback = self
                    .load_report_fallback(workspace, state_store, binding, None, budget, reasons)
                    .await;
                return RunProjection::continue_with(
                    fallback.map(|fallback| fallback_segment(binding, fallback, 0)),
                );
            }
        };

        self.project_snapshot(workspace, state_store, binding, snapshot, budget, reasons)
            .await
    }

    async fn project_snapshot(
        &self,
        workspace: &ProductWorkspace,
        state_store: &StateStore,
        binding: &ProductSessionRunBinding,
        snapshot: RunEventSnapshot,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> RunProjection {
        let RunEventSnapshot {
            run,
            high_water_seq,
            events: records,
            has_more,
        } = snapshot;

        if !run_identity_matches(&run, binding) {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    binding,
                    None,
                    None,
                ),
            );
            let fallback = self
                .load_report_fallback(workspace, state_store, binding, None, budget, reasons)
                .await;
            return RunProjection::continue_with(
                fallback.map(|fallback| fallback_segment(binding, fallback, 0)),
            );
        }

        let Some(run_status) = parse_run_status(&run.status) else {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::CorruptArtifact,
                    binding,
                    None,
                    None,
                ),
            );
            let fallback = self
                .load_report_fallback(workspace, state_store, binding, None, budget, reasons)
                .await;
            return RunProjection::continue_with(
                fallback.map(|fallback| fallback_segment(binding, fallback, high_water_seq)),
            );
        };

        let record_count = records.len();
        let mut projected = Vec::with_capacity(record_count);
        let mut expected_seq = 1_u64;
        let mut terminal = None;
        let mut canonical_incomplete = false;
        let mut response_limited = false;

        for record in records {
            if record.run_id != binding.runtime_run_id {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                break;
            }
            if record.seq != expected_seq {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::MissingEventRange,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                break;
            }
            if record.event_json.len() > MAX_EVENT_JSON_BYTES
                || record.event_json.len() > budget.event_json_bytes_remaining
            {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::ResponseLimitReached,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                response_limited = true;
                break;
            }

            let event = match serde_json::from_str::<StreamEvent>(&record.event_json) {
                Ok(event) => event,
                Err(_) => {
                    push_reason(
                        reasons,
                        reason_for_binding(
                            ProductTranscriptPartialReasonCode::CorruptEvent,
                            binding,
                            Some(expected_seq),
                            Some(record.seq),
                        ),
                    );
                    canonical_incomplete = true;
                    break;
                }
            };
            if record.event_name != event.event_name() {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::CorruptEvent,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                break;
            }

            if expected_seq == 1 {
                match &event {
                    StreamEvent::RunStarted { run_id, job_id, .. }
                        if *run_id == binding.runtime_run_id
                            && *job_id == binding.runtime_job_id => {}
                    StreamEvent::RunStarted { .. } => {
                        push_reason(
                            reasons,
                            reason_for_binding(
                                ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                                binding,
                                Some(expected_seq),
                                Some(record.seq),
                            ),
                        );
                        canonical_incomplete = true;
                        break;
                    }
                    _ => {
                        push_reason(
                            reasons,
                            reason_for_binding(
                                ProductTranscriptPartialReasonCode::CorruptEvent,
                                binding,
                                Some(expected_seq),
                                Some(record.seq),
                            ),
                        );
                        canonical_incomplete = true;
                        break;
                    }
                }
            } else if matches!(&event, StreamEvent::RunStarted { .. }) {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::CorruptEvent,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                break;
            }

            if terminal.is_some() {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::CorruptEvent,
                        binding,
                        Some(expected_seq),
                        Some(record.seq),
                    ),
                );
                canonical_incomplete = true;
                break;
            }
            if let StreamEvent::RunCompleted { reason, .. } = &event {
                terminal = Some((record.seq, terminal_status_for_reason(reason)));
            }

            budget.events_remaining -= 1;
            budget.event_json_bytes_remaining -= record.event_json.len();
            projected.push(JobStreamEvent {
                seq: record.seq,
                event,
            });
            expected_seq += 1;
        }

        let observed_through_seq = projected.last().map_or(0, |event| event.seq);
        let consumed_all_records = projected.len() == record_count;
        if consumed_all_records && has_more {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    binding,
                    Some(observed_through_seq.saturating_add(1)),
                    Some(high_water_seq),
                ),
            );
            canonical_incomplete = true;
            response_limited = true;
        } else if consumed_all_records && observed_through_seq < high_water_seq {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::MissingEventRange,
                    binding,
                    Some(observed_through_seq.saturating_add(1)),
                    Some(high_water_seq),
                ),
            );
            canonical_incomplete = true;
        }

        if projected.is_empty() && !response_limited {
            canonical_incomplete = true;
            self.classify_trace_availability(
                state_store,
                binding,
                record_count == 0 && high_water_seq == 0,
                reasons,
            )
            .await;
        }

        if !canonical_incomplete
            && let Some(issue) =
                terminal_consistency_issue(&run_status, terminal.as_ref(), high_water_seq)
        {
            push_reason(
                reasons,
                reason_for_binding(issue.code, binding, issue.expected_seq, issue.observed_seq),
            );
            canonical_incomplete = true;
        }

        let fallback = if canonical_incomplete && !response_limited {
            self.load_report_fallback(
                workspace,
                state_store,
                binding,
                Some(&run_status),
                budget,
                reasons,
            )
            .await
        } else {
            None
        };
        let segment = ProductTranscriptRunSegment {
            binding: binding.clone(),
            run_status,
            observed_through_seq,
            last_event_seq: high_water_seq,
            events: projected,
            fallback: fallback.map(|fallback| fallback.fallback),
        };

        if response_limited {
            RunProjection::stop(Some(segment))
        } else {
            RunProjection::continue_with(Some(segment))
        }
    }

    async fn classify_trace_availability(
        &self,
        state_store: &StateStore,
        binding: &ProductSessionRunBinding,
        missing_if_present: bool,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) {
        let trace_path = state_store
            .run_store
            .run_dir(&binding.runtime_run_id)
            .join("trace.jsonl");
        match tokio::fs::metadata(trace_path).await {
            Ok(_) if missing_if_present => push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::MissingEventRange,
                    binding,
                    Some(1),
                    Some(0),
                ),
            ),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::CleanedHistory,
                    binding,
                    None,
                    None,
                ),
            ),
            Err(error) => push_reason(
                reasons,
                reason_for_binding(runtime_read_reason(error.kind()), binding, None, None),
            ),
        }
    }

    async fn load_report_fallback(
        &self,
        workspace: &ProductWorkspace,
        state_store: &StateStore,
        binding: &ProductSessionRunBinding,
        expected_run_status: Option<&RunStatus>,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> Option<ValidatedFallback> {
        let read_limit = MAX_REPORT_BYTES.min(budget.report_bytes_remaining);
        if read_limit == 0 {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    binding,
                    None,
                    None,
                ),
            );
            return None;
        }

        let report_path = state_store
            .run_store
            .run_dir(&binding.runtime_run_id)
            .join("report.json");
        let file = match tokio::fs::File::open(report_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return None,
            Err(error) => {
                push_reason(
                    reasons,
                    reason_for_binding(runtime_read_reason(error.kind()), binding, None, None),
                );
                return None;
            }
        };
        let mut bytes = Vec::with_capacity(read_limit.min(16 * 1_024));
        let mut bounded = file.take(read_limit.saturating_add(1) as u64);
        if let Err(error) = bounded.read_to_end(&mut bytes).await {
            budget.report_bytes_remaining = budget
                .report_bytes_remaining
                .saturating_sub(bytes.len().min(read_limit));
            push_reason(
                reasons,
                reason_for_binding(runtime_read_reason(error.kind()), binding, None, None),
            );
            return None;
        }
        budget.report_bytes_remaining = budget
            .report_bytes_remaining
            .saturating_sub(bytes.len().min(read_limit));
        if bytes.len() > read_limit {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    binding,
                    None,
                    None,
                ),
            );
            return None;
        }

        let report = match serde_json::from_slice::<RunReport>(&bytes) {
            Ok(report) => report,
            Err(_) => {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::CorruptArtifact,
                        binding,
                        None,
                        None,
                    ),
                );
                return None;
            }
        };
        if !report_identity_matches(&report, workspace, binding) {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    binding,
                    None,
                    None,
                ),
            );
            return None;
        }
        let Some(run_status) = validated_report_status(&report) else {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::CorruptArtifact,
                    binding,
                    None,
                    None,
                ),
            );
            return None;
        };
        if expected_run_status
            .is_some_and(|expected| !is_live_status(expected) && expected != &run_status)
        {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::CorruptArtifact,
                    binding,
                    None,
                    None,
                ),
            );
            return None;
        }

        let summary = report
            .output
            .as_deref()
            .map(|output| truncate_utf8(output, MAX_FALLBACK_SUMMARY_BYTES));
        if report
            .output
            .as_ref()
            .is_some_and(|output| output.len() > MAX_FALLBACK_SUMMARY_BYTES)
        {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    binding,
                    None,
                    None,
                ),
            );
        }

        Some(ValidatedFallback {
            run_status,
            fallback: ProductTranscriptFallback {
                source: ProductTranscriptFallbackSource::Report,
                status: report.status,
                summary,
            },
        })
    }
}

#[async_trait]
impl ProductTranscriptReader for CanonicalProductTranscriptReader {
    async fn read_transcript(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductTranscriptResponse, ProductStoreError> {
        let catalog = self.catalog_snapshot(session_id).await?;
        let workspace_id = catalog.context.session.workspace_id.clone();
        let mut reasons = Vec::new();
        let mut segments = Vec::new();

        if &catalog.context.session.id != session_id
            || catalog.context.workspace.id != catalog.context.session.workspace_id
        {
            push_reason(
                &mut reasons,
                ProductTranscriptPartialReason {
                    code: ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    run_ordinal: None,
                    run_id: None,
                    expected_seq: None,
                    observed_seq: None,
                },
            );
            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
            ));
        }

        let validation_count = catalog
            .bindings
            .len()
            .min(MAX_TRANSCRIPT_RUNS.saturating_add(1));
        let binding_window = &catalog.bindings[..validation_count];
        let valid_prefix = binding_prefix_len(session_id, binding_window, &mut reasons);
        if !catalog.latest_consistent {
            let summary = catalog.context.session.runtime_binding.as_ref();
            push_reason(
                &mut reasons,
                ProductTranscriptPartialReason {
                    code: ProductTranscriptPartialReasonCode::MissingRunMapping,
                    run_ordinal: summary.map(|binding| binding.ordinal),
                    run_id: summary.map(|binding| binding.latest_run_id),
                    expected_seq: None,
                    observed_seq: None,
                },
            );
        }

        if catalog.bindings.is_empty() {
            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
            ));
        }

        let identity_prefix =
            runtime_chain_prefix_len(&binding_window[..valid_prefix], &mut reasons);
        let process_count = identity_prefix.min(MAX_TRANSCRIPT_RUNS);
        if identity_prefix > MAX_TRANSCRIPT_RUNS {
            let omitted = &catalog.bindings[MAX_TRANSCRIPT_RUNS];
            push_reason(
                &mut reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    omitted,
                    None,
                    None,
                ),
            );
        }
        if process_count == 0 {
            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
            ));
        }

        let state_store = match self
            .runtime_state_resolver
            .state_store_for(&catalog.context.workspace)
        {
            Ok(state_store) => state_store,
            Err(error) if is_returnable_runtime_error(error.code) => {
                let code = match error.code {
                    ProductErrorCode::ProductSessionRuntimeStateCorrupt => {
                        ProductTranscriptPartialReasonCode::CorruptArtifact
                    }
                    _ => ProductTranscriptPartialReasonCode::RuntimeStateUnavailable,
                };
                push_reason(
                    &mut reasons,
                    ProductTranscriptPartialReason {
                        code,
                        run_ordinal: catalog.bindings.first().map(|binding| binding.ordinal),
                        run_id: catalog
                            .bindings
                            .first()
                            .map(|binding| binding.runtime_run_id),
                        expected_seq: None,
                        observed_seq: None,
                    },
                );
                return Ok(transcript_response(
                    session_id,
                    workspace_id,
                    reasons,
                    segments,
                ));
            }
            Err(error) => return Err(error),
        };

        let mut budget = ProjectionBudget::default();
        for binding in catalog.bindings.iter().take(process_count) {
            let startup_race_possible = matches!(
                catalog.context.session.status,
                crate::product::ProductSessionStatus::Running
            ) && catalog
                .context
                .session
                .runtime_binding
                .as_ref()
                .is_some_and(|latest| latest.ordinal == binding.ordinal);
            let projection = self
                .project_run(
                    &catalog.context.workspace,
                    &state_store,
                    binding,
                    startup_race_possible,
                    &mut budget,
                    &mut reasons,
                )
                .await;
            if let Some(segment) = projection.segment {
                segments.push(segment);
            }
            if projection.stop {
                break;
            }
        }

        Ok(transcript_response(
            session_id,
            workspace_id,
            reasons,
            segments,
        ))
    }
}

struct CatalogSnapshot {
    context: ProductSessionContext,
    bindings: Vec<ProductSessionRunBinding>,
    latest_consistent: bool,
}

struct RunProjection {
    segment: Option<ProductTranscriptRunSegment>,
    stop: bool,
}

impl RunProjection {
    fn continue_with(segment: Option<ProductTranscriptRunSegment>) -> Self {
        Self {
            segment,
            stop: false,
        }
    }

    fn stop(segment: Option<ProductTranscriptRunSegment>) -> Self {
        Self {
            segment,
            stop: true,
        }
    }
}

struct ValidatedFallback {
    run_status: RunStatus,
    fallback: ProductTranscriptFallback,
}

struct ProjectionBudget {
    events_remaining: usize,
    event_json_bytes_remaining: usize,
    report_bytes_remaining: usize,
}

impl Default for ProjectionBudget {
    fn default() -> Self {
        Self {
            events_remaining: MAX_TOTAL_EVENTS,
            event_json_bytes_remaining: MAX_TOTAL_EVENT_JSON_BYTES,
            report_bytes_remaining: MAX_TOTAL_REPORT_BYTES,
        }
    }
}

fn fallback_segment(
    binding: &ProductSessionRunBinding,
    fallback: ValidatedFallback,
    last_event_seq: u64,
) -> ProductTranscriptRunSegment {
    ProductTranscriptRunSegment {
        binding: binding.clone(),
        run_status: fallback.run_status,
        observed_through_seq: 0,
        last_event_seq,
        events: Vec::new(),
        fallback: Some(fallback.fallback),
    }
}

fn transcript_response(
    session_id: &ProductSessionId,
    workspace_id: crate::product::ProductWorkspaceId,
    partial_reasons: Vec<ProductTranscriptPartialReason>,
    segments: Vec<ProductTranscriptRunSegment>,
) -> ProductTranscriptResponse {
    ProductTranscriptResponse {
        product_session_id: session_id.clone(),
        workspace_id,
        status: if partial_reasons.is_empty() {
            ProductTranscriptStatus::Complete
        } else {
            ProductTranscriptStatus::Partial
        },
        partial_reasons,
        segments,
    }
}
