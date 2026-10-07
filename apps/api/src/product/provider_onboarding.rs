use std::fmt;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use rove_app_bootstrap::{
    CredentialReference, OnboardingCredential, ProviderOnboardingError, ProviderOnboardingRequest,
    ProviderOnboardingService, ProviderProbeFailureKind, ProviderProfileId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use zeroize::Zeroizing;

use super::routes::product_json;
use super::{ProductProviderProfileId, ProductProviderType};
use crate::{ApiError, ApiErrorResponse, ApiState, docs};

/// Safe metadata accepted by the in-process Desktop onboarding facade.
///
/// The raw credential is deliberately a separate `&str` argument on
/// [`ApiState::onboard_product_provider`]. This type therefore cannot become
/// an accidental HTTP or WebView secret payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductProviderOnboardingRequest {
    pub profile_id: Option<ProductProviderProfileId>,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    pub model: String,
    pub make_default: bool,
    pub expected_revision: Option<String>,
}

/// The HTTP-visible onboarding body — the only Product payload that carries a
/// secret. The credential is accepted transiently, handed straight to the OS
/// keyring by the shared onboarding service, and never read back, logged, or
/// serialized into any response. `write_only` marks it in the spec so generated
/// clients know it is not returned.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OnboardProductProviderRequest {
    /// Reuse an existing catalog profile id to rotate or repair its credential.
    #[serde(default)]
    pub profile_id: Option<ProductProviderProfileId>,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    pub model: String,
    #[serde(default = "default_make_default")]
    pub make_default: bool,
    #[serde(default)]
    pub expected_revision: Option<String>,
    /// The raw provider key. Bounded, never persisted outside the OS keyring.
    #[schema(write_only = true)]
    pub credential: String,
}

fn default_make_default() -> bool {
    true
}

/// A pasted credential is the whole point of this route; bound it to the
/// largest plausible key rather than letting a request body grow unchecked.
const MAX_PROVIDER_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_ONBOARDING_LABEL_BYTES: usize = 256;
const MAX_ONBOARDING_MODEL_BYTES: usize = 1_024;
const MAX_ONBOARDING_API_BASE_BYTES: usize = 2_048;

#[utoipa::path(
    post,
    path = "/product/provider-onboarding",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    request_body = OnboardProductProviderRequest,
    responses(
        (status = 201, description = "Credential stored in the OS keyring and the provider profile published", body = ProductProviderOnboardingReceipt),
        (status = 400, description = "Invalid onboarding request", body = ApiErrorResponse),
        (status = 403, description = "Credential entry requires a loopback-bound API", body = ApiErrorResponse),
        (status = 409, description = "Provider catalog revision conflict or reconciliation required", body = ApiErrorResponse),
        (status = 429, description = "Provider inventory was rate limited", body = ApiErrorResponse),
        (status = 500, description = "Provider catalog operation failed", body = ApiErrorResponse),
        (status = 502, description = "Provider inventory failed", body = ApiErrorResponse),
        (status = 503, description = "OS credential storage is unavailable", body = ApiErrorResponse),
        (status = 504, description = "Provider inventory timed out", body = ApiErrorResponse)
    )
)]
pub(crate) async fn onboard_product_provider(
    State(state): State<ApiState>,
    body: Result<Json<OnboardProductProviderRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductProviderOnboardingReceipt>), ApiError> {
    require_loopback_bind(&state)?;
    let request = product_json(body)?;
    validate_onboarding_body(&request)?;

    // The secret leaves the request envelope here and only here: registered
    // for redaction inside `onboard`, written to the keyring by the shared
    // service, then zeroized. It never touches ProductStore or a response.
    let secret = Zeroizing::new(request.credential);
    let receipt = state
        .onboard_product_provider(
            ProductProviderOnboardingRequest {
                profile_id: request.profile_id,
                label: request.label,
                provider_type: request.provider_type,
                api_base: request.api_base,
                model: request.model,
                make_default: request.make_default,
                expected_revision: request.expected_revision,
            },
            secret.as_str(),
        )
        .await
        .map_err(map_onboarding_http_failure)?;
    Ok((StatusCode::CREATED, Json(receipt)))
}

/// Credential entry is loopback-only: a remotely reachable socket would let
/// whoever holds the bearer token push secrets into this machine's keyring.
/// Checking the configured bind is equivalent to checking the remote peer —
/// a loopback listener is unreachable from anywhere but this machine, and a
/// non-loopback listener cannot distinguish local from remote callers.
fn require_loopback_bind(state: &ApiState) -> Result<(), ApiError> {
    let is_loopback = state
        .inner
        .config
        .api
        .bind_addr
        .parse::<SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false);
    if !is_loopback {
        return Err(ApiError::forbidden_with_code(
            "provider_onboarding_loopback_required",
            "provider credential entry is only available when the API binds a loopback address",
        ));
    }
    Ok(())
}

fn validate_onboarding_body(request: &OnboardProductProviderRequest) -> Result<(), ApiError> {
    let invalid = |message: &'static str| {
        ApiError::bad_request_with_code("provider_onboarding_invalid", message)
    };
    if request.credential.trim().is_empty() {
        return Err(invalid("provider credential cannot be empty"));
    }
    if request.credential.len() > MAX_PROVIDER_CREDENTIAL_BYTES {
        return Err(invalid("provider credential is too large"));
    }
    if request.label.trim().is_empty() || request.label.len() > MAX_ONBOARDING_LABEL_BYTES {
        return Err(invalid("provider label is invalid"));
    }
    if request.model.trim().is_empty() || request.model.len() > MAX_ONBOARDING_MODEL_BYTES {
        return Err(invalid("provider model is invalid"));
    }
    if request.api_base.trim().is_empty() || request.api_base.len() > MAX_ONBOARDING_API_BASE_BYTES
    {
        return Err(invalid("provider api base is invalid"));
    }
    Ok(())
}

/// Map the typed onboarding failure onto the API error envelope. The failure
/// codes are already the outward contract (the Desktop command maps the same
/// set); only the HTTP status is chosen here.
fn map_onboarding_http_failure(failure: ProductProviderOnboardingFailure) -> ApiError {
    let message = failure.message;
    match failure.code.as_str() {
        "provider_onboarding_invalid" => {
            ApiError::bad_request_with_code("provider_onboarding_invalid", message)
        }
        "provider_authentication" => {
            ApiError::bad_request_with_code("provider_authentication", message)
        }
        "provider_model_unavailable" => {
            ApiError::bad_request_with_code("provider_model_unavailable", message)
        }
        "provider_rate_limited" => {
            ApiError::too_many_requests_with_code("provider_rate_limited", message)
        }
        "provider_upstream" => ApiError::bad_gateway_with_code("provider_upstream", message),
        "provider_transport" => ApiError::bad_gateway_with_code("provider_transport", message),
        "provider_protocol_mismatch" => {
            ApiError::bad_gateway_with_code("provider_protocol_mismatch", message)
        }
        "provider_timeout" => ApiError::gateway_timeout_with_code("provider_timeout", message),
        "provider_credential_store" => {
            ApiError::service_unavailable_with_code("provider_credential_store", message)
        }
        "product_revision_conflict" => {
            ApiError::conflict_with_code("product_revision_conflict", message)
        }
        "provider_reconciliation_required" => {
            ApiError::conflict_with_code("provider_reconciliation_required", message)
        }
        "provider_product_projection" => {
            ApiError::conflict_with_code("provider_product_projection", message)
        }
        "provider_catalog" => ApiError::internal(message),
        _ => ApiError::internal("provider onboarding failed"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct ProductProviderOnboardingProbe {
    pub inventory_count: usize,
    pub streaming_supported: bool,
    pub native_tool_calls_supported: bool,
    pub usage_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductProviderCatalogSelectionReceipt {
    pub profile_id: ProductProviderProfileId,
    pub model: String,
    pub catalog_revision: String,
}

/// Secret-free result safe to return across the Tauri command boundary or as
/// an HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct ProductProviderOnboardingReceipt {
    pub profile_id: ProductProviderProfileId,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    pub model: String,
    pub catalog_revision: String,
    pub credential_source: String,
    pub probe: ProductProviderOnboardingProbe,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductProviderOnboardingFailureCode {
    Invalid,
    CredentialStore,
    Authentication,
    RateLimited,
    Upstream,
    Timeout,
    Transport,
    Protocol,
    ModelUnavailable,
    RevisionConflict,
    Catalog,
    ReconciliationRequired,
    ProductProjection,
}

impl ProductProviderOnboardingFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "provider_onboarding_invalid",
            Self::CredentialStore => "provider_credential_store",
            Self::Authentication => "provider_authentication",
            Self::RateLimited => "provider_rate_limited",
            Self::Upstream => "provider_upstream",
            Self::Timeout => "provider_timeout",
            Self::Transport => "provider_transport",
            Self::Protocol => "provider_protocol_mismatch",
            Self::ModelUnavailable => "provider_model_unavailable",
            Self::RevisionConflict => "product_revision_conflict",
            Self::Catalog => "provider_catalog",
            Self::ReconciliationRequired => "provider_reconciliation_required",
            Self::ProductProjection => "provider_product_projection",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductProviderOnboardingFailure {
    pub code: String,
    pub message: String,
}

impl ProductProviderOnboardingFailure {
    fn new(code: ProductProviderOnboardingFailureCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str().to_string(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ProductProviderOnboardingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProductProviderOnboardingFailure {}

pub(crate) async fn onboard(
    state: &ApiState,
    service: ProviderOnboardingService,
    request: ProductProviderOnboardingRequest,
    secret: &str,
) -> Result<ProductProviderOnboardingReceipt, ProductProviderOnboardingFailure> {
    if secret.trim().is_empty() {
        return Err(ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            "provider credential cannot be empty",
        ));
    }
    let provider_type = super::provider_catalog::provider_type_name(request.provider_type);
    if !matches!(provider_type, "openai" | "openai-responses" | "anthropic") {
        return Err(ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            "native credential onboarding supports remote authenticated providers only",
        ));
    }
    // The raw credential reaches this process here and is handed straight to
    // the keyring. Registering it first means a failure anywhere below — a
    // rejected profile id, a keyring write error, a projection failure —
    // cannot echo the credential into an error envelope or a trace.
    rove_runtime::secrets::registry().register_value(secret);
    let product_profile_id = request.profile_id.unwrap_or_default();
    let profile_id = ProviderProfileId::new(product_profile_id.to_string()).map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            "provider profile id is invalid",
        )
    })?;
    let label = request.label.trim().to_string();

    let receipt = service
        .onboard(ProviderOnboardingRequest {
            profile_id,
            label: label.clone(),
            provider_type: provider_type.to_string(),
            base_url: request.api_base,
            model: request.model,
            credential: OnboardingCredential::Secret(secret.to_string()),
            make_default: request.make_default,
            expected_revision: request.expected_revision,
        })
        .await
        .map_err(map_onboarding_error)?;

    if !matches!(receipt.credential, CredentialReference::Keyring { .. }) {
        return Err(ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "provider credential publication did not resolve to the OS keyring",
        ));
    }

    let catalog = service.catalog().load().map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "provider catalog could not be reloaded after publication",
        )
    })?;
    let profile = super::provider_catalog::get(&catalog, &product_profile_id).map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "published provider profile could not be projected",
        )
    })?;

    state
        .product_store()
        .map_err(|_| map_product_projection_error("ProductStore is unavailable"))?
        .upsert_provider_catalog_identity(
            &profile.id,
            &profile.label,
            profile.provider_type,
            &profile.catalog_revision,
        )
        .await
        .map_err(|error| map_product_projection_error(error.to_string()))?;

    Ok(ProductProviderOnboardingReceipt {
        profile_id: product_profile_id,
        label: profile.label,
        provider_type: profile.provider_type,
        api_base: receipt.base_url,
        model: receipt.model,
        catalog_revision: receipt.catalog_revision,
        credential_source: "keyring".to_string(),
        probe: ProductProviderOnboardingProbe {
            inventory_count: receipt.probe.inventory_count,
            streaming_supported: receipt.probe.streaming_supported,
            native_tool_calls_supported: receipt.probe.native_tool_calls_supported,
            usage_supported: receipt.probe.usage_supported,
        },
        selected: receipt.selected,
    })
}

pub(crate) async fn probe(
    service: ProviderOnboardingService,
    profile_id: ProductProviderProfileId,
    model_override: Option<String>,
) -> Result<ProductProviderOnboardingProbe, ProductProviderOnboardingFailure> {
    let catalog_profile_id = ProviderProfileId::new(profile_id.to_string()).map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            "provider profile id is invalid",
        )
    })?;
    let receipt = service
        .probe(&catalog_profile_id, model_override.as_deref())
        .await
        .map_err(map_onboarding_error)?;
    Ok(ProductProviderOnboardingProbe {
        inventory_count: receipt.inventory_count,
        streaming_supported: receipt.streaming_supported,
        native_tool_calls_supported: receipt.native_tool_calls_supported,
        usage_supported: receipt.usage_supported,
    })
}

pub(crate) async fn use_profile(
    state: &ApiState,
    service: ProviderOnboardingService,
    profile_id: ProductProviderProfileId,
    model_override: Option<String>,
    expected_revision: Option<String>,
) -> Result<ProductProviderCatalogSelectionReceipt, ProductProviderOnboardingFailure> {
    let catalog_profile_id = ProviderProfileId::new(profile_id.to_string()).map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            "provider profile id is invalid",
        )
    })?;
    let use_service = service.clone();
    let use_profile_id = catalog_profile_id.clone();
    let selection = tokio::task::spawn_blocking(move || {
        use_service.use_profile(
            &use_profile_id,
            model_override.as_deref(),
            expected_revision.as_deref(),
        )
    })
    .await
    .map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Catalog,
            "provider catalog operation did not complete",
        )
    })?
    .map_err(map_onboarding_error)?;
    let catalog = service.catalog().load().map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "provider catalog could not be reloaded after selection",
        )
    })?;
    let profile = super::provider_catalog::get(&catalog, &profile_id).map_err(|_| {
        ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "selected provider profile could not be projected",
        )
    })?;
    state
        .product_store()
        .map_err(|_| map_product_projection_error("ProductStore is unavailable"))?
        .upsert_provider_catalog_identity(
            &profile.id,
            &profile.label,
            profile.provider_type,
            &profile.catalog_revision,
        )
        .await
        .map_err(|error| map_product_projection_error(error.to_string()))?;
    Ok(ProductProviderCatalogSelectionReceipt {
        profile_id,
        model: selection.model,
        catalog_revision: selection.revision,
    })
}

fn map_product_projection_error(error: impl Into<String>) -> ProductProviderOnboardingFailure {
    ProductProviderOnboardingFailure::new(
        ProductProviderOnboardingFailureCode::ProductProjection,
        format!(
            "provider was published but Product state requires reconciliation: {}",
            error.into()
        ),
    )
}

fn map_onboarding_error(error: ProviderOnboardingError) -> ProductProviderOnboardingFailure {
    match error {
        ProviderOnboardingError::Invalid(message) => ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Invalid,
            message,
        ),
        ProviderOnboardingError::CredentialStore => ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::CredentialStore,
            "Windows credential storage is unavailable",
        ),
        ProviderOnboardingError::Probe { kind } => match kind {
            ProviderProbeFailureKind::Unauthorized => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::Authentication,
                "Provider authentication failed",
            ),
            ProviderProbeFailureKind::RateLimited => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::RateLimited,
                "Provider inventory was rate limited",
            ),
            ProviderProbeFailureKind::Upstream => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::Upstream,
                "Provider inventory returned an upstream failure",
            ),
            ProviderProbeFailureKind::Timeout => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::Timeout,
                "Provider inventory timed out",
            ),
            ProviderProbeFailureKind::Transport => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::Transport,
                "Provider could not be reached",
            ),
            ProviderProbeFailureKind::InvalidResponse => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::Protocol,
                "Provider returned an incompatible model catalog",
            ),
            ProviderProbeFailureKind::ModelUnavailable => ProductProviderOnboardingFailure::new(
                ProductProviderOnboardingFailureCode::ModelUnavailable,
                "Selected model was not present in Provider inventory",
            ),
        },
        ProviderOnboardingError::RevisionConflict => ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::RevisionConflict,
            "Provider catalog changed; reload Settings and retry",
        ),
        ProviderOnboardingError::Catalog(_) => ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::Catalog,
            "Provider catalog publication failed",
        ),
        ProviderOnboardingError::ReconciliationRequired => ProductProviderOnboardingFailure::new(
            ProductProviderOnboardingFailureCode::ReconciliationRequired,
            "Provider publication requires reconciliation",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use rove_app_bootstrap::{
        AppConfig, AppConfigOverrides, ProviderCatalogService, ProviderCredentialStore,
        UserConfigPaths,
    };
    use rove_runtime::workspace::Workspace;

    use super::*;

    #[derive(Default)]
    struct RecordingCredentialStore {
        puts: Mutex<Vec<(String, String, String)>>,
    }

    #[async_trait]
    impl ProviderCredentialStore for RecordingCredentialStore {
        async fn put(&self, service: &str, account: &str, secret: &str) -> Result<(), ()> {
            self.puts.lock().unwrap().push((
                service.to_string(),
                account.to_string(),
                secret.to_string(),
            ));
            Ok(())
        }

        async fn delete(&self, _service: &str, _account: &str) -> Result<(), ()> {
            Ok(())
        }
    }

    fn one_response_server(body: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_string();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{address}/v1"), thread)
    }

    #[tokio::test]
    async fn secure_onboarding_projects_catalog_identity_without_serializing_the_secret() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).unwrap();
        let user_paths = UserConfigPaths::from_root(temp.path().join("user"));
        let config = AppConfig::load_with_user_config_paths(
            &workspace_root,
            AppConfigOverrides {
                data_root: Some(temp.path().join("data")),
                ..AppConfigOverrides::default()
            },
            user_paths.clone(),
        )
        .unwrap();
        let state = ApiState::new(Workspace::open_folder(&workspace_root).unwrap(), config);
        let catalog = ProviderCatalogService::new(user_paths);
        let credentials = Arc::new(RecordingCredentialStore::default());
        let service =
            ProviderOnboardingService::with_credential_store(catalog, credentials.clone());
        let (api_base, server) =
            one_response_server(r#"{"data":[{"id":"deepseek-ai/DeepSeek-V3.2"}]}"#);
        let secret = "desktop-provider-secret-canary";

        let receipt = onboard(
            &state,
            service,
            ProductProviderOnboardingRequest {
                profile_id: Some(
                    ProductProviderProfileId::from_catalog_id("siliconflow-desktop").unwrap(),
                ),
                label: "SiliconFlow Desktop".to_string(),
                provider_type: ProductProviderType::Openai,
                api_base,
                model: "deepseek-ai/DeepSeek-V3.2".to_string(),
                make_default: true,
                expected_revision: None,
            },
            secret,
        )
        .await
        .unwrap();
        server.join().unwrap();

        assert_eq!(receipt.credential_source, "keyring");
        assert!(receipt.selected);
        assert!(receipt.probe.native_tool_calls_supported);
        assert!(!serde_json::to_string(&receipt).unwrap().contains(secret));
        assert_eq!(credentials.puts.lock().unwrap().len(), 1);
        let projected = state
            .product_store()
            .unwrap()
            .get_provider_profile(&receipt.profile_id)
            .await
            .unwrap();
        assert_eq!(projected.label, "SiliconFlow Desktop");
        assert!(projected.api_base.is_empty());
        assert!(projected.api_key_env.is_none());
    }

    #[test]
    fn onboarding_errors_are_typed_and_redacted() {
        let failure = map_onboarding_error(ProviderOnboardingError::Probe {
            kind: ProviderProbeFailureKind::Unauthorized,
        });
        assert_eq!(failure.code, "provider_authentication");
        assert!(!failure.message.contains("secret"));
        assert!(
            !serde_json::to_string(&failure)
                .unwrap()
                .contains("Authorization")
        );
    }
}
