pub mod api_server;
pub mod commands;
pub mod config;

use anyhow::Result;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager, WebviewWindowBuilder, WindowEvent};
use tracing::{error, info};

/// Initialize the desktop application
pub fn init() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Rove Desktop initializing");

    Ok(())
}

/// Run the Tauri application
pub fn run() -> Result<()> {
    init()?;

    // Load config early to get bearer token
    let desktop_config = config::load_or_create_config()?;
    if desktop_config.bearer_token.trim().is_empty() {
        anyhow::bail!("desktop bearer token must not be empty");
    }
    config::install_crash_handler()?;
    let bearer_token = desktop_config.bearer_token.clone();

    tauri::Builder::default()
        .setup(move |app| {
            let server_state = tauri::async_runtime::block_on(setup_async(bearer_token.clone()))?;
            let api_url = server_state.base_url.clone().unwrap_or_default();
            let init_script = desktop_init_script(&bearer_token, &api_url)?;
            let window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Desktop window configuration is missing"))?;

            app.manage(server_state);
            app.manage(commands::WorkspaceRoots::for_process());
            WebviewWindowBuilder::from_config(app, &window_config)?
                .initialization_script(init_script)
                .build()?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let window = window.clone();
                let Some(state) = app.try_state::<ApiServerState>() else {
                    return;
                };
                if state.closing.swap(true, Ordering::AcqRel) {
                    return;
                }
                api.prevent_close();
                let state = state.inner.clone();
                tauri::async_runtime::spawn(async move {
                    let handle = state.lock().await.take();
                    if let Some(handle) = handle {
                        if let Err(error) = handle.shutdown().await {
                            error!("failed to shut down embedded API server: {error}");
                        }
                    }
                    let _ = window.close();
                    info!("Desktop API lifecycle stopped");
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_paths,
            commands::workspace_select,
            commands::provider_credential_prompt,
            commands::open_external,
            commands::show_in_folder,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Failed to run Tauri application: {}", e))?;

    Ok(())
}

fn desktop_init_script(bearer_token: &str, api_url: &str) -> Result<String> {
    let token = serde_json::to_string(bearer_token)?;
    let api_url = serde_json::to_string(api_url)?;
    let allow_dev_origin = if cfg!(debug_assertions) {
        "window.location.origin === 'http://localhost:3000'"
    } else {
        "false"
    };
    Ok(format!(
        r#"(() => {{
  if (window.top !== window) return;
  const appOrigin = window.location.protocol === 'tauri:' ||
    (window.location.protocol === 'http:' && window.location.hostname === 'tauri.localhost');
  if (!appOrigin && !({allow_dev_origin})) return;
  Object.defineProperty(window, '__ROVE_TOKEN__', {{ value: {token}, writable: false, configurable: false }});
  Object.defineProperty(window, '__ROVE_API_URL__', {{ value: {api_url}, writable: false, configurable: false }});
}})();"#
    ))
}

/// Async setup function
async fn setup_async(bearer_token: String) -> Result<ApiServerState> {
    info!("Starting async setup");

    // Get directories
    let state_dir = config::get_state_dir()?;
    let config_dir = config::get_config_dir()?;
    let logs_dir = config::get_logs_dir()?;

    // Ensure directories exist
    std::fs::create_dir_all(&state_dir)?;
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&logs_dir)?;

    // Start API server
    let api_config = api_server::ApiServerConfig {
        bearer_token: bearer_token.clone(),
        state_dir,
        config_dir,
        logs_dir,
    };

    let api_handle = api_server::start_api_server(api_config).await?;
    info!("API server started at {}", api_handle.base_url);

    let base_url = api_handle.base_url.clone();
    Ok(ApiServerState {
        base_url: Some(base_url),
        inner: Arc::new(tokio::sync::Mutex::new(Some(api_handle))),
        closing: Arc::new(AtomicBool::new(false)),
    })
}

/// API lifecycle state stored in Tauri app state.
#[derive(Clone)]
pub struct ApiServerState {
    pub(crate) base_url: Option<String>,
    pub(crate) inner: Arc<tokio::sync::Mutex<Option<api_server::ApiServerHandle>>>,
    pub(crate) closing: Arc<AtomicBool>,
}

pub fn startup_failure_message(log_path: &std::path::Path) -> String {
    format!(
        "Rove could not start its local API. Close any other Rove instances and try again. A redacted startup marker was written to {}.",
        log_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_gate_only_claims_the_first_close_request() {
        let closing = AtomicBool::new(false);
        assert!(!closing.swap(true, Ordering::AcqRel));
        assert!(closing.swap(true, Ordering::AcqRel));
    }

    #[test]
    fn initialization_script_is_origin_bounded_and_json_encoded() {
        let script = desktop_init_script("token-value", "http://127.0.0.1:49152").unwrap();
        assert!(script.contains("window.top !== window"));
        assert!(script.contains("window.location.protocol === 'tauri:'"));
        assert!(script.contains("window.location.hostname === 'tauri.localhost'"));
        assert!(script.contains("value: \"token-value\""));
        assert!(script.contains("value: \"http://127.0.0.1:49152\""));
        assert!(script.contains("writable: false"));
    }

    #[test]
    fn startup_failure_message_is_actionable_without_error_payloads() {
        let message =
            startup_failure_message(std::path::Path::new("C:/Rove/logs/desktop-startup.log"));
        assert!(message.contains("local API"));
        assert!(message.contains("desktop-startup.log"));
        assert!(!message.contains("provider credential"));
    }
}
