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
//!
//! Silent-turn recovery lives here too. It answers a different failure: the
//! provider succeeded and the run terminated with a final answer, but that
//! answer had no visible text. [`SilentTurnEvidence`] records the two facts a
//! run observes while it runs, [`SilentTurnObservation`] adds the two facts
//! only known at termination, and [`SilentTurnRecoveryPolicy`] caps how many
//! extra turns the run loop may spend on it.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::execution::{ExecutionDegradation, ExecutionPhase};
use rove_models::ModelError;

/// The fixed conversation nudge injected before a silent-turn recovery turn.
///
/// It is a compile-time constant, so no user, workspace, or provider text can
/// reach it, and it is safe for the event stream, the trace, and the report.
pub const SILENT_TURN_NUDGE: &str = "Your previous turn produced no visible response. Continue: either finish the task or summarize the progress you have so far.";

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
    /// so it is safe for the event stream, the trace, and the report. The class
    /// is a parameter rather than re-derived here, so the string set stays
    /// total over the retryable classes and cannot describe an unretried error.
    pub fn reason(class: RetryClass, error: &ModelError) -> String {
        match class {
            RetryClass::RateLimited => "rate_limited".to_string(),
            RetryClass::Transient => format!("transient:{}", error.error_code()),
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

/// Runtime-owned silent-turn recovery budget.
///
/// The default is enabled with the single extra turn the design budgets for,
/// so recovery can never become a loop. `max_attempts == 0` disables it and
/// restores the behavior that existed before silent-turn recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentTurnRecoveryPolicy {
    /// Extra model turns a single run may spend recovering a silent turn.
    pub max_attempts: u32,
}

impl Default for SilentTurnRecoveryPolicy {
    fn default() -> Self {
        Self { max_attempts: 1 }
    }
}

impl SilentTurnRecoveryPolicy {
    /// A policy that never spends a recovery turn.
    pub fn disabled() -> Self {
        Self { max_attempts: 0 }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_attempts > 0
    }

    /// Whether one more recovery turn may be spent after `attempts_used`.
    pub fn allows(&self, attempts_used: u32) -> bool {
        attempts_used < self.max_attempts
    }

    /// The canonical `model_status` value announced before a recovery turn.
    pub const STATUS: &'static str = "recovering_silent_turn";

    /// The `execution_degraded` code recorded for a recovery turn.
    pub const DEGRADATION_CODE: &'static str = "silent_turn_recovery";

    /// The safe summary recorded with that degradation. Fixed text: the fact
    /// that a run was silent is not user data, and the nudge itself is already
    /// on the status event.
    pub const DEGRADATION_SUMMARY: &'static str =
        "The run ended without a visible response; one recovery turn was started.";

    /// One degradation fact for a spent recovery turn, following the identity
    /// and timestamp conventions of the other `ExecutionDegraded` producers.
    pub fn degradation(&self) -> ExecutionDegradation {
        ExecutionDegradation {
            degradation_id: ulid::Ulid::new().to_string(),
            phase: ExecutionPhase::Run,
            code: Self::DEGRADATION_CODE.to_string(),
            safe_summary: Self::DEGRADATION_SUMMARY.to_string(),
            occurred_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// The facts a run observes while it is still running.
///
/// Both are read from the canonical events the run emits rather than inferred
/// from state, so "silent" means exactly what the event stream says: no
/// `ToolCallStarted` happened and no `LlmMessage` carried text.
#[derive(Debug, Default)]
pub struct SilentTurnEvidence {
    tool_call_started: bool,
    model_text_seen: bool,
}

impl SilentTurnEvidence {
    /// Record one canonical `ToolCallStarted`.
    pub fn record_tool_call_started(&mut self) {
        self.tool_call_started = true;
    }

    /// Record one canonical `LlmMessage`'s full text.
    pub fn record_model_message(&mut self, full: &str) {
        if !full.trim().is_empty() {
            self.model_text_seen = true;
        }
    }

    /// Finish the observation with the facts only known at termination.
    ///
    /// `final_output` is the answer the run terminated with; `user_message` is
    /// the input that triggered the run. `final_output_empty` treats
    /// whitespace-only text as empty, so a provider that emits spaces does not
    /// count as a visible response.
    pub fn observe(&self, final_output: &str, user_message: &str) -> SilentTurnObservation {
        SilentTurnObservation {
            tool_call_started: self.tool_call_started,
            model_text_seen: self.model_text_seen,
            final_output_empty: final_output.trim().is_empty(),
            user_message_present: !user_message.trim().is_empty(),
        }
    }
}

/// The complete detection input for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentTurnObservation {
    /// A tool call actually started during the run.
    pub tool_call_started: bool,
    /// A model message carried non-empty text during the run.
    pub model_text_seen: bool,
    /// The run's final answer had no visible text.
    pub final_output_empty: bool,
    /// The run was triggered by a real user message.
    pub user_message_present: bool,
}

impl SilentTurnObservation {
    /// A run is silent only when **all** of these hold:
    ///
    /// 1. it produced no visible text — neither a model message nor the final
    ///    answer carried any;
    /// 2. no tool call started, because a run that did tool work already made
    ///    progress the user can inspect;
    /// 3. it answered a real user message rather than a runtime-injected turn.
    pub fn is_silent(&self) -> bool {
        !self.model_text_seen
            && self.final_output_empty
            && !self.tool_call_started
            && self.user_message_present
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
    use super::{
        ProviderRetryPolicy, RetryClass, SILENT_TURN_NUDGE, SilentTurnEvidence,
        SilentTurnRecoveryPolicy, retry_after_ms, retry_class,
    };
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
        assert_eq!(
            ProviderRetryPolicy::reason(
                RetryClass::RateLimited,
                &ModelError::RateLimited {
                    retry_after_ms: 900
                }
            ),
            "rate_limited"
        );
        assert_eq!(
            ProviderRetryPolicy::reason(
                RetryClass::Transient,
                &ModelError::RequestFailed("api key sk-secret leaked".to_string())
            ),
            "transient:request_failed"
        );
        assert_eq!(
            ProviderRetryPolicy::reason(
                RetryClass::Transient,
                &ModelError::StreamInterrupted("connection reset by peer".to_string())
            ),
            "transient:stream_interrupted"
        );
        // An error with no retry class cannot reach a reason at all: it has no
        // budget to charge and is terminal on its first attempt.
        assert_eq!(retry_class(&ModelError::AuthFailed), None);
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

    #[test]
    fn silent_turn_recovery_is_enabled_once_by_default() {
        let policy = SilentTurnRecoveryPolicy::default();
        assert!(policy.is_enabled());
        assert_eq!(policy.max_attempts, 1);
        assert!(policy.allows(0), "the first recovery turn is available");
        assert!(
            !policy.allows(1),
            "the default budget is a hard cap, not a loop"
        );
        assert!(!SilentTurnRecoveryPolicy::disabled().is_enabled());
        assert!(!SilentTurnRecoveryPolicy::disabled().allows(0));
    }

    #[test]
    fn a_degradation_fact_is_identified_and_safely_summarized() {
        let record = SilentTurnRecoveryPolicy::default().degradation();
        assert_eq!(record.code, SilentTurnRecoveryPolicy::DEGRADATION_CODE);
        assert_eq!(record.code, "silent_turn_recovery");
        assert_eq!(record.phase, crate::execution::ExecutionPhase::Run);
        assert!(!record.degradation_id.is_empty());
        assert!(record.occurred_at.ends_with('Z') || record.occurred_at.contains('+'));
        // The nudge is a fixed constant with no user data in it, and the
        // summary must not smuggle any in either.
        assert!(SILENT_TURN_NUDGE.contains("no visible response"));
        assert!(!record.safe_summary.contains(SILENT_TURN_NUDGE));
    }

    #[test]
    fn silence_requires_every_rule_to_hold() {
        let evidence = SilentTurnEvidence::default();
        let silent = evidence.observe("", "do the task");
        assert!(silent.is_silent(), "a bare empty final answer is silent");

        // Rule 1: whitespace-only text is still no visible response, but any
        // real model or final text is one.
        assert!(evidence.observe("  \n ", "do the task").is_silent());
        let mut spoke = SilentTurnEvidence::default();
        spoke.record_model_message("an answer");
        assert!(!spoke.observe("", "do the task").is_silent());
        assert!(!evidence.observe("an answer", "do the task").is_silent());
        // A redacted Review-mode message counts as text, so a Review turn that
        // produced a message is not read as silent.
        let mut redacted = SilentTurnEvidence::default();
        redacted.record_model_message("[review model output omitted]");
        assert!(!redacted.observe("", "review this").is_silent());

        // Rule 2: a run that used tools is not silent, whatever it answered.
        let mut worked = SilentTurnEvidence::default();
        worked.record_tool_call_started();
        assert!(!worked.observe("", "do the task").is_silent());

        // Rule 3: a runtime-injected turn is not a real user message.
        assert!(!evidence.observe("", "   ").is_silent());
    }
}
