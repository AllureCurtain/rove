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
    ProductErrorCode, ProductForkInheritedRun, ProductRuntimeStateResolver, ProductSessionContext,
    ProductSessionId, ProductSessionRunBinding, ProductStore, ProductStoreError,
    ProductTranscriptEventContract, ProductTranscriptFallback, ProductTranscriptFallbackSource,
    ProductTranscriptPartialReason, ProductTranscriptPartialReasonCode, ProductTranscriptQuery,
    ProductTranscriptReader, ProductTranscriptResponse, ProductTranscriptRunSegment,
    ProductTranscriptStatus, ProductWorkspace,
};
use crate::types::JobStreamEvent;

use super::validation::{
    binding_prefix_len, is_live_status, is_returnable_runtime_error, latest_binding_matches,
    parse_run_status, push_reason, reason_for_binding, report_fallback_allowed,
    report_identity_matches, run_identity_matches, runtime_chain_prefix_len, runtime_read_reason,
    terminal_consistency_issue, terminal_status_for_reason, truncate_utf8, validated_report_status,
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
        request: ProjectionRequest<'_>,
        startup_race_possible: bool,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> RunProjection {
        let ProjectionRequest {
            workspace,
            state_store,
            binding,
            ..
        } = request;
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

        self.project_snapshot(request, snapshot, budget, reasons)
            .await
    }

    async fn project_snapshot(
        &self,
        request: ProjectionRequest<'_>,
        snapshot: RunEventSnapshot,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> RunProjection {
        let ProjectionRequest {
            workspace,
            state_store,
            binding,
            event_contract,
        } = request;
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
        let mut terminal = None;
        let mut canonical_incomplete = false;
        let mut response_limited = false;
        // Durable rows this response read but did not deliver because the
        // requesting client cannot decode their kind. They are neither a gap nor
        // corruption, so they are accounted separately.
        let mut withheld = WithheldEvents::default();

        for (expected_seq, record) in (1_u64..).zip(records) {
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
            // A row the declared contract excludes is never delivered, so its
            // outcome is decided before the size gate below: it can neither
            // consume the response budget nor end the projection. The stored
            // `event_name` is this row's durable kind, and a kind this build can
            // name is a kind it can decide without reading the payload — which
            // matters because such a row may be larger than the response limit.
            // A kind this build cannot name is validated as a well-formed
            // envelope first (see the decode below), so a damaged row is never
            // passed off as merely newer.
            if event_contract.is_some_and(|contract| !contract.accepts(&record.event_name))
                && StreamEvent::is_known_event_name(&record.event_name)
            {
                withheld.record(record.seq);
                continue;
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
                    // A row written by a newer build names a kind this build does
                    // not define. That is not corruption: the same rule the
                    // product directory stream applies to an unrecognized `kind`
                    // applies here — withhold the row, keep its position, and
                    // never let it park the projection. A negotiated request is
                    // what enables it, so a legacy request keeps its previous
                    // contract exactly.
                    if event_contract.is_some()
                        && is_unknown_event_kind(&record.event_json, &record.event_name)
                    {
                        withheld.record(record.seq);
                        continue;
                    }
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

            // The lifecycle checks above read durable facts, so they run for
            // every row the reader decoded. A row outside the declared contract
            // never reaches this point — it was withheld before the size gate — so
            // this billing is only ever charged for a row the client receives.
            budget.events_remaining -= 1;
            budget.event_json_bytes_remaining -= record.event_json.len();
            projected.push(JobStreamEvent {
                seq: record.seq,
                event,
            });
        }

        let observed_through_seq = projected.last().map_or(0, |event| event.seq);
        // The loop consumed the whole snapshot when every record either reached
        // the client or was withheld from it. A record the loop *broke* on was
        // not consumed, so it must not count here — that is what keeps the
        // parameterless contract identical to the one that never withheld
        // anything.
        let consumed_all_records = projected.len() + withheld.count == record_count;
        // The highest sequence this response can account for, delivered or
        // withheld. Only what lies beyond it is a gap.
        let accounted_through_seq = observed_through_seq.max(withheld.accounted_through_seq());
        if let Some(first_seq) = withheld.first_seq {
            // Withheld rows are accounted, not swallowed: the response names the
            // durable sequence range it could not deliver, and every delivered
            // event keeps its own true `seq`, so a client can see exactly where
            // the withheld rows sit instead of a silent hole.
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::UnknownEventType,
                    binding,
                    Some(first_seq),
                    Some(withheld.last_seq),
                ),
            );
        }
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
        } else if consumed_all_records && accounted_through_seq < high_water_seq {
            // Whatever lies beyond the accounted sequence — delivered or withheld
            // — is a gap the response cannot explain, so it is reported. Asking
            // instead whether *anything* was withheld would let one withheld row
            // anywhere hide an unrelated residue at the end of the run (a durable
            // high-water mark advanced by a trace line that was never indexed).
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::MissingEventRange,
                    binding,
                    Some(accounted_through_seq.saturating_add(1)),
                    Some(high_water_seq),
                ),
            );
            canonical_incomplete = true;
        }

        // Unlike the tail check above, this guard asks the narrow question: was
        // the projection empty *because* every record was withheld? Only then is
        // the emptiness already accounted for, and only then is it wrong to
        // classify the run as having nothing to project. A run with no indexed
        // records at all keeps its original classification.
        let every_record_withheld = record_count > 0 && withheld.count == record_count;
        if projected.is_empty() && !response_limited && !every_record_withheld {
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

        let fallback = if report_fallback_allowed(
            canonical_incomplete,
            response_limited,
            terminal.is_some(),
        ) {
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
        let segment = run_segment(
            binding,
            run_status,
            observed_through_seq,
            high_water_seq,
            projected,
            fallback.map(|fallback| fallback.fallback),
        );

        if response_limited {
            RunProjection::stop(Some(segment))
        } else {
            RunProjection::continue_with(Some(segment))
        }
    }

    async fn project_candidate(
        &self,
        workspace: &ProductWorkspace,
        state_store: &StateStore,
        candidate: &TranscriptCandidate,
        event_contract: Option<ProductTranscriptEventContract>,
        budget: &mut ProjectionBudget,
        reasons: &mut Vec<ProductTranscriptPartialReason>,
    ) -> RunProjection {
        let mut projection = self
            .project_run(
                ProjectionRequest {
                    workspace,
                    state_store,
                    binding: &candidate.binding,
                    event_contract,
                },
                candidate.startup_race_possible,
                budget,
                reasons,
            )
            .await;
        let Some(source_product_session_id) = &candidate.source_product_session_id else {
            return projection;
        };
        if let Some(segment) = projection.segment.as_mut() {
            segment.inherited = true;
            segment.source_product_session_id = Some(source_product_session_id.clone());
            if let Some(expected) = candidate.inherited_through_event_seq
                && segment.last_event_seq != expected
            {
                push_reason(
                    reasons,
                    reason_for_binding(
                        ProductTranscriptPartialReasonCode::CorruptArtifact,
                        &candidate.binding,
                        Some(expected),
                        Some(segment.last_event_seq),
                    ),
                );
            }
        }
        projection
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
        query: ProductTranscriptQuery,
    ) -> Result<ProductTranscriptResponse, ProductStoreError> {
        let catalog = self.catalog_snapshot(session_id).await?;
        let workspace_id = catalog.context.session.workspace_id.clone();
        let mut reasons = Vec::new();
        let mut segments = Vec::new();
        // A cursor request always reports its page shape, including the empty
        // pages this projection can return before any run is read.
        let empty_page = query.is_page().then_some(TranscriptCursor::terminal());

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
                empty_page,
            ));
        }

        let inherited_runs = catalog
            .context
            .fork
            .as_ref()
            .map(|fork| fork.inherited_runs.as_slice())
            .unwrap_or_default();
        let inherited_count = u64::try_from(inherited_runs.len()).unwrap_or(u64::MAX);
        // A child owns local binding ordinals starting at one. Transcript
        // ordinals are presentation ordering over the immutable inherited
        // prefix plus local runs, so shift only the projected copies.
        let display_bindings: Vec<ProductSessionRunBinding> = catalog
            .bindings
            .iter()
            .cloned()
            .map(|mut binding| {
                binding.ordinal = binding.ordinal.saturating_add(inherited_count);
                binding
            })
            .collect();
        let validation_count = catalog
            .bindings
            .len()
            .min(MAX_TRANSCRIPT_RUNS.saturating_add(1));
        // Validate persisted local ordinals before applying the inherited
        // presentation offset. A fork child still owns bindings starting at
        // one, even though its visible transcript starts after the immutable
        // inherited prefix.
        let raw_binding_window = &catalog.bindings[..validation_count];
        let valid_prefix = binding_prefix_len(session_id, raw_binding_window, &mut reasons);
        if !catalog.latest_consistent {
            let summary = catalog.context.session.runtime_binding.as_ref();
            push_reason(
                &mut reasons,
                ProductTranscriptPartialReason {
                    code: ProductTranscriptPartialReasonCode::MissingRunMapping,
                    run_ordinal: summary
                        .map(|binding| binding.ordinal.saturating_add(inherited_count)),
                    run_id: summary.map(|binding| binding.latest_run_id),
                    expected_seq: None,
                    observed_seq: None,
                },
            );
        }

        if display_bindings.is_empty() && inherited_runs.is_empty() {
            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
                empty_page,
            ));
        }

        let identity_prefix =
            runtime_chain_prefix_len(&raw_binding_window[..valid_prefix], &mut reasons);
        let inherited_process_count = inherited_runs.len().min(MAX_TRANSCRIPT_RUNS);
        if inherited_runs.len() > inherited_process_count {
            let omitted = inherited_transcript_binding(
                &inherited_runs[inherited_process_count],
                None,
                &catalog
                    .context
                    .fork
                    .as_ref()
                    .expect("inherited runs require fork provenance")
                    .fork
                    .created_at,
            );
            push_reason(
                &mut reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::ResponseLimitReached,
                    &omitted,
                    None,
                    None,
                ),
            );
        }
        let local_segment_capacity = MAX_TRANSCRIPT_RUNS.saturating_sub(inherited_process_count);
        let process_count = identity_prefix.min(local_segment_capacity);
        if identity_prefix > process_count {
            let omitted = &display_bindings[process_count];
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
        if inherited_process_count == 0 && process_count == 0 {
            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
                empty_page,
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
                    empty_page,
                ));
            }
            Err(error) => return Err(error),
        };

        let mut budget = ProjectionBudget::default();
        let candidates = transcript_candidates(
            &catalog,
            inherited_runs,
            &display_bindings,
            inherited_process_count,
            process_count,
        );

        if !query.is_page() {
            for candidate in &candidates {
                let projection = self
                    .project_candidate(
                        &catalog.context.workspace,
                        &state_store,
                        candidate,
                        query.event_contract,
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

            return Ok(transcript_response(
                session_id,
                workspace_id,
                reasons,
                segments,
                None,
            ));
        }

        // A cursor page is selected newest to oldest, so the response budget is
        // spent on the newest runs and the cursor always points at the oldest
        // run the page actually carries. Runs the budget pushed out of the page
        // stay strictly older than that cursor, so no run is skipped.
        let ordinals: Vec<u64> = candidates
            .iter()
            .map(|candidate| candidate.binding.ordinal)
            .collect();
        let page_runs = query
            .limit_runs
            .unwrap_or(MAX_TRANSCRIPT_RUNS)
            .clamp(1, MAX_TRANSCRIPT_RUNS);
        let mut walk = PageWalk::new(&ordinals, query.before_ordinal, page_runs);
        let mut page = Vec::with_capacity(page_runs);
        while let Some(index) = walk.next_index() {
            if budget.is_exhausted() {
                walk.stop_on_exhausted_budget();
                break;
            }
            let candidate = &candidates[index];
            let projection = self
                .project_candidate(
                    &catalog.context.workspace,
                    &state_store,
                    candidate,
                    query.event_contract,
                    &mut budget,
                    &mut reasons,
                )
                .await;
            walk.record(index, projection.stop);
            if let Some(segment) = projection.segment {
                page.push(segment);
            }
        }
        page.reverse();

        let cursor = TranscriptCursor {
            next_before_ordinal: walk.next_before_ordinal(&ordinals),
        };
        Ok(transcript_response(
            session_id,
            workspace_id,
            reasons,
            page,
            Some(cursor),
        ))
    }
}

/// Drives the newest-to-oldest selection of one cursor page.
///
/// The walk needs a verdict from the projection it drives: whether reading a
/// candidate stopped on the response budget. Keeping that verdict — and the
/// page-full decision — here instead of interleaving it with the reads makes
/// the cursor rules testable without a runtime state store.
struct PageWalk {
    /// Candidate index the walk reads next, counted down from the end of the
    /// window this cursor selects.
    index: usize,
    page_runs: usize,
    /// Indices of the candidates this page carries, newest first.
    taken: Vec<usize>,
    /// The walk stopped: either the page is full, the budget is gone, or a
    /// read was cut short by the budget.
    stopped: bool,
}

impl PageWalk {
    /// `ordinals` holds the ascending transcript ordinals a cursor can select.
    fn new(ordinals: &[u64], before_ordinal: Option<u64>, page_runs: usize) -> Self {
        let end = match before_ordinal {
            Some(before) if ordinals.last().is_some_and(|newest| *newest >= before) => {
                ordinals.partition_point(|ordinal| *ordinal < before)
            }
            // A cursor past the newest run selects nothing: "older than this"
            // has no answer, and clamping it to the newest page would hand the
            // client runs it already holds.
            Some(_) => 0,
            None => ordinals.len(),
        };
        Self {
            index: end,
            page_runs: page_runs.max(1),
            taken: Vec::new(),
            stopped: false,
        }
    }

    /// The next candidate index this page should read, or `None` when the page
    /// can take nothing more.
    fn next_index(&mut self) -> Option<usize> {
        if self.stopped || self.index == 0 {
            return None;
        }
        if self.taken.len() >= self.page_runs {
            self.stopped = true;
            return None;
        }
        self.index -= 1;
        Some(self.index)
    }

    /// Records the candidate `next_index` handed out. `truncated_by_budget` is
    /// the projection's own verdict that the response budget cut the run short.
    fn record(&mut self, index: usize, truncated_by_budget: bool) {
        self.taken.push(index);
        if truncated_by_budget {
            self.stopped = true;
        }
    }

    /// The response budget was already gone, so the next candidate was never
    /// read.
    fn stop_on_exhausted_budget(&mut self) {
        self.stopped = true;
    }

    /// The cursor for the next older page. It exists exactly when the walk
    /// stopped while a candidate older than everything this page carries was
    /// left unread.
    fn next_before_ordinal(&self, ordinals: &[u64]) -> Option<u64> {
        match (self.stopped, self.taken.last()) {
            (true, Some(oldest)) if *oldest > 0 => Some(ordinals[*oldest]),
            _ => None,
        }
    }
}

/// One run the projection may emit. Candidates are ordered by ascending
/// transcript ordinal, so a cursor page is a suffix window of this list.
struct TranscriptCandidate {
    binding: ProductSessionRunBinding,
    /// Terminal source boundary of an inherited run, when the parent had one.
    inherited_through_event_seq: Option<u64>,
    source_product_session_id: Option<ProductSessionId>,
    /// The newest run of a live session can still be committing its binding.
    startup_race_possible: bool,
}

fn transcript_candidates(
    catalog: &CatalogSnapshot,
    inherited_runs: &[ProductForkInheritedRun],
    display_bindings: &[ProductSessionRunBinding],
    inherited_process_count: usize,
    process_count: usize,
) -> Vec<TranscriptCandidate> {
    let mut candidates = Vec::with_capacity(inherited_process_count + process_count);
    let bound_at = catalog
        .context
        .fork
        .as_ref()
        .map(|fork| fork.fork.created_at.as_str());
    let mut inherited_previous_run_id = None;
    for inherited in inherited_runs.iter().take(inherited_process_count) {
        let binding = inherited_transcript_binding(
            inherited,
            inherited_previous_run_id,
            bound_at.expect("inherited runs require fork provenance"),
        );
        inherited_previous_run_id = Some(inherited.runtime_run_id);
        candidates.push(TranscriptCandidate {
            binding,
            inherited_through_event_seq: inherited.through_event_seq,
            source_product_session_id: Some(inherited.source_product_session_id.clone()),
            startup_race_possible: false,
        });
    }

    let latest_binding = catalog.context.session.runtime_binding.as_ref();
    let live_session = matches!(
        catalog.context.session.status,
        crate::product::ProductSessionStatus::Running
    );
    for (index, binding) in display_bindings.iter().take(process_count).enumerate() {
        let startup_race_possible = live_session
            && latest_binding
                .is_some_and(|latest| latest.ordinal == catalog.bindings[index].ordinal);
        candidates.push(TranscriptCandidate {
            binding: binding.clone(),
            inherited_through_event_seq: None,
            source_product_session_id: None,
            startup_race_possible,
        });
    }

    candidates
}

fn inherited_transcript_binding(
    inherited: &ProductForkInheritedRun,
    resumed_from_run_id: Option<rove_runtime::types::RunId>,
    bound_at: &str,
) -> ProductSessionRunBinding {
    ProductSessionRunBinding {
        product_session_id: inherited.source_product_session_id.clone(),
        ordinal: inherited.ordinal,
        runtime_session_id: inherited.runtime_session_id,
        runtime_job_id: inherited.runtime_job_id,
        runtime_run_id: inherited.runtime_run_id,
        resumed_from_run_id,
        bound_at: bound_at.to_string(),
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

/// Durable rows one run's projection read but did not deliver.
///
/// A withheld row is a row whose kind is outside the requesting client's
/// declared event contract, or a row written by a newer build that this build
/// cannot decode at all. Both are real durable facts that the response must
/// account for without pretending they were missing or corrupt.
#[derive(Default)]
struct WithheldEvents {
    /// Lowest withheld `seq`, or `None` when nothing was withheld.
    first_seq: Option<u64>,
    /// Highest withheld `seq`; only meaningful when `first_seq` is `Some`.
    last_seq: u64,
    /// How many rows were withheld.
    count: usize,
}

impl WithheldEvents {
    fn record(&mut self, seq: u64) {
        self.first_seq.get_or_insert(seq);
        self.last_seq = seq;
        self.count += 1;
    }

    /// Highest sequence this run accounted for by withholding rather than by
    /// delivering, or `0` when nothing was withheld.
    fn accounted_through_seq(&self) -> u64 {
        match self.first_seq {
            Some(_) => self.last_seq,
            None => 0,
        }
    }
}

/// True when a row this build cannot decode into [`StreamEvent`] names a kind
/// this build does not define.
///
/// A newer writer can add a canonical kind; that row is outside this build's
/// contract, not corrupt. Anything else — malformed JSON, a missing or
/// non-string `type`, a `type` that disagrees with the stored `event_name`, or a
/// kind this build *does* define that still failed to decode — stays corrupt, so
/// the distinction never hides a genuinely broken row.
fn is_unknown_event_kind(event_json: &str, stored_event_name: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(event_json) else {
        return false;
    };
    let Some(kind) = value.get("type").and_then(|kind| kind.as_str()) else {
        return false;
    };
    kind == stored_event_name && !StreamEvent::is_known_event_name(kind)
}

/// The immutable inputs of one run's projection.
///
/// The reader itself is shared across requests, so every per-request fact —
/// including the canonical-event contract the client declared — travels here
/// instead of on the reader. Grouping them also keeps the projection helpers
/// below clippy's argument limit without weakening the lint.
#[derive(Clone, Copy)]
struct ProjectionRequest<'a> {
    workspace: &'a ProductWorkspace,
    state_store: &'a StateStore,
    binding: &'a ProductSessionRunBinding,
    /// Canonical-event contract the request declared, if any. `None` keeps the
    /// legacy contract: every row this build can decode is delivered.
    event_contract: Option<ProductTranscriptEventContract>,
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

impl ProjectionBudget {
    /// True once an event no longer fits in this response.
    fn is_exhausted(&self) -> bool {
        self.events_remaining == 0 || self.event_json_bytes_remaining == 0
    }
}

/// Page shape of a cursor response. `has_more` mirrors
/// `next_before_ordinal.is_some()` so clients can branch on either form.
#[derive(Debug, Clone, Copy)]
struct TranscriptCursor {
    next_before_ordinal: Option<u64>,
}

impl TranscriptCursor {
    /// An empty page: no older run was carried, so nothing follows it.
    fn terminal() -> Self {
        Self {
            next_before_ordinal: None,
        }
    }
}

/// Builds one projected run segment.
///
/// This is the only construction site for `answer_aborted`, so the field is
/// always derived from the events the segment itself carries — the projection
/// and the report fallback both go through here, and the fallback projects no
/// events, so it derives `false` exactly as before. See
/// `ProductTranscriptRunSegment::answer_aborted` for when its absence is
/// conclusive (only inside a `complete` response).
fn run_segment(
    binding: &ProductSessionRunBinding,
    run_status: RunStatus,
    observed_through_seq: u64,
    last_event_seq: u64,
    events: Vec<JobStreamEvent>,
    fallback: Option<ProductTranscriptFallback>,
) -> ProductTranscriptRunSegment {
    ProductTranscriptRunSegment {
        binding: binding.clone(),
        inherited: false,
        source_product_session_id: None,
        run_status,
        observed_through_seq,
        last_event_seq,
        answer_aborted: crate::types::answer_aborted(&events),
        events,
        fallback,
    }
}

fn fallback_segment(
    binding: &ProductSessionRunBinding,
    fallback: ValidatedFallback,
    last_event_seq: u64,
) -> ProductTranscriptRunSegment {
    run_segment(
        binding,
        fallback.run_status,
        0,
        last_event_seq,
        Vec::new(),
        Some(fallback.fallback),
    )
}

fn transcript_response(
    session_id: &ProductSessionId,
    workspace_id: crate::product::ProductWorkspaceId,
    partial_reasons: Vec<ProductTranscriptPartialReason>,
    segments: Vec<ProductTranscriptRunSegment>,
    cursor: Option<TranscriptCursor>,
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
        next_before_ordinal: cursor.and_then(|cursor| cursor.next_before_ordinal),
        has_more: cursor.map(|cursor| cursor.next_before_ordinal.is_some()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transcript is the surface a restored session is rebuilt from, so this
    /// segment carries the same additive marker as the job state response, read
    /// off the events the segment actually projects. It is omitted — never sent
    /// as `false` — so an unmarked run keeps the bytes it had before the field
    /// existed, and a legacy client parses it unchanged.
    #[test]
    fn a_segment_marks_only_the_salvaged_partial_it_projects() {
        let binding = ProductSessionRunBinding {
            product_session_id: ProductSessionId::new(),
            ordinal: 1,
            runtime_session_id: rove_runtime::types::SessionId::new(),
            runtime_job_id: rove_runtime::types::JobId::new(),
            runtime_run_id: rove_runtime::types::RunId::new(),
            resumed_from_run_id: None,
            bound_at: "2026-09-27T00:00:00Z".to_string(),
        };
        let aborted = segment_with(&binding, true);
        let value = serde_json::to_value(&aborted).unwrap();
        assert_eq!(value["answer_aborted"], serde_json::json!(true), "{value}");

        let complete = serde_json::to_string(&segment_with(&binding, false)).unwrap();
        assert!(
            !complete.contains("answer_aborted"),
            "an unmarked segment must keep its previous bytes: {complete}"
        );
        let decoded: ProductTranscriptRunSegment = serde_json::from_str(&complete).unwrap();
        assert!(!decoded.answer_aborted);
        assert_eq!(serde_json::to_string(&decoded).unwrap(), complete);
    }

    /// `run_segment` is the production constructor the projection uses, so this
    /// test fails if the projection stops deriving the marker — including if the
    /// field were wired as a constant.
    fn segment_with(
        binding: &ProductSessionRunBinding,
        aborted: bool,
    ) -> ProductTranscriptRunSegment {
        let event = StreamEvent::LlmMessage {
            full: "half an answer".to_string(),
            usage: rove_models::Usage::default(),
            tool_calls: Vec::new(),
            assistant_turn: None,
            aborted,
        };
        run_segment(
            binding,
            RunStatus::Cancelled,
            1,
            1,
            vec![JobStreamEvent { seq: 1, event }],
            None,
        )
    }

    #[test]
    fn only_a_well_formed_unknown_kind_is_treated_as_outside_this_contract() {
        // A newer build's row: JSON is well-formed and names itself consistently,
        // but this build has never heard of the kind.
        assert!(is_unknown_event_kind(
            r#"{"type":"future_widget_event","detail":"newer build"}"#,
            "future_widget_event"
        ));
        // A kind this build defines that failed to decode stays corrupt: the
        // envelope is readable and the row is genuinely broken.
        assert!(!is_unknown_event_kind(
            r#"{"type":"run_completed"}"#,
            "run_completed"
        ));
        // A row whose stored name disagrees with its payload is corrupt, not
        // "unknown": nothing about it can be trusted.
        assert!(!is_unknown_event_kind(
            r#"{"type":"future_widget_event"}"#,
            "some_other_event"
        ));
        assert!(!is_unknown_event_kind("not json", "future_widget_event"));
        assert!(!is_unknown_event_kind("{}", "future_widget_event"));
        assert!(!is_unknown_event_kind(
            r#"{"type":7}"#,
            "future_widget_event"
        ));
    }

    #[test]
    fn withheld_events_report_the_range_they_cover() {
        let mut withheld = WithheldEvents::default();
        assert!(withheld.first_seq.is_none());
        assert_eq!(withheld.count, 0);
        assert_eq!(withheld.accounted_through_seq(), 0);

        // The reader walks a run in `seq` order, so the range it reports is the
        // lowest and the highest withheld sequence, and the count is what keeps
        // `consumed_all_records` honest about rows that were read but not
        // delivered.
        withheld.record(4);
        withheld.record(6);
        withheld.record(9);

        assert_eq!(withheld.first_seq, Some(4));
        assert_eq!(withheld.last_seq, 9);
        assert_eq!(withheld.count, 3);
        assert_eq!(withheld.accounted_through_seq(), 9);
    }

    #[test]
    fn a_consumed_snapshot_counts_delivered_and_withheld_rows_only() {
        // `consumed_all_records` decides whether the tail checks may speak about
        // the end of the run at all, so it must be false whenever the loop broke
        // on a record. A withheld row, by contrast, *was* consumed: the loop read
        // it, accounted for it, and moved on. This is the arithmetic behind that
        // rule, kept where it can be asserted without a state store.
        let delivered = 3_usize;
        let record_count = 5_usize;

        let mut withheld = WithheldEvents::default();
        withheld.record(4);
        withheld.record(5);
        assert_eq!(delivered + withheld.count, record_count);

        // A loop that broke on its fourth record delivered three rows and withheld
        // none — the rows after the break were never read, so they cannot be
        // withheld either. The snapshot is therefore *not* consumed.
        let delivered_before_the_break = 3_usize;
        let withheld_before_the_break = 0_usize;
        assert_ne!(
            delivered_before_the_break + withheld_before_the_break,
            record_count
        );
    }

    /// One ascending candidate window: ordinals one per run.
    fn ascending_ordinals(count: u64) -> Vec<u64> {
        (1..=count).collect()
    }

    /// Runs a page walk with a scripted read outcome per candidate, returning
    /// the ordinals the page carried (ascending) and its cursor.
    fn walk_page(
        ordinals: &[u64],
        before_ordinal: Option<u64>,
        page_runs: usize,
        mut outcome: impl FnMut(u64) -> ReadOutcome,
    ) -> (Vec<u64>, Option<u64>) {
        let mut walk = PageWalk::new(ordinals, before_ordinal, page_runs);
        let mut carried = Vec::new();
        while let Some(index) = walk.next_index() {
            let ordinal = ordinals[index];
            match outcome(ordinal) {
                ReadOutcome::BudgetGone => {
                    walk.stop_on_exhausted_budget();
                    break;
                }
                ReadOutcome::Read { truncated } => {
                    walk.record(index, truncated);
                    carried.push(ordinal);
                }
            }
        }
        carried.reverse();
        (carried, walk.next_before_ordinal(ordinals))
    }

    enum ReadOutcome {
        Read { truncated: bool },
        BudgetGone,
    }

    fn read() -> ReadOutcome {
        ReadOutcome::Read { truncated: false }
    }

    fn truncated() -> ReadOutcome {
        ReadOutcome::Read { truncated: true }
    }

    /// Walks every page of a session the way the Web client does, prepending
    /// each older page, and returns the merged ascending ordinals.
    fn walk_all_pages(
        ordinals: &[u64],
        page_runs: usize,
        mut outcome: impl FnMut(u64, usize) -> ReadOutcome,
    ) -> Vec<u64> {
        let mut merged = Vec::new();
        let mut cursor: Option<u64> = None;
        let mut page = 0;
        loop {
            let (carried, next) = walk_page(ordinals, cursor, page_runs, |ordinal| {
                outcome(ordinal, page)
            });
            assert!(
                !carried.is_empty() || next.is_none(),
                "a page that carries nothing must terminate the walk"
            );
            merged.splice(0..0, carried);
            page += 1;
            assert!(page < 64, "the page walk did not terminate");
            match next {
                Some(next) => cursor = Some(next),
                None => return merged,
            }
        }
    }

    #[test]
    fn page_walk_takes_the_newest_runs_and_cursor_points_at_the_oldest_one() {
        let ordinals = ascending_ordinals(10);
        let (carried, cursor) = walk_page(&ordinals, None, 4, |_| read());
        assert_eq!(carried, vec![7, 8, 9, 10]);
        assert_eq!(cursor, Some(7));
        assert_eq!(
            walk_all_pages(&ordinals, 4, |_, _| read()),
            (1..=10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn page_walk_ends_without_a_cursor_when_no_older_run_remains() {
        let ordinals = ascending_ordinals(3);
        let (carried, cursor) = walk_page(&ordinals, None, 4, |_| read());
        assert_eq!(carried, vec![1, 2, 3]);
        assert_eq!(cursor, None);

        // A page that exactly consumes its window but leaves older runs still
        // reports a cursor; the next page then reports the end.
        let (carried, cursor) = walk_page(&ordinals, None, 3, |_| read());
        assert_eq!(carried, vec![1, 2, 3]);
        assert_eq!(cursor, None);

        let window = ascending_ordinals(6);
        let (carried, cursor) = walk_page(&window, None, 3, |_| read());
        assert_eq!(carried, vec![4, 5, 6]);
        assert_eq!(cursor, Some(4));
        let (carried, cursor) = walk_page(&window, cursor, 3, |_| read());
        assert_eq!(carried, vec![1, 2, 3]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn page_walk_selects_strictly_older_runs_for_a_cursor() {
        let ordinals = ascending_ordinals(10);
        let (carried, cursor) = walk_page(&ordinals, Some(10), 3, |_| read());
        assert_eq!(carried, vec![7, 8, 9], "the cursor run itself is excluded");
        assert_eq!(cursor, Some(7));

        // A cursor between runs keeps the same boundary.
        let (carried, _) = walk_page(&ordinals, Some(8), 3, |_| read());
        assert_eq!(carried, vec![5, 6, 7]);
    }

    #[test]
    fn page_walk_reports_an_empty_terminal_page_for_a_cursor_at_or_past_the_oldest_run() {
        let ordinals = ascending_ordinals(4);
        for cursor in [Some(1), Some(0), Some(5), Some(u64::MAX)] {
            let (carried, next) = walk_page(&ordinals, cursor, 2, |_| read());
            assert!(carried.is_empty(), "cursor {cursor:?} carried {carried:?}");
            assert_eq!(next, None, "an empty page must terminate the walk");
        }

        // An empty candidate window is also a terminal empty page.
        let (carried, next) = walk_page(&[], None, 2, |_| read());
        assert!(carried.is_empty());
        assert_eq!(next, None);
    }

    #[test]
    fn page_walk_continues_older_after_the_budget_stops_a_page() {
        // The budget disappeared after two runs, before the third read.
        let ordinals = ascending_ordinals(10);
        let (carried, cursor) = walk_page(&ordinals, None, 4, |ordinal| {
            if ordinal == 8 {
                ReadOutcome::BudgetGone
            } else {
                read()
            }
        });
        assert_eq!(carried, vec![9, 10]);
        assert_eq!(
            cursor,
            Some(9),
            "the cursor must stay strictly older than every carried run"
        );
        assert_eq!(
            walk_all_pages(&ordinals, 4, |ordinal, page| {
                if page == 0 && ordinal == 8 {
                    ReadOutcome::BudgetGone
                } else {
                    read()
                }
            }),
            (1..=10).collect::<Vec<_>>(),
            "a budget-stopped page must not skip or repeat the runs after it"
        );
    }

    #[test]
    fn page_walk_keeps_paging_after_a_run_is_truncated_by_the_budget() {
        // Run 10 was read but the budget cut it short, so its own events are
        // incomplete; the cursor still moves strictly older.
        let ordinals = ascending_ordinals(10);
        let (carried, cursor) = walk_page(&ordinals, None, 4, |ordinal| {
            if ordinal == 10 { truncated() } else { read() }
        });
        assert_eq!(carried, vec![10]);
        assert_eq!(cursor, Some(10));
        assert_eq!(
            walk_all_pages(&ordinals, 4, |ordinal, _| {
                if ordinal == 10 { truncated() } else { read() }
            }),
            (1..=10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn page_walk_cursor_exists_exactly_when_an_older_run_was_left_unread() {
        let ordinals = ascending_ordinals(5);
        // Everything was read: nothing follows.
        let (carried, next) = walk_page(&ordinals, None, 5, |_| read());
        assert_eq!(carried, (1..=5).collect::<Vec<_>>());
        assert_eq!(next, None);

        // The page filled up while older runs remain.
        let (carried, next) = walk_page(&ordinals, None, 2, |_| read());
        assert_eq!(carried, vec![4, 5]);
        assert_eq!(next, Some(4));

        // The budget stopped the page early while older runs remain.
        let (carried, next) = walk_page(&ordinals, None, 5, |ordinal| {
            if ordinal == 4 {
                ReadOutcome::BudgetGone
            } else {
                read()
            }
        });
        assert_eq!(carried, vec![5]);
        assert_eq!(next, Some(5));

        // The walk reached the oldest run.
        let (carried, next) = walk_page(&ordinals, Some(2), 5, |_| read());
        assert_eq!(carried, vec![1]);
        assert_eq!(next, None);
    }
}
