//! SQLite ProductStore implementation lane.
//!
//! Product metadata is API-global and intentionally contains no canonical
//! runtime event payloads. Every async trait operation crosses a blocking
//! boundary before opening SQLite.

mod repository;
mod schema;
mod validation;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::product::{
    CommitProductRunBinding, CreateProductControlRequest, CreateProductForkRequest,
    CreateProductMessageRequest, CreateProductProviderProfileRequest, CreateProductReviewRecord,
    CreateProductSessionRequest, CreateProductWorkspaceRequest, M1BrowserMigrationPreflight,
    M1BrowserMigrationRequest, M1BrowserMigrationResponse, PreparedM1BrowserMigration,
    ProductControl, ProductControlId, ProductControlKind, ProductControlStatus, ProductErrorCode,
    ProductFollowupTurnClaim, ProductFork, ProductMessage, ProductMessagePage,
    ProductMessagePageQuery, ProductPreferences, ProductProviderProfile, ProductProviderProfileId,
    ProductResumeHealth, ProductReview, ProductReviewFindingsQuery, ProductReviewFindingsResponse,
    ProductReviewId, ProductSession, ProductSessionContext, ProductSessionId,
    ProductSessionModelConfig, ProductSessionRunBinding, ProductSessionRunModelView,
    ProductSessionStatus, ProductStore, ProductStoreError, ProductTurnClaim, ProductTurnClaimId,
    ProductTurnControlFinish, ProductWorkspace, ProductWorkspaceId,
    UpdateProductPreferencesRequest, UpdateProductProviderProfileRequest,
    UpdateProductSessionModelConfigRequest, UpdateProductSessionRequest,
    VerifiedProductForkBoundary,
};
use rove_runtime::types::RunId;

use repository::ProductRepository;
use schema::ProductDatabase;

#[derive(Debug, Clone)]
struct SqliteProductStore {
    repository: ProductRepository,
}

impl SqliteProductStore {
    /// Open the API-global product database synchronously.
    ///
    /// API state is also constructed outside a Tokio runtime in some callers,
    /// so schema migration and conservative stale-claim recovery happen here.
    fn open(path: impl Into<PathBuf>, busy_timeout_ms: u64) -> Result<Self, ProductStoreError> {
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

/// Construct the API-global product store for coordinator-owned state wiring.
pub(crate) fn open_product_store(
    path: PathBuf,
    busy_timeout_ms: u64,
) -> Result<Arc<dyn ProductStore>, ProductStoreError> {
    Ok(Arc::new(SqliteProductStore::open(path, busy_timeout_ms)?))
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

    async fn get_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<ProductWorkspace, ProductStoreError> {
        let workspace_id = workspace_id.clone();
        self.blocking(move |repository| repository.get_workspace(&workspace_id))
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

    async fn get_session_model_config(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionModelConfig, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.get_session_model_config(&session_id))
            .await
    }

    async fn update_session_model_config(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionModelConfigRequest,
    ) -> Result<ProductSessionModelConfig, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| {
            repository.update_session_model_config(&session_id, request)
        })
        .await
    }

    async fn list_session_run_models(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunModelView>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_session_run_models(&session_id))
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

    async fn create_review(
        &self,
        record: CreateProductReviewRecord,
    ) -> Result<(ProductReview, bool), ProductStoreError> {
        self.blocking(move |repository| repository.create_review(record))
            .await
    }

    async fn list_reviews(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductReview>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_reviews(&session_id))
            .await
    }

    async fn get_review(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.get_review(&review_id))
            .await
    }

    async fn bind_review_runtime(
        &self,
        review_id: &ProductReviewId,
        runtime_session_id: rove_runtime::types::SessionId,
        job_id: rove_runtime::types::JobId,
        run_id: rove_runtime::types::RunId,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| {
            repository.bind_review_runtime(&review_id, runtime_session_id, job_id, run_id)
        })
        .await
    }

    async fn finalize_review(
        &self,
        review_id: &ProductReviewId,
        result: rove_runtime::review::ReviewResult,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.finalize_review(&review_id, result))
            .await
    }

    async fn cancel_review(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.cancel_review(&review_id))
            .await
    }

    async fn mark_review_needs_attention(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.mark_review_needs_attention(&review_id))
            .await
    }

    async fn mark_review_unavailable(
        &self,
        review_id: &ProductReviewId,
    ) -> Result<ProductReview, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.mark_review_unavailable(&review_id))
            .await
    }

    async fn list_review_findings(
        &self,
        review_id: &ProductReviewId,
        query: ProductReviewFindingsQuery,
    ) -> Result<ProductReviewFindingsResponse, ProductStoreError> {
        let review_id = review_id.clone();
        self.blocking(move |repository| repository.list_review_findings(&review_id, query))
            .await
    }

    async fn create_fork(
        &self,
        request: CreateProductForkRequest,
        boundary: VerifiedProductForkBoundary,
    ) -> Result<(ProductSession, ProductFork, bool), ProductStoreError> {
        self.blocking(move |repository| repository.create_fork(request, boundary))
            .await
    }

    async fn replay_fork(
        &self,
        parent_session_id: &ProductSessionId,
        request: &CreateProductForkRequest,
    ) -> Result<Option<(ProductSession, ProductFork)>, ProductStoreError> {
        let parent_session_id = parent_session_id.clone();
        let request = request.clone();
        self.blocking(move |repository| repository.replay_fork(&parent_session_id, &request))
            .await
    }

    async fn list_forks(
        &self,
        parent_session_id: &ProductSessionId,
    ) -> Result<Vec<ProductFork>, ProductStoreError> {
        let parent_session_id = parent_session_id.clone();
        self.blocking(move |repository| repository.list_forks(&parent_session_id))
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

    async fn finish_session_turn_and_claim_followup(
        &self,
        claim_id: &ProductTurnClaimId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError> {
        let claim_id = claim_id.clone();
        self.blocking(move |repository| {
            repository.finish_session_turn_and_claim_followup(&claim_id)
        })
        .await
    }

    async fn drop_unapplied_steers_for_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: RunId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let claim_id = claim_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| {
            repository.drop_unapplied_steers_for_turn(&claim_id, run_id, &reason)
        })
        .await
    }

    async fn finish_session_turn_and_abandon_pending_controls(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: Option<RunId>,
        status: ProductSessionStatus,
        reason: &str,
    ) -> Result<ProductTurnControlFinish, ProductStoreError> {
        let claim_id = claim_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| {
            repository.finish_session_turn_and_abandon_pending_controls(
                &claim_id, run_id, status, &reason,
            )
        })
        .await
    }

    async fn list_provider_profiles(
        &self,
    ) -> Result<Vec<ProductProviderProfile>, ProductStoreError> {
        self.blocking(|repository| repository.list_provider_profiles())
            .await
    }

    async fn get_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<ProductProviderProfile, ProductStoreError> {
        let profile_id = profile_id.clone();
        self.blocking(move |repository| repository.get_provider_profile(&profile_id))
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

    async fn upsert_provider_catalog_identity(
        &self,
        profile_id: &ProductProviderProfileId,
        label: &str,
        provider_type: crate::product::ProductProviderType,
        catalog_revision: &str,
    ) -> Result<(), ProductStoreError> {
        let profile_id = profile_id.clone();
        let label = label.to_string();
        let catalog_revision = catalog_revision.to_string();
        self.blocking(move |repository| {
            repository.upsert_provider_catalog_identity(
                &profile_id,
                &label,
                provider_type,
                &catalog_revision,
            )
        })
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

    async fn get_resume_health(&self) -> Result<ProductResumeHealth, ProductStoreError> {
        self.blocking(|repository| repository.get_resume_health())
            .await
    }

    async fn preflight_m1_browser_migration(
        &self,
        request: &M1BrowserMigrationRequest,
    ) -> Result<M1BrowserMigrationPreflight, ProductStoreError> {
        let request = request.clone();
        self.blocking(move |repository| repository.preflight_m1_browser_migration(&request))
            .await
    }

    async fn apply_m1_browser_migration(
        &self,
        migration: PreparedM1BrowserMigration,
    ) -> Result<M1BrowserMigrationResponse, ProductStoreError> {
        self.blocking(move |repository| repository.apply_m1_browser_migration(migration))
            .await
    }

    async fn create_control(
        &self,
        session_id: &ProductSessionId,
        kind: ProductControlKind,
        request: CreateProductControlRequest,
    ) -> Result<(ProductControl, bool), ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.create_control(&session_id, kind, request))
            .await
    }

    async fn create_message(
        &self,
        session_id: &ProductSessionId,
        request: CreateProductMessageRequest,
    ) -> Result<(ProductMessage, bool), ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.create_message(&session_id, request))
            .await
    }

    async fn promote_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let session_id = session_id.clone();
        let message_id = message_id.clone();
        self.blocking(move |repository| repository.promote_message(&session_id, &message_id))
            .await
    }

    async fn revoke_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let session_id = session_id.clone();
        let message_id = message_id.clone();
        self.blocking(move |repository| repository.revoke_message(&session_id, &message_id))
            .await
    }

    async fn list_messages(
        &self,
        session_id: &ProductSessionId,
        query: ProductMessagePageQuery,
    ) -> Result<ProductMessagePage, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_messages(&session_id, query))
            .await
    }

    async fn get_message(
        &self,
        session_id: &ProductSessionId,
        message_id: &ProductControlId,
    ) -> Result<ProductMessage, ProductStoreError> {
        let session_id = session_id.clone();
        let message_id = message_id.clone();
        self.blocking(move |repository| repository.get_message(&session_id, &message_id))
            .await
    }

    async fn list_controls(
        &self,
        session_id: &ProductSessionId,
        filter: Option<ProductControlStatus>,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_controls(&session_id, filter))
            .await
    }

    async fn get_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError> {
        let session_id = session_id.clone();
        let control_id = control_id.clone();
        self.blocking(move |repository| repository.get_control(&session_id, &control_id))
            .await
    }

    async fn transition_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
        from: ProductControlStatus,
        to: ProductControlStatus,
        applied_run_id: Option<&RunId>,
    ) -> Result<ProductControl, ProductStoreError> {
        let session_id = session_id.clone();
        let control_id = control_id.clone();
        let applied_run_id = applied_run_id.copied();
        self.blocking(move |repository| {
            repository.transition_control(
                &session_id,
                &control_id,
                from,
                to,
                applied_run_id.as_ref(),
            )
        })
        .await
    }

    async fn confirm_abandoned_followup(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError> {
        let session_id = session_id.clone();
        let control_id = control_id.clone();
        self.blocking(move |repository| {
            repository.confirm_abandoned_followup(&session_id, &control_id)
        })
        .await
    }

    async fn abandon_pending_controls(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<u64, ProductStoreError> {
        let session_id = session_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| repository.abandon_pending_controls(&session_id, &reason))
            .await
    }

    async fn list_pending_followups(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.list_pending_followups(&session_id))
            .await
    }

    async fn claim_next_pending_followup(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductControl>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.claim_next_pending_followup(&session_id))
            .await
    }

    async fn claim_next_followup_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError> {
        let session_id = session_id.clone();
        self.blocking(move |repository| repository.claim_next_followup_turn(&session_id))
            .await
    }

    async fn requeue_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
    ) -> Result<(), ProductStoreError> {
        let claim_id = claim_id.clone();
        let control_id = control_id.clone();
        self.blocking(move |repository| repository.requeue_followup_turn(&claim_id, &control_id))
            .await
    }

    async fn reserve_followup_run(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        run_id: RunId,
    ) -> Result<(), ProductStoreError> {
        let claim_id = claim_id.clone();
        let control_id = control_id.clone();
        self.blocking(move |repository| {
            repository.reserve_followup_run(&claim_id, &control_id, run_id)
        })
        .await
    }

    async fn abandon_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        reason: &str,
    ) -> Result<(), ProductStoreError> {
        let claim_id = claim_id.clone();
        let control_id = control_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| {
            repository.abandon_followup_turn(&claim_id, &control_id, &reason)
        })
        .await
    }

    async fn list_idle_sessions_with_pending_followups(
        &self,
    ) -> Result<Vec<ProductSessionId>, ProductStoreError> {
        self.blocking(|repository| repository.list_idle_sessions_with_pending_followups())
            .await
    }

    async fn drop_pending_steers(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let session_id = session_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| repository.drop_pending_steers(&session_id, &reason))
            .await
    }

    async fn abandon_pending_followups(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError> {
        let session_id = session_id.clone();
        let reason = reason.to_string();
        self.blocking(move |repository| repository.abandon_pending_followups(&session_id, &reason))
            .await
    }
}

#[cfg(test)]
mod tests;
