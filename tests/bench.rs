use std::collections::BTreeSet;

use rove::bench::{BenchmarkOutcome, load_benchmark_suite, run_benchmark_suite};

#[tokio::test]
async fn default_benchmark_suite_has_at_least_three_no_network_tasks() {
    let suite = load_benchmark_suite("benchmarks/agent-smoke.json")
        .await
        .unwrap();

    assert!(suite.tasks.len() >= 3);
    let names = suite
        .tasks
        .iter()
        .map(|task| task.name.as_str())
        .collect::<BTreeSet<_>>();
    assert!(names.contains("echo_smoke"));
    assert!(names.contains("write_file"));
    assert!(names.contains("resume_interrupted"));
    assert!(
        suite
            .tasks
            .iter()
            .all(|task| task.requires_network == Some(false))
    );
}

#[tokio::test]
async fn default_benchmark_suite_passes_and_reports_artifact_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let suite = load_benchmark_suite("benchmarks/agent-smoke.json")
        .await
        .unwrap();

    let report = run_benchmark_suite(&suite, tmp.path()).await.unwrap();

    assert!(report.passed);
    assert!(report.tasks.len() >= 3);
    for task in &report.tasks {
        assert_eq!(task.outcome, BenchmarkOutcome::Passed, "{task:?}");
        assert!(task.artifacts.run_dir.is_dir(), "{task:?}");
        assert!(task.artifacts.trace_jsonl.is_file(), "{task:?}");
        assert!(task.artifacts.task_state_json.is_file(), "{task:?}");
        assert!(task.artifacts.report_json.is_file(), "{task:?}");
    }
}

#[test]
fn acceptance_matrix_covers_m0_to_m6_with_concrete_verification() {
    let content = std::fs::read_to_string("docs/runtime/acceptance-matrix.md").unwrap();

    for milestone in ["M0", "M1", "M2", "M3", "M4", "M5", "M6"] {
        assert!(content.contains(&format!("| {milestone} |")), "{milestone}");
    }

    assert!(!content.contains("| manual |"));
    assert!(!content.contains("TBD"));
    assert!(!content.contains("TODO"));
    assert!(content.contains("cargo test"));
    assert!(content.contains("cargo run --bin rove-bench"));

    let guide = std::fs::read_to_string("docs/runtime/implementation-guide.md").unwrap();
    assert!(guide.contains("docs/runtime/acceptance-matrix.md"));
}
