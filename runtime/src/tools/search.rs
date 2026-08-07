use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::Value;

use crate::environment::{EnvironmentError, Observation};
use crate::tools::coding::map_environment_error;
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

#[derive(Debug, Clone, Serialize)]
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
    observation_id: String,
    version: String,
    continuation: Option<String>,
    artifact_ref: Option<String>,
}

struct SearchScan {
    matches: Vec<SearchMatch>,
    files_scanned: usize,
    complete: bool,
    truncated_reason: Option<String>,
    version: String,
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
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 50,
                        "description": "Maximum matches returned in this page"
                    },
                    "continuation": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 512,
                        "description": "Continuation returned by an earlier unchanged search"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("workspace.search.text".to_string()),
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
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(self.policy.max_matches.min(50) as u64) as usize;
        if limit == 0 || limit > self.policy.max_matches.min(50) {
            return Err(ToolError::InvalidInput {
                reason: format!(
                    "search page limit must be between 1 and {}",
                    self.policy.max_matches.min(50)
                ),
            });
        }
        let continuation = args.get("continuation").and_then(Value::as_str);

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
        let scan = tokio::time::timeout(
            Duration::from_millis(policy.timeout_ms),
            scan_filesystem(
                filesystem,
                search_path.as_deref(),
                &pattern,
                glob_pattern.as_ref(),
                &policy,
            ),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            timeout_ms: policy.timeout_ms,
        })?
        .map_err(map_environment_error)?;

        let source = format!(
            "search:{}|{}|{}|{}|{}",
            path_label,
            query_owned,
            glob.unwrap_or(""),
            use_regex,
            case_insensitive
        );
        let start = search_continuation_start(
            services,
            continuation,
            &source,
            &scan.version,
            scan.matches.len(),
        )
        .await?;
        let end = start.saturating_add(limit).min(scan.matches.len());
        let mut page = scan.matches[start..end].to_vec();
        let mut truncated = end < scan.matches.len() || !scan.complete;
        let mut truncated_reason = if end < scan.matches.len() {
            Some("page_limit".to_string())
        } else {
            scan.truncated_reason.clone()
        };
        while !page.is_empty()
            && estimate_output_bytes(
                &query_owned,
                &path_label,
                &page,
                scan.files_scanned,
                truncated,
            ) > policy.max_output_bytes
        {
            page.pop();
            truncated = true;
            truncated_reason = Some("max_output_bytes".to_string());
        }
        let page_end = start + page.len();
        let payload = serde_json::to_vec(&page).map_err(|error| ToolError::ExecutionFailed {
            reason: error.to_string(),
        })?;
        let artifact_ref = if truncated && services.environment.capabilities().artifact_projection {
            match services.environment.artifacts() {
                Some(sink) => sink.put(&source, &payload).await.ok().flatten(),
                None => None,
            }
        } else {
            None
        };
        let has_more_page = page_end < scan.matches.len();
        let mut observation = Observation::from_bytes(
            source,
            start,
            &payload,
            scan.version.clone(),
            has_more_page,
            artifact_ref.clone(),
        );
        observation.start = start;
        observation.end = page_end;
        services
            .environment
            .observations()
            .put_with_payload(observation.clone(), payload)
            .await
            .map_err(map_environment_error)?;
        let result = SearchOutput {
            query: query_owned,
            path: path_label,
            match_count: page.len(),
            matches: page,
            files_scanned: scan.files_scanned,
            truncated,
            truncated_reason,
            observation_id: observation.id.clone(),
            version: scan.version,
            continuation: has_more_page.then(|| format!("search:{}:{page_end}", observation.id)),
            artifact_ref,
        };

        let content = serde_json::to_string(&result).map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;
        Ok(ToolOutput::text(content))
    }
}

async fn scan_filesystem(
    filesystem: &dyn crate::environment::WorkspaceFileSystem,
    raw_path: Option<&str>,
    pattern: &Regex,
    glob: Option<&Regex>,
    policy: &SearchCodePolicy,
) -> Result<SearchScan, EnvironmentError> {
    let entries = filesystem
        .list_files(raw_path, policy.max_files_scanned.saturating_add(1))
        .await?;
    let mut matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut complete = true;
    let mut truncated_reason = None;
    let mut version_material = String::new();
    for entry in entries {
        if files_scanned >= policy.max_files_scanned {
            complete = false;
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
        version_material.push_str(&entry.relative_path);
        version_material.push('|');
        version_material.push_str(&entry.byte_len.to_string());
        version_material.push('|');
        version_material.push_str(&format!(
            "sha256:{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(&read.bytes)
        ));
        version_material.push('\n');
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
                    complete = false;
                    truncated_reason = Some("max_matches".to_string());
                    break;
                }
            }
        }
        if matches.len() >= policy.max_matches {
            complete = false;
            truncated_reason = Some("max_matches".to_string());
            break;
        }
    }
    Ok(SearchScan {
        matches,
        files_scanned,
        complete,
        truncated_reason,
        version: crate::context::prompt_metadata::stable_hash(&version_material),
    })
}

async fn search_continuation_start(
    services: &crate::tools::runtime_context::RuntimeToolServices,
    continuation: Option<&str>,
    source: &str,
    version: &str,
    total: usize,
) -> Result<usize, ToolError> {
    let Some(token) = continuation else {
        return Ok(0);
    };
    let Some(rest) = token.strip_prefix("search:") else {
        return Err(ToolError::InvalidInput {
            reason: "invalid search continuation".to_string(),
        });
    };
    let Some((observation_id, raw_start)) = rest.rsplit_once(':') else {
        return Err(ToolError::InvalidInput {
            reason: "invalid search continuation".to_string(),
        });
    };
    let start = raw_start
        .parse::<usize>()
        .map_err(|_| ToolError::InvalidInput {
            reason: "invalid search continuation cursor".to_string(),
        })?;
    if start > total {
        return Err(ToolError::InvalidInput {
            reason: "search continuation exceeds the current result set".to_string(),
        });
    }
    let observation = services
        .environment
        .observations()
        .require_version(observation_id, version)
        .await
        .map_err(map_environment_error)?;
    if observation.source != source || observation.end != start || !observation.truncated {
        return Err(ToolError::InvalidInput {
            reason: "search continuation does not match the current request".to_string(),
        });
    }
    Ok(start)
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
