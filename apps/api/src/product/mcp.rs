use std::collections::HashMap;
use std::io::ErrorKind;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use rove_runtime::tools::mcp_config::{
    create_product_mcp_server_sync, delete_product_mcp_server_sync, list_product_mcp_servers_sync,
    promote_product_mcp_catalog_sync, update_product_mcp_server_sync, validate_product_mcp_server,
    validate_product_mcp_server_name,
};
use rove_runtime::tools::mcp_proxy::{
    McpProbeFailure, McpProbeFailureKind, McpServerConfig, McpServerHealthStatus,
    McpServerRuntimeSnapshot, McpToolInfo, McpTransport, McpTransportPolicy,
    probe_mcp_server_with_environment,
};
use serde::Deserialize;
use utoipa::IntoParams;

use super::*;
use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

const PRODUCT_MCP_STDERR_CAPTURE_BYTES: usize = 16 * 1024;

struct ProductMcpConfigPaths {
    active: std::path::PathBuf,
    write: std::path::PathBuf,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProductMcpWorkspaceQuery {
    /// Server-owned product workspace identity. Client paths are never accepted.
    pub workspace_id: ProductWorkspaceId,
}

#[utoipa::path(
    get,
    path = "/product/mcp/servers",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ProductMcpWorkspaceQuery),
    responses(
        (status = 200, description = "Workspace MCP server catalog without environment values", body = ProductMcpServersResponse),
        (status = 400, description = "Missing or invalid product workspace id", body = ApiErrorResponse),
        (status = 404, description = "Product workspace not found", body = ApiErrorResponse),
        (status = 409, description = "MCP config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "MCP config read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_product_mcp_servers(
    State(state): State<ApiState>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
) -> Result<Json<ProductMcpServersResponse>, ApiError> {
    let Query(query) = product_mcp_query(query)?;
    let path = product_mcp_config_path(&state, &query.workspace_id).await?;
    let servers = tokio::task::spawn_blocking(move || list_product_mcp_servers_sync(&path))
        .await
        .map_err(|_| product_mcp_internal())?
        .map_err(map_mcp_io_error)?;
    let servers = servers
        .into_iter()
        .map(product_mcp_server)
        .collect::<Vec<_>>();
    let total = servers.len();
    Ok(Json(ProductMcpServersResponse { servers, total }))
}

#[utoipa::path(
    get,
    path = "/product/mcp/health",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ProductMcpWorkspaceQuery),
    responses(
        (status = 200, description = "Last real runtime MCP activation health; unactivated servers are explicitly unknown", body = ProductMcpHealthResponse),
        (status = 400, description = "Missing or invalid product workspace id", body = ApiErrorResponse),
        (status = 404, description = "Product workspace not found", body = ApiErrorResponse),
        (status = 409, description = "MCP config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "MCP config read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_mcp_health(
    State(state): State<ApiState>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
) -> Result<Json<ProductMcpHealthResponse>, ApiError> {
    let Query(query) = product_mcp_query(query)?;
    let path = product_mcp_config_path(&state, &query.workspace_id).await?;
    let read_path = path.clone();
    let servers = tokio::task::spawn_blocking(move || list_product_mcp_servers_sync(&read_path))
        .await
        .map_err(|_| product_mcp_internal())?
        .map_err(map_mcp_io_error)?;
    let runtime = state
        .inner
        .mcp_health
        .read()
        .await
        .get(&path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|snapshot| (snapshot.server_config_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let snapshots = servers
        .into_iter()
        .map(|server| {
            let runtime = runtime.get(&server.name);
            product_mcp_health(server, runtime)
        })
        .collect::<Vec<_>>();
    let total = snapshots.len();
    Ok(Json(ProductMcpHealthResponse {
        servers: snapshots,
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/product/mcp/servers",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(ProductMcpWorkspaceQuery),
    request_body = CreateProductMcpServerRequest,
    responses(
        (status = 201, description = "Workspace MCP server created", body = ProductMcpServer),
        (status = 400, description = "Invalid MCP server configuration or secret-shaped input", body = ApiErrorResponse),
        (status = 404, description = "Product workspace not found", body = ApiErrorResponse),
        (status = 409, description = "MCP server already exists or config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "MCP config write failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_product_mcp_server(
    State(state): State<ApiState>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
    body: Result<Json<CreateProductMcpServerRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ProductMcpServer>), ApiError> {
    let Query(query) = product_mcp_query(query)?;
    let request = super::routes::product_json(body)?;
    let server = mcp_config_from_create(request);
    validate_product_mcp_server(&server).map_err(map_mcp_io_error)?;
    let paths = product_mcp_write_paths(&state, &query.workspace_id).await?;
    let active_health_path = paths.active.clone();
    let health_path = paths.write.clone();
    let server = tokio::task::spawn_blocking(move || {
        promote_product_mcp_catalog_sync(&paths.active, &paths.write)?;
        create_product_mcp_server_sync(&paths.write, server)
    })
    .await
    .map_err(|_| product_mcp_internal())?
    .map_err(map_mcp_io_error)?;
    let mut health = state.inner.mcp_health.write().await;
    health.remove(&active_health_path);
    health.remove(&health_path);
    Ok((StatusCode::CREATED, Json(product_mcp_server(server))))
}

#[utoipa::path(
    put,
    path = "/product/mcp/servers/{name}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMcpWorkspaceQuery,
        ("name" = String, Path, description = "Immutable workspace MCP server name")
    ),
    request_body = UpdateProductMcpServerRequest,
    responses(
        (status = 200, description = "Workspace MCP server updated", body = ProductMcpServer),
        (status = 400, description = "Invalid MCP server configuration or secret-shaped input", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or MCP server not found", body = ApiErrorResponse),
        (status = 409, description = "MCP config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "MCP config write failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_product_mcp_server(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
    body: Result<Json<UpdateProductMcpServerRequest>, JsonRejection>,
) -> Result<Json<ProductMcpServer>, ApiError> {
    let Query(query) = product_mcp_query(query)?;
    let request = super::routes::product_json(body)?;
    let server = mcp_config_from_update(name.clone(), request);
    validate_product_mcp_server(&server).map_err(map_mcp_io_error)?;
    let paths = product_mcp_write_paths(&state, &query.workspace_id).await?;
    let active_health_path = paths.active.clone();
    let health_path = paths.write.clone();
    let server = tokio::task::spawn_blocking(move || {
        promote_product_mcp_catalog_sync(&paths.active, &paths.write)?;
        update_product_mcp_server_sync(&paths.write, &name, server)
    })
    .await
    .map_err(|_| product_mcp_internal())?
    .map_err(map_mcp_io_error)?;
    let mut health = state.inner.mcp_health.write().await;
    health.remove(&active_health_path);
    health.remove(&health_path);
    Ok(Json(product_mcp_server(server)))
}

#[utoipa::path(
    delete,
    path = "/product/mcp/servers/{name}",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMcpWorkspaceQuery,
        ("name" = String, Path, description = "Workspace MCP server name")
    ),
    responses(
        (status = 204, description = "Workspace MCP server deleted"),
        (status = 400, description = "Invalid MCP server name or workspace query", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or MCP server not found", body = ApiErrorResponse),
        (status = 409, description = "MCP config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 500, description = "MCP config write failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_product_mcp_server(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
) -> Result<StatusCode, ApiError> {
    let Query(query) = product_mcp_query(query)?;
    validate_product_mcp_server_name(&name).map_err(map_mcp_io_error)?;
    let paths = product_mcp_write_paths(&state, &query.workspace_id).await?;
    let active_health_path = paths.active.clone();
    let health_path = paths.write.clone();
    tokio::task::spawn_blocking(move || {
        promote_product_mcp_catalog_sync(&paths.active, &paths.write)?;
        delete_product_mcp_server_sync(&paths.write, &name)
    })
    .await
    .map_err(|_| product_mcp_internal())?
    .map_err(map_mcp_io_error)?;
    let mut health = state.inner.mcp_health.write().await;
    health.remove(&active_health_path);
    health.remove(&health_path);
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/product/mcp/servers/{name}/probe",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ProductMcpWorkspaceQuery,
        ("name" = String, Path, description = "Workspace MCP server name")
    ),
    responses(
        (status = 200, description = "MCP server initialized and returned a non-empty tool catalog", body = ProductMcpProbeResponse),
        (status = 400, description = "A configured environment variable name is unavailable", body = ApiErrorResponse),
        (status = 404, description = "Product workspace or MCP server not found", body = ApiErrorResponse),
        (status = 409, description = "MCP config is locked, unsafe, or corrupt", body = ApiErrorResponse),
        (status = 502, description = "MCP spawn, transport, protocol, or empty-tool failure", body = ApiErrorResponse),
        (status = 504, description = "MCP probe timed out", body = ApiErrorResponse),
        (status = 500, description = "MCP config read failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn probe_product_mcp_server(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
) -> Result<Json<ProductMcpProbeResponse>, ApiError> {
    let Query(query) = product_mcp_query(query)?;
    let (path, environment) = product_mcp_activation(&state, &query.workspace_id).await?;
    let requested_name = name.clone();
    let server = tokio::task::spawn_blocking(move || {
        list_product_mcp_servers_sync(&path).and_then(|servers| {
            servers
                .into_iter()
                .find(|server| server.name == requested_name)
                .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "MCP server not found"))
        })
    })
    .await
    .map_err(|_| product_mcp_internal())?
    .map_err(map_mcp_io_error)?;
    let transport = product_mcp_transport(server.transport);
    let tools = probe_mcp_server_with_environment(server, environment)
        .await
        .map_err(map_mcp_probe_failure)?
        .into_iter()
        .map(product_mcp_tool)
        .collect();
    Ok(Json(ProductMcpProbeResponse {
        server_name: name,
        transport,
        tools,
        tested_at: Utc::now().to_rfc3339(),
    }))
}

fn product_mcp_query(
    query: Result<Query<ProductMcpWorkspaceQuery>, QueryRejection>,
) -> Result<Query<ProductMcpWorkspaceQuery>, ApiError> {
    query.map_err(|_| {
        ApiError::bad_request_with_code(
            ProductErrorCode::ProductMcpInvalidInput.as_str(),
            "a valid product workspace id is required for MCP operations",
        )
    })
}

async fn product_mcp_config_path(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
) -> Result<std::path::PathBuf, ApiError> {
    Ok(
        product_mcp_config_paths_with_activation(state, workspace_id, false, false)
            .await?
            .active,
    )
}

async fn product_mcp_write_paths(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
) -> Result<ProductMcpConfigPaths, ApiError> {
    product_mcp_config_paths_with_activation(state, workspace_id, false, true).await
}

async fn product_mcp_activation(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
) -> Result<
    (
        std::path::PathBuf,
        std::sync::Arc<dyn rove_runtime::environment::ExecutionEnvironment>,
    ),
    ApiError,
> {
    let path = product_mcp_config_paths_with_activation(state, workspace_id, true, false)
        .await?
        .active;
    let product_workspace = state.product_store()?.get_workspace(workspace_id).await?;
    let workspace = crate::open_product_workspace(&product_workspace)?;
    let environment = rove_runtime::environment::local_environment(&workspace);
    Ok((path, environment))
}

async fn product_mcp_config_paths_with_activation(
    state: &ApiState,
    workspace_id: &ProductWorkspaceId,
    require_activation: bool,
    materialize_write: bool,
) -> Result<ProductMcpConfigPaths, ApiError> {
    let store = state.product_store()?;
    let product_workspace = store.get_workspace(workspace_id).await?;
    let workspace = crate::open_product_workspace(&product_workspace)?;
    let (workspace, mut config) = crate::rebased_workspace_config(state, workspace)?;
    let authority = state.project_trust()?;
    let catalog = state.provider_catalog().await?;
    let provider_selector = super::trust::product_provider_capability_selector(
        &store,
        &catalog,
        workspace_id,
        &workspace.root,
    )
    .await?;
    let trust = super::trust::resolve_product_workspace_trust(
        &authority,
        &workspace.root,
        workspace.kind.clone(),
        &provider_selector,
    )
    .await?;
    config.apply_project_trust_resolution(trust);
    if require_activation
        && !config.project_capability_allowed(rove_app_bootstrap::CAP_MCP_PROCESSES)
    {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProjectTrustRequired.as_str(),
            "project trust is required before probing or starting workspace MCP servers",
        ));
    }
    let active = config
        .workspace_bounded_mcp_config_path()
        .map_err(|error| {
            tracing::warn!(
                product_workspace_id = %workspace_id,
                "rejected unbounded product MCP config path: {error}"
            );
            ApiError::conflict_with_code(
                ProductErrorCode::ProductMcpConflict.as_str(),
                "product MCP config is not bounded by the selected workspace",
            )
        })?;
    let write = if materialize_write {
        config
            .ensure_workspace_bounded_mcp_write_path()
            .map_err(|error| {
                tracing::warn!(
                    product_workspace_id = %workspace_id,
                    "rejected or unavailable product MCP write path: {error}"
                );
                ApiError::conflict_with_code(
                    ProductErrorCode::ProductMcpConflict.as_str(),
                    "product MCP config is not bounded by the selected workspace",
                )
            })?
    } else {
        active.clone()
    };
    Ok(ProductMcpConfigPaths { active, write })
}

fn mcp_config_from_create(request: CreateProductMcpServerRequest) -> McpServerConfig {
    mcp_config(ProductMcpConfigInput {
        name: request.name,
        enabled: request.enabled,
        required: request.required,
        transport: request.transport,
        command: request.command,
        args: request.args,
        env_names: request.env_names,
        url: request.url,
        request_timeout_ms: request.request_timeout_ms,
    })
}

fn mcp_config_from_update(name: String, request: UpdateProductMcpServerRequest) -> McpServerConfig {
    mcp_config(ProductMcpConfigInput {
        name,
        enabled: request.enabled,
        required: request.required,
        transport: request.transport,
        command: request.command,
        args: request.args,
        env_names: request.env_names,
        url: request.url,
        request_timeout_ms: request.request_timeout_ms,
    })
}

struct ProductMcpConfigInput {
    name: String,
    enabled: bool,
    required: bool,
    transport: ProductMcpTransport,
    command: Option<String>,
    args: Vec<String>,
    env_names: Vec<String>,
    url: Option<String>,
    request_timeout_ms: u64,
}

fn mcp_config(input: ProductMcpConfigInput) -> McpServerConfig {
    McpServerConfig {
        name: input.name,
        enabled: input.enabled,
        required: input.required,
        transport: match input.transport {
            ProductMcpTransport::Stdio => McpTransport::Stdio,
            ProductMcpTransport::Sse => McpTransport::Sse,
            ProductMcpTransport::StreamableHttp => McpTransport::StreamableHttp,
        },
        command: input.command.unwrap_or_default(),
        args: input.args,
        env: HashMap::new(),
        env_names: input.env_names,
        url: input.url.unwrap_or_default(),
        policy: McpTransportPolicy {
            request_timeout_ms: input.request_timeout_ms,
            stderr_capture_bytes: PRODUCT_MCP_STDERR_CAPTURE_BYTES,
        },
    }
}

fn product_mcp_server(server: McpServerConfig) -> ProductMcpServer {
    let transport = product_mcp_transport(server.transport);
    ProductMcpServer {
        name: server.name,
        enabled: server.enabled,
        required: server.required,
        transport,
        command: (transport == ProductMcpTransport::Stdio).then_some(server.command),
        args: server.args,
        env_names: server.env_names,
        url: transport.is_http().then_some(server.url),
        request_timeout_ms: server.policy.request_timeout_ms,
        transport_deprecated: transport.is_deprecated(),
    }
}

fn product_mcp_health(
    server: McpServerConfig,
    runtime: Option<&McpServerRuntimeSnapshot>,
) -> ProductMcpHealthSnapshot {
    let status = runtime.map_or_else(
        || {
            if server.enabled {
                ProductMcpHealthStatus::Unknown
            } else {
                ProductMcpHealthStatus::Disabled
            }
        },
        |snapshot| match snapshot.status {
            McpServerHealthStatus::Ready => ProductMcpHealthStatus::Ready,
            McpServerHealthStatus::Degraded => ProductMcpHealthStatus::Degraded,
            McpServerHealthStatus::Disabled => ProductMcpHealthStatus::Disabled,
        },
    );
    ProductMcpHealthSnapshot {
        server_name: server.name,
        required: server.required,
        transport: product_mcp_transport(server.transport),
        status,
        server_config_hash: runtime.map(|snapshot| snapshot.server_config_hash.clone()),
        server_identity_hash: runtime.map(|snapshot| snapshot.server_identity_hash.clone()),
        protocol_version: runtime.and_then(|snapshot| snapshot.protocol_version.clone()),
        catalog_hash: runtime.and_then(|snapshot| snapshot.catalog_hash.clone()),
        capability_snapshot_id: runtime
            .and_then(|snapshot| snapshot.capability_snapshot_id.clone()),
        tool_count: runtime.map_or(0, |snapshot| snapshot.tool_count),
        failure_code: runtime.and_then(|snapshot| snapshot.failure_code.clone()),
        refreshed_at: runtime.map(|snapshot| snapshot.refreshed_at.clone()),
    }
}

fn product_mcp_transport(transport: McpTransport) -> ProductMcpTransport {
    match transport {
        McpTransport::Stdio => ProductMcpTransport::Stdio,
        McpTransport::Sse => ProductMcpTransport::Sse,
        McpTransport::StreamableHttp => ProductMcpTransport::StreamableHttp,
    }
}

fn product_mcp_tool(tool: McpToolInfo) -> ProductMcpToolDescriptor {
    ProductMcpToolDescriptor {
        name: bounded_mcp_text(&tool.remote_name),
        description: bounded_mcp_text(&tool.schema.description),
        destructive: true,
        parallel_safe: false,
    }
}

fn bounded_mcp_text(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.len() <= MAX_PRODUCT_TEXT_BYTES {
        return sanitized;
    }
    let mut end = 0;
    for (index, character) in sanitized.char_indices() {
        let next = index + character.len_utf8();
        if next > MAX_PRODUCT_TEXT_BYTES {
            break;
        }
        end = next;
    }
    sanitized[..end].to_string()
}

fn map_mcp_io_error(error: std::io::Error) -> ApiError {
    match error.kind() {
        ErrorKind::InvalidInput => ApiError::bad_request_with_code(
            ProductErrorCode::ProductMcpInvalidInput.as_str(),
            "invalid product MCP server configuration",
        ),
        ErrorKind::NotFound => ApiError::not_found_with_code(
            ProductErrorCode::ProductMcpNotFound.as_str(),
            "product MCP server was not found",
        ),
        ErrorKind::AlreadyExists
        | ErrorKind::WouldBlock
        | ErrorKind::InvalidData
        | ErrorKind::PermissionDenied => ApiError::conflict_with_code(
            ProductErrorCode::ProductMcpConflict.as_str(),
            "product MCP config is locked, unsafe, corrupt, or already contains this server",
        ),
        _ => product_mcp_internal(),
    }
}

fn map_mcp_probe_failure(failure: McpProbeFailure) -> ApiError {
    match failure.kind {
        McpProbeFailureKind::EnvironmentMissing => ApiError::bad_request_with_code(
            "product_mcp_environment_missing",
            "one or more configured MCP environment variables are unavailable",
        ),
        McpProbeFailureKind::Spawn => ApiError::bad_gateway_with_code(
            "product_mcp_spawn_failed",
            "the MCP stdio server could not be started",
        ),
        McpProbeFailureKind::Timeout => {
            ApiError::gateway_timeout_with_code("product_mcp_timeout", "the MCP probe timed out")
        }
        McpProbeFailureKind::Transport => ApiError::bad_gateway_with_code(
            "product_mcp_transport",
            "the MCP transport failed during probe",
        ),
        McpProbeFailureKind::Protocol => ApiError::bad_gateway_with_code(
            "product_mcp_protocol_mismatch",
            "the MCP server returned an incompatible protocol response",
        ),
        McpProbeFailureKind::NoTools => ApiError::bad_gateway_with_code(
            "product_mcp_no_tools",
            "the MCP server returned no tools",
        ),
    }
}

fn product_mcp_internal() -> ApiError {
    ProductStoreError::new(
        ProductErrorCode::ProductStorageFailure,
        "product MCP config operation failed",
    )
    .into()
}
