use anyhow::{Context, Result};
use std::net::TcpListener;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

/// API server configuration
#[derive(Debug, Clone)]
pub struct ApiServerConfig {
    /// Bearer token for authentication
    pub bearer_token: String,
    /// State directory for ProductStore
    pub state_dir: std::path::PathBuf,
    /// Config directory
    pub config_dir: std::path::PathBuf,
    /// Logs directory
    pub logs_dir: std::path::PathBuf,
}

/// API server handle
pub struct ApiServerHandle {
    /// Server base URL (http://localhost:PORT)
    pub base_url: String,
    /// Port the server is listening on
    pub port: u16,
    /// Cancellation sender
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Join handle for the server task
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ApiServerHandle {
    /// Shutdown the server gracefully
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            // Wait up to 5 seconds for graceful shutdown
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    info!("API server shut down gracefully");
                    Ok(())
                }
                Ok(Err(e)) => {
                    error!("API server task panicked: {}", e);
                    Err(anyhow::anyhow!("Server task panicked: {}", e))
                }
                Err(_) => {
                    warn!("API server shutdown timed out after 5 seconds");
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }
}

/// Find an available port by binding to localhost:0
fn find_available_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("Failed to bind to localhost:0")?;
    let port = listener.local_addr()?.port();
    drop(listener); // Release the port
    Ok(port)
}

/// Start the embedded API server
pub async fn start_api_server(config: ApiServerConfig) -> Result<ApiServerHandle> {
    let port = find_available_port()?;
    let base_url = format!("http://localhost:{}", port);

    info!("Starting API server on {}", base_url);

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Clone config for the task
    let _task_config = config.clone();
    let task_base_url = base_url.clone();

    // Spawn the API server task
    let task_handle = tokio::spawn(async move {
        // TODO: Initialize rove-api server with:
        // - Port: port
        // - Bearer token: task_config.bearer_token
        // - State dir: task_config.state_dir
        // - Config dir: task_config.config_dir
        // - Logs dir: task_config.logs_dir
        // - CORS: allow localhost origins
        // - Shutdown signal: shutdown_rx

        info!("API server task started on {}", task_base_url);

        // For now, just wait for shutdown signal
        // Real implementation will start rove-api::serve() here
        let _ = shutdown_rx.await;

        info!("API server task shutting down");
    });

    // Wait for server to be ready
    let health_url = format!("{}/health", base_url);
    let mut ready = false;

    for attempt in 1..=30 {
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Try to connect to health endpoint
        match reqwest::get(&health_url).await {
            Ok(response) if response.status().is_success() => {
                info!("API server health check passed on attempt {}", attempt);
                ready = true;
                break;
            }
            Ok(response) => {
                warn!(
                    "API server health check returned status {} on attempt {}",
                    response.status(),
                    attempt
                );
            }
            Err(e) => {
                if attempt == 1 || attempt % 10 == 0 {
                    info!(
                        "Waiting for API server health check (attempt {}): {}",
                        attempt, e
                    );
                }
            }
        }
    }

    if !ready {
        error!("API server failed to become ready after 3 seconds");
        return Err(anyhow::anyhow!("API server health check timeout"));
    }

    Ok(ApiServerHandle {
        base_url,
        port,
        shutdown_tx: Some(shutdown_tx),
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_available_port() {
        let port = find_available_port().unwrap();
        assert!(port > 1024, "Port should be above 1024");
        assert!(port < 65535, "Port should be below 65535");
    }

    #[tokio::test]
    async fn test_server_lifecycle() {
        // This is a placeholder test
        // Real implementation will test actual server startup/shutdown
        let _config = ApiServerConfig {
            bearer_token: "test-token".to_string(),
            state_dir: std::env::temp_dir().join("rove-test-state"),
            config_dir: std::env::temp_dir().join("rove-test-config"),
            logs_dir: std::env::temp_dir().join("rove-test-logs"),
        };

        // TODO: Uncomment when real API server integration is complete
        // let handle = start_api_server(config).await.unwrap();
        // assert!(handle.port > 1024);
        // handle.shutdown().await.unwrap();
    }
}
