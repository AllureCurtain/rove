use std::io::ErrorKind;
use std::path::Path;

use crate::core::types::SessionId;
use crate::core::workspace::Workspace;

const MAX_SESSION_SUMMARY_LINES: usize = 200;
const MAX_SESSION_SUMMARY_BYTES: usize = 25_000;

/// Read `.rove/memory/sessions/<session_id>.md` for prompt construction.
pub fn read_session_summary_sync(
    workspace: &Workspace,
    session_id: SessionId,
) -> std::io::Result<Option<String>> {
    read_session_summary_from_dir_sync(
        &workspace.state_dir.join("memory").join("sessions"),
        session_id,
    )
}

pub fn read_session_summary_from_dir_sync(
    session_dir: &Path,
    session_id: SessionId,
) -> std::io::Result<Option<String>> {
    let path = session_dir.join(format!("{session_id}.md"));
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let truncated = truncate_session_summary(&content);
    let trimmed = truncated.trim_end();
    if trimmed.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

/// Write `.rove/memory/sessions/<session_id>.md` for future prompt construction.
pub fn write_session_summary_sync(
    workspace: &Workspace,
    session_id: SessionId,
    summary: &str,
) -> std::io::Result<()> {
    write_session_summary_to_dir_sync(
        &workspace.state_dir.join("memory").join("sessions"),
        session_id,
        summary,
    )
}

pub fn write_session_summary_to_dir_sync(
    session_dir: &Path,
    session_id: SessionId,
    summary: &str,
) -> std::io::Result<()> {
    let truncated = truncate_session_summary(summary);
    let trimmed = truncated.trim_end();
    if trimmed.trim().is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(session_dir)?;
    std::fs::write(session_dir.join(format!("{session_id}.md")), trimmed)
}

fn truncate_session_summary(content: &str) -> String {
    let byte_limited = truncate_to_byte_boundary(content, MAX_SESSION_SUMMARY_BYTES);
    byte_limited
        .lines()
        .take(MAX_SESSION_SUMMARY_LINES)
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
