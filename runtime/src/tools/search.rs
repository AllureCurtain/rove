use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::Value;

use crate::environment::EnvironmentError;
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

/// Built-in structured workspace search (not shell `rg`, not vector RAG).
pub struct SearchCodeTool {
    policy: SearchCodePolicy,
}

/// Bounds for first-class code search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCodePolicy {
    pub timeout_ms: u64,
    pub max_matches: usize,
    pub max_output_bytes: usize,
    pub max_file_bytes: usize,
    pub max_files_scanned: usize,
}

impl Default for SearchCodePolicy {
    fn default() -> Self {
        Self {
            timeout_ms: 10_000,
            max_matches: 50,
            max_output_bytes: 64 * 1024,
            max_file_bytes: 1_048_576,
            max_files_scanned: 10_000,
        }
    }
}

#[derive(Debug, Serialize)]
struct SearchMatch {
    path: String,
    line: usize,
    column: usize,
    text: String,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    query: String,
    path: String,
    match_count: usize,
    matches: Vec<SearchMatch>,
    files_scanned: usize,
    truncated: bool,
    truncated_reason: Option<String>,
}

impl SearchCodeTool {
    pub fn new(root: PathBuf) -> Self {
        Self::with_policy(root, SearchCodePolicy::default())
    }

    pub fn with_policy(_root: PathBuf, policy: SearchCodePolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for SearchCodeTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "search_code".to_string(),
            description: "Search workspace files for a text or regex pattern. Prefer this over run_shell for structured code search. Paths stay inside the workspace; results are match-capped and timed out.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Literal text to find, or a regex when regex=true"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional workspace-relative file or directory to search (default: workspace root)"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional simple glob filter on relative paths (e.g. *.rs, src/**/*.toml)"
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Interpret query as a regular expression (default false)"
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Case-insensitive search (default false)"
                    }
                },
                "required": ["query"]
            }),
            destructive: false,
            parallel_safe: true,
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: query".to_string(),
            })?;
        if query.is_empty() {
            return Err(ToolError::InvalidInput {
                reason: "query must not be empty".to_string(),
            });
        }
        if query.contains('\0') {
            return Err(ToolError::InvalidInput {
                reason: "query may not contain NUL bytes".to_string(),
            });
        }

        let raw_path = args.get("path").and_then(|value| value.as_str());
        let glob = args.get("glob").and_then(|value| value.as_str());
        if let Some(pattern) = glob {
            if pattern.is_empty() {
                return Err(ToolError::InvalidInput {
                    reason: "glob must not be empty when provided".to_string(),
                });
            }
            if pattern.contains('\0') {
                return Err(ToolError::InvalidInput {
                    reason: "glob may not contain NUL bytes".to_string(),
                });
            }
        }

        let use_regex = args
            .get("regex")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let pattern = compile_query(query, use_regex, case_insensitive)?;
        let glob_pattern = match glob {
            Some(value) => Some(compile_glob(value)?),
            None => None,
        };

        let policy = self.policy.clone();
        let query_owned = query.to_string();
        let path_label = raw_path.unwrap_or(".").to_string();
        let search_path = raw_path.map(str::to_string);
        let services = runtime_tool_services(ctx)?;
        if !services.environment.capabilities().filesystem_read {
            return Err(map_environment_error(
                EnvironmentError::CapabilityUnavailable("filesystem_read"),
            ));
        }
        let filesystem = services.environment.filesystem();
        let result = tokio::time::timeout(
            Duration::from_millis(policy.timeout_ms),
            search_filesystem(
                filesystem,
                search_path.as_deref(),
                &pattern,
                glob_pattern.as_ref(),
                &policy,
                &query_owned,
                &path_label,
            ),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            timeout_ms: policy.timeout_ms,
        })?
        .map_err(map_environment_error)?;

        let content = serde_json::to_string(&result).map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;
        Ok(ToolOutput::text(content))
    }
}

async fn search_filesystem(
    filesystem: &dyn crate::environment::WorkspaceFileSystem,
    raw_path: Option<&str>,
    pattern: &Regex,
    glob: Option<&Regex>,
    policy: &SearchCodePolicy,
    query: &str,
    path_label: &str,
) -> Result<SearchOutput, EnvironmentError> {
    let entries = filesystem
        .list_files(raw_path, policy.max_files_scanned.saturating_add(1))
        .await?;
    let mut matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut truncated = false;
    let mut truncated_reason = None;
    for entry in entries {
        if files_scanned >= policy.max_files_scanned {
            truncated = true;
            truncated_reason = Some("max_files_scanned".to_string());
            break;
        }
        files_scanned += 1;
        if entry.byte_len > policy.max_file_bytes {
            continue;
        }
        if let Some(glob) = glob
            && !glob.is_match(&entry.relative_path)
        {
            continue;
        }
        let read = filesystem
            .read_relative_bytes(&entry.relative_path, policy.max_file_bytes)
            .await?;
        if read.truncated || read.bytes.contains(&0) {
            continue;
        }
        let content = match String::from_utf8(read.bytes) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for (index, line) in content.lines().enumerate() {
            if let Some(found) = pattern.find(line) {
                matches.push(SearchMatch {
                    path: entry.relative_path.clone(),
                    line: index + 1,
                    column: found.start() + 1,
                    text: truncate_line(line, 400),
                });
                if matches.len() >= policy.max_matches {
                    truncated = true;
                    truncated_reason = Some("max_matches".to_string());
                    break;
                }
            }
        }
        if matches.len() >= policy.max_matches
            || estimate_output_bytes(query, path_label, &matches, files_scanned, truncated)
                > policy.max_output_bytes
        {
            truncated = true;
            truncated_reason = Some(if matches.len() >= policy.max_matches {
                "max_matches".to_string()
            } else {
                "max_output_bytes".to_string()
            });
            break;
        }
    }
    if estimate_output_bytes(query, path_label, &matches, files_scanned, truncated)
        > policy.max_output_bytes
    {
        while !matches.is_empty()
            && estimate_output_bytes(query, path_label, &matches, files_scanned, true)
                > policy.max_output_bytes
        {
            matches.pop();
        }
        truncated = true;
        truncated_reason = Some("max_output_bytes".to_string());
    }
    Ok(SearchOutput {
        query: query.to_string(),
        path: path_label.to_string(),
        match_count: matches.len(),
        matches,
        files_scanned,
        truncated,
        truncated_reason,
    })
}

fn map_environment_error(error: EnvironmentError) -> ToolError {
    match error {
        EnvironmentError::Timeout(timeout_ms) => ToolError::Timeout { timeout_ms },
        EnvironmentError::Cancelled => ToolError::ExecutionFailed {
            reason: "execution cancelled".to_string(),
        },
        EnvironmentError::CapabilityUnavailable(capability) => ToolError::PermissionDenied {
            reason: format!("execution capability unavailable: {capability}"),
        },
        EnvironmentError::StaleObservation => ToolError::InvalidInput {
            reason: "observation version is stale".to_string(),
        },
        EnvironmentError::NotFound => ToolError::ExecutionFailed {
            reason: "workspace file was not found".to_string(),
        },
        EnvironmentError::Boundary => ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        },
        EnvironmentError::InvalidPath(reason) if reason.contains("escapes workspace") => {
            ToolError::PermissionDenied { reason }
        }
        EnvironmentError::InvalidPath(reason) | EnvironmentError::Host(reason) => {
            ToolError::ExecutionFailed { reason }
        }
    }
}

fn compile_query(query: &str, use_regex: bool, case_insensitive: bool) -> Result<Regex, ToolError> {
    let source = if use_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&source)
        .case_insensitive(case_insensitive)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|err| ToolError::InvalidInput {
            reason: format!("invalid search pattern: {err}"),
        })
}

fn compile_glob(pattern: &str) -> Result<Regex, ToolError> {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            c if r".+()|[]{}^$\\".contains(c) => {
                regex.push('\\');
                regex.push(c);
            }
            c => regex.push(c),
        }
    }
    regex.push('$');
    RegexBuilder::new(&regex)
        .case_insensitive(cfg!(windows))
        .build()
        .map_err(|err| ToolError::InvalidInput {
            reason: format!("invalid glob: {err}"),
        })
}

#[allow(dead_code)]
fn is_noise_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".next" | "dist" | "build" | "__pycache__" | ".rove"
    )
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let count = line.chars().count();
    if count <= max_chars {
        return line.to_string();
    }
    let mut out: String = line.chars().take(max_chars).collect();
    out.push('…');
    out
}

fn estimate_output_bytes(
    query: &str,
    path_label: &str,
    matches: &[SearchMatch],
    files_scanned: usize,
    truncated: bool,
) -> usize {
    // Conservative estimate so we cap before serde; avoids building huge strings first.
    let mut total = query.len() + path_label.len() + 128 + files_scanned.to_string().len();
    if truncated {
        total += 32;
    }
    for item in matches {
        total += item.path.len() + item.text.len() + 48;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_query_is_escaped() {
        let pattern = compile_query("foo(bar)", false, false).unwrap();
        assert!(pattern.is_match("foo(bar)"));
        assert!(!pattern.is_match("fooxbar"));
    }

    #[test]
    fn glob_star_matches_suffix() {
        let pattern = compile_glob("*.rs").unwrap();
        assert!(pattern.is_match("src/main.rs"));
        assert!(!pattern.is_match("src/main.toml"));
    }

    #[test]
    fn noise_dirs_are_recognized() {
        assert!(is_noise_dir_name("target"));
        assert!(is_noise_dir_name(".git"));
        assert!(!is_noise_dir_name("src"));
    }
}
