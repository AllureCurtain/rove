use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use chrono::{SecondsFormat, Utc};
use rove_app_bootstrap::{
    ProjectActivationState, ProjectTrustRecord, canonical_root_key, capability_digest_map,
    resolve_project_trust_record, workspace_identity_digest,
};
use rove_runtime::workspace::{Workspace, WorkspaceKind};

use super::{
    ProductErrorCode, ProductStore, ProductTrustDecisionRequest, ProductTrustState,
    ProductTrustStatus, ProductWorkspaceId, ProductWorkspaceKind, StoredProjectTrustRecord,
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
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let (root, kind) = open_workspace(&workspace)?;
    Ok(Json(
        trust_status(&store, &workspace_id, &root, kind).await?,
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
            ProductErrorCode::ProductInvalidInput.as_str(),
            "project trust capability list is too large",
        ));
    }
    let store = state.product_store()?;
    let workspace = store.get_workspace(&workspace_id).await?;
    let (root, kind) = open_workspace(&workspace)?;
    let all_digests =
        capability_digest_map(&root, Some(&root.join(".rove/mcp_servers.json")), None);
    let requested = selected_capability_digests(&request, all_digests)?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let trust_state = match request.decision {
        super::ProductTrustDecision::Grant => ProductTrustState::Trusted,
        super::ProductTrustDecision::Deny => ProductTrustState::Restricted,
        super::ProductTrustDecision::Revoke => ProductTrustState::Revoked,
    };
    let record = StoredProjectTrustRecord {
        canonical_root: canonical_root_key(&root),
        workspace_kind: product_workspace_kind(&kind)?,
        identity_digest: workspace_identity_digest(&root, kind.clone()),
        state: trust_state,
        capability_digests: if trust_state == ProductTrustState::Trusted {
            requested
        } else {
            BTreeMap::new()
        },
        granted_at: (trust_state == ProductTrustState::Trusted).then_some(now.clone()),
        revoked_at: (trust_state == ProductTrustState::Revoked).then_some(now.clone()),
        updated_at: now,
    };
    store.put_project_trust_record(record).await?;
    if trust_state == ProductTrustState::Revoked {
        state.quarantine_workspace_jobs(&root).await;
    }
    Ok(Json(
        trust_status(&store, &workspace_id, &root, kind).await?,
    ))
}

fn selected_capability_digests(
    request: &ProductTrustDecisionRequest,
    all_digests: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ApiError> {
    if request.capabilities.is_empty() {
        return Ok(all_digests);
    }
    let mut selected = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for capability in &request.capabilities {
        let key = capability.as_str();
        if !seen.insert(key) {
            return Err(ApiError::bad_request_with_code(
                ProductErrorCode::ProductInvalidInput.as_str(),
                "project trust capability list contains duplicates",
            ));
        }
        let digest = all_digests.get(key).ok_or_else(|| {
            ApiError::bad_request_with_code(
                ProductErrorCode::ProductInvalidInput.as_str(),
                "unknown project trust capability",
            )
        })?;
        selected.insert(key.to_string(), digest.clone());
    }
    Ok(selected)
}

pub(crate) async fn resolve_product_workspace_trust(
    store: &Arc<dyn ProductStore>,
    root: &std::path::Path,
    kind: WorkspaceKind,
    provider_selector: Option<&str>,
) -> Result<rove_app_bootstrap::ProjectTrustResolution, ApiError> {
    let canonical_root = canonical_root_key(root);
    let record = store
        .get_project_trust_record(&canonical_root, product_workspace_kind(&kind)?)
        .await?;
    let bootstrap_record = record.as_ref().map(stored_to_bootstrap_record);
    let digests = capability_digest_map(
        root,
        Some(&root.join(".rove/mcp_servers.json")),
        provider_selector,
    );
    Ok(resolve_project_trust_record(
        bootstrap_record.as_ref(),
        workspace_identity_digest(root, kind),
        &digests,
    ))
}

async fn trust_status(
    store: &Arc<dyn ProductStore>,
    workspace_id: &ProductWorkspaceId,
    root: &std::path::Path,
    kind: WorkspaceKind,
) -> Result<ProductTrustStatus, ApiError> {
    let canonical_root = canonical_root_key(root);
    let record = store
        .get_project_trust_record(&canonical_root, product_workspace_kind(&kind)?)
        .await?;
    let resolution = resolve_product_workspace_trust(store, root, kind, None).await?;
    let state = if record.is_none() {
        ProductTrustState::Unknown
    } else {
        activation_state_to_product(resolution.state)
    };
    Ok(ProductTrustStatus {
        workspace_id: workspace_id.clone(),
        state,
        identity_digest: resolution.identity_digest,
        invalidated_capabilities: resolution.invalidated_capabilities,
        granted_capabilities: resolution.granted_capabilities.into_iter().collect(),
    })
}

fn stored_to_bootstrap_record(record: &StoredProjectTrustRecord) -> ProjectTrustRecord {
    ProjectTrustRecord {
        canonical_root: record.canonical_root.clone(),
        workspace_kind: match record.workspace_kind {
            ProductWorkspaceKind::Folder => WorkspaceKind::Folder,
            ProductWorkspaceKind::Repo => WorkspaceKind::Repo,
        },
        identity_digest: record.identity_digest.clone(),
        state: match record.state {
            ProductTrustState::Unknown => ProjectActivationState::Unknown,
            ProductTrustState::Restricted => ProjectActivationState::Restricted,
            ProductTrustState::Trusted => ProjectActivationState::Trusted,
            ProductTrustState::Revoked => ProjectActivationState::Revoked,
        },
        capability_digests: record.capability_digests.clone(),
        granted_at: record.granted_at.clone(),
        revoked_at: record.revoked_at.clone(),
        updated_at: record.updated_at.clone(),
    }
}

fn activation_state_to_product(state: ProjectActivationState) -> ProductTrustState {
    match state {
        ProjectActivationState::Unknown => ProductTrustState::Unknown,
        ProjectActivationState::Restricted => ProductTrustState::Restricted,
        ProjectActivationState::Trusted => ProductTrustState::Trusted,
        ProjectActivationState::Revoked => ProductTrustState::Revoked,
    }
}

fn product_workspace_kind(kind: &WorkspaceKind) -> Result<ProductWorkspaceKind, ApiError> {
    match kind {
        WorkspaceKind::Folder => Ok(ProductWorkspaceKind::Folder),
        WorkspaceKind::Repo => Ok(ProductWorkspaceKind::Repo),
        WorkspaceKind::Task => Err(ApiError::bad_request_with_code(
            ProductErrorCode::ProductInvalidInput.as_str(),
            "task workspaces do not support durable product trust",
        )),
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
