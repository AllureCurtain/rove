//! Local-only native folder picker for the product shell.
//!
//! Browser File System Access APIs cannot expose absolute paths. When a display
//! is available, this endpoint opens the OS folder dialog and returns the
//! selected absolute path. Headless/CI operators should set
//! `ROVE_DISABLE_NATIVE_FOLDER_PICKER=1`. Unix hosts without a display and
//! concurrent requests return `Unavailable` using the existing response schema.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use tokio::sync::Semaphore;
use utoipa::ToSchema;

use super::contracts::ProductErrorCode;
use crate::{ApiError, ApiState};

/// Opt out of the native dialog (headless CI, remote demos).
pub(crate) const DISABLE_FOLDER_PICKER_ENV: &str = "ROVE_DISABLE_NATIVE_FOLDER_PICKER";

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ProductWorkspacePickerResponse {
    Selected { path: String },
    Canceled,
    Unavailable { reason: String },
}

fn folder_picker_disabled() -> bool {
    matches!(
        std::env::var(DISABLE_FOLDER_PICKER_ENV),
        Ok(value) if matches!(value.as_str(), "1" | "true" | "yes")
    )
}

fn picker_unavailable_reason(disabled: bool, missing_display: bool) -> Option<&'static str> {
    if disabled {
        Some("native_folder_picker_disabled")
    } else if missing_display {
        Some("native_folder_picker_display_unavailable")
    } else {
        None
    }
}

fn display_missing() -> bool {
    // Do not enter a native backend on obviously headless Unix hosts. Display
    // variables do not apply to Windows or macOS; retain their native dialog.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        !["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        false
    }
}

fn picker_task_failure(_: tokio::task::JoinError) -> ApiError {
    // Never format the JoinError or its panic payload into responses or logs.
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: ProductErrorCode::ProductStorageFailure.as_str(),
        message: "native folder picker task failed".to_string(),
    }
}

async fn run_picker(
    gate: Arc<Semaphore>,
    picker: impl FnOnce() -> Option<PathBuf> + Send + 'static,
) -> Result<Json<ProductWorkspacePickerResponse>, ApiError> {
    let Ok(permit) = gate.try_acquire_owned() else {
        return Ok(Json(ProductWorkspacePickerResponse::Unavailable {
            reason: "native_folder_picker_busy".to_string(),
        }));
    };
    let selected = tokio::task::spawn_blocking(move || {
        // The task owns admission even if the HTTP request is canceled: a
        // running native dialog cannot be aborted by dropping its JoinHandle.
        let _permit = permit;
        picker()
    })
    .await
    .map_err(picker_task_failure)?;

    Ok(Json(match selected {
        Some(path) => ProductWorkspacePickerResponse::Selected {
            path: path.to_string_lossy().into_owned(),
        },
        None => ProductWorkspacePickerResponse::Canceled,
    }))
}

#[utoipa::path(
    post,
    path = "/product/workspace-picker",
    tag = crate::docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    responses(
        (status = 200, description = "Folder picker result", body = ProductWorkspacePickerResponse)
    )
)]
pub(crate) async fn pick_product_workspace_folder(
    State(_state): State<ApiState>,
) -> Result<Json<ProductWorkspacePickerResponse>, ApiError> {
    if let Some(reason) = picker_unavailable_reason(folder_picker_disabled(), display_missing()) {
        return Ok(Json(ProductWorkspacePickerResponse::Unavailable {
            reason: reason.to_string(),
        }));
    }

    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    run_picker(
        Arc::clone(GATE.get_or_init(|| Arc::new(Semaphore::new(1)))),
        || {
            rfd::FileDialog::new()
                .set_title("Open workspace folder")
                .pick_folder()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_preserves_enabled_desktop_and_rejects_headless() {
        assert_eq!(picker_unavailable_reason(false, false), None);
        assert_eq!(
            picker_unavailable_reason(false, true),
            Some("native_folder_picker_display_unavailable")
        );
        for missing_display in [false, true] {
            assert_eq!(
                picker_unavailable_reason(true, missing_display),
                Some("native_folder_picker_disabled")
            );
        }
    }

    #[tokio::test]
    async fn panicked_task_returns_safe_500_and_releases_gate() {
        let gate = Arc::new(Semaphore::new(1));
        let error = run_picker(Arc::clone(&gate), || panic!("private panic payload"))
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code, ProductErrorCode::ProductStorageFailure.as_str());
        assert_eq!(error.message, "native folder picker task failed");
        assert_eq!(gate.available_permits(), 1);
    }

    #[tokio::test]
    async fn canceled_task_returns_safe_500() {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let error = picker_task_failure(task.await.unwrap_err());
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.message, "native folder picker task failed");
    }

    #[tokio::test]
    async fn canceled_request_keeps_gate_until_native_task_finishes() {
        let gate = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let request = tokio::spawn(run_picker(Arc::clone(&gate), move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            None
        }));
        started_rx.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        let Json(result) = run_picker(Arc::clone(&gate), || panic!("must not open"))
            .await
            .unwrap();
        assert!(matches!(
            result,
            ProductWorkspacePickerResponse::Unavailable { reason }
                if reason == "native_folder_picker_busy"
        ));
        release_tx.send(()).unwrap();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), gate.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert!(matches!(
            run_picker(gate, || None).await.unwrap().0,
            ProductWorkspacePickerResponse::Canceled
        ));
    }
}
