//! Model-call retry budget for a single model turn.
//!
//! The routing wrapper (`rove_models::routing`) retries *below* this budget and
//! only when a fallback target is configured. This budget lives at the run
//! loop's model-turn boundary, so it also covers a directly assembled provider
//! that the wrapper never wraps.
//!
//! Two rules shape the behavior:
//!
//! - Rate limiting and transient transport failures spend from **independent**
//!   attempt budgets, so a throttled provider cannot consume the transport
//!   budget (or the other way round).
//! - A retry is only allowed while the turn has produced **no output**. Once
//!   text has been streamed, a failure is terminal for the turn: a retry would
//!   re-generate from scratch and could duplicate what the user already saw.
//!
//! Every wait is cancellable, and nothing here reads or reports provider
//! payloads: [`ProviderRetryPolicy::reason`] is a fixed whitelist.

use std::time::{SystemTime, UNIX_EPOCH};

use rove_models::ModelError;

/// Failure classes that carry an independent attempt budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// Provider throttling. An explicit `retry_after_ms` is honoured.
    RateLimited,
    /// Connection and transport failures observed before any output.
    Transient,
}

/// Operator-tunable model-call retry budget.
///
/// Defaults are conservative and bounded: at most five extra attempts for
/// throttling and three for transport failures, with a 2s base delay that never
/// exceeds 30s. `max_attempts == 1` in both classes disables retry entirely and
/// restores the pre-recovery behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRetryPolicy {
    /// Total attempts allowed for a rate-limited call, including the first.
    pub rate_limit_max_attempts: u32,
    /// Total attempts allowed for a transient failure, including the first.
    pub transient_max_attempts: u32,
    /// First backoff delay; each further retry doubles it.
    pub backoff_base_ms: u64,
    /// Ceiling for one backoff delay, and for a provider `retry_after_ms`.
    pub backoff_max_ms: u64,
    /// Fraction of a delay that may be shaved off at random, `0.0..=1.0`.
    /// `0.0` makes the delay exactly deterministic.
    pub jitter_ratio: f64,
}

impl Default for ProviderRetryPolicy {
    fn default() -> Self {
        Self {
            rate_limit_max_attempts: 6,
            transient_max_attempts: 4,
            backoff_base_ms: 2_000,
            backoff_max_ms: 30_000,
            jitter_ratio: 0.2,
        }
    }
}

impl ProviderRetryPolicy {
    /// A budget that never retries, i.e. the behavior before this budget
    /// existed. Operators reach it through `max_attempts = 1` as well.
    pub fn disabled() -> Self {
        Self {
            rate_limit_max_attempts: 1,
            transient_max_attempts: 1,
            ..Self::default()
        }
    }

    /// Whether any failure class may spend a retry.
    pub fn is_enabled(&self) -> bool {
        self.max_attempts(RetryClass::RateLimited) > 1
            || self.max_attempts(RetryClass::Transient) > 1
    }

    /// Total attempts allowed in one class, counting the first call.
    pub fn max_attempts(&self, class: RetryClass) -> u32 {
        match class {
            RetryClass::RateLimited => self.rate_limit_max_attempts.max(1),
            RetryClass::Transient => self.transient_max_attempts.max(1),
        }
    }

    /// The delay before `attempt`, which is 1-based: the first call is attempt
    /// 1, so a retry is attempt 2 or later.
    ///
    /// The class does not change the curve; it selects the budget and decides
    /// whether an explicit `retry_after_ms` is available.
    pub fn delay_ms(&self, attempt: u32, retry_after_ms: Option<u64>) -> u64 {
        let ceiling = self.backoff_max_ms.max(1);
        if let Some(retry_after) = retry_after_ms {
            // An explicit provider instruction is used as given (clamped to the
            // ceiling) and never jittered: the provider asked for that wait.
            return retry_after.min(ceiling);
        }
        let steps = attempt.saturating_sub(2).min(16);
        let base = self.backoff_base_ms.max(1);
        let exponential = base.saturating_mul(1u64 << steps).min(ceiling);
        self.apply_jitter(exponential)
    }

    /// A safe retry reason: the failure class and its typed error code only.
    ///
    /// Provider messages, request bodies, and headers never reach this string,
    /// so it is safe for the event stream, the trace, and the report.
    pub fn reason(&self, error: &ModelError) -> String {
        match retry_class(error) {
            Some(RetryClass::RateLimited) => "rate_limited".to_string(),
            Some(RetryClass::Transient) => format!("transient:{}", error.error_code()),
            None => "not_retryable".to_string(),
        }
    }

    fn apply_jitter(&self, delay_ms: u64) -> u64 {
        let ratio = self.jitter_ratio.clamp(0.0, 1.0);
        if ratio <= 0.0 || delay_ms <= 1 {
            return delay_ms;
        }
        let floor = delay_ms as f64 * (1.0 - ratio);
        let span = delay_ms as f64 * ratio;
        let jittered = floor + span * pseudo_random_unit();
        jittered.round().clamp(1.0, delay_ms as f64) as u64
    }
}

/// Classify a retryable model error into its independent budget.
///
/// Returns `None` for errors the runtime must not retry, which keeps the
/// classification next to the budget instead of at the call site.
pub fn retry_class(error: &ModelError) -> Option<RetryClass> {
    match error {
        ModelError::RateLimited { .. } => Some(RetryClass::RateLimited),
        ModelError::RequestFailed(_) | ModelError::StreamInterrupted(_) => {
            Some(RetryClass::Transient)
        }
        _ => None,
    }
}

/// The provider's explicit wait, when the failure carries one.
pub fn retry_after_ms(error: &ModelError) -> Option<u64> {
    match error {
        ModelError::RateLimited { retry_after_ms } => Some(*retry_after_ms),
        _ => None,
    }
}

/// A cheap `[0, 1)` sample, so jitter needs no RNG dependency and no seeded
/// state. Only used to spread retries out, never for anything security-related.
fn pseudo_random_unit() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() as u64 ^ elapsed.as_secs())
        .unwrap_or(0);
    let mut state = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    (state >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::{ProviderRetryPolicy, RetryClass, retry_after_ms, retry_class};
    use rove_models::ModelError;

    fn deterministic() -> ProviderRetryPolicy {
        ProviderRetryPolicy {
            jitter_ratio: 0.0,
            ..ProviderRetryPolicy::default()
        }
    }

    #[test]
    fn defaults_match_the_documented_budget() {
        let policy = ProviderRetryPolicy::default();
        assert_eq!(policy.rate_limit_max_attempts, 6);
        assert_eq!(policy.transient_max_attempts, 4);
        assert_eq!(policy.backoff_base_ms, 2_000);
        assert_eq!(policy.backoff_max_ms, 30_000);
        assert!(policy.is_enabled());
        assert!(!ProviderRetryPolicy::disabled().is_enabled());
    }

    #[test]
    fn classes_spend_from_independent_budgets() {
        let policy = deterministic();
        assert_eq!(policy.max_attempts(RetryClass::RateLimited), 6);
        assert_eq!(policy.max_attempts(RetryClass::Transient), 4);
        let single = ProviderRetryPolicy {
            rate_limit_max_attempts: 0,
            transient_max_attempts: 0,
            ..ProviderRetryPolicy::default()
        };
        assert_eq!(single.max_attempts(RetryClass::RateLimited), 1);
        assert_eq!(single.max_attempts(RetryClass::Transient), 1);
        assert!(!single.is_enabled());
    }

    #[test]
    fn delay_doubles_from_the_base_and_stops_at_the_ceiling() {
        let policy = deterministic();
        let delays: Vec<u64> = (2..=9)
            .map(|attempt| policy.delay_ms(attempt, None))
            .collect();
        assert_eq!(
            delays,
            vec![2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000, 30_000]
        );
    }

    #[test]
    fn provider_retry_after_wins_and_is_clamped() {
        let policy = deterministic();
        assert_eq!(policy.delay_ms(2, Some(750)), 750);
        assert_eq!(policy.delay_ms(5, Some(120_000)), 30_000);
        // An explicit wait is not jittered, even when jitter is configured.
        let jittered = ProviderRetryPolicy::default();
        assert_eq!(jittered.delay_ms(2, Some(1_500)), 1_500);
    }

    #[test]
    fn jitter_only_shaves_the_delay_and_stays_bounded() {
        let policy = ProviderRetryPolicy::default();
        for _ in 0..64 {
            // Attempt 3 is the second retry: the 2s base doubled once.
            let delay = policy.delay_ms(3, None);
            assert!(
                (3_200..=4_000).contains(&delay),
                "jittered delay out of range: {delay}"
            );
        }
    }

    #[test]
    fn reasons_are_whitelisted_and_never_carry_provider_text() {
        let policy = deterministic();
        assert_eq!(
            policy.reason(&ModelError::RateLimited {
                retry_after_ms: 900
            }),
            "rate_limited"
        );
        assert_eq!(
            policy.reason(&ModelError::RequestFailed(
                "api key sk-secret leaked".to_string()
            )),
            "transient:request_failed"
        );
        assert_eq!(
            policy.reason(&ModelError::StreamInterrupted(
                "connection reset by peer".to_string()
            )),
            "transient:stream_interrupted"
        );
        assert_eq!(policy.reason(&ModelError::AuthFailed), "not_retryable");
    }

    #[test]
    fn classification_separates_rate_limits_from_transport_failures() {
        assert_eq!(
            retry_class(&ModelError::RateLimited { retry_after_ms: 1 }),
            Some(RetryClass::RateLimited)
        );
        assert_eq!(
            retry_class(&ModelError::RequestFailed("x".to_string())),
            Some(RetryClass::Transient)
        );
        assert_eq!(
            retry_class(&ModelError::StreamInterrupted("x".to_string())),
            Some(RetryClass::Transient)
        );
        assert_eq!(retry_class(&ModelError::AuthFailed), None);
        assert_eq!(
            retry_class(&ModelError::ContextLengthExceeded { used: 1, max: 2 }),
            None
        );
        assert_eq!(
            retry_after_ms(&ModelError::RateLimited { retry_after_ms: 42 }),
            Some(42)
        );
        assert_eq!(retry_after_ms(&ModelError::AuthFailed), None);
    }
}
