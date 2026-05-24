use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::core::workspace::Workspace;

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;
const MAX_TOPIC_SNIPPET_BYTES: usize = 1_200;

/// Read `.rove/memory/MEMORY.md` for prompt construction.
pub fn read_memory_index_sync(workspace: &Workspace) -> std::io::Result<Option<String>> {
    let path = workspace.state_dir.join("memory").join("MEMORY.md");
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let truncated = truncate_memory_index(&content);
    if truncated.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(truncated))
    }
}

/// Recall a bounded, relevant subset of durable memory for prompt construction.
pub fn recall_durable_memory_sync(
    workspace: &Workspace,
    query: &str,
    limit: usize,
) -> std::io::Result<Option<String>> {
    if limit == 0 {
        return Ok(None);
    }

    let memory_dir = workspace.state_dir.join("memory");
    let index_path = memory_dir.join("MEMORY.md");
    let content = match std::fs::read_to_string(index_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let entries = parse_memory_index(&content);
    if entries.is_empty() {
        return Ok(None);
    }

    let query_terms = tokenize(query);
    let mut scored = entries
        .into_iter()
        .map(|entry| {
            let score = relevance_score(&query_terms, &entry);
            (score, entry)
        })
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left_entry), (right_score, right_entry)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_entry.slug.cmp(&right_entry.slug))
    });

    let selected = scored.into_iter().take(limit).collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(None);
    }

    let mut recalled = "# rove Memory\n\n".to_string();
    for (_, entry) in selected {
        recalled.push_str(&entry.line);
        recalled.push('\n');
        if let Some(snippet) = read_topic_snippet(&memory_dir, &entry.slug)? {
            recalled.push_str(&format!("  snippet: {snippet}\n"));
        }
    }

    let truncated = truncate_memory_index(&recalled);
    if truncated.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(truncated))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryIndexEntry {
    slug: String,
    title: String,
    memory_type: String,
    description: String,
    line: String,
}

fn parse_memory_index(content: &str) -> Vec<MemoryIndexEntry> {
    truncate_memory_index(content)
        .lines()
        .filter_map(parse_memory_index_line)
        .collect()
}

fn parse_memory_index_line(line: &str) -> Option<MemoryIndexEntry> {
    let trimmed = line.trim();
    let after_list = trimmed.strip_prefix("- [")?;
    let (title, rest) = after_list.split_once("](topics/")?;
    let (slug, rest) = rest.split_once(".md)")?;
    let description = rest
        .trim_start()
        .trim_start_matches('-')
        .trim_start_matches('\u{2014}')
        .trim()
        .to_string();
    let memory_type = description
        .split_whitespace()
        .next()
        .unwrap_or("reference")
        .to_string();
    Some(MemoryIndexEntry {
        slug: slug.to_string(),
        title: title.to_string(),
        memory_type,
        description,
        line: trimmed.to_string(),
    })
}

fn relevance_score(query_terms: &BTreeSet<String>, entry: &MemoryIndexEntry) -> usize {
    if query_terms.is_empty() {
        return 0;
    }

    let entry_terms = tokenize(&format!(
        "{} {} {} {}",
        entry.slug, entry.title, entry.memory_type, entry.description
    ));
    query_terms
        .iter()
        .filter(|term| entry_terms.contains(*term))
        .count()
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn read_topic_snippet(memory_dir: &Path, slug: &str) -> std::io::Result<Option<String>> {
    let topic_path = memory_dir.join("topics").join(format!("{slug}.md"));
    if !safe_topic_path(memory_dir, &topic_path) {
        return Ok(None);
    }
    let content = match std::fs::read_to_string(topic_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let body = strip_frontmatter(&content);
    let snippet = truncate_to_byte_boundary(body.trim(), MAX_TOPIC_SNIPPET_BYTES)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if snippet.is_empty() {
        Ok(None)
    } else {
        Ok(Some(snippet))
    }
}

fn safe_topic_path(memory_dir: &Path, topic_path: &Path) -> bool {
    let topics_dir = normalize_lexical_path(memory_dir.join("topics"));
    normalize_lexical_path(topic_path).starts_with(topics_dir)
}

fn normalize_lexical_path(path: impl Into<PathBuf>) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.into().components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    if let Some((_, body)) = rest.split_once("\n---\n") {
        body
    } else {
        content
    }
}

fn truncate_memory_index(content: &str) -> String {
    let byte_limited = truncate_to_byte_boundary(content, MAX_MEMORY_INDEX_BYTES);
    byte_limited
        .lines()
        .take(MAX_MEMORY_INDEX_LINES)
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_to_byte_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }

    let mut end = 0;
    for (idx, ch) in content.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &content[..end]
}
