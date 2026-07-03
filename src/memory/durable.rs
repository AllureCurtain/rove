use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::workspace::Workspace;

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;
const MAX_TOPIC_SNIPPET_BYTES: usize = 1_200;
/// Minimum character length for Latin word tokens.
const MIN_LATIN_TOKEN_LEN: usize = 3;
/// CJK bigram length (characters).
const CJK_BIGRAM_WINDOW: usize = 2;

// ── Public types ──────────────────────────────────────────────────────

/// Parsed memory topic metadata from frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicMetadata {
    pub title: String,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub source: String,
    pub confidence: f32,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Memory type categories.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" => Some(Self::User),
            "feedback" => Some(Self::Feedback),
            "project" => Some(Self::Project),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// Memory scope controls how broadly a memory applies.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Applies to all projects for this user.
    Global,
    /// Applies to the current project/workspace only.
    #[default]
    Project,
    /// Applies to a specific session/conversation only.
    Session,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Session => "session",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "global" => Self::Global,
            "session" => Self::Session,
            _ => Self::Project,
        }
    }
}

/// A scored memory recall result.
#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub slug: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub confidence: f32,
    pub score: f64,
    pub snippet: Option<String>,
}

/// Options for a recall query.
#[derive(Debug, Clone, Default)]
pub struct RecallOptions {
    pub type_filter: Option<MemoryType>,
    pub limit: usize,
}

// ── Public API ────────────────────────────────────────────────────────

/// Read `.rove/memory/MEMORY.md` for prompt construction.
pub fn read_memory_index_sync(workspace: &Workspace) -> std::io::Result<Option<String>> {
    read_memory_index_from_dir_sync(&workspace.state_dir.join("memory"))
}

pub fn read_memory_index_from_dir_sync(memory_dir: &Path) -> std::io::Result<Option<String>> {
    let path = memory_dir.join("MEMORY.md");
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
///
/// This is the main prompt-construction entry point. It returns all memory types
/// (no type filtering); use [`recall_with_scores_sync`] with [`RecallOptions`]
/// when you need `type_filter` (e.g. a debug endpoint that only wants `decision`).
pub fn recall_durable_memory_sync(
    workspace: &Workspace,
    query: &str,
    limit: usize,
) -> std::io::Result<Option<String>> {
    recall_durable_memory_from_dir_sync(&workspace.state_dir.join("memory"), query, limit)
}

pub fn recall_durable_memory_from_dir_sync(
    memory_dir: &Path,
    query: &str,
    limit: usize,
) -> std::io::Result<Option<String>> {
    let hits = recall_with_scores_from_dir_sync(
        memory_dir,
        query,
        RecallOptions {
            limit,
            type_filter: None,
        },
    )?;
    if hits.is_empty() {
        return Ok(None);
    }
    let mut recalled = String::from("# rove Memory\n\n");
    for hit in &hits {
        recalled.push_str(&format!(
            "- [{}](topics/{}.md) — {} {} memory (confidence: {:.0}%)\n",
            hit.title,
            hit.slug,
            hit.scope.as_str(),
            hit.memory_type.as_str(),
            hit.confidence * 100.0,
        ));
        if let Some(snippet) = &hit.snippet {
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

/// Recall with detailed scores — used by debug endpoints and testing.
pub fn recall_with_scores_sync(
    workspace: &Workspace,
    query: &str,
    opts: RecallOptions,
) -> std::io::Result<Vec<RecallHit>> {
    recall_with_scores_from_dir_sync(&workspace.state_dir.join("memory"), query, opts)
}

pub fn recall_with_scores_from_dir_sync(
    memory_dir: &Path,
    query: &str,
    opts: RecallOptions,
) -> std::io::Result<Vec<RecallHit>> {
    let limit = if opts.limit == 0 { 8 } else { opts.limit };
    let index_path = memory_dir.join("MEMORY.md");
    let content = match std::fs::read_to_string(index_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let entries = parse_memory_index_with_metadata(memory_dir, &content);
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // Build corpus and compute IDF.
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-tokenize all entries (title, slug, type, scope, description, body) for IDF.
    struct EntryTokens {
        title: BTreeSet<String>,
        slug: BTreeSet<String>,
        type_scope: BTreeSet<String>,
        description: BTreeSet<String>,
        body: BTreeSet<String>,
        entry: IndexEntry,
    }

    let mut tokenized: Vec<EntryTokens> = Vec::new();
    for entry in &entries {
        // Read snippet body for scoring.
        let body_text = read_topic_body_text(memory_dir, &entry.slug).unwrap_or_default();
        let et = EntryTokens {
            title: tokenize(&entry.title),
            slug: tokenize(&entry.slug.replace('-', " ")),
            type_scope: {
                let mut s = BTreeSet::new();
                // `memory_type` (user/feedback/project/reference) is searchable text.
                // `scope` is a structural classification label, not content —
                // including it would let a query containing "project" match every
                // legacy topic that defaulted to scope=Project, over-recalling.
                // `source` is free-form provenance text and is searchable.
                s.insert(entry.metadata.memory_type.as_str().to_string());
                if !entry.metadata.source.is_empty() {
                    s.extend(tokenize(&entry.metadata.source));
                }
                s
            },
            description: tokenize(&entry.description),
            body: tokenize(&body_text),
            entry: entry.clone(),
        };
        tokenized.push(et);
    }

    // Compute IDF: smoothed log(N / df) for each term.
    let n = tokenized.len() as f64;
    let mut df: HashMap<&str, usize> = HashMap::new();
    for et in &tokenized {
        let mut all_terms = BTreeSet::new();
        all_terms.extend(&et.title);
        all_terms.extend(&et.slug);
        all_terms.extend(&et.type_scope);
        all_terms.extend(&et.description);
        all_terms.extend(&et.body);
        for term in all_terms {
            *df.entry(term.as_str()).or_insert(0) += 1;
        }
    }

    let idf = |term: &str| -> f64 {
        let doc_freq = *df.get(term).unwrap_or(&0);
        if doc_freq == 0 {
            return 0.0;
        }
        ((n + 1.0) / (doc_freq as f64 + 1.0)).ln() + 1.0
    };

    // Score each entry.
    let mut scored: Vec<(f64, IndexEntry, Option<String>)> = Vec::new();
    for et in &tokenized {
        // Apply type filter (hard filter: skip non-matching types entirely).
        if let Some(tf) = opts.type_filter
            && et.entry.metadata.memory_type != tf
        {
            continue;
        }

        let mut score = 0.0f64;

        // Title matches: weight 3.0
        for term in &query_terms {
            if et.title.contains(term) {
                score += 3.0 * idf(term);
            }
        }
        // Slug matches: weight 2.0
        for term in &query_terms {
            if et.slug.contains(term) {
                score += 2.0 * idf(term);
            }
        }
        // Type/scope/source matches: weight 2.0
        for term in &query_terms {
            if et.type_scope.contains(term) {
                score += 2.0 * idf(term);
            }
        }
        // Description matches: weight 1.5
        for term in &query_terms {
            if et.description.contains(term) {
                score += 1.5 * idf(term);
            }
        }
        // Body matches: weight 1.0
        for term in &query_terms {
            if et.body.contains(term) {
                score += 1.0 * idf(term);
            }
        }

        // Exact phrase bonus (full query appears in title).
        if !query.trim().is_empty() {
            let title_lower = et.entry.title.to_lowercase();
            let query_lower = query.trim().to_lowercase();
            if title_lower.contains(&query_lower) {
                score += 10.0;
            }
        }

        // Confidence weighting (0.0–1.0 multiplier).
        score *= et.entry.metadata.confidence.clamp(0.0, 1.0) as f64;

        // Recency bonus: updated_at within last 7 days gets a small boost.
        if let Some(updated_str) = &et.entry.metadata.updated_at
            && let Ok(updated) = chrono::DateTime::parse_from_rfc3339(updated_str)
        {
            let age_hours = (chrono::Utc::now() - updated.with_timezone(&chrono::Utc)).num_hours();
            if (0..24 * 7).contains(&age_hours) {
                let recency_factor = 1.0 + 0.1 * (1.0 - age_hours as f64 / (24.0 * 7.0));
                score *= recency_factor;
            }
        }

        if score > 0.0 {
            let snippet = read_topic_snippet(memory_dir, &et.entry.slug).unwrap_or(None);
            scored.push((score, et.entry.clone(), snippet));
        }
    }

    scored.sort_by(|(a_score, a_entry, _), (b_score, b_entry, _)| {
        b_score
            .partial_cmp(a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a_entry.slug.cmp(&b_entry.slug))
    });

    let hits: Vec<RecallHit> = scored
        .into_iter()
        .take(limit)
        .map(|(score, entry, snippet)| RecallHit {
            slug: entry.slug,
            title: entry.title,
            memory_type: entry.metadata.memory_type,
            scope: entry.metadata.scope,
            confidence: entry.metadata.confidence,
            score,
            snippet,
        })
        .collect();
    Ok(hits)
}

/// List all topics in the memory index with their metadata.
pub fn list_memory_topics_sync(workspace: &Workspace) -> std::io::Result<Vec<MemoryTopicInfo>> {
    list_memory_topics_from_dir_sync(&workspace.state_dir.join("memory"))
}

pub fn list_memory_topics_from_dir_sync(
    memory_dir: &Path,
) -> std::io::Result<Vec<MemoryTopicInfo>> {
    let index_path = memory_dir.join("MEMORY.md");
    let content = match std::fs::read_to_string(index_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let entries = parse_memory_index_with_metadata(memory_dir, &content);
    Ok(entries
        .into_iter()
        .map(|e| MemoryTopicInfo {
            slug: e.slug,
            title: e.title,
            memory_type: e.metadata.memory_type,
            scope: e.metadata.scope,
            source: e.metadata.source,
            confidence: e.metadata.confidence,
            created_at: e.metadata.created_at,
            updated_at: e.metadata.updated_at,
            description: e.description,
        })
        .collect())
}

/// Read a topic file's full content (including frontmatter).
pub fn read_topic_file_sync(memory_dir: &Path, slug: &str) -> std::io::Result<Option<String>> {
    let topic_path = memory_dir.join("topics").join(format!("{slug}.md"));
    if !safe_topic_path(memory_dir, &topic_path) {
        return Ok(None);
    }
    match std::fs::read_to_string(topic_path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryTopicInfo {
    pub slug: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub source: String,
    pub confidence: f32,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub description: String,
}

// ── Internal types ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct IndexEntry {
    slug: String,
    title: String,
    description: String,
    metadata: TopicMetadata,
}

// ── Parsing ───────────────────────────────────────────────────────────

fn parse_memory_index_with_metadata(memory_dir: &Path, content: &str) -> Vec<IndexEntry> {
    truncate_memory_index(content)
        .lines()
        .filter_map(|line| parse_memory_index_line(memory_dir, line))
        .collect()
}

fn parse_memory_index_line(memory_dir: &Path, line: &str) -> Option<IndexEntry> {
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

    // Determine type from description (first word), for backward compat with
    // index lines written before topic metadata existed.
    let type_from_line = description
        .split_whitespace()
        .next()
        .and_then(MemoryType::parse)
        .unwrap_or(MemoryType::Reference);

    // Try to read full metadata from the topic file's frontmatter.
    let metadata = read_topic_metadata(memory_dir, slug, title, type_from_line);

    Some(IndexEntry {
        slug: slug.to_string(),
        title: title.to_string(),
        description,
        metadata,
    })
}

fn read_topic_metadata(
    memory_dir: &Path,
    slug: &str,
    fallback_title: &str,
    fallback_type: MemoryType,
) -> TopicMetadata {
    let topic_path = memory_dir.join("topics").join(format!("{slug}.md"));
    let content = match std::fs::read_to_string(&topic_path) {
        Ok(c) => c,
        Err(_) => {
            return TopicMetadata {
                title: fallback_title.to_string(),
                memory_type: fallback_type,
                scope: MemoryScope::default(),
                source: String::new(),
                confidence: 0.7,
                created_at: None,
                updated_at: None,
            };
        }
    };

    let fm = parse_frontmatter(&content);
    let title = fm
        .get("title")
        .cloned()
        .unwrap_or_else(|| fallback_title.to_string());
    let memory_type = fm
        .get("type")
        .and_then(|v| MemoryType::parse(v))
        .unwrap_or(fallback_type);
    let scope = fm
        .get("scope")
        .map(|v| MemoryScope::parse(v))
        .unwrap_or_default();
    let source = fm.get("source").cloned().unwrap_or_default();
    let confidence = fm
        .get("confidence")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.7);
    let created_at = fm.get("created_at").cloned();
    let updated_at = fm.get("updated_at").cloned();

    TopicMetadata {
        title,
        memory_type,
        scope,
        source,
        confidence: confidence.clamp(0.0, 1.0),
        created_at,
        updated_at,
    }
}

/// Parse YAML frontmatter into a key-value map. Handles simple `key: value` pairs.
pub(crate) fn parse_frontmatter(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(rest) = content.strip_prefix("---\n") else {
        return map;
    };
    for line in rest.lines() {
        if line == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

// ── Tokenization (CJK-aware) ──────────────────────────────────────────

/// Tokenize text into searchable terms.
///
/// - For CJK characters: generate overlapping bigrams (2-char windows), which
///   provides robust sub-word matching without a dictionary. Individual
///   characters are also added as unigrams (each CJK char is meaningful).
/// - For Latin/Cyrillic/other alphabetic scripts: split on non-alphanumeric
///   boundaries, lowercase, filter by minimum length.
/// - Numbers are kept as-is (regardless of length).
fn tokenize(text: &str) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    if text.is_empty() {
        return terms;
    }

    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];

        if is_cjk_char(ch) {
            // Collect consecutive CJK characters into a run, then produce bigrams.
            let run_start = i;
            while i < chars.len() && is_cjk_char(chars[i]) {
                i += 1;
            }
            let run: String = chars[run_start..i].iter().collect();
            // Add unigrams (single CJK characters are meaningful).
            for c in run.chars() {
                terms.insert(c.to_string());
            }
            // Add bigrams for better phrase matching.
            let run_chars: Vec<char> = run.chars().collect();
            if run_chars.len() >= CJK_BIGRAM_WINDOW {
                for window in run_chars.windows(CJK_BIGRAM_WINDOW) {
                    let bigram: String = window.iter().collect();
                    terms.insert(bigram);
                }
            }
            // Also add the entire CJK run as a term (trigram+).
            if run.chars().count() >= 3 {
                terms.insert(run);
            }
        } else if ch.is_alphanumeric() {
            // Collect a run of alphanumeric (Latin word or number).
            let run_start = i;
            while i < chars.len() && chars[i].is_alphanumeric() && !is_cjk_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[run_start..i].iter().collect();
            if word.len() >= MIN_LATIN_TOKEN_LEN || word.chars().all(|c| c.is_ascii_digit()) {
                terms.insert(word);
            }
        } else {
            i += 1;
        }
    }

    terms
}

/// Check if a character is a CJK Unified Ideograph or related script
/// (Hiragana, Katakana, Hangul). Used by [`tokenize`] for CJK-aware splitting.
fn is_cjk_char(ch: char) -> bool {
    // CJK Unified Ideographs (common)
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
    // CJK Unified Ideographs Extension A
    || ('\u{3400}'..='\u{4dbf}').contains(&ch)
    // CJK Compatibility Ideographs
    || ('\u{f900}'..='\u{faff}').contains(&ch)
    // Hiragana
    || ('\u{3040}'..='\u{309f}').contains(&ch)
    // Katakana
    || ('\u{30a0}'..='\u{30ff}').contains(&ch)
    // Hangul Syllables
    || ('\u{ac00}'..='\u{d7af}').contains(&ch)
    // Hangul Jamo
    || ('\u{1100}'..='\u{11ff}').contains(&ch)
}

// ── File reading helpers ──────────────────────────────────────────────

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

fn read_topic_body_text(memory_dir: &Path, slug: &str) -> std::io::Result<String> {
    let topic_path = memory_dir.join("topics").join(format!("{slug}.md"));
    if !safe_topic_path(memory_dir, &topic_path) {
        return Ok(String::new());
    }
    let content = match std::fs::read_to_string(topic_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err),
    };
    Ok(strip_frontmatter(&content).to_string())
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

pub(crate) fn strip_frontmatter(content: &str) -> &str {
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

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_memory_dir() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
        (tmp, memory_dir)
    }

    fn write_topic(memory_dir: &Path, slug: &str, title: &str, mtype: &str, content: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let doc = format!(
            "---\ntitle: {title}\ntype: {mtype}\ncreated_at: {now}\nupdated_at: {now}\nscope: project\nsource: test\nconfidence: 0.8\n---\n\n{content}\n"
        );
        std::fs::write(memory_dir.join("topics").join(format!("{slug}.md")), doc).unwrap();
    }

    fn write_topic_with_confidence(
        memory_dir: &Path,
        slug: &str,
        title: &str,
        mtype: &str,
        confidence: f32,
        content: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let doc = format!(
            "---\ntitle: {title}\ntype: {mtype}\ncreated_at: {now}\nupdated_at: {now}\nscope: project\nsource: test\nconfidence: {confidence}\n---\n\n{content}\n"
        );
        std::fs::write(memory_dir.join("topics").join(format!("{slug}.md")), doc).unwrap();
    }

    fn write_index(memory_dir: &Path, entries: &[(&str, &str, &str)]) {
        let mut index = String::from("# rove Memory\n\n");
        for (slug, title, mtype) in entries {
            index.push_str(&format!("- [{title}](topics/{slug}.md) — {mtype} memory\n"));
        }
        std::fs::write(memory_dir.join("MEMORY.md"), index).unwrap();
    }

    #[test]
    fn cjk_tokenization_produces_unigrams_and_bigrams() {
        let terms = tokenize("数据库配置");
        // Should contain individual characters (unigrams)
        assert!(terms.contains("数"), "should contain unigram 数");
        assert!(terms.contains("据"), "should contain unigram 据");
        // Should contain bigrams
        assert!(terms.contains("数据"), "should contain bigram 数据");
        assert!(terms.contains("据库"), "should contain bigram 据库");
        assert!(terms.contains("库配"), "should contain bigram 库配");
        assert!(terms.contains("配置"), "should contain bigram 配置");
    }

    #[test]
    fn latin_tokenization_lowercases_and_filters_short_words() {
        let terms = tokenize("Database Configuration Setup");
        assert!(terms.contains("database"));
        assert!(terms.contains("configuration"));
        assert!(terms.contains("setup"));
        // Short words (<3 chars) should be filtered
        assert!(!terms.contains("is"));
        assert!(!terms.contains("a"));
    }

    #[test]
    fn mixed_cjk_and_latin_tokenizes_both() {
        let terms = tokenize("数据库 database 配置 config");
        assert!(terms.contains("数据"));
        assert!(terms.contains("database"));
        assert!(terms.contains("配置"));
        assert!(terms.contains("config"));
    }

    #[test]
    fn cjk_recall_finds_relevant_entry() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(
            &dir,
            "db-config",
            "数据库配置",
            "project",
            "MySQL 数据库连接字符串配置，使用环境变量 DATABASE_URL",
        );
        write_index(&dir, &[("db-config", "数据库配置", "project")]);

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "数据库",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(!hits.is_empty(), "CJK query should find the entry");
        assert_eq!(hits[0].slug, "db-config");
    }

    #[test]
    fn japanese_recall_works() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(
            &dir,
            "api-rules",
            "API設計ルール",
            "reference",
            "APIの設計ルールとガイドライン、REST原則に従う",
        );
        write_index(&dir, &[("api-rules", "API設計ルール", "reference")]);

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "設計ルール",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(!hits.is_empty(), "Japanese query should find the entry");
    }

    #[test]
    fn korean_recall_works() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(
            &dir,
            "coding-style",
            "코딩 스타일",
            "project",
            "코딩 스타일 가이드라인, 들여쓰기는 2칸 사용",
        );
        write_index(&dir, &[("coding-style", "코딩 스타일", "project")]);

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "코딩 스타일",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(!hits.is_empty(), "Korean query should find the entry");
    }

    #[test]
    fn rare_terms_rank_higher_than_common_terms() {
        let (_tmp, dir) = setup_memory_dir();
        // "model" is common across many entries; "circuit breaker" is rare
        write_topic(
            &dir,
            "routing",
            "Model Routing",
            "reference",
            "Model client routing with fallback support",
        );
        write_topic(
            &dir,
            "health",
            "Model Health Store",
            "reference",
            "Circuit breaker for model health tracking",
        );
        write_topic(
            &dir,
            "factory",
            "Model Factory",
            "reference",
            "Build model clients from configuration",
        );
        write_topic(
            &dir,
            "circuit-breaker",
            "Circuit Breaker Design",
            "reference",
            "Circuit breaker pattern implementation details",
        );
        write_index(
            &dir,
            &[
                ("routing", "Model Routing", "reference"),
                ("health", "Model Health Store", "reference"),
                ("factory", "Model Factory", "reference"),
                ("circuit-breaker", "Circuit Breaker Design", "reference"),
            ],
        );

        // Query for "circuit breaker" should rank the circuit breaker entry highest
        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "circuit breaker",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(
            hits[0].slug,
            "circuit-breaker",
            "circuit breaker entry should be ranked first, got {:?}",
            hits.iter().map(|h| (&h.slug, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn type_filter_excludes_other_types() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(
            &dir,
            "user-pref",
            "User Preferences",
            "user",
            "User prefers dark mode and concise responses",
        );
        write_topic(
            &dir,
            "proj-rules",
            "Project Rules",
            "project",
            "Run cargo fmt before committing",
        );
        write_index(
            &dir,
            &[
                ("user-pref", "User Preferences", "user"),
                ("proj-rules", "Project Rules", "project"),
            ],
        );

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "preferences rules",
            RecallOptions {
                limit: 8,
                type_filter: Some(MemoryType::User),
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory_type, MemoryType::User);
        assert_eq!(hits[0].slug, "user-pref");
    }

    #[test]
    fn confidence_weighting_affects_scoring() {
        let (_tmp, dir) = setup_memory_dir();
        // Two similar entries, one high confidence one low
        write_topic_with_confidence(
            &dir,
            "high-conf",
            "Important Fact",
            "reference",
            0.95,
            "The database uses PostgreSQL",
        );
        write_topic_with_confidence(
            &dir,
            "low-conf",
            "Important Fact",
            "reference",
            0.3,
            "The database uses PostgreSQL maybe",
        );
        write_index(
            &dir,
            &[
                ("high-conf", "Important Fact", "reference"),
                ("low-conf", "Important Fact", "reference"),
            ],
        );

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "database postgresql",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(hits.len() >= 2);
        assert!(
            hits[0].score > hits[1].score,
            "Higher confidence entry should score higher: {} vs {}",
            hits[0].score,
            hits[1].score
        );
        assert_eq!(hits[0].slug, "high-conf");
    }

    #[test]
    fn backward_compat_with_old_format_topics() {
        // Old format: only title, type, created_at, updated_at; no scope/source/confidence
        let (_tmp, dir) = setup_memory_dir();
        let now = chrono::Utc::now().to_rfc3339();
        let old_doc = format!(
            "---\ntitle: Old Topic\ntype: project\ncreated_at: {now}\nupdated_at: {now}\n---\n\nOld format content here\n"
        );
        std::fs::write(dir.join("topics/old-topic.md"), old_doc).unwrap();
        write_index(&dir, &[("old-topic", "Old Topic", "project")]);

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "old format content",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1, "Old format entries should still be findable");
        assert_eq!(hits[0].slug, "old-topic");
        assert_eq!(hits[0].scope, MemoryScope::Project); // default
        assert!((hits[0].confidence - 0.7).abs() < 0.01); // default confidence
    }

    #[test]
    fn recall_for_prompt_injection_returns_formatted_string() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(&dir, "test-entry", "Test Entry", "project", "Test content");
        write_index(&dir, &[("test-entry", "Test Entry", "project")]);

        let result = recall_durable_memory_from_dir_sync(&dir, "test content", 8).unwrap();
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("# rove Memory"));
        assert!(text.contains("Test Entry"));
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(&dir, "test", "Test", "project", "content");
        write_index(&dir, &[("test", "Test", "project")]);

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn list_topics_returns_all_entries() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(&dir, "a", "Topic A", "project", "content a");
        write_topic(&dir, "b", "Topic B", "user", "content b");
        write_index(
            &dir,
            &[("a", "Topic A", "project"), ("b", "Topic B", "user")],
        );

        let topics = list_memory_topics_from_dir_sync(&dir).unwrap();
        assert_eq!(topics.len(), 2);
    }

    #[test]
    fn title_match_scores_higher_than_body_match() {
        let (_tmp, dir) = setup_memory_dir();
        write_topic(
            &dir,
            "rust-rules",
            "Rust Coding Rules",
            "project",
            "Use proper error handling in Rust code",
        );
        write_topic(
            &dir,
            "error-handling",
            "Error Handling Guide",
            "reference",
            "Rust error handling patterns with thiserror and anyhow",
        );
        write_index(
            &dir,
            &[
                ("rust-rules", "Rust Coding Rules", "project"),
                ("error-handling", "Error Handling Guide", "reference"),
            ],
        );

        let hits = recall_with_scores_from_dir_sync(
            &dir,
            "error handling",
            RecallOptions {
                limit: 8,
                type_filter: None,
            },
        )
        .unwrap();
        assert_eq!(
            hits[0].slug,
            "error-handling",
            "Title match for 'error handling' should rank higher, got {:?}",
            hits.iter().map(|h| &h.slug).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tokenize_handles_empty_string() {
        let terms = tokenize("");
        assert!(terms.is_empty());
    }

    #[test]
    fn tokenize_handles_numbers() {
        let terms = tokenize("error 404 not found 2024");
        assert!(terms.contains("404"));
        assert!(terms.contains("2024"));
        assert!(terms.contains("error"));
        assert!(terms.contains("found"));
    }
}
