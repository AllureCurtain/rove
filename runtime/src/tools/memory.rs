use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::memory::durable::{MemoryScope, MemoryType, parse_frontmatter};
use crate::memory::management::{
    checked_topic_file, is_valid_memory_topic_slug, management_guard,
    recover_interrupted_index_replacement, write_memory_index_unlocked,
};
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;
const MAX_TOPIC_SLUG_BYTES: usize = 80;
const MAX_TOPIC_METADATA_BYTES: usize = 8 * 1_024;
const MAX_MEMORY_DIRECTORY_ENTRIES: usize = 4_096;
const MAX_INDEX_METADATA_BYTES: usize = 512;

/// Save a durable memory topic under the workspace state directory.
pub struct SaveMemoryTool;

impl SaveMemoryTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SaveMemoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SaveMemoryTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "save_memory".to_string(),
            description: "Save a durable memory entry that should persist across sessions."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Short topic name, such as project conventions"
                    },
                    "content": {
                        "type": "string",
                        "description": "Durable fact, preference, feedback, or reference text"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference"],
                        "description": "Memory category"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["global", "project", "session"],
                        "default": "project",
                        "description": "Scope of the memory: global (all projects), project (current project), session (current conversation)"
                    },
                    "confidence": {
                        "type": "number",
                        "minimum": 0.0,
                        "maximum": 1.0,
                        "default": 0.7,
                        "description": "Confidence score 0.0-1.0 for this memory entry"
                    }
                },
                "required": ["topic", "content", "type"]
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("memory.entry.save".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_topic = required_string(&args, "topic")?;
        let raw_content = required_string(&args, "content")?;
        let raw_type = required_string(&args, "type")?;
        let slug = normalize_topic(raw_topic)?;
        let memory_type = parse_memory_type(raw_type)?;
        let content = validate_content(raw_content)?;
        validate_promotion_policy(raw_topic, &content)?;
        let title = display_title(raw_topic);

        let scope = args
            .get("scope")
            .and_then(|v| v.as_str())
            .map(MemoryScope::parse)
            .unwrap_or_default();
        let confidence = args
            .get("confidence")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.7)
            .clamp(0.0, 1.0);

        let memory_dir = memory_dir(ctx)?;
        let now = Utc::now().to_rfc3339();
        let saved_slug = slug.clone();
        tokio::task::spawn_blocking(move || {
            save_memory_topic_sync(
                &memory_dir,
                MemoryTopicWrite {
                    slug: &saved_slug,
                    title: &title,
                    memory_type,
                    scope,
                    confidence,
                    now: &now,
                    content: &content,
                },
            )
        })
        .await
        .map_err(memory_task_failed)?
        .map_err(execution_failed)?;

        Ok(ToolOutput::text(format!("saved memory: {slug}")))
    }
}

/// Rebuild the workspace memory index from existing topic files.
pub struct UpdateMemoryIndexTool;

impl UpdateMemoryIndexTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpdateMemoryIndexTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdateMemoryIndexTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "reindex_memory".to_string(),
            description: "Rebuild the durable memory index from saved topic files.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: Some("memory.index.rebuild".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let memory_dir = memory_dir(ctx)?;
        tokio::fs::create_dir_all(&memory_dir)
            .await
            .map_err(execution_failed)?;
        rebuild_memory_index(&memory_dir).await?;

        Ok(ToolOutput::text("updated memory index"))
    }
}

/// Read a durable memory topic from the workspace state directory.
pub struct ReadMemoryTopicTool;

impl ReadMemoryTopicTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ReadMemoryTopicTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadMemoryTopicTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "read_memory".to_string(),
            description: "Read a durable memory topic by name from the memory topics directory."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Topic name to read"
                    }
                },
                "required": ["name"]
            }),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("memory.topic.read".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_name = required_string(&args, "name")?;
        let slug = normalize_topic(raw_name)?;
        let topic_path = memory_dir(ctx)?.join("topics").join(format!("{slug}.md"));

        let content = tokio::fs::read_to_string(topic_path).await.map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                ToolError::InvalidInput {
                    reason: format!("memory topic not found: {slug}"),
                }
            } else {
                execution_failed(err)
            }
        })?;

        Ok(ToolOutput::text(content))
    }
}

struct IndexEntry {
    slug: String,
    title: String,
    memory_type: String,
    scope: String,
}

fn memory_dir(ctx: &ToolContext<'_>) -> Result<PathBuf, ToolError> {
    Ok(runtime_tool_services(ctx)?.memory_paths.durable_dir.clone())
}

async fn rebuild_memory_index(memory_dir: &Path) -> Result<(), ToolError> {
    let memory_dir = memory_dir.to_path_buf();
    tokio::task::spawn_blocking(move || rebuild_memory_index_sync(&memory_dir))
        .await
        .map_err(memory_task_failed)?
        .map_err(execution_failed)
}

struct MemoryTopicWrite<'a> {
    slug: &'a str,
    title: &'a str,
    memory_type: MemoryType,
    scope: MemoryScope,
    confidence: f32,
    now: &'a str,
    content: &'a str,
}

fn save_memory_topic_sync(memory_dir: &Path, topic: MemoryTopicWrite<'_>) -> std::io::Result<()> {
    let _guard = management_guard()?;
    let topics_dir = memory_dir.join("topics");
    fs::create_dir_all(&topics_dir)?;
    recover_interrupted_index_replacement(memory_dir)?;

    let topics_metadata = fs::symlink_metadata(&topics_dir)?;
    if topics_metadata.file_type().is_symlink() || !topics_metadata.is_dir() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "memory topics location must be a regular directory",
        ));
    }
    let canonical_memory = fs::canonicalize(memory_dir)?;
    let canonical_topics = fs::canonicalize(&topics_dir)?;
    if canonical_topics.parent() != Some(canonical_memory.as_path()) {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "memory topics directory escapes the configured memory directory",
        ));
    }

    let existing_topic = checked_topic_file(memory_dir, topic.slug)?;
    let created_at = existing_topic
        .as_deref()
        .map(read_topic_metadata_prefix)
        .transpose()?
        .and_then(|existing| parse_frontmatter(&existing).get("created_at").cloned())
        .unwrap_or_else(|| topic.now.to_string());
    let topic_document = format!(
        "---\ntitle: {}\ntype: {}\nscope: {}\nsource: llm_tool\nconfidence: {:.2}\ncreated_at: {created_at}\nupdated_at: {}\n---\n\n{}\n",
        topic.title,
        topic.memory_type.as_str(),
        topic.scope.as_str(),
        topic.confidence,
        topic.now,
        topic.content,
    );

    match existing_topic {
        Some(topic_path) => {
            let mut file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(topic_path)?;
            file.write_all(topic_document.as_bytes())?;
            file.sync_all()?;
        }
        None => {
            let topic_path = canonical_topics.join(format!("{}.md", topic.slug));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(topic_path)?;
            file.write_all(topic_document.as_bytes())?;
            file.sync_all()?;
        }
    }

    rebuild_memory_index_unlocked(memory_dir)
}

fn rebuild_memory_index_sync(memory_dir: &Path) -> std::io::Result<()> {
    let _guard = management_guard()?;
    fs::create_dir_all(memory_dir)?;
    recover_interrupted_index_replacement(memory_dir)?;
    rebuild_memory_index_unlocked(memory_dir)
}

fn rebuild_memory_index_unlocked(memory_dir: &Path) -> std::io::Result<()> {
    let topics_dir = memory_dir.join("topics");
    let mut topic_paths = Vec::new();
    let entries = match fs::read_dir(&topics_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return write_memory_index_unlocked(memory_dir, b"# rove Memory\n\n".to_vec());
        }
        Err(err) => return Err(err),
    };
    let topics_metadata = fs::symlink_metadata(&topics_dir)?;
    if topics_metadata.file_type().is_symlink() || !topics_metadata.is_dir() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "memory topics location must be a regular directory",
        ));
    }
    let canonical_memory = fs::canonicalize(memory_dir)?;
    let canonical_topics = fs::canonicalize(&topics_dir)?;
    if canonical_topics.parent() != Some(canonical_memory.as_path()) {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "memory topics directory escapes the configured memory directory",
        ));
    }

    for (entry_index, entry) in entries.enumerate() {
        if entry_index >= MAX_MEMORY_DIRECTORY_ENTRIES {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "memory topics directory exceeds its supported size",
            ));
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            let file_type = entry.file_type()?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "memory topic must be a regular file",
                ));
            }
            let canonical_path = fs::canonicalize(path)?;
            if canonical_path.parent() != Some(canonical_topics.as_path()) {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "memory topic escapes the configured topics directory",
                ));
            }
            topic_paths.push(canonical_path);
        }
    }
    topic_paths.sort();

    let mut index_entries = Vec::new();
    for path in topic_paths {
        let content = read_topic_metadata_prefix(&path)?;
        let Some(slug) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !is_valid_memory_topic_slug(slug) {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "memory topic filename is unsafe",
            ));
        }
        let fm = parse_frontmatter(&content);
        let title = bounded_index_metadata(
            &fm.get("title")
                .cloned()
                .unwrap_or_else(|| slug.replace('-', " ")),
        );
        let memory_type = fm
            .get("type")
            .and_then(|value| MemoryType::parse(value))
            .unwrap_or(MemoryType::Reference)
            .as_str()
            .to_string();
        let scope = fm
            .get("scope")
            .map(|value| MemoryScope::parse(value))
            .unwrap_or_default()
            .as_str()
            .to_string();
        index_entries.push(IndexEntry {
            slug: slug.to_string(),
            title,
            memory_type,
            scope,
        });
    }

    let index = build_memory_index(index_entries);
    write_memory_index_unlocked(memory_dir, index.into_bytes())
}

fn build_memory_index(entries: Vec<IndexEntry>) -> String {
    let mut index = "# rove Memory\n\n".to_string();
    for entry in entries {
        let line = format!(
            "- [{}](topics/{}.md) \u{2014} {} {} memory\n",
            entry.title, entry.slug, entry.scope, entry.memory_type
        );
        if index.lines().count() + line.lines().count() > MAX_MEMORY_INDEX_LINES {
            break;
        }
        if index.len() + line.len() > MAX_MEMORY_INDEX_BYTES {
            break;
        }
        index.push_str(&line);
    }
    index
}

fn read_topic_metadata_prefix(path: &Path) -> std::io::Result<String> {
    let read_limit = MAX_TOPIC_METADATA_BYTES.saturating_add(4);
    let mut bytes = Vec::with_capacity(MAX_TOPIC_METADATA_BYTES);
    File::open(path)?
        .take(u64::try_from(read_limit).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MAX_TOPIC_METADATA_BYTES;
    if truncated {
        bytes.truncate(MAX_TOPIC_METADATA_BYTES);
    }
    match String::from_utf8(bytes) {
        Ok(content) => Ok(content),
        Err(error) if truncated && error.utf8_error().error_len().is_none() => {
            let valid_up_to = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid_up_to].to_vec()).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidData, "memory topic is not valid UTF-8")
            })
        }
        Err(_) => Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "memory topic is not valid UTF-8",
        )),
    }
}

fn bounded_index_metadata(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '[' | ']') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.len() <= MAX_INDEX_METADATA_BYTES {
        return sanitized;
    }
    let mut end = 0;
    for (index, character) in sanitized.char_indices() {
        let next = index + character.len_utf8();
        if next > MAX_INDEX_METADATA_BYTES {
            break;
        }
        end = next;
    }
    sanitized[..end].to_string()
}

fn required_string<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args.get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs {
            reason: format!("Missing required argument: {field}"),
        })
}

/// Normalize a topic name to a filesystem-safe slug.
///
/// Supports CJK and other Unicode characters by allowing any alphanumeric Unicode
/// character (using Unicode-aware `is_alphanumeric()` instead of ASCII-only).
/// Spaces, hyphens, and underscores become dashes. Path-traversal characters are rejected.
fn normalize_topic(raw: &str) -> Result<String, ToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return Err(unsafe_topic_error());
    }

    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in trimmed.chars() {
        if ch.is_alphanumeric() {
            // Unicode-aware: accepts CJK, Latin, Cyrillic, etc.
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            for lower_ch in ch.to_lowercase() {
                if lower_ch.is_alphanumeric() {
                    slug.push(lower_ch);
                }
            }
            pending_dash = false;
        } else if matches!(ch, ' ' | '-' | '_') {
            if !slug.is_empty() {
                pending_dash = true;
            }
        } else {
            // Disallow punctuation/symbols that are not safe for filenames
            // (but allow CJK and other alphanumeric which pass is_alphanumeric above).
            return Err(unsafe_topic_error());
        }
    }

    if slug.is_empty() || slug.len() > MAX_TOPIC_SLUG_BYTES {
        return Err(unsafe_topic_error());
    }

    Ok(slug)
}

fn validate_content(raw: &str) -> Result<String, ToolError> {
    if raw.trim().is_empty() {
        return Err(ToolError::InvalidInput {
            reason: "memory content must not be empty".to_string(),
        });
    }
    if raw.contains('\0') {
        return Err(ToolError::InvalidInput {
            reason: "memory content may not contain NUL bytes".to_string(),
        });
    }
    Ok(raw.trim_end().to_string())
}

fn validate_promotion_policy(topic: &str, content: &str) -> Result<(), ToolError> {
    let lower = format!("{} {}", topic, content).to_ascii_lowercase();
    if contains_secret_signal(&lower) {
        return Err(ToolError::InvalidInput {
            reason: "durable memory must not contain secrets, tokens, passwords, cookies, or private keys"
                .to_string(),
        });
    }
    if contains_transient_signal(&lower) {
        return Err(ToolError::InvalidInput {
            reason:
                "durable memory must describe stable long-term facts, preferences, or decisions"
                    .to_string(),
        });
    }
    Ok(())
}

fn contains_secret_signal(lower: &str) -> bool {
    [
        "api key",
        "apikey",
        "auth token",
        "bearer ",
        "cookie",
        "password",
        "private key",
        "secret",
        "token:",
        "sk-",
        "ghp_",
        "xoxb-",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_transient_signal(lower: &str) -> bool {
    [
        "/tmp/",
        "debug output",
        "log output",
        "one-time",
        "one off",
        "scratch",
        "short-term",
        "temporary",
        "transient",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn display_title(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bridge durable memory [`MemoryType::parse`] (which returns `Option`) into a
/// typed [`ToolError`] for the tool's argument validation.
fn parse_memory_type(raw: &str) -> Result<MemoryType, ToolError> {
    MemoryType::parse(raw).ok_or_else(|| ToolError::InvalidInput {
        reason: format!("memory type must be one of user, feedback, project, reference; got {raw}"),
    })
}

fn unsafe_topic_error() -> ToolError {
    ToolError::InvalidInput {
        reason: "topic must be a safe topic name using letters (including Unicode), numbers, spaces, hyphens, or underscores".to_string(),
    }
}

fn execution_failed(err: std::io::Error) -> ToolError {
    ToolError::ExecutionFailed {
        reason: err.to_string(),
    }
}

fn memory_task_failed(_error: tokio::task::JoinError) -> ToolError {
    execution_failed(std::io::Error::other(
        "memory filesystem task did not complete",
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::memory::management::delete_memory_topic_for_product_sync;

    #[test]
    fn normalized_topic_stays_valid_after_unicode_lowercase_expansion() {
        let slug = normalize_topic("\u{130}").unwrap();

        assert_eq!(slug, "i");
        assert!(is_valid_memory_topic_slug(&slug));
    }

    #[test]
    fn bounded_index_metadata_cannot_forge_a_topic_link() {
        let title = bounded_index_metadata("Visible](topics/forged.md)");

        assert!(!title.contains("](topics/"));
    }

    #[test]
    fn save_reindex_and_product_delete_share_the_mutation_guard() {
        let temp = tempfile::TempDir::new().unwrap();
        let memory_dir = temp.path().join("memory");
        let guard = management_guard().unwrap();
        let (started_tx, started_rx) = mpsc::channel();

        let save_dir = memory_dir.clone();
        let save_started = started_tx.clone();
        let save = std::thread::spawn(move || {
            save_started.send(()).unwrap();
            save_memory_topic_sync(
                &save_dir,
                MemoryTopicWrite {
                    slug: "shared-guard",
                    title: "Shared Guard",
                    memory_type: MemoryType::Project,
                    scope: MemoryScope::Project,
                    confidence: 0.7,
                    now: "2026-07-27T00:00:00Z",
                    content: "writers serialize through one guard",
                },
            )
        });

        let reindex_dir = memory_dir.clone();
        let reindex_started = started_tx.clone();
        let reindex = std::thread::spawn(move || {
            reindex_started.send(()).unwrap();
            rebuild_memory_index_sync(&reindex_dir)
        });

        let delete_dir = memory_dir;
        let delete_started = started_tx;
        let delete = std::thread::spawn(move || {
            delete_started.send(()).unwrap();
            delete_memory_topic_for_product_sync(&delete_dir, "shared-guard").map(|_| ())
        });

        for _ in 0..3 {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        let completed_while_locked =
            save.is_finished() || reindex.is_finished() || delete.is_finished();
        drop(guard);

        let save_result = save.join().unwrap();
        let reindex_result = reindex.join().unwrap();
        let delete_result = delete.join().unwrap();

        assert!(!completed_while_locked);
        save_result.unwrap();
        reindex_result.unwrap();
        delete_result.unwrap();
    }
}
