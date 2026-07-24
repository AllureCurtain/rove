use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A benchmark suite definition, either loaded from JSON or generated in code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<BenchmarkTask>,
}

/// Resume state from a benchmark JSON (subset of TaskState fields).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchmarkResumeState {
    #[serde(default)]
    pub goal: String,
    /// Accept either a bare string (`"summary": "..."`) or an optional field.
    #[serde(default, deserialize_with = "deserialize_optional_stringish")]
    pub summary: Option<String>,
}

fn deserialize_optional_stringish<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s),
        Some(other) => Some(other.to_string()),
    })
}

/// A single benchmark task within a suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub name: String,
    pub message: String,
    #[serde(default)]
    pub setup_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub turns: Vec<BenchmarkTurn>,
    #[serde(default)]
    pub max_steps: u32,
    #[serde(default)]
    pub checks: Vec<BenchmarkCheck>,
    #[serde(default)]
    pub expected_output_contains: Vec<String>,
    #[serde(default)]
    pub expected_files: Vec<BenchmarkFile>,
    #[serde(default)]
    pub expected_summary_contains: Vec<String>,
    #[serde(default)]
    pub requires_network: Option<bool>,
    #[serde(default)]
    pub resume_state: Option<BenchmarkResumeState>,
    /// When set, the runner cancels after this many LLM turns and resumes from checkpoint.
    #[serde(default)]
    pub cancel_resume_after_turns: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BenchmarkTurn {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolBatch {
        calls: Vec<BenchmarkToolCall>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// A validation check run after task completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchmarkCheck {
    /// Assert that a file or directory exists in the workspace.
    FileExists {
        path: String,
        #[serde(default)]
        description: String,
    },
    /// Assert that a file exists and its content contains a substring.
    FileContentContains {
        path: String,
        substring: String,
        #[serde(default)]
        description: String,
    },
    /// Assert that the trace JSONL contains at least one event of the given type.
    TraceHasEvent {
        event_type: String,
        #[serde(default)]
        description: String,
    },
    /// Run a local shell command and assert exit code 0 (and optionally stdout contains).
    CommandOracle {
        command: String,
        #[serde(default)]
        workdir: Option<String>,
        #[serde(default)]
        expected_stdout_contains: Option<String>,
        #[serde(default)]
        description: String,
    },
    /// Assert a field in report.json equals or meets a numeric minimum.
    ReportField {
        field: String,
        #[serde(default)]
        equals: Option<String>,
        #[serde(default)]
        min: Option<u64>,
        #[serde(default)]
        description: String,
    },
    /// Assert a named runtime artifact exists under the run directory.
    ArtifactExists {
        name: String,
        #[serde(default)]
        description: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub suite: String,
    pub profile: String,
    pub passed: bool,
    pub started_at: String,
    pub finished_at: String,
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub failed_tasks: usize,
    pub tasks: Vec<BenchmarkTaskReport>,
    pub evidence_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTaskReport {
    pub name: String,
    pub outcome: BenchmarkOutcome,
    pub termination_reason: String,
    pub steps: u32,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub artifacts: BenchmarkArtifacts,
    pub output: Option<String>,
    pub check_results: Vec<CheckResult>,
    pub failures: Vec<String>,
    #[serde(default)]
    pub resumed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkOutcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArtifacts {
    pub run_dir: PathBuf,
    pub trace_jsonl: PathBuf,
    pub task_state_json: PathBuf,
    pub report_json: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: String,
    pub description: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ProfileParams {
    pub task_count: usize,
    pub input_files_per_task: usize,
    pub max_steps: u32,
    pub include_failure_recovery: bool,
    pub include_cancel_resume: bool,
}

pub fn default_profile_params() -> ProfileParams {
    ProfileParams {
        task_count: 4,
        input_files_per_task: 2,
        max_steps: 20,
        include_failure_recovery: true,
        include_cancel_resume: false,
    }
}

pub fn stress_profile_params() -> ProfileParams {
    ProfileParams {
        task_count: 14,
        input_files_per_task: 3,
        max_steps: 30,
        include_failure_recovery: true,
        include_cancel_resume: true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteInfo {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
}

/// Returns a shell echo command that works in both sh and PowerShell.
pub fn shell_echo(msg: &str) -> String {
    format!("echo {}", msg)
}
