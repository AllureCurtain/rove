use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::memory::durable::{MemoryScope, MemoryType, parse_frontmatter};
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::ToolDescriptor as ToolSchema;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput};

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;
const MAX_TOPIC_SLUG_BYTES: usize = 80;

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
    fn schema(&self) -> ToolSchema {
        ToolSchema {
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
        let topics_dir = memory_dir.join("topics");
        tokio::fs::create_dir_all(&topics_dir)
            .await
            .map_err(execution_failed)?;

        let topic_path = topics_dir.join(format!("{slug}.md"));
        let now = Utc::now().to_rfc3339();
        let created_at = match tokio::fs::read_to_string(&topic_path).await {
            Ok(existing) => parse_frontmatter(&existing)
                .get("created_at")
                .cloned()
                .unwrap_or_else(|| now.clone()),
            Err(err) if err.kind() == ErrorKind::NotFound => now.clone(),
            Err(err) => return Err(execution_failed(err)),
        };

        let topic_document = format!(
            "---\ntitle: {title}\ntype: {}\nscope: {}\nsource: llm_tool\nconfidence: {:.2}\ncreated_at: {created_at}\nupdated_at: {now}\n---\n\n{content}\n",
            memory_type.as_str(),
            scope.as_str(),
            confidence,
        );
        tokio::fs::write(&topic_path, topic_document)
            .await
            .map_err(execution_failed)?;

        update_memory_index(&memory_dir).await?;

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
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "update_memory_index".to_string(),
            description: "Rebuild the durable memory index from saved topic files.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            destructive: false,
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let memory_dir = memory_dir(ctx)?;
        tokio::fs::create_dir_all(&memory_dir)
            .await
            .map_err(execution_failed)?;
        update_memory_index(&memory_dir).await?;

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
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_memory_topic".to_string(),
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

async fn update_memory_index(memory_dir: &Path) -> Result<(), ToolError> {
    let topics_dir = memory_dir.join("topics");
    let mut topic_paths = Vec::new();
    let mut entries = match tokio::fs::read_dir(&topics_dir).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            tokio::fs::write(memory_dir.join("MEMORY.md"), "# rove Memory\n\n")
                .await
                .map_err(execution_failed)?;
            return Ok(());
        }
        Err(err) => return Err(execution_failed(err)),
    };

    while let Some(entry) = entries.next_entry().await.map_err(execution_failed)? {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            topic_paths.push(path);
        }
    }
    topic_paths.sort();

    let mut index_entries = Vec::new();
    for path in topic_paths {
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(execution_failed)?;
        let Some(slug) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let fm = parse_frontmatter(&content);
        let title = fm
            .get("title")
            .cloned()
            .unwrap_or_else(|| slug.replace('-', " "));
        let memory_type = fm
            .get("type")
            .cloned()
            .unwrap_or_else(|| "reference".to_string());
        let scope = fm
            .get("scope")
            .cloned()
            .unwrap_or_else(|| "project".to_string());
        index_entries.push(IndexEntry {
            slug: slug.to_string(),
            title,
            memory_type,
            scope,
        });
    }

    let index = build_memory_index(index_entries);
    tokio::fs::write(memory_dir.join("MEMORY.md"), index)
        .await
        .map_err(execution_failed)
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
                slug.push(lower_ch);
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
