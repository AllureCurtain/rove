//! The application menu bar.
//!
//! `Menu::default()` gives a platform-native skeleton but no About item and a
//! Windows File menu that does not match the product. This builds the
//! conventional set: an app menu on macOS (About, hide/show, Quit) plus the
//! standard Edit/View/Window columns, and a leaner File/Edit/View/Window set
//! on Windows and Linux where there is no system-owned app menu.
//!
//! Quit is left to the platform predefined item; it drives the same
//! `ExitRequested` path as the window close, which already routes through the
//! `ExitGate` drain — no separate shutdown wiring is added here.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime};

/// Build and install the platform-appropriate menu bar.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build(app)?;
    app.set_menu(menu)?;
    // Only the custom `view.reload` item needs an action; the predefined
    // items carry their own platform behavior (Quit already routes through
    // the ExitRequested drain, window items are handled by the OS).
    app.on_menu_event(|app, event| {
        if event.id().0 == "view.reload" {
            if let Some(window) = app.webview_windows().values().next() {
                let _ = window.eval("window.location.reload()");
            }
        }
    });
    Ok(())
}

fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let edit = edit_menu(app)?;
    let view = view_menu(app)?;
    let window = window_menu(app)?;

    #[cfg(target_os = "macos")]
    {
        Menu::with_items(app, &[&app_menu(app)?, &edit, &view, &window])
    }

    #[cfg(not(target_os = "macos"))]
    {
        Menu::with_items(app, &[&file_menu(app)?, &edit, &view, &window])
    }
}

/// macOS app menu: the bold "Rove" column the system expects first.
#[cfg(target_os = "macos")]
fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "Rove",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Rove"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Hide Rove"))?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit Rove"))?,
        ],
    )
}

/// Windows/Linux first column: File with just Quit, since there is no
/// system-managed app menu on those platforms.
#[cfg(not(target_os = "macos"))]
fn file_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "File",
        true,
        &[&PredefinedMenuItem::quit(app, Some("Quit"))?],
    )
}

fn edit_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )
}

fn view_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "View",
        true,
        &[
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "view.reload", "Reload", true, Some("CmdOrCtrl+R"))?,
        ],
    )
}

fn window_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )
}
