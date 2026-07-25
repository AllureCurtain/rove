use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::Value;
use walkdir::WalkDir;

use crate::boundary::resolve_workspace_read_path;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

/// Built-in structured workspace search (not shell `rg`, not vector RAG).
pub struct SearchCodeTool {
    root: PathBuf,
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

    pub fn with_policy(root: PathBuf, policy: SearchCodePolicy) -> Self {
        Self { root, policy }
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

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
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

        let root = self.root.clone();
        let policy = self.policy.clone();
        let query_owned = query.to_string();
        let path_label = raw_path.unwrap_or(".").to_string();
        let search_path = raw_path.map(str::to_string);

        let result = tokio::task::spawn_blocking(move || {
            search_workspace(
                &root,
                search_path.as_deref(),
                &pattern,
                glob_pattern.as_ref(),
                &policy,
                &query_owned,
                &path_label,
            )
        })
        .await
        .map_err(|err| ToolError::ExecutionFailed {
            reason: format!("search task failed: {err}"),
        })??;

        let content = serde_json::to_string(&result).map_err(|err| ToolError::ExecutionFailed {
            reason: err.to_string(),
        })?;
        Ok(ToolOutput::text(content))
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

fn search_workspace(
    root: &Path,
    raw_path: Option<&str>,
    pattern: &Regex,
    glob: Option<&Regex>,
    policy: &SearchCodePolicy,
    query: &str,
    path_label: &str,
) -> Result<SearchOutput, ToolError> {
    let started = Instant::now();
    let timeout = Duration::from_millis(policy.timeout_ms);
    let (canonical_root, search_root) = resolve_search_root(root, raw_path)?;

    let mut matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut truncated = false;
    let mut truncated_reason = None;

    if search_root.is_file() {
        files_scanned = 1;
        if let Some(file_matches) = search_file(
            &canonical_root,
            &search_root,
            pattern,
            glob,
            policy.max_file_bytes,
            policy.max_matches.saturating_sub(matches.len()),
        )? {
            matches.extend(file_matches);
        }
    } else {
        for entry in WalkDir::new(&search_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_descend)
        {
            if started.elapsed() > timeout {
                return Err(ToolError::Timeout {
                    timeout_ms: policy.timeout_ms,
                });
            }

            let entry = match entry {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            if files_scanned >= policy.max_files_scanned {
                truncated = true;
                truncated_reason = Some("max_files_scanned".to_string());
                break;
            }
            files_scanned += 1;

            let remaining = policy.max_matches.saturating_sub(matches.len());
            if remaining == 0 {
                truncated = true;
                truncated_reason = Some("max_matches".to_string());
                break;
            }

            match search_file(
                &canonical_root,
                entry.path(),
                pattern,
                glob,
                policy.max_file_bytes,
                remaining,
            ) {
                Ok(Some(file_matches)) => {
                    matches.extend(file_matches);
                    if matches.len() >= policy.max_matches {
                        truncated = true;
                        truncated_reason = Some("max_matches".to_string());
                        break;
                    }
                }
                Ok(None) => {}
                Err(ToolError::PermissionDenied { .. }) => continue,
                Err(err) => return Err(err),
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
                break;
            }
        }
    }

    if started.elapsed() > timeout {
        return Err(ToolError::Timeout {
            timeout_ms: policy.timeout_ms,
        });
    }

    // Final byte cap even for single-file searches.
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

fn resolve_search_root(
    root: &Path,
    raw_path: Option<&str>,
) -> Result<(PathBuf, PathBuf), ToolError> {
    let canonical_root = root.canonicalize().map_err(|err| ToolError::InvalidInput {
        reason: format!("invalid workspace root: {err}"),
    })?;

    let search_root = match raw_path {
        None | Some("") | Some(".") => canonical_root.clone(),
        Some(path) => resolve_workspace_read_path(root, path)?,
    };

    ensure_under_workspace(&canonical_root, &search_root)?;
    Ok((canonical_root, search_root))
}

fn should_descend(entry: &walkdir::DirEntry) -> bool {
    // Always keep the walk root (depth 0), even if the user pointed path at a
    // normally-pruned directory such as `target/`.
    if entry.depth() == 0 {
        return true;
    }
    !is_noise_dir_name(&entry.file_name().to_string_lossy())
}

fn is_noise_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".next" | "dist" | "build" | "__pycache__" | ".rove"
    )
}

fn search_file(
    canonical_root: &Path,
    path: &Path,
    pattern: &Regex,
    glob: Option<&Regex>,
    max_file_bytes: usize,
    max_matches: usize,
) -> Result<Option<Vec<SearchMatch>>, ToolError> {
    if max_matches == 0 {
        return Ok(None);
    }

    let canonical_path = match path.canonicalize() {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    ensure_under_workspace(canonical_root, &canonical_path)?;

    let relative = path_relative_to(canonical_root, &canonical_path)?;
    let relative_str = relative.to_string_lossy().replace('\\', "/");
    if let Some(glob) = glob
        && !glob.is_match(&relative_str)
    {
        return Ok(None);
    }

    let metadata = match std::fs::metadata(&canonical_path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if metadata.len() as usize > max_file_bytes {
        return Ok(None);
    }

    let bytes = match std::fs::read(&canonical_path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if bytes.contains(&0) {
        return Ok(None);
    }
    let content = match String::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    let mut matches = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if let Some(found) = pattern.find(line) {
            matches.push(SearchMatch {
                path: relative_str.clone(),
                line: idx + 1,
                column: found.start() + 1,
                text: truncate_line(line, 400),
            });
            if matches.len() >= max_matches {
                break;
            }
        }
    }

    if matches.is_empty() {
        Ok(None)
    } else {
        Ok(Some(matches))
    }
}

fn path_relative_to(root: &Path, target: &Path) -> Result<PathBuf, ToolError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        })?;
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            _ => {
                return Err(ToolError::PermissionDenied {
                    reason: "path escapes workspace".to_string(),
                });
            }
        }
    }
    Ok(normalized)
}

fn ensure_under_workspace(root: &Path, target: &Path) -> Result<(), ToolError> {
    if target.starts_with(root) {
        Ok(())
    } else {
        Err(ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        })
    }
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
