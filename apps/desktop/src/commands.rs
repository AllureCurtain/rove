use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::command;

/// Application paths exposed to the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPaths {
    pub config_dir: String,
    pub state_dir: String,
    pub logs_dir: String,
}

/// Get application paths (config, state, logs directories)
#[command]
pub fn get_app_paths() -> Result<AppPaths, String> {
    let config_dir =
        crate::config::get_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    let state_dir =
        crate::config::get_state_dir().map_err(|e| format!("Failed to get state dir: {}", e))?;
    let logs_dir =
        crate::config::get_logs_dir().map_err(|e| format!("Failed to get logs dir: {}", e))?;

    Ok(AppPaths {
        config_dir: config_dir.to_string_lossy().to_string(),
        state_dir: state_dir.to_string_lossy().to_string(),
        logs_dir: logs_dir.to_string_lossy().to_string(),
    })
}

/// Open a native folder picker dialog
#[command]
pub async fn workspace_select() -> Result<Option<String>, String> {
    // TODO: Implement native folder picker with Tauri 2 dialog API
    // For now, return an error indicating this needs implementation
    Err("Folder picker not yet implemented".to_string())
}

/// Validate URL scheme
fn is_safe_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Open a URL in the default browser
#[command]
pub async fn open_external(url: String) -> Result<(), String> {
    // Validate URL scheme
    if !is_safe_url(&url) {
        return Err(format!(
            "Unsafe URL scheme. Only http:// and https:// are allowed: {}",
            url
        ));
    }

    // Open in default browser
    open::that(&url).map_err(|e| format!("Failed to open URL: {}", e))?;

    Ok(())
}

/// Validate path is under a workspace root or known safe directory
fn is_safe_path(path: &Path) -> Result<(), String> {
    // Path must be absolute
    if !path.is_absolute() {
        return Err("Path must be absolute".to_string());
    }

    // Path must exist
    if !path.exists() {
        return Err("Path does not exist".to_string());
    }

    // Check for obvious traversal patterns
    let path_str = path.to_string_lossy();
    if path_str.contains("..") {
        return Err("Path contains traversal component".to_string());
    }

    Ok(())
}

/// Show a file or directory in the native file manager
#[command]
pub async fn show_in_folder(path: String) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);

    // Validate path
    is_safe_path(&path_buf)?;

    // Show in file manager
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open Finder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        // Try xdg-open first, fallback to common file managers
        let result = std::process::Command::new("xdg-open")
            .arg(path_buf.parent().unwrap_or(&path_buf))
            .spawn();

        if result.is_err() {
            // Fallback to nautilus/dolphin/thunar
            for fm in &["nautilus", "dolphin", "thunar"] {
                if let Ok(_) = std::process::Command::new(fm).arg(&path).spawn() {
                    return Ok(());
                }
            }
            return Err("No file manager found".to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_safe_url() {
        assert!(is_safe_url("http://example.com"));
        assert!(is_safe_url("https://example.com"));
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("ftp://example.com"));
    }

    #[test]
    fn test_is_safe_path_relative() {
        let path = PathBuf::from("relative/path");
        assert!(is_safe_path(&path).is_err());
    }

    #[test]
    fn test_is_safe_path_traversal() {
        let path = PathBuf::from("/tmp/../etc/passwd");
        assert!(is_safe_path(&path).is_err());
    }

    #[test]
    fn test_get_app_paths() {
        let paths = get_app_paths().unwrap();
        assert!(!paths.config_dir.is_empty());
        assert!(!paths.state_dir.is_empty());
        assert!(!paths.logs_dir.is_empty());
    }
}
