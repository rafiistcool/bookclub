//! `GET /api/books/isbn/{isbn}` (endpoint-map §4): the full ISBN chain, persisted. Local
//! editions first, then by registration group: 978-3 → DNB SRU → Open Library, every other
//! group → Open Library only (conventions §7.4).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use super::hits::{prefetch_covers, shelf_entries};
use super::service::{Busy, NoAnswer, persist};
use super::shared::books::{BookRef, book_out};
use super::shared::isbn::{Isbn13, parse_isbn};
use super::shared::lookup::{LookupError, Resolved, local_isbn};
use super::{INVALID_ISBN, ISBN_NOT_FOUND, ISBN_UNAVAILABLE, busy};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::ApiPath;

/// Resolves `isbn` for CAT's routes: the local edition (unless `remote_only`), then the remote
/// route of conventions §7.4. No definite answer is 429 `busy` (with the cooldown of the source
/// whose throttle refused) or 502 `unavailable`.
///
/// When `state.lookup` is CAT's own service (production, and `TestApp::with_real_lookup`), it
/// runs in three phases, so no pool connection is held while the DNB and Open Library are asked
/// (each step can take a throttle wait and two 15 s attempts): the local step on a short-lived
/// connection, the remote route without one, then the hit is stored on a new connection. Any
/// other `CatalogLookup` (`FakeCatalog` in tests) is called through the trait.
pub(crate) async fn resolve_isbn(
    state: &AppState,
    isbn: &Isbn13,
    remote_only: bool,
    unavailable: &'static str,
) -> ApiResult<Option<Resolved>> {
    let own = std::ptr::addr_eq(Arc::as_ptr(&state.lookup), Arc::as_ptr(&state.catalog));
    if !own {
        let mut conn = state.db.acquire().await?;
        let result = if remote_only {
            state.lookup.resolve_isbn_remote(&mut conn, isbn).await
        } else {
            state.lookup.resolve_isbn(&mut conn, isbn).await
        };
        return result.map_err(|error| match error {
            // The trait does not say which source refused; Open Library is the one every
            // route asks.
            LookupError::Throttled => {
                no_answer(state, NoAnswer::Throttled(Busy::OpenLibrary), unavailable)
            }
            LookupError::Unavailable => no_answer(state, NoAnswer::Unavailable, unavailable),
            LookupError::Db(error) => error.into(),
        });
    }
    if !remote_only {
        let mut conn = state.db.acquire().await?;
        if let Some(found) = local_isbn(&mut conn, isbn).await? {
            return Ok(Some(found));
        }
    }
    let hit = match state.catalog.remote_isbn(isbn).await {
        Ok(Some(hit)) => hit,
        Ok(None) => return Ok(None),
        Err(error) => return Err(no_answer(state, error, unavailable)),
    };
    let mut conn = state.db.acquire().await?;
    persist(&mut conn, isbn, &hit)
        .await
        .map_err(|error| match error {
            LookupError::Db(error) => error.into(),
            other => AppError::internal(format!("storing an ISBN hit failed: {other}")),
        })
}

/// 429 with the hint of the throttle that refused, or 502 `unavailable`.
fn no_answer(state: &AppState, error: NoAnswer, unavailable: &'static str) -> AppError {
    match error {
        NoAnswer::Throttled(Busy::OpenLibrary) => busy(&[&state.outbound.ol_api]),
        NoAnswer::Throttled(Busy::DnbSru) => busy(&[&state.outbound.dnb_sru]),
        NoAnswer::Unavailable => AppError::bad_gateway(unavailable),
    }
}

/// The edition an ISBN resolved to, with its work.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IsbnHitOut {
    /// The normalised ISBN-13.
    pub isbn: String,
    pub work_id: i64,
    pub edition_id: i64,
    /// `"/works/OL…W"` for Open Library works, else null.
    pub ol_work_key: Option<String>,
    /// The edition's own title when it has one, else the work's.
    pub title: String,
    pub authors: String,
    pub year: Option<i32>,
    /// The edition's pages, else the work's.
    pub pages: Option<i32>,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    /// The caller has the work on their shelf.
    pub on_shelf: bool,
    pub shelf_id: Option<i64>,
}

/// Looks an ISBN up and stores what it found. ISBN-10 input becomes its ISBN-13 and checksums
/// must hold. German-language ISBNs (978-3) are asked at the DNB first, then at Open Library
/// (also while the DNB is busy); every other ISBN only at Open Library. A book that is already
/// in the catalog is answered without an outbound call.
#[utoipa::path(
    get,
    path = "/api/books/isbn/{isbn}",
    operation_id = "catalog_lookup_isbn",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("isbn" = String, Path, description = "An ISBN-10 or ISBN-13; spaces and hyphens are ignored")),
    responses(
        (status = 200, body = IsbnHitOut, description = "The edition and its work"),
        (status = 400, body = ErrorBody, description = "`That is not a valid ISBN-10 or ISBN-13`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No book found for that ISBN`"),
        (status = 429, body = ErrorBody, description = "Every source asked is busy"),
        (status = 502, body = ErrorBody, description = "`Could not look that ISBN up right now. Try again.`")
    )
)]
pub async fn lookup_isbn(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<IsbnHitOut>> {
    let isbn = parse_isbn(&raw).ok_or_else(|| AppError::bad_request(INVALID_ISBN))?;
    let resolved = resolve_isbn(&state, &isbn, false, ISBN_UNAVAILABLE)
        .await?
        .ok_or_else(|| AppError::not_found(ISBN_NOT_FOUND))?;
    let mut conn = state.db.acquire().await?;
    // CAT's lookups always name the edition; another `CatalogLookup` may only name the work.
    let edition_id = match resolved.edition_id {
        Some(id) => id,
        None => local_isbn(&mut conn, &isbn)
            .await?
            .and_then(|local| local.edition_id)
            .ok_or_else(|| AppError::not_found(ISBN_NOT_FOUND))?,
    };
    prefetch_covers(&mut conn, &state.media_service, &[resolved.work_id]).await?;
    let book = book_out(
        &mut conn,
        BookRef {
            work_id: resolved.work_id,
            edition_id: Some(edition_id),
        },
    )
    .await?
    .ok_or_else(|| AppError::not_found(ISBN_NOT_FOUND))?;
    let shelf_id = shelf_entries(&mut conn, user.id, &[resolved.work_id])
        .await?
        .remove(&resolved.work_id);
    Ok(Json(IsbnHitOut {
        isbn: isbn.as_str().to_owned(),
        work_id: book.id,
        edition_id,
        ol_work_key: book.ol_work_key,
        title: book.title,
        authors: book.authors,
        year: book.year,
        pages: book.pages,
        cover_url: book.cover_url,
        cover_thumb_url: book.cover_thumb_url,
        cover_public_id: book.cover_public_id,
        on_shelf: shelf_id.is_some(),
        shelf_id,
    }))
}
