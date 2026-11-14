//! Window geometry persistence across launches.
//!
//! A third-party `tauri-plugin-window-state` exists, but it writes to Tauri's
//! own `app_config_dir` (`~/Library/Application Support/com.rove.agent` on
//! macOS), bypassing `ROVE_DESKTOP_ROOT`. Persisting through `config`'s
//! directory keeps window state on the same isolation boundary as the bearer
//! token, state index and logs — test runs that redirect `ROVE_DESKTOP_ROOT`
//! to a scratch dir then keep the real profile untouched, and production
//! writes the standard per-OS config location.
//!
//! Only geometry is recorded — size, position and the maximized flag. The
//! plugin's other flags (visible, decorated, fullscreen) are either fixed by
//! `tauri.conf.json` or carry no product value, so they are intentionally not
//! modeled here.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{LogicalPosition, LogicalSize, WebviewWindow, WindowEvent};

const FILENAME: &str = "window-state.json";

/// The window geometry worth restoring on the next launch.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct WindowGeometry {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub maximized: bool,
}

fn state_path() -> Option<PathBuf> {
    crate::config::get_config_dir()
        .ok()
        .map(|dir| dir.join(FILENAME))
}

/// Load the persisted geometry, if any. A missing or unreadable file is a
/// first-launch, not an error — the caller falls back to `tauri.conf.json`.
pub fn load() -> Option<WindowGeometry> {
    let path = state_path()?;
    let raw = std::fs::read(&path).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// In-memory geometry tracker for one window, updated by move/resize events
/// and flushed once at exit. Tracking in memory avoids a write per mouse
/// movement; a single write on close is enough to restore the next launch.
pub struct Tracker {
    geometry: Mutex<WindowGeometry>,
}

impl Tracker {
    /// Build a tracker seeded from the window's real initial geometry, so a
    /// close without any movement still writes a coherent state.
    pub fn for_window(window: &WebviewWindow) -> Self {
        let mut geometry = WindowGeometry::default();
        if let Ok(size) = window.inner_size() {
            geometry.width = size.width;
            geometry.height = size.height;
        }
        if let Ok(position) = window.outer_position() {
            geometry.x = position.x;
            geometry.y = position.y;
        }
        geometry.maximized = window.is_maximized().unwrap_or(false);
        Self {
            geometry: Mutex::new(geometry),
        }
    }

    /// Apply a recorded geometry to a freshly built window. Invalid stored
    /// values (e.g. a path moved to a disconnected monitor) are simply set;
    /// the OS clamps them rather than refusing the launch.
    pub fn apply_saved(&self, window: &WebviewWindow, saved: WindowGeometry) {
        if saved.width > 0 && saved.height > 0 {
            let _ = window.set_size(LogicalSize::new(saved.width, saved.height));
        }
        if saved.maximized {
            let _ = window.maximize();
        } else {
            let _ = window.set_position(LogicalPosition::new(saved.x, saved.y));
        }
    }

    /// Fold one window event into the tracked geometry. `Resized` and `Moved`
    /// keep the latest values; `CloseRequested` and `Destroyed` trigger a save.
    pub fn on_event(&self, window: &WebviewWindow, event: &WindowEvent) {
        let mut geometry = self.geometry.lock().unwrap();
        match event {
            WindowEvent::Resized(size) => {
                if !window.is_minimized().unwrap_or(false)
                    && !window.is_maximized().unwrap_or(false)
                {
                    geometry.width = size.width;
                    geometry.height = size.height;
                }
                geometry.maximized = window.is_maximized().unwrap_or(false);
            }
            WindowEvent::Moved(position) => {
                if !window.is_minimized().unwrap_or(false) {
                    geometry.x = position.x;
                    geometry.y = position.y;
                }
            }
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
                geometry.maximized = window.is_maximized().unwrap_or(false);
                drop(geometry);
                self.save();
            }
            _ => {}
        }
    }

    /// Write the tracked geometry to the config directory. Best-effort: a
    /// failed write leaves the next launch at the default geometry, which is
    /// acceptable — never fail the exit path over cosmetics.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let geometry = *self.geometry.lock().unwrap();
        if let Ok(raw) = serde_json::to_vec_pretty(&geometry) {
            let _ = std::fs::write(path, raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_round_trips_through_json() {
        let geometry = WindowGeometry {
            width: 1440,
            height: 900,
            x: 120,
            y: 80,
            maximized: false,
        };
        let raw = serde_json::to_vec(&geometry).unwrap();
        let decoded: WindowGeometry = serde_json::from_slice(&raw).unwrap();
        assert_eq!(decoded, geometry);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        let decoded: WindowGeometry = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, WindowGeometry::default());
        assert!(!decoded.maximized);
    }

    #[test]
    fn tracker_save_writes_under_rove_desktop_root() {
        let root = tempfile::tempdir().unwrap();
        // Tracker::save resolves through config::get_config_dir, which honors
        // ROVE_DESKTOP_ROOT; point it at the scratch dir so the write lands
        // there rather than the real profile.
        std::env::set_var(crate::config::DESKTOP_ROOT_ENV, root.path());
        let tracker = Tracker {
            geometry: Mutex::new(WindowGeometry {
                width: 1024,
                height: 768,
                x: 10,
                y: 20,
                maximized: true,
            }),
        };
        tracker.save();
        std::env::remove_var(crate::config::DESKTOP_ROOT_ENV);
        let written = root.path().join("config").join(FILENAME);
        let decoded: WindowGeometry =
            serde_json::from_slice(&std::fs::read(written).unwrap()).unwrap();
        assert_eq!(decoded.width, 1024);
        assert!(decoded.maximized);
    }
}
