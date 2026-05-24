#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrieveKind {
    Code,
    Docs,
}

impl RetrieveKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Docs => "docs",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievedChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content: String,
    pub score: f32,
    pub source: String,
    pub heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_hash: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexManifest {
    pub schema_version: u32,
    pub workspace_root: String,
    pub embedding: EmbeddingManifest,
    pub chunking: ChunkingManifest,
    pub files: Vec<IndexedFile>,
    pub chunks: Vec<ManifestChunk>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingManifest {
    pub provider: String,
    pub model: String,
    pub dims: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkingManifest {
    pub strategy: String,
    pub target_chars: usize,
    pub overlap_chars: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexedFile {
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_count: usize,
    pub indexed_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_hash: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub heading: Option<String>,
    pub content: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct DocumentChunk {
    pub id: String,
    pub path: String,
    pub kind: RetrieveKind,
    pub content_hash: String,
    pub chunk_hash: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub heading: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub absolute_path: std::path::PathBuf,
    pub relative_path: String,
    pub kind: RetrieveKind,
}

#[derive(Debug, Clone)]
pub struct EmbeddedChunk {
    pub chunk: DocumentChunk,
    pub vector: Vec<f32>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}
