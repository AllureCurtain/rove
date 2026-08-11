use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::{command, AppHandle, Manager};
use tokio::sync::RwLock;

/// Roots selected through the native picker. Native reveal operations are
/// bounded by these roots, so a renderer cannot turn the command into a
/// general filesystem browser.
#[derive(Clone, Default)]
pub struct WorkspaceRoots(pub Arc<RwLock<Vec<PathBuf>>>);

impl WorkspaceRoots {
    pub fn for_process() -> Self {
        let mut roots = Vec::new();
        if let Ok(cwd) = std::env::current_dir().and_then(|path| path.canonicalize()) {
            roots.push(cwd);
        }
        for path in [
            crate::config::get_config_dir(),
            crate::config::get_state_dir(),
            crate::config::get_logs_dir(),
        ]
        .into_iter()
        .filter_map(Result::ok)
        {
            if let Ok(path) = path.canonicalize() {
                roots.push(path);
            }
        }
        Self(Arc::new(RwLock::new(roots)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPaths {
    pub config_dir: String,
    pub state_dir: String,
    pub logs_dir: String,
}

#[command]
pub fn get_app_paths() -> Result<AppPaths, String> {
    let config_dir = crate::config::get_config_dir().map_err(|e| e.to_string())?;
    let state_dir = crate::config::get_state_dir().map_err(|e| e.to_string())?;
    let logs_dir = crate::config::get_logs_dir().map_err(|e| e.to_string())?;
    Ok(AppPaths {
        config_dir: config_dir.to_string_lossy().to_string(),
        state_dir: state_dir.to_string_lossy().to_string(),
        logs_dir: logs_dir.to_string_lossy().to_string(),
    })
}

#[command]
pub async fn workspace_select(app: AppHandle) -> Result<Option<String>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Choose a Rove workspace")
            .pick_folder()
    })
    .await
    .map_err(|error| format!("folder picker failed: {error}"))?;
    let Some(path) = selected else {
        return Ok(None);
    };
    let root = canonical_existing_directory(&path)?;
    let roots = app.state::<WorkspaceRoots>();
    let mut known = roots.0.write().await;
    if !known.iter().any(|entry| entry == &root) {
        known.push(root.clone());
    }
    Ok(Some(root.to_string_lossy().to_string()))
}

fn is_safe_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}

#[command]
pub async fn open_external(url: String) -> Result<(), String> {
    if !is_safe_url(&url) {
        return Err("only absolute http:// and https:// URLs are allowed".to_string());
    }
    open::that(&url).map_err(|error| format!("failed to open URL: {error}"))
}

fn canonical_existing_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("path is not accessible: {error}"))?;
    if !canonical.is_dir() {
        return Err("workspace path must be a directory".to_string());
    }
    Ok(canonical)
}

fn canonical_existing_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err("path contains a traversal component".to_string());
    }
    path.canonicalize()
        .map_err(|error| format!("path is not accessible: {error}"))
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn is_safe_path(path: &Path, allowed_roots: &[PathBuf]) -> Result<PathBuf, String> {
    let canonical = canonical_existing_path(path)?;
    if allowed_roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| path_is_within(&canonical, &root))
    {
        return Ok(canonical);
    }
    Err("path is outside an approved workspace".to_string())
}

#[command]
pub async fn show_in_folder(app: AppHandle, path: String) -> Result<(), String> {
    let path_buf = PathBuf::from(path);
    let roots = app.state::<WorkspaceRoots>();
    let known = roots.0.read().await.clone();
    let canonical = is_safe_path(&path_buf, &known)?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&canonical)
            .spawn()
            .map_err(|error| format!("failed to open file manager: {error}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&canonical)
            .spawn()
            .map_err(|error| format!("failed to open Finder: {error}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(canonical.parent().unwrap_or(&canonical))
            .spawn()
            .map_err(|error| format!("failed to open file manager: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_url_requires_http_host() {
        assert!(is_safe_url("https://example.com/path"));
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("https://"));
    }

    #[test]
    fn path_validation_rejects_traversal_and_escape() {
        let root =
            std::env::temp_dir().join(format!("rove-desktop-command-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let inside = root.join("nested");
        std::fs::write(inside.join("file.txt"), "ok").unwrap();
        assert!(is_safe_path(&inside.join("file.txt"), std::slice::from_ref(&root)).is_ok());
        let mut traversal = root.join("nested");
        traversal.push("..");
        traversal.push("file.txt");
        assert!(is_safe_path(&traversal, std::slice::from_ref(&root)).is_err());
        let outside =
            std::env::temp_dir().join(format!("rove-desktop-outside-{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "outside").unwrap();
        assert!(is_safe_path(&outside, std::slice::from_ref(&root)).is_err());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(outside);
    }
}
