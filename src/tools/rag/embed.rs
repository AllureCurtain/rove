use std::sync::Arc;

use async_trait::async_trait;

use crate::models::health::ModelHealthStore;
use crate::models::traits::ModelClientId;

pub const EMBEDDING_DIMS: usize = 64;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("embedding:unknown")
    }
}

#[derive(Debug, Default)]
pub struct DeterministicEmbedder;

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(deterministic_embedding(text))
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("embedding-deterministic", "local", "deterministic-64")
    }
}

pub struct OpenAiEmbedder {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl OpenAiEmbedder {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
        });
        let response = self
            .client
            .post(format!("{}/embeddings", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("embedding request failed: {}", response.text().await?);
        }
        let json: serde_json::Value = response.json().await?;
        let values = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("embedding response missing data[0].embedding"))?;
        Ok(values
            .iter()
            .map(|value| value.as_f64().unwrap_or_default() as f32)
            .collect())
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("embedding-openai-compatible", &self.api_base, &self.model)
    }
}

pub struct RoutingEmbedder {
    candidates: Vec<Box<dyn Embedder>>,
    health: Arc<ModelHealthStore>,
}

impl RoutingEmbedder {
    pub fn new(candidates: Vec<Box<dyn Embedder>>, health: Arc<ModelHealthStore>) -> Self {
        Self { candidates, health }
    }
}

#[async_trait]
impl Embedder for RoutingEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut last_error = None;
        for candidate in &self.candidates {
            let candidate_id = candidate.client_id().to_string();
            if !self.health.allow_call(&candidate_id) {
                continue;
            }
            match candidate.embed(text).await {
                Ok(vector) => {
                    self.health.mark_success(&candidate_id);
                    return Ok(vector);
                }
                Err(err) => {
                    self.health.mark_failure(&candidate_id);
                    last_error = Some(err);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("all embedding candidates failed")))
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("embedding-routing")
    }
}

fn deterministic_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0; EMBEDDING_DIMS];
    for token in tokenize(text) {
        let idx = stable_hash(&token) % EMBEDDING_DIMS;
        vector[idx] += 1.0;
    }
    normalize(vector)
}

pub fn normalize_dims(vector: &[f32]) -> Vec<f32> {
    let mut fixed = vec![0.0; EMBEDDING_DIMS];
    for (idx, value) in vector.iter().take(EMBEDDING_DIMS).enumerate() {
        fixed[idx] = *value;
    }
    normalize(fixed)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let left = normalize_dims(left);
    let right = normalize_dims(right);
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn stable_hash(value: &str) -> usize {
    let mut hash: usize = 1469598103934665603usize;
    for byte in value.bytes() {
        hash ^= byte as usize;
        hash = hash.wrapping_mul(1099511628211usize);
    }
    hash
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::models::health::{HealthConfig, ModelHealthStore};
    use crate::models::traits::ModelClientId;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct FailingEmbedder {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Embedder for FailingEmbedder {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("embedding unavailable")
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    struct StaticEmbedder {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Embedder for StaticEmbedder {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1.0; EMBEDDING_DIMS])
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    #[tokio::test]
    async fn routing_embedder_falls_back_after_failure() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: 1,
            open_cooldown: std::time::Duration::from_secs(30),
        }));
        let embedder = RoutingEmbedder::new(
            vec![
                Box::new(FailingEmbedder {
                    id: "embedding-primary",
                    calls: primary_calls.clone(),
                }),
                Box::new(StaticEmbedder {
                    id: "embedding-fallback",
                    calls: fallback_calls.clone(),
                }),
            ],
            health,
        );

        let vector = embedder.embed("project context").await.unwrap();

        assert_eq!(vector.len(), EMBEDDING_DIMS);
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }
}
