use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::core::types::{RunId, TerminationReason, Usage};

/// Summary report for a completed run.
///
/// Written to `.rove/runs/<run_id>/report.json` after the run finishes.
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub run_id: RunId,
    pub status: String,
    pub termination_reason: TerminationReason,
    pub steps: u32,
    pub total_usage: Usage,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub output: Option<String>,
    pub timestamp: String,
}

impl RunReport {
    pub fn new(run_id: RunId, reason: TerminationReason) -> Self {
        Self {
            run_id,
            status: match &reason {
                TerminationReason::Final => "success".to_string(),
                TerminationReason::Error => "error".to_string(),
                TerminationReason::Cancelled => "cancelled".to_string(),
                _ => "incomplete".to_string(),
            },
            termination_reason: reason,
            steps: 0,
            total_usage: Usage::default(),
            tool_calls: 0,
            tool_failures: 0,
            output: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Write a report to the run directory.
pub fn write_report(run_dir: &Path, report: &RunReport) -> std::io::Result<()> {
    fs::create_dir_all(run_dir)?;
    let path = run_dir.join("report.json");
    let json = serde_json::to_string_pretty(report).map_err(std::io::Error::other)?;
    fs::write(path, json)
}
