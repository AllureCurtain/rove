use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use chrono::Utc;

use super::durable::{MemoryScope, MemoryType, parse_frontmatter, strip_frontmatter};

pub const PRODUCT_MEMORY_CONTENT_LIMIT_BYTES: usize = 64 * 1_024;

const MAX_MEMORY_INDEX_BYTES: u64 = 25_000;
const MAX_MEMORY_TOPICS: usize = 200;
const MAX_TOPIC_METADATA_BYTES: usize = 8 * 1_024;
const MAX_TOPIC_SLUG_BYTES: usize = 80;
const MAX_RECOVERY_DIRECTORY_ENTRIES: usize = 256;
const MAX_RECOVERY_CANDIDATES: usize = 8;
const TEMP_NAME_ATTEMPTS: usize = 16;
const REPLACEMENT_MARKER: &[u8] = b"rove-memory-index-replacement-v1\n";
const REPLACEMENT_PREFIX: &str = ".memory-index-";
const TOPIC_REPLACEMENT_MARKER: &str = "rove-memory-topic-replacement-v1";
const TOPIC_REPLACEMENT_PREFIX: &str = ".memory-topic-";
const MAX_TOPIC_DOCUMENT_BYTES: usize =
    PRODUCT_MEMORY_CONTENT_LIMIT_BYTES + MAX_TOPIC_METADATA_BYTES;
const MAX_TOPIC_TITLE_BYTES: usize = 256;
const MAX_TOPIC_DESCRIPTION_BYTES: usize = 1_024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static MANAGEMENT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedMemoryTopicContent {
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryTopicDeleteOutcome {
    pub topic_deleted: bool,
    pub index_entry_removed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedMemoryTopicInfo {
    pub slug: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub confidence: f32,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub description: String,
    pub source: ManagedMemorySource,
    pub metadata_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedMemorySource {
    ProductSettings,
    LlmTool,
    Other,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedMemoryTopicWrite {
    pub slug: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub confidence: f32,
    pub description: String,
    pub content: String,
}

pub fn is_valid_memory_topic_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_TOPIC_SLUG_BYTES
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-')
}

pub fn read_memory_topic_for_product_sync(
    memory_dir: &Path,
    slug: &str,
) -> std::io::Result<Option<ManagedMemoryTopicContent>> {
    let _guard = management_guard()?;
    recover_interrupted_index_replacement(memory_dir)?;
    if !is_valid_memory_topic_slug(slug) {
        return Err(invalid_input("invalid memory topic slug"));
    }
    let Some(topic_path) = checked_topic_file(memory_dir, slug)? else {
        return Ok(None);
    };
    read_topic_body_with_limit(&topic_path, PRODUCT_MEMORY_CONTENT_LIMIT_BYTES).map(Some)
}

pub fn list_memory_topics_for_product_sync(
    memory_dir: &Path,
) -> std::io::Result<Vec<ManagedMemoryTopicInfo>> {
    let _guard = management_guard()?;
    recover_interrupted_index_replacement(memory_dir)?;
    list_memory_topics_unlocked(memory_dir)
}

pub fn delete_memory_topic_for_product_sync(
    memory_dir: &Path,
    slug: &str,
) -> std::io::Result<MemoryTopicDeleteOutcome> {
    let _guard = management_guard()?;
    recover_interrupted_index_replacement(memory_dir)?;
    if !is_valid_memory_topic_slug(slug) {
        return Err(invalid_input("invalid memory topic slug"));
    }
    let topic_path = checked_topic_file(memory_dir, slug)?;
    let index_replacement = prepare_index_replacement(memory_dir, slug)?;

    let topic_deleted = if let Some(topic_path) = topic_path {
        fs::remove_file(topic_path)?;
        true
    } else {
        false
    };
    let index_entry_removed = if let Some(replacement) = index_replacement {
        replace_index(replacement)?;
        true
    } else {
        false
    };

    Ok(MemoryTopicDeleteOutcome {
        topic_deleted,
        index_entry_removed,
    })
}

pub fn create_memory_topic_for_product_sync(
    memory_dir: &Path,
    topic: ManagedMemoryTopicWrite,
) -> std::io::Result<ManagedMemoryTopicInfo> {
    write_memory_topic_for_product_sync(memory_dir, topic, MemoryTopicWriteMode::Create, None)
}

pub fn update_memory_topic_for_product_sync(
    memory_dir: &Path,
    topic: ManagedMemoryTopicWrite,
    expected_updated_at: Option<&str>,
) -> std::io::Result<ManagedMemoryTopicInfo> {
    write_memory_topic_for_product_sync(
        memory_dir,
        topic,
        MemoryTopicWriteMode::Update,
        expected_updated_at,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryTopicWriteMode {
    Create,
    Update,
}

fn write_memory_topic_for_product_sync(
    memory_dir: &Path,
    topic: ManagedMemoryTopicWrite,
    mode: MemoryTopicWriteMode,
    expected_updated_at: Option<&str>,
) -> std::io::Result<ManagedMemoryTopicInfo> {
    validate_memory_topic_write(&topic)?;
    let _guard = management_guard()?;
    ensure_memory_layout(memory_dir)?;
    recover_interrupted_index_replacement(memory_dir)?;
    recover_interrupted_topic_replacements(memory_dir)?;

    let existing = load_existing_topic(memory_dir, &topic.slug)?;
    match (mode, existing.as_ref()) {
        (MemoryTopicWriteMode::Create, Some(existing)) => {
            if !existing_matches_write(existing, &topic) {
                return Err(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    "memory topic already exists",
                ));
            }
        }
        (MemoryTopicWriteMode::Update, None) => {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                "memory topic does not exist",
            ));
        }
        (MemoryTopicWriteMode::Update, Some(existing)) => {
            let actual = existing.frontmatter.get("updated_at").map(String::as_str);
            if actual != expected_updated_at && !existing_matches_write(existing, &topic) {
                return Err(permission_denied(
                    "memory topic changed since it was loaded",
                ));
            }
        }
        (MemoryTopicWriteMode::Create, None) => {}
    }

    let index_content = prepare_index_upsert(memory_dir, &topic)?;
    let now = Utc::now().to_rfc3339();
    let created_at = existing
        .as_ref()
        .and_then(|existing| existing.frontmatter.get("created_at").cloned())
        .unwrap_or_else(|| now.clone());
    let document = render_topic_document(&topic, &created_at, &now);

    let document_already_matches = existing
        .as_ref()
        .is_some_and(|existing| existing.document == document);
    if !document_already_matches {
        persist_topic_document(
            memory_dir,
            &topic.slug,
            document.as_bytes(),
            existing
                .as_ref()
                .map(|existing| existing.document.as_bytes()),
        )?;
    }
    write_memory_index_unlocked(memory_dir, index_content)?;

    list_memory_topics_unlocked(memory_dir)?
        .into_iter()
        .find(|candidate| candidate.slug == topic.slug)
        .ok_or_else(|| invalid_data("memory topic was not indexed after write"))
}

fn validate_memory_topic_write(topic: &ManagedMemoryTopicWrite) -> std::io::Result<()> {
    if !is_valid_memory_topic_slug(&topic.slug) {
        return Err(invalid_input("invalid memory topic slug"));
    }
    if !valid_single_line_metadata(&topic.title, MAX_TOPIC_TITLE_BYTES)
        || topic.title.contains('[')
        || topic.title.contains(']')
    {
        return Err(invalid_input("invalid memory topic title"));
    }
    if !topic.description.is_empty()
        && !valid_single_line_metadata(&topic.description, MAX_TOPIC_DESCRIPTION_BYTES)
    {
        return Err(invalid_input("invalid memory topic description"));
    }
    if !topic.confidence.is_finite() || !(0.0..=1.0).contains(&topic.confidence) {
        return Err(invalid_input("invalid memory topic confidence"));
    }
    if topic.content.len() > PRODUCT_MEMORY_CONTENT_LIMIT_BYTES || topic.content.contains('\0') {
        return Err(invalid_input("invalid memory topic content"));
    }
    Ok(())
}

fn valid_single_line_metadata(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '\r' | '\n'))
}

fn ensure_memory_layout(memory_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(memory_dir)?;
    let memory_metadata = fs::symlink_metadata(memory_dir)?;
    if memory_metadata.file_type().is_symlink() || !memory_metadata.is_dir() {
        return Err(permission_denied(
            "memory location must be a regular directory",
        ));
    }
    let topics_dir = memory_dir.join("topics");
    fs::create_dir_all(&topics_dir)?;
    let topics_metadata = fs::symlink_metadata(&topics_dir)?;
    if topics_metadata.file_type().is_symlink() || !topics_metadata.is_dir() {
        return Err(permission_denied(
            "memory topics location must be a regular directory",
        ));
    }
    let canonical_memory = fs::canonicalize(memory_dir)?;
    let canonical_topics = fs::canonicalize(topics_dir)?;
    if canonical_topics.parent() != Some(canonical_memory.as_path()) {
        return Err(permission_denied(
            "memory topics directory escapes the configured memory directory",
        ));
    }
    Ok(())
}

struct ExistingMemoryTopic {
    document: String,
    frontmatter: BTreeMap<String, String>,
    body: String,
}

fn load_existing_topic(
    memory_dir: &Path,
    slug: &str,
) -> std::io::Result<Option<ExistingMemoryTopic>> {
    let Some(path) = checked_topic_file(memory_dir, slug)? else {
        return Ok(None);
    };
    let document = read_utf8_with_limit(&path, MAX_TOPIC_DOCUMENT_BYTES)?;
    if document.truncated {
        return Err(invalid_data("memory topic exceeds its supported size"));
    }
    let frontmatter = parse_frontmatter(&document.content);
    let body = product_topic_body(&document.content)?.to_string();
    Ok(Some(ExistingMemoryTopic {
        document: document.content,
        frontmatter,
        body,
    }))
}

fn existing_matches_write(existing: &ExistingMemoryTopic, topic: &ManagedMemoryTopicWrite) -> bool {
    existing.frontmatter.get("title") == Some(&topic.title)
        && existing
            .frontmatter
            .get("type")
            .is_some_and(|value| MemoryType::parse(value) == Some(topic.memory_type))
        && existing
            .frontmatter
            .get("scope")
            .is_some_and(|value| MemoryScope::parse(value) == topic.scope)
        && existing
            .frontmatter
            .get("confidence")
            .and_then(|value| value.parse::<f32>().ok())
            .is_some_and(|value| (value - topic.confidence).abs() < 0.005)
        && existing
            .frontmatter
            .get("description")
            .map(String::as_str)
            .unwrap_or_default()
            == topic.description
        && existing.body == topic.content
}

fn render_topic_document(
    topic: &ManagedMemoryTopicWrite,
    created_at: &str,
    updated_at: &str,
) -> String {
    format!(
        "---\ntitle: {}\ntype: {}\nscope: {}\nsource: product_settings\nconfidence: {:.2}\ndescription: {}\ncreated_at: {created_at}\nupdated_at: {updated_at}\n---\n{}",
        topic.title,
        topic.memory_type.as_str(),
        topic.scope.as_str(),
        topic.confidence,
        topic.description,
        topic.content,
    )
}

fn prepare_index_upsert(
    memory_dir: &Path,
    topic: &ManagedMemoryTopicWrite,
) -> std::io::Result<Vec<u8>> {
    let mut index = match read_safe_index(memory_dir)? {
        Some(index) => String::from_utf8(index.content)
            .map_err(|_| invalid_data("memory index is not valid UTF-8"))?,
        None => "# rove Memory\n\n".to_string(),
    };
    let mut topic_count = 0usize;
    let mut found = false;
    let mut lines = Vec::new();
    for line in index.lines() {
        if let Some(slug) = memory_index_line_slug(line) {
            if !is_valid_memory_topic_slug(slug) {
                return Err(permission_denied(
                    "memory index contains an unsafe topic slug",
                ));
            }
            topic_count = topic_count.saturating_add(1);
            if slug == topic.slug {
                found = true;
                continue;
            }
        }
        lines.push(line);
    }
    if !found && topic_count >= MAX_MEMORY_TOPICS {
        return Err(invalid_data(
            "memory topic catalog exceeds its supported size",
        ));
    }
    index = lines.join("\n");
    if !index.ends_with('\n') {
        index.push('\n');
    }
    if !index.ends_with("\n\n") && index.lines().count() <= 1 {
        index.push('\n');
    }
    let description = if topic.description.is_empty() {
        format!(
            "{} {} memory",
            topic.scope.as_str(),
            topic.memory_type.as_str()
        )
    } else {
        topic.description.clone()
    };
    index.push_str(&format!(
        "- [{}](topics/{}.md) - {}\n",
        topic.title, topic.slug, description
    ));
    if index.len() > usize::try_from(MAX_MEMORY_INDEX_BYTES).unwrap_or(usize::MAX) {
        return Err(invalid_data("memory index exceeds its supported size"));
    }
    Ok(index.into_bytes())
}

fn persist_topic_document(
    memory_dir: &Path,
    slug: &str,
    document: &[u8],
    expected_original: Option<&[u8]>,
) -> std::io::Result<()> {
    let canonical_topics = fs::canonicalize(memory_dir.join("topics"))?;
    let destination = canonical_topics.join(format!("{slug}.md"));
    let (temporary, backup, ready, mut file) = create_topic_replacement_file(&canonical_topics)?;
    let before_marker = (|| {
        file.write_all(document)?;
        file.sync_all()?;
        drop(file);
        write_topic_ready_marker(&ready, slug)
    })();
    if let Err(error) = before_marker {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let result = (|| {
        match expected_original {
            Some(expected) => {
                let current = fs::read(&destination)?;
                if current != expected {
                    return Err(permission_denied("memory topic changed before replacement"));
                }
                ensure_path_absent(&backup)?;
                fs::rename(&destination, &backup)?;
            }
            None => ensure_path_absent(&destination)?,
        }
        if let Err(error) = fs::rename(&temporary, &destination) {
            if expected_original.is_some() {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(error);
        }
        if expected_original.is_some() {
            fs::remove_file(&backup)?;
        }
        fs::remove_file(&ready)?;
        sync_directory(&canonical_topics)
    })();
    if result.is_err() {
        let _ = recover_interrupted_topic_replacements(memory_dir);
    }
    result
}

fn create_topic_replacement_file(
    topics_dir: &Path,
) -> std::io::Result<(PathBuf, PathBuf, PathBuf, File)> {
    for _ in 0..TEMP_NAME_ATTEMPTS {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stem = format!("{TOPIC_REPLACEMENT_PREFIX}{}-{suffix}", std::process::id());
        let temporary = topics_dir.join(format!("{stem}.tmp"));
        let backup = topics_dir.join(format!("{stem}.bak"));
        let ready = topics_dir.join(format!("{stem}.ready"));
        if path_exists(&backup)? || path_exists(&ready)? {
            continue;
        }
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, backup, ready, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        ErrorKind::AlreadyExists,
        "could not reserve a memory topic replacement file",
    ))
}

fn write_topic_ready_marker(path: &Path, slug: &str) -> std::io::Result<()> {
    let mut marker = OpenOptions::new().create_new(true).write(true).open(path)?;
    marker.write_all(format!("{TOPIC_REPLACEMENT_MARKER}\n{slug}\n").as_bytes())?;
    marker.sync_all()
}

fn recover_interrupted_topic_replacements(memory_dir: &Path) -> std::io::Result<()> {
    let topics_dir = memory_dir.join("topics");
    let metadata = match fs::symlink_metadata(&topics_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(permission_denied(
            "memory topics location must be a regular directory",
        ));
    }
    let canonical_topics = fs::canonicalize(&topics_dir)?;
    let mut artifacts = BTreeMap::<String, RecoveryArtifacts>::new();
    for (entry_index, entry) in fs::read_dir(&canonical_topics)?.enumerate() {
        if entry_index >= MAX_RECOVERY_DIRECTORY_ENTRIES {
            return Err(invalid_data(
                "memory topics directory exceeds its supported size",
            ));
        }
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(TOPIC_REPLACEMENT_PREFIX) {
            continue;
        }
        let (stem, kind) = if let Some(stem) = name.strip_suffix(".tmp") {
            (stem, RecoveryArtifactKind::Temporary)
        } else if let Some(stem) = name.strip_suffix(".bak") {
            (stem, RecoveryArtifactKind::Backup)
        } else if let Some(stem) = name.strip_suffix(".ready") {
            (stem, RecoveryArtifactKind::Ready)
        } else {
            continue;
        };
        if artifacts.len() >= MAX_RECOVERY_CANDIDATES && !artifacts.contains_key(stem) {
            return Err(invalid_data("too many memory topic recovery candidates"));
        }
        validate_topic_recovery_file(&canonical_topics, &entry.path())?;
        let candidate = artifacts.entry(stem.to_string()).or_default();
        let slot = match kind {
            RecoveryArtifactKind::Temporary => &mut candidate.temporary,
            RecoveryArtifactKind::Backup => &mut candidate.backup,
            RecoveryArtifactKind::Ready => &mut candidate.ready,
        };
        if slot.replace(entry.path()).is_some() {
            return Err(permission_denied(
                "duplicate memory topic recovery artifact",
            ));
        }
    }

    for candidate in artifacts.into_values() {
        let Some(ready) = candidate.ready.as_ref() else {
            if candidate.backup.is_some() {
                return Err(permission_denied(
                    "memory topic backup has no valid recovery marker",
                ));
            }
            if let Some(temporary) = candidate.temporary {
                fs::remove_file(temporary)?;
            }
            continue;
        };
        let slug = read_topic_ready_slug(ready)?;
        let destination = canonical_topics.join(format!("{slug}.md"));
        let destination_exists = path_exists(&destination)?;
        if destination_exists {
            validate_topic_recovery_file(&canonical_topics, &destination)?;
        }
        if let Some(backup) = candidate.backup.as_ref() {
            if destination_exists {
                fs::remove_file(backup)?;
            } else {
                fs::rename(backup, &destination)?;
            }
        }
        if let Some(temporary) = candidate.temporary {
            fs::remove_file(temporary)?;
        }
        fs::remove_file(ready)?;
    }
    sync_directory(&canonical_topics)
}

fn validate_topic_recovery_file(canonical_topics: &Path, path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(permission_denied(
            "memory topic recovery artifact must be a regular file",
        ));
    }
    if fs::canonicalize(path)?.parent() != Some(canonical_topics) {
        return Err(permission_denied(
            "memory topic recovery artifact escapes the topics directory",
        ));
    }
    Ok(())
}

fn read_topic_ready_slug(path: &Path) -> std::io::Result<String> {
    let marker = read_utf8_with_limit(path, 256)?;
    if marker.truncated {
        return Err(permission_denied("memory topic recovery marker is invalid"));
    }
    let mut lines = marker.content.lines();
    if lines.next() != Some(TOPIC_REPLACEMENT_MARKER) {
        return Err(permission_denied("memory topic recovery marker is invalid"));
    }
    let slug = lines
        .next()
        .filter(|slug| is_valid_memory_topic_slug(slug))
        .ok_or_else(|| permission_denied("memory topic recovery marker is invalid"))?;
    Ok(slug.to_string())
}

pub(crate) fn checked_topic_file(
    memory_dir: &Path,
    slug: &str,
) -> std::io::Result<Option<PathBuf>> {
    let topics_dir = memory_dir.join("topics");
    let topic_path = topics_dir.join(format!("{slug}.md"));
    let metadata = match fs::symlink_metadata(&topic_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(permission_denied("memory topic must be a regular file"));
    }

    let canonical_memory = fs::canonicalize(memory_dir)?;
    let canonical_topics = fs::canonicalize(&topics_dir)?;
    if !canonical_topics.starts_with(&canonical_memory) {
        return Err(permission_denied(
            "memory topics directory escapes the configured memory directory",
        ));
    }
    let canonical_topic = fs::canonicalize(&topic_path)?;
    if !canonical_topic.starts_with(&canonical_topics) {
        return Err(permission_denied(
            "memory topic escapes the configured topics directory",
        ));
    }
    Ok(Some(canonical_topic))
}

fn read_utf8_with_limit(
    path: &Path,
    max_bytes: usize,
) -> std::io::Result<ManagedMemoryTopicContent> {
    let read_limit = max_bytes.saturating_add(4);
    let mut bytes = Vec::with_capacity(max_bytes);
    File::open(path)?
        .take(u64::try_from(read_limit).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) if error.utf8_error().error_len().is_none() && truncated => {
            let valid_up_to = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid_up_to].to_vec())
                .map_err(|_| invalid_data("memory topic is not valid UTF-8"))?
        }
        Err(_) => return Err(invalid_data("memory topic is not valid UTF-8")),
    };
    Ok(ManagedMemoryTopicContent { content, truncated })
}

fn read_topic_body_with_limit(
    path: &Path,
    max_body_bytes: usize,
) -> std::io::Result<ManagedMemoryTopicContent> {
    let max_read_bytes = max_body_bytes
        .saturating_add(MAX_TOPIC_METADATA_BYTES)
        .saturating_add(8);
    let content = read_utf8_with_limit(path, max_read_bytes)?;
    let body = product_topic_body(&content.content)?;
    let body_truncated = body.len() > max_body_bytes;
    let body = truncate_utf8(body, max_body_bytes).to_string();
    Ok(ManagedMemoryTopicContent {
        content: body,
        truncated: content.truncated || body_truncated,
    })
}

fn product_topic_body(content: &str) -> std::io::Result<&str> {
    if let Some(content) = content.strip_prefix("---\n") {
        return content
            .split_once("\n---\n")
            .map(|(_, body)| body)
            .ok_or_else(|| invalid_data("memory topic frontmatter is malformed or too large"));
    }
    if let Some(content) = content.strip_prefix("---\r\n") {
        return content
            .split_once("\r\n---\r\n")
            .map(|(_, body)| body)
            .ok_or_else(|| invalid_data("memory topic frontmatter is malformed or too large"));
    }
    if content.starts_with("---") {
        return Err(invalid_data("memory topic frontmatter is malformed"));
    }
    Ok(strip_frontmatter(content))
}

fn truncate_utf8(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = 0;
    for (index, character) in content.char_indices() {
        let next = index + character.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &content[..end]
}

fn list_memory_topics_unlocked(memory_dir: &Path) -> std::io::Result<Vec<ManagedMemoryTopicInfo>> {
    let Some(index) = read_safe_index(memory_dir)? else {
        return Ok(Vec::new());
    };
    let index_text = String::from_utf8(index.content)
        .map_err(|_| invalid_data("memory index is not valid UTF-8"))?;
    let mut topics = Vec::new();
    for line in index_text.lines() {
        let Some((title, slug, description)) = parse_memory_index_line(line) else {
            continue;
        };
        if !is_valid_memory_topic_slug(slug) {
            return Err(permission_denied(
                "memory index contains an unsafe topic slug",
            ));
        }
        let Some(topic_path) = checked_topic_file(memory_dir, slug)? else {
            continue;
        };
        if topics.len() >= MAX_MEMORY_TOPICS {
            return Err(invalid_data(
                "memory topic catalog exceeds its supported size",
            ));
        }
        let metadata_content = read_utf8_with_limit(&topic_path, MAX_TOPIC_METADATA_BYTES)?;
        let frontmatter = parse_frontmatter(&metadata_content.content);
        let fallback_type = description
            .split_whitespace()
            .find_map(MemoryType::parse)
            .unwrap_or(MemoryType::Reference);
        topics.push(ManagedMemoryTopicInfo {
            slug: slug.to_string(),
            title: frontmatter
                .get("title")
                .cloned()
                .unwrap_or_else(|| title.to_string()),
            memory_type: frontmatter
                .get("type")
                .and_then(|value| MemoryType::parse(value))
                .unwrap_or(fallback_type),
            scope: frontmatter
                .get("scope")
                .map(|value| MemoryScope::parse(value))
                .unwrap_or_default(),
            confidence: frontmatter
                .get("confidence")
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite())
                .unwrap_or(0.7)
                .clamp(0.0, 1.0),
            created_at: frontmatter.get("created_at").cloned(),
            updated_at: frontmatter.get("updated_at").cloned(),
            description: frontmatter
                .get("description")
                .cloned()
                .unwrap_or_else(|| description.to_string()),
            source: managed_memory_source(frontmatter.get("source").map(String::as_str)),
            metadata_truncated: metadata_content.truncated,
        });
    }
    Ok(topics)
}

fn managed_memory_source(source: Option<&str>) -> ManagedMemorySource {
    match source.map(str::trim) {
        Some("product_settings") => ManagedMemorySource::ProductSettings,
        Some("llm_tool") => ManagedMemorySource::LlmTool,
        Some("") | None => ManagedMemorySource::Unknown,
        Some(_) => ManagedMemorySource::Other,
    }
}

fn parse_memory_index_line(line: &str) -> Option<(&str, &str, &str)> {
    let after_list = line.trim().strip_prefix("- [")?;
    let (title, rest) = after_list.split_once("](topics/")?;
    let (slug, rest) = rest.split_once(".md)")?;
    let description = rest
        .trim_start()
        .trim_start_matches('-')
        .trim_start_matches('\u{2014}')
        .trim();
    Some((title, slug, description))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexGeneration {
    len: u64,
    modified: Option<SystemTime>,
}

impl IndexGeneration {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

struct SafeIndex {
    canonical_memory: PathBuf,
    destination: PathBuf,
    generation: IndexGeneration,
    content: Vec<u8>,
}

struct IndexReplacement {
    canonical_memory: PathBuf,
    destination: PathBuf,
    original_generation: IndexGeneration,
    original_content: Vec<u8>,
    replacement_content: Vec<u8>,
}

pub(crate) fn management_guard() -> std::io::Result<MutexGuard<'static, ()>> {
    MANAGEMENT_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("memory management lock is poisoned"))
}

fn read_safe_index(memory_dir: &Path) -> std::io::Result<Option<SafeIndex>> {
    let index_path = memory_dir.join("MEMORY.md");
    let path_metadata = match fs::symlink_metadata(&index_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(permission_denied("memory index must be a regular file"));
    }
    if path_metadata.len() > MAX_MEMORY_INDEX_BYTES {
        return Err(invalid_data("memory index exceeds its supported size"));
    }

    let canonical_memory = fs::canonicalize(memory_dir)?;
    let destination = fs::canonicalize(&index_path)?;
    if destination.parent() != Some(canonical_memory.as_path()) {
        return Err(permission_denied(
            "memory index escapes the configured memory directory",
        ));
    }

    let mut file = File::open(&destination)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > MAX_MEMORY_INDEX_BYTES {
        return Err(invalid_data("memory index exceeds its supported size"));
    }
    let generation = IndexGeneration::from_metadata(&before);
    let mut content = Vec::with_capacity(usize::try_from(before.len()).unwrap_or(0));
    Read::by_ref(&mut file)
        .take(MAX_MEMORY_INDEX_BYTES.saturating_add(1))
        .read_to_end(&mut content)?;
    if content.len() > usize::try_from(MAX_MEMORY_INDEX_BYTES).unwrap_or(usize::MAX) {
        return Err(invalid_data("memory index exceeds its supported size"));
    }
    if generation != IndexGeneration::from_metadata(&file.metadata()?) {
        return Err(permission_denied("memory index changed while it was read"));
    }

    let current_path_metadata = fs::symlink_metadata(&index_path)?;
    if current_path_metadata.file_type().is_symlink() || !current_path_metadata.is_file() {
        return Err(permission_denied("memory index changed while it was read"));
    }
    if fs::canonicalize(&index_path)? != destination {
        return Err(permission_denied("memory index changed while it was read"));
    }

    Ok(Some(SafeIndex {
        canonical_memory,
        destination,
        generation,
        content,
    }))
}

fn prepare_index_replacement(
    memory_dir: &Path,
    slug: &str,
) -> std::io::Result<Option<IndexReplacement>> {
    let Some(index) = read_safe_index(memory_dir)? else {
        return Ok(None);
    };
    let index_text = String::from_utf8(index.content.clone())
        .map_err(|_| invalid_data("memory index is not valid UTF-8"))?;
    let mut removed = false;
    let mut updated = index_text
        .lines()
        .filter(|line| {
            let matches = memory_index_line_slug(line) == Some(slug);
            removed |= matches;
            !matches
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !removed {
        return Ok(None);
    }
    if !updated.is_empty() {
        updated.push('\n');
    }
    Ok(Some(IndexReplacement {
        canonical_memory: index.canonical_memory,
        destination: index.destination,
        original_generation: index.generation,
        original_content: index.content,
        replacement_content: updated.into_bytes(),
    }))
}

pub(crate) fn write_memory_index_unlocked(
    memory_dir: &Path,
    content: Vec<u8>,
) -> std::io::Result<()> {
    if content.len() > usize::try_from(MAX_MEMORY_INDEX_BYTES).unwrap_or(usize::MAX) {
        return Err(invalid_data("memory index exceeds its supported size"));
    }
    recover_interrupted_index_replacement(memory_dir)?;
    if let Some(index) = read_safe_index(memory_dir)? {
        return replace_index(IndexReplacement {
            canonical_memory: index.canonical_memory,
            destination: index.destination,
            original_generation: index.generation,
            original_content: index.content,
            replacement_content: content,
        });
    }

    let canonical_memory = fs::canonicalize(memory_dir)?;
    let destination = canonical_memory.join("MEMORY.md");
    let (temporary, _backup, _ready, mut file) = create_replacement_file(&canonical_memory)?;
    let result = (|| {
        file.write_all(&content)?;
        file.sync_all()?;
        drop(file);
        ensure_path_absent(&destination)?;
        fs::hard_link(&temporary, &destination)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&destination)?
            .sync_all()?;
        fs::remove_file(&temporary)?;
        sync_directory(&canonical_memory)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn replace_index(replacement: IndexReplacement) -> std::io::Result<()> {
    let (temporary, backup, ready, mut file) =
        create_replacement_file(&replacement.canonical_memory)?;
    let result = (|| {
        file.write_all(&replacement.replacement_content)?;
        file.sync_all()?;
        drop(file);

        ensure_index_unchanged(&replacement)?;
        write_ready_marker(&ready)?;
        ensure_index_unchanged(&replacement)?;
        ensure_path_absent(&backup)?;

        fs::rename(&replacement.destination, &backup)?;
        if let Err(error) = fs::rename(&temporary, &replacement.destination) {
            let _ = restore_backup(&backup, &replacement.destination);
            return Err(error);
        }
        fs::remove_file(&backup)?;
        fs::remove_file(&ready)?;
        sync_directory(&replacement.canonical_memory)?;
        Ok(())
    })();
    if result.is_err() && fs::symlink_metadata(&backup).is_err() {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&ready);
    }
    result
}

fn ensure_index_unchanged(replacement: &IndexReplacement) -> std::io::Result<()> {
    let Some(current) = read_safe_index(&replacement.canonical_memory)? else {
        return Err(permission_denied("memory index changed before replacement"));
    };
    if current.destination != replacement.destination
        || current.generation != replacement.original_generation
        || current.content != replacement.original_content
    {
        return Err(permission_denied("memory index changed before replacement"));
    }
    Ok(())
}

fn create_replacement_file(
    memory_dir: &Path,
) -> std::io::Result<(PathBuf, PathBuf, PathBuf, File)> {
    for _ in 0..TEMP_NAME_ATTEMPTS {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stem = format!("{REPLACEMENT_PREFIX}{}-{suffix}", std::process::id());
        let temporary = memory_dir.join(format!("{stem}.tmp"));
        let backup = memory_dir.join(format!("{stem}.bak"));
        let ready = memory_dir.join(format!("{stem}.ready"));
        if path_exists(&backup)? || path_exists(&ready)? {
            continue;
        }
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, backup, ready, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        ErrorKind::AlreadyExists,
        "could not reserve a memory index replacement file",
    ))
}

fn write_ready_marker(path: &Path) -> std::io::Result<()> {
    let mut marker = OpenOptions::new().create_new(true).write(true).open(path)?;
    marker.write_all(REPLACEMENT_MARKER)?;
    marker.sync_all()
}

fn restore_backup(backup: &Path, destination: &Path) -> std::io::Result<()> {
    fs::hard_link(backup, destination)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(destination)?
        .sync_all()?;
    fs::remove_file(backup)
}

fn ensure_path_absent(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(permission_denied(
            "memory index replacement artifact already exists",
        )),
        Err(error) => Err(error),
    }
}

fn path_exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RecoveryArtifactKind {
    Temporary,
    Backup,
    Ready,
}

#[derive(Debug, Default)]
struct RecoveryArtifacts {
    temporary: Option<PathBuf>,
    backup: Option<PathBuf>,
    ready: Option<PathBuf>,
}

pub(crate) fn recover_interrupted_index_replacement(memory_dir: &Path) -> std::io::Result<()> {
    let Some((canonical_memory, candidates)) = recovery_candidates(memory_dir)? else {
        return Ok(());
    };
    for candidate in candidates.into_values() {
        recover_candidate(&canonical_memory, candidate)?;
    }
    Ok(())
}

fn recovery_candidates(
    memory_dir: &Path,
) -> std::io::Result<Option<(PathBuf, BTreeMap<String, RecoveryArtifacts>)>> {
    let metadata = match fs::metadata(memory_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() {
        return Err(permission_denied(
            "configured memory location must be a directory",
        ));
    }
    let canonical_memory = fs::canonicalize(memory_dir)?;
    let mut candidates = BTreeMap::<String, RecoveryArtifacts>::new();
    for (entry_index, entry) in fs::read_dir(&canonical_memory)?.enumerate() {
        if entry_index >= MAX_RECOVERY_DIRECTORY_ENTRIES {
            return Err(invalid_data(
                "memory directory exceeds its supported recovery scan size",
            ));
        }
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some((stem, kind)) = recovery_artifact_name(file_name) else {
            continue;
        };
        if !candidates.contains_key(stem) && candidates.len() >= MAX_RECOVERY_CANDIDATES {
            return Err(invalid_data(
                "memory directory contains too many recovery candidates",
            ));
        }
        let artifacts = candidates.entry(stem.to_string()).or_default();
        let target = match kind {
            RecoveryArtifactKind::Temporary => &mut artifacts.temporary,
            RecoveryArtifactKind::Backup => &mut artifacts.backup,
            RecoveryArtifactKind::Ready => &mut artifacts.ready,
        };
        *target = Some(entry.path());
    }
    Ok(Some((canonical_memory, candidates)))
}

fn recovery_artifact_name(file_name: &str) -> Option<(&str, RecoveryArtifactKind)> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    let transaction = stem.strip_prefix(REPLACEMENT_PREFIX)?;
    let (process_id, counter) = transaction.split_once('-')?;
    if process_id.is_empty()
        || counter.is_empty()
        || !process_id.bytes().all(|byte| byte.is_ascii_digit())
        || !counter.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let kind = match extension {
        "tmp" => RecoveryArtifactKind::Temporary,
        "bak" => RecoveryArtifactKind::Backup,
        "ready" => RecoveryArtifactKind::Ready,
        _ => return None,
    };
    Some((stem, kind))
}

fn recover_candidate(canonical_memory: &Path, candidate: RecoveryArtifacts) -> std::io::Result<()> {
    if let Some(path) = candidate.temporary.as_deref() {
        validate_recovery_file(canonical_memory, path)?;
    }
    if let Some(path) = candidate.backup.as_deref() {
        validate_recovery_file(canonical_memory, path)?;
    }
    let Some(ready) = candidate.ready.as_deref() else {
        if candidate.backup.is_some() {
            return Err(permission_denied(
                "memory index backup has no valid recovery marker",
            ));
        }
        if let Some(temporary) = candidate.temporary.as_deref() {
            remove_recovery_file(canonical_memory, temporary)?;
        }
        return Ok(());
    };
    validate_ready_marker(canonical_memory, ready)?;

    let destination = canonical_memory.join("MEMORY.md");
    if read_safe_index(canonical_memory)?.is_some() {
        if let Some(temporary) = candidate.temporary.as_deref() {
            remove_recovery_file(canonical_memory, temporary)?;
        }
        if let Some(backup) = candidate.backup.as_deref() {
            remove_recovery_file(canonical_memory, backup)?;
        }
        remove_recovery_file(canonical_memory, ready)?;
        sync_directory(canonical_memory)?;
        return Ok(());
    }

    let Some(backup) = candidate.backup.as_deref() else {
        return Err(permission_denied(
            "memory index is missing and no marked backup can restore it",
        ));
    };
    validate_recovery_index(canonical_memory, backup)?;
    restore_backup(backup, &destination)?;
    if let Some(temporary) = candidate.temporary.as_deref() {
        remove_recovery_file(canonical_memory, temporary)?;
    }
    remove_recovery_file(canonical_memory, ready)?;
    sync_directory(canonical_memory)
}

fn validate_recovery_file(canonical_memory: &Path, path: &Path) -> std::io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(permission_denied(
            "memory index recovery artifact must be a regular file",
        ));
    }
    if fs::canonicalize(path)?.parent() != Some(canonical_memory) {
        return Err(permission_denied(
            "memory index recovery artifact escapes the memory directory",
        ));
    }
    Ok(metadata)
}

fn validate_ready_marker(canonical_memory: &Path, path: &Path) -> std::io::Result<()> {
    let metadata = validate_recovery_file(canonical_memory, path)?;
    if metadata.len() != u64::try_from(REPLACEMENT_MARKER.len()).unwrap_or(u64::MAX) {
        return Err(permission_denied("memory index recovery marker is invalid"));
    }
    let mut marker = Vec::with_capacity(REPLACEMENT_MARKER.len());
    File::open(path)?.read_to_end(&mut marker)?;
    if marker != REPLACEMENT_MARKER {
        return Err(permission_denied("memory index recovery marker is invalid"));
    }
    Ok(())
}

fn validate_recovery_index(canonical_memory: &Path, path: &Path) -> std::io::Result<()> {
    let metadata = validate_recovery_file(canonical_memory, path)?;
    if metadata.len() > MAX_MEMORY_INDEX_BYTES {
        return Err(invalid_data(
            "memory index backup exceeds its supported size",
        ));
    }
    let content = read_utf8_with_limit(
        path,
        usize::try_from(MAX_MEMORY_INDEX_BYTES).unwrap_or(usize::MAX),
    )?;
    if content.truncated {
        return Err(invalid_data(
            "memory index backup exceeds its supported size",
        ));
    }
    Ok(())
}

fn remove_recovery_file(canonical_memory: &Path, path: &Path) -> std::io::Result<()> {
    match validate_recovery_file(canonical_memory, path) {
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn memory_index_line_slug(line: &str) -> Option<&str> {
    let (_, rest) = line.trim().strip_prefix("- [")?.split_once("](topics/")?;
    let (slug, _) = rest.split_once(".md)")?;
    Some(slug)
}

fn invalid_input(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, message)
}

fn permission_denied(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let memory_dir = temp.path().join("memory");
        fs::create_dir_all(memory_dir.join("topics")).unwrap();
        (temp, memory_dir)
    }

    fn write_index(memory_dir: &Path, lines: &[(&str, &str)]) {
        let mut index = String::from("# rove Memory\n\n");
        for (slug, title) in lines {
            index.push_str(&format!(
                "- [{title}](topics/{slug}.md) - project reference memory\n"
            ));
        }
        fs::write(memory_dir.join("MEMORY.md"), index).unwrap();
    }

    fn product_topic(slug: &str, content: &str) -> ManagedMemoryTopicWrite {
        ManagedMemoryTopicWrite {
            slug: slug.to_string(),
            title: "Project conventions".to_string(),
            memory_type: MemoryType::Project,
            scope: MemoryScope::Project,
            confidence: 0.9,
            description: "Stable project rules".to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn product_create_and_update_are_indexed_cas_safe_and_source_classified() {
        let (_temp, memory_dir) = setup();
        let created = create_memory_topic_for_product_sync(
            &memory_dir,
            product_topic("project-conventions", "Use focused tests."),
        )
        .unwrap();
        assert_eq!(created.source, ManagedMemorySource::ProductSettings);
        assert_eq!(created.description, "Stable project rules");
        let initial_updated_at = created.updated_at.clone().unwrap();
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "project-conventions")
                .unwrap()
                .unwrap()
                .content,
            "Use focused tests."
        );
        assert!(
            fs::read_to_string(memory_dir.join("MEMORY.md"))
                .unwrap()
                .contains("topics/project-conventions.md")
        );

        let different_create = create_memory_topic_for_product_sync(
            &memory_dir,
            product_topic("project-conventions", "Overwrite without update."),
        )
        .unwrap_err();
        assert_eq!(different_create.kind(), ErrorKind::AlreadyExists);

        let stale = update_memory_topic_for_product_sync(
            &memory_dir,
            product_topic("project-conventions", "Stale overwrite."),
            Some("2026-01-01T00:00:00Z"),
        )
        .unwrap_err();
        assert_eq!(stale.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "project-conventions")
                .unwrap()
                .unwrap()
                .content,
            "Use focused tests."
        );

        let updated = update_memory_topic_for_product_sync(
            &memory_dir,
            product_topic("project-conventions", "Run focused tests first."),
            Some(&initial_updated_at),
        )
        .unwrap();
        assert_eq!(updated.slug, "project-conventions");
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "project-conventions")
                .unwrap()
                .unwrap()
                .content,
            "Run focused tests first."
        );
    }

    #[test]
    fn product_write_rejects_index_injection_and_oversized_content() {
        let (_temp, memory_dir) = setup();
        let mut injected = product_topic("unsafe-title", "content");
        injected.title = "Unsafe](topics/escape.md)".to_string();
        assert_eq!(
            create_memory_topic_for_product_sync(&memory_dir, injected)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );

        let oversized = product_topic(
            "oversized",
            &"x".repeat(PRODUCT_MEMORY_CONTENT_LIMIT_BYTES + 1),
        );
        assert_eq!(
            create_memory_topic_for_product_sync(&memory_dir, oversized)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert!(!memory_dir.join("topics/oversized.md").exists());
    }

    #[test]
    fn product_topic_recovery_restores_a_marked_backup() {
        let (_temp, memory_dir) = setup();
        create_memory_topic_for_product_sync(
            &memory_dir,
            product_topic("recover-topic", "original"),
        )
        .unwrap();
        let topics_dir = memory_dir.join("topics");
        let destination = topics_dir.join("recover-topic.md");
        let stem = ".memory-topic-999-1";
        let backup = topics_dir.join(format!("{stem}.bak"));
        let ready = topics_dir.join(format!("{stem}.ready"));
        let temporary = topics_dir.join(format!("{stem}.tmp"));
        fs::rename(&destination, &backup).unwrap();
        fs::write(&temporary, "uncommitted").unwrap();
        fs::write(
            &ready,
            format!("{TOPIC_REPLACEMENT_MARKER}\nrecover-topic\n"),
        )
        .unwrap();

        recover_interrupted_topic_replacements(&memory_dir).unwrap();

        assert!(destination.exists());
        assert!(!backup.exists());
        assert!(!temporary.exists());
        assert!(!ready.exists());
        assert!(
            fs::read_to_string(destination)
                .unwrap()
                .contains("original")
        );
    }

    #[test]
    fn bounded_read_has_a_fixed_utf8_safe_limit() {
        let (_temp, memory_dir) = setup();
        fs::write(
            memory_dir.join("topics/large.md"),
            format!(
                "{}\u{754c}",
                "a".repeat(PRODUCT_MEMORY_CONTENT_LIMIT_BYTES - 1)
            ),
        )
        .unwrap();

        let topic = read_memory_topic_for_product_sync(&memory_dir, "large")
            .unwrap()
            .unwrap();

        assert!(topic.truncated);
        assert_eq!(topic.content.len(), PRODUCT_MEMORY_CONTENT_LIMIT_BYTES - 1);
    }

    #[test]
    fn read_rejects_invalid_slugs_directories_and_invalid_utf8() {
        let (_temp, memory_dir) = setup();
        fs::create_dir(memory_dir.join("topics/directory.md")).unwrap();
        fs::write(memory_dir.join("topics/corrupt.md"), [0xff, 0xfe]).unwrap();

        assert!(!is_valid_memory_topic_slug("../escape"));
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "../escape")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "directory")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "corrupt")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn delete_is_idempotent_and_repairs_a_stale_index() {
        let (_temp, memory_dir) = setup();
        fs::write(memory_dir.join("topics/remove-me.md"), "content").unwrap();
        write_index(&memory_dir, &[("remove-me", "Remove Me")]);

        let first = delete_memory_topic_for_product_sync(&memory_dir, "remove-me").unwrap();
        assert_eq!(
            first,
            MemoryTopicDeleteOutcome {
                topic_deleted: true,
                index_entry_removed: true
            }
        );

        write_index(&memory_dir, &[("remove-me", "Remove Me")]);
        let retry = delete_memory_topic_for_product_sync(&memory_dir, "remove-me").unwrap();
        assert_eq!(
            retry,
            MemoryTopicDeleteOutcome {
                topic_deleted: false,
                index_entry_removed: true
            }
        );
        assert!(
            !fs::read_to_string(memory_dir.join("MEMORY.md"))
                .unwrap()
                .contains("remove-me")
        );

        let settled = delete_memory_topic_for_product_sync(&memory_dir, "remove-me").unwrap();
        assert_eq!(
            settled,
            MemoryTopicDeleteOutcome {
                topic_deleted: false,
                index_entry_removed: false
            }
        );
    }

    #[test]
    fn delete_rejects_a_topic_directory_without_changing_the_index() {
        let (_temp, memory_dir) = setup();
        fs::create_dir(memory_dir.join("topics/blocked.md")).unwrap();
        write_index(&memory_dir, &[("blocked", "Blocked")]);
        let before = fs::read(memory_dir.join("MEMORY.md")).unwrap();

        let error = delete_memory_topic_for_product_sync(&memory_dir, "blocked").unwrap_err();

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(fs::read(memory_dir.join("MEMORY.md")).unwrap(), before);
    }

    #[test]
    fn delete_rejects_invalid_slugs_and_a_corrupt_index_before_mutation() {
        let (_temp, memory_dir) = setup();
        fs::write(memory_dir.join("topics/keep.md"), "content").unwrap();
        fs::write(memory_dir.join("MEMORY.md"), [0xff, 0xfe]).unwrap();

        assert_eq!(
            delete_memory_topic_for_product_sync(&memory_dir, "../keep")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            delete_memory_topic_for_product_sync(&memory_dir, "keep")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
        assert!(memory_dir.join("topics/keep.md").exists());
    }

    #[test]
    fn product_read_strips_frontmatter_including_source() {
        let (_temp, memory_dir) = setup();
        fs::write(
            memory_dir.join("topics/private-source.md"),
            "---\ntitle: Private Source\nsource: /private/workspace/notes.md\ntype: project\n---\nVisible body\n",
        )
        .unwrap();

        let topic = read_memory_topic_for_product_sync(&memory_dir, "private-source")
            .unwrap()
            .unwrap();

        assert_eq!(topic.content, "Visible body\n");
        assert!(!topic.content.contains("source"));
        assert!(!topic.content.contains("/private/workspace"));
    }

    #[test]
    fn product_catalog_rejects_corrupt_index_and_topic_utf8() {
        let (_temp, memory_dir) = setup();
        fs::write(memory_dir.join("MEMORY.md"), [0xff, 0xfe]).unwrap();
        assert_eq!(
            list_memory_topics_for_product_sync(&memory_dir)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );

        write_index(&memory_dir, &[("corrupt", "Corrupt")]);
        fs::write(memory_dir.join("topics/corrupt.md"), [0xff, 0xfe]).unwrap();
        assert_eq!(
            list_memory_topics_for_product_sync(&memory_dir)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn replacement_rejects_and_preserves_a_concurrent_index_change() {
        let (_temp, memory_dir) = setup();
        write_index(&memory_dir, &[("remove-me", "Remove Me")]);
        let replacement = prepare_index_replacement(&memory_dir, "remove-me")
            .unwrap()
            .unwrap();
        let concurrent =
            "# rove Memory\n\n- [Concurrent](topics/concurrent.md) - project reference memory\n";
        fs::write(memory_dir.join("MEMORY.md"), concurrent).unwrap();

        let error = replace_index(replacement).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap(),
            concurrent
        );
        assert!(
            fs::read_dir(&memory_dir)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(REPLACEMENT_PREFIX))
        );
    }

    #[test]
    fn recovery_discards_uncommitted_temporary_and_ready_files() {
        let (_temp, memory_dir) = setup();
        write_index(&memory_dir, &[("keep", "Keep")]);
        let original = fs::read(memory_dir.join("MEMORY.md")).unwrap();
        let stem = ".memory-index-999-1";
        let temporary = memory_dir.join(format!("{stem}.tmp"));
        let ready = memory_dir.join(format!("{stem}.ready"));
        fs::write(&temporary, "replacement").unwrap();
        fs::write(&ready, REPLACEMENT_MARKER).unwrap();

        recover_interrupted_index_replacement(&memory_dir).unwrap();

        assert_eq!(fs::read(memory_dir.join("MEMORY.md")).unwrap(), original);
        assert!(!temporary.exists());
        assert!(!ready.exists());

        let orphan = memory_dir.join(".memory-index-999-2.tmp");
        fs::write(&orphan, "orphan").unwrap();
        recover_interrupted_index_replacement(&memory_dir).unwrap();
        assert!(!orphan.exists());
    }

    #[test]
    fn recovery_restores_backup_when_destination_is_missing() {
        let (_temp, memory_dir) = setup();
        write_index(&memory_dir, &[("keep", "Keep")]);
        let original = fs::read(memory_dir.join("MEMORY.md")).unwrap();
        let stem = ".memory-index-999-3";
        let temporary = memory_dir.join(format!("{stem}.tmp"));
        let backup = memory_dir.join(format!("{stem}.bak"));
        let ready = memory_dir.join(format!("{stem}.ready"));
        fs::write(&temporary, "replacement").unwrap();
        fs::write(&ready, REPLACEMENT_MARKER).unwrap();
        fs::rename(memory_dir.join("MEMORY.md"), &backup).unwrap();

        recover_interrupted_index_replacement(&memory_dir).unwrap();

        assert_eq!(fs::read(memory_dir.join("MEMORY.md")).unwrap(), original);
        assert!(!temporary.exists());
        assert!(!backup.exists());
        assert!(!ready.exists());
    }

    #[test]
    fn recovery_keeps_committed_destination_and_cleans_backup() {
        let (_temp, memory_dir) = setup();
        let destination = memory_dir.join("MEMORY.md");
        let committed = "# rove Memory\n\n";
        fs::write(&destination, committed).unwrap();
        let stem = ".memory-index-999-4";
        let backup = memory_dir.join(format!("{stem}.bak"));
        let ready = memory_dir.join(format!("{stem}.ready"));
        fs::write(&backup, "# old index\n").unwrap();
        fs::write(&ready, REPLACEMENT_MARKER).unwrap();

        recover_interrupted_index_replacement(&memory_dir).unwrap();

        assert_eq!(fs::read_to_string(destination).unwrap(), committed);
        assert!(!backup.exists());
        assert!(!ready.exists());
    }

    #[test]
    fn recovery_rejects_unmarked_backups_and_invalid_markers() {
        let (_temp, memory_dir) = setup();
        write_index(&memory_dir, &[]);
        let backup = memory_dir.join(".memory-index-999-5.bak");
        fs::write(&backup, "# old index\n").unwrap();
        assert_eq!(
            recover_interrupted_index_replacement(&memory_dir)
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        fs::remove_file(backup).unwrap();

        let ready = memory_dir.join(".memory-index-999-6.ready");
        fs::write(&ready, "not a rove marker").unwrap();
        assert_eq!(
            recover_interrupted_index_replacement(&memory_dir)
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn recovery_candidate_scan_is_bounded() {
        let (_temp, memory_dir) = setup();
        for index in 0..=MAX_RECOVERY_CANDIDATES {
            fs::write(
                memory_dir.join(format!(".memory-index-999-{index}.tmp")),
                "temporary",
            )
            .unwrap();
        }

        assert_eq!(
            recover_interrupted_index_replacement(&memory_dir)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_and_delete_reject_topic_and_index_symlinks() {
        use std::os::unix::fs::symlink;

        let (temp, memory_dir) = setup();
        let outside_topic = temp.path().join("outside-topic.md");
        fs::write(&outside_topic, "outside").unwrap();
        symlink(&outside_topic, memory_dir.join("topics/linked.md")).unwrap();
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "linked")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );

        fs::remove_file(memory_dir.join("topics/linked.md")).unwrap();
        fs::write(memory_dir.join("topics/linked.md"), "inside").unwrap();
        let outside_index = temp.path().join("outside-index.md");
        fs::write(&outside_index, "outside index").unwrap();
        symlink(&outside_index, memory_dir.join("MEMORY.md")).unwrap();
        assert_eq!(
            delete_memory_topic_for_product_sync(&memory_dir, "linked")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        assert!(memory_dir.join("topics/linked.md").exists());
        assert_eq!(fs::read_to_string(outside_index).unwrap(), "outside index");
    }

    #[cfg(windows)]
    #[test]
    fn read_and_delete_reject_topic_and_index_symlinks_when_supported() {
        use std::os::windows::fs::symlink_file;

        let (temp, memory_dir) = setup();
        let outside_topic = temp.path().join("outside-topic.md");
        fs::write(&outside_topic, "outside").unwrap();
        if symlink_file(&outside_topic, memory_dir.join("topics/linked.md")).is_err() {
            return;
        }
        assert_eq!(
            read_memory_topic_for_product_sync(&memory_dir, "linked")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );

        fs::remove_file(memory_dir.join("topics/linked.md")).unwrap();
        fs::write(memory_dir.join("topics/linked.md"), "inside").unwrap();
        let outside_index = temp.path().join("outside-index.md");
        fs::write(&outside_index, "outside index").unwrap();
        symlink_file(&outside_index, memory_dir.join("MEMORY.md")).unwrap();
        assert_eq!(
            delete_memory_topic_for_product_sync(&memory_dir, "linked")
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        assert!(memory_dir.join("topics/linked.md").exists());
        assert_eq!(fs::read_to_string(outside_index).unwrap(), "outside index");
    }
}
