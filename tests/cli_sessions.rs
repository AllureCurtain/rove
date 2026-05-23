use rove::core::types::{JobId, RunId, SessionId, TaskState};
use rove::interfaces::cli::sessions::format_task_states;

fn task_state(
    run_id: RunId,
    session_id: SessionId,
    job_id: JobId,
    goal: &str,
    step: u32,
) -> TaskState {
    TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: goal.to_string(),
        step,
        history: vec![],
        summary: Some(format!("summary for {goal}")),
        plan: None,
    }
}

#[test]
fn format_task_states_lists_resumable_runs() {
    let session_id = SessionId::new();
    let first_run = RunId::new();
    let second_run = RunId::new();
    let first = task_state(first_run, session_id, JobId::new(), "inspect", 2);
    let second = task_state(second_run, session_id, JobId::new(), "continue", 5);

    let output = format_task_states(&[second, first]);

    assert!(output.contains("run_id"));
    assert!(output.contains(&second_run.to_string()));
    assert!(output.contains(&first_run.to_string()));
    assert!(output.find(&second_run.to_string()) < output.find(&first_run.to_string()));
    assert!(output.contains("continue"));
    assert!(output.contains("step 5"));
}

#[test]
fn format_task_states_handles_empty_state_list() {
    let output = format_task_states(&[]);

    assert_eq!(output.trim(), "No resumable task states found.");
}
