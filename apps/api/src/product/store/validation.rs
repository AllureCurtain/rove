use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Url;
use rove_runtime::{Workspace, WorkspaceKind};

use crate::product::{
    CreateProductProviderProfileRequest, CreateProductWorkspaceRequest,
    M1_BROWSER_SOURCE_SCHEMA_VERSION, M1BrowserMigrationRequest, M1MigrationIssue,
    MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES, MAX_PRODUCT_API_BASE_BYTES, MAX_PRODUCT_PATH_BYTES,
    MAX_PRODUCT_PROVIDER_PROFILES, MAX_PRODUCT_SESSIONS, MAX_PRODUCT_TEXT_BYTES,
    MAX_PRODUCT_WORKSPACES, ProductApprovalPreference, ProductErrorCode, ProductProviderProfileId,
    ProductProviderSelection, ProductProviderType, ProductSessionId, ProductStoreError,
    ProductThemePreference, ProductWorkspaceId, ProductWorkspaceKind,
    UpdateProductPreferencesRequest, UpdateProductProviderProfileRequest,
};

pub(super) const MAX_PATH_BYTES: usize = MAX_PRODUCT_PATH_BYTES;
pub(super) const MAX_RUN_BINDINGS_PER_SESSION: u64 = 10_000;
const MAX_PROVIDER_MAX_STEPS: u32 = 4_096;
const PRODUCT_PREFERENCES_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub(super) struct ValidatedWorkspace {
    pub canonical_root_text: String,
    pub canonical_key: String,
    pub kind: ProductWorkspaceKind,
    pub display_name: String,
    pub pinned: bool,
    pub last_opened_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedProviderProfile {
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    pub api_key_env: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedPreferences {
    pub schema_version: u32,
    pub expected_revision: Option<u64>,
    pub theme: ProductThemePreference,
    pub default_approval_policy: Option<ProductApprovalPreference>,
    pub active_workspace_id: Option<ProductWorkspaceId>,
    pub active_session_id: Option<ProductSessionId>,
    pub provider_selection: Option<ProductProviderSelection>,
}

pub(super) fn validate_workspace_request(
    request: CreateProductWorkspaceRequest,
    now: &str,
) -> Result<ValidatedWorkspace, ProductStoreError> {
    validate_workspace(
        &request.root,
        request.kind,
        request.display_name.as_deref(),
        request.pinned,
        now,
    )
}

pub(super) fn validate_workspace(
    root: &Path,
    kind: ProductWorkspaceKind,
    display_name: Option<&str>,
    pinned: bool,
    last_opened_at: &str,
) -> Result<ValidatedWorkspace, ProductStoreError> {
    validate_path_input(root)?;
    let runtime_workspace = match kind {
        ProductWorkspaceKind::Folder => Workspace::open_folder(root),
        ProductWorkspaceKind::Repo => Workspace::open_repo(root),
    }
    .map_err(|_| invalid("workspace root is not a valid workspace of the requested kind"))?;
    let expected_kind = match kind {
        ProductWorkspaceKind::Folder => WorkspaceKind::Folder,
        ProductWorkspaceKind::Repo => WorkspaceKind::Repo,
    };
    if runtime_workspace.kind != expected_kind {
        return Err(invalid(
            "workspace root is not a valid workspace of the requested kind",
        ));
    }

    let canonical_root_text = super::schema::path_to_utf8(&runtime_workspace.root)?.to_string();
    if canonical_root_text.len() > MAX_PRODUCT_PATH_BYTES
        || canonical_root_text.contains('\0')
        || canonical_root_text.chars().any(char::is_control)
    {
        return Err(invalid("canonical workspace root is too long or invalid"));
    }
    let fallback_name = runtime_workspace
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(canonical_root_text.as_str());
    let display_name = validate_required_text(
        "workspace display name",
        display_name.unwrap_or(fallback_name),
    )?;
    let last_opened_at = validate_timestamp("workspace last_opened_at", last_opened_at)?;

    Ok(ValidatedWorkspace {
        canonical_key: canonical_workspace_key(&canonical_root_text),
        canonical_root_text,
        kind,
        display_name,
        pinned,
        last_opened_at,
    })
}

pub(super) fn validate_title(value: Option<&str>) -> Result<String, ProductStoreError> {
    validate_required_text("session title", value.unwrap_or("New session"))
}

pub(super) fn validate_provider_create(
    request: CreateProductProviderProfileRequest,
) -> Result<ValidatedProviderProfile, ProductStoreError> {
    validate_provider(
        &request.label,
        request.provider_type,
        &request.api_base,
        request.api_key_env.as_deref(),
        request.default_model.as_deref(),
    )
}

pub(super) fn validate_provider_update(
    request: UpdateProductProviderProfileRequest,
) -> Result<ValidatedProviderProfile, ProductStoreError> {
    validate_provider(
        &request.label,
        request.provider_type,
        &request.api_base,
        request.api_key_env.as_deref(),
        request.default_model.as_deref(),
    )
}

pub(super) fn validate_provider(
    label: &str,
    provider_type: ProductProviderType,
    api_base: &str,
    api_key_env: Option<&str>,
    default_model: Option<&str>,
) -> Result<ValidatedProviderProfile, ProductStoreError> {
    let label = validate_required_text("provider profile label", label)?;
    let api_base = validate_api_base(provider_type, api_base)?;
    let api_key_env = validate_api_key_env(api_key_env)?;
    if provider_type == ProductProviderType::Fake && api_key_env.is_some() {
        return Err(invalid(
            "fake provider profiles cannot reference an API key",
        ));
    }
    let default_model = default_model
        .map(|value| validate_required_text("default model", value))
        .transpose()?;
    Ok(ValidatedProviderProfile {
        label,
        provider_type,
        api_base,
        api_key_env,
        default_model,
    })
}

pub(super) fn validate_preferences(
    request: UpdateProductPreferencesRequest,
) -> Result<ValidatedPreferences, ProductStoreError> {
    if request.schema_version != PRODUCT_PREFERENCES_SCHEMA_VERSION {
        return Err(invalid("unsupported product preferences schema version"));
    }
    if request.active_session_id.is_some() && request.active_workspace_id.is_none() {
        return Err(invalid("active_session_id requires an active_workspace_id"));
    }
    if request
        .expected_revision
        .is_some_and(|revision| revision > i64::MAX as u64)
    {
        return Err(invalid(
            "expected preference revision is outside the supported range",
        ));
    }
    let provider_selection = request
        .provider_selection
        .map(validate_provider_selection)
        .transpose()?;
    Ok(ValidatedPreferences {
        schema_version: request.schema_version,
        expected_revision: request.expected_revision,
        theme: request.theme,
        default_approval_policy: request.default_approval_policy,
        active_workspace_id: request.active_workspace_id,
        active_session_id: request.active_session_id,
        provider_selection,
    })
}

pub(super) fn validate_provider_selection(
    mut selection: ProductProviderSelection,
) -> Result<ProductProviderSelection, ProductStoreError> {
    selection.model = validate_required_text("provider selection model", &selection.model)?;
    if selection.max_steps == 0 || selection.max_steps > MAX_PROVIDER_MAX_STEPS {
        return Err(invalid("provider max_steps is outside the supported range"));
    }
    Ok(selection)
}

pub(super) fn validate_migration_envelope(
    request: &M1BrowserMigrationRequest,
    issues: &[M1MigrationIssue],
) -> Result<(), ProductStoreError> {
    if request.source_schema_version != M1_BROWSER_SOURCE_SCHEMA_VERSION {
        return Err(invalid("unsupported M1 browser source schema version"));
    }
    validate_idempotency_key(&request.idempotency_key)?;
    validate_collection_len(
        "workspaces",
        request.workspaces.len(),
        MAX_PRODUCT_WORKSPACES,
    )?;
    validate_collection_len("sessions", request.sessions.len(), MAX_PRODUCT_SESSIONS)?;
    validate_collection_len(
        "provider profiles",
        request.provider_profiles.len(),
        MAX_PRODUCT_PROVIDER_PROFILES,
    )?;

    validate_unique_source_ids(
        "workspace",
        request
            .workspaces
            .iter()
            .map(|item| item.source_id.as_str()),
    )?;
    validate_unique_source_ids(
        "session",
        request.sessions.iter().map(|item| item.source_id.as_str()),
    )?;
    validate_unique_source_ids(
        "provider profile",
        request
            .provider_profiles
            .iter()
            .map(|item| item.source_id.as_str()),
    )?;

    for workspace in &request.workspaces {
        validate_source_id("workspace source_id", &workspace.source_id)?;
        validate_required_text("workspace display name", &workspace.display_name)?;
        validate_timestamp("workspace last_opened_at", &workspace.last_opened_at)?;
        validate_path_shape(&workspace.root)?;
    }
    for session in &request.sessions {
        validate_source_id("session source_id", &session.source_id)?;
        validate_source_id("session source_workspace_id", &session.source_workspace_id)?;
        validate_required_text("session title", &session.title)?;
        validate_timestamp("session created_at", &session.created_at)?;
        validate_timestamp("session updated_at", &session.updated_at)?;
        for hint in [
            session.legacy_active_job_id.as_deref(),
            session.legacy_active_run_id.as_deref(),
            session.legacy_resumed_from_run_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_required_text("legacy runtime hint", hint)?;
        }
    }
    for profile in &request.provider_profiles {
        validate_source_id("provider profile source_id", &profile.source_id)?;
        validate_migration_provider(
            &profile.label,
            profile.provider_type,
            &profile.api_base,
            profile.api_key_env.as_deref(),
            profile.default_model.as_deref(),
        )?;
        validate_timestamp("provider profile updated_at", &profile.updated_at)?;
    }
    if let Some(source_id) = request
        .safe_preferences
        .source_active_workspace_id
        .as_deref()
    {
        validate_source_id("active workspace source_id", source_id)?;
    }
    if let Some(source_id) = request.safe_preferences.source_active_session_id.as_deref() {
        validate_source_id("active session source_id", source_id)?;
    }
    if let Some(selection) = &request.safe_preferences.provider_selection {
        if let Some(source_id) = selection.source_profile_id.as_deref() {
            validate_source_id("selected provider source_id", source_id)?;
        }
        validate_provider_selection(ProductProviderSelection {
            profile_id: None,
            model: selection.model.clone(),
            approval: selection.approval,
            max_steps: selection.max_steps,
        })?;
    }
    for issue in issues {
        validate_required_text("migration issue entity", &issue.entity)?;
        if let Some(source_id) = issue.source_id.as_deref() {
            validate_source_id("migration issue source_id", source_id)?;
        }
    }
    Ok(())
}

pub(super) fn validate_migration_provider(
    label: &str,
    provider_type: ProductProviderType,
    api_base: &str,
    api_key_env: Option<&str>,
    default_model: Option<&str>,
) -> Result<ValidatedProviderProfile, ProductStoreError> {
    let normalized_api_base =
        if provider_type == ProductProviderType::Fake && api_base.trim() == "local" {
            ""
        } else {
            api_base
        };
    validate_provider(
        label,
        provider_type,
        normalized_api_base,
        api_key_env,
        default_model,
    )
}

pub(super) fn normalized_timestamp(
    field: &'static str,
    value: &str,
) -> Result<String, ProductStoreError> {
    validate_timestamp(field, value)
}

pub(super) fn validate_source_id(
    field: &'static str,
    value: &str,
) -> Result<String, ProductStoreError> {
    validate_required_text(field, value)
}

pub(super) fn validate_issue_entity(value: &str) -> Result<String, ProductStoreError> {
    validate_required_text("migration issue entity", value)
}

fn validate_api_base(
    provider_type: ProductProviderType,
    value: &str,
) -> Result<String, ProductStoreError> {
    let value = value.trim();
    if provider_type == ProductProviderType::Fake {
        if !value.is_empty() {
            return Err(invalid("fake provider profile api_base must be empty"));
        }
        return Ok(String::new());
    }
    if value.is_empty()
        || value.len() > MAX_PRODUCT_API_BASE_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(invalid("provider api_base is empty, too long, or invalid"));
    }
    let parsed = Url::parse(value).map_err(|_| invalid("provider api_base is invalid"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(invalid(
            "provider api_base must be an HTTP URL without credentials",
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid(
            "provider api_base must not contain query parameters or a fragment",
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn validate_api_key_env(value: Option<&str>) -> Result<Option<String>, ProductStoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 256
        || (!bytes[0].is_ascii_uppercase() && bytes[0] != b'_')
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        return Err(invalid("provider API key environment name is invalid"));
    }
    Ok(Some(value.to_string()))
}

pub(super) fn validate_required_text(
    field: &'static str,
    value: &str,
) -> Result<String, ProductStoreError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PRODUCT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(invalid(format!("{field} is empty, too long, or invalid")));
    }
    Ok(value.to_string())
}

fn validate_timestamp(field: &'static str, value: &str) -> Result<String, ProductStoreError> {
    if value.is_empty() || value.len() > MAX_PRODUCT_TEXT_BYTES {
        return Err(invalid(format!("{field} is empty or too long")));
    }
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid(format!("{field} must be an RFC3339 timestamp")))?;
    Ok(parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn validate_idempotency_key(value: &str) -> Result<(), ProductStoreError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid("migration idempotency key is invalid"));
    }
    Ok(())
}

fn validate_collection_len(
    field: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), ProductStoreError> {
    if actual > maximum {
        return Err(invalid(format!("migration {field} exceeds its limit")));
    }
    Ok(())
}

fn validate_unique_source_ids<'a>(
    entity: &'static str,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), ProductStoreError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value.trim()) {
            return Err(invalid(format!(
                "migration contains duplicate {entity} source_id"
            )));
        }
    }
    Ok(())
}

fn validate_path_shape(path: &Path) -> Result<(), ProductStoreError> {
    let path_text = path.to_string_lossy();
    if path.as_os_str().is_empty()
        || path_text.len() > MAX_PRODUCT_PATH_BYTES
        || path_text.contains('\0')
        || path_text.chars().any(char::is_control)
    {
        return Err(invalid("workspace root is empty, too long, or invalid"));
    }
    Ok(())
}

fn validate_path_input(path: &Path) -> Result<(), ProductStoreError> {
    validate_path_shape(path)?;
    if !path.is_absolute() {
        return Err(invalid("workspace root must be absolute"));
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        if !matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        ) {
            return Err(invalid(
                "workspace root must use a local drive path, not a UNC or device namespace",
            ));
        }
    }
    Ok(())
}

/// The uniqueness key a canonical workspace root maps to.
///
/// Exposed to the product module so ownership recovery derives the same key the
/// create path derives, rather than storing a second copy that could drift.
pub(crate) fn canonical_workspace_key(value: &str) -> String {
    if cfg!(windows) {
        value.replace('\\', "/").to_lowercase()
    } else {
        value.to_string()
    }
}

pub(super) fn invalid(message: impl Into<String>) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductInvalidInput, message)
}

pub(super) fn profile_id_string(id: Option<&ProductProviderProfileId>) -> Option<String> {
    id.map(ToString::to_string)
}
