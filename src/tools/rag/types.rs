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
