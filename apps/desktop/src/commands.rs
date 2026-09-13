use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::{command, AppHandle, Manager};
use tokio::sync::RwLock;
use zeroize::Zeroize;

use rove_api::{
    ProductProviderCatalogSelectionReceipt, ProductProviderOnboardingFailure,
    ProductProviderOnboardingProbe, ProductProviderOnboardingReceipt,
    ProductProviderOnboardingRequest, ProductProviderProfileId, ProductProviderType,
};

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

/// The WebView submits safe Provider metadata only. The native host collects
/// the raw credential and passes it directly to the shared Product API
/// onboarding facade; the value never crosses the command boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentialPromptRequest {
    #[serde(default)]
    pub profile_id: Option<String>,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    pub model: String,
    #[serde(default = "default_true")]
    pub make_default: bool,
    #[serde(default)]
    pub expected_revision: Option<String>,
}

fn default_true() -> bool {
    true
}

fn product_onboarding_request(
    request: &ProviderCredentialPromptRequest,
) -> Result<ProductProviderOnboardingRequest, ProviderCredentialPromptFailure> {
    let profile_id = request
        .profile_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            ProductProviderProfileId::from_catalog_id(value.to_string()).map_err(|_| {
                ProviderCredentialPromptFailure::new(
                    "provider_onboarding_invalid",
                    "provider profile id is invalid",
                )
            })
        })
        .transpose()?;
    let label = request.label.trim();
    if label.is_empty() || label.len() > 256 || request.label.chars().any(char::is_control) {
        return Err(ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider profile label is invalid",
        ));
    }
    let api_base = request.api_base.trim();
    if api_base.is_empty() || api_base.len() > 2_048 || api_base.chars().any(char::is_control) {
        return Err(ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider API base is invalid",
        ));
    }
    let model = request.model.trim();
    if model.is_empty() || model.len() > 1_024 || request.model.chars().any(char::is_control) {
        return Err(ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider model is invalid",
        ));
    }
    Ok(ProductProviderOnboardingRequest {
        profile_id,
        label: label.to_string(),
        provider_type: request.provider_type,
        api_base: api_base.to_string(),
        model: model.to_string(),
        make_default: request.make_default,
        expected_revision: request.expected_revision.clone(),
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCredentialPromptFailure {
    pub code: String,
    pub message: String,
}

impl ProviderCredentialPromptFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<ProductProviderOnboardingFailure> for ProviderCredentialPromptFailure {
    fn from(failure: ProductProviderOnboardingFailure) -> Self {
        Self {
            code: failure.code,
            message: failure.message,
        }
    }
}

fn product_profile_id(
    profile_id: &str,
) -> Result<ProductProviderProfileId, ProviderCredentialPromptFailure> {
    let profile_id = profile_id.trim();
    ProductProviderProfileId::from_catalog_id(profile_id.to_string()).map_err(|_| {
        ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider profile id is invalid",
        )
    })
}

fn optional_model(
    model: Option<String>,
) -> Result<Option<String>, ProviderCredentialPromptFailure> {
    model
        .map(|model| {
            let trimmed = model.trim();
            if trimmed.is_empty() || trimmed.len() > 1_024 || model.chars().any(char::is_control) {
                return Err(ProviderCredentialPromptFailure::new(
                    "provider_onboarding_invalid",
                    "provider model is invalid",
                ));
            }
            Ok(trimmed.to_string())
        })
        .transpose()
}

fn optional_catalog_revision(
    revision: Option<String>,
) -> Result<Option<String>, ProviderCredentialPromptFailure> {
    revision
        .map(|revision| {
            let trimmed = revision.trim();
            if trimmed.is_empty() || trimmed.len() > 128 || revision.chars().any(char::is_control) {
                return Err(ProviderCredentialPromptFailure::new(
                    "provider_onboarding_invalid",
                    "provider catalog revision is invalid",
                ));
            }
            Ok(trimmed.to_string())
        })
        .transpose()
}

/// Prompt for a provider API key using the host's native secure credential UI
/// and publish it through the shared ProviderOnboardingService. No secret
/// crosses the Tauri command boundary or is included in the return value.
#[command]
pub async fn provider_credential_prompt(
    app: AppHandle,
    request: ProviderCredentialPromptRequest,
) -> Result<ProductProviderOnboardingReceipt, ProviderCredentialPromptFailure> {
    let onboarding = product_onboarding_request(&request)?;
    let label = request.label.trim().to_string();
    let prompt_target = onboarding
        .profile_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "new-profile".to_string());
    let mut secret = tauri::async_runtime::spawn_blocking(move || {
        native_provider_secret(&label, &prompt_target)
    })
    .await
    .map_err(|_| {
        ProviderCredentialPromptFailure::new(
            "provider_credential_prompt",
            "provider credential prompt did not complete",
        )
    })??;
    let state = app.state::<crate::ApiServerState>();
    let result = state
        .api_state
        .onboard_product_provider(onboarding, &secret)
        .await
        .map_err(ProviderCredentialPromptFailure::from);
    secret.zeroize();
    result
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfileProbeRequest {
    pub profile_id: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// Probe a published shared-Catalog profile. The host resolves its existing
/// credential reference; the WebView never receives or supplies the secret.
#[command]
pub async fn provider_profile_probe(
    app: AppHandle,
    request: ProviderProfileProbeRequest,
) -> Result<ProductProviderOnboardingProbe, ProviderCredentialPromptFailure> {
    let profile_id = product_profile_id(&request.profile_id)?;
    let model = optional_model(request.model)?;
    let state = app.state::<crate::ApiServerState>();
    state
        .api_state
        .probe_product_provider(profile_id, model)
        .await
        .map_err(ProviderCredentialPromptFailure::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfileUseRequest {
    pub profile_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<String>,
}

/// Persist the native product's default Provider through the same Catalog CAS
/// path used by the CLI/TUI. Product preferences remain the API-owned exact
/// per-session selection and are updated by the Web product state afterward.
#[command]
pub async fn provider_profile_use(
    app: AppHandle,
    request: ProviderProfileUseRequest,
) -> Result<ProductProviderCatalogSelectionReceipt, ProviderCredentialPromptFailure> {
    let profile_id = product_profile_id(&request.profile_id)?;
    let model = optional_model(request.model)?;
    let expected_revision = optional_catalog_revision(request.expected_revision)?;
    let state = app.state::<crate::ApiServerState>();
    state
        .api_state
        .use_product_provider(profile_id, model, expected_revision)
        .await
        .map_err(ProviderCredentialPromptFailure::from)
}

#[cfg(target_os = "windows")]
fn native_provider_secret(
    label: &str,
    profile_id: &str,
) -> Result<String, ProviderCredentialPromptFailure> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{BOOL, ERROR_CANCELLED};
    use windows::Win32::Security::Credentials::{
        CredUIPromptForCredentialsW, CREDUI_FLAGS, CREDUI_FLAGS_ALWAYS_SHOW_UI,
        CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_INFOW,
    };

    let caption: Vec<u16> = "Rove provider credential"
        .encode_utf16()
        .chain([0])
        .collect();
    let message = format!("Enter the API key for {label}");
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    let target: Vec<u16> = format!("rove.provider.onboarding:{profile_id}")
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
            CREDUI_FLAGS(
                CREDUI_FLAGS_ALWAYS_SHOW_UI.0
                    | CREDUI_FLAGS_GENERIC_CREDENTIALS.0
                    | CREDUI_FLAGS_DO_NOT_PERSIST.0,
            ),
        )
    };
    if result == ERROR_CANCELLED {
        username.fill(0);
        password.fill(0);
        return Err(ProviderCredentialPromptFailure::new(
            "provider_credential_cancelled",
            "provider credential prompt was cancelled",
        ));
    }
    if result.is_err() {
        username.fill(0);
        password.fill(0);
        return Err(ProviderCredentialPromptFailure::new(
            "provider_credential_prompt",
            "native provider credential prompt failed",
        ));
    }
    let end = password
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(password.len());
    let decoded = String::from_utf16(&password[..end]);
    username.fill(0);
    password.fill(0);
    let secret = decoded.map_err(|_| {
        ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider credential contains invalid characters",
        )
    })?;
    if secret.is_empty() {
        return Err(ProviderCredentialPromptFailure::new(
            "provider_onboarding_invalid",
            "provider credential cannot be empty",
        ));
    }
    Ok(secret)
}

#[cfg(not(target_os = "windows"))]
fn native_provider_secret(
    _label: &str,
    _profile_id: &str,
) -> Result<String, ProviderCredentialPromptFailure> {
    Err(ProviderCredentialPromptFailure::new(
        "provider_credential_unsupported",
        "native provider credential onboarding is only available on Windows",
    ))
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
    fn credential_prompt_validates_metadata_without_accepting_a_secret() {
        let request = ProviderCredentialPromptRequest {
            profile_id: Some("profile-123".to_string()),
            label: "SiliconFlow".to_string(),
            provider_type: ProductProviderType::Openai,
            api_base: "https://api.siliconflow.cn/v1".to_string(),
            model: "deepseek-ai/DeepSeek-V3.2".to_string(),
            make_default: true,
            expected_revision: None,
        };
        let onboarding = product_onboarding_request(&request).unwrap();
        assert_eq!(onboarding.profile_id.unwrap().as_str(), "profile-123");
        assert_eq!(onboarding.provider_type, ProductProviderType::Openai);
        assert_eq!(onboarding.model, "deepseek-ai/DeepSeek-V3.2");
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("password"));
    }

    #[test]
    fn credential_prompt_rejects_path_like_or_control_metadata() {
        let mut request = ProviderCredentialPromptRequest {
            profile_id: Some("../profile".to_string()),
            label: "Provider".to_string(),
            provider_type: ProductProviderType::Openai,
            api_base: "https://example.test/v1".to_string(),
            model: "model".to_string(),
            make_default: true,
            expected_revision: None,
        };
        assert!(product_onboarding_request(&request).is_err());
        request.profile_id = Some("profile".to_string());
        request.label = "Provider\n".to_string();
        assert!(product_onboarding_request(&request).is_err());
    }

    #[test]
    fn probe_and_use_metadata_are_bounded_and_secret_free() {
        let probe = ProviderProfileProbeRequest {
            profile_id: "siliconflow-deepseek-v3-2".to_string(),
            model: Some("deepseek-ai/DeepSeek-V3.2".to_string()),
        };
        assert_eq!(
            product_profile_id(&probe.profile_id).unwrap().as_str(),
            "siliconflow-deepseek-v3-2"
        );
        assert_eq!(
            optional_model(probe.model.clone()).unwrap().as_deref(),
            Some("deepseek-ai/DeepSeek-V3.2")
        );
        let use_request = ProviderProfileUseRequest {
            profile_id: probe.profile_id,
            model: probe.model,
            expected_revision: Some("sha256:catalog-revision".to_string()),
        };
        let encoded = serde_json::to_string(&use_request).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("password"));
        assert!(optional_model(Some("\n".to_string())).is_err());
        assert!(optional_catalog_revision(Some("\n".to_string())).is_err());
    }
}
