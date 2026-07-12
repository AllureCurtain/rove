use std::path::Path;
use std::process::Command as SysCommand;

use super::schema::{BenchmarkArtifacts, BenchmarkCheck, CheckResult};

pub async fn run_check(
    check: &BenchmarkCheck,
    workspace_root: &Path,
    artifacts: &BenchmarkArtifacts,
) -> CheckResult {
    match check {
        BenchmarkCheck::FileExists { path, description } => {
            let full = workspace_root.join(path);
            let exists = tokio::fs::try_exists(&full).await.unwrap_or(false);
            CheckResult {
                kind: "file_exists".to_string(),
                description: description.clone(),
                passed: exists,
                detail: if exists {
                    format!("{path} exists")
                } else {
                    format!("{path} does not exist")
                },
            }
        }
        BenchmarkCheck::FileContentContains {
            path,
            substring,
            description,
        } => {
            let full = workspace_root.join(path);
            match tokio::fs::read_to_string(&full).await {
                Ok(content) => {
                    let contains = content.contains(substring);
                    CheckResult {
                        kind: "file_content_contains".to_string(),
                        description: description.clone(),
                        passed: contains,
                        detail: if contains {
                            format!("{path} contains expected substring")
                        } else {
                            format!("{path} does not contain {substring:?}")
                        },
                    }
                }
                Err(e) => CheckResult {
                    kind: "file_content_contains".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to read {path}: {e}"),
                },
            }
        }
        BenchmarkCheck::TraceHasEvent {
            event_type,
            description,
        } => match tokio::fs::read_to_string(&artifacts.trace_jsonl).await {
            Ok(content) => {
                let needle = format!("\"type\":\"{event_type}\"");
                let found = content.lines().any(|line| line.contains(&needle));
                CheckResult {
                    kind: "trace_has_event".to_string(),
                    description: description.clone(),
                    passed: found,
                    detail: if found {
                        format!("trace contains {event_type} event")
                    } else {
                        format!("trace does not contain {event_type} event")
                    },
                }
            }
            Err(e) => CheckResult {
                kind: "trace_has_event".to_string(),
                description: description.clone(),
                passed: false,
                detail: format!("failed to read trace: {e}"),
            },
        },
        BenchmarkCheck::CommandOracle {
            command,
            workdir,
            expected_stdout_contains,
            description,
        } => {
            let exec_dir = if let Some(wd) = workdir {
                workspace_root.join(wd)
            } else {
                workspace_root.to_path_buf()
            };
            #[cfg(target_os = "windows")]
            let output = SysCommand::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", command])
                .current_dir(&exec_dir)
                .output();
            #[cfg(not(target_os = "windows"))]
            let output = SysCommand::new("sh")
                .args(["-lc", command])
                .current_dir(&exec_dir)
                .output();

            match output {
                Ok(result) => {
                    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
                    let exit_ok = result.status.success();
                    let stdout_ok = match expected_stdout_contains {
                        Some(expected) => stdout.contains(expected),
                        None => true,
                    };
                    let passed = exit_ok && stdout_ok;
                    let mut detail_parts = Vec::new();
                    if !exit_ok {
                        detail_parts.push(format!("exit code {:?}", result.status.code()));
                    }
                    if !stdout_ok {
                        detail_parts.push("stdout did not contain expected text".to_string());
                    }
                    if !stderr.is_empty() {
                        detail_parts.push(format!("stderr: {}", stderr.trim()));
                    }
                    if passed {
                        detail_parts.push(format!("command succeeded: {}", stdout.trim()));
                    }
                    CheckResult {
                        kind: "command_oracle".to_string(),
                        description: description.clone(),
                        passed,
                        detail: detail_parts.join("; "),
                    }
                }
                Err(e) => CheckResult {
                    kind: "command_oracle".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to execute command: {e}"),
                },
            }
        }
        BenchmarkCheck::ReportField {
            field,
            equals,
            min,
            description,
        } => match tokio::fs::read_to_string(&artifacts.report_json).await {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(report) => {
                    let value = report.get(field);
                    let (passed, detail) = match (value, equals, min) {
                        (Some(v), Some(expected), _) => {
                            let actual = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string().trim_matches('"').to_string(),
                            };
                            let ok = actual.eq_ignore_ascii_case(expected)
                                || actual == *expected
                                || v.to_string().contains(expected);
                            (
                                ok,
                                if ok {
                                    format!("report.{field} matches {expected}")
                                } else {
                                    format!("report.{field}={actual:?}, expected {expected:?}")
                                },
                            )
                        }
                        (Some(v), None, Some(min_v)) => {
                            let actual = v.as_u64().or_else(|| v.as_i64().map(|n| n as u64));
                            match actual {
                                Some(n) if n >= *min_v => {
                                    (true, format!("report.{field}={n} >= {min_v}"))
                                }
                                Some(n) => (false, format!("report.{field}={n} < {min_v}")),
                                None => (false, format!("report.{field} is not numeric: {v}")),
                            }
                        }
                        (Some(_), None, None) => (true, format!("report.{field} exists")),
                        (None, _, _) => (false, format!("report.{field} missing")),
                    };
                    CheckResult {
                        kind: "report_field".to_string(),
                        description: description.clone(),
                        passed,
                        detail,
                    }
                }
                Err(e) => CheckResult {
                    kind: "report_field".to_string(),
                    description: description.clone(),
                    passed: false,
                    detail: format!("failed to parse report.json: {e}"),
                },
            },
            Err(e) => CheckResult {
                kind: "report_field".to_string(),
                description: description.clone(),
                passed: false,
                detail: format!("failed to read report.json: {e}"),
            },
        },
        BenchmarkCheck::ArtifactExists { name, description } => {
            let path = artifacts.run_dir.join(name);
            let exists = tokio::fs::try_exists(&path).await.unwrap_or(false);
            CheckResult {
                kind: "artifact_exists".to_string(),
                description: description.clone(),
                passed: exists,
                detail: if exists {
                    format!("artifact {name} exists")
                } else {
                    format!("artifact {name} missing")
                },
            }
        }
    }
}
