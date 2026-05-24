use async_trait::async_trait;

pub const EMBEDDING_DIMS: usize = 64;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

#[derive(Debug, Default)]
pub struct DeterministicEmbedder;

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(deterministic_embedding(text))
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
