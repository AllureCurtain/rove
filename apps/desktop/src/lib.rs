pub mod api_server;
pub mod commands;
pub mod config;

use anyhow::Result;
use tauri::{Manager, WindowEvent};
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
    let bearer_token = desktop_config.bearer_token.clone();

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            let token_for_async = bearer_token.clone();

            // Spawn async initialization
            tauri::async_runtime::spawn(async move {
                match setup_async(handle, token_for_async).await {
                    Ok(()) => {
                        info!("Desktop application setup completed successfully");
                    }
                    Err(e) => {
                        error!("Failed to setup desktop application: {}", e);
                        std::process::exit(1);
                    }
                }
            });

            // Inject initialization script into all windows
            let init_script = format!(
                r#"
                window.__ROVE_TOKEN__ = "{}";
                console.log("Rove Desktop: Bearer token injected");
                "#,
                bearer_token
            );

            for (_label, window) in app.webview_windows() {
                window
                    .eval(&init_script)
                    .map_err(|e| anyhow::anyhow!("Failed to inject init script: {}", e))?;
            }

            Ok(())
        })
        .on_window_event(|_window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                info!("Window close requested, shutting down");
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_paths,
            commands::workspace_select,
            commands::open_external,
            commands::show_in_folder,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Failed to run Tauri application: {}", e))?;

    Ok(())
}

/// Async setup function
async fn setup_async(app_handle: tauri::AppHandle, bearer_token: String) -> Result<()> {
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

    // Store API URL in app state
    app_handle.manage(ApiState {
        base_url: api_handle.base_url.clone(),
        bearer_token,
    });

    // Inject API URL into all windows
    let api_url_script = format!(
        r#"
        window.__ROVE_API_URL__ = "{}";
        console.log("Rove Desktop: API URL injected -", "{}");
        "#,
        api_handle.base_url, api_handle.base_url
    );

    for (_label, window) in app_handle.webview_windows() {
        if let Err(e) = window.eval(&api_url_script) {
            error!("Failed to inject API URL: {}", e);
        }
    }

    // Store server handle for cleanup
    app_handle.manage(api_handle);

    Ok(())
}

/// API state stored in Tauri app state
#[derive(Clone)]
#[allow(dead_code)] // Fields will be used when API integration is complete
struct ApiState {
    base_url: String,
    bearer_token: String,
}
