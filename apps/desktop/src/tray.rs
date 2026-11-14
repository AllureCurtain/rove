//! The system tray icon.
//!
//! A coding agent runs tasks that outlive the user's attention at the window;
//! the tray keeps the process reachable when the window is hidden
//! (`CloseAction::HideToTray`). The menu is deliberately minimal: bring the
//! window back, or quit through the same `ExitRequested` drain the titlebar
//! close and app-menu Quit already use — no separate shutdown path.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

pub const MENU_SHOW: &str = "tray.show";
pub const MENU_QUIT: &str = "tray.quit";

/// Bring the (hidden or background) main window back to the foreground.
fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.webview_windows().values().next() {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Install the tray icon with a Show/Quit menu.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, MENU_SHOW, "Show Rove", true, None::<&str>)?,
            &MenuItem::with_id(app, MENU_QUIT, "Quit Rove", true, None::<&str>)?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("rove")
        .menu(&menu)
        .tooltip("Rove")
        .on_menu_event(|app, event| match event.id().0.as_str() {
            MENU_SHOW => show_main_window(app),
            MENU_QUIT => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    #[cfg(target_os = "macos")]
    {
        // macOS menu bars flip light/dark with the OS; a template icon lets the
        // system recolor it instead of shipping a baked-in color that can
        // disappear against either bar.
        builder = builder.icon_as_template(true);
    }
    builder.build(app)?;
    Ok(())
}
