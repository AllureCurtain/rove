use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Desktop application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Bearer token for API authentication
    pub bearer_token: String,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            bearer_token: generate_bearer_token(),
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
    let config_path = get_config_path()?;

    if config_path.exists() {
        // Load existing config
        let content =
            std::fs::read_to_string(&config_path).context("Failed to read config file")?;
        let config: DesktopConfig =
            serde_json::from_str(&content).context("Failed to parse config file")?;
        Ok(config)
    } else {
        // Create new config
        let config = DesktopConfig::default();

        // Ensure parent directory exists
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        // Write config to disk
        let content =
            serde_json::to_string_pretty(&config).context("Failed to serialize config")?;
        std::fs::write(&config_path, content).context("Failed to write config file")?;

        // Set file permissions to 0600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&config_path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&config_path, perms)?;
        }

        tracing::info!("Created new config at {:?}", config_path);
        Ok(config)
    }
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
        assert!(dir.to_string_lossy().contains("Rove") || dir.to_string_lossy().contains("rove"));
    }

    #[test]
    fn test_get_state_dir() {
        let dir = get_state_dir().unwrap();
        assert!(dir.to_string_lossy().contains("Rove") || dir.to_string_lossy().contains("rove"));
    }

    #[test]
    fn test_get_logs_dir() {
        let dir = get_logs_dir().unwrap();
        assert!(dir.to_string_lossy().contains("Rove") || dir.to_string_lossy().contains("rove"));
    }
}
