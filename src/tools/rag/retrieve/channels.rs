use std::time::Instant;

use async_trait::async_trait;

use super::channel::{ChannelStatus, RetrievalContext, SearchChannel, SearchChannelResult};
use crate::tools::rag::RagIndex;
use crate::tools::rag::embed::{Embedder, cosine_similarity, tokenize};
use crate::tools::rag::types::{IndexManifest, RetrievedChunk};

pub struct VectorSearchChannel;

#[async_trait]
impl SearchChannel for VectorSearchChannel {
    fn name(&self) -> &'static str {
        "vector"
    }

    fn priority(&self) -> u8 {
        1
    }

    fn is_enabled(&self, _context: &RetrievalContext) -> bool {
        true
    }

    async fn search(
        &self,
        context: &RetrievalContext,
        index: &RagIndex,
        embedder: &dyn Embedder,
    ) -> anyhow::Result<SearchChannelResult> {
        let start = Instant::now();
        let query_vector = embedder.embed(&context.normalized_query).await?;
        let search_limit = (context.limit * 4).max(context.limit);
        let mut fallback_used = false;
        let mut results = match index
            .search_lancedb(context.kind, &query_vector, search_limit)
            .await
        {
            Ok(results) if !results.is_empty() => results,
            _ => {
                fallback_used = true;
                search_manifest_vector(index.load_manifest().await?, context, &query_vector)
            }
        };
        for result in &mut results {
            result.source = "vector".to_string();
        }
        results.truncate(search_limit);
        Ok(SearchChannelResult {
            name: self.name().to_string(),
            status: ChannelStatus::Completed,
            result_count: results.len(),
            duration_ms: start.elapsed().as_millis(),
            fallback_used,
            error: None,
            results,
        })
    }
}

pub struct LexicalSearchChannel;

#[async_trait]
impl SearchChannel for LexicalSearchChannel {
    fn name(&self) -> &'static str {
        "lexical"
    }

    fn priority(&self) -> u8 {
        2
    }

    fn is_enabled(&self, _context: &RetrievalContext) -> bool {
        true
    }

    async fn search(
        &self,
        context: &RetrievalContext,
        index: &RagIndex,
        _embedder: &dyn Embedder,
    ) -> anyhow::Result<SearchChannelResult> {
        let start = Instant::now();
        let mut results = search_manifest_lexical(index.load_manifest().await?, context, false);
        results.truncate((context.limit * 4).max(context.limit));
        Ok(SearchChannelResult {
            name: self.name().to_string(),
            status: ChannelStatus::Completed,
            result_count: results.len(),
            duration_ms: start.elapsed().as_millis(),
            fallback_used: true,
            error: None,
            results,
        })
    }
}

pub struct PathScopedSearchChannel;

#[async_trait]
impl SearchChannel for PathScopedSearchChannel {
    fn name(&self) -> &'static str {
        "path"
    }

    fn priority(&self) -> u8 {
        0
    }

    fn is_enabled(&self, context: &RetrievalContext) -> bool {
        context.path_hint.is_some()
    }

    async fn search(
        &self,
        context: &RetrievalContext,
        index: &RagIndex,
        _embedder: &dyn Embedder,
    ) -> anyhow::Result<SearchChannelResult> {
        let start = Instant::now();
        let mut results = search_manifest_lexical(index.load_manifest().await?, context, true);
        results.truncate((context.limit * 4).max(context.limit));
        Ok(SearchChannelResult {
            name: self.name().to_string(),
            status: ChannelStatus::Completed,
            result_count: results.len(),
            duration_ms: start.elapsed().as_millis(),
            fallback_used: true,
            error: None,
            results,
        })
    }
}

fn search_manifest_vector(
    manifest: Option<IndexManifest>,
    context: &RetrievalContext,
    query_vector: &[f32],
) -> Vec<RetrievedChunk> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    let mut results: Vec<_> = manifest
        .chunks
        .into_iter()
        .filter(|chunk| chunk.kind == context.kind)
        .map(|chunk| RetrievedChunk {
            id: chunk.id,
            path: chunk.path,
            kind: chunk.kind,
            content: chunk.content,
            score: cosine_similarity(query_vector, &chunk.vector),
            source: "vector".to_string(),
            heading: chunk.heading,
            chunk_hash: Some(chunk.chunk_hash),
        })
        .collect();
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    results
}

fn search_manifest_lexical(
    manifest: Option<IndexManifest>,
    context: &RetrievalContext,
    path_scoped: bool,
) -> Vec<RetrievedChunk> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    let path_hint = context.path_hint.as_deref();
    let mut results: Vec<_> = manifest
        .chunks
        .into_iter()
        .filter(|chunk| chunk.kind == context.kind)
        .filter(|chunk| {
            !path_scoped
                || path_hint
                    .map(|hint| chunk.path.contains(hint) || hint.contains(&chunk.path))
                    .unwrap_or(false)
        })
        .filter_map(|chunk| {
            let mut score = lexical_score(&context.normalized_query, &chunk.content);
            if let Some(hint) = path_hint {
                if chunk.path.contains(hint) || hint.contains(&chunk.path) {
                    score += 2.0;
                }
            }
            if score <= 0.0 {
                return None;
            }
            Some(RetrievedChunk {
                id: chunk.id,
                path: chunk.path,
                kind: chunk.kind,
                content: chunk.content,
                score,
                source: if path_scoped { "path" } else { "lexical" }.to_string(),
                heading: chunk.heading,
                chunk_hash: Some(chunk.chunk_hash),
            })
        })
        .collect();
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    results
}

fn lexical_score(query: &str, content: &str) -> f32 {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let content_tokens = tokenize(content);
    let matches = query_tokens
        .iter()
        .filter(|token| content_tokens.contains(token))
        .count();
    matches as f32 / query_tokens.len() as f32
}
