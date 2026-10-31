//! The router fallbacks: JSON 404/405 for the API, and the built SPA from `STATIC_DIR`
//! (docs/phase1/conventions.md §3, endpoint-map §1).

use std::path::Path;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::AppState;
use crate::error::{AppError, msg};

const NO_STORE: &str = "no-store";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The Content-Security-Policy of every answer the SPA fallback gives (the shell, its assets and
/// the other built files). The browser loads nothing from another origin, which the privacy
/// text promises:
/// - scripts only from our files: `index.html` has no inline script (the theme boot script is
///   `/theme-init.js`), Vite emits none, and vue-i18n 11 interprets messages without `eval`.
/// - WebAssembly compiles from our own `/assets` file only, for the barcode decoder (#56):
///   `'wasm-unsafe-eval'`; `'unsafe-eval'` stays out.
/// - inline styles stay allowed: bound `style` attributes and the drag-and-drop library set
///   them, and a style cannot run code.
/// - pictures from `/media`, plus `data:` and `blob:` URLs, so a local preview of a picture
///   before its upload needs no change here.
/// - the manifest from our file or the `blob:` URL `brand.ts` builds from `/api/config`.
/// - no plugins, no `<base>` elsewhere, forms only to us, and no framing at all (the API's
///   `frame-ancestors 'none'` from `lib.rs`, which keeps this header when it is present).
pub const SHELL_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; \
    connect-src 'self'; manifest-src 'self' blob:; object-src 'none'; base-uri 'self'; \
    form-action 'self'; frame-ancestors 'none'";

/// Every request no route matched. `/api` and `/media` paths get a JSON 404; with `STATIC_DIR`
/// set, GET and HEAD get a file or the SPA shell with [`SHELL_CSP`], other methods a JSON 405.
pub async fn fallback(State(state): State<AppState>, request: Request) -> Response {
    let path = request.uri().path();
    let is_api = path == "/api" || path.starts_with("/api/") || path.starts_with("/media/");
    match &state.config.static_dir {
        Some(dir) if !is_api => {
            if matches!(*request.method(), Method::GET | Method::HEAD) {
                let mut response = serve(dir, request).await;
                response
                    .headers_mut()
                    .insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(SHELL_CSP));
                response
            } else {
                method_not_allowed().await.into_response()
            }
        }
        _ => AppError::not_found(msg::NOT_FOUND).into_response(),
    }
}

/// A route exists for the path but not for the method.
pub async fn method_not_allowed() -> AppError {
    AppError::new(StatusCode::METHOD_NOT_ALLOWED, msg::METHOD_NOT_ALLOWED)
}

/// Serves the SPA:
/// - `/assets/*`: the hashed build output, cached for a year; a miss is a JSON 404, never the
///   shell, so a stale shell's missing chunk fails loudly.
/// - any other existing file: as is (an `index.html` with `no-store`).
/// - everything else: `index.html` with `no-store`, so clients never keep a shell that points
///   at old assets.
async fn serve(dir: &Path, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let files = ServeDir::new(dir).append_index_html_on_directories(false);
    let response = match files.oneshot(request).await {
        Ok(response) => response.map(Body::new),
        Err(infallible) => match infallible {},
    };
    let status = response.status();

    if path.starts_with("/assets/") {
        if status.is_success() || status == StatusCode::NOT_MODIFIED {
            return with_cache_control(response, IMMUTABLE);
        }
        if status.is_client_error() {
            return AppError::not_found(msg::NOT_FOUND).into_response();
        }
        return response;
    }
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        if path.ends_with("/index.html") {
            return with_cache_control(response, NO_STORE);
        }
        return response;
    }
    shell(dir).await
}

/// `index.html`, read on every request so a new build needs no restart.
async fn shell(dir: &Path) -> Response {
    match tokio::fs::read(dir.join("index.html")).await {
        Ok(html) => (
            [
                (
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (CACHE_CONTROL, HeaderValue::from_static(NO_STORE)),
            ],
            html,
        )
            .into_response(),
        Err(_) => AppError::not_found(msg::NOT_FOUND).into_response(),
    }
}

fn with_cache_control(mut response: Response, value: &'static str) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    response
}
