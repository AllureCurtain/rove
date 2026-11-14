use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DESKTOP_ROOT_ENV: &str = "ROVE_DESKTOP_ROOT";

fn desktop_root() -> Result<Option<PathBuf>> {
    let Some(root) = std::env::var_os(DESKTOP_ROOT_ENV).map(PathBuf::from) else {
        return Ok(None);
    };
    anyhow::ensure!(
        root.is_absolute(),
        "{DESKTOP_ROOT_ENV} must be an absolute path"
    );
    Ok(Some(root))
}

/// What closing the last window does. Persisted in `desktop.json` so the
/// choice survives restarts and stays a host-local concern.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    /// Hide the window and keep the app alive in the system tray.
    #[default]
    HideToTray,
    /// Exit through the normal `ExitRequested` drain.
    Quit,
}

/// Desktop application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Bearer token for API authentication
    pub bearer_token: String,
    /// What a window close request does; defaults to minimizing to the tray.
    #[serde(default)]
    pub close_action: CloseAction,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            bearer_token: generate_bearer_token(),
            close_action: CloseAction::default(),
        }
    }
}

/// Generate a random bearer token
pub fn generate_bearer_token() -> String {
    use rand::Rng;
    let random_bytes: [u8; 32] = rand::thread_rng().gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Get the config file path
pub fn get_config_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join("desktop.json"))
}

/// Get the config directory
pub fn get_config_dir() -> Result<PathBuf> {
    if let Some(root) = desktop_root()? {
        return Ok(root.join("config"));
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("APPDATA environment variable not set")?;
        Ok(PathBuf::from(appdata).join("Rove").join("config"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Rove")
            .join("config"))
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            Ok(PathBuf::from(xdg_config).join("rove"))
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            Ok(PathBuf::from(home).join(".config").join("rove"))
        }
    }
}

/// Get the state directory
pub fn get_state_dir() -> Result<PathBuf> {
    if let Some(root) = desktop_root()? {
        return Ok(root.join("state"));
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("APPDATA environment variable not set")?;
        Ok(PathBuf::from(appdata).join("Rove").join("state"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Rove")
            .join("state"))
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            Ok(PathBuf::from(xdg_data).join("rove"))
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("rove"))
        }
    }
}

/// Get the logs directory
pub fn get_logs_dir() -> Result<PathBuf> {
    if let Some(root) = desktop_root()? {
        return Ok(root.join("logs"));
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("APPDATA environment variable not set")?;
        Ok(PathBuf::from(appdata).join("Rove").join("logs"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join("Rove"))
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
            Ok(PathBuf::from(xdg_state).join("rove").join("logs"))
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            Ok(PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("rove")
                .join("logs"))
        }
    }
}

/// Load or create desktop configuration
pub fn load_or_create_config() -> Result<DesktopConfig> {
    load_or_create_config_at(&get_config_path()?)
}

fn load_or_create_config_at(config_path: &std::path::Path) -> Result<DesktopConfig> {
    use std::io::Write;
    let read = || -> Result<DesktopConfig> {
        let content = std::fs::read_to_string(config_path).context("Failed to read config file")?;
        serde_json::from_str(&content).context("Failed to parse config file")
    };
    match std::fs::symlink_metadata(config_path) {
        Ok(_) => return read(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Failed to inspect config file"),
    }
    let parent = config_path
        .parent()
        .context("Config directory is missing")?;
    std::fs::create_dir_all(parent).context("Failed to create config directory")?;
    let config = DesktopConfig::default();
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(&config)?)?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(config_path) {
        Ok(_) => Ok(config),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error.error).context("Failed to publish desktop config"),
    }
}

/// Update `close_action` in the on-disk config, rewriting the file atomically.
/// The read-modify-write races with a concurrent setter are acceptable: the
/// field is user-driven and the last write wins.
pub fn save_close_action(action: CloseAction) -> Result<()> {
    save_close_action_at(&get_config_path()?, action)
}

fn save_close_action_at(config_path: &std::path::Path, action: CloseAction) -> Result<()> {
    let mut config = load_or_create_config_at(config_path)?;
    config.close_action = action;
    write_config_atomically(config_path, &config)
}

fn write_config_atomically(config_path: &std::path::Path, config: &DesktopConfig) -> Result<()> {
    use std::io::Write;
    let parent = config_path
        .parent()
        .context("Config directory is missing")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(config)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(config_path)
        .map_err(|error| error.error)
        .context("Failed to publish desktop config")?;
    Ok(())
}

/// Install a minimal crash marker without recording panic payloads, which may
/// contain provider credentials or user content. The normal stderr hook is
/// intentionally replaced so secrets cannot be copied into a crash log.
pub fn install_crash_handler() -> Result<()> {
    let log_path = get_logs_dir()?.join("desktop-crash.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create crash log directory")?;
    }
    std::panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let line = format!("desktop panic at {location}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
    }));
    Ok(())
}

/// Record a payload-free startup marker and return its path for the native
/// failure dialog. Startup errors can contain paths or provider configuration,
/// so the detailed error is intentionally not copied into the log.
pub fn record_startup_failure() -> Result<PathBuf> {
    let log_path = get_logs_dir()?.join("desktop-startup.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create startup log directory")?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("failed to open startup log")?;
    std::io::Write::write_all(
        &mut file,
        b"Rove Desktop startup failed; sensitive error details were omitted.\n",
    )
    .context("failed to write startup log")?;
    Ok(log_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_first_launch_publishes_one_complete_token() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/desktop.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    load_or_create_config_at(&path).unwrap().bearer_token
                })
            })
            .collect();
        let tokens: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(tokens.iter().all(|token| token == &tokens[0]));
        assert_eq!(
            load_or_create_config_at(&path).unwrap().bearer_token,
            tokens[0]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn close_action_defaults_to_hide_to_tray_and_round_trips() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/desktop.json");
        let config = load_or_create_config_at(&path).unwrap();
        assert_eq!(config.close_action, CloseAction::HideToTray);
        // A config written before the field existed must still load.
        std::fs::write(&path, r#"{"bearer_token":"token"}"#).unwrap();
        let config = load_or_create_config_at(&path).unwrap();
        assert_eq!(config.close_action, CloseAction::HideToTray);
    }

    #[test]
    fn save_close_action_rewrites_config_preserving_token() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/desktop.json");
        let config = load_or_create_config_at(&path).unwrap();
        save_close_action_at(&path, CloseAction::Quit).unwrap();
        let reloaded: DesktopConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reloaded.close_action, CloseAction::Quit);
        assert_eq!(reloaded.bearer_token, config.bearer_token);
    }

    #[test]
    fn test_generate_bearer_token() {
        let token1 = generate_bearer_token();
        let token2 = generate_bearer_token();

        // Tokens should be non-empty
        assert!(!token1.is_empty());
        assert!(!token2.is_empty());

        // Tokens should be different
        assert_ne!(token1, token2);

        // Token should be valid base64
        assert!(base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&token1)
            .is_ok());
    }

    #[test]
    fn test_get_config_dir() {
        let dir = get_config_dir().unwrap();
        assert!(dir.is_absolute());
        if let Some(root) = desktop_root().unwrap() {
            assert_eq!(dir, root.join("config"));
        }
    }

    #[test]
    fn test_get_state_dir() {
        let dir = get_state_dir().unwrap();
        assert!(dir.is_absolute());
        if let Some(root) = desktop_root().unwrap() {
            assert_eq!(dir, root.join("state"));
        }
    }

    #[test]
    fn test_get_logs_dir() {
        let dir = get_logs_dir().unwrap();
        assert!(dir.is_absolute());
        if let Some(root) = desktop_root().unwrap() {
            assert_eq!(dir, root.join("logs"));
        }
    }
}
