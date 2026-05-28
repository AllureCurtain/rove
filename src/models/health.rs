use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub failure_threshold: u32,
    pub open_cooldown: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            open_cooldown: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct ModelHealthStore {
    config: HealthConfig,
    states: Mutex<HashMap<String, HealthState>>,
}

impl ModelHealthStore {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            states: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow_call(&self, target_id: &str) -> bool {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
        match state.status {
            CircuitStatus::Closed => true,
            CircuitStatus::Open => {
                if state
                    .opened_at
                    .is_some_and(|opened_at| opened_at.elapsed() >= self.config.open_cooldown)
                {
                    state.status = CircuitStatus::HalfOpen;
                    state.half_open_token = true;
                }
                if state.status == CircuitStatus::HalfOpen && state.half_open_token {
                    state.half_open_token = false;
                    true
                } else {
                    false
                }
            }
            CircuitStatus::HalfOpen => {
                if state.half_open_token {
                    state.half_open_token = false;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn mark_success(&self, target_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
        state.status = CircuitStatus::Closed;
        state.consecutive_failures = 0;
        state.opened_at = None;
        state.half_open_token = true;
    }

    pub fn mark_failure(&self, target_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.status == CircuitStatus::HalfOpen
            || state.consecutive_failures >= self.config.failure_threshold.max(1)
        {
            state.status = CircuitStatus::Open;
            state.opened_at = Some(Instant::now());
            state.half_open_token = false;
        }
    }

    #[cfg(test)]
    pub fn status_for_test(&self, target_id: &str) -> CircuitStatus {
        self.states
            .lock()
            .expect("model health mutex poisoned")
            .get(target_id)
            .map(|state| state.status)
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
struct HealthState {
    status: CircuitStatus,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_token: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CircuitStatus {
    #[default]
    Closed,
    Open,
    HalfOpen,
}
