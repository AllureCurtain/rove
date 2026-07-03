use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
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
    persist_path: Option<PathBuf>,
}

impl ModelHealthStore {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            states: Mutex::new(HashMap::new()),
            persist_path: None,
        }
    }

    pub fn with_persistence(config: HealthConfig, state_dir: &Path) -> Self {
        let mut store = Self {
            config,
            states: Mutex::new(HashMap::new()),
            persist_path: Some(state_dir.join("circuit_breakers.json")),
        };
        store.load_from_disk();
        store
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
        drop(states);
        self.persist_to_disk();
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
        drop(states);
        self.persist_to_disk();
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

    fn load_from_disk(&mut self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(persisted) = serde_json::from_str::<PersistedHealthState>(&content) else {
            tracing::warn!("failed to parse circuit breaker state from disk, starting fresh");
            return;
        };

        let now = Instant::now();
        let unix_now_ms = unix_now_ms();
        let mut states = self.states.lock().expect("model health mutex poisoned");
        for (target_id, persisted_state) in persisted.states {
            let status = match persisted_state.status {
                PersistedCircuitStatus::Closed => CircuitStatus::Closed,
                PersistedCircuitStatus::Open => CircuitStatus::Open,
                PersistedCircuitStatus::HalfOpen => CircuitStatus::HalfOpen,
            };
            let opened_at = persisted_state.opened_at_unix_ms.map(|opened_at_unix_ms| {
                let elapsed = Duration::from_millis(unix_now_ms.saturating_sub(opened_at_unix_ms));
                checked_instant_sub(now, elapsed).unwrap_or(now)
            });

            states.insert(
                target_id,
                HealthState {
                    status,
                    consecutive_failures: persisted_state.consecutive_failures,
                    opened_at,
                    half_open_token: matches!(
                        status,
                        CircuitStatus::Closed | CircuitStatus::HalfOpen
                    ),
                },
            );
        }
    }

    fn persist_to_disk(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        let states = self.states.lock().expect("model health mutex poisoned");
        let unix_now_ms = unix_now_ms();
        let mut persisted = PersistedHealthState::default();

        for (target_id, state) in states.iter() {
            persisted.states.insert(
                target_id.clone(),
                PersistedHealthEntry {
                    status: match state.status {
                        CircuitStatus::Closed => PersistedCircuitStatus::Closed,
                        CircuitStatus::Open => PersistedCircuitStatus::Open,
                        CircuitStatus::HalfOpen => PersistedCircuitStatus::HalfOpen,
                    },
                    consecutive_failures: state.consecutive_failures,
                    opened_at_unix_ms: state.opened_at.map(|opened_at| {
                        unix_now_ms.saturating_sub(opened_at.elapsed().as_millis() as u64)
                    }),
                },
            );
        }
        drop(states);

        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("failed to create circuit breaker state dir: {err}");
            return;
        }
        let Ok(content) = serde_json::to_string_pretty(&persisted) else {
            tracing::warn!("failed to serialize circuit breaker state");
            return;
        };
        if let Err(err) = std::fs::write(path, content) {
            tracing::warn!("failed to persist circuit breaker state: {err}");
        }
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedHealthState {
    states: HashMap<String, PersistedHealthEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedHealthEntry {
    status: PersistedCircuitStatus,
    consecutive_failures: u32,
    opened_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PersistedCircuitStatus {
    Closed,
    Open,
    HalfOpen,
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn checked_instant_sub(instant: Instant, duration: Duration) -> Option<Instant> {
    instant.checked_sub(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn persists_open_circuit_across_store_restart() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = HealthConfig {
            failure_threshold: 2,
            open_cooldown: Duration::from_secs(60),
        };

        {
            let store = ModelHealthStore::with_persistence(config.clone(), dir.path());
            assert!(store.allow_call("target-a"));
            store.mark_failure("target-a");
            store.mark_failure("target-a");

            assert_eq!(store.status_for_test("target-a"), CircuitStatus::Open);
            assert!(!store.allow_call("target-a"));
        }

        let store = ModelHealthStore::with_persistence(config, dir.path());

        assert_eq!(store.status_for_test("target-a"), CircuitStatus::Open);
        assert!(!store.allow_call("target-a"));
    }

    #[test]
    fn persists_closed_circuit_across_store_restart() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = HealthConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(60),
        };

        {
            let store = ModelHealthStore::with_persistence(config.clone(), dir.path());
            store.mark_failure("target-a");
            assert_eq!(store.status_for_test("target-a"), CircuitStatus::Open);
            store.mark_success("target-a");
            assert_eq!(store.status_for_test("target-a"), CircuitStatus::Closed);
        }

        let store = ModelHealthStore::with_persistence(config, dir.path());

        assert_eq!(store.status_for_test("target-a"), CircuitStatus::Closed);
        assert!(store.allow_call("target-a"));
    }

    #[test]
    fn expired_open_circuit_restores_as_half_open_candidate() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = HealthConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(60),
        };
        let mut states = HashMap::new();
        states.insert(
            "target-a".to_string(),
            PersistedHealthEntry {
                status: PersistedCircuitStatus::Open,
                consecutive_failures: 1,
                opened_at_unix_ms: Some(unix_now_ms().saturating_sub(120_000)),
            },
        );
        let persisted = PersistedHealthState { states };
        std::fs::write(
            dir.path().join("circuit_breakers.json"),
            serde_json::to_string(&persisted).unwrap(),
        )
        .unwrap();

        let store = ModelHealthStore::with_persistence(config, dir.path());

        assert!(store.allow_call("target-a"));
        assert_eq!(store.status_for_test("target-a"), CircuitStatus::HalfOpen);
        assert!(!store.allow_call("target-a"));
    }
}
