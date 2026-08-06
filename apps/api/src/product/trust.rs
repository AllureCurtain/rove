use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use rove_app_bootstrap::{
    ProjectActivationState, ProjectTrustDecision, ProjectTrustRepository, capability_digest_map,
    provider_capability_selector_for_workspace,
};
use rove_runtime::workspace::{Workspace, WorkspaceKind};

use super::{
    ProductErrorCode, ProductStoreError, ProductTrustDecisionRequest, ProductTrustState,
    ProductTrustStatus, ProductWorkspaceId, ProductWorkspaceKind,
};
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

#[utoipa::path(
    get,
    path = "/product/workspaces/{workspace_id}/trust",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("workspace_id" = ProductWorkspaceId, Path, description = "Server-owned workspace id")),
    responses(
        (status = 200, description = "Project trust state and safe capability summary", body = ProductTrustStatus),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 409, description = "Workspace identity or trust store is invalid", body = ApiErrorResponse),
        (status = 500, description = "Project trust storage failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_project_trust(
    State(state): State<ApiState>,
    Path(workspace_id): Path<ProductWorkspaceId>,
) -> Result<Json<ProductTrustStatus>, ApiError> {
    let authority = state.project_trust()?;
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let (root, kind) = open_workspace(&workspace)?;
    Ok(Json(
        trust_status(&authority, &workspace_id, &root, kind).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/product/workspaces/{workspace_id}/trust",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("workspace_id" = ProductWorkspaceId, Path, description = "Server-owned workspace id")),
    request_body = ProductTrustDecisionRequest,
    responses(
        (status = 200, description = "Durable project trust decision", body = ProductTrustStatus),
        (status = 400, description = "Invalid trust decision", body = ApiErrorResponse),
        (status = 404, description = "Workspace not found", body = ApiErrorResponse),
        (status = 409, description = "Workspace identity or trust store is invalid", body = ApiErrorResponse),
        (status = 500, description = "Project trust storage failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn decide_project_trust(
    State(state): State<ApiState>,
    Path(workspace_id): Path<ProductWorkspaceId>,
    body: Result<Json<ProductTrustDecisionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ProductTrustStatus>, ApiError> {
    let request = super::routes::product_json(body)?;
    if request.capabilities.len() > super::MAX_PROJECT_TRUST_CAPABILITIES {
        return Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProjectTrustInvalidInput.as_str(),
            "project trust capability list is too large",
        ));
    }
    let authority = state.project_trust()?;
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let (root, kind) = open_workspace(&workspace)?;
    let provider_selector = provider_capability_selector_for_workspace(&root);
    let all_digests = capability_digest_map(&root, None, Some(&provider_selector));
    let requested = selected_capability_digests(&request, all_digests)?;
    let trust_state = match request.decision {
        super::ProductTrustDecision::Grant => ProjectActivationState::Trusted,
        super::ProductTrustDecision::Deny => ProjectActivationState::Restricted,
        super::ProductTrustDecision::Revoke => ProjectActivationState::Revoked,
    };
    authority
        .decide(
            &root,
            kind.clone(),
            match request.decision {
                super::ProductTrustDecision::Grant => ProjectTrustDecision::Grant,
                super::ProductTrustDecision::Deny => ProjectTrustDecision::Deny,
                super::ProductTrustDecision::Revoke => ProjectTrustDecision::Revoke,
            },
            requested,
        )
        .map_err(|error| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProjectTrustUnavailable,
                format!("project trust authority failed: {error}"),
            ))
        })?;
    if trust_state == ProjectActivationState::Revoked {
        state.quarantine_workspace_jobs(&root).await;
    }
    Ok(Json(
        trust_status(&authority, &workspace_id, &root, kind).await?,
    ))
}

fn selected_capability_digests(
    request: &ProductTrustDecisionRequest,
    all_digests: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ApiError> {
    if request.capabilities.is_empty() {
        return Ok(match request.decision {
            super::ProductTrustDecision::Grant => all_digests,
            super::ProductTrustDecision::Deny | super::ProductTrustDecision::Revoke => {
                BTreeMap::new()
            }
        });
    }
    let mut selected = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for capability in &request.capabilities {
        let key = capability.as_str();
        if !seen.insert(key) {
            return Err(ApiError::bad_request_with_code(
                ProductErrorCode::ProjectTrustInvalidInput.as_str(),
                "project trust capability list contains duplicates",
            ));
        }
        let digest = all_digests.get(key).ok_or_else(|| {
            ApiError::bad_request_with_code(
                ProductErrorCode::ProjectTrustInvalidInput.as_str(),
                "unknown project trust capability",
            )
        })?;
        selected.insert(key.to_string(), digest.clone());
    }
    Ok(selected)
}

pub(crate) async fn resolve_product_workspace_trust(
    authority: &Arc<ProjectTrustRepository>,
    root: &std::path::Path,
    kind: WorkspaceKind,
    provider_selector: &str,
) -> Result<rove_app_bootstrap::ProjectTrustResolution, ApiError> {
    let digests = capability_digest_map(root, None, Some(provider_selector));
    authority.resolve(root, kind, &digests).map_err(|error| {
        ApiError::from(ProductStoreError::new(
            ProductErrorCode::ProjectTrustUnavailable,
            format!("project trust authority failed: {error}"),
        ))
    })
}

async fn trust_status(
    authority: &Arc<ProjectTrustRepository>,
    workspace_id: &ProductWorkspaceId,
    root: &std::path::Path,
    kind: WorkspaceKind,
) -> Result<ProductTrustStatus, ApiError> {
    let provider_selector = provider_capability_selector_for_workspace(root);
    let resolution =
        resolve_product_workspace_trust(authority, root, kind, &provider_selector).await?;
    Ok(ProductTrustStatus {
        workspace_id: workspace_id.clone(),
        state: activation_state_to_product(resolution.state),
        identity_digest: resolution.identity_digest,
        invalidated_capabilities: resolution.invalidated_capabilities,
        granted_capabilities: resolution.granted_capabilities.into_iter().collect(),
    })
}

fn activation_state_to_product(state: ProjectActivationState) -> ProductTrustState {
    match state {
        ProjectActivationState::Unknown => ProductTrustState::Unknown,
        ProjectActivationState::Restricted => ProductTrustState::Restricted,
        ProjectActivationState::Trusted => ProductTrustState::Trusted,
        ProjectActivationState::Revoked => ProductTrustState::Revoked,
    }
}

fn open_workspace(
    workspace: &super::ProductWorkspace,
) -> Result<(std::path::PathBuf, WorkspaceKind), ApiError> {
    let opened = match workspace.kind {
        ProductWorkspaceKind::Folder => Workspace::open_folder(&workspace.canonical_root),
        ProductWorkspaceKind::Repo => Workspace::open_repo(&workspace.canonical_root),
    }
    .map_err(|error| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            format!("workspace identity is no longer valid: {error}"),
        )
    })?;
    Ok((opened.root, opened.kind))
}
