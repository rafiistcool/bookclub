//! Reports on works and editions readers added (#54, Q11 option B):
//! `POST /api/books/works/{work_id}/reports` and `POST /api/books/editions/{edition_id}/reports`.
//! Covers and profile pictures are reported through `POST /api/media/{public_id}/reports`
//! (`media::reports`); every report lands in the one admin queue.
//!
//! Only `source = 'user'` records can be reported: their text is what a reader typed, and every
//! signed-in user sees it. Records from Open Library, the DNB or the old app are corrected
//! through an edit request or an admin's direct edit instead (Q18). A report follows merges to
//! the work a reader sees now. Like a cover report it is idempotent per reader and target (a
//! repeat answers with the reader's open report), and it shares the cover reports' limits: 20
//! new reports a day and 30 requests a minute per account. A report of a record whose text an
//! admin already withdrew ([`withdrawn`]) is recorded closed on arrival (`removed`), like a
//! report of a picture already taken down, so it never puts the record back in the queue.
//!
//! **The reported record.** A work report keeps the work as reported in
//! `reports.target_work_id`. A later merge re-points `reports.work_id` to the surviving work
//! (`catalog::merge`), so the queue shows the book readers see now, but the decision acts on
//! `target_work_id` only: a merge never hands the survivor, which can be an Open Library or DNB
//! work or another reader's, to a "remove". An edition report names its edition, which no merge
//! replaces (a merge moves editions to the surviving work).
//!
//! **"Remove"** ([`decide`]) withdraws what the reader typed and keeps the record, so the
//! shelves, posts and picks that point at it keep working:
//!
//! - a work gets the title [`REMOVED_TITLE`] and loses its subtitle, authors, description and
//!   subjects;
//! - an edition loses its own title, subtitle and publisher (it then shows its work's title).
//!
//! The fields go through `ingest::apply_admin_fields`, so they get provenance `admin` and no
//! source fills them again. Covers stay: a cover is reported on its own. A work merged away
//! since it was reported no longer shows on its own and cannot be removed (409
//! [`REPORTED_BOOK_MERGED`]); the admin closes the report, and corrects the surviving work with
//! the direct edit if it took over some of the reported text. "Dismiss" changes nothing. Either
//! way every open report of the same work (or edition) is resolved with it.
//!
//! Lock order of a decision: the work row, then (for an edition) the edition row, then the
//! reports, the order ingest and the direct admin edit use.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use sqlx::{Connection, PgConnection};

use super::detail::require_local_work;
use super::shared::ingest::{CoverTarget, apply_admin_fields};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::CatalogSource;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath};
use crate::media::reports::{REPORT_LOCK_SPACE, ReportCreated, ReportIn, created};

/// 400 for a report of a work or edition no reader added.
pub const ONLY_READER_BOOKS: &str = "Only books readers added can be reported";
/// The title of a user-added work an admin removed on a report. The SPA shows it translated on
/// the book page (`frontend/src/components/ReportSheet.vue`, `REMOVED_TITLE`).
pub const REMOVED_TITLE: &str = "[removed]";
/// 409 for "remove" on a work report whose work was merged into another one since it was
/// reported (admin only): the survivor is not the record the reader added.
pub const REPORTED_BOOK_MERGED: &str = "The reported book was merged into another one";

/// A work or an edition a reader added, as a report names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogTarget {
    /// The work as reported (`reports.target_work_id`).
    Work(i64),
    /// An edition and the work it belongs to.
    Edition { id: i64, work_id: i64 },
}

impl CatalogTarget {
    /// `reports.target_kind`.
    pub fn kind(self) -> &'static str {
        match self {
            CatalogTarget::Work(_) => "work",
            CatalogTarget::Edition { .. } => "edition",
        }
    }

    /// `reports.work_id`, `reports.edition_id` and `reports.target_work_id` of a new report.
    fn columns(self) -> (i64, Option<i64>, Option<i64>) {
        match self {
            CatalogTarget::Work(id) => (id, None, Some(id)),
            CatalogTarget::Edition { id, work_id } => (work_id, Some(id), None),
        }
    }

    /// The advisory lock key of one reader's reports of this target (`REPORT_LOCK_SPACE`; a
    /// cover report's key is `<user>:<media id>`).
    fn lock_key(self, user_id: i64) -> String {
        match self {
            CatalogTarget::Work(id) => format!("{user_id}:work:{id}"),
            CatalogTarget::Edition { id, .. } => format!("{user_id}:edition:{id}"),
        }
    }
}

/// Reports a book a reader added: its title, authors or description break the law or the
/// terms (`copyright`, `offensive`, `other`), or it is the wrong book (`wrong_book`). 20 new
/// reports per day and 30 requests per minute, shared with cover reports. Idempotent per
/// reader and book.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/reports",
    operation_id = "catalog_report_work",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or an admin already removed the record's text (`removed`, closed on arrival); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Only books readers added can be reported`, `Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report_work(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    hit_request_limit(&state, user.id)?;
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let source = sqlx::query_scalar!(
        r#"SELECT source AS "source: CatalogSource" FROM works WHERE id = $1"#,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    if source != CatalogSource::User {
        return Err(AppError::bad_request(ONLY_READER_BOOKS));
    }
    file(
        &state,
        &mut conn,
        user.id,
        CatalogTarget::Work(work_id),
        body,
    )
    .await
}

/// Reports an edition a reader added (its title, subtitle or publisher). Same limits and
/// queue as book reports.
#[utoipa::path(
    post,
    path = "/api/books/editions/{edition_id}/reports",
    operation_id = "catalog_report_edition",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("edition_id" = i64, Path, description = "The edition id")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or an admin already removed the record's text (`removed`, closed on arrival); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Only books readers added can be reported`, `Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report_edition(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(edition_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    hit_request_limit(&state, user.id)?;
    let mut conn = state.db.acquire().await?;
    let edition = sqlx::query!(
        r#"SELECT work_id, source AS "source: CatalogSource" FROM editions WHERE id = $1"#,
        edition_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    if edition.source != CatalogSource::User {
        return Err(AppError::bad_request(ONLY_READER_BOOKS));
    }
    let target = CatalogTarget::Edition {
        id: edition_id,
        work_id: edition.work_id,
    };
    file(&state, &mut conn, user.id, target, body).await
}

/// Every report request, a repeat or a refusal too, counts first (30 a minute).
fn hit_request_limit(state: &AppState, user_id: i64) -> ApiResult<()> {
    state
        .limits
        .report_request_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))
}

/// The reader's report of `target` with this status (`open`, or `removed` for a record whose
/// text was withdrawn), if any.
async fn earlier_report(
    conn: &mut PgConnection,
    user_id: i64,
    target: CatalogTarget,
    status: &str,
) -> sqlx::Result<Option<i64>> {
    let (_, edition_id, target_work_id) = target.columns();
    sqlx::query_scalar!(
        "SELECT id FROM reports
         WHERE created_by = $1 AND target_kind = $2 AND status = $5
           AND CASE $2 WHEN 'work' THEN target_work_id = $3 ELSE edition_id = $4 END
         ORDER BY id DESC LIMIT 1",
        user_id,
        target.kind(),
        target_work_id,
        edition_id,
        status
    )
    .fetch_optional(conn)
    .await
}

/// Whether an admin already withdrew the text of `target` on a report ([`decide`]): it was
/// removed once and has nothing left to withdraw (an admin's direct edit since that gave it
/// text again makes it reportable again). With `lock`, the record is read under the lock a
/// decision waits for, the work row first (conventions §7.2): `FOR SHARE` for a work, and for
/// an edition its work `FOR KEY SHARE` (the report's foreign key takes that lock anyway) and the
/// edition `FOR SHARE`. A decision under way then finishes first and this report sees it; a
/// later one waits and resolves the report.
async fn withdrawn(
    conn: &mut PgConnection,
    target: CatalogTarget,
    lock: bool,
) -> sqlx::Result<bool> {
    let withdrawn = match target {
        CatalogTarget::Work(id) => {
            let read = if lock {
                sqlx::query_scalar!(
                    r#"SELECT title = $2 AND subtitle = '' AND author_names = '' AND description = ''
                              AND cardinality(subjects) = 0 AS "bare!"
                       FROM works WHERE id = $1 FOR SHARE"#,
                    id,
                    REMOVED_TITLE
                )
                .fetch_optional(&mut *conn)
                .await?
            } else {
                sqlx::query_scalar!(
                    r#"SELECT title = $2 AND subtitle = '' AND author_names = '' AND description = ''
                              AND cardinality(subjects) = 0 AS "bare!"
                       FROM works WHERE id = $1"#,
                    id,
                    REMOVED_TITLE
                )
                .fetch_optional(&mut *conn)
                .await?
            };
            read.unwrap_or(false)
        }
        CatalogTarget::Edition { id, work_id } => {
            let read = if lock {
                sqlx::query!("SELECT id FROM works WHERE id = $1 FOR KEY SHARE", work_id)
                    .fetch_optional(&mut *conn)
                    .await?;
                sqlx::query_scalar!(
                    r#"SELECT title IS NULL AND subtitle IS NULL AND publisher = '' AS "bare!"
                       FROM editions WHERE id = $1 FOR SHARE"#,
                    id
                )
                .fetch_optional(&mut *conn)
                .await?
            } else {
                sqlx::query_scalar!(
                    r#"SELECT title IS NULL AND subtitle IS NULL AND publisher = '' AS "bare!"
                       FROM editions WHERE id = $1"#,
                    id
                )
                .fetch_optional(&mut *conn)
                .await?
            };
            read.unwrap_or(false)
        }
    };
    if !withdrawn {
        return Ok(false);
    }
    let (_, edition_id, target_work_id) = target.columns();
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM reports
                          WHERE target_kind = $1 AND status = 'removed' AND NOT closed_on_arrival
                            AND CASE $1 WHEN 'work' THEN target_work_id = $2
                                        ELSE edition_id = $3 END) AS "removed!""#,
        target.kind(),
        target_work_id,
        edition_id
    )
    .fetch_one(conn)
    .await
}

/// Files the report, or answers with the reader's earlier one. The reader's reports of one
/// target take turns (the advisory lock), so two at once are one report; only a new row counts
/// against the daily limit. A record whose text was withdrawn gets a report closed on arrival
/// (`removed`, no `resolved_by`), which never enters the open queue. A repeat is answered from a
/// plain read before any lock, as for picture reports (security review, Phase 2 Wave 1 #1).
async fn file(
    state: &AppState,
    conn: &mut PgConnection,
    user_id: i64,
    target: CatalogTarget,
    body: ReportIn,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    let status = |withdrawn: bool| if withdrawn { "removed" } else { "open" };
    let removed = withdrawn(conn, target, false).await?;
    if let Some(id) = earlier_report(conn, user_id, target, status(removed)).await? {
        return Ok(created(id, status(removed)));
    }
    let mut tx = conn.begin().await?;
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        REPORT_LOCK_SPACE,
        target.lock_key(user_id)
    )
    .execute(&mut *tx)
    .await?;
    let removed = withdrawn(&mut tx, target, true).await?;
    if let Some(id) = earlier_report(&mut tx, user_id, target, status(removed)).await? {
        tx.rollback().await?;
        return Ok(created(id, status(removed)));
    }
    state
        .limits
        .report_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (work_id, edition_id, target_work_id) = target.columns();
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, work_id, edition_id, target_work_id, reason, details,
                              created_by, status, resolved_at, closed_on_arrival)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CASE WHEN $9 THEN now() END, $9)
         RETURNING id",
        target.kind(),
        work_id,
        edition_id,
        target_work_id,
        body.reason,
        body.details.unwrap_or_default(),
        user_id,
        status(removed),
        removed
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(created(id, status(removed)))
}

/// What a decision on a catalog report found, for its notices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// The title as it was before the decision (an edition's own title, else its work's).
    pub title: String,
    /// The reader who added the record, if known.
    pub owner: Option<i64>,
    /// The record still had text to withdraw: "remove" changed it (and the creator gets a
    /// statement of reasons). False for a record withdrawn before.
    pub changed: bool,
    /// The reported work was merged into another one: it no longer shows on its own, and only
    /// "dismiss" decides it.
    pub merged: bool,
}

/// Decides a report of `target` inside the admin's transaction (see the module docs): with
/// `remove`, withdraws what the reader typed; otherwise only locks the record, so two
/// decisions of one record take turns. The caller then resolves the open reports. `None` when
/// the record is gone.
///
/// "Remove" acts only on a record a reader added that still shows on its own: a work merged
/// away since the report is 409 [`REPORTED_BOOK_MERGED`], a record from another source (which
/// filing refuses, so only a direct database change makes one) 400 [`ONLY_READER_BOOKS`].
pub async fn decide(
    conn: &mut PgConnection,
    target: CatalogTarget,
    remove: bool,
) -> ApiResult<Option<Decided>> {
    // An edition's work as it is now (a merge moves editions to the surviving work).
    let (work_id, edition_id) = match target {
        CatalogTarget::Work(id) => (id, None),
        CatalogTarget::Edition { id, .. } => {
            let work_id = sqlx::query_scalar!("SELECT work_id FROM editions WHERE id = $1", id)
                .fetch_optional(&mut *conn)
                .await?;
            match work_id {
                Some(work_id) => (work_id, Some(id)),
                None => return Ok(None),
            }
        }
    };
    let work = sqlx::query!(
        r#"SELECT title, subtitle, author_names, description,
                  cardinality(subjects) AS "subjects!", created_by,
                  source AS "source: CatalogSource", merged_into_id IS NOT NULL AS "merged!"
           FROM works WHERE id = $1 FOR UPDATE"#,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(work) = work else {
        return Ok(None);
    };
    let (decided, fields) = match edition_id {
        None => {
            if remove && work.merged {
                return Err(AppError::conflict(REPORTED_BOOK_MERGED));
            }
            if remove && work.source != CatalogSource::User {
                return Err(AppError::bad_request(ONLY_READER_BOOKS));
            }
            let changed = work.title != REMOVED_TITLE
                || !work.subtitle.is_empty()
                || !work.author_names.is_empty()
                || !work.description.is_empty()
                || work.subjects > 0;
            let decided = Decided {
                title: work.title,
                owner: work.created_by,
                changed,
                merged: work.merged,
            };
            let fields = fields(&[
                ("title", json!(REMOVED_TITLE)),
                ("subtitle", Value::Null),
                ("authors", Value::Null),
                ("description", Value::Null),
                ("subjects", Value::Null),
            ]);
            (decided, fields)
        }
        Some(edition_id) => {
            let edition = sqlx::query!(
                r#"SELECT title, subtitle, publisher, created_by,
                          source AS "source: CatalogSource"
                   FROM editions WHERE id = $1 FOR UPDATE"#,
                edition_id
            )
            .fetch_optional(&mut *conn)
            .await?;
            let Some(edition) = edition else {
                return Ok(None);
            };
            if remove && edition.source != CatalogSource::User {
                return Err(AppError::bad_request(ONLY_READER_BOOKS));
            }
            let changed = edition.title.is_some()
                || edition.subtitle.is_some()
                || !edition.publisher.is_empty();
            let decided = Decided {
                title: edition.title.unwrap_or(work.title),
                owner: edition.created_by,
                changed,
                merged: false,
            };
            let fields = fields(&[
                ("title", Value::Null),
                ("subtitle", Value::Null),
                ("publisher", Value::Null),
            ]);
            (decided, fields)
        }
    };
    if remove && decided.changed {
        let target = match edition_id {
            Some(id) => CoverTarget::Edition(id),
            None => CoverTarget::Work(work_id),
        };
        apply_admin_fields(conn, target, &fields).await?;
    }
    Ok(Some(decided))
}

/// The `fields` of an admin edit (`apply_admin_fields`).
fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}
