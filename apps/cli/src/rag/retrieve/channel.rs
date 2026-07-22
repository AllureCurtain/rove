use std::path::PathBuf;

use async_trait::async_trait;

use crate::rag::{Embedder, RagIndex, RetrieveKind, RetrievedChunk};

#[derive(Debug, Clone)]
pub struct RetrievalContext {
    pub workspace_root: PathBuf,
    pub original_query: String,
    pub normalized_query: String,
    pub sub_queries: Vec<String>,
    pub kind: RetrieveKind,
    pub limit: usize,
    pub path_hint: Option<String>,
}

impl RetrievalContext {
    #[cfg(test)]
    pub fn for_test(query: &str, kind: RetrieveKind, limit: usize) -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            original_query: query.to_string(),
            normalized_query: query.to_string(),
            sub_queries: vec![query.to_string()],
            kind,
            limit,
            path_hint: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchChannelResult {
    pub name: String,
    pub status: ChannelStatus,
    pub result_count: usize,
    pub duration_ms: u128,
    pub fallback_used: bool,
    pub error: Option<String>,
    pub results: Vec<RetrievedChunk>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelStatus {
    Completed,
    Failed,
    Skipped,
}

#[async_trait]
pub trait SearchChannel: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> u8;
    fn is_enabled(&self, context: &RetrievalContext) -> bool;

    async fn search(
        &self,
        context: &RetrievalContext,
        index: &RagIndex,
        embedder: &dyn Embedder,
    ) -> anyhow::Result<SearchChannelResult>;
}
