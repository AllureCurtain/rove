use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::rag::types::RetrievedChunk;
use rove_models::ModelClientId;
use rove_models::health::ModelHealthStore;

#[async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;

    fn client_id(&self) -> ModelClientId;
}

#[derive(Debug, Default)]
pub struct NoopReranker;

#[async_trait]
impl Reranker for NoopReranker {
    async fn rerank(
        &self,
        _query: &str,
        mut candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        candidates.truncate(top_n);
        Ok(candidates)
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("rerank-noop")
    }
}

pub struct DashScopeReranker {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl DashScopeReranker {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self::with_timeout(api_base, api_key, model, Duration::from_secs(30))
    }

    pub fn with_timeout(
        api_base: String,
        api_key: String,
        model: String,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
            api_key,
            model,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/services/rerank/text-rerank/text-rerank", self.api_base)
    }
}

#[async_trait]
impl Reranker for DashScopeReranker {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        if candidates.is_empty() || top_n == 0 {
            return Ok(Vec::new());
        }
        let documents = candidates
            .iter()
            .map(|candidate| candidate.content.as_str())
            .collect::<Vec<_>>();
        let body = serde_json::json!({
            "model": self.model,
            "input": {
                "query": query,
                "documents": documents,
            },
            "parameters": {
                "top_n": top_n,
                "return_documents": true,
            },
        });
        let response = self
            .client
            .post(self.endpoint())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("rerank request failed: {}", response.text().await?);
        }
        let json: serde_json::Value = response.json().await?;
        map_dashscope_results(candidates, &json, top_n)
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("rerank-dashscope", &self.api_base, &self.model)
    }
}

pub struct RoutingReranker {
    candidates: Vec<Box<dyn Reranker>>,
    health: Arc<ModelHealthStore>,
}

impl RoutingReranker {
    pub fn new(candidates: Vec<Box<dyn Reranker>>, health: Arc<ModelHealthStore>) -> Self {
        Self { candidates, health }
    }
}

#[async_trait]
impl Reranker for RoutingReranker {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        let mut last_error = None;
        for candidate in &self.candidates {
            let candidate_id = candidate.client_id().to_string();
            if !self.health.allow_call(&candidate_id) {
                continue;
            }
            match candidate.rerank(query, candidates.clone(), top_n).await {
                Ok(results) => {
                    self.health.mark_success(&candidate_id);
                    return Ok(results);
                }
                Err(err) => {
                    self.health.mark_failure(&candidate_id);
                    last_error = Some(err);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("all rerank candidates failed")))
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("rerank-routing")
    }
}

fn map_dashscope_results(
    candidates: Vec<RetrievedChunk>,
    response: &serde_json::Value,
    top_n: usize,
) -> anyhow::Result<Vec<RetrievedChunk>> {
    let results = response
        .pointer("/output/results")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow::anyhow!("rerank response missing output.results"))?;
    let mut used_indexes = HashSet::new();
    let mut reranked = Vec::new();

    for result in results {
        let Some(index) = result.get("index").and_then(|value| value.as_u64()) else {
            continue;
        };
        let index = index as usize;
        if index >= candidates.len() || !used_indexes.insert(index) {
            continue;
        }
        let mut candidate = candidates[index].clone();
        if let Some(score) = result
            .get("relevance_score")
            .and_then(|value| value.as_f64())
        {
            candidate.score = score as f32;
        }
        reranked.push(candidate);
        if reranked.len() >= top_n {
            return Ok(reranked);
        }
    }

    for (index, candidate) in candidates.into_iter().enumerate() {
        if used_indexes.contains(&index) {
            continue;
        }
        reranked.push(candidate);
        if reranked.len() >= top_n {
            break;
        }
    }

    Ok(reranked)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::rag::types::{RetrieveKind, RetrievedChunk};
    use rove_app_bootstrap::AppConfig;
    use rove_models::ModelClientId;
    use rove_models::health::{HealthConfig, ModelHealthStore};

    use super::{
        DashScopeReranker, NoopReranker, Reranker, RoutingReranker, map_dashscope_results,
    };

    #[tokio::test]
    async fn noop_reranker_truncates_and_preserves_order() {
        let chunks = vec![chunk("a", 0.9), chunk("b", 0.8), chunk("c", 0.7)];
        let reranked = NoopReranker
            .rerank("query", chunks.clone(), 2)
            .await
            .unwrap();

        assert_eq!(reranked.len(), 2);
        assert_eq!(reranked[0].id, "a");
        assert_eq!(reranked[1].id, "b");
        assert_eq!(NoopReranker.client_id().to_string(), "rerank-noop");
    }

    #[test]
    fn rerank_maps_returned_indexes_to_original_chunks() {
        let chunks = vec![chunk("a", 0.2), chunk("b", 0.3), chunk("c", 0.4)];
        let response = serde_json::json!({
            "output": {
                "results": [
                    { "index": 2, "relevance_score": 0.99 },
                    { "index": 0, "relevance_score": 0.77 }
                ]
            }
        });

        let mapped = map_dashscope_results(chunks, &response, 3).unwrap();

        assert_eq!(mapped[0].id, "c");
        assert_eq!(mapped[0].score, 0.99);
        assert_eq!(mapped[1].id, "a");
        assert_eq!(mapped[1].score, 0.77);
        assert_eq!(mapped[2].id, "b");
        assert_eq!(mapped[2].score, 0.3);
    }

    #[test]
    fn rerank_ignores_invalid_or_duplicate_indexes_and_preserves_missing_originals() {
        let chunks = vec![chunk("a", 0.2), chunk("b", 0.3), chunk("c", 0.4)];
        let response = serde_json::json!({
            "output": {
                "results": [
                    { "index": 9, "relevance_score": 1.0 },
                    { "index": 1, "relevance_score": 0.8 },
                    { "index": 1, "relevance_score": 0.7 }
                ]
            }
        });

        let mapped = map_dashscope_results(chunks, &response, 3).unwrap();

        assert_eq!(
            mapped
                .iter()
                .map(|chunk| chunk.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert_eq!(mapped[0].score, 0.8);
    }

    #[test]
    fn build_rag_reranker_requires_key_when_fallback_disabled() {
        let mut config = AppConfig::default();
        config.rag.rerank_provider = Some("dashscope".to_string());
        config.rag.rerank_model = Some("qwen3-rerank".to_string());
        config.rag.rerank_api_key = None;
        config.rag.fallback_to_deterministic = false;

        let err = match crate::cli::index::build_rag_reranker(&config) {
            Ok(_) => panic!("expected missing rerank key to fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("rag.rerank_api_key is required"));
    }

    #[test]
    fn build_rag_reranker_falls_back_to_noop_when_key_missing_and_fallback_enabled() {
        let mut config = AppConfig::default();
        config.rag.rerank_provider = Some("dashscope".to_string());
        config.rag.rerank_model = Some("qwen3-rerank".to_string());
        config.rag.rerank_api_key = None;
        config.rag.fallback_to_deterministic = true;

        let reranker = crate::cli::index::build_rag_reranker(&config).unwrap();

        assert_eq!(reranker.client_id().to_string(), "rerank-noop");
    }

    #[tokio::test]
    async fn routing_reranker_falls_back_after_remote_failure() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(30),
        }));
        let candidates: Vec<Box<dyn Reranker>> = vec![
            Box::new(FailingReranker {
                id: "rerank-primary",
                calls: primary_calls.clone(),
            }),
            Box::new(CountingReranker {
                id: "rerank-fallback",
                calls: fallback_calls.clone(),
            }),
        ];
        let reranker = RoutingReranker::new(candidates, health);

        let reranked = reranker
            .rerank("query", vec![chunk("a", 0.5)], 1)
            .await
            .unwrap();

        assert_eq!(reranked[0].id, "a");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn routing_reranker_skips_open_remote_target() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(30),
        }));
        health.mark_failure("rerank-primary");
        let candidates: Vec<Box<dyn Reranker>> = vec![
            Box::new(CountingReranker {
                id: "rerank-primary",
                calls: primary_calls.clone(),
            }),
            Box::new(CountingReranker {
                id: "rerank-fallback",
                calls: fallback_calls.clone(),
            }),
        ];
        let reranker = RoutingReranker::new(candidates, health);

        let reranked = reranker
            .rerank("query", vec![chunk("a", 0.5)], 1)
            .await
            .unwrap();

        assert_eq!(reranked[0].id, "a");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dashscope_reranker_reorders_with_remote_response() {
        let server = test_server(serde_json::json!({
            "output": {
                "results": [
                    { "index": 1, "relevance_score": 0.95 },
                    { "index": 0, "relevance_score": 0.42 }
                ]
            }
        }))
        .await;
        let reranker = DashScopeReranker::new(
            server.base_url,
            "secret".to_string(),
            "qwen3-rerank".to_string(),
        );

        let reranked = reranker
            .rerank("query", vec![chunk("a", 0.2), chunk("b", 0.3)], 2)
            .await
            .unwrap();

        assert_eq!(reranked[0].id, "b");
        assert_eq!(reranked[0].score, 0.95);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }

    struct FailingReranker {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Reranker for FailingReranker {
        async fn rerank(
            &self,
            _query: &str,
            _candidates: Vec<RetrievedChunk>,
            _top_n: usize,
        ) -> anyhow::Result<Vec<RetrievedChunk>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("rerank unavailable")
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    struct CountingReranker {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Reranker for CountingReranker {
        async fn rerank(
            &self,
            _query: &str,
            mut candidates: Vec<RetrievedChunk>,
            top_n: usize,
        ) -> anyhow::Result<Vec<RetrievedChunk>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            candidates.truncate(top_n);
            Ok(candidates)
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    struct TestServer {
        base_url: String,
        requests: Arc<AtomicUsize>,
    }

    async fn test_server(response: serde_json::Value) -> TestServer {
        use axum::Router;
        use axum::routing::post;

        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_handler = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/services/rerank/text-rerank/text-rerank",
            post(move || {
                let requests = requests_for_handler.clone();
                let response = response.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    axum::Json(response)
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        TestServer {
            base_url: format!("http://{addr}"),
            requests,
        }
    }

    fn chunk(id: &str, score: f32) -> RetrievedChunk {
        RetrievedChunk {
            id: id.to_string(),
            path: format!("{id}.md"),
            kind: RetrieveKind::Docs,
            content: format!("content {id}"),
            score,
            source: "test".to_string(),
            heading: None,
            chunk_hash: None,
        }
    }
}
