use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::{command, AppHandle, Manager};
use tokio::sync::RwLock;
use zeroize::Zeroize;

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

/// The native credential prompt never accepts a secret from the WebView. The
/// Web surface submits only a profile label and receives this opaque receipt
/// after the host has stored the value in the OS keyring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentialPromptRequest {
    pub profile_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentialReceipt {
    pub profile_id: String,
    pub source: String,
    pub service: String,
    pub account: String,
}

const DEFAULT_PROVIDER_KEYRING_SERVICE: &str = "com.rove.agent.provider";

fn credential_reference(
    request: &ProviderCredentialPromptRequest,
) -> Result<(String, String, String), String> {
    let profile_id = request.profile_id.trim();
    if profile_id != request.profile_id
        || profile_id.is_empty()
        || profile_id.len() > 128
        || !profile_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("provider profile id is invalid".to_string());
    }
    let label = request.label.trim();
    if label.is_empty() || label.len() > 256 || request.label.chars().any(char::is_control) {
        return Err("provider profile label is invalid".to_string());
    }
    Ok((
        profile_id.to_string(),
        DEFAULT_PROVIDER_KEYRING_SERVICE.to_string(),
        format!("profile:{profile_id}:{}", uuid::Uuid::new_v4()),
    ))
}

/// Prompt for a provider API key using the host's native secure credential UI
/// and persist it directly to the OS keyring. No secret crosses the Tauri
/// command boundary or is included in the return value.
#[command]
pub async fn provider_credential_prompt(
    request: ProviderCredentialPromptRequest,
) -> Result<ProviderCredentialReceipt, String> {
    let (profile_id, service, account) = credential_reference(&request)?;
    let label = request.label.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let mut secret = native_provider_secret(&label, &service, &account)?;
        let stored = keyring::Entry::new(&service, &account)
            .map_err(|error| format!("provider keyring is unavailable: {error}"))
            .and_then(|entry| {
                entry.set_password(&secret).map_err(|error| {
                    format!("provider credential could not be stored securely: {error}")
                })
            });
        secret.zeroize();
        stored?;
        Ok(ProviderCredentialReceipt {
            profile_id,
            source: "keyring".to_string(),
            service,
            account,
        })
    })
    .await
    .map_err(|error| format!("provider credential prompt failed: {error}"))?
}

#[cfg(target_os = "windows")]
fn native_provider_secret(label: &str, service: &str, account: &str) -> Result<String, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{BOOL, ERROR_CANCELLED};
    use windows::Win32::Security::Credentials::{
        CredUIPromptForCredentialsW, CREDUI_FLAGS, CREDUI_FLAGS_ALWAYS_SHOW_UI,
        CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_INFOW,
    };

    let caption: Vec<u16> = "Rove provider credential"
        .encode_utf16()
        .chain([0])
        .collect();
    let message = format!("Enter the API key for {label}");
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    let target: Vec<u16> = format!("{service}:{account}")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut username = vec![0u16; 512];
    let mut password = vec![0u16; 4096];
    let mut save = BOOL(0);
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        pszMessageText: PCWSTR(message.as_ptr()),
        pszCaptionText: PCWSTR(caption.as_ptr()),
        ..Default::default()
    };
    let result = unsafe {
        CredUIPromptForCredentialsW(
            Some(&info),
            PCWSTR(target.as_ptr()),
            None,
            0,
            &mut username,
            &mut password,
            Some(&mut save),
            CREDUI_FLAGS(CREDUI_FLAGS_ALWAYS_SHOW_UI.0 | CREDUI_FLAGS_GENERIC_CREDENTIALS.0),
        )
    };
    if result == ERROR_CANCELLED {
        username.fill(0);
        password.fill(0);
        return Err("provider credential prompt was cancelled".to_string());
    }
    if result.is_err() {
        username.fill(0);
        password.fill(0);
        return Err("native provider credential prompt failed".to_string());
    }
    let end = password
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(password.len());
    let decoded = String::from_utf16(&password[..end]);
    username.fill(0);
    password.fill(0);
    let secret =
        decoded.map_err(|_| "provider credential contains invalid characters".to_string())?;
    if secret.is_empty() {
        return Err("provider credential cannot be empty".to_string());
    }
    Ok(secret)
}

#[cfg(not(target_os = "windows"))]
fn native_provider_secret(_label: &str, _service: &str, _account: &str) -> Result<String, String> {
    Err("native provider credential onboarding is only available on Windows".to_string())
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

    #[test]
    fn credential_prompt_validates_metadata_and_returns_no_secret() {
        let request = ProviderCredentialPromptRequest {
            profile_id: "profile-123".to_string(),
            label: "SiliconFlow".to_string(),
        };
        let (profile_id, service, account) = credential_reference(&request).unwrap();
        assert_eq!(profile_id, "profile-123");
        assert_eq!(service, DEFAULT_PROVIDER_KEYRING_SERVICE);
        assert!(account.starts_with("profile:profile-123:"));
        let receipt = ProviderCredentialReceipt {
            profile_id,
            source: "keyring".to_string(),
            service,
            account,
        };
        let encoded = serde_json::to_string(&receipt).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("password"));
    }

    #[test]
    fn credential_prompt_rejects_path_like_or_control_metadata() {
        let mut request = ProviderCredentialPromptRequest {
            profile_id: "../profile".to_string(),
            label: "Provider".to_string(),
        };
        assert!(credential_reference(&request).is_err());
        request.profile_id = "profile".to_string();
        request.label = "Provider\n".to_string();
        assert!(credential_reference(&request).is_err());
    }
}
