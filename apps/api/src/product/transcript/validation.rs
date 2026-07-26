use std::collections::HashSet;
use std::io::ErrorKind;

use rove_runtime::WorkspaceKind;
use rove_runtime::state::index::RunIndexRecord;
use rove_runtime::state::report::RunReport;
use rove_runtime::types::{RunStatus, TerminationReason};

use crate::product::{
    ProductErrorCode, ProductRuntimeBinding, ProductSessionId, ProductSessionRunBinding,
    ProductTranscriptPartialReason, ProductTranscriptPartialReasonCode, ProductWorkspace,
    ProductWorkspaceKind,
};

pub(super) fn latest_binding_matches(
    summary: Option<&ProductRuntimeBinding>,
    latest: Option<&ProductSessionRunBinding>,
) -> bool {
    match (summary, latest) {
        (None, None) => true,
        (Some(summary), Some(latest)) => {
            summary.ordinal == latest.ordinal
                && summary.runtime_session_id == latest.runtime_session_id
                && summary.latest_job_id == latest.runtime_job_id
                && summary.latest_run_id == latest.runtime_run_id
        }
        _ => false,
    }
}

pub(super) fn binding_prefix_len(
    session_id: &ProductSessionId,
    bindings: &[ProductSessionRunBinding],
    reasons: &mut Vec<ProductTranscriptPartialReason>,
) -> usize {
    for (index, binding) in bindings.iter().enumerate() {
        let expected_ordinal = index as u64 + 1;
        if &binding.product_session_id != session_id {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    binding,
                    None,
                    None,
                ),
            );
            return index;
        }
        if binding.ordinal != expected_ordinal {
            push_reason(
                reasons,
                ProductTranscriptPartialReason {
                    code: ProductTranscriptPartialReasonCode::MissingRunMapping,
                    run_ordinal: Some(expected_ordinal),
                    run_id: Some(binding.runtime_run_id),
                    expected_seq: None,
                    observed_seq: None,
                },
            );
            return index;
        }
    }
    bindings.len()
}

pub(super) fn runtime_chain_prefix_len(
    bindings: &[ProductSessionRunBinding],
    reasons: &mut Vec<ProductTranscriptPartialReason>,
) -> usize {
    let Some(first) = bindings.first() else {
        return 0;
    };
    if first.resumed_from_run_id.is_some() {
        push_reason(
            reasons,
            reason_for_binding(
                ProductTranscriptPartialReasonCode::MissingRunMapping,
                first,
                None,
                None,
            ),
        );
    }

    let mut run_ids = HashSet::with_capacity(bindings.len());
    run_ids.insert(first.runtime_run_id);

    for index in 1..bindings.len() {
        let previous = &bindings[index - 1];
        let binding = &bindings[index];
        if binding.runtime_session_id != first.runtime_session_id
            || binding.runtime_job_id != first.runtime_job_id
        {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    binding,
                    None,
                    None,
                ),
            );
            return index;
        }
        if !run_ids.insert(binding.runtime_run_id) {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch,
                    binding,
                    None,
                    None,
                ),
            );
            return index;
        }
        if binding.resumed_from_run_id != Some(previous.runtime_run_id) {
            push_reason(
                reasons,
                reason_for_binding(
                    ProductTranscriptPartialReasonCode::MissingRunMapping,
                    binding,
                    None,
                    None,
                ),
            );
            return index;
        }
    }
    bindings.len()
}

pub(super) fn run_identity_matches(
    run: &RunIndexRecord,
    binding: &ProductSessionRunBinding,
) -> bool {
    run.run_id == binding.runtime_run_id
        && run.session_id == binding.runtime_session_id
        && run.job_id == binding.runtime_job_id
}

pub(super) fn report_identity_matches(
    report: &RunReport,
    workspace: &ProductWorkspace,
    binding: &ProductSessionRunBinding,
) -> bool {
    report.session_id == binding.runtime_session_id
        && report.job_id == binding.runtime_job_id
        && report.run_id == binding.runtime_run_id
        && report.workspace_root == workspace.canonical_root
        && workspace_kind_matches(workspace.kind, &report.workspace_kind)
}

fn workspace_kind_matches(product: ProductWorkspaceKind, runtime: &WorkspaceKind) -> bool {
    matches!(
        (product, runtime),
        (ProductWorkspaceKind::Folder, WorkspaceKind::Folder)
            | (ProductWorkspaceKind::Repo, WorkspaceKind::Repo)
    )
}

pub(super) fn parse_run_status(value: &str) -> Option<RunStatus> {
    match value {
        "init" => Some(RunStatus::Init),
        "running" => Some(RunStatus::Running),
        "done" => Some(RunStatus::Done),
        "error" => Some(RunStatus::Error),
        "cancelled" => Some(RunStatus::Cancelled),
        "interrupted" => Some(RunStatus::Interrupted),
        _ => None,
    }
}

fn terminal_status_for_reason(reason: &TerminationReason) -> RunStatus {
    match reason {
        TerminationReason::Final
        | TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => RunStatus::Done,
        TerminationReason::Error => RunStatus::Error,
        TerminationReason::Cancelled => RunStatus::Cancelled,
    }
}

fn requires_terminal_event(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Done | RunStatus::Error | RunStatus::Cancelled | RunStatus::Interrupted
    )
}

pub(super) fn is_live_status(status: &RunStatus) -> bool {
    matches!(status, RunStatus::Init | RunStatus::Running)
}

pub(super) struct TerminalConsistencyIssue {
    pub(super) code: ProductTranscriptPartialReasonCode,
    pub(super) expected_seq: Option<u64>,
    pub(super) observed_seq: Option<u64>,
}

/// Validate the indexed lifecycle without turning normal live-write ordering
/// into a fake event gap. A live status with a terminal event is unstable, and
/// an interrupted status without one remains honestly partial.
pub(super) fn terminal_consistency_issue(
    run_status: &RunStatus,
    terminal: Option<&(u64, RunStatus)>,
    high_water_seq: u64,
) -> Option<TerminalConsistencyIssue> {
    let Some((terminal_seq, terminal_status)) = terminal else {
        return requires_terminal_event(run_status).then_some(TerminalConsistencyIssue {
            code: ProductTranscriptPartialReasonCode::MissingEventRange,
            expected_seq: Some(high_water_seq.saturating_add(1)),
            observed_seq: Some(high_water_seq),
        });
    };

    if *terminal_seq != high_water_seq {
        return Some(TerminalConsistencyIssue {
            code: ProductTranscriptPartialReasonCode::CorruptEvent,
            expected_seq: Some(high_water_seq),
            observed_seq: Some(*terminal_seq),
        });
    }
    if is_live_status(run_status) {
        return Some(TerminalConsistencyIssue {
            code: ProductTranscriptPartialReasonCode::RuntimeStateUnavailable,
            expected_seq: None,
            observed_seq: None,
        });
    }
    if run_status == &RunStatus::Interrupted || terminal_status != run_status {
        return Some(TerminalConsistencyIssue {
            code: ProductTranscriptPartialReasonCode::CorruptArtifact,
            expected_seq: Some(high_water_seq),
            observed_seq: Some(*terminal_seq),
        });
    }
    None
}

pub(super) fn validated_report_status(report: &RunReport) -> Option<RunStatus> {
    let expected_label = match &report.termination_reason {
        TerminationReason::Final => "success",
        TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => "incomplete",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    };
    if report.status != expected_label {
        return None;
    }
    Some(terminal_status_for_reason(&report.termination_reason))
}

pub(super) fn runtime_read_reason(kind: ErrorKind) -> ProductTranscriptPartialReasonCode {
    if kind == ErrorKind::InvalidData {
        ProductTranscriptPartialReasonCode::CorruptArtifact
    } else {
        ProductTranscriptPartialReasonCode::RuntimeStateUnavailable
    }
}

pub(super) fn is_returnable_runtime_error(code: ProductErrorCode) -> bool {
    matches!(
        code,
        ProductErrorCode::ProductSessionRuntimeStateMissing
            | ProductErrorCode::ProductSessionRuntimeStateCorrupt
    )
}

pub(super) fn reason_for_binding(
    code: ProductTranscriptPartialReasonCode,
    binding: &ProductSessionRunBinding,
    expected_seq: Option<u64>,
    observed_seq: Option<u64>,
) -> ProductTranscriptPartialReason {
    ProductTranscriptPartialReason {
        code,
        run_ordinal: Some(binding.ordinal),
        run_id: Some(binding.runtime_run_id),
        expected_seq,
        observed_seq,
    }
}

pub(super) fn push_reason(
    reasons: &mut Vec<ProductTranscriptPartialReason>,
    reason: ProductTranscriptPartialReason,
) {
    let duplicate = reasons.iter().any(|existing| {
        existing.code == reason.code
            && existing.run_ordinal == reason.run_ordinal
            && existing.run_id == reason.run_id
            && existing.expected_seq == reason.expected_seq
            && existing.observed_seq == reason.observed_seq
    });
    if !duplicate {
        reasons.push(reason);
    }
}

pub(super) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_runtime::types::{JobId, RunId, SessionId};

    fn binding(
        product_session_id: &ProductSessionId,
        ordinal: u64,
        runtime_session_id: SessionId,
        runtime_job_id: JobId,
        runtime_run_id: RunId,
        resumed_from_run_id: Option<RunId>,
    ) -> ProductSessionRunBinding {
        ProductSessionRunBinding {
            product_session_id: product_session_id.clone(),
            ordinal,
            runtime_session_id,
            runtime_job_id,
            runtime_run_id,
            resumed_from_run_id,
            bound_at: "2026-07-26T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn binding_ordinals_are_one_based_and_contiguous() {
        let product_session_id = ProductSessionId::new();
        let runtime_session_id = SessionId::new();
        let runtime_job_id = JobId::new();
        let first_run_id = RunId::new();
        let second_run_id = RunId::new();
        let bindings = vec![
            binding(
                &product_session_id,
                1,
                runtime_session_id,
                runtime_job_id,
                first_run_id,
                None,
            ),
            binding(
                &product_session_id,
                3,
                runtime_session_id,
                runtime_job_id,
                second_run_id,
                Some(first_run_id),
            ),
        ];
        let mut reasons = Vec::new();

        let prefix = binding_prefix_len(&product_session_id, &bindings, &mut reasons);

        assert_eq!(prefix, 1);
        assert_eq!(reasons.len(), 1);
        assert_eq!(
            reasons[0].code,
            ProductTranscriptPartialReasonCode::MissingRunMapping
        );
        assert_eq!(reasons[0].run_ordinal, Some(2));
    }

    #[test]
    fn runtime_chain_rejects_cross_session_stitching() {
        let product_session_id = ProductSessionId::new();
        let runtime_job_id = JobId::new();
        let first_run_id = RunId::new();
        let bindings = vec![
            binding(
                &product_session_id,
                1,
                SessionId::new(),
                runtime_job_id,
                first_run_id,
                None,
            ),
            binding(
                &product_session_id,
                2,
                SessionId::new(),
                runtime_job_id,
                RunId::new(),
                Some(first_run_id),
            ),
        ];
        let mut reasons = Vec::new();

        let prefix = runtime_chain_prefix_len(&bindings, &mut reasons);

        assert_eq!(prefix, 1);
        assert_eq!(
            reasons[0].code,
            ProductTranscriptPartialReasonCode::RuntimeIdentityMismatch
        );
    }

    #[test]
    fn fallback_summary_truncation_preserves_utf8_boundaries() {
        let value = "a界b";

        assert_eq!(truncate_utf8(value, 2), "a");
        assert_eq!(truncate_utf8(value, 4), "a界");
    }

    #[test]
    fn interrupted_run_without_canonical_terminal_is_partial() {
        let issue = terminal_consistency_issue(&RunStatus::Interrupted, None, 4)
            .expect("interrupted run must not be reported complete");

        assert_eq!(
            issue.code,
            ProductTranscriptPartialReasonCode::MissingEventRange
        );
        assert_eq!(issue.expected_seq, Some(5));
        assert_eq!(issue.observed_seq, Some(4));
    }

    #[test]
    fn live_status_with_terminal_event_is_partial_without_fake_gap() {
        let terminal = (4, RunStatus::Done);
        let issue = terminal_consistency_issue(&RunStatus::Running, Some(&terminal), 4)
            .expect("live status with terminal event must expose unstable state");

        assert_eq!(
            issue.code,
            ProductTranscriptPartialReasonCode::RuntimeStateUnavailable
        );
        assert_eq!(issue.expected_seq, None);
        assert_eq!(issue.observed_seq, None);
    }
}
