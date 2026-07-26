//! SQLite ProductStore implementation lane.
//!
//! Product metadata is API-global and intentionally contains no canonical
//! runtime event payloads. Every async trait operation crosses a blocking
//! boundary before opening SQLite.

mod repository;
mod schema;
mod validation;

use std::path::PathBuf;

use async_trait::async_trait;

use crate::product::{
    CommitProductRunBinding, CreateProductProviderProfileRequest, CreateProductSessionRequest,
    CreateProductWorkspaceRequest, M1BrowserMigrationResponse, PreparedM1BrowserMigration,
    ProductErrorCode, ProductPreferences, ProductProviderProfile, ProductProviderProfileId,
    ProductSession, ProductSessionContext, ProductSessionId, ProductSessionRunBinding,
    ProductSessionStatus, ProductStore, ProductStoreError, ProductTurnClaim, ProductTurnClaimId,
    ProductWorkspace, ProductWorkspaceId, UpdateProductPreferencesRequest,
    UpdateProductProviderProfileRequest, UpdateProductSessionRequest,
};

use repository::ProductRepository;
use schema::ProductDatabase;

#[derive(Debug, Clone)]
pub(crate) struct SqliteProductStore {
    repository: ProductRepository,
}

impl SqliteProductStore {
    /// Open the API-global product database synchronously.
    ///
    /// API state is also constructed outside a Tokio runtime in some callers,
    /// so schema migration and conservative stale-claim recovery happen here.
    pub(crate) fn open(
        path: impl Into<PathBuf>,
        busy_timeout_ms: u64,
    ) -> Result<Self, ProductStoreError> {
        let repository =
            ProductRepository::new(ProductDatabase::new(path.into(), busy_timeout_ms)?);
        repository.initialize_and_recover()?;
        Ok(Self { repository })
    }

    async fn blocking<T, F>(&self, operation: F) -> Result<T, ProductStoreError>
    where
        T: Send + 'static,
        F: FnOnce(ProductRepository) -> Result<T, ProductStoreError> + Send + 'static,
    {
        let repository = self.repository.clone();
        tokio::task::spawn_blocking(move || operation(repository))
            .await
            .map_err(|_| {
                ProductStoreError::new(
                    ProductErrorCode::ProductStorageFailure,
                    "product store blocking operation did not complete",
                )
            })?
    }
}

#[async_trait]
impl ProductStore for SqliteProductStore {
    async fn recover_stale_turn_claims(&self) -> Result<u64, ProductStoreError> {
        self.blocking(|repository| repository.recover_stale_turn_claims())
            .await
    }

    async fn list_workspaces(&self) -> Result<Vec<ProductWorkspace>, ProductStoreError> {
        self.blocking(|repository| repository.list_workspaces())
            .await
    }

    async fn create_workspace(
        &self,
        request: CreateProductWorkspaceRequest,
    ) -> Result<ProductWorkspace, ProductStoreError> {
        self.blocking(move |repository| repository.create_workspace(request))
            .await
    }

    async fn delete_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<(), ProductStoreError> {
        let workspace_id = workspace_id.clone();
        self.blocking(move |repository| repository.delete_workspace(&workspace_id))
            .await
    }

    async fn list_sessions(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<Vec<ProductSession>, ProductStoreError> {
        let workspace_id = workspace_id.clone();
        self.blocking(move |repository| repository.list_sessions(&workspace_id))
            .await
    }

    async fn create_session(
        &self,
        request: CreateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError> {
        self.blocking(move |repository| repository.create_session(request))
            .await
    }

    async fn update_session(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.update_session(&session_id, request))
            .await
    }

    async fn delete_session(&self, session_id: &ProductSessionId) -> Result<(), ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.delete_session(&session_id))
            .await
    }

    async fn get_session_context(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionContext, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.get_session_context(&session_id))
            .await
    }

    async fn list_run_bindings(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunBinding>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_run_bindings(&session_id))
            .await
    }

    async fn claim_session_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductTurnClaim, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.claim_session_turn(&session_id))
            .await
    }

    async fn commit_run_binding(
        &self,
        binding: CommitProductRunBinding,
    ) -> Result<ProductSessionRunBinding, ProductStoreError> {
        self.blocking(move |repository| repository.commit_run_binding(binding))
            .await
    }

    async fn finish_session_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        status: ProductSessionStatus,
    ) -> Result<(), ProductStoreError> {
        let claim_id = claim_id.clone();
        self.blocking(move |repository| repository.finish_session_turn(&claim_id, status))
            .await
    }

    async fn list_provider_profiles(
        &self,
    ) -> Result<Vec<ProductProviderProfile>, ProductStoreError> {
        self.blocking(|repository| repository.list_provider_profiles())
            .await
    }

    async fn create_provider_profile(
        &self,
        request: CreateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        self.blocking(move |repository| repository.create_provider_profile(request))
            .await
    }

    async fn update_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
        request: UpdateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        let profile_id = profile_id.clone();
        self.blocking(move |repository| repository.update_provider_profile(&profile_id, request))
            .await
    }

    async fn delete_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<(), ProductStoreError> {
        let profile_id = profile_id.clone();
        self.blocking(move |repository| repository.delete_provider_profile(&profile_id))
            .await
    }

    async fn get_preferences(&self) -> Result<ProductPreferences, ProductStoreError> {
        self.blocking(|repository| repository.get_preferences())
            .await
    }

    async fn update_preferences(
        &self,
        request: UpdateProductPreferencesRequest,
    ) -> Result<ProductPreferences, ProductStoreError> {
        self.blocking(move |repository| repository.update_preferences(request))
            .await
    }

    async fn apply_m1_browser_migration(
        &self,
        migration: PreparedM1BrowserMigration,
    ) -> Result<M1BrowserMigrationResponse, ProductStoreError> {
        self.blocking(move |repository| repository.apply_m1_browser_migration(migration))
            .await
    }
}

#[cfg(test)]
mod tests;
