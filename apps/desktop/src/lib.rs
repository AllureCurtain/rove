pub mod api_server;
pub mod commands;
pub mod config;
mod lifecycle;
mod menu;
pub mod notifications;
mod tray;
mod window_state;

use anyhow::Result;
use lifecycle::{ExitAction, ExitGate};
use std::sync::{Arc, RwLock};
use tauri::{Manager, RunEvent, WebviewWindowBuilder, WindowEvent};
use tracing::{error, info};

/// Process-wide copy of the persisted `close_action`, so the window-event
/// handler reads memory instead of disk on every close request.
#[derive(Clone)]
pub struct CloseActionState(pub Arc<RwLock<config::CloseAction>>);

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
    let notifications_token = bearer_token.clone();
    let close_action_state = CloseActionState(Arc::new(RwLock::new(desktop_config.close_action)));
    // Startup errors must reach main's native dialog, not panic inside Tauri's
    // setup callback (which runs after Builder::build returns).
    let server_state = tauri::async_runtime::block_on(setup_async(bearer_token.clone()))?;

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
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
            app.manage(close_action_state);
            let window = WebviewWindowBuilder::from_config(app, &window_config)?
                .initialization_script(init_script)
                .build()?;

            let tracker = window_state::Tracker::for_window(&window);
            if let Some(saved) = window_state::load() {
                tracker.apply_saved(&window, saved);
            }
            app.manage(tracker);
            menu::install(app.handle())?;
            tray::install(app.handle())?;
            if let Some(state) = app.try_state::<ApiServerState>() {
                notifications::spawn(
                    state.inner().clone(),
                    app.handle().clone(),
                    notifications_token.clone(),
                    state.stop_token(),
                );
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            if let (Some(tracker), Some(webview_window)) = (
                app.try_state::<window_state::Tracker>(),
                app.get_webview_window(window.label()),
            ) {
                tracker.on_event(&webview_window, event);
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let hide_to_tray = window
                    .app_handle()
                    .try_state::<CloseActionState>()
                    .map(|state| *state.0.read().unwrap_or_else(|e| e.into_inner()))
                    == Some(config::CloseAction::HideToTray);
                if hide_to_tray {
                    // The app stays alive in the tray; Quit there (or the app
                    // menu) still routes through the ExitRequested drain.
                    let _ = window.hide();
                } else {
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_paths,
            commands::workspace_select,
            commands::provider_credential_prompt,
            commands::provider_profile_probe,
            commands::provider_profile_use,
            commands::open_external,
            commands::show_in_folder,
            commands::get_close_action,
            commands::set_close_action,
        ])
        .build(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Failed to build Tauri application: {}", e))?
        .run(|app, event| match event {
            RunEvent::ExitRequested { api, code, .. } => {
                let Some(state) = app.try_state::<ApiServerState>() else {
                    return;
                };
                match state.exit_gate.request() {
                    ExitAction::Exit => {}
                    ExitAction::Wait => api.prevent_exit(),
                    ExitAction::StartShutdown => {
                        api.prevent_exit();
                        let state = state.inner().clone();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            state.shutdown_api().await;
                            app.exit(code.unwrap_or(0));
                        });
                    }
                }
            }
            RunEvent::Exit => {
                // Last reliable point to flush window geometry before the
                // process is gone; CloseRequested/Destroyed may not fire on a
                // native Quit.
                if let Some(tracker) = app.try_state::<window_state::Tracker>() {
                    tracker.save();
                }
                // Native macOS Quit can skip ExitRequested. Tauri calls this
                // before cleanup; join any existing drain before returning.
                if let Some(state) = app.try_state::<ApiServerState>() {
                    tauri::async_runtime::block_on(state.shutdown_api());
                }
            }
            _ => {}
        });

    Ok(())
}

fn desktop_init_script(bearer_token: &str, api_url: &str) -> Result<String> {
    let token = serde_json::to_string(bearer_token)?;
    let api_url = serde_json::to_string(api_url)?;
    let credential_entry = if cfg!(target_os = "windows") {
        "native"
    } else {
        "loopback"
    };
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
  Object.defineProperty(window, '__ROVE_CREDENTIAL_ENTRY__', {{ value: '{credential_entry}', writable: false, configurable: false }});
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

    Ok(ApiServerState::new(api_handle))
}

/// API lifecycle state stored in Tauri app state.
#[derive(Clone)]
pub struct ApiServerState {
    pub(crate) base_url: Option<String>,
    pub(crate) inner: Arc<tokio::sync::Mutex<Option<api_server::ApiServerHandle>>>,
    pub(crate) api_state: rove_api::ApiState,
    exit_gate: Arc<ExitGate>,
    shutdown_complete: Arc<tokio::sync::OnceCell<()>>,
    /// Stops background consumers of the embedded API (the notification pump)
    /// before the server itself is asked to drain.
    stop: tokio_util::sync::CancellationToken,
}

impl ApiServerState {
    fn new(handle: api_server::ApiServerHandle) -> Self {
        Self {
            base_url: Some(handle.base_url.clone()),
            api_state: handle.api_state.clone(),
            inner: Arc::new(tokio::sync::Mutex::new(Some(handle))),
            exit_gate: Arc::new(ExitGate::default()),
            shutdown_complete: Arc::new(tokio::sync::OnceCell::new()),
            stop: tokio_util::sync::CancellationToken::new(),
        }
    }

    pub(crate) fn stop_token(&self) -> tokio_util::sync::CancellationToken {
        self.stop.clone()
    }

    async fn shutdown_api(&self) {
        self.shutdown_complete
            .get_or_init(|| async {
                self.stop.cancel();
                let handle = self.inner.lock().await.take();
                if let Some(handle) = handle {
                    if let Err(error) = handle.shutdown().await {
                        error!("failed to shut down embedded API server: {error}");
                    }
                }
                info!("Desktop API lifecycle stopped");
                self.exit_gate.finish();
            })
            .await;
    }
}

pub fn startup_failure_message(log_path: &std::path::Path) -> String {
    format!(
        "Rove could not start its local API. Close any other Rove instances and try again. A redacted startup marker was written to {}.",
        log_path.display()
    )
}

pub fn startup_error_message(error: &anyhow::Error, log_path: &std::path::Path) -> String {
    if error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rove_api::ApiOwnershipError>(),
            Some(rove_api::ApiOwnershipError::Busy)
        )
    }) {
        return "Rove is already using this data directory. Return to the running app, or close it before opening another instance.".to_string();
    }
    startup_failure_message(log_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_script_is_origin_bounded_and_json_encoded() {
        let script = desktop_init_script("token-value", "http://127.0.0.1:49152").unwrap();
        assert!(script.contains("window.top !== window"));
        assert!(script.contains("window.location.protocol === 'tauri:'"));
        assert!(script.contains("window.location.hostname === 'tauri.localhost'"));
        assert!(script.contains("value: \"token-value\""));
        assert!(script.contains("value: \"http://127.0.0.1:49152\""));
        assert!(script.contains("writable: false"));
        let entry = if cfg!(target_os = "windows") {
            "native"
        } else {
            "loopback"
        };
        assert!(script.contains(&format!("value: '{entry}'")));
    }

    #[test]
    fn startup_failure_message_is_actionable_without_error_payloads() {
        let message =
            startup_failure_message(std::path::Path::new("C:/Rove/logs/desktop-startup.log"));
        assert!(message.contains("local API"));
        assert!(message.contains("desktop-startup.log"));
        assert!(!message.contains("provider credential"));
    }

    #[test]
    fn duplicate_host_error_is_actionable_through_context_without_payloads() {
        let error =
            anyhow::Error::new(rove_api::ApiOwnershipError::Busy).context("secret startup context");
        let text = startup_error_message(&error, std::path::Path::new("logs"));
        assert!(text.contains("already using"));
        assert!(!text.contains("secret"));
    }
}
