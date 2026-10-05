//! `ApiState` construction and its accessors.

use super::*;

impl ApiState {
    /// The loopback address of the isolated preview origin, when bound.
    /// Tests and hosts use it to build preview URLs without guessing ports.
    pub async fn preview_origin_addr(&self) -> Option<SocketAddr> {
        self.inner.preview.origin_addr().await
    }
}

impl ApiState {
    pub fn new(workspace: Workspace, config: AppConfig) -> Self {
        Self::with_shutdown(workspace, config, CancellationToken::new())
    }

    pub fn with_shutdown(
        workspace: Workspace,
        config: AppConfig,
        shutdown_token: CancellationToken,
    ) -> Self {
        let project_trust = match ProjectTrustRepository::operator_default() {
            Ok(repository) => Some(Arc::new(repository)),
            Err(error) => {
                tracing::warn!("project trust authority is unavailable: {error}");
                None
            }
        };
        Self::with_shutdown_and_project_trust(workspace, config, shutdown_token, project_trust)
    }

    /// Build API state with an explicit canonical Project Trust authority.
    /// Embedders and tests can use the same repository instance as CLI or
    /// bootstrap without relying on process-global path overrides.
    pub fn with_project_trust_repository(
        workspace: Workspace,
        config: AppConfig,
        project_trust: Arc<ProjectTrustRepository>,
    ) -> Self {
        Self::with_shutdown_and_project_trust(
            workspace,
            config,
            CancellationToken::new(),
            Some(project_trust),
        )
    }

    pub(crate) fn with_shutdown_and_project_trust(
        workspace: Workspace,
        mut config: AppConfig,
        shutdown_token: CancellationToken,
        project_trust: Option<Arc<ProjectTrustRepository>>,
    ) -> Self {
        if config.source_summary.workspace_root == std::path::Path::new(".") {
            config.source_summary.workspace_root = workspace.root.clone();
            config.source_summary.project_config_path = workspace.root.join(".rove/config.toml");
        }
        // The API bearer token is a credential this process holds and already
        // compares against a request header. Registering it once at assembly
        // means a value that matches no pattern is still removed from a trace,
        // a report, an SSE frame, an error envelope, or a diagnostics response.
        if let Some(token) = config.api.token_auth.as_deref() {
            rove_runtime::secrets::registry().register_value(token);
        }
        let state_store = state_store_for_parts(&workspace, &config);
        if let Err(err) = state_store.index.initialize() {
            tracing::warn!("failed to initialize API state index: {err}");
        }
        if let Err(err) = state_store.index.mark_running_jobs_interrupted() {
            tracing::warn!("failed to mark stale API jobs interrupted: {err}");
        }
        spawn_state_index_backfill(&workspace, &config);
        let product_store_path = config.product_sqlite_path();
        let provider_catalog = ProviderCatalogService::new(UserConfigPaths::for_config_file(
            &config.source_summary.user_config_path,
        ));
        let product_store = match product::store::open_product_store(
            product_store_path.clone(),
            config.state.sqlite_busy_timeout_ms,
        ) {
            Ok(store) => Some(store),
            Err(err) => {
                tracing::warn!("product store is unavailable: {err}");
                None
            }
        };
        if let Some(repository) = project_trust.as_ref()
            && let Err(error) = repository.import_product_store_snapshot(&product_store_path)
        {
            tracing::warn!("project trust compatibility import failed: {error}");
        }
        let product_transcript_reader = product_store.as_ref().and_then(|store| {
            let runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver> =
                Arc::new(ApiProductRuntimeStateResolver {
                    config: config.clone(),
                });
            match product::transcript::open_product_transcript_reader(
                Arc::clone(store),
                runtime_state_resolver,
            ) {
                Ok(reader) => Some(reader),
                Err(err) => {
                    tracing::warn!("product transcript reader is unavailable: {err}");
                    None
                }
            }
        });
        if let Some(store) = product_store.as_ref() {
            spawn_product_ownership_recovery(Arc::clone(store), &workspace, &config);
        }
        let model_health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        }));
        let attachment_storage = product::attachments::AttachmentStorage::new(
            product::attachments::attachments_root(&product_store_path),
        );
        if let Some(store) = product_store.as_ref() {
            spawn_attachment_cleanup(
                Arc::clone(store),
                attachment_storage.clone(),
                shutdown_token.clone(),
            );
        }
        Self {
            inner: Arc::new(ApiStateInner {
                workspace,
                config,
                product_store_path,
                attachment_storage,
                attachment_uploads: Arc::new(tokio::sync::Semaphore::new(
                    product::MAX_CONCURRENT_PRODUCT_ATTACHMENT_UPLOADS,
                )),
                product_store,
                provider_catalog,
                project_trust,
                product_transcript_reader,
                preview: product::preview::PreviewRegistry::new(),
                shutdown_token,
                product_events: broadcast::channel(PRODUCT_EVENT_NOTIFY_BUFFER).0,
                job_starts: TaskTracker::new(),
                supervisors: TaskTracker::new(),
                followup_start_attempts: Mutex::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                review_jobs: RwLock::new(HashMap::new()),
                mcp_health: RwLock::new(HashMap::new()),
                model_health,
                rate_limit: tokio::sync::Mutex::new(RateLimitState::default()),
                bench_runs: Arc::new(BenchState::default()),
            }),
        }
    }

    /// Stable API-global ProductStore location. Requests cannot override it.
    pub fn product_store_path(&self) -> &FsPath {
        &self.inner.product_store_path
    }

    /// The process-wide shutdown signal.
    ///
    /// A long-lived stream ends with the server instead of parking forever, so
    /// a test or a shutdown cannot hang on an open `/product/events` connection.
    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.inner.shutdown_token.clone()
    }

    /// Subscribe to product-directory change nudges.
    ///
    /// The nudge is only a wake-up: the subscriber re-reads the durable event
    /// log from its own cursor, so missing one never loses an event.
    pub(crate) fn subscribe_product_events(&self) -> broadcast::Receiver<()> {
        self.inner.product_events.subscribe()
    }

    /// Wake every open product event stream.
    ///
    /// Call this after a product mutation committed, from the path that
    /// performed it. A send with no subscribers is not an error.
    pub(crate) fn notify_product_events(&self) {
        let _ = self.inner.product_events.send(());
    }

    /// Secure in-process Provider onboarding used by native delivery hosts.
    /// The credential is never accepted by an HTTP route or serialized Product
    /// request; the shared onboarding service owns keyring storage, probing,
    /// Catalog CAS publication, and failure compensation.
    pub async fn onboard_product_provider(
        &self,
        request: ProductProviderOnboardingRequest,
        secret: &str,
    ) -> Result<ProductProviderOnboardingReceipt, ProductProviderOnboardingFailure> {
        let service =
            rove_app_bootstrap::ProviderOnboardingService::new(self.provider_catalog_service());
        product::provider_onboarding::onboard(self, service, request, secret).await
    }

    /// Re-run the shared Provider inventory probe without serializing a
    /// credential. Native Settings uses this for an already-published profile.
    pub async fn probe_product_provider(
        &self,
        profile_id: ProductProviderProfileId,
        model_override: Option<String>,
    ) -> Result<ProductProviderOnboardingProbe, ProductProviderOnboardingFailure> {
        let service =
            rove_app_bootstrap::ProviderOnboardingService::new(self.provider_catalog_service());
        product::provider_onboarding::probe(service, profile_id, model_override).await
    }

    /// Persist the shared Catalog's default selection and ensure the Product
    /// identity stub is present before the caller writes Product preferences.
    pub async fn use_product_provider(
        &self,
        profile_id: ProductProviderProfileId,
        model_override: Option<String>,
        expected_revision: Option<String>,
    ) -> Result<ProductProviderCatalogSelectionReceipt, ProductProviderOnboardingFailure> {
        let service =
            rove_app_bootstrap::ProviderOnboardingService::new(self.provider_catalog_service());
        product::provider_onboarding::use_profile(
            self,
            service,
            profile_id,
            model_override,
            expected_revision,
        )
        .await
    }

    pub(crate) fn product_store(&self) -> Result<Arc<dyn ProductStore>, ApiError> {
        self.inner
            .product_store
            .clone()
            .ok_or_else(|| ProductStoreError::unavailable().into())
    }

    /// The attachment payload tree for this API's data root.
    pub(crate) fn attachment_storage(&self) -> product::attachments::AttachmentStorage {
        self.inner.attachment_storage.clone()
    }

    /// Take one of the process-wide upload slots, or refuse so the caller can
    /// answer `429` instead of queueing an unbounded amount of request body.
    pub(crate) fn try_acquire_attachment_upload(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
        Arc::clone(&self.inner.attachment_uploads)
            .try_acquire_owned()
            .map_err(|_| {
                ApiError::too_many_requests_with_code(
                    product::ProductErrorCode::ProductAttachmentBusy.as_str(),
                    "too many concurrent attachment uploads",
                )
            })
    }

    pub(crate) fn provider_catalog_service(&self) -> ProviderCatalogService {
        self.inner.provider_catalog.clone()
    }

    pub(crate) async fn provider_catalog(&self) -> Result<ProviderCatalog, ApiError> {
        let service = self.provider_catalog_service();
        tokio::task::spawn_blocking(move || service.load())
            .await
            .map_err(|_| ApiError::internal("provider catalog operation did not complete"))?
            .map_err(product::provider_catalog::catalog_error)
    }

    pub(crate) fn project_trust(&self) -> Result<Arc<ProjectTrustRepository>, ApiError> {
        self.inner.project_trust.clone().ok_or_else(|| {
            ProductStoreError::new(
                ProductErrorCode::ProjectTrustUnavailable,
                "project trust authority is not available",
            )
            .into()
        })
    }

    pub(crate) async fn quarantine_workspace_jobs(&self, workspace_root: &FsPath) {
        let jobs = self.inner.jobs.read().await;
        for record in jobs.values() {
            if record.workspace.root == workspace_root {
                record.cancel_token.cancel();
            }
        }
    }

    /// Count one failed start of a queued successor and return its attempt
    /// number, where the initial failed start is attempt 1.
    pub(crate) async fn record_followup_start_failure(&self, control_id: &ProductControlId) -> u32 {
        let mut attempts = self.inner.followup_start_attempts.lock().await;
        if attempts.len() >= FOLLOWUP_REDRIVE_MAX_TRACKED && !attempts.contains_key(control_id) {
            // Only reachable under churn: a successor is forgotten as soon as it
            // starts or escalates, so a full map means many are failing at once.
            // Dropping the whole map re-arms those budgets rather than letting
            // the bookkeeping grow without a bound.
            attempts.clear();
        }
        let attempt = attempts.entry(control_id.clone()).or_insert(0);
        *attempt += 1;
        *attempt
    }

    /// Forget a successor's failed start attempts: it started, it escalated, or
    /// its fate moved back to the store.
    pub(crate) async fn clear_followup_start_failures(&self, control_id: &ProductControlId) {
        self.inner
            .followup_start_attempts
            .lock()
            .await
            .remove(control_id);
    }

    pub(crate) fn product_transcript_reader(
        &self,
    ) -> Result<Arc<dyn ProductTranscriptReader>, ApiError> {
        self.inner
            .product_transcript_reader
            .clone()
            .ok_or_else(|| ProductStoreError::unavailable().into())
    }

    pub(crate) fn product_state_store_for_workspace(&self, workspace: &Workspace) -> StateStore {
        ApiProductRuntimeStateResolver {
            config: self.inner.config.clone(),
        }
        .state_store_for_workspace(workspace.clone())
    }

    pub(crate) fn product_state_store_for_product_workspace(
        &self,
        workspace: &ProductWorkspace,
    ) -> Result<StateStore, ProductStoreError> {
        ApiProductRuntimeStateResolver {
            config: self.inner.config.clone(),
        }
        .state_store_for(workspace)
    }
}
