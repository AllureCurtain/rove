use std::io::ErrorKind;

use crate::core::workspace::Workspace;

const MAX_MEMORY_INDEX_LINES: usize = 200;
const MAX_MEMORY_INDEX_BYTES: usize = 25_000;

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
