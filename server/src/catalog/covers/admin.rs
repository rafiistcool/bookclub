//! The admin side of the cover search (#74, docs/phase2/notes/cover.md §5.1): the suggestions
//! the search did not attach on its own, the books still without a cover, and "Search now".
//! Every route sits under `/api/admin/`, behind the `InstanceAdmin` route layer.
//!
//! Lock order (conventions §7.2): an approval reads the suggestion without a lock (its picture
//! and work never change), then takes the picture `FOR SHARE`, the work `FOR UPDATE`, the
//! suggestion `FOR UPDATE`; a rejection takes the work, the suggestion, then `cover_search`.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};

use super::dead;
use super::search::{self, Context, Outcome, Pacer, search_one};
use crate::AppState;
use crate::admin::queue::{MAX_PAGE, PAGE_SIZE, QueueQuery};
use crate::auth::extract::InstanceAdmin;
use crate::catalog::detail::require_local_work;
use crate::catalog::shared::books::{
    BookRef, book_out, book_outs, follow_merges, follow_merges_many,
};
use crate::catalog::shared::ingest::{CoverTarget, Source, set_cover};
use crate::catalog::{COVER_EXISTS, PICTURE_REMOVED, REQUEST_DECIDED, SEARCH_UNAVAILABLE};
use crate::catalog::{busy, upstream_turn};
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiPath, ApiQuery, Validate, Validator};
use crate::media::shared::urls::cover_urls;
use crate::outbound::RetryPolicy;

/// The pydantic enum message for an unknown `scope`.
pub const SCOPE_CHOICES: &str = "Input should be 'in_use' or 'all'";

/// A cover the search found for a book but did not attach on its own.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverSuggestionOut {
    pub id: i64,
    /// `open`, `approved`, `rejected` or `closed` (the server closed it: the book got a cover,
    /// was merged away, or the picture or the Open Library work is out).
    pub status: String,
    /// The book the suggestion was made for (it may have been merged since).
    pub work_id: i64,
    /// The book now (merges followed); its cover is null while it has none.
    pub book: Option<BookOut>,
    /// The candidate picture, from our `/media`; null once it was taken down or is gone.
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    /// The Open Library work (`OL…W`) and edition (`OL…M`) the picture belongs to.
    pub ol_work_id: String,
    pub ol_edition_id: Option<String>,
    /// Open Library's title (and subtitle) of the candidate.
    pub title: String,
    /// Open Library's authors, `", "`-joined.
    pub authors: String,
    pub year: Option<i32>,
    /// The edition's language (MARC code), when the picture is an edition's.
    pub language: Option<String>,
    /// Why it was not attached on its own: `single_record` (the only title-equal Open Library
    /// work, but a record with one edition), `title_differs`, `number_differs`, `several_works`,
    /// `generic_title`, `no_author` or `after_takedown`.
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
}

/// A page of cover suggestions: the open ones oldest first, or the decided ones newest
/// decision first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverSuggestionList {
    pub items: Vec<CoverSuggestionOut>,
    /// The page these items are (1-based).
    pub page: i64,
    /// Another page follows.
    pub has_more: bool,
}

/// A catalog or imported book without a usable cover.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MissingCoverOut {
    pub book: BookOut,
    /// Readers use it (a shelf entry, favourite, quote, pick, nomination, post or legacy key).
    pub in_use: bool,
    /// It came from the legacy import.
    pub imported: bool,
    /// `none` (no cover), `failing` (Open Library's picture of the book, or of an edition
    /// readers use, failed to load) or `photo_waiting` (a reader's photo waits in the requests
    /// tab).
    pub state: String,
    /// `failing`: the edition whose picture failed, when it is not the book's own.
    pub failing_edition_id: Option<i64>,
    /// `failing`: when the picture last failed to load.
    #[serde(with = "time::serde::rfc3339::option")]
    pub failed_at: Option<OffsetDateTime>,
    /// When the search last had an answer for it; null = never searched.
    #[serde(with = "time::serde::rfc3339::option")]
    pub searched_at: Option<OffsetDateTime>,
    /// The last outcome: `attached`, `inherited`, `isbn`, `suggested`, `uncertain`, `no_match`
    /// or `blocked`.
    pub outcome: Option<String>,
    /// Not searched again before this; null = due.
    #[serde(with = "time::serde::rfc3339::option")]
    pub next_search_at: Option<OffsetDateTime>,
    /// Attempts in a row Open Library did not answer.
    pub failures: i32,
    /// A takedown cleared its cover: the search only suggests from now on.
    pub after_takedown: bool,
    /// The open suggestion for it, if any.
    pub suggestion_id: Option<i64>,
}

/// A page of books without a cover: books readers use first, then by title.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MissingCoverList {
    pub items: Vec<MissingCoverOut>,
    pub page: i64,
    pub has_more: bool,
}

/// What "Search now" came to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverSearchOut {
    /// `attached`, `inherited`, `isbn`, `suggested`, `no_match`, or `failing`: the book's own
    /// Open Library picture, or that of an edition readers use, was fetched again and still
    /// cannot be loaded. The search never replaces a picture the book has; an uploaded picture
    /// does.
    pub outcome: String,
    /// The book as it is now.
    pub book: BookOut,
    /// `suggested`: the suggestion to decide.
    pub suggestion: Option<CoverSuggestionOut>,
    /// `failing`: when the picture failed to load this time.
    #[serde(with = "time::serde::rfc3339::option")]
    pub failed_at: Option<OffsetDateTime>,
    /// `failing`: the edition whose picture fails, when it is not the book's own.
    pub failing_edition_id: Option<i64>,
}

/// A failing Open Library picture of a work: its own, else the one of the lowest-id edition
/// readers use (`cover_failing_editions`), as (when it last failed, the edition).
async fn failing_picture(
    conn: &mut PgConnection,
    work_id: i64,
) -> sqlx::Result<Option<(OffsetDateTime, Option<i64>)>> {
    let row = sqlx::query!(
        r#"SELECT (SELECT f.failed_at FROM cover_failing f WHERE f.media_id = w.cover_media_id)
                      AS "work_failed_at?",
                  fe.edition_id AS "edition_id?", fe.failed_at AS "edition_failed_at?"
           FROM works w
           LEFT JOIN LATERAL (
               SELECT f.edition_id, f.failed_at FROM cover_failing_editions f
               WHERE f.work_id = w.id ORDER BY f.edition_id LIMIT 1
           ) fe ON true
           WHERE w.id = $1"#,
        work_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(
        row.and_then(|row| match (row.work_failed_at, row.edition_failed_at) {
            (Some(at), _) => Some((at, None)),
            (None, Some(at)) => Some((at, row.edition_id)),
            (None, None) => None,
        }),
    )
}

/// `?scope=&page=` of the books without a cover.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MissingCoverQuery {
    /// `in_use` (the default): books readers use. `all`: every catalog and imported book.
    #[serde(default = "default_scope")]
    #[param(value_type = Option<String>)]
    pub scope: String,
    /// 1-based, 50 items a page; default 1.
    #[serde(default = "first_page")]
    #[param(value_type = Option<i64>)]
    pub page: i64,
}

fn default_scope() -> String {
    "in_use".to_owned()
}

fn first_page() -> i64 {
    1
}

impl Validate for MissingCoverQuery {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            matches!(self.scope.as_str(), "in_use" | "all"),
            SCOPE_CHOICES,
        );
        v.range_i64(Some(self.page), 1, MAX_PAGE);
    }
}

/// The [`CoverSuggestionOut`]s of `ids`, in that order (missing ids are left out).
pub async fn suggestion_outs(
    conn: &mut PgConnection,
    ids: &[i64],
) -> sqlx::Result<Vec<CoverSuggestionOut>> {
    let rows = sqlx::query!(
        r#"SELECT s.id, s.status, s.work_id, s.ol_work_id, s.ol_edition_id, s.title, s.authors,
                  s.year, s.language, s.reason, s.created_at, s.decided_at,
                  m.public_id AS "cover?"
           FROM cover_suggestions s
           LEFT JOIN media m ON m.ol_cover_id = s.ol_cover_id AND m.removed_at IS NULL
                            AND m.ol_gone_at IS NULL
           WHERE s.id = ANY($1)"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let works: Vec<i64> = rows.iter().map(|row| row.work_id).collect();
    let current = follow_merges_many(conn, &works).await?;
    let refs: Vec<BookRef> = current.values().map(|&id| BookRef::work(id)).collect();
    let books = book_outs(conn, &refs).await?;
    let mut by_id: HashMap<i64, CoverSuggestionOut> = rows
        .into_iter()
        .map(|row| {
            let book = current
                .get(&row.work_id)
                .and_then(|&id| books.get(&BookRef::work(id)).cloned());
            let (cover_url, cover_thumb_url) = cover_urls(row.cover.as_deref());
            let out = CoverSuggestionOut {
                id: row.id,
                status: row.status,
                work_id: row.work_id,
                book,
                cover_url,
                cover_thumb_url,
                cover_public_id: row.cover,
                ol_work_id: row.ol_work_id,
                ol_edition_id: row.ol_edition_id,
                title: row.title,
                authors: row.authors,
                year: row.year,
                language: row.language,
                reason: row.reason,
                created_at: row.created_at,
                decided_at: row.decided_at,
            };
            (row.id, out)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

async fn suggestion_out(conn: &mut PgConnection, id: i64) -> ApiResult<CoverSuggestionOut> {
    suggestion_outs(conn, &[id])
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))
}

/// A page of cover suggestions: `?status=open` (the default) lists the open ones oldest first,
/// hiding those nobody needs to decide any more (the book has a cover now, was merged away, or
/// the picture is gone); `?status=resolved` the approved, rejected and closed ones, the newest
/// decision first. 50 a page.
#[utoipa::path(
    get,
    path = "/api/admin/cover-suggestions",
    operation_id = "catalog_admin_list_cover_suggestions",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(QueueQuery),
    responses(
        (status = 200, body = CoverSuggestionList, description = "A page of the open or the decided suggestions"),
        (status = 400, body = ErrorBody, description = "`Input should be 'open' or 'resolved'`, `Input should be greater than or equal to 1`, `Input should be less than or equal to 1000`, `Input should be a valid integer, unable to parse string as an integer`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (not an instance admin)")
    )
)]
pub async fn list_cover_suggestions(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<QueueQuery>,
) -> ApiResult<Json<CoverSuggestionList>> {
    let mut conn = state.db.acquire().await?;
    let ids = if query.resolved() {
        sqlx::query_scalar!(
            "SELECT id FROM cover_suggestions WHERE status <> 'open'
             ORDER BY decided_at DESC NULLS LAST, id DESC
             LIMIT $1 OFFSET $2",
            query.limit(),
            query.offset()
        )
        .fetch_all(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar!(
            "SELECT id FROM cover_suggestions
             WHERE status = 'open' AND NOT bc_cover_suggestion_moot(id)
             ORDER BY created_at, id
             LIMIT $1 OFFSET $2",
            query.limit(),
            query.offset()
        )
        .fetch_all(&mut *conn)
        .await?
    };
    let (ids, has_more) = query.split(ids);
    let items = suggestion_outs(&mut conn, &ids).await?;
    Ok(Json(CoverSuggestionList {
        items,
        page: query.page,
        has_more,
    }))
}

/// The suggestion's work, picture and status, read without a lock: 404 `Not found` when there
/// is no such suggestion.
struct SuggestionRow {
    work_id: i64,
    ol_work_id: String,
    ol_cover_id: i64,
    status: String,
}

async fn read_suggestion(conn: &mut PgConnection, id: i64) -> ApiResult<SuggestionRow> {
    sqlx::query_as!(
        SuggestionRow,
        "SELECT work_id, ol_work_id, ol_cover_id, status FROM cover_suggestions WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))
}

/// Locks the current work (following merges) `FOR UPDATE`; `None` when it is gone.
async fn lock_current_work(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<i64>> {
    let mut work_id = work_id;
    for _ in 0..3 {
        let Some(current) = follow_merges(conn, work_id).await? else {
            return Ok(None);
        };
        let locked = sqlx::query_scalar!(
            "SELECT id FROM works WHERE id = $1 AND merged_into_id IS NULL FOR UPDATE",
            current
        )
        .fetch_optional(&mut *conn)
        .await?;
        if locked.is_some() {
            return Ok(Some(current));
        }
        work_id = current;
    }
    Ok(None)
}

/// Locks the suggestion `FOR UPDATE`; 409 when it is no longer open.
async fn lock_open_suggestion(conn: &mut PgConnection, id: i64) -> ApiResult<()> {
    let status = sqlx::query_scalar!(
        "SELECT status FROM cover_suggestions WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if status != "open" {
        return Err(AppError::conflict(REQUEST_DECIDED));
    }
    Ok(())
}

/// Uses the suggested picture: it becomes the cover of the book (merges followed) with
/// provenance `admin`, while the book still has none. That Open Library work is never
/// suggested again for the book.
#[utoipa::path(
    post,
    path = "/api/admin/cover-suggestions/{suggestion_id}/approve",
    operation_id = "catalog_admin_approve_cover_suggestion",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("suggestion_id" = i64, Path, description = "The suggestion id")),
    responses(
        (status = 200, body = CoverSuggestionOut, description = "The approved suggestion, with the book's new cover"),
        (status = 400, body = ErrorBody, description = "`This picture was removed` (taken down, or Open Library no longer has it)"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (no such suggestion, or not an instance admin), or `No such book.`"),
        (status = 409, body = ErrorBody, description = "`This request was already decided` or `This book has a cover now`")
    )
)]
pub async fn approve_cover_suggestion(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(suggestion_id): ApiPath<i64>,
) -> ApiResult<Json<CoverSuggestionOut>> {
    let mut conn = state.db.acquire().await?;
    let row = read_suggestion(&mut conn, suggestion_id).await?;
    if row.status != "open" {
        return Err(AppError::conflict(REQUEST_DECIDED));
    }
    let mut tx = conn.begin().await?;
    let media = sqlx::query!(
        r#"SELECT id, removed_at IS NOT NULL OR ol_gone_at IS NOT NULL AS "unusable!"
           FROM media WHERE ol_cover_id = $1 FOR SHARE"#,
        row.ol_cover_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let media_id = match media {
        Some(media) if !media.unusable => media.id,
        _ => return Err(AppError::bad_request(PICTURE_REMOVED)),
    };
    let work_id = lock_current_work(&mut tx, row.work_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    lock_open_suggestion(&mut tx, suggestion_id).await?;
    let missing = sqlx::query_scalar!(
        r#"SELECT bc_cover_missing(cover_media_id) AS "missing!" FROM works WHERE id = $1"#,
        work_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if !missing
        || !set_cover(
            &mut tx,
            CoverTarget::Work(work_id),
            media_id,
            Source::Admin,
            false,
        )
        .await?
    {
        return Err(AppError::conflict(COVER_EXISTS));
    }
    sqlx::query!(
        "UPDATE cover_suggestions SET status = 'approved', decided_at = now() WHERE id = $1",
        suggestion_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO cover_search AS cs (work_id, outcome, attached_ol_work_id,
                                         attached_ol_cover_id)
         VALUES ($1, 'attached', $2, $3)
         ON CONFLICT (work_id) DO UPDATE SET
             outcome = 'attached', misses = 0, failures = 0, next_search_at = NULL,
             last_error = NULL, attached_ol_work_id = $2, attached_ol_cover_id = $3",
        work_id,
        row.ol_work_id,
        row.ol_cover_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.media_service.prefetch(&[media_id]);
    Ok(Json(suggestion_out(&mut conn, suggestion_id).await?))
}

/// "Not this book": that Open Library work is never suggested again for the book (or a book
/// merged into it), and the book is searched again at once for the next candidate.
#[utoipa::path(
    post,
    path = "/api/admin/cover-suggestions/{suggestion_id}/reject",
    operation_id = "catalog_admin_reject_cover_suggestion",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("suggestion_id" = i64, Path, description = "The suggestion id")),
    responses(
        (status = 200, body = CoverSuggestionOut, description = "The rejected suggestion"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (no such suggestion, or not an instance admin)"),
        (status = 409, body = ErrorBody, description = "`This request was already decided`")
    )
)]
pub async fn reject_cover_suggestion(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(suggestion_id): ApiPath<i64>,
) -> ApiResult<Json<CoverSuggestionOut>> {
    let mut conn = state.db.acquire().await?;
    let row = read_suggestion(&mut conn, suggestion_id).await?;
    let mut tx = conn.begin().await?;
    let work_id = lock_current_work(&mut tx, row.work_id).await?;
    lock_open_suggestion(&mut tx, suggestion_id).await?;
    sqlx::query!(
        "UPDATE cover_suggestions SET status = 'rejected', decided_at = now() WHERE id = $1",
        suggestion_id
    )
    .execute(&mut *tx)
    .await?;
    if let Some(work_id) = work_id {
        sqlx::query!(
            "UPDATE cover_search SET next_search_at = NULL WHERE work_id = $1",
            work_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(suggestion_out(&mut conn, suggestion_id).await?))
}

/// The catalog and imported books without a usable cover (readers' own books are left out):
/// books the search looks for, books a reader's photo waits for, and books readers use whose
/// Open Library picture (or that of an edition readers use) failed to load. `?scope=in_use`
/// (the default) lists the books readers use, `?scope=all` every one; books readers use first,
/// then by title. 50 a page.
#[utoipa::path(
    get,
    path = "/api/admin/missing-covers",
    operation_id = "catalog_admin_list_missing_covers",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(MissingCoverQuery),
    responses(
        (status = 200, body = MissingCoverList, description = "A page of books without a cover"),
        (status = 400, body = ErrorBody, description = "`Input should be 'in_use' or 'all'`, `Input should be greater than or equal to 1`, `Input should be less than or equal to 1000`, `Input should be a valid integer, unable to parse string as an integer`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (not an instance admin)")
    )
)]
pub async fn list_missing_covers(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<MissingCoverQuery>,
) -> ApiResult<Json<MissingCoverList>> {
    let mut conn = state.db.acquire().await?;
    // The view `cover_gaps` is the one definition of the list (and of the summary's count).
    // Two statements rather than `$1 = 'all' OR in_use`, so the in-use one is planned as a
    // semi-join driven by the works readers use.
    let (limit, offset) = (PAGE_SIZE + 1, (query.page - 1) * PAGE_SIZE);
    let ids = if query.scope == "all" {
        sqlx::query_scalar!(
            r#"SELECT work_id AS "id!" FROM cover_gaps
               ORDER BY in_use DESC, lower(bc_unaccent(title)), work_id
               LIMIT $1 OFFSET $2"#,
            limit,
            offset
        )
        .fetch_all(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar!(
            r#"SELECT work_id AS "id!" FROM cover_gaps WHERE in_use
               ORDER BY lower(bc_unaccent(title)), work_id
               LIMIT $1 OFFSET $2"#,
            limit,
            offset
        )
        .fetch_all(&mut *conn)
        .await?
    };
    let has_more = ids.len() as i64 > PAGE_SIZE;
    let ids: Vec<i64> = ids.into_iter().take(PAGE_SIZE as usize).collect();
    let items = missing_cover_outs(&mut conn, &ids).await?;
    Ok(Json(MissingCoverList {
        items,
        page: query.page,
        has_more,
    }))
}

/// The [`MissingCoverOut`]s of `ids`, in that order.
async fn missing_cover_outs(
    conn: &mut PgConnection,
    ids: &[i64],
) -> sqlx::Result<Vec<MissingCoverOut>> {
    let rows = sqlx::query!(
        r#"SELECT w.id AS "id!", bc_cover_in_use(w.id) AS "in_use!",
                  EXISTS (SELECT 1 FROM legacy_work_keys k WHERE k.work_id = w.id) AS "imported!",
                  bc_cover_missing(w.cover_media_id) AS "missing!",
                  EXISTS (SELECT 1 FROM catalog_edit_requests r
                          WHERE r.work_id = w.id AND r.edition_id IS NULL
                            AND r.kind = 'cover_replace' AND r.status = 'open')
                      AS "photo_waiting!",
                  (SELECT f.failed_at FROM cover_failing f WHERE f.media_id = w.cover_media_id)
                      AS "work_failed_at?",
                  fe.edition_id AS "failing_edition_id?", fe.failed_at AS "edition_failed_at?",
                  cs.searched_at AS "searched_at?", cs.outcome AS "outcome?",
                  cs.next_search_at AS "next_search_at?", COALESCE(cs.failures, 0) AS "failures!",
                  EXISTS (SELECT 1 FROM cover_search t JOIN works tw ON tw.id = t.work_id
                          WHERE t.takedown_at IS NOT NULL
                            AND (tw.id = w.id OR tw.merged_into_id = w.id)) AS "after_takedown!",
                  (SELECT s.id FROM cover_suggestions s
                   WHERE s.work_id = w.id AND s.status = 'open') AS suggestion_id
           FROM works w
           LEFT JOIN cover_search cs ON cs.work_id = w.id
           LEFT JOIN LATERAL (
               SELECT f.edition_id, f.failed_at FROM cover_failing_editions f
               WHERE f.work_id = w.id ORDER BY f.edition_id LIMIT 1
           ) fe ON true
           WHERE w.id = ANY($1)"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = ids.iter().map(|&id| BookRef::work(id)).collect();
    let mut books = book_outs(conn, &refs).await?;
    let mut by_id: HashMap<i64, MissingCoverOut> = HashMap::new();
    for row in rows {
        let Some(book) = books.remove(&BookRef::work(row.id)) else {
            continue;
        };
        let (state, failing_edition_id, failed_at) = if row.missing {
            let state = if row.photo_waiting {
                "photo_waiting"
            } else {
                "none"
            };
            (state, None, None)
        } else if row.work_failed_at.is_some() {
            ("failing", None, row.work_failed_at)
        } else {
            ("failing", row.failing_edition_id, row.edition_failed_at)
        };
        by_id.insert(
            row.id,
            MissingCoverOut {
                book,
                in_use: row.in_use,
                imported: row.imported,
                state: state.to_owned(),
                failing_edition_id,
                failed_at,
                searched_at: row.searched_at,
                outcome: row.outcome,
                next_search_at: row.next_search_at,
                failures: row.failures,
                after_takedown: row.after_takedown,
                suggestion_id: row.suggestion_id,
            },
        );
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Searches this book now: ignores the schedule, the misses and the stop for books nobody
/// uses, respects rejections, approvals and takedowns. A failing Open Library picture of the
/// book is fetched again first: when it (or that of an edition readers use) still cannot be
/// loaded, the outcome is `failing` and nothing is searched (the search never replaces a
/// picture the book has; an upload does). An open suggestion is returned as it is. For catalog
/// and imported books only: a reader's own book is never sent to Open Library.
#[utoipa::path(
    post,
    path = "/api/admin/works/{work_id}/cover-search",
    operation_id = "catalog_admin_search_cover",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    responses(
        (status = 200, body = CoverSearchOut, description = "What the search came to, or `failing` when the book's picture still cannot be loaded"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.` (also for a book a reader added), or `Not found` (not an instance admin)"),
        (status = 409, body = ErrorBody, description = "`This book has a cover now`"),
        (status = 429, body = ErrorBody, description = "Open Library is busy, or `Slow down a little. Try again later.` (60 catalog requests per account a minute)"),
        (status = 502, body = ErrorBody, description = "`Could not search the library right now. Try again.`")
    )
)]
pub async fn search_cover(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<CoverSearchOut>> {
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let facts = search::facts(&mut conn, work_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    if facts.readers_book() {
        return Err(AppError::not_found(msg::NO_SUCH_BOOK));
    }
    let _turn = upstream_turn(&state, admin.id).await?;
    let failing = dead::failing_rows_of(&mut conn, work_id).await?;
    if !failing.is_empty() {
        dead::recheck(
            &state,
            &failing,
            &mut Pacer::none(),
            &mut None,
            RetryPolicy::MEDIA,
        )
        .await?;
    }
    let missing = sqlx::query_scalar!(
        r#"SELECT bc_cover_missing(cover_media_id) AS "missing!" FROM works WHERE id = $1"#,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    if !missing {
        // The picture the book has (or an edition readers use has) may still fail to load:
        // that is no cover for its readers, and the admin needs to know it is not settled.
        let Some((failed_at, edition_id)) = failing_picture(&mut conn, work_id).await? else {
            return Err(AppError::conflict(COVER_EXISTS));
        };
        let book = book_out(&mut conn, BookRef::work(work_id))
            .await?
            .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
        return Ok(Json(CoverSearchOut {
            outcome: "failing".to_owned(),
            book,
            suggestion: None,
            failed_at: Some(failed_at),
            failing_edition_id: edition_id,
        }));
    }
    let mut pacer = Pacer::none();
    let mut ctx = Context {
        lookup: state.lookup.as_ref(),
        pacer: &mut pacer,
        always_suggest: true,
        forced: true,
        rollback: false,
    };
    let outcome = search_one(&mut conn, &mut ctx, work_id).await?;
    let (name, suggestion) = match outcome {
        Outcome::Attached { media_id } => {
            state.media_service.prefetch(&[media_id]);
            ("attached", None)
        }
        Outcome::Isbn { media_id } => {
            state.media_service.prefetch(&[media_id]);
            ("isbn", None)
        }
        Outcome::Inherited => ("inherited", None),
        Outcome::Suggested { suggestion_id } => (
            "suggested",
            Some(suggestion_out(&mut conn, suggestion_id).await?),
        ),
        Outcome::Uncertain | Outcome::NoMatch | Outcome::Blocked | Outcome::Stopped => {
            ("no_match", None)
        }
        Outcome::Throttled => return Err(busy(&[&state.outbound.ol_api])),
        Outcome::Unavailable => return Err(AppError::bad_gateway(SEARCH_UNAVAILABLE)),
        Outcome::Skipped => return Err(AppError::conflict(COVER_EXISTS)),
    };
    let book = book_out(&mut conn, BookRef::work(work_id))
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    Ok(Json(CoverSearchOut {
        outcome: name.to_owned(),
        book,
        suggestion,
        failed_at: None,
        failing_edition_id: None,
    }))
}

/// The cover counts of the admin summary: open suggestions an admin still needs to decide, and
/// the books of the "Without a cover" list's default view (books readers use without a cover,
/// or whose picture fails to load): the same view, `cover_gaps`, so the count is the list's.
pub async fn cover_counts(conn: &mut PgConnection) -> sqlx::Result<(i64, i64)> {
    let row = sqlx::query!(
        r#"SELECT
               (SELECT count(*) FROM cover_suggestions
                WHERE status = 'open' AND NOT bc_cover_suggestion_moot(id)) AS "suggestions!",
               (SELECT count(*) FROM cover_gaps WHERE in_use) AS "missing!""#
    )
    .fetch_one(conn)
    .await?;
    Ok((row.suggestions, row.missing))
}
