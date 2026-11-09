use anyhow::{Context, Result};
use rove_api::{embedded_api_state, serve_state_listener};
use std::path::{Path, PathBuf};
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
    pub(crate) api_state: rove_api::ApiState,
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
    let cwd = std::env::current_dir().context("failed to resolve desktop working directory")?;
    start_api_server_at(config, &cwd).await
}

async fn start_api_server_at(config: ApiServerConfig, cwd: &Path) -> Result<ApiServerHandle> {
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
    let state = embedded_api_state(
        cwd,
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
    let api_state = state.clone();
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
        api_state,
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    async fn get_json(client: &reqwest::Client, url: &str) -> Value {
        let response = client
            .get(url)
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        serde_json::from_str(&response.text().await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn shutdown_drains_pending_approval_and_streams_before_restart() {
        let root = std::env::temp_dir().join(format!("rove-desktop-exit-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = ApiServerConfig {
            bearer_token: "test-token".to_string(),
            state_dir: root.join("state"),
            config_dir: root.join("config"),
            logs_dir: root.join("logs"),
        };
        let handle = start_api_server_at(config.clone(), &workspace)
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        let response = client
            .post(format!("{}/jobs", handle.base_url))
            .bearer_auth("test-token")
            .header("content-type", "application/json")
            .body(
                json!({
                    "message": json!({"tool":"write_file", "args":{
                        "path":"must-not-exist.txt", "content":"unapproved"
                    }}).to_string(),
                    "model":"fake-raw", "approval":"ask", "max_steps":1
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let job: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        let state_path = format!("/jobs/{}/state", job["job_id"].as_str().unwrap());
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let state = get_json(&client, &format!("{}{state_path}", handle.base_url)).await;
                if !state["pending_approvals"].as_array().unwrap().is_empty() {
                    assert_eq!(state["pending_approvals"][0]["name"], "write_file");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("job must reach the approval gate");

        let job_events = client
            .get(format!(
                "{}/jobs/{}/events",
                handle.base_url,
                job["job_id"].as_str().unwrap()
            ))
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let product_events = client
            .get(format!("{}/product/events", handle.base_url))
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let old_url = handle.base_url.clone();
        let host = crate::ApiServerState::new(handle);
        assert_eq!(
            host.exit_gate.request(),
            crate::lifecycle::ExitAction::StartShutdown
        );
        tokio::join!(
            host.shutdown_api(),
            host.shutdown_api(),
            host.shutdown_api()
        );
        assert_eq!(host.exit_gate.request(), crate::lifecycle::ExitAction::Exit);
        let terminal_events = job_events.text().await.unwrap();
        assert!(terminal_events.contains("run_completed"));
        assert!(terminal_events.contains("cancelled"));
        product_events.text().await.unwrap();
        assert!(client
            .get(format!("{old_url}/product/runtime"))
            .send()
            .await
            .is_err());
        assert!(!workspace.join("must-not-exist.txt").exists());

        let restarted = start_api_server_at(config, &workspace).await.unwrap();
        let state = get_json(&client, &format!("{}{state_path}", restarted.base_url)).await;
        assert_eq!(state["status"], "cancelled");
        assert!(state["pending_approvals"].as_array().unwrap().is_empty());
        assert!(state["events"].as_array().unwrap().iter().any(|e| {
            e["event"]["type"] == "run_completed" && e["event"]["reason"] == "cancelled"
        }));
        restarted.shutdown().await.unwrap();
        assert!(!workspace.join("must-not-exist.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

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
