use super::channel::{RetrievalContext, SearchChannel, SearchChannelResult};
use super::channels::{LexicalSearchChannel, PathScopedSearchChannel, VectorSearchChannel};
use super::postprocess::{
    DeduplicationPostProcessor, NoopRerankPostProcessor, ScoreNormalizationPostProcessor,
    SearchResultPostProcessor,
};
use crate::tools::rag::RagIndex;
use crate::tools::rag::embed::Embedder;
use crate::tools::rag::rewrite::{DeterministicQueryRewriteService, QueryRewriteService};
use crate::tools::rag::types::{RetrieveKind, RetrievedChunk};

pub struct RetrievalPipeline<'a> {
    index: &'a RagIndex,
    embedder: &'a dyn Embedder,
}

impl<'a> RetrievalPipeline<'a> {
    pub fn new(index: &'a RagIndex, embedder: &'a dyn Embedder) -> Self {
        Self { index, embedder }
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
            Box::new(NoopRerankPostProcessor),
        ];
        postprocessors.sort_by_key(|processor| processor.order());
        for processor in postprocessors {
            if processor.is_enabled(&context) {
                results = processor.process(&context, results)?;
            }
        }

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
