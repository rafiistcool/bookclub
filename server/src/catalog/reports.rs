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
//! new reports a day and 30 requests a minute per account.
//!
//! **"Remove"** ([`decide`]) withdraws what the reader typed and keeps the record, so the
//! shelves, posts and picks that point at it keep working:
//!
//! - a work gets the title [`REMOVED_TITLE`] and loses its subtitle, authors, description and
//!   subjects;
//! - an edition loses its own title, subtitle and publisher (it then shows its work's title).
//!
//! The fields go through `ingest::apply_admin_fields`, so they get provenance `admin` and no
//! source fills them again. Covers stay: a cover is reported on its own. "Dismiss" changes
//! nothing. Either way every open report of the same work (or edition) is resolved with it.
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
/// The title of a user-added work an admin removed on a report.
pub const REMOVED_TITLE: &str = "[removed]";

/// A work or an edition a reader added, as a report names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogTarget {
    Work(i64),
    /// An edition and the work it belongs to.
    Edition {
        id: i64,
        work_id: i64,
    },
}

impl CatalogTarget {
    /// `reports.target_kind`.
    pub fn kind(self) -> &'static str {
        match self {
            CatalogTarget::Work(_) => "work",
            CatalogTarget::Edition { .. } => "edition",
        }
    }

    /// `reports.work_id` and `reports.edition_id`.
    fn columns(self) -> (i64, Option<i64>) {
        match self {
            CatalogTarget::Work(id) => (id, None),
            CatalogTarget::Edition { id, work_id } => (work_id, Some(id)),
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
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`); a repeat names the same report"),
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
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`); a repeat names the same report"),
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

/// The reader's open report of `target`, if any.
async fn open_report(
    conn: &mut PgConnection,
    user_id: i64,
    target: CatalogTarget,
) -> sqlx::Result<Option<i64>> {
    let (work_id, edition_id) = target.columns();
    sqlx::query_scalar!(
        "SELECT id FROM reports
         WHERE created_by = $1 AND target_kind = $2 AND status = 'open'
           AND CASE $2 WHEN 'work' THEN work_id = $3 ELSE edition_id = $4 END
         ORDER BY id DESC LIMIT 1",
        user_id,
        target.kind(),
        work_id,
        edition_id
    )
    .fetch_optional(conn)
    .await
}

/// Files the report, or answers with the reader's open one. The reader's reports of one target
/// take turns (the advisory lock), so two at once are one report; only a new row counts
/// against the daily limit.
async fn file(
    state: &AppState,
    conn: &mut PgConnection,
    user_id: i64,
    target: CatalogTarget,
    body: ReportIn,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    if let Some(id) = open_report(conn, user_id, target).await? {
        return Ok(created(id, "open"));
    }
    let mut tx = conn.begin().await?;
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        REPORT_LOCK_SPACE,
        target.lock_key(user_id)
    )
    .execute(&mut *tx)
    .await?;
    if let Some(id) = open_report(&mut tx, user_id, target).await? {
        tx.rollback().await?;
        return Ok(created(id, "open"));
    }
    state
        .limits
        .report_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (work_id, edition_id) = target.columns();
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, work_id, edition_id, reason, details, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
        target.kind(),
        work_id,
        edition_id,
        body.reason,
        body.details.unwrap_or_default(),
        user_id
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(created(id, "open"))
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
}

/// Decides a report of `target` inside the admin's transaction (see the module docs): with
/// `remove`, withdraws what the reader typed; otherwise only locks the record, so two
/// decisions of one record take turns. The caller then resolves the open reports. `None` when
/// the record is gone.
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
                  cardinality(subjects) AS "subjects!", created_by
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
            let changed = work.title != REMOVED_TITLE
                || !work.subtitle.is_empty()
                || !work.author_names.is_empty()
                || !work.description.is_empty()
                || work.subjects > 0;
            let decided = Decided {
                title: work.title,
                owner: work.created_by,
                changed,
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
                "SELECT title, subtitle, publisher, created_by
                 FROM editions WHERE id = $1 FOR UPDATE",
                edition_id
            )
            .fetch_optional(&mut *conn)
            .await?;
            let Some(edition) = edition else {
                return Ok(None);
            };
            let changed = edition.title.is_some()
                || edition.subtitle.is_some()
                || !edition.publisher.is_empty();
            let decided = Decided {
                title: edition.title.unwrap_or(work.title),
                owner: edition.created_by,
                changed,
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
