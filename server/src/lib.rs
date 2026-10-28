//! Bookclub server: the HTTP API for the hosted multi-club service.
//!
//! This file only assembles the router; docs/phase1/conventions.md §2 and §3 describe it.

pub mod activity;
pub mod admin;
pub mod auth;
pub mod catalog;
pub mod client_ip;
pub mod clubs;
pub mod config;
pub mod dto;
pub mod email;
pub mod error;
pub mod extract;
pub mod harvest;
mod health;
pub mod importer;
pub mod jobs;
pub mod media;
pub mod openapi;
pub mod outbound;
pub mod overlap;
pub mod picks;
pub mod posts;
pub mod ratelimit;
pub mod serve;
pub mod shelf;
pub mod spa;
pub mod text;
pub mod tz;
pub mod visibility;
pub mod votes;

use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::DefaultBodyLimit;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::auth::extract::InstanceAdmin;
use crate::config::{Config, EmailTransport};
use crate::error::{AppError, msg};

/// Body limit for every route that does not set its own (uploads do, conventions §4.6).
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Shared state handed to every request handler. Frozen after Wave 1: packages keep their
/// private state inside `CatalogService` / `MediaService`, never in new fields.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub limits: Arc<ratelimit::RateLimits>,
    pub outbound: Arc<outbound::Outbound>,
    /// argon2 parameters plus the concurrency semaphore.
    pub passwords: Arc<auth::password::Passwords>,
    pub email: Arc<dyn email::EmailSender>,
    pub media: Arc<dyn media::shared::MediaStore>,
    /// CAT-owned state; CAT adds fields to the struct freely.
    pub catalog: Arc<catalog::CatalogService>,
    /// `catalog` in production; `FakeCatalog` in tests.
    pub lookup: Arc<dyn catalog::shared::lookup::CatalogLookup>,
    /// MED-owned state (fetch coalescing, image limits, the prefetch queue).
    pub media_service: Arc<media::MediaService>,
}

impl AppState {
    /// The production state. Tests build theirs through the harness (tests/common).
    pub fn new(db: PgPool, config: Config) -> anyhow::Result<Self> {
        let config = Arc::new(config);
        let outbound = Arc::new(outbound::Outbound::new(&config)?);
        let media: Arc<dyn media::shared::MediaStore> =
            Arc::new(media::shared::FsMediaStore::new(config.media_dir.clone()));
        let email: Arc<dyn email::EmailSender> = match config.email_transport {
            EmailTransport::Log => Arc::new(email::LogEmail),
            EmailTransport::Smtp => Arc::new(email::SmtpEmail::new(&config)?),
        };
        let catalog = Arc::new(catalog::CatalogService::new(&config, outbound.clone()));
        let media_service = Arc::new(media::MediaService::new(
            &config,
            media.clone(),
            outbound.clone(),
        ));
        Ok(Self {
            db,
            limits: Arc::new(ratelimit::RateLimits::new()),
            passwords: Arc::new(auth::password::Passwords::new()),
            email,
            media,
            lookup: catalog.clone(),
            catalog,
            media_service,
            outbound,
            config,
        })
    }
}

/// Every route outside `/api/admin`, with its OpenAPI paths.
fn api_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(openapi::ApiDoc::openapi())
        .merge(health::routes())
        .merge(openapi::routes())
        .merge(auth::routes())
        .merge(clubs::routes())
        .merge(catalog::routes())
        .merge(harvest::routes())
        .merge(media::routes())
        .merge(shelf::routes())
        .merge(posts::routes())
        .merge(picks::routes())
        .merge(votes::routes())
        .merge(overlap::routes())
        .merge(activity::routes())
}

/// Every `/api/admin/*` route.
fn admin_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(admin::invites::admin_routes())
        .merge(catalog::admin_routes())
        .merge(media::admin_routes())
}

/// The OpenAPI document of every route (`GET /api/openapi.json`, `bookclub-server openapi`).
pub fn openapi() -> utoipa::openapi::OpenApi {
    api_router().merge(admin_router()).split_for_parts().1
}

/// The complete application: routes, fallbacks and layers.
pub fn app(state: AppState) -> Router {
    let (api, _) = api_router().split_for_parts();
    let (admin, _) = admin_router().split_for_parts();
    // One guard for every admin route: a handler that forgets its InstanceAdmin argument is
    // still guarded. (`admin::invites` always has routes; axum panics on a route layer over an
    // empty router.)
    let admin = admin.route_layer(middleware::from_extractor_with_state::<
        InstanceAdmin,
        AppState,
    >(state.clone()));

    // Layers run outermost-last: the last `.layer` call wraps all the others.
    let mut router = api
        .merge(admin)
        // API answers carry personal data (the account, shelves, a club's invite codes): no
        // shared cache may keep one for the next user, and no browser cache after a logout.
        // A route that sets its own (`/media`, the export) keeps it.
        .route_layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .fallback(spa::fallback)
        .method_not_allowed_fallback(spa::method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session::maintain_cookie,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::origin::check,
        ));
    if let Some(cors) = cors_layer(&state.config) {
        router = router.layer(cors);
    }
    router
        .layer(middleware::map_response(json_server_errors))
        .layer(middleware::from_fn(|request, next| {
            serve::deadline(serve::REQUEST_DEADLINE, request, next)
        }))
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let request_id = request
                        .headers()
                        .get("x-request-id")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        path = %loggable_path(request.uri().path()),
                        request_id,
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        // Outside the origin check, CORS and the panic handler, so their answers carry them too.
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        // No page of ours may be framed: a same-site page (a sibling subdomain) would get the
        // Lax session cookie into the frame and could trick a click on a one-tap action.
        // `if_not_present`: the SPA's answers carry the full policy (`spa::SHELL_CSP`, which
        // also says `frame-ancestors 'none'`); every other answer gets this one.
        .layer(SetResponseHeaderLayer::if_not_present(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(state)
}

/// CORS for `CORS_ORIGINS` (the Phase 4 apps) and the Vite origin in DEV_MODE; none otherwise.
fn cors_layer(config: &Config) -> Option<CorsLayer> {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins()
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    if origins.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_credentials(true)
            .allow_methods(AllowMethods::mirror_request())
            .allow_headers(AllowHeaders::mirror_request()),
    )
}

/// The path for the access log, without invite codes: a code is a credential (§19), and
/// `/api/invites/{code}`, `/api/invites/{code}/accept` and the SPA's `/join/{code}` carry one.
fn loggable_path(path: &str) -> Cow<'_, str> {
    for prefix in ["/api/invites/", "/join/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let tail = rest.find('/').map_or("", |slash| &rest[slash..]);
            return Cow::Owned(format!("{prefix}{{code}}{tail}"));
        }
    }
    Cow::Borrowed(path)
}

/// A plain-text 500 becomes the JSON 500 of every other error (§4.1). axum answers one when a
/// response body fails to serialise (a timestamp outside the years RFC 3339 can write, say).
async fn json_server_errors(response: Response) -> Response {
    let plain = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/plain"));
    if response.status() != StatusCode::INTERNAL_SERVER_ERROR || !plain {
        return response;
    }
    let text = to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    AppError::internal(String::from_utf8_lossy(&text)).into_response()
}

/// A panicking handler answers 500 `{"detail":"Internal Server Error"}` like every other error.
fn panic_response(panic: Box<dyn Any + Send + 'static>) -> Response<Body> {
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("(no message)");
    tracing::error!(panic = message, "request handler panicked");
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, msg::INTERNAL).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_codes_stay_out_of_the_log() {
        for (path, logged) in [
            ("/api/invites/AB12CD34EF", "/api/invites/{code}"),
            (
                "/api/invites/AB12CD34EF/accept",
                "/api/invites/{code}/accept",
            ),
            ("/join/AB12CD34EF", "/join/{code}"),
            (
                "/api/clubs/readers-x1y2/invites/7",
                "/api/clubs/readers-x1y2/invites/7",
            ),
            ("/api/books/works/42", "/api/books/works/42"),
        ] {
            assert_eq!(loggable_path(path), logged);
        }
    }

    #[tokio::test]
    async fn a_plain_text_500_becomes_json() {
        let plain = (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(CONTENT_TYPE, "text/plain; charset=utf-8")],
            "The year component cannot be formatted into the requested format.",
        )
            .into_response();
        let response = json_server_errors(plain).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], br#"{"detail":"Internal Server Error"}"#);
        // Everything else passes untouched.
        let teapot = (StatusCode::IM_A_TEAPOT, "short and stout").into_response();
        let response = json_server_errors(teapot).await;
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    }

    #[tokio::test]
    async fn a_panic_is_a_json_500() {
        for panic in [
            Box::new("boom") as Box<dyn Any + Send>,
            Box::new(String::from("boom")),
        ] {
            let response = panic_response(panic);
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(&body[..], br#"{"detail":"Internal Server Error"}"#);
        }
    }
}
