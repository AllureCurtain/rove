use rove_desktop::config;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct AppDataOverride {
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl AppDataOverride {
    fn install(value: &std::path::Path) -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", value);
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for AppDataOverride {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var("APPDATA", previous);
        } else {
            std::env::remove_var("APPDATA");
        }
    }
}

fn lock_environment() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Test that bearer token persists across config reloads
#[test]
fn test_bearer_token_persistence() {
    // Create a temporary directory for config
    let temp_dir = std::env::temp_dir().join(format!("rove-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Override config directory for this test
    let appdata = AppDataOverride::install(&temp_dir);

    // First load should generate a token
    let config1 = config::load_or_create_config().unwrap();
    let token1 = config1.bearer_token.clone();
    assert!(!token1.is_empty());

    // Second load should use the same token
    let config2 = config::load_or_create_config().unwrap();
    let token2 = config2.bearer_token;
    assert_eq!(token1, token2);

    // Cleanup
    drop(appdata);
    std::fs::remove_dir_all(temp_dir).ok();
}

/// Test that config directories are resolved correctly
#[test]
fn test_directory_resolution() {
    let _environment = lock_environment();
    let config_dir = config::get_config_dir().unwrap();
    let state_dir = config::get_state_dir().unwrap();
    let logs_dir = config::get_logs_dir().unwrap();

    // All paths should contain "Rove" or "rove"
    let config_str = config_dir.to_string_lossy().to_lowercase();
    let state_str = state_dir.to_string_lossy().to_lowercase();
    let logs_str = logs_dir.to_string_lossy().to_lowercase();

    assert!(
        config_str.contains("rove"),
        "Config dir missing 'rove': {}",
        config_str
    );
    assert!(
        state_str.contains("rove"),
        "State dir missing 'rove': {}",
        state_str
    );
    assert!(
        logs_str.contains("rove"),
        "Logs dir missing 'rove': {}",
        logs_str
    );

    // All paths should be absolute
    assert!(config_dir.is_absolute());
    assert!(state_dir.is_absolute());
    assert!(logs_dir.is_absolute());

    // Paths should be different
    assert_ne!(config_dir, state_dir);
    assert_ne!(config_dir, logs_dir);
}

/// Test that bearer tokens are cryptographically random
#[test]
fn test_token_randomness() {
    let _environment = lock_environment();
    let tokens: Vec<String> = (0..10).map(|_| config::generate_bearer_token()).collect();

    // All tokens should be non-empty
    assert!(tokens.iter().all(|t| !t.is_empty()));

    // All tokens should be unique
    let unique_count = tokens
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(unique_count, tokens.len(), "Tokens are not unique");

    // All tokens should be valid base64
    for token in &tokens {
        assert!(
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, token)
                .is_ok(),
            "Token is not valid base64: {}",
            token
        );
    }
}
