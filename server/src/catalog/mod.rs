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
//! - [`detail`]: `GET /api/books/works/{work_id}` (`BookDetailOut`, the one-time enrichment of
//!   stubs) and its refresh.
//! - [`custom`] and [`editions`]: user-added works and editions.
//! - [`requests`]: edit and merge requests; [`admin`]: the admin queue, the direct merge and the
//!   direct admin edit.
//! - [`reports`]: reports on works and editions readers added, and what "remove" does to them.
//! - [`merge`]: `merge_works`, two works becoming one (conventions §7.6).
//! - [`covers`]: the cover search for catalog and imported books without a cover (#74).

pub mod admin;
pub mod cache;
pub mod covers;
pub mod custom;
pub mod detail;
pub mod dnb_sru;
pub mod editions;
pub mod hits;
pub mod isbn_lookup;
pub mod merge;
pub mod openlibrary;
pub mod reports;
pub mod requests;
pub mod search;
pub mod service;
pub mod shared;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use merge::{MergeReport, merge_works};
pub use service::CatalogService;

use crate::AppState;
use crate::error::{ApiResult, AppError, msg};
use crate::outbound::HostThrottle;
use crate::ratelimit::GatePass;
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
/// Refresh of a work that is not an Open Library work (user-added, DNB or legacy).
pub const NOT_FROM_OPEN_LIBRARY: &str = "This book was added by a reader, not Open Library.";
/// A user-added book whose title is blank after trimming.
pub const TITLE_REQUIRED: &str = "A title is required";
/// 409 when a user-added book or edition names an ISBN a source or the catalog knows.
pub const ISBN_IN_CATALOG: &str = "That ISBN is already in the catalog";
/// A merge (request) whose two works are one.
pub const MERGE_INTO_ITSELF: &str = "A book cannot be merged into itself";
/// Approving a `cover_replace` request whose picture was taken down meanwhile.
pub const PICTURE_REMOVED: &str = "This picture was removed";
/// Approving or rejecting a request that is no longer open (admin only).
pub const REQUEST_DECIDED: &str = "This request was already decided";
/// 409 on approving a cover suggestion or searching a book's cover when the book has a cover
/// meanwhile (#74). Mapped in apiErrors.ts.
pub const COVER_EXISTS: &str = "This book has a cover now";

/// Catalog routes under `/api/books`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(search::search))
        .routes(routes!(search::trending))
        .routes(routes!(search::subject))
        .routes(routes!(isbn_lookup::lookup_isbn))
        .routes(routes!(custom::create_custom_book))
        .routes(routes!(detail::get_work))
        .routes(routes!(detail::refresh_work))
        .routes(routes!(editions::list_editions, editions::add_edition))
        .routes(routes!(editions::get_edition))
        .routes(routes!(requests::request_work_edit))
        .routes(routes!(requests::request_edition_edit))
        .routes(routes!(requests::request_merge))
        .routes(routes!(reports::report_work))
        .routes(routes!(reports::report_edition))
}

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin::list_edit_requests))
        .routes(routes!(admin::approve_edit_request))
        .routes(routes!(admin::reject_edit_request))
        .routes(routes!(admin::merge_work))
        .routes(routes!(admin::edit_work))
        .routes(routes!(admin::edit_edition))
        .routes(routes!(covers::admin::list_cover_suggestions))
        .routes(routes!(covers::admin::approve_cover_suggestion))
        .routes(routes!(covers::admin::reject_cover_suggestion))
        .routes(routes!(covers::admin::list_missing_covers))
        .routes(routes!(covers::admin::search_cover))
}

/// One account's turn at a request that may call Open Library (search, trending, a subject, the
/// import or first view of a book), held until the request ends. Every user shares the `ol_api`
/// throttle (3 requests a second, at most 2 s of waiting on request paths), so one account
/// must not fill it (security review 2026-10-28 #5):
///
/// - `catalog_user`: 60 such requests per account a minute (429 `Slow down a little. Try again
///   later.`);
/// - `catalog_gate`: two at a time per account, further ones wait for a turn. One account then
///   holds at most a few throttle slots, and every other request still gets one within its wait.
pub async fn upstream_turn(state: &AppState, user_id: i64) -> ApiResult<GatePass> {
    let key = format!("u:{user_id}");
    state
        .limits
        .catalog_user
        .check_and_hit(&key)
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    Ok(state.limits.catalog_gate.enter(&key).await)
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
