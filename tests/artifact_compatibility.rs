use rove_runtime::events::StreamEvent;
use rove_runtime::state::report::RunReport;
use rove_runtime::types::TaskState;

#[test]
fn pre_lifecycle_task_state_fixture_keeps_additive_defaults() {
    let state: TaskState = serde_json::from_str(include_str!(
        "fixtures/artifacts/pre-lifecycle-task-state.json"
    ))
    .unwrap();

    assert!(state.checkpoint.is_none());
    assert!(state.runtime_identity.is_none());
    assert!(state.step_ledger.is_empty());
}

#[test]
fn pre_lifecycle_report_fixture_keeps_additive_defaults() {
    let report: RunReport =
        serde_json::from_str(include_str!("fixtures/artifacts/pre-lifecycle-report.json")).unwrap();

    assert!(report.step_records.is_empty());
    assert!(report.plan_decisions.is_empty());
    assert!(report.plan_revisions.is_empty());
}

#[test]
fn pre_lifecycle_trace_fixture_remains_readable() {
    let events = include_str!("fixtures/artifacts/pre-lifecycle-trace.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .collect::<Vec<_>>();

    assert!(matches!(
        &events[0],
        StreamEvent::PlanCreated {
            identity,
            plan_revision: None,
            ..
        } if !identity.is_complete()
    ));
    assert!(matches!(
        &events[1],
        StreamEvent::PlanStepStarted { attempt, .. } if !attempt.is_complete()
    ));
}
