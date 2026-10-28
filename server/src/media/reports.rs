//! Notice and takedown (endpoint-map §6): any signed-in reader reports a cover, an instance
//! admin removes or dismisses it.
//!
//! **A report** names a picture, not a book, and the queue holds one open report per reader and
//! picture: a repeat answers with the reader's open report, and a report of a picture already
//! taken down is recorded as resolved (`removed`) and never enters the open queue. Both are
//! answered like the first report (201 with the same id), and only a new row counts against the
//! daily limit (`report_user`). Every request, a repeat too, counts against the per-minute limit
//! (`report_request_user`). A repeat is answered from a plain read before any lock, so a reader
//! cannot hold the picture with repeats and keep its takedown waiting (security review, Phase 2
//! Wave 1 #1). **Dismissing** closes every open report of the picture.
//!
//! **A takedown** ([`take_down`]), in one transaction: the media row is locked, the hashes of
//! its original bytes and of every stored variant go into `media_removed_hashes`, the row gets
//! `removed_at` (and loses `storage_key`), then `ingest::clear_media` clears every cover that
//! showed it, then the picture's open reports are resolved as `removed` and its open
//! `cover_replace` requests are rejected with a note. That order (media row, catalog rows,
//! reports, requests) is the one ingest, `set_cover` and `merge_works` lock in, so a takedown
//! waits for an ingest that is attaching the picture and then clears it too. Dismissals and
//! cover approvals lock the media row first as well; a new report locks it after its reader's
//! turn (the advisory lock) and before the reports row. The bytes are deleted after the
//! commit. The row stays, with its hash: `ingest::ol_cover_media` never hands
//! it out again, no fetch or backfill downloads it again, `/media/…` answers 410, and an upload
//! of the same file, or of a variant a reader saved from `/media/…`, is refused.
//!
//! **A direct takedown with a body** (`TakeDownIn`, D2) also records the notice it answers, an
//! email from someone without an account: one resolved `removed` report of the picture that
//! arrived at `received_at`, so the history and the response-time statistics include it.
//!
//! **Closed on arrival:** a report or notice that arrives when the picture is down already is
//! recorded as `removed` with no `resolved_by` and `resolved_at = created_at` (both writers set
//! that). The three together mark it, not `resolved_by` alone: deleting an admin's account sets
//! `resolved_by` to null on their decisions too. One case stays ambiguous: a notice recorded as
//! arriving at the instant of its own takedown (no `received_at`, or one clamped to now) whose
//! admin later deletes the account reads as closed on arrival (0 hours either way).

use std::collections::HashMap;

use axum::Json;
use axum::body::Body;
use axum::extract::{FromRequest, Request, State};
use axum::http::StatusCode;
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::delete_prefixes;
use super::service::sha256_all;
use super::shared::MediaStore;
use super::shared::urls::{cover_urls, storage_prefix};
use crate::AppState;
use crate::admin::queue::QueueQuery;
use crate::auth::extract::{CurrentUser, InstanceAdmin};
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::catalog::shared::ingest::clear_media;
use crate::dto::{BookOut, UserSummary, user_summaries};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{
    ApiJson, ApiPath, ApiQuery, BODY_TOO_LARGE, FIELD_REQUIRED, Validate, Validator,
};
use crate::text::{py_strip, truncate_chars};

/// Longest `details` of a report (silently cut, like request notes).
pub const MAX_DETAILS_CHARS: usize = 2000;
/// The pydantic enum message for an unknown `reason`.
pub const REASON_CHOICES: &str =
    "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'";
/// The pydantic enum message for an unknown `action`.
pub const ACTION_CHOICES: &str = "Input should be 'remove' or 'dismiss'";
/// 409 for a report an admin already resolved (admin only).
pub const REPORT_RESOLVED: &str = "This report was already resolved";
/// The decision note of a `cover_replace` request whose picture a takedown removed.
pub const TAKEN_DOWN_NOTE: &str = "The picture was taken down";
/// 400 for a notice `received_at` in the future (pydantic's `PastDatetime` message).
pub const RECEIVED_IN_FUTURE: &str = "Input should be in the past";
/// 400 for a notice about a picture that is not a cover: `reports.target_kind` knows only covers
/// until ADM2 widens it, so the notice could not be recorded.
pub const NOTICE_COVERS_ONLY: &str = "Only notices about covers can be recorded";
/// How far a `received_at` may lie ahead of the server's clock (the admin's clock may run fast).
const RECEIVED_AT_SLACK: time::Duration = time::Duration::minutes(5);

/// The advisory lock space (two-key form) that serialises one reader's reports of one picture.
/// Separate from `jobs::lock_ids` and ingest's `0x6263_0100`.
const REPORT_LOCK_SPACE: i32 = 0x6263_0200;

const REASONS: [&str; 4] = ["copyright", "wrong_book", "offensive", "other"];

/// A reader's report of a cover.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportIn {
    /// `copyright`, `wrong_book`, `offensive` or `other`.
    pub reason: String,
    /// What is wrong; at most 2000 characters.
    #[serde(default)]
    pub details: Option<String>,
}

impl Validate for ReportIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(REASONS.contains(&self.reason.as_str()), REASON_CHOICES);
        if let Some(details) = &mut self.details {
            *details = truncate_chars(py_strip(details), MAX_DETAILS_CHARS).to_owned();
        }
    }
}

/// The report is in the admin queue, or the picture is gone already.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportCreated {
    /// The report; a repeat names the reader's earlier report.
    pub id: i64,
    /// `open`: an admin will look at it; `removed`: the picture was already taken down, and
    /// nothing waits in the queue.
    pub status: String,
}

/// A report in the admin queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportOut {
    pub id: i64,
    /// `copyright`, `wrong_book`, `offensive` or `other`.
    pub reason: String,
    pub details: String,
    /// `open`, `removed` or `dismissed`.
    pub status: String,
    /// The reported picture.
    pub media_public_id: Option<String>,
    /// The picture was taken down (by this report or otherwise).
    pub removed: bool,
    /// The picture, null once it was taken down.
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    /// The book that showed the picture when it was reported (as it is now).
    pub book: Option<BookOut>,
    pub created_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub resolved_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    /// The picture was down already when the report arrived, so it was closed at once and no
    /// admin decided it. A decided report whose admin deleted the account has no `resolved_by`
    /// either, but is not closed on arrival.
    pub closed_on_arrival: bool,
}

/// A page of the report queue: the open reports oldest first, or the resolved ones newest
/// decision first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportList {
    pub items: Vec<ReportOut>,
    /// The page these items are (1-based).
    pub page: i64,
    /// Another page follows.
    pub has_more: bool,
}

/// What the admin decided.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportResolveIn {
    /// `remove` takes the picture down; `dismiss` keeps it.
    pub action: String,
}

impl Validate for ReportResolveIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            matches!(self.action.as_str(), "remove" | "dismiss"),
            ACTION_CHOICES,
        );
    }
}

/// The notice a direct takedown answers (`DELETE /api/admin/media/{public_id}` with a body):
/// how emailed notices from people without an account enter the record (D2).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct TakeDownIn {
    /// What the notice said and who sent it; at most 2000 characters (silently cut).
    #[serde(default)]
    pub details: Option<String>,
    /// When the notice arrived (RFC 3339); default now. Not in the future.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub received_at: Option<OffsetDateTime>,
    /// `copyright`, `wrong_book`, `offensive` or `other` (the default).
    #[serde(default)]
    pub reason: Option<String>,
}

impl Validate for TakeDownIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(reason) = &self.reason {
            v.check(REASONS.contains(&reason.as_str()), REASON_CHOICES);
        }
        if let Some(details) = &mut self.details {
            *details = truncate_chars(py_strip(details), MAX_DETAILS_CHARS).to_owned();
        }
        if let Some(received_at) = self.received_at {
            v.check(
                received_at <= OffsetDateTime::now_utc() + RECEIVED_AT_SLACK,
                RECEIVED_IN_FUTURE,
            );
        }
    }
}

/// A JSON body that may be left out: no body at all, or the JSON `null`, is `None`; anything
/// else goes through [`ApiJson`], with its 400s.
pub struct OptionalJson<T>(pub Option<T>);

impl<T, S> FromRequest<S> for OptionalJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();
        let bytes = Bytes::from_request(Request::from_parts(parts.clone(), body), state)
            .await
            .map_err(|rejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    AppError::payload_too_large(BODY_TOO_LARGE)
                } else {
                    AppError::bad_request(FIELD_REQUIRED)
                }
            })?;
        if bytes.is_empty()
            || serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|v| v.is_null())
        {
            return Ok(OptionalJson(None));
        }
        let ApiJson(value) =
            ApiJson::<T>::from_request(Request::from_parts(parts, Body::from(bytes)), state)
                .await?;
        Ok(OptionalJson(Some(value)))
    }
}

/// Reports a cover (`cover_public_id` of a book or edition): copyright, the wrong book,
/// offensive, or something else. 20 new reports per day, and 30 requests per minute, repeats
/// included. Idempotent per reader and picture: a repeat answers with the reader's open report,
/// and a picture already taken down gets a closed record (`status` `removed`); neither counts
/// against the daily limit again.
#[utoipa::path(
    post,
    path = "/api/media/{public_id}/reports",
    operation_id = "media_report",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("public_id" = String, Path, description = "The cover's `cover_public_id`")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or the picture was already taken down (`removed`); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`: no such cover"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(public_id): ApiPath<String>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    // Every request, a repeat or a 404 too, before anything else: a repeat costs nothing against
    // the daily limit, but it cannot be sent without end either.
    state
        .limits
        .report_request_user
        .check_and_hit(&format!("u:{}", user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let mut conn = state.db.acquire().await?;
    // A repeat is answered from this plain read, one snapshot, without a lock. A repeat that
    // locked the picture could, sent often enough at once, hold it `FOR SHARE` without a break,
    // and a takedown or dismissal would wait for its turn until the request deadline. One that
    // races a takedown may still answer `open`; the takedown then resolves that report.
    let found = sqlx::query!(
        r#"SELECT m.id, m.removed_at IS NOT NULL AS "removed!",
                  (SELECT r.id FROM reports r
                   WHERE r.created_by = $2 AND r.media_id = m.id
                     AND r.status = CASE WHEN m.removed_at IS NULL THEN 'open' ELSE 'removed' END
                   ORDER BY r.id DESC LIMIT 1) AS earlier
           FROM media m WHERE m.public_id = $1 AND m.kind = 'cover'"#,
        public_id,
        user.id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if let Some(id) = found.earlier {
        return Ok(created(id, if found.removed { "removed" } else { "open" }));
    }
    let media_id = found.id;
    let mut tx = conn.begin().await?;
    // The reader's turn first: one reader's reports of one picture take turns, so two at once
    // are one report, and only the one whose turn it is holds the picture. Taken before the
    // picture, a request that waits for its turn holds nothing a takedown needs.
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2::int8::text || ':' || $3::int8::text))",
        REPORT_LOCK_SPACE,
        user.id,
        media_id
    )
    .execute(&mut *tx)
    .await?;
    // Then the picture (conventions §7.2), FOR SHARE: a takedown or a dismissal that is under
    // way finishes first and this report sees it; a later one waits and then resolves it.
    let removed = sqlx::query_scalar!(
        r#"SELECT removed_at IS NOT NULL AS "removed!" FROM media WHERE id = $1 FOR SHARE"#,
        media_id
    )
    .fetch_one(&mut *tx)
    .await?;
    let status = if removed { "removed" } else { "open" };
    let earlier = sqlx::query_scalar!(
        "SELECT id FROM reports
         WHERE created_by = $1 AND media_id = $2 AND status = $3
         ORDER BY id DESC LIMIT 1",
        user.id,
        media_id,
        status
    )
    .fetch_optional(&mut *tx)
    .await?;
    // Another of the reader's requests filed it while this one waited for its turn, or the
    // picture came down since the plain read.
    if let Some(id) = earlier {
        tx.rollback().await?;
        return Ok(created(id, status));
    }
    state
        .limits
        .report_user
        .check_and_hit(&format!("u:{}", user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (work_id, edition_id) = shown_on(&mut tx, media_id).await?;
    // A picture taken down already: the notice is recorded, closed at once (no admin decided
    // it, so `resolved_by` stays null).
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, media_id, work_id, edition_id, reason, details,
                              created_by, status, resolved_at)
         VALUES ('cover', $1, $2, $3, $4, $5, $6, $7, CASE WHEN $8 THEN now() END)
         RETURNING id",
        media_id,
        work_id,
        edition_id,
        body.reason,
        body.details.unwrap_or_default(),
        user.id,
        status,
        removed
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(created(id, status))
}

/// The book that shows the picture, for a report's `work_id` and `edition_id`: an edition (and
/// its work) first, else a work, else the book a `cover_replace` request proposed it for.
async fn shown_on(
    conn: &mut PgConnection,
    media_id: i64,
) -> sqlx::Result<(Option<i64>, Option<i64>)> {
    let row = sqlx::query!(
        r#"SELECT (SELECT e.work_id FROM editions e WHERE e.cover_media_id = $1
                   ORDER BY e.id LIMIT 1) AS edition_work,
                  (SELECT e.id FROM editions e WHERE e.cover_media_id = $1
                   ORDER BY e.id LIMIT 1) AS edition,
                  (SELECT w.id FROM works w WHERE w.cover_media_id = $1
                   ORDER BY w.id LIMIT 1) AS work,
                  (SELECT r.work_id FROM catalog_edit_requests r WHERE r.media_id = $1
                   ORDER BY r.id LIMIT 1) AS requested_work,
                  (SELECT r.edition_id FROM catalog_edit_requests r WHERE r.media_id = $1
                   ORDER BY r.id LIMIT 1) AS requested_edition"#,
        media_id
    )
    .fetch_one(conn)
    .await?;
    Ok(match (row.edition_work, row.edition, row.work) {
        (Some(work), Some(edition), _) => (Some(work), Some(edition)),
        (_, _, Some(work)) => (Some(work), None),
        _ => (row.requested_work, row.requested_edition),
    })
}

fn created(id: i64, status: &str) -> (StatusCode, Json<ReportCreated>) {
    (
        StatusCode::CREATED,
        Json(ReportCreated {
            id,
            status: status.to_owned(),
        }),
    )
}

/// A page of the report queue: `?status=open` (the default) lists the open reports oldest
/// first, `?status=resolved` the removed and dismissed ones, the newest decision first (a report
/// of a picture already taken down is `closed_on_arrival`, with no `resolved_by`). 50 a page.
#[utoipa::path(
    get,
    path = "/api/admin/reports",
    operation_id = "media_admin_list_reports",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(QueueQuery),
    responses(
        (status = 200, body = ReportList, description = "A page of the open or the resolved reports"),
        (status = 400, body = ErrorBody, description = "`Input should be 'open' or 'resolved'`, `Input should be greater than or equal to 1`, `Input should be less than or equal to 1000`, `Input should be a valid integer, unable to parse string as an integer`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` for everyone but instance admins")
    )
)]
pub async fn list_reports(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<QueueQuery>,
) -> ApiResult<Json<ReportList>> {
    let mut conn = state.db.acquire().await?;
    let ids = if query.resolved() {
        sqlx::query_scalar!(
            "SELECT id FROM reports WHERE status <> 'open'
             ORDER BY resolved_at DESC NULLS LAST, id DESC
             LIMIT $1 OFFSET $2",
            query.limit(),
            query.offset()
        )
        .fetch_all(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar!(
            "SELECT id FROM reports WHERE status = 'open'
             ORDER BY created_at, id
             LIMIT $1 OFFSET $2",
            query.limit(),
            query.offset()
        )
        .fetch_all(&mut *conn)
        .await?
    };
    let (ids, has_more) = query.split(ids);
    Ok(Json(ReportList {
        items: report_outs(&mut conn, &ids).await?,
        page: query.page,
        has_more,
    }))
}

/// Removes the reported picture (everywhere it is shown, for good) or dismisses the report.
/// Either way every open report of the picture is resolved with it.
#[utoipa::path(
    post,
    path = "/api/admin/reports/{report_id}/resolve",
    operation_id = "media_admin_resolve_report",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("report_id" = i64, Path, description = "The report id")),
    request_body = ReportResolveIn,
    responses(
        (status = 200, body = ReportOut, description = "The resolved report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'remove' or 'dismiss'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`"),
        (status = 409, body = ErrorBody, description = "`This report was already resolved`")
    )
)]
pub async fn resolve_report(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(report_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportResolveIn>,
) -> ApiResult<Json<ReportOut>> {
    let report = sqlx::query!(
        "SELECT media_id, status FROM reports WHERE id = $1",
        report_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if report.status != "open" {
        return Err(AppError::conflict(REPORT_RESOLVED));
    }
    let mut tx = state.db.begin().await?;
    let mut prefixes = Vec::new();
    let status = if body.action == "remove" {
        if let Some(media_id) = report.media_id {
            // Resolves the picture's open reports, this one included.
            prefixes = take_down_locked(&mut tx, state.media.as_ref(), media_id, admin.id).await?;
        }
        "removed"
    } else {
        if let Some(media_id) = report.media_id {
            // The picture first, as a takedown and a report lock it: dismissals and takedowns
            // of one picture take turns, and a report filed meanwhile is dismissed too.
            sqlx::query_scalar!(
                "SELECT id FROM media WHERE id = $1 FOR NO KEY UPDATE",
                media_id
            )
            .fetch_optional(&mut *tx)
            .await?;
        }
        "dismissed"
    };
    // This report, and every other open report of the picture.
    sqlx::query!(
        "UPDATE reports SET status = $3, resolved_by = $2, resolved_at = now()
         WHERE status = 'open' AND (id = $1 OR media_id = $4)",
        report_id,
        admin.id,
        status,
        report.media_id
    )
    .execute(&mut *tx)
    .await?;
    // Another admin may have resolved it since it was read; then this answer changes nothing
    // (the rollback undoes a takedown too).
    let ours = sqlx::query_scalar!(
        r#"SELECT COALESCE(resolved_by = $2 AND resolved_at = now(), false) AS "ours!"
           FROM reports WHERE id = $1"#,
        report_id,
        admin.id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if !ours {
        return Err(AppError::conflict(REPORT_RESOLVED));
    }
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    let mut conn = state.db.acquire().await?;
    let out = report_outs(&mut conn, &[report_id])
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    Ok(Json(out))
}

/// Takes a picture down directly, without a report: same effect as resolving one with
/// `remove`. Taking down a removed picture changes nothing. With a body (a notice that arrived
/// by email, D2), the takedown is recorded as a resolved `removed` report of the picture:
/// `created_at` is when the notice arrived, `created_by` is null (the sender has no account
/// here), `resolved_by` the admin. A picture that was down already gets the record too, closed
/// when it came down (or on arrival, with no `resolved_by`, when it was down before the notice
/// arrived). A notice about an avatar is refused before anything changes (400, as
/// `reports.target_kind` knows only covers until ADM2); without a body an avatar is taken down.
#[utoipa::path(
    delete,
    path = "/api/admin/media/{public_id}",
    operation_id = "media_admin_take_down",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("public_id" = String, Path, description = "The picture's public id")),
    request_body(
        content = Option<TakeDownIn>,
        description = "Optional: the notice this takedown answers, recorded as a resolved report. No body, or `null`, takes the picture down without a record"
    ),
    responses(
        (status = 204, description = "The picture is removed (and the notice recorded)"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `Input should be in the past` (`received_at`), `Invalid value`; `Only notices about covers can be recorded` (a notice about an avatar: nothing is taken down)"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`")
    )
)]
pub async fn take_down(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(public_id): ApiPath<String>,
    OptionalJson(notice): OptionalJson<TakeDownIn>,
) -> ApiResult<StatusCode> {
    let media_id = sqlx::query_scalar!("SELECT id FROM media WHERE public_id = $1", public_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    let mut tx = state.db.begin().await?;
    // The picture first (conventions §7.2), and the book that shows it before the takedown
    // clears its covers.
    let picture = sqlx::query!(
        "SELECT kind, removed_at FROM media WHERE id = $1 FOR UPDATE",
        media_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    // The record would be lost: refuse before anything changes (the rollback drops the lock).
    if notice.is_some() && picture.kind != "cover" {
        return Err(AppError::bad_request(NOTICE_COVERS_ONLY));
    }
    let shown = shown_on(&mut tx, media_id).await?;
    let prefixes = take_down_locked(&mut tx, state.media.as_ref(), media_id, admin.id).await?;
    if let Some(notice) = notice {
        record_notice(
            &mut tx,
            media_id,
            shown,
            &notice,
            picture.removed_at,
            admin.id,
        )
        .await?;
    }
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The record of a direct takedown ([`take_down`] with a body): a `removed` report that arrived
/// at `received_at` and was resolved by `admin_id` now, or, when the picture was down already
/// (`removed_at`), when it came down; a notice that arrived after that was closed on arrival
/// (no `resolved_by`, like a reader's report of a removed picture).
async fn record_notice(
    conn: &mut PgConnection,
    media_id: i64,
    (work_id, edition_id): (Option<i64>, Option<i64>),
    notice: &TakeDownIn,
    removed_at: Option<OffsetDateTime>,
    admin_id: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO reports (target_kind, media_id, work_id, edition_id, reason, details,
                              created_by, created_at, status, resolved_by, resolved_at)
         SELECT 'cover', $1, $2, $3, $4, $5, NULL, received, 'removed',
                CASE WHEN removed IS NULL OR removed > received THEN $7::int8 END,
                CASE WHEN removed IS NULL THEN now() ELSE greatest(removed, received) END
         FROM (SELECT least(coalesce($6, now()), now()) AS received,
                      $8::timestamptz AS removed) AS t",
        media_id,
        work_id,
        edition_id,
        notice.reason.as_deref().unwrap_or("other"),
        notice.details.as_deref().unwrap_or_default(),
        notice.received_at,
        admin_id,
        removed_at
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The takedown inside the caller's transaction (see the module docs): locks the media row,
/// records the hashes it must not come back under, sets `removed_at` and clears `storage_key`,
/// clears every cover that shows it, resolves the picture's open reports as `removed` by
/// `admin_id`, and rejects its open `cover_replace` requests with [`TAKEN_DOWN_NOTE`]. Returns the storage prefixes to delete after the commit (empty when the picture
/// was removed already).
pub async fn take_down_locked(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
    media_id: i64,
    admin_id: i64,
) -> ApiResult<Vec<String>> {
    let Some(row) = sqlx::query!(
        r#"SELECT kind, public_id, storage_key, variants, sha256,
                  removed_at IS NOT NULL AS "removed!"
           FROM media WHERE id = $1 FOR UPDATE"#,
        media_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(Vec::new());
    };
    let mut prefixes = Vec::new();
    if !row.removed {
        let stored = match &row.storage_key {
            Some(prefix) => stored_variants(store, prefix, &row.variants).await?,
            None => Vec::new(),
        };
        record_removed_hashes(conn, media_id, row.sha256, stored).await?;
        sqlx::query!(
            "UPDATE media SET removed_at = now(), storage_key = NULL WHERE id = $1",
            media_id
        )
        .execute(&mut *conn)
        .await?;
        prefixes.push(storage_prefix(&row.kind, &row.public_id));
        prefixes.extend(row.storage_key);
    }
    clear_media(conn, media_id).await?;
    sqlx::query!(
        "UPDATE reports SET status = 'removed', resolved_by = $2, resolved_at = now()
         WHERE media_id = $1 AND status = 'open'",
        media_id,
        admin_id
    )
    .execute(&mut *conn)
    .await?;
    // Approving them later could only fail with `This picture was removed`.
    sqlx::query!(
        "UPDATE catalog_edit_requests
         SET status = 'rejected', decided_by = $2, decided_at = now(), decision_note = $3
         WHERE media_id = $1 AND kind = 'cover_replace' AND status = 'open'",
        media_id,
        admin_id,
        TAKEN_DOWN_NOTE
    )
    .execute(&mut *conn)
    .await?;
    Ok(prefixes)
}

/// The bytes of every stored variant, as `/media/{public_id}/{variant}` served them. A variant
/// whose bytes are missing is skipped (nothing can have been served from it).
async fn stored_variants(
    store: &dyn MediaStore,
    prefix: &str,
    variants: &[String],
) -> anyhow::Result<Vec<(String, Bytes)>> {
    let mut stored = Vec::new();
    for variant in variants {
        let key = format!("{prefix}/{variant}");
        match store.get(&key).await? {
            Some(bytes) => stored.push((variant.clone(), bytes)),
            None => tracing::warn!(key, "a stored variant of a picture taken down has no bytes"),
        }
    }
    Ok(stored)
}

/// Records the hashes a taken-down picture must not come back under (`media_removed_hashes`,
/// checked by every upload): the bytes as received (`original`, a copy of `media.sha256` that
/// outlives the row) and every stored variant, the bytes readers actually saw and saved. An
/// Open Library cover taken down before its first fetch has neither: we never had its bytes,
/// and fetching them now would download the removed picture again.
async fn record_removed_hashes(
    conn: &mut PgConnection,
    media_id: i64,
    original: Option<Vec<u8>>,
    stored: Vec<(String, Bytes)>,
) -> anyhow::Result<()> {
    let (names, bytes): (Vec<String>, Vec<Bytes>) = stored.into_iter().unzip();
    let mut hashes: Vec<Vec<u8>> = sha256_all(bytes)
        .await?
        .into_iter()
        .map(|hash| hash.to_vec())
        .collect();
    let mut variants: Vec<Option<String>> = names.into_iter().map(Some).collect();
    if let Some(original) = original {
        hashes.insert(0, original);
        variants.insert(0, None);
    }
    sqlx::query!(
        "INSERT INTO media_removed_hashes (sha256, media_id, variant)
         SELECT sha256, $1, variant FROM UNNEST($2::bytea[], $3::text[]) AS h (sha256, variant)
         ON CONFLICT (sha256) DO NOTHING",
        media_id,
        &hashes,
        &variants as &[Option<String>]
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The [`ReportOut`]s of `ids`, in that order (missing ids are left out).
pub async fn report_outs(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<Vec<ReportOut>> {
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!", r.reason AS "reason!", r.details AS "details!",
                  r.status AS "status!", r.work_id, r.edition_id, r.created_by,
                  r.created_at AS "created_at!", r.resolved_by, r.resolved_at,
                  COALESCE(r.status = 'removed' AND r.resolved_by IS NULL
                           AND r.resolved_at = r.created_at, false) AS "closed_on_arrival!",
                  m.public_id AS "public_id?", m.removed_at IS NOT NULL AS "removed?"
           FROM reports r
           LEFT JOIN media m ON m.id = r.media_id
           WHERE r.id = ANY($1)"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = rows
        .iter()
        .filter_map(|row| {
            row.work_id.map(|work_id| BookRef {
                work_id,
                edition_id: row.edition_id,
            })
        })
        .collect();
    let books = book_outs(conn, &refs).await?;
    let user_ids: Vec<i64> = rows
        .iter()
        .flat_map(|row| [row.created_by, row.resolved_by])
        .flatten()
        .collect();
    let users = user_summaries(conn, &user_ids).await?;
    let mut by_id: HashMap<i64, ReportOut> = rows
        .into_iter()
        .map(|row| {
            let removed = row.removed.unwrap_or(false);
            let shown = row.public_id.as_deref().filter(|_| !removed);
            let (cover_url, cover_thumb_url) = cover_urls(shown);
            let book = row.work_id.and_then(|work_id| {
                books
                    .get(&BookRef {
                        work_id,
                        edition_id: row.edition_id,
                    })
                    .cloned()
            });
            let out = ReportOut {
                id: row.id,
                reason: row.reason,
                details: row.details,
                status: row.status,
                media_public_id: row.public_id,
                removed,
                cover_url,
                cover_thumb_url,
                book,
                created_by: row.created_by.and_then(|id| users.get(&id).cloned()),
                created_at: row.created_at,
                resolved_by: row.resolved_by.and_then(|id| users.get(&id).cloned()),
                resolved_at: row.resolved_at,
                closed_on_arrival: row.closed_on_arrival,
            };
            (row.id, out)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}
