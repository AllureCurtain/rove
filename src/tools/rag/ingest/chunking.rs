use crate::tools::rag::types::{DocumentChunk, ParsedDocument, sha256_hex};

pub trait ChunkingStrategy: Send + Sync {
    fn name(&self) -> &'static str;
    fn target_chars(&self) -> usize;
    fn overlap_chars(&self) -> usize;
    fn chunk(&self, document: &ParsedDocument) -> Vec<DocumentChunk>;
}

#[derive(Debug, Clone)]
pub struct FixedTextChunker {
    target_chars: usize,
    overlap_chars: usize,
}

impl FixedTextChunker {
    pub fn new(target_chars: usize, overlap_chars: usize) -> Self {
        let target_chars = target_chars.max(1);
        let overlap_chars = overlap_chars.min(target_chars.saturating_sub(1));
        Self {
            target_chars,
            overlap_chars,
        }
    }
}

impl ChunkingStrategy for FixedTextChunker {
    fn name(&self) -> &'static str {
        "fixed"
    }

    fn target_chars(&self) -> usize {
        self.target_chars
    }

    fn overlap_chars(&self) -> usize {
        self.overlap_chars
    }

    fn chunk(&self, document: &ParsedDocument) -> Vec<DocumentChunk> {
        let normalized = normalize_text(&document.content);
        if normalized.trim().is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let len = normalized.len();
        let mut start = 0;

        while start < len {
            let mut end = if start + self.target_chars >= len {
                len
            } else {
                let target_end = start + self.target_chars;
                if len - target_end <= self.overlap_chars {
                    len
                } else {
                    adjust_to_boundary(&normalized, start, target_end)
                }
            };

            if end <= start {
                end = (start + self.target_chars).min(len);
            }

            let content = normalized[start..end].trim().to_string();
            if !content.is_empty() {
                let id = format!("{}#{:04}", document.path, chunks.len());
                chunks.push(DocumentChunk {
                    id,
                    path: document.path.clone(),
                    kind: document.kind,
                    content_hash: document.content_hash.clone(),
                    chunk_hash: sha256_hex(content.as_bytes()),
                    start_byte: start,
                    end_byte: end,
                    heading: None,
                    content,
                });
            }

            if end >= len {
                break;
            }

            let next_start = end.saturating_sub(self.overlap_chars);
            start = if next_start <= start { end } else { next_start };
        }

        chunks
    }
}

#[derive(Debug, Clone)]
pub struct MarkdownAwareChunker {
    target_chars: usize,
    overlap_chars: usize,
}

impl MarkdownAwareChunker {
    pub fn new(target_chars: usize, overlap_chars: usize) -> Self {
        let target_chars = target_chars.max(1);
        let overlap_chars = overlap_chars.min(target_chars.saturating_sub(1));
        Self {
            target_chars,
            overlap_chars,
        }
    }
}

impl ChunkingStrategy for MarkdownAwareChunker {
    fn name(&self) -> &'static str {
        "markdown-aware"
    }

    fn target_chars(&self) -> usize {
        self.target_chars
    }

    fn overlap_chars(&self) -> usize {
        self.overlap_chars
    }

    fn chunk(&self, document: &ParsedDocument) -> Vec<DocumentChunk> {
        let normalized = document.content.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.trim().is_empty() {
            return Vec::new();
        }

        let sections = markdown_sections(&normalized);
        let mut chunks = Vec::new();
        for section in sections {
            let content = normalized[section.start..section.end].trim().to_string();
            if content.is_empty() {
                continue;
            }

            if content.len() > self.target_chars && !contains_code_fence(&content) {
                let section_document = ParsedDocument {
                    path: document.path.clone(),
                    kind: document.kind,
                    content_hash: document.content_hash.clone(),
                    content,
                };
                let fixed = FixedTextChunker::new(self.target_chars, self.overlap_chars);
                for mut chunk in fixed.chunk(&section_document) {
                    chunk.id = format!("{}#{:04}", document.path, chunks.len());
                    chunk.start_byte += section.start;
                    chunk.end_byte += section.start;
                    chunk.heading = section.heading.clone();
                    chunks.push(chunk);
                }
                continue;
            }

            chunks.push(DocumentChunk {
                id: format!("{}#{:04}", document.path, chunks.len()),
                path: document.path.clone(),
                kind: document.kind,
                content_hash: document.content_hash.clone(),
                chunk_hash: sha256_hex(content.as_bytes()),
                start_byte: section.start,
                end_byte: section.end,
                heading: section.heading,
                content,
            });
        }

        chunks
    }
}

#[derive(Debug, Clone)]
struct MarkdownSection {
    start: usize,
    end: usize,
    heading: Option<String>,
}

fn markdown_sections(text: &str) -> Vec<MarkdownSection> {
    let mut sections = Vec::new();
    let mut heading_stack: Vec<String> = Vec::new();
    let mut current_start = 0;
    let mut current_heading = None;
    let mut in_code_fence = false;

    for (line_start, line) in lines_with_offsets(text) {
        let trimmed = line.trim_end_matches('\n').trim_end();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }

        let Some((level, title)) = parse_heading(trimmed) else {
            continue;
        };

        if line_start > current_start {
            sections.push(MarkdownSection {
                start: current_start,
                end: line_start,
                heading: current_heading.clone(),
            });
        }

        heading_stack.truncate(level.saturating_sub(1));
        heading_stack.push(title.to_string());
        current_heading = Some(heading_stack.join(" > "));
        current_start = line_start;
    }

    if current_start < text.len() {
        sections.push(MarkdownSection {
            start: current_start,
            end: text.len(),
            heading: current_heading,
        });
    }

    if sections.is_empty() {
        sections.push(MarkdownSection {
            start: 0,
            end: text.len(),
            heading: None,
        });
    }

    merge_heading_only_sections(text, sections)
}

fn merge_heading_only_sections(text: &str, sections: Vec<MarkdownSection>) -> Vec<MarkdownSection> {
    let mut merged = Vec::new();
    let mut pending_start = None;

    for mut section in sections {
        if is_heading_only_section(text, &section) {
            pending_start = Some(pending_start.unwrap_or(section.start));
            continue;
        }

        if let Some(start) = pending_start.take() {
            section.start = start;
        }
        merged.push(section);
    }

    merged
}

fn is_heading_only_section(text: &str, section: &MarkdownSection) -> bool {
    let content = text[section.start..section.end].trim();
    let mut non_empty = content.lines().filter(|line| !line.trim().is_empty());
    let Some(first) = non_empty.next() else {
        return true;
    };
    parse_heading(first.trim()).is_some() && non_empty.next().is_none()
}

fn lines_with_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        lines.push((start, line));
        start += line.len();
    }
    if start < text.len() {
        lines.push((start, &text[start..]));
    }
    lines
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let title = line[hashes..].trim();
    if title.is_empty() {
        return None;
    }
    Some((hashes, title))
}

fn contains_code_fence(text: &str) -> bool {
    text.lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count()
        >= 2
}

fn normalize_text(text: &str) -> String {
    let src = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\n' && out.ends_with('.') && looks_like_url_tail(&out) {
            if matches!(chars.peek(), Some(next) if next.is_ascii_alphabetic() || *next == '/') {
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn looks_like_url_tail(text: &str) -> bool {
    let tail = text
        .rsplit_once(char::is_whitespace)
        .map(|(_, tail)| tail)
        .unwrap_or(text);
    tail.starts_with("http://") || tail.starts_with("https://")
}

fn adjust_to_boundary(text: &str, start: usize, target_end: usize) -> usize {
    let window = &text[start..target_end];
    for pattern in ["\n\n", "\n#"] {
        if let Some(offset) = window.rfind(pattern) {
            return start + offset + pattern.len();
        }
    }

    for ch in ['。', '！', '？'] {
        if let Some(offset) = window.rfind(ch) {
            return start + offset + ch.len_utf8();
        }
    }

    for ch in ['.', '!', '?'] {
        if let Some(offset) = window.rfind(ch) {
            let end = start + offset + ch.len_utf8();
            if end >= text.len() || text[end..].starts_with(char::is_whitespace) {
                return end;
            }
        }
    }

    if let Some(offset) = window.rfind(char::is_whitespace) {
        return start + offset + 1;
    }

    target_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::rag::types::{ParsedDocument, RetrieveKind};

    #[test]
    fn fixed_chunker_uses_overlap_and_stable_boundaries() {
        let document = ParsedDocument {
            path: "docs/guide.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:doc".to_string(),
            content: "Alpha sentence. Beta sentence.\n\nGamma sentence. Delta sentence."
                .to_string(),
        };
        let chunker = FixedTextChunker::new(35, 8);

        let chunks = chunker.chunk(&document);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "docs/guide.md#0000");
        assert_eq!(chunks[1].id, "docs/guide.md#0001");
        assert!(chunks[0].content.ends_with("Beta sentence."));
        assert!(chunks[1].start_byte < chunks[0].end_byte);
        assert!(chunks[1].content.contains("Gamma sentence."));
    }

    #[test]
    fn fixed_chunker_preserves_broken_url_lines() {
        let document = ParsedDocument {
            path: "docs/link.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:url".to_string(),
            content: "See https://example.\ncom/path for details.".to_string(),
        };
        let chunker = FixedTextChunker::new(1600, 160);

        let chunks = chunker.chunk(&document);

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("https://example.com/path"));
    }

    #[test]
    fn markdown_chunker_tracks_heading_metadata() {
        let document = ParsedDocument {
            path: "docs/rag.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:md".to_string(),
            content: "# RAG\n\nIntro paragraph.\n\n## Retrieval\n\nDetails paragraph.".to_string(),
        };
        let chunker = MarkdownAwareChunker::new(60, 8);

        let chunks = chunker.chunk(&document);

        assert_eq!(chunks[0].heading.as_deref(), Some("RAG"));
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.heading.as_deref() == Some("RAG > Retrieval"))
        );
    }

    #[test]
    fn markdown_chunker_keeps_code_fences_atomic_when_possible() {
        let document = ParsedDocument {
            path: "docs/code.md".to_string(),
            kind: RetrieveKind::Docs,
            content_hash: "sha256:code".to_string(),
            content: "## Example\n\n```rust\nfn searchable_symbol() {}\n```\n\nAfter.".to_string(),
        };
        let chunker = MarkdownAwareChunker::new(120, 8);

        let chunks = chunker.chunk(&document);

        let code_chunks: Vec<_> = chunks
            .iter()
            .filter(|chunk| chunk.content.contains("searchable_symbol"))
            .collect();
        assert_eq!(code_chunks.len(), 1);
        assert!(code_chunks[0].content.contains("```rust"));
        assert!(code_chunks[0].content.contains("```"));
        assert_eq!(code_chunks[0].heading.as_deref(), Some("Example"));
    }
}
