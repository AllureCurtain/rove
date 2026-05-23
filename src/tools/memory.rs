use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use super::traits::{Tool, ToolOutput};
use crate::core::types::ToolSchema;
use crate::errors::ToolError;

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;
const MAX_TOPIC_SLUG_BYTES: usize = 80;

/// Save a durable memory topic under `.rove/memory/`.
pub struct SaveMemoryTool {
    root: PathBuf,
}

impl SaveMemoryTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
                    }
                },
                "required": ["topic", "content", "type"]
            }),
            destructive: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let raw_topic = required_string(&args, "topic")?;
        let raw_content = required_string(&args, "content")?;
        let raw_type = required_string(&args, "type")?;
        let slug = normalize_topic(raw_topic)?;
        let memory_type = MemoryType::parse(raw_type)?;
        let content = validate_content(raw_content)?;
        let title = display_title(raw_topic);

        let memory_dir = self.root.join(".rove").join("memory");
        let topics_dir = memory_dir.join("topics");
        tokio::fs::create_dir_all(&topics_dir)
            .await
            .map_err(execution_failed)?;

        let topic_path = topics_dir.join(format!("{slug}.md"));
        let now = Utc::now().to_rfc3339();
        let created_at = match tokio::fs::read_to_string(&topic_path).await {
            Ok(existing) => {
                parse_frontmatter_field(&existing, "created_at").unwrap_or_else(|| now.clone())
            }
            Err(err) if err.kind() == ErrorKind::NotFound => now.clone(),
            Err(err) => return Err(execution_failed(err)),
        };

        let topic_document = format!(
            "---\ntitle: {title}\ntype: {}\ncreated_at: {created_at}\nupdated_at: {now}\n---\n\n{content}\n",
            memory_type.as_str()
        );
        tokio::fs::write(&topic_path, topic_document)
            .await
            .map_err(execution_failed)?;

        update_memory_index(&memory_dir).await?;

        Ok(ToolOutput {
            content: format!("saved memory: {slug}"),
        })
    }
}

/// Rebuild `.rove/memory/MEMORY.md` from existing topic files.
pub struct UpdateMemoryIndexTool {
    root: PathBuf,
}

impl UpdateMemoryIndexTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
        }
    }

    async fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        let memory_dir = self.root.join(".rove").join("memory");
        tokio::fs::create_dir_all(&memory_dir)
            .await
            .map_err(execution_failed)?;
        update_memory_index(&memory_dir).await?;

        Ok(ToolOutput {
            content: "updated memory index".to_string(),
        })
    }
}

/// Read a durable memory topic from `.rove/memory/topics/`.
pub struct ReadMemoryTopicTool {
    root: PathBuf,
}

impl ReadMemoryTopicTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let raw_name = required_string(&args, "name")?;
        let slug = normalize_topic(raw_name)?;
        let topic_path = self
            .root
            .join(".rove")
            .join("memory")
            .join("topics")
            .join(format!("{slug}.md"));

        let content = tokio::fs::read_to_string(topic_path).await.map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                ToolError::InvalidInput {
                    reason: format!("memory topic not found: {slug}"),
                }
            } else {
                execution_failed(err)
            }
        })?;

        Ok(ToolOutput { content })
    }
}

#[derive(Debug, Clone, Copy)]
enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    fn parse(raw: &str) -> Result<Self, ToolError> {
        match raw.trim() {
            "user" => Ok(Self::User),
            "feedback" => Ok(Self::Feedback),
            "project" => Ok(Self::Project),
            "reference" => Ok(Self::Reference),
            other => Err(ToolError::InvalidInput {
                reason: format!(
                    "memory type must be one of user, feedback, project, reference; got {other}"
                ),
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }
}

struct IndexEntry {
    slug: String,
    title: String,
    memory_type: String,
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
        let title =
            parse_frontmatter_field(&content, "title").unwrap_or_else(|| slug.replace('-', " "));
        let memory_type =
            parse_frontmatter_field(&content, "type").unwrap_or_else(|| "reference".to_string());
        index_entries.push(IndexEntry {
            slug: slug.to_string(),
            title,
            memory_type,
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
            "- [{}](topics/{}.md) \u{2014} {} memory\n",
            entry.title, entry.slug, entry.memory_type
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
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            pending_dash = false;
        } else if matches!(ch, ' ' | '-' | '_') {
            if !slug.is_empty() {
                pending_dash = true;
            }
        } else {
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

fn display_title(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }
    let prefix = format!("{field}: ");
    for line in lines {
        if line == "---" {
            return None;
        }
        if let Some(value) = line.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

fn unsafe_topic_error() -> ToolError {
    ToolError::InvalidInput {
        reason: "topic must be a safe topic name using letters, numbers, spaces, hyphens, or underscores".to_string(),
    }
}

fn execution_failed(err: std::io::Error) -> ToolError {
    ToolError::ExecutionFailed {
        reason: err.to_string(),
    }
}
