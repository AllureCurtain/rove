use super::schema::{BenchmarkOutcome, BenchmarkReport};

pub fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "run".to_string()
    } else {
        out
    }
}

pub fn render_summary_md(report: &BenchmarkReport) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Benchmark Run: {}\n\n", report.suite));
    md.push_str(&format!("- **Profile**: {}\n", report.profile));
    md.push_str(&format!("- **Started**: {}\n", report.started_at));
    md.push_str(&format!("- **Finished**: {}\n", report.finished_at));
    md.push_str(&format!(
        "- **Result**: {} / {} passed\n\n",
        report.passed_tasks, report.total_tasks
    ));
    md.push_str("## Tasks\n\n");
    md.push_str("| Task | Outcome | Steps | Tool Calls | Failures | Checks Passed | Resumed |\n");
    md.push_str("|------|---------|-------|------------|----------|---------------|---------|\n");
    for task in &report.tasks {
        let checks_passed = task.check_results.iter().filter(|c| c.passed).count();
        let checks_total = task.check_results.len();
        let outcome = match task.outcome {
            BenchmarkOutcome::Passed => "PASS",
            BenchmarkOutcome::Failed => "FAIL",
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {}/{} | {} |\n",
            task.name,
            outcome,
            task.steps,
            task.tool_calls,
            task.tool_failures,
            checks_passed,
            checks_total,
            if task.resumed { "yes" } else { "no" }
        ));
    }
    md.push_str("\n## Failed Checks\n\n");
    let mut any_failures = false;
    for task in &report.tasks {
        for check in &task.check_results {
            if !check.passed {
                any_failures = true;
                md.push_str(&format!(
                    "- **{}** [{}]: {}\n",
                    task.name, check.kind, check.detail
                ));
            }
        }
        for failure in &task.failures {
            any_failures = true;
            md.push_str(&format!("- **{}**: {}\n", task.name, failure));
        }
    }
    if !any_failures {
        md.push_str("All checks passed.\n");
    }
    md.push_str(&format!(
        "\n## Evidence Location\n\n`{}`\n",
        report.evidence_root.display()
    ));
    md
}
