//! The catalog (#41, owner CAT): search, works, editions, ISBN lookups, user-added books, edit
//! requests and merges. `shared/` holds the frozen interfaces every package uses.
//!
//! - [`service`]: `CatalogService` and its `CatalogLookup` implementation (ISBN routing by
//!   registration group, conventions §7.4).
//! - [`openlibrary`] and [`dnb_sru`]: the upstream clients, both through `outbound` and the
//!   response caches of [`cache`].
//! - [`search`]: `GET /api/books/search`, `/trending` and `/subjects/{subject}`.
//! - [`isbn_lookup`]: `GET /api/books/isbn/{isbn}`.
//! - [`hits`]: persisting remote hits and building annotated `SearchHit`s.

pub mod cache;
pub mod dnb_sru;
pub mod hits;
pub mod isbn_lookup;
pub mod openlibrary;
pub mod search;
pub mod service;
pub mod shared;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use service::CatalogService;

use crate::AppState;
use crate::error::AppError;
use crate::outbound::HostThrottle;
use cache::Fetch;

// Detail strings of the catalog (docs/phase1/spec/catalog.md §1.11, endpoint-map §4), byte for
// byte. None of them is mapped in apiErrors.ts: Discover branches on the status.
pub const SEARCH_TOO_LONG: &str = "Search is too long";
pub const QUERY_TOO_SHORT: &str = "Need at least 3 characters. Add the author, or paste an ISBN.";
pub const SEARCH_UNAVAILABLE: &str = "Could not search the library right now. Try again.";
pub const BROWSE_UNAVAILABLE: &str = "Could not load the library right now. Try again.";
pub const RATE_LIMITED: &str = "The library is busy. Wait a few seconds and try again.";
pub const NO_SUCH_SUBJECT: &str = "No such subject.";
pub const INVALID_SUBJECT: &str = "That is not a valid subject.";
pub const INVALID_ISBN: &str = "That is not a valid ISBN-10 or ISBN-13";
pub const ISBN_NOT_FOUND: &str = "No book found for that ISBN";
pub const ISBN_UNAVAILABLE: &str = "Could not look that ISBN up right now. Try again.";
/// The pydantic enum message for a bad `sort` (also `sort=`).
pub const INVALID_SORT: &str = "Input should be 'readinglog', 'new', 'title' or 'relevance'";

/// Catalog routes under `/api/books`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(search::search))
        .routes(routes!(search::trending))
        .routes(routes!(search::subject))
        .routes(routes!(isbn_lookup::lookup_isbn))
}

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

/// 429 for a busy upstream: `The library is busy. Try again in {n} seconds.` (and
/// `Retry-After: n`) while one of `throttles` cools down after an upstream 429, the longest
/// wait winning; else `The library is busy. Wait a few seconds and try again.` (the local
/// throttle stayed full for its whole wait).
pub fn busy(throttles: &[&HostThrottle]) -> AppError {
    let wait = throttles
        .iter()
        .filter_map(|throttle| throttle.cooldown_remaining())
        .max()
        .map(|left| left.as_secs() + u64::from(left.subsec_nanos() > 0))
        .filter(|&seconds| seconds > 0);
    match wait {
        Some(seconds) => AppError::too_many(
            format!("The library is busy. Try again in {seconds} seconds."),
            Some(seconds),
        ),
        None => AppError::too_many(RATE_LIMITED, None),
    }
}

/// The answer to a failed Open Library call on a search or browse path (spec/catalog.md §2.16):
/// throttled → 429 ([`busy`]); 422 → 400 [`QUERY_TOO_SHORT`]; 404 → 404 `missing` when given;
/// anything else → 502 `unavailable`.
pub fn browse_error(
    fetch: &Fetch,
    throttle: &HostThrottle,
    unavailable: &'static str,
    missing: Option<&'static str>,
) -> AppError {
    match (fetch, missing) {
        (Fetch::Throttled, _) => busy(&[throttle]),
        (Fetch::Unprocessable, _) => AppError::bad_request(QUERY_TOO_SHORT),
        (Fetch::Missing, Some(missing)) => AppError::not_found(missing),
        _ => AppError::bad_gateway(unavailable),
    }
}
