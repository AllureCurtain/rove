use anyhow::{Context, Result};
use rove_api::{embedded_api_state, serve_state_listener};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Configuration for the API owned by the Desktop host.
#[derive(Debug, Clone)]
pub struct ApiServerConfig {
    pub bearer_token: String,
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub logs_dir: PathBuf,
}

/// Handle for the embedded, shared rove-api server.
pub struct ApiServerHandle {
    pub base_url: String,
    pub port: u16,
    shutdown: CancellationToken,
    task_handle: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl ApiServerHandle {
    pub async fn shutdown(mut self) -> Result<()> {
        self.shutdown.cancel();

        if let Some(mut handle) = self.task_handle.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
                Ok(Ok(Ok(()))) => {
                    info!("embedded API server shut down gracefully");
                    Ok(())
                }
                Ok(Ok(Err(error))) => Err(error.context("embedded API server failed")),
                Ok(Err(error)) => Err(anyhow::anyhow!("embedded API task panicked: {error}")),
                Err(_) => {
                    warn!("embedded API server shutdown timed out after 5 seconds");
                    handle.abort();
                    let _ = handle.await;
                    Err(anyhow::anyhow!(
                        "embedded API server shutdown timed out and was aborted"
                    ))
                }
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for ApiServerHandle {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}

async fn bind_available_listener() -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind embedded API listener")
}

/// Start the real `rove-api` router in-process. The Desktop crate owns only
/// lifecycle and configuration; request handling remains in `rove-api`.
pub async fn start_api_server(config: ApiServerConfig) -> Result<ApiServerHandle> {
    std::fs::create_dir_all(&config.state_dir).context("failed to create API state directory")?;
    std::fs::create_dir_all(&config.config_dir).context("failed to create API config directory")?;
    std::fs::create_dir_all(&config.logs_dir).context("failed to create API logs directory")?;

    let listener = bind_available_listener().await?;
    let addr = listener
        .local_addr()
        .context("embedded API listener has no local address")?;
    let base_url = format!("http://127.0.0.1:{}", addr.port());
    info!("starting embedded API server on {}", base_url);

    let shutdown = CancellationToken::new();
    let cwd = std::env::current_dir().context("failed to resolve desktop working directory")?;
    let state = embedded_api_state(
        &cwd,
        addr,
        config.state_dir.clone(),
        config.bearer_token.clone(),
        vec![
            "tauri://localhost".to_string(),
            "http://tauri.localhost".to_string(),
        ],
        shutdown.clone(),
    )
    .context("failed to assemble embedded Rove API state")?;
    let task_handle = tokio::spawn(async move {
        let result = serve_state_listener(listener, state).await;
        info!("embedded API server task stopped");
        result
    });

    // There is intentionally no unauthenticated health endpoint. Runtime info
    // proves that routing, bearer middleware, and ProductStore are ready.
    let health_url = format!("{base_url}/product/runtime");
    let client = reqwest::Client::new();
    let mut ready = false;
    for attempt in 1..=30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match client
            .get(&health_url)
            .bearer_auth(&config.bearer_token)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                info!("embedded API readiness check passed on attempt {}", attempt);
                ready = true;
                break;
            }
            Ok(response) => {
                warn!(
                    "embedded API readiness returned status {} on attempt {}",
                    response.status(),
                    attempt
                );
            }
            Err(error) if attempt == 1 || attempt % 10 == 0 => {
                info!(
                    "waiting for embedded API readiness (attempt {}): {}",
                    attempt, error
                );
            }
            Err(_) => {}
        }
    }
    if !ready {
        shutdown.cancel();
        let mut task_handle = task_handle;
        if tokio::time::timeout(Duration::from_secs(5), &mut task_handle)
            .await
            .is_err()
        {
            task_handle.abort();
            let _ = task_handle.await;
        }
        return Err(anyhow::anyhow!(
            "embedded API server readiness check timed out"
        ));
    }

    Ok(ApiServerHandle {
        base_url,
        port: addr.port(),
        shutdown,
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_starts_and_shuts_down() {
        let root = std::env::temp_dir().join(format!("rove-desktop-api-{}", uuid::Uuid::new_v4()));
        let config = ApiServerConfig {
            bearer_token: "test-token".to_string(),
            state_dir: root.join("state"),
            config_dir: root.join("config"),
            logs_dir: root.join("logs"),
        };
        let handle = start_api_server(config).await.unwrap();
        let response = reqwest::Client::new()
            .get(format!("{}/product/runtime", handle.base_url))
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        handle.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
