use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // tests package lives at <workspace>/tests
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

use std::collections::BTreeSet;

use rove_bench::{
    BenchmarkOutcome, default_profile_params, generate_dataprep_suite, load_benchmark_suite,
    resolve_suite, run_benchmark_suite, stress_profile_params,
};

#[tokio::test]
async fn default_benchmark_suite_has_at_least_three_no_network_tasks() {
    let suite = load_benchmark_suite(workspace_path("benchmarks/agent-smoke.json"))
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
    let suite = load_benchmark_suite(workspace_path("benchmarks/agent-smoke.json"))
        .await
        .unwrap();

    let report = run_benchmark_suite(&suite, tmp.path(), "default")
        .await
        .unwrap();

    assert!(report.passed, "{report:?}");
    assert!(report.tasks.len() >= 3);
    assert!(report.evidence_root.is_dir());
    assert!(report.evidence_root.join("metrics.json").is_file());
    assert!(report.evidence_root.join("summary.md").is_file());
    for task in &report.tasks {
        assert_eq!(task.outcome, BenchmarkOutcome::Passed, "{task:?}");
        assert!(task.artifacts.run_dir.is_dir(), "{task:?}");
        assert!(task.artifacts.trace_jsonl.is_file(), "{task:?}");
        assert!(task.artifacts.task_state_json.is_file(), "{task:?}");
        assert!(task.artifacts.report_json.is_file(), "{task:?}");
    }
}

#[tokio::test]
async fn dataprep_default_profile_passes_with_multi_phase_checks() {
    let tmp = tempfile::TempDir::new().unwrap();
    let suite = generate_dataprep_suite(&default_profile_params());
    assert!(suite.tasks.len() >= 4);

    let report = run_benchmark_suite(&suite, tmp.path(), "default")
        .await
        .unwrap();
    assert!(report.passed, "{report:?}");
    assert_eq!(report.failed_tasks, 0);
    for task in &report.tasks {
        assert!(
            task.check_results.iter().any(|c| c.kind == "file_exists"),
            "{task:?}"
        );
        assert!(
            task.check_results
                .iter()
                .any(|c| c.kind == "file_content_contains"),
            "{task:?}"
        );
        assert!(
            task.check_results
                .iter()
                .any(|c| c.kind == "trace_has_event"),
            "{task:?}"
        );
        assert!(
            task.check_results
                .iter()
                .any(|c| c.kind == "command_oracle"),
            "{task:?}"
        );
        assert!(
            task.check_results.iter().any(|c| c.kind == "report_field"),
            "{task:?}"
        );
        assert!(
            task.check_results
                .iter()
                .any(|c| c.kind == "artifact_exists"),
            "{task:?}"
        );
    }
}

#[tokio::test]
async fn dataprep_stress_includes_cancel_resume_task() {
    let tmp = tempfile::TempDir::new().unwrap();
    let suite = generate_dataprep_suite(&stress_profile_params());
    assert!(
        suite
            .tasks
            .iter()
            .any(|t| t.cancel_resume_after_turns.is_some()),
        "stress suite should include a cancel+resume task"
    );

    let report = run_benchmark_suite(&suite, tmp.path(), "stress")
        .await
        .unwrap();
    assert!(report.passed, "{report:?}");
    assert!(
        report.tasks.iter().any(|t| t.resumed),
        "at least one task should report resumed=true: {:?}",
        report.tasks
    );
}

#[test]
fn dataprep_stress_profile_meets_scale_floors() {
    let params = stress_profile_params();
    let suite = generate_dataprep_suite(&params);
    assert!(suite.tasks.len() >= 12, "task count {}", suite.tasks.len());

    let mut input_files = 0usize;
    for task in &suite.tasks {
        input_files += task
            .setup_files
            .iter()
            .filter(|f| f.path.starts_with("inputs/"))
            .count();
    }
    assert!(input_files >= 20, "input files {input_files}");

    let phases = ["read", "summarize", "recover", "aggregate"];
    let scripted = serde_json::to_string(&suite.tasks[0].turns).unwrap();
    for phase in phases {
        assert!(scripted.contains(phase), "missing phase {phase}");
    }
}

#[test]
fn resolve_suite_supports_dataprep_and_agent_smoke() {
    let dataprep = resolve_suite("dataprep", "default").unwrap();
    assert_eq!(dataprep.name, "dataprep");
    assert!(dataprep.tasks.len() >= 4);

    let smoke = resolve_suite("agent-smoke", "default").unwrap();
    assert_eq!(smoke.name, "agent-smoke");
    assert!(smoke.tasks.len() >= 3);
}

#[test]
fn acceptance_matrix_covers_m0_to_m6_with_concrete_verification() {
    let content =
        std::fs::read_to_string(workspace_path("docs/runtime/acceptance-matrix.md")).unwrap();

    for milestone in ["M0", "M1", "M2", "M3", "M4", "M5", "M6"] {
        assert!(content.contains(&format!("| {milestone} |")), "{milestone}");
    }

    assert!(!content.contains("| manual |"));
    assert!(!content.contains("TBD"));
    assert!(!content.contains("TODO"));
    assert!(content.contains("cargo test"));
    assert!(content.contains("cargo run -p rove-bench"));

    let guide =
        std::fs::read_to_string(workspace_path("docs/runtime/implementation-guide.md")).unwrap();
    assert!(guide.contains("docs/runtime/acceptance-matrix.md"));
}
