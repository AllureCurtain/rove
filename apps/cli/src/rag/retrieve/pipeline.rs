use super::channel::{RetrievalContext, SearchChannel, SearchChannelResult};
use super::channels::{LexicalSearchChannel, PathScopedSearchChannel, VectorSearchChannel};
use super::postprocess::{
    DeduplicationPostProcessor, ScoreNormalizationPostProcessor, SearchResultPostProcessor,
};
use crate::rag::RagIndex;
use crate::rag::embed::Embedder;
use crate::rag::rerank::{NoopReranker, Reranker};
use crate::rag::rewrite::{DeterministicQueryRewriteService, QueryRewriteService};
use crate::rag::types::{RetrieveKind, RetrievedChunk};

pub struct RetrievalPipeline<'a> {
    index: &'a RagIndex,
    embedder: &'a dyn Embedder,
    reranker: &'a dyn Reranker,
}

impl<'a> RetrievalPipeline<'a> {
    pub fn new(index: &'a RagIndex, embedder: &'a dyn Embedder) -> Self {
        Self {
            index,
            embedder,
            reranker: &NoopReranker,
        }
    }

    pub fn with_reranker(
        index: &'a RagIndex,
        embedder: &'a dyn Embedder,
        reranker: &'a dyn Reranker,
    ) -> Self {
        Self {
            index,
            embedder,
            reranker,
        }
    }

    pub async fn retrieve(
        &self,
        kind: RetrieveKind,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        Ok(self.run(kind, query, limit).await?.results)
    }

    pub async fn run(
        &self,
        kind: RetrieveKind,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<RetrievalPipelineOutput> {
        let rewrite = DeterministicQueryRewriteService.rewrite(query);
        let context = RetrievalContext {
            workspace_root: self.index.workspace_root().to_path_buf(),
            original_query: rewrite.original_query,
            normalized_query: rewrite.normalized_query,
            sub_queries: rewrite.sub_queries,
            kind,
            limit,
            path_hint: rewrite.path_hint,
        };

        let channels: Vec<Box<dyn SearchChannel>> = vec![
            Box::new(PathScopedSearchChannel),
            Box::new(VectorSearchChannel),
            Box::new(LexicalSearchChannel),
        ];
        let mut channel_results = Vec::new();
        for channel in channels
            .into_iter()
            .filter(|channel| channel.is_enabled(&context))
        {
            channel_results.push(channel.search(&context, self.index, self.embedder).await?);
        }

        let mut results: Vec<_> = channel_results
            .iter()
            .flat_map(|channel| channel.results.clone())
            .collect();
        let mut postprocessors: Vec<Box<dyn SearchResultPostProcessor>> = vec![
            Box::new(DeduplicationPostProcessor),
            Box::new(ScoreNormalizationPostProcessor),
        ];
        postprocessors.sort_by_key(|processor| processor.order());
        for processor in postprocessors {
            if processor.is_enabled(&context) {
                results = processor.process(&context, results)?;
            }
        }
        results = self
            .reranker
            .rerank(&context.normalized_query, results, limit)
            .await?;

        Ok(RetrievalPipelineOutput {
            context,
            channels: channel_results,
            results,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RetrievalPipelineOutput {
    pub context: RetrievalContext,
    pub channels: Vec<SearchChannelResult>,
    pub results: Vec<RetrievedChunk>,
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::RetrievalPipeline;
    use crate::rag::embed::DeterministicEmbedder;
    use crate::rag::rerank::Reranker;
    use crate::rag::{RagIndex, RetrieveKind, RetrievedChunk};
    use rove_models::ModelClientId;

    struct ReverseReranker;

    #[async_trait]
    impl Reranker for ReverseReranker {
        async fn rerank(
            &self,
            _query: &str,
            mut candidates: Vec<RetrievedChunk>,
            top_n: usize,
        ) -> anyhow::Result<Vec<RetrievedChunk>> {
            candidates.sort_by(|left, right| left.path.cmp(&right.path));
            candidates.reverse();
            candidates.truncate(top_n);
            Ok(candidates)
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque("rerank-reverse")
        }
    }

    #[tokio::test]
    async fn retrieval_pipeline_uses_configured_reranker() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
        std::fs::write(
            tmp.path().join("docs").join("a.md"),
            "shared retrieval query alpha",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("docs").join("z.md"),
            "shared retrieval query zulu",
        )
        .unwrap();

        let index = RagIndex::new(tmp.path().to_path_buf());
        let embedder = DeterministicEmbedder;
        index.ingest_workspace(&embedder).await.unwrap();

        let output = RetrievalPipeline::with_reranker(&index, &embedder, &ReverseReranker)
            .run(RetrieveKind::Docs, "shared retrieval query", 2)
            .await
            .unwrap();

        assert_eq!(output.results[0].path, "docs/z.md");
    }
}
