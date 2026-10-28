//! `GET /api/books/works/{work_id}` and `POST /api/books/works/{work_id}/refresh`
//! (endpoint-map §4, docs/phase1/spec/catalog.md §1.5, §1.6, §2.19).
//!
//! A known work is answered from the catalog. Two one-time exceptions go outbound on a work's
//! first detail view: an Open Library stub (a search hit, `details_fetched_at` NULL) is enriched
//! with its full record, and a DNB work gets the one Open Library cover check by ISBN
//! (conventions §7.4). Either failing serves the stored row with 200. An unknown `OL…W` id is
//! imported. A refresh fetches the Open Library record again, bypassing the caches, and stores it
//! with `Mode::Refresh`, so what a reader entered or an admin approved stays.
//!
//! Readers, the rating mean and the quote count cover the viewer's audience only: they read
//! through `bc_visible_shelf` and `bc_visible_quotes` (conventions §6.2).

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;

use super::cache::Fetch;
use super::hits::{club_picks, prefetch_covers};
use super::service::attach_checked_cover;
use super::shared::books::{BookRef, book_out};
use super::shared::ingest::{self, Mode, WorkRecord};
use super::shared::isbn::Isbn13;
use super::shared::work_ref::{WorkRef, resolve_local};
use super::{BROWSE_UNAVAILABLE, NOT_FROM_OPEN_LIBRARY, busy, upstream_turn};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::{CatalogSource, ClubRef, ShelfStatus, Visibility, user_summaries};
use crate::error::{ApiResult, AppError, ErrorBody, is_deadlock, msg};
use crate::extract::ApiPath;
use crate::text::round_half_even;

/// Another reader of a book, from the viewer's audience.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BookReader {
    pub username: String,
    pub status: ShelfStatus,
    /// Null when the reader keeps their rating to themselves (`only_me`).
    pub rating: Option<i32>,
    pub progress: Option<i32>,
    pub avatar_url: Option<String>,
}

/// One work with the viewer's shelf state, their audience's readers and the catalog facts.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BookDetailOut {
    /// The work id; a merged-away work answers with the work it was merged into.
    pub id: i64,
    /// `"/works/OL…W"` for Open Library works, else null.
    pub ol_work_key: Option<String>,
    pub title: String,
    /// `""` when the work has none.
    pub subtitle: String,
    pub authors: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    /// `""` when unknown.
    pub description: String,
    pub subjects: Vec<String>,
    pub pages: Option<i32>,
    pub ol_rating: Option<f64>,
    pub ol_rating_count: Option<i32>,
    pub source: CatalogSource,
    /// `source == "user"`: added by a reader.
    pub custom: bool,
    pub editions_count: i64,
    /// The viewer's shelf status, or null when the work is not on their shelf.
    pub on_shelf: Option<ShelfStatus>,
    pub shelf_id: Option<i64>,
    /// The edition on the viewer's shelf entry.
    pub edition_id: Option<i64>,
    pub rating: Option<i32>,
    pub rating_visibility: Option<Visibility>,
    pub take: String,
    pub dnf_reason: String,
    pub progress: Option<i32>,
    /// Everyone else in the viewer's audience with this work on their shelf, by username.
    pub readers: Vec<BookReader>,
    /// The mean of the finished ratings the viewer may see (their own included), one decimal,
    /// ties to even.
    pub club_rating: Option<f64>,
    pub rating_count: i64,
    /// The quotes from this work the viewer may see: their own and their audience's.
    pub quote_count: i64,
    /// The viewer's favourite slot (1 to 3) for this work.
    pub favorite_position: Option<i32>,
    /// The viewer's clubs whose current pick this work is.
    pub club_picks: Vec<ClubRef>,
    /// An Open Library work: `POST …/refresh` fetches its record again.
    pub can_refresh: bool,
}

/// The [`BookDetailOut`] of the current work `work_id` for `viewer`. 404 `No such book.` when
/// the work does not exist.
pub async fn book_detail(
    conn: &mut PgConnection,
    viewer: i64,
    work_id: i64,
) -> ApiResult<BookDetailOut> {
    let book = book_out(conn, BookRef::work(work_id))
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    let work = sqlx::query!(
        r#"SELECT w.subtitle, w.description, w.subjects, w.ol_rating, w.ol_rating_count,
                  (SELECT count(*) FROM editions e WHERE e.work_id = w.id) AS "editions_count!"
           FROM works w WHERE w.id = $1"#,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    // The viewer's own entry: their data, no policy needed.
    let own = sqlx::query!(
        r#"SELECT id, status AS "status: ShelfStatus", edition_id, rating,
                  rating_visibility AS "rating_visibility: Visibility", take, dnf_reason, progress
           FROM shelf_entries WHERE user_id = $1 AND work_id = $2"#,
        viewer,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let reader_rows = sqlx::query!(
        r#"SELECT s.user_id AS "user_id!", s.status AS "status!: ShelfStatus", s.rating,
                  s.progress
           FROM bc_visible_shelf($1) AS s
           WHERE s.work_id = $2 AND NOT s.is_self"#,
        viewer,
        work_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<i64> = reader_rows.iter().map(|row| row.user_id).collect();
    let mut names = user_summaries(conn, &ids).await?;
    let mut readers: Vec<BookReader> = reader_rows
        .into_iter()
        .filter_map(|row| {
            let user = names.remove(&row.user_id)?;
            Some(BookReader {
                username: user.username,
                status: row.status,
                rating: row.rating,
                progress: row.progress,
                avatar_url: user.avatar_url,
            })
        })
        .collect();
    // Byte order is code-point order; usernames are [a-z0-9_].
    readers.sort_by(|a, b| a.username.cmp(&b.username));
    let ratings = sqlx::query!(
        r#"SELECT count(*) AS "count!", sum(s.rating)::bigint AS total
           FROM bc_visible_shelf($1) AS s
           WHERE s.work_id = $2 AND s.status = 'finished' AND s.rating IS NOT NULL"#,
        viewer,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let club_rating = match (ratings.count, ratings.total) {
        (count, Some(total)) if count > 0 => Some(round_half_even(total as f64 / count as f64, 1)),
        _ => None,
    };
    let quote_count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM bc_visible_quotes($1) AS q WHERE q.work_id = $2"#,
        viewer,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let favorite_position = sqlx::query_scalar!(
        "SELECT position FROM user_favorites WHERE user_id = $1 AND work_id = $2",
        viewer,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let picks = club_picks(conn, viewer, &[work_id])
        .await?
        .remove(&work_id)
        .unwrap_or_default();
    let can_refresh = book.source == CatalogSource::Openlibrary && book.ol_work_key.is_some();
    Ok(BookDetailOut {
        id: book.id,
        ol_work_key: book.ol_work_key,
        title: book.title,
        subtitle: work.subtitle,
        authors: book.authors,
        year: book.year,
        cover_url: book.cover_url,
        cover_thumb_url: book.cover_thumb_url,
        cover_public_id: book.cover_public_id,
        description: work.description,
        subjects: work.subjects,
        pages: book.pages,
        ol_rating: work.ol_rating,
        ol_rating_count: work.ol_rating_count,
        source: book.source,
        custom: book.custom,
        editions_count: work.editions_count,
        on_shelf: own.as_ref().map(|entry| entry.status),
        shelf_id: own.as_ref().map(|entry| entry.id),
        edition_id: own.as_ref().and_then(|entry| entry.edition_id),
        rating: own.as_ref().and_then(|entry| entry.rating),
        rating_visibility: own.as_ref().map(|entry| entry.rating_visibility),
        take: own.as_ref().map(|e| e.take.clone()).unwrap_or_default(),
        dnf_reason: own
            .as_ref()
            .map(|e| e.dnf_reason.clone())
            .unwrap_or_default(),
        progress: own.as_ref().and_then(|entry| entry.progress),
        readers,
        club_rating,
        rating_count: ratings.count,
        quote_count,
        favorite_position,
        club_picks: picks,
        can_refresh,
    })
}

/// A `{work_id}` that parses; 404 `No such book.` otherwise (no outbound call).
pub fn parse_work_ref(raw: &str) -> ApiResult<WorkRef> {
    WorkRef::parse(raw).ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))
}

/// The current local work a `{work_id}` names; 404 `No such book.` when there is none. Writes
/// (editions, requests) need a known work: nothing is imported for them.
pub async fn require_local_work(conn: &mut PgConnection, raw: &str) -> ApiResult<i64> {
    let work_ref = parse_work_ref(raw)?;
    resolve_local(conn, &work_ref)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))
}

/// What the first detail view needs to know about a stored work.
struct StoredWork {
    id: i64,
    source: CatalogSource,
    ol_work_id: Option<String>,
    details_fetched: bool,
}

async fn stored_work(state: &AppState, work_ref: &WorkRef) -> sqlx::Result<Option<StoredWork>> {
    let mut conn = state.db.acquire().await?;
    let Some(id) = resolve_local(&mut conn, work_ref).await? else {
        return Ok(None);
    };
    let row = sqlx::query!(
        r#"SELECT source AS "source: CatalogSource", ol_work_id,
                  details_fetched_at IS NOT NULL AS "details_fetched!"
           FROM works WHERE id = $1"#,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|row| StoredWork {
        id,
        source: row.source,
        ol_work_id: row.ol_work_id,
        details_fetched: row.details_fetched,
    }))
}

/// Stores a full Open Library record (`Mode::Refresh`: fields entered by a reader or approved
/// by an admin stay, values the record lacks are kept) in its own transaction, retried once
/// after a deadlock. Returns the current work (the survivor when the record's work was merged
/// away).
async fn store_record(conn: &mut PgConnection, record: &WorkRecord) -> sqlx::Result<i64> {
    let mut attempt = 0;
    loop {
        let mut tx = conn.begin().await?;
        match ingest::upsert_work(&mut tx, record, Mode::Refresh).await {
            Ok(id) => {
                tx.commit().await?;
                return Ok(id);
            }
            Err(error) if is_deadlock(&error) && attempt == 0 => {
                tracing::warn!("storing a work record deadlocked; retrying once");
                tx.rollback().await?;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// The answer to a failed work fetch: 404 `No such book.` for a work Open Library does not
/// have, 429 while it is busy, else 502 (an upstream 422 included, decision 11).
fn work_error(state: &AppState, fetch: &Fetch) -> AppError {
    match fetch {
        Fetch::Missing => AppError::not_found(msg::NO_SUCH_BOOK),
        Fetch::Throttled => busy(&[&state.outbound.ol_api]),
        Fetch::Unprocessable | Fetch::Failed(_) => AppError::bad_gateway(BROWSE_UNAVAILABLE),
    }
}

/// Fetches and stores the full record of Open Library work `ol_work_id`; a refresh bypasses the
/// caches. Its cover is queued for prefetching. Returns the current work id.
async fn import(state: &AppState, ol_work_id: &str, refresh: bool) -> ApiResult<i64> {
    let record = state
        .catalog
        .ol_work_record(ol_work_id, refresh)
        .await
        .map_err(|fetch| work_error(state, &fetch))?;
    let mut conn = state.db.acquire().await?;
    let work_id = store_record(&mut conn, &record).await?;
    prefetch_covers(&mut conn, &state.media_service, &[work_id]).await?;
    Ok(work_id)
}

/// The one-time work of a stored work's first detail view: an Open Library stub gets its full
/// record, a DNB work its cover check. A failing upstream leaves the row as it is (it is served
/// with 200, and the next view tries again).
async fn first_view(state: &AppState, work: &StoredWork) -> ApiResult<()> {
    if work.details_fetched {
        return Ok(());
    }
    match (work.source, &work.ol_work_id) {
        (CatalogSource::Openlibrary, Some(ol_work_id)) => {
            match state.catalog.ol_work_record(ol_work_id, false).await {
                Ok(record) => {
                    let mut conn = state.db.acquire().await?;
                    let id = store_record(&mut conn, &record).await?;
                    prefetch_covers(&mut conn, &state.media_service, &[id]).await?;
                }
                Err(fetch) => {
                    tracing::warn!(%ol_work_id, ?fetch, "enriching a stub failed; serving the stub");
                }
            }
        }
        (CatalogSource::Dnb, _) => dnb_cover_check(state, work.id).await?,
        _ => {}
    }
    Ok(())
}

/// The one Open Library cover check by ISBN for a DNB work whose check never ran (harvested
/// works): `/isbn/{isbn}.json` for its first edition with an ISBN. Without an answer the check
/// stays open for the next view.
async fn dnb_cover_check(state: &AppState, work_id: i64) -> ApiResult<()> {
    let edition = {
        let mut conn = state.db.acquire().await?;
        sqlx::query!(
            r#"SELECT id, isbn13 AS "isbn13!" FROM editions
               WHERE work_id = $1 AND isbn13 IS NOT NULL ORDER BY id LIMIT 1"#,
            work_id
        )
        .fetch_optional(&mut *conn)
        .await?
    };
    let Some(edition) = edition else {
        return Ok(());
    };
    let isbn = Isbn13(edition.isbn13);
    let cover = match state.catalog.ol_isbn_cover(&isbn).await {
        Ok(cover) => cover,
        Err(fetch) => {
            tracing::warn!(%isbn, ?fetch, "the Open Library cover check failed; it stays open");
            return Ok(());
        }
    };
    let mut conn = state.db.acquire().await?;
    let mut tx = conn.begin().await?;
    // The media row is locked before any catalog row, like ingest does.
    let media = match cover {
        Some(cover_id) => ingest::ol_cover_media(&mut tx, cover_id).await?,
        None => None,
    };
    attach_checked_cover(&mut tx, work_id, edition.id, media).await?;
    tx.commit().await?;
    prefetch_covers(&mut conn, &state.media_service, &[work_id]).await?;
    Ok(())
}

/// One book, with the viewer's shelf state and their audience's readers. `{work_id}` is a
/// numeric id, an Open Library id (`OL…W`, imported on its first view) or a legacy key (`BC…`,
/// `ISBN…`, `GB…`). A merged work answers with the work it was merged into. A known book is
/// served from the catalog; only an Open Library stub's first view fetches its full record, and
/// a DNB book's first view asks Open Library once for a cover.
#[utoipa::path(
    get,
    path = "/api/books/works/{work_id}",
    operation_id = "catalog_get_work",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    responses(
        (status = 200, body = BookDetailOut, description = "The book"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "Open Library is busy (importing an unknown `OL…W` id), or `Slow down a little. Try again later.` (60 catalog requests per account a minute)"),
        (status = 502, body = ErrorBody, description = "`Could not load the library right now. Try again.`")
    )
)]
pub async fn get_work(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<BookDetailOut>> {
    let work_ref = parse_work_ref(&raw)?;
    let work_id = match stored_work(&state, &work_ref).await? {
        Some(work) => {
            // A first view asks Open Library, under the account's catalog limits
            // (`upstream_turn`); over them, the stub is served as it is and enriched later.
            if !work.details_fetched {
                match upstream_turn(&state, user.id).await {
                    Ok(_turn) => first_view(&state, &work).await?,
                    Err(_) => tracing::debug!(work_id = work.id, "first view deferred"),
                }
            }
            work.id
        }
        None => match &work_ref {
            WorkRef::OpenLibrary(ol_work_id) => {
                let _turn = upstream_turn(&state, user.id).await?;
                import(&state, ol_work_id, false).await?
            }
            _ => return Err(AppError::not_found(msg::NO_SUCH_BOOK)),
        },
    };
    let mut conn = state.db.acquire().await?;
    Ok(Json(book_detail(&mut conn, user.id, work_id).await?))
}

/// Fetches an Open Library book's record again (bypassing the caches) and stores it. Fields a
/// reader entered or an admin approved, covers included, are never overwritten, and fields
/// Open Library no longer has keep their value. Books added by a reader, from the DNB or from
/// the legacy import cannot be refreshed. 10 per hour per user.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/refresh",
    operation_id = "catalog_refresh_work",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    responses(
        (status = 200, body = BookDetailOut, description = "The refreshed book"),
        (status = 400, body = ErrorBody, description = "`This book was added by a reader, not Open Library.`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (10 per hour), or Open Library is busy"),
        (status = 502, body = ErrorBody, description = "`Could not load the library right now. Try again.`")
    )
)]
pub async fn refresh_work(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<BookDetailOut>> {
    let work_ref = parse_work_ref(&raw)?;
    let ol_work_id = match stored_work(&state, &work_ref).await? {
        Some(StoredWork {
            source: CatalogSource::Openlibrary,
            ol_work_id: Some(ol_work_id),
            ..
        }) => ol_work_id,
        Some(_) => return Err(AppError::bad_request(NOT_FROM_OPEN_LIBRARY)),
        None => match work_ref {
            WorkRef::OpenLibrary(ol_work_id) => ol_work_id,
            _ => return Err(AppError::not_found(msg::NO_SUCH_BOOK)),
        },
    };
    state
        .limits
        .refresh_user
        .check_and_hit(&format!("u:{}", user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let work_id = import(&state, &ol_work_id, true).await?;
    let mut conn = state.db.acquire().await?;
    Ok(Json(book_detail(&mut conn, user.id, work_id).await?))
}
