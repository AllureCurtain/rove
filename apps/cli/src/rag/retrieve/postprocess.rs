use std::collections::{BTreeSet, HashMap};

use super::channel::RetrievalContext;
use crate::rag::types::{RetrievedChunk, sha256_hex};

pub trait SearchResultPostProcessor: Send + Sync {
    fn name(&self) -> &'static str;
    fn order(&self) -> u8;
    fn is_enabled(&self, context: &RetrievalContext) -> bool;

    fn process(
        &self,
        context: &RetrievalContext,
        results: Vec<RetrievedChunk>,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;
}

pub struct DeduplicationPostProcessor;

impl SearchResultPostProcessor for DeduplicationPostProcessor {
    fn name(&self) -> &'static str {
        "dedupe"
    }

    fn order(&self) -> u8 {
        1
    }

    fn is_enabled(&self, _context: &RetrievalContext) -> bool {
        true
    }

    fn process(
        &self,
        _context: &RetrievalContext,
        results: Vec<RetrievedChunk>,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        let mut order = Vec::new();
        let mut by_key: HashMap<String, RetrievedChunk> = HashMap::new();

        for result in results {
            let key = dedupe_key(&result);
            if let Some(existing) = by_key.get_mut(&key) {
                let merged_source = merge_sources(&existing.source, &result.source);
                if result.score > existing.score {
                    *existing = result;
                }
                existing.source = merged_source;
            } else {
                order.push(key.clone());
                by_key.insert(key, result);
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|key| by_key.remove(&key))
            .collect())
    }
}

pub struct ScoreNormalizationPostProcessor;

impl SearchResultPostProcessor for ScoreNormalizationPostProcessor {
    fn name(&self) -> &'static str {
        "score-normalization"
    }

    fn order(&self) -> u8 {
        2
    }

    fn is_enabled(&self, _context: &RetrievalContext) -> bool {
        true
    }

    fn process(
        &self,
        context: &RetrievalContext,
        mut results: Vec<RetrievedChunk>,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        for result in &mut results {
            result.score = normalize_score(result.score);
        }
        results.sort_by(|left, right| right.score.total_cmp(&left.score));
        results.truncate(context.limit);
        Ok(results)
    }
}

fn dedupe_key(chunk: &RetrievedChunk) -> String {
    if !chunk.id.is_empty() {
        return format!("id:{}", chunk.id);
    }
    if let Some(hash) = &chunk.chunk_hash {
        return format!("chunk:{hash}");
    }
    let normalized = chunk
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    format!("content:{}", sha256_hex(normalized.as_bytes()))
}

fn merge_sources(left: &str, right: &str) -> String {
    let mut sources = BTreeSet::new();
    for source in left.split('+').chain(right.split('+')) {
        if !source.is_empty() {
            sources.insert(source);
        }
    }
    let mut ordered = Vec::new();
    for preferred in ["vector", "lexical", "path"] {
        if sources.remove(preferred) {
            ordered.push(preferred.to_string());
        }
    }
    ordered.extend(sources.into_iter().map(ToString::to_string));
    ordered.join("+")
}

fn normalize_score(score: f32) -> f32 {
    if score <= 0.0 {
        0.0
    } else if score <= 1.0 {
        score
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::retrieve::channel::RetrievalContext;
    use crate::rag::types::{RetrieveKind, RetrievedChunk};

    fn chunk(id: &str, content: &str, score: f32, source: &str) -> RetrievedChunk {
        RetrievedChunk {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            kind: RetrieveKind::Code,
            content: content.to_string(),
            score,
            source: source.to_string(),
            heading: None,
            chunk_hash: None,
        }
    }

    #[test]
    fn dedupe_keeps_highest_score_and_merges_sources() {
        let processor = DeduplicationPostProcessor;
        let context = RetrievalContext::for_test("authentication token", RetrieveKind::Code, 5);

        let results = processor
            .process(
                &context,
                vec![
                    chunk("src/lib.rs#0000", "same content", 0.2, "vector"),
                    chunk("src/lib.rs#0000", "same content", 0.8, "lexical"),
                ],
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].score, 0.8);
        assert_eq!(results[0].source, "vector+lexical");
    }

    #[test]
    fn score_normalization_orders_mixed_channel_scores() {
        let processor = ScoreNormalizationPostProcessor;
        let context = RetrievalContext::for_test("invoice total", RetrieveKind::Code, 5);

        let results = processor
            .process(
                &context,
                vec![
                    chunk("a#0000", "invoice total exact", 4.0, "lexical"),
                    chunk("b#0000", "near vector match", 0.75, "vector"),
                ],
            )
            .unwrap();

        assert!(results[0].score <= 1.0);
        assert!(results[1].score <= 1.0);
        assert!(results[0].score >= results[1].score);
    }
}
