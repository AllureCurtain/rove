use crate::schema::{
    BenchmarkCheck, BenchmarkFile, BenchmarkSuite, BenchmarkTask, BenchmarkTurn, ProfileParams,
    shell_echo,
};

/// Generate the "dataprep" suite for a given profile.
///
/// Phases:
/// 1. Read input data files
/// 2. Generate intermediate summary files
/// 3. Encounter and recover from a failure
/// 4. Aggregate results via shell and write final report
///
/// Stress profile can also include a true cancel+resume task.
pub fn generate_dataprep_suite(params: &ProfileParams) -> BenchmarkSuite {
    let mut tasks = Vec::new();
    let datasets = ["sales", "users", "events", "logs", "metrics", "inventory"];

    let regular_count = if params.include_cancel_resume {
        params.task_count.saturating_sub(1).max(1)
    } else {
        params.task_count
    };

    for i in 0..regular_count {
        let dataset = datasets[i % datasets.len()];
        tasks.push(build_dataprep_task(i, dataset, params, false));
    }

    if params.include_cancel_resume {
        let idx = regular_count;
        let dataset = datasets[idx % datasets.len()];
        tasks.push(build_dataprep_task(idx, dataset, params, true));
    }

    BenchmarkSuite {
        name: "dataprep".to_string(),
        description: "Data preparation pipeline: read inputs, write intermediates, recover from failure, produce final report.".to_string(),
        tasks,
    }
}

fn build_dataprep_task(
    idx: usize,
    dataset: &str,
    params: &ProfileParams,
    cancel_resume: bool,
) -> BenchmarkTask {
    let task_name = if cancel_resume {
        format!("dataprep_{idx:03}_{dataset}_resume")
    } else {
        format!("dataprep_{idx:03}_{dataset}")
    };
    let num_inputs = params.input_files_per_task;

    let mut setup_files = Vec::new();
    for j in 0..num_inputs {
        let filename = format!("inputs/{dataset}-part{j}.csv");
        let header = format!("id,{dataset},value,timestamp\n");
        let mut rows = String::new();
        for r in 0..5 {
            rows.push_str(&format!(
                "{r},{dataset}_{j},{},2024-01-{:02}T10:00:00Z\n",
                r * 10 + j,
                r + 1
            ));
        }
        setup_files.push(BenchmarkFile {
            path: filename,
            content: header + &rows,
        });
    }
    setup_files.push(BenchmarkFile {
        path: format!("inputs/{dataset}-manifest.json"),
        content: serde_json::to_string_pretty(&serde_json::json!({
            "dataset": dataset,
            "parts": num_inputs,
            "format": "csv",
            "version": "1.0"
        }))
        .unwrap(),
    });

    let mut turns = Vec::new();

    // Phase 1: Read inputs
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{idx}_read0"),
        name: "read_file".to_string(),
        args: serde_json::json!({ "path": format!("inputs/{dataset}-part0.csv") }),
    });

    // Phase 2: Write intermediate summary
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{idx}_write_summary"),
        name: "write_file".to_string(),
        args: serde_json::json!({
            "path": format!("intermediates/{dataset}-summary.json"),
            "content": serde_json::json!({
                "dataset": dataset,
                "rows_processed": 5 * num_inputs,
                "columns": ["id", dataset, "value", "timestamp"],
                "status": "summarized"
            }).to_string()
        }),
    });

    // Phase 3: Recoverable failure
    if params.include_failure_recovery {
        turns.push(BenchmarkTurn::ToolUse {
            id: format!("call_{idx}_fail_read"),
            name: "read_file".to_string(),
            args: serde_json::json!({ "path": format!("inputs/{dataset}-MISSING.csv") }),
        });
        turns.push(BenchmarkTurn::ToolUse {
            id: format!("call_{idx}_read_manifest"),
            name: "read_file".to_string(),
            args: serde_json::json!({ "path": format!("inputs/{dataset}-manifest.json") }),
        });
    }

    // Phase 4: Aggregate via shell + write artifacts
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{idx}_shell"),
        name: "run_shell".to_string(),
        args: serde_json::json!({ "command": shell_echo(&format!("aggregate_phase_{dataset}")) }),
    });
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{idx}_write_report"),
        name: "write_file".to_string(),
        args: serde_json::json!({
            "path": format!("outputs/{dataset}-final-report.json"),
            "content": serde_json::json!({
                "dataset": dataset,
                "task_index": idx,
                "phases_completed": ["read", "summarize", "recover", "aggregate"],
                "total_input_files": num_inputs + 1,
                "intermediates_generated": 1,
                "failures_encountered": if params.include_failure_recovery { 1 } else { 0 },
                "final_status": "complete",
                "cancel_resume": cancel_resume,
                "artifacts": [
                    format!("intermediates/{dataset}-summary.json"),
                    format!("outputs/{dataset}-final-report.json")
                ]
            }).to_string()
        }),
    });
    turns.push(BenchmarkTurn::ToolUse {
        id: format!("call_{idx}_write_evidence"),
        name: "write_file".to_string(),
        args: serde_json::json!({
            "path": format!("outputs/{dataset}-evidence.md"),
            "content": format!(
                "# Evidence for {dataset}\n\n- Dataset: {dataset}\n- Task index: {idx}\n- Input files read: {num_inputs}\n- Failures recovered: {}\n- Cancel resume: {cancel_resume}\n- Status: PASS\n",
                if params.include_failure_recovery { 1 } else { 0 }
            )
        }),
    });
    turns.push(BenchmarkTurn::Text {
        text: format!(
            "Data preparation for {dataset} complete. Processed {} input files, generated summary and final report. Encountered and recovered from 1 missing file error. Artifacts in outputs/ directory.",
            num_inputs + 1
        ),
    });

    let mut checks = Vec::new();
    checks.push(BenchmarkCheck::FileExists {
        path: format!("intermediates/{dataset}-summary.json"),
        description: "Intermediate summary file exists".to_string(),
    });
    checks.push(BenchmarkCheck::FileContentContains {
        path: format!("outputs/{dataset}-final-report.json"),
        substring: "\"final_status\":\"complete\"".to_string(),
        description: "Final report contains complete status".to_string(),
    });
    if params.include_failure_recovery {
        checks.push(BenchmarkCheck::TraceHasEvent {
            event_type: "tool_call_failed".to_string(),
            description: "Trace records the recoverable tool failure".to_string(),
        });
    }
    checks.push(BenchmarkCheck::TraceHasEvent {
        event_type: "run_completed".to_string(),
        description: "Trace records run completion".to_string(),
    });
    checks.push(BenchmarkCheck::CommandOracle {
        command: shell_echo("oracle_verification_passed"),
        workdir: None,
        expected_stdout_contains: Some("oracle_verification_passed".to_string()),
        description: "Shell command executes and produces expected stdout".to_string(),
    });
    checks.push(BenchmarkCheck::FileExists {
        path: format!("outputs/{dataset}-evidence.md"),
        description: "Evidence markdown file exists".to_string(),
    });
    checks.push(BenchmarkCheck::ReportField {
        field: "tool_calls".to_string(),
        equals: None,
        min: Some(3),
        description: "Report records multiple tool calls".to_string(),
    });
    checks.push(BenchmarkCheck::ArtifactExists {
        name: "report.json".to_string(),
        description: "Run report artifact exists".to_string(),
    });
    if cancel_resume {
        checks.push(BenchmarkCheck::TraceHasEvent {
            event_type: "run_started".to_string(),
            description: "Merged trace still contains run lifecycle events".to_string(),
        });
    }

    // Cancel after intermediate write + optional recovery reads, before final aggregation.
    // Turn count is 1-based LLM turns; keep this before the final Text answer.
    let cancel_resume_after_turns = if cancel_resume {
        // read + write_summary + (fail_read + read_manifest)? => cancel before shell aggregate
        Some(if params.include_failure_recovery {
            4
        } else {
            2
        })
    } else {
        None
    };

    BenchmarkTask {
        name: task_name,
        message: format!(
            "Process the {dataset} dataset: read inputs from inputs/, create intermediate summaries, handle any errors gracefully, and produce a final report in outputs/."
        ),
        setup_files,
        turns,
        max_steps: params.max_steps,
        checks,
        expected_output_contains: vec![dataset.to_string(), "complete".to_string()],
        expected_files: vec![BenchmarkFile {
            path: format!("outputs/{dataset}-final-report.json"),
            content: String::new(),
        }],
        expected_summary_contains: Vec::new(),
        requires_network: Some(false),
        resume_state: None,
        cancel_resume_after_turns,
    }
}
