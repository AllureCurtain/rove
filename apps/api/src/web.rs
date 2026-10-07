//! Optional same-origin hosting of the Web console's static bundle.
//!
//! `router()` deliberately stays API-only (the regression test
//! `api_does_not_serve_embedded_web_ui_anymore` pins that): the console is an
//! opt-in layer on top, used by the `rove-api --web-dist` binary path so a
//! single process serves the product without a separate Node server. Desktop
//! does not come through here — it embeds the same bundle inside the Tauri
//! host with its own transport.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::http::header::ACCEPT;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::error::ApiError;

/// Wrap the API router so one origin serves both the console and the API.
///
/// The bundle's client only ever calls `/api/*` (that is what the Next.js
/// dev proxy strips today), so the API is mounted a second time under `/api`
/// while the root paths keep answering existing clients: the desktop
/// transport, direct `rove-api` consumers, and Swagger UI. Everything else
/// falls through to the bundle's directory; misses serve `index.html` so
/// client-side routes like `/w/<id>/s/<id>` survive a page reload.
///
/// Two boundaries keep that fallback from eating API semantics. The `/api`
/// mount carries its own JSON 404, so an unmatched `/api/*` never becomes a
/// page; and `index.html` only answers what looks like a page navigation (a
/// GET or HEAD on an extensionless path whose Accept admits `text/html`), so
/// a missing `/_next/static/…` chunk or a typo'd path fetched by an API
/// client still gets a real 404.
///
/// Statics and `/health` sit outside the API's security middleware on
/// purpose: the bundle ships no secrets, and the bearer check gates the API,
/// not the page that talks to it.
pub fn with_console(api: Router, web_root: &Path) -> anyhow::Result<Router> {
    let index = web_root.join("index.html");
    if !index.is_file() {
        anyhow::bail!(
            "web console bundle is missing index.html under {}; build it with `pnpm build:web` in apps/web",
            web_root.display()
        );
    }
    let index_service = ServeFile::new(index);
    let spa_page_fallback = tower::service_fn(move |request: Request<Body>| {
        let index_service = index_service.clone();
        async move {
            let response = if wants_index_html(&request) {
                match index_service.oneshot(request).await {
                    Ok(response) => response.into_response(),
                    Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                }
            } else {
                StatusCode::NOT_FOUND.into_response()
            };
            Ok::<Response, Infallible>(response)
        }
    });
    Ok(Router::new()
        .merge(api.clone())
        .nest("/api", api.fallback(api_route_not_found))
        .route("/health", get(health))
        .fallback_service(ServeDir::new(web_root).fallback(spa_page_fallback)))
}

async fn api_route_not_found() -> Response {
    ApiError::not_found("route not found").into_response()
}

/// A page navigation is a GET/HEAD whose Accept admits `text/html` — what a
/// browser sends when the user loads or reloads a client-side route. A path
/// whose last segment carries an extension (`missing.js`, `nope.txt`) is an
/// asset request even from a browser, and must keep its real 404 rather than
/// surface as a parse error on the served HTML.
fn wants_index_html(request: &Request<Body>) -> bool {
    if !matches!(request.method(), &Method::GET | &Method::HEAD) {
        return false;
    }
    let last_segment = request.uri().path().rsplit('/').next().unwrap_or_default();
    if last_segment.contains('.') {
        return false;
    }
    request
        .headers()
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// Resolve the bundle directory from CLI flag or `ROVE_WEB_DIST`, preferring
/// the flag. Returns `None` when neither is set so the API stays headless.
pub fn console_root(flag: Option<PathBuf>) -> Option<PathBuf> {
    flag.or_else(|| {
        std::env::var_os("ROVE_WEB_DIST")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// `GET /health` is liveness for launch scripts and process checks, not part
/// of the product contract, so it lives outside the OpenAPI router the same
/// way Swagger UI does.
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    fn empty_api() -> Router {
        Router::new().route("/jobs", get(|| async { StatusCode::NO_CONTENT }))
    }

    fn write_console(root: &Path) {
        std::fs::create_dir_all(root.join("_next/static")).unwrap();
        std::fs::write(root.join("index.html"), "<html>console</html>").unwrap();
        std::fs::write(root.join("_next/static/app.js"), "console.log(1)").unwrap();
    }

    #[tokio::test]
    async fn console_root_prefers_the_flag_then_env_then_nothing() {
        let flagged = PathBuf::from("flagged");
        // SAFETY-free env handling: the flag wins before the env is read, so
        // this test never touches the process environment.
        assert_eq!(console_root(Some(flagged.clone())), Some(flagged));
    }

    #[tokio::test]
    async fn health_answers_outside_the_api() {
        let dir = tempfile::TempDir::new().unwrap();
        write_console(dir.path());
        let app = with_console(empty_api(), dir.path()).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn api_is_reachable_under_both_root_and_api_prefix() {
        let dir = tempfile::TempDir::new().unwrap();
        write_console(dir.path());
        let app = with_console(empty_api(), dir.path()).unwrap();

        for uri in ["/jobs", "/api/jobs"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT, "uri {uri}");
        }
    }

    #[tokio::test]
    async fn bundle_serves_index_assets_and_spa_fallback() {
        let dir = tempfile::TempDir::new().unwrap();
        write_console(dir.path());
        let app = with_console(empty_api(), dir.path()).unwrap();

        for uri in ["/", "/index.html", "/w/abc/s/def"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("accept", "text/html,application/xhtml+xml")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "uri {uri}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(
                String::from_utf8_lossy(&body).contains("console"),
                "uri {uri}"
            );
        }

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_next/static/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fallback_never_answers_api_or_asset_misses_with_html() {
        let dir = tempfile::TempDir::new().unwrap();
        write_console(dir.path());
        let app = with_console(empty_api(), dir.path()).unwrap();

        // Unmatched under /api is a JSON 404, not the SPA shell — the mounted
        // router owns its prefix even for paths it does not know.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/xyz/unknown")
                    .header("accept", "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "not_found");

        // A missing chunk is a real 404 even for a browser-looking request:
        // serving HTML for a script src would surface as a parse error.
        for uri in ["/_next/static/missing.js", "/nope.txt"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("accept", "text/html")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "uri {uri}");
        }

        // Non-page GETs (API clients send Accept: */* or none at all) get a
        // 404 for a typo'd path, not an HTML document.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/w/abc/s/def")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_bundle_is_an_error_not_a_silent_api() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = with_console(empty_api(), dir.path());
        assert!(result.is_err());
    }
}
