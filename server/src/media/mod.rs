//! Media (#43, owner MED): every picture the app shows, served from our own origin
//! (docs/phase1/conventions.md §8, endpoint-map §6).
//!
//! - [`pipeline`]: decode, orient, crop or scale, and re-encode as WebP without metadata.
//! - [`color`]: the spine colour of a cover (the legacy median cut).
//! - [`service`]: `MediaService` (the pipeline's concurrency limit, the coalesced Open Library
//!   cover fetch, the prefetch queue) and the prefetch worker `run_prefetch`.
//! - [`serve`]: `GET /media/{public_id}/{variant}`.
//! - [`avatar`]: `PUT`/`DELETE /api/auth/me/avatar`.
//! - [`cover`]: `POST /api/books/works/{work_id}/cover`.
//! - [`spine`]: `POST /api/shelf/spine-colors`.
//! - [`reports`]: `POST /api/media/{public_id}/reports` and the admin queue and takedown.
//! - [`shared`]: the frozen store, keys and media rows every package uses (W1).

pub mod avatar;
pub mod color;
pub mod cover;
pub mod pipeline;
pub mod reports;
pub mod serve;
pub mod service;
pub mod shared;
pub mod spine;
pub mod upload;

use axum::extract::DefaultBodyLimit;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use service::MediaService;

use crate::AppState;
use shared::MediaStore;

/// 400 for an upload of a taken-down picture, 410 for its URL.
pub const PICTURE_REMOVED: &str = "This picture was removed";
/// 429 while Open Library's cover host is throttled (the catalog's busy string).
pub const COVER_BUSY: &str = "The library is busy. Wait a few seconds and try again.";
/// 502 when Open Library did not answer for a cover.
pub const COVER_UNAVAILABLE: &str = "Could not load the cover right now. Try again.";

/// Media routes outside `/api/admin`. The upload routes raise the body limit to 50 MiB plus
/// 64 KiB (conventions §4.6).
pub fn routes() -> OpenApiRouter<AppState> {
    let uploads = OpenApiRouter::new()
        .routes(routes!(avatar::put_avatar, avatar::delete_avatar))
        .routes(routes!(cover::upload_cover))
        .layer(DefaultBodyLimit::max(upload::UPLOAD_BODY_LIMIT));
    OpenApiRouter::new()
        .merge(uploads)
        .routes(routes!(serve::get_media))
        .routes(routes!(spine::spine_colors))
        .routes(routes!(reports::report))
}

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(reports::list_reports))
        .routes(routes!(reports::resolve_report))
        .routes(routes!(reports::take_down))
}

/// Deletes the bytes under each prefix after a commit; a failure is logged, never an error
/// (the row is gone or removed already, so nothing serves them).
pub async fn delete_prefixes(store: &dyn MediaStore, prefixes: &[String]) {
    let mut done: Vec<&str> = Vec::new();
    for prefix in prefixes {
        if done.contains(&prefix.as_str()) {
            continue;
        }
        done.push(prefix);
        if let Err(error) = store.delete_prefix(prefix).await {
            tracing::warn!(prefix, error = %error, "cannot delete media bytes");
        }
    }
}
