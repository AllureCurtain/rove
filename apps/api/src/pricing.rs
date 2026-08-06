//! Versioned bundled model pricing for product cost estimates.
//!
//! Absence of a price means cost is reported as unavailable rather than
//! fabricated. Fake/local models are an explicit local-zero class so a `$0`
//! total is never confused with a missing price.

use rove_models::Usage;

/// Bump only when the bundled rate table changes. Persisted run snapshots keep
/// the version that was active when the product turn was bound.
pub const BUNDLED_PRICING_SOURCE: &str = "bundled";
pub const BUNDLED_PRICING_VERSION: &str = "2026-08-05.1";
pub const BUNDLED_PRICING_CURRENCY: &str = "USD";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingAvailability {
    /// Known commercial rates. Cost may be computed.
    Priced,
    /// Explicit local/fake zero-cost provider. Cost is `$0`, not unavailable.
    LocalZero,
    /// No trusted rate. Cost must stay unavailable.
    Unpriced,
}

impl PricingAvailability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Priced => "priced",
            Self::LocalZero => "local_zero",
            Self::Unpriced => "unpriced",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PricingSnapshot {
    pub source: String,
    pub version: String,
    pub currency: String,
    pub availability: PricingAvailability,
    pub per_mtok_prompt: Option<f64>,
    pub per_mtok_completion: Option<f64>,
    pub per_mtok_cache_read: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostBreakdown {
    pub currency: String,
    pub prompt_usd: f64,
    pub completion_usd: f64,
    pub cache_read_usd: f64,
    pub total_usd: f64,
    pub availability: PricingAvailability,
}

impl PricingSnapshot {
    pub fn bundled_for_model(model_id: &str) -> Self {
        let normalized = model_id.to_ascii_lowercase();
        let rates = match () {
            _ if normalized.contains("fake") => {
                Some((PricingAvailability::LocalZero, 0.0, 0.0, 0.0))
            }
            _ if normalized.contains("claude-opus") => {
                Some((PricingAvailability::Priced, 15.0, 75.0, 1.50))
            }
            _ if normalized.contains("claude-sonnet") => {
                Some((PricingAvailability::Priced, 3.0, 15.0, 0.30))
            }
            _ if normalized.contains("claude-haiku") => {
                Some((PricingAvailability::Priced, 0.80, 4.0, 0.08))
            }
            _ if normalized.contains("gpt-4o") => {
                Some((PricingAvailability::Priced, 2.50, 10.0, 1.25))
            }
            _ if normalized.contains("gpt-4.1") => {
                Some((PricingAvailability::Priced, 2.00, 8.00, 0.50))
            }
            _ if normalized.contains("gpt-4") => {
                Some((PricingAvailability::Priced, 30.0, 60.0, 15.0))
            }
            _ if normalized.contains("gpt-3.5") => {
                Some((PricingAvailability::Priced, 0.50, 1.50, 0.25))
            }
            _ if normalized.starts_with("o1") => {
                Some((PricingAvailability::Priced, 15.0, 60.0, 7.50))
            }
            _ if normalized.starts_with("o3") => {
                Some((PricingAvailability::Priced, 2.00, 8.00, 1.00))
            }
            _ => None,
        };

        match rates {
            Some((availability, prompt, completion, cache_read)) => Self {
                source: BUNDLED_PRICING_SOURCE.to_string(),
                version: BUNDLED_PRICING_VERSION.to_string(),
                currency: BUNDLED_PRICING_CURRENCY.to_string(),
                availability,
                per_mtok_prompt: Some(prompt),
                per_mtok_completion: Some(completion),
                per_mtok_cache_read: Some(cache_read),
            },
            None => Self {
                source: BUNDLED_PRICING_SOURCE.to_string(),
                version: BUNDLED_PRICING_VERSION.to_string(),
                currency: BUNDLED_PRICING_CURRENCY.to_string(),
                availability: PricingAvailability::Unpriced,
                per_mtok_prompt: None,
                per_mtok_completion: None,
                per_mtok_cache_read: None,
            },
        }
    }

    pub fn cost_for(&self, usage: &Usage) -> Option<CostBreakdown> {
        match self.availability {
            PricingAvailability::Unpriced => None,
            PricingAvailability::LocalZero | PricingAvailability::Priced => {
                let prompt = self.per_mtok_prompt.unwrap_or(0.0);
                let completion = self.per_mtok_completion.unwrap_or(0.0);
                let cache_read = self.per_mtok_cache_read.unwrap_or(0.0);
                // Provider usage reports expose cached input as a subset of
                // prompt/input tokens. Charge that subset at the cache-read
                // rate instead of charging it a second time at the full input
                // rate.
                let uncached_prompt_tokens =
                    usage.prompt_tokens.saturating_sub(usage.cached_tokens);
                let prompt_usd = (f64::from(uncached_prompt_tokens) / 1_000_000.0) * prompt;
                let completion_usd =
                    (f64::from(usage.completion_tokens) / 1_000_000.0) * completion;
                let cache_read_usd = (f64::from(usage.cached_tokens) / 1_000_000.0) * cache_read;
                Some(CostBreakdown {
                    currency: self.currency.to_string(),
                    prompt_usd,
                    completion_usd,
                    cache_read_usd,
                    total_usd: prompt_usd + completion_usd + cache_read_usd,
                    availability: self.availability,
                })
            }
        }
    }
}

pub fn round_usd(value: f64) -> f64 {
    // Preserve micro-dollar precision so small commercial calls do not look
    // like explicit zero-cost runs. Presentation code may choose a compact
    // display without changing the machine-readable evidence.
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// Bundled context hard limits are metadata, not provider inventory claims.
/// Unknown and custom model identifiers intentionally stay unavailable.
pub fn bundled_context_window(model_id: &str) -> Option<u64> {
    let normalized = model_id.to_ascii_lowercase();
    match () {
        _ if normalized.contains("claude-") => Some(200_000),
        _ if normalized.contains("gpt-4.1") => Some(1_047_576),
        _ if normalized.contains("gpt-4o") => Some(128_000),
        _ if normalized.contains("gpt-4-turbo") => Some(128_000),
        _ if normalized.contains("gpt-4") => Some(8_192),
        _ if normalized.contains("gpt-3.5") => Some(16_385),
        _ if normalized.starts_with("o1") || normalized.starts_with("o3") => Some(200_000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_computes_cost() {
        let usage = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 500_000,
            total_tokens: 1_500_000,
            cached_tokens: 200_000,
        };
        let snapshot = PricingSnapshot::bundled_for_model("claude-sonnet-4-20250514");
        let cost = snapshot.cost_for(&usage).expect("priced");
        assert_eq!(snapshot.availability, PricingAvailability::Priced);
        assert_eq!(snapshot.version, BUNDLED_PRICING_VERSION);
        assert!((cost.total_usd - 9.96).abs() < 0.000_001);
    }

    #[test]
    fn unknown_model_is_unpriced() {
        let snapshot = PricingSnapshot::bundled_for_model("totally-custom-model");
        assert_eq!(snapshot.availability, PricingAvailability::Unpriced);
        assert!(snapshot.cost_for(&Usage::default()).is_none());
    }

    #[test]
    fn fake_model_is_explicit_local_zero() {
        let usage = Usage {
            prompt_tokens: 1000,
            completion_tokens: 1000,
            total_tokens: 2000,
            cached_tokens: 0,
        };
        let snapshot = PricingSnapshot::bundled_for_model("fake-raw");
        assert_eq!(snapshot.availability, PricingAvailability::LocalZero);
        let cost = snapshot.cost_for(&usage).expect("local zero");
        assert_eq!(cost.total_usd, 0.0);
    }

    #[test]
    fn cached_prompt_tokens_are_not_double_charged() {
        let usage = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 0,
            total_tokens: 1_000_000,
            cached_tokens: 250_000,
        };
        let snapshot = PricingSnapshot::bundled_for_model("gpt-4o");
        let cost = snapshot.cost_for(&usage).expect("priced");
        assert!((cost.prompt_usd - 1.875).abs() < f64::EPSILON);
        assert!((cost.cache_read_usd - 0.3125).abs() < f64::EPSILON);
        assert!((cost.total_usd - 2.1875).abs() < f64::EPSILON);
    }

    #[test]
    fn small_nonzero_cost_survives_machine_readable_rounding() {
        assert_eq!(round_usd(0.000_123_49), 0.000_123);
        assert_ne!(round_usd(0.004), 0.0);
    }
}
