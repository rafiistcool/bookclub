//! Notice and takedown (endpoint-map §6): any signed-in reader reports a cover, or the profile
//! picture of someone they can see (they share a club, ADM2, Q11 option B); an instance admin
//! removes or dismisses it. Reports on works and editions readers added are filed in
//! `catalog::reports` and decided here, in the same queue ([`resolve_report`]).
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
//! turn (the advisory lock) and before the reports row. For a profile picture, each of them
//! locks the owner's row before the picture ([`lock_owner`]). The bytes are deleted after the
//! commit. The row stays, with its hash: `ingest::ol_cover_media` never hands
//! it out again, no fetch or backfill downloads it again, `/media/…` answers 410, and an upload
//! of the same file, or of a variant a reader saved from `/media/…`, is refused.
//!
//! **A direct takedown with a body** (`TakeDownIn`, D2) also records the notice it answers, an
//! email from someone without an account: one resolved `removed` report of the picture (cover
//! or avatar, `via = 'notice'`) that arrived at `received_at`, so the history and the
//! response-time statistics include it.
//!
//! **"Remove" per kind** (docs/phase2/notes/adm2.md): a cover or a profile picture is taken
//! down as above (a removed avatar shows nowhere: every reader of `users.avatar_media_id` skips
//! a removed picture); a work or edition a reader added loses what the reader typed
//! (`catalog::reports::decide`).
//!
//! **Decision notices** (Q12 option A, `admin::notices`): after a decision commits, each
//! reporter is mailed the decision, and on a removal the uploader, the avatar's owner or the
//! reader who added the record gets a statement of reasons. A rollback sends nothing.
//!
//! **Held** (security review, Phase 2 Wave 2): a profile picture its owner replaces or deletes
//! while it is reported keeps its row and files until an admin decides (`media/avatar.rs`),
//! under a new public id that only the admin queue shows; its old URL answers 404 as for a
//! deleted picture, and the owner's export leaves it out, so neither tells the owner of the
//! report. "Remove" takes it down like any picture and blocks its hashes, so the same file cannot
//! come back; a takedown of a profile picture also takes down the owner's other live profile
//! pictures with the same bytes as received (the same file uploaded again meanwhile). "Dismiss"
//! deletes the held picture with its files, and its reporters learn that it is no longer online.
//!
//! **Copies of a cover** (#74, security review of the cover search): a cover's takedown also
//! takes down the live covers with the same bytes as received that the same reader uploaded,
//! or that came from Open Library (no uploader, so one statement of reasons covers them all):
//! every such copy when the takedown judged the picture ([`Judged::Picture`]), only those the
//! same book shows when every reason behind it was `wrong_book` ([`Judged::WrongBook`]). The
//! cover search would otherwise put a copy on an edition back on the book. Only a takedown that
//! judged a cover's picture marks its hashes `blocks_covers`: Open Library fetches and the cover
//! search refuse those bytes, while every recorded hash keeps refusing uploads.
//!
//! **Gone** (`ReportOut.gone`): a profile picture whose row is gone, because its owner deleted
//! the account (`auth::deletion`; the report keeps `target_user_id` until then), or a work merged
//! into another one since it was reported (`catalog::reports`). Nothing is left to take down, so
//! "remove" is refused (409 [`PICTURE_GONE`], `catalog::reports::REPORTED_BOOK_MERGED`) and
//! "dismiss" closes the report, a merged work's with every other open report of that work; the
//! reporters learn that it is no longer online, and nobody gets a statement of reasons. Each
//! report of a deleted account's profile picture is closed on its own: the deletion clears its
//! `media_id` and its `target_user_id` (`ON DELETE SET NULL`), so nothing ties it to the others.
//!
//! **Closed on arrival** (`reports.closed_on_arrival`, migration 2101): a report or notice that
//! arrives when the picture is down already is recorded as `removed`, closed at once, with no
//! `resolved_by` and `resolved_at = created_at`.

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
use super::shared::urls::{avatar_url, cover_urls, storage_prefix};
use crate::AppState;
use crate::admin::notices::{self, Notices, Removal};
use crate::admin::queue::QueueQuery;
use crate::auth::extract::{CurrentUser, InstanceAdmin};
use crate::catalog::reports::{self as catalog_reports, CatalogTarget};
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::catalog::shared::ingest::clear_media;
use crate::dto::{BookOut, UserSummary, user_summaries};
use crate::email::templates::Reported;
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
/// 409 for "remove" on a report of a profile picture whose row is gone since (its owner deleted
/// the account; admin only): there is nothing left to take down.
pub const PICTURE_GONE: &str = "The reported picture is no longer online";
/// The decision note of a `cover_replace` request whose picture a takedown removed.
pub const TAKEN_DOWN_NOTE: &str = "The picture was taken down";
/// 400 for a notice `received_at` in the future (pydantic's `PastDatetime` message).
pub const RECEIVED_IN_FUTURE: &str = "Input should be in the past";
/// How far a `received_at` may lie ahead of the server's clock (the admin's clock may run fast).
const RECEIVED_AT_SLACK: time::Duration = time::Duration::minutes(5);

/// The advisory lock space (two-key form) that serialises one reader's reports of one picture
/// (key `<user id>:<media id>`), work (`<user id>:work:<id>`) or edition
/// (`<user id>:edition:<id>`). Separate from `jobs::lock_ids` and ingest's `0x6263_0100`.
pub(crate) const REPORT_LOCK_SPACE: i32 = 0x6263_0200;

const REASONS: [&str; 4] = ["copyright", "wrong_book", "offensive", "other"];

/// A reader's report of a cover, a profile picture, or a work or edition a reader added.
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
    /// What was reported: `cover`, `avatar` (a profile picture), `work` or `edition` (one a
    /// reader added).
    pub target_kind: String,
    /// `copyright`, `wrong_book`, `offensive` or `other`.
    pub reason: String,
    pub details: String,
    /// `open`, `removed` or `dismissed`.
    pub status: String,
    /// `app` (a reader's report) or `notice` (an emailed notice an admin recorded with a
    /// takedown; no `created_by`).
    pub via: String,
    /// The reported picture (covers and avatars).
    pub media_public_id: Option<String>,
    /// The picture was taken down (by this report or otherwise).
    pub removed: bool,
    /// A reported cover, null once it was taken down.
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    /// A reported profile picture (`/media/{public_id}/a`), null once it was taken down.
    pub avatar_url: Option<String>,
    /// Whose profile picture was reported (avatars), as the account is now.
    pub target_user: Option<UserSummary>,
    /// Covers: the book that showed the picture when it was reported; works and editions: the
    /// reported book (with the edition), as it is now.
    pub book: Option<BookOut>,
    pub created_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub resolved_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    /// The picture was down already when the report arrived, so it was closed at once and no
    /// admin decided it (`reports.closed_on_arrival`). A decided report whose admin deleted the
    /// account has no `resolved_by` either, but is not closed on arrival.
    pub closed_on_arrival: bool,
    /// What was reported no longer shows as reported, as it is now: a profile picture without
    /// its row (the account was deleted, or the picture was held for a report and dismissed),
    /// or a book merged into another one (`book` is that one). Only `dismiss` decides such a
    /// report; `remove` answers 409. A picture its owner replaced or deleted while it was
    /// reported is held for the decision, and not gone.
    pub gone: bool,
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
    /// `remove` takes the picture down, or withdraws the text of a reader's work or edition;
    /// `dismiss` keeps it.
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

/// Reports a cover (`cover_public_id` of a book or edition) or a profile picture (the public id
/// in an `avatar_url`): copyright, the wrong book, offensive, or something else. A profile
/// picture can be reported by whoever can see its owner (they share a club), else it is 404
/// like an unknown picture. 20 new reports per day, and 30 requests per minute, repeats
/// included. Idempotent per reader and picture: a repeat answers with the reader's open report,
/// and a picture already taken down gets a closed record (`status` `removed`); neither counts
/// against the daily limit again.
#[utoipa::path(
    post,
    path = "/api/media/{public_id}/reports",
    operation_id = "media_report",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("public_id" = String, Path, description = "The picture's public id: a cover's `cover_public_id`, or the id in an `avatar_url`")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or the picture was already taken down (`removed`); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`: no such picture, or the profile picture of someone the reader cannot see"),
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
    // A profile picture only when the reader can see its owner (`bc_can_see_user`: they share a
    // club, or it is their own); otherwise it is as unknown as a picture that does not exist.
    let found = sqlx::query!(
        r#"SELECT m.id, m.kind, m.uploaded_by, m.removed_at IS NOT NULL AS "removed!",
                  (SELECT r.id FROM reports r
                   WHERE r.created_by = $2 AND r.media_id = m.id
                     AND r.status = CASE WHEN m.removed_at IS NULL THEN 'open' ELSE 'removed' END
                   ORDER BY r.id DESC LIMIT 1) AS earlier
           FROM media m
           WHERE m.public_id = $1
             AND (m.kind = 'cover' OR (m.kind = 'avatar' AND bc_can_see_user($2, m.uploaded_by)))"#,
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
    let avatar = found.kind == "avatar";
    // Whose profile picture it is: the report keeps the owner when the picture row goes.
    let target_user = if avatar { found.uploaded_by } else { None };
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
    // A profile picture's owner before the picture, as the owner's avatar writes and account
    // deletion lock them (conventions §7.2).
    lock_owner(&mut tx, target_user).await?;
    // Then the picture (conventions §7.2), FOR SHARE: a takedown or a dismissal that is under
    // way finishes first and this report sees it; a later one waits and then resolves it.
    // A profile picture its owner replaced or deleted since the plain read, while no report of
    // it was open, has no row any more.
    let removed = sqlx::query_scalar!(
        r#"SELECT removed_at IS NOT NULL AS "removed!" FROM media WHERE id = $1 FOR SHARE"#,
        media_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
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
    let (work_id, edition_id) = if avatar {
        (None, None)
    } else {
        shown_on(&mut tx, media_id).await?
    };
    // A picture taken down already: the notice is recorded, closed on arrival (no admin decided
    // it, so `resolved_by` stays null).
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, media_id, work_id, edition_id, target_user_id, reason,
                              details, created_by, status, resolved_at, closed_on_arrival)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CASE WHEN $10 THEN now() END, $10)
         RETURNING id",
        found.kind,
        media_id,
        work_id,
        edition_id,
        target_user,
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

/// 201 with the report (`catalog::reports` answers the same).
pub(crate) fn created(id: i64, status: &str) -> (StatusCode, Json<ReportCreated>) {
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

/// Decides a report: `remove` takes the reported picture down (everywhere it is shown, for
/// good) or withdraws the text of a reported work or edition a reader added; `dismiss` keeps
/// it. Either way every open report of the same picture, work or edition is resolved with it.
/// After the commit each reporter is mailed the decision, and on a removal the uploader, the
/// avatar's owner or the reader who added the record gets a statement of reasons. A profile
/// picture its owner replaced or deleted while it was reported is held for the decision:
/// "remove" takes it down, "dismiss" deletes it. A report of something gone (`ReportOut.gone`:
/// the profile picture of an account deleted since, a work merged away) is only dismissed, a
/// merged work's with the other open reports of that work, a deleted account's picture's on its
/// own; its reporters learn that it is no longer online.
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
        (status = 400, body = ErrorBody, description = "`Input should be 'remove' or 'dismiss'`; on `remove`, `Only books readers added can be reported` (a work or edition from another source)"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`"),
        (status = 409, body = ErrorBody, description = "`This report was already resolved`; on `remove`, `The reported picture is no longer online` (the profile picture of an account deleted since) or `The reported book was merged into another one`")
    )
)]
pub async fn resolve_report(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(report_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportResolveIn>,
) -> ApiResult<Json<ReportOut>> {
    let report = sqlx::query!(
        "SELECT target_kind, media_id, work_id, edition_id, target_work_id, target_user_id,
                status
         FROM reports WHERE id = $1",
        report_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if report.status != "open" {
        return Err(AppError::conflict(REPORT_RESOLVED));
    }
    let remove = body.action == "remove";
    let status = if remove { "removed" } else { "dismissed" };
    let avatar = report.target_kind == "avatar";
    let mut tx = state.db.begin().await?;
    let mut prefixes = Vec::new();
    let decision = match report.target_kind.as_str() {
        "work" | "edition" => {
            let target = match (
                report.target_kind.as_str(),
                report.target_work_id,
                report.edition_id,
            ) {
                // The work as reported, not the one a merge re-pointed `work_id` to.
                ("work", Some(work_id), _) => Some(CatalogTarget::Work(work_id)),
                ("edition", _, Some(id)) => Some(CatalogTarget::Edition {
                    id,
                    work_id: report.work_id.unwrap_or_default(),
                }),
                _ => None,
            };
            let decided = match target {
                Some(target) => catalog_reports::decide(&mut tx, target, remove).await?,
                None => None,
            };
            decided.map(|decided| Decision {
                title: Some(decided.title),
                owner: decided.owner,
                removed: remove && decided.changed,
                gone: decided.merged,
                held: false,
                earlier: false,
            })
        }
        _ => {
            let title = book_title(&mut tx, report.work_id, report.edition_id).await?;
            // A profile picture's owner first (conventions §7.2): the takedown below also locks
            // the owner's copies of the picture, which an avatar upload locks after the owner.
            lock_owner(&mut tx, report.target_user_id.filter(|_| avatar)).await?;
            // Then the picture, as a takedown and a report lock it: dismissals and takedowns
            // of one picture take turns, and a report filed meanwhile is resolved too. The
            // profile picture of an account deleted since has no row any more.
            let present = match report.media_id {
                Some(media_id) => lock_picture(&mut tx, media_id, remove).await?,
                None => false,
            };
            let gone = avatar && !present;
            if gone && remove {
                return Err(AppError::conflict(PICTURE_GONE));
            }
            // A profile picture its owner replaced or deleted while it was reported, held for
            // this decision (`media/avatar.rs`): no account shows it any more.
            let held = match report.media_id.filter(|_| avatar && present) {
                Some(media_id) => held_avatar(&mut tx, media_id).await?,
                None => false,
            };
            Some(match report.media_id.filter(|_| present) {
                // Resolves the picture's open reports, this one included.
                Some(media_id) if remove => {
                    prefixes =
                        take_down_locked(&mut tx, state.media.as_ref(), media_id, admin.id).await?;
                    let owner = uploader(&mut tx, media_id).await?;
                    Decision {
                        title,
                        owner,
                        removed: !prefixes.is_empty(),
                        gone,
                        held,
                        earlier: avatar && !took_current_avatar(&mut tx, owner).await?,
                    }
                }
                _ => Decision {
                    title,
                    owner: None,
                    removed: false,
                    gone,
                    held,
                    earlier: false,
                },
            })
        }
    };
    // This report, and every other open report of the same picture, work or edition. A gone
    // profile picture has no `media_id`, and its account's deletion cleared `target_user_id`
    // too, so its report is closed alone.
    sqlx::query!(
        "UPDATE reports SET status = $3, resolved_by = $2, resolved_at = now()
         WHERE status = 'open'
           AND (id = $1 OR media_id = $4
                OR (target_kind = 'work' AND $5 = 'work' AND target_work_id = $6)
                OR (target_kind = 'edition' AND $5 = 'edition' AND edition_id = $7))",
        report_id,
        admin.id,
        status,
        report.media_id,
        report.target_kind,
        report.target_work_id,
        report.edition_id
    )
    .execute(&mut *tx)
    .await?;
    // A held profile picture that stays dismissed goes now, with its files after the commit; its
    // reports keep `target_user_id` and become `gone`.
    let held = decision.as_ref().is_some_and(|decision| decision.held);
    if let Some(media_id) = report.media_id.filter(|_| held && !remove) {
        prefixes = delete_held_avatar(&mut tx, media_id).await?;
    }
    // Another admin may have resolved it since it was read; then this answer changes nothing
    // (the rollback undoes a takedown too, and sends no notice).
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
    let what = decision.as_ref().and_then(|decision| {
        let kind = if decision.earlier {
            "earlier_avatar"
        } else {
            report.target_kind.as_str()
        };
        reported(kind, decision.title.as_deref())
    });
    let notices = match (&decision, what) {
        (Some(decision), Some(what)) => {
            let removal = decision.removed.then_some(Removal {
                owner: decision.owner,
            });
            // A held picture that stays dismissed is no longer online either.
            notices::decided(
                &mut tx,
                &state.config,
                admin.id,
                what,
                removal,
                decision.gone || (decision.held && !remove),
            )
            .await?
        }
        // The work or edition is gone: nothing left to name.
        _ => Notices::default(),
    };
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    notices.send(&state).await;
    let mut conn = state.db.acquire().await?;
    let out = report_outs(&mut conn, &[report_id])
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    Ok(Json(out))
}

/// What a decision found, for its notices.
struct Decision {
    /// The reported book's title (a cover's book, a work or edition as it was before).
    title: Option<String>,
    /// On a removal, whose content it was (the uploader, the avatar's owner, the reader who
    /// added the record).
    owner: Option<i64>,
    /// The decision removed something, so the owner gets a statement of reasons.
    removed: bool,
    /// What was reported no longer shows as reported (`ReportOut.gone`).
    gone: bool,
    /// A profile picture its owner replaced or deleted while it was reported, kept for this
    /// decision (`media/avatar.rs`).
    held: bool,
    /// A profile picture taken down that is not the owner's current one (a held picture, its
    /// copy not taken down with it): the statement names it as an earlier picture.
    earlier: bool,
}

/// Whether this transaction took down the profile picture `owner` shows now. False after the
/// takedown of a held picture: the owner had replaced or deleted it, and their current picture,
/// if any, stays (unless it was a copy of the same file, which the takedown takes down too).
async fn took_current_avatar(conn: &mut PgConnection, owner: Option<i64>) -> sqlx::Result<bool> {
    let Some(owner) = owner else {
        return Ok(false);
    };
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM users u JOIN media m ON m.id = u.avatar_media_id
                          WHERE u.id = $1 AND m.removed_at = now()) AS "taken!""#,
        owner
    )
    .fetch_one(conn)
    .await
}

/// Locks the owner of a profile picture (`FOR KEY SHARE`) before the picture, as every path that
/// touches a profile picture does (conventions §7.2). The owner's avatar upload or deletion and
/// the account's deletion lock the users row `FOR UPDATE` and then the avatar rows; a reports
/// row that names the owner (`target_user_id`) takes this lock anyway through its foreign key,
/// and a profile picture's takedown locks the owner's copies of it. Taken only after the picture,
/// either could wait for the owner's row while the owner's transaction waited for the picture,
/// and Postgres would abort one of them. An account deleted meanwhile has no row: its pictures
/// are gone too, which the caller's read of the picture finds.
async fn lock_owner(conn: &mut PgConnection, owner: Option<i64>) -> sqlx::Result<()> {
    if let Some(owner) = owner {
        sqlx::query!("SELECT id FROM users WHERE id = $1 FOR KEY SHARE", owner)
            .fetch_optional(conn)
            .await?;
    }
    Ok(())
}

/// True when the profile picture `media_id` is no account's profile picture any more: its owner
/// replaced or deleted it while it was reported, and `media/avatar.rs` kept it for the decision.
async fn held_avatar(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT NOT EXISTS (SELECT 1 FROM users WHERE avatar_media_id = $1) AS "held!""#,
        media_id
    )
    .fetch_one(conn)
    .await
}

/// Deletes a held profile picture (locked by the caller) whose reports were dismissed, unless a
/// report of it is still open or it was taken down meanwhile. Returns its storage prefixes, to
/// delete after the commit.
async fn delete_held_avatar(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<Vec<String>> {
    let row = sqlx::query!(
        "DELETE FROM media m
         WHERE m.id = $1 AND m.kind = 'avatar' AND m.removed_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM users u WHERE u.avatar_media_id = m.id)
           AND NOT EXISTS (SELECT 1 FROM reports r WHERE r.media_id = m.id AND r.status = 'open')
         RETURNING m.kind, m.public_id, m.storage_key",
        media_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row
        .map(|row| {
            let mut prefixes = vec![storage_prefix(&row.kind, &row.public_id)];
            prefixes.extend(row.storage_key);
            prefixes
        })
        .unwrap_or_default())
}

/// Locks a reported picture's row for a decision, as a takedown (`FOR UPDATE`, on `remove`) or
/// a dismissal (`FOR NO KEY UPDATE`) locks it. False when the row is gone: the profile picture
/// of an account deleted since.
async fn lock_picture(conn: &mut PgConnection, media_id: i64, remove: bool) -> sqlx::Result<bool> {
    let row = if remove {
        sqlx::query_scalar!("SELECT id FROM media WHERE id = $1 FOR UPDATE", media_id)
            .fetch_optional(conn)
            .await?
    } else {
        sqlx::query_scalar!(
            "SELECT id FROM media WHERE id = $1 FOR NO KEY UPDATE",
            media_id
        )
        .fetch_optional(conn)
        .await?
    };
    Ok(row.is_some())
}

/// What a decision's notices name: a cover (with its book's title), a profile picture (an
/// earlier one: `earlier_avatar`), or a work or edition by its title before the decision. `None`
/// for a work or edition that is gone.
fn reported<'a>(kind: &str, title: Option<&'a str>) -> Option<Reported<'a>> {
    match kind {
        "avatar" => Some(Reported::Avatar),
        "earlier_avatar" => Some(Reported::EarlierAvatar),
        "work" => title.map(Reported::Work),
        "edition" => title.map(Reported::Edition),
        _ => Some(Reported::Cover(title)),
    }
}

/// The title of the book a report names (an edition's own title first), if it still exists.
async fn book_title(
    conn: &mut PgConnection,
    work_id: Option<i64>,
    edition_id: Option<i64>,
) -> sqlx::Result<Option<String>> {
    let Some(work_id) = work_id else {
        return Ok(None);
    };
    sqlx::query_scalar!(
        r#"SELECT coalesce(e.title, w.title) AS "title!"
           FROM works w LEFT JOIN editions e ON e.id = $2 AND e.work_id = w.id
           WHERE w.id = $1"#,
        work_id,
        edition_id
    )
    .fetch_optional(conn)
    .await
}

/// Who uploaded a picture (an avatar's owner); none for Open Library covers.
async fn uploader(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<Option<i64>> {
    Ok(
        sqlx::query_scalar!("SELECT uploaded_by FROM media WHERE id = $1", media_id)
            .fetch_optional(conn)
            .await?
            .flatten(),
    )
}

/// Takes a picture down directly, without a report: same effect as resolving one with
/// `remove`. Taking down a removed picture changes nothing. With a body (a notice that arrived
/// by email, D2), the takedown is recorded as a resolved `removed` report of the picture (a
/// cover or a profile picture, `via` `notice`): `created_at` is when the notice arrived,
/// `created_by` is null (the sender has no account here), `resolved_by` the admin. A picture
/// that was down already gets the record too, closed when it came down (or on arrival, with no
/// `resolved_by`, when it was down before the notice arrived). After the commit the reporters
/// of the open reports it resolved are mailed the decision, and the uploader or the avatar's
/// owner a statement of reasons.
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
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `Input should be in the past` (`received_at`), `Invalid value`"),
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
    let found = sqlx::query!(
        "SELECT id, kind, uploaded_by FROM media WHERE public_id = $1",
        public_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    let media_id = found.id;
    let mut tx = state.db.begin().await?;
    // A profile picture's owner first (conventions §7.2): the notice names the owner, and the
    // takedown locks the owner's copies of the picture.
    lock_owner(
        &mut tx,
        found.uploaded_by.filter(|_| found.kind == "avatar"),
    )
    .await?;
    // Then the picture (conventions §7.2), and the book that shows it before the takedown
    // clears its covers.
    let picture = sqlx::query!(
        "SELECT kind, removed_at, uploaded_by FROM media WHERE id = $1 FOR UPDATE",
        media_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    let avatar = picture.kind == "avatar";
    let shown = if avatar {
        (None, None)
    } else {
        shown_on(&mut tx, media_id).await?
    };
    let title = book_title(&mut tx, shown.0, shown.1).await?;
    let notice_reason = notice
        .as_ref()
        .map(|notice| notice.reason.as_deref().unwrap_or("other"));
    let prefixes = take_down_answering(
        &mut tx,
        state.media.as_ref(),
        media_id,
        admin.id,
        notice_reason,
    )
    .await?;
    if let Some(notice) = &notice {
        let target = Picture {
            media_id,
            kind: &picture.kind,
            owner: picture.uploaded_by.filter(|_| avatar),
            shown,
        };
        record_notice(&mut tx, target, notice, picture.removed_at, admin.id).await?;
    }
    let what = if !avatar {
        Reported::Cover(title.as_deref())
    } else if took_current_avatar(&mut tx, picture.uploaded_by).await? {
        Reported::Avatar
    } else {
        // A held picture (by the id the admin queue shows), or one taken down before.
        Reported::EarlierAvatar
    };
    // Only a picture that was up gets a statement: a removed one lost nothing now.
    let removal = (!prefixes.is_empty()).then_some(Removal {
        owner: picture.uploaded_by,
    });
    let notices = notices::decided(&mut tx, &state.config, admin.id, what, removal, false).await?;
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    notices.send(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The picture a notice is recorded for: its kind (`cover` or `avatar`), the avatar's owner,
/// and the book that showed a cover.
struct Picture<'a> {
    media_id: i64,
    kind: &'a str,
    owner: Option<i64>,
    shown: (Option<i64>, Option<i64>),
}

/// The record of a direct takedown ([`take_down`] with a body): a `removed` report (`via`
/// `notice`) that arrived at `received_at` and was resolved by `admin_id` now, or, when the
/// picture was down already (`removed_at`), when it came down; a notice that arrived after that
/// was closed on arrival (no `resolved_by`, like a reader's report of a removed picture).
async fn record_notice(
    conn: &mut PgConnection,
    picture: Picture<'_>,
    notice: &TakeDownIn,
    removed_at: Option<OffsetDateTime>,
    admin_id: i64,
) -> sqlx::Result<()> {
    let (work_id, edition_id) = picture.shown;
    sqlx::query!(
        "INSERT INTO reports (target_kind, media_id, work_id, edition_id, target_user_id, reason,
                              details, created_by, created_at, status, resolved_by, resolved_at,
                              via, closed_on_arrival)
         SELECT $9, $1, $2, $3, $10, $4, $5, NULL, received, 'removed',
                CASE WHEN removed IS NULL OR removed > received THEN $7::int8 END,
                CASE WHEN removed IS NULL THEN now() ELSE greatest(removed, received) END,
                'notice', NOT (removed IS NULL OR removed > received)
         FROM (SELECT least(coalesce($6, now()), now()) AS received,
                      $8::timestamptz AS removed) AS t",
        picture.media_id,
        work_id,
        edition_id,
        notice.reason.as_deref().unwrap_or("other"),
        notice.details.as_deref().unwrap_or_default(),
        notice.received_at,
        admin_id,
        removed_at,
        picture.kind,
        picture.owner
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The takedown inside the caller's transaction (see the module docs): locks the media row,
/// records the hashes it must not come back under, sets `removed_at` and clears `storage_key`,
/// clears every cover that shows it, resolves the picture's open reports as `removed` by
/// `admin_id`, and rejects its open `cover_replace` requests with [`TAKEN_DOWN_NOTE`]. A
/// profile picture takes the owner's other live profile pictures with the same bytes as received
/// down with it: the same file uploaded again while this one was reported and held (security
/// review, Phase 2 Wave 2), which the hash block, a check on new uploads only, would leave up.
/// A cover takes its copies down with it ([`cover_copies`]; see the module docs), judged by the
/// picture's open reports ([`judged`]). Returns the storage prefixes to delete after the commit
/// (empty when the picture was removed already).
pub async fn take_down_locked(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
    media_id: i64,
    admin_id: i64,
) -> ApiResult<Vec<String>> {
    take_down_answering(conn, store, media_id, admin_id, None).await
}

/// [`take_down_locked`] answering a notice with this reason as well as the open reports.
async fn take_down_answering(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
    media_id: i64,
    admin_id: i64,
    notice_reason: Option<&str>,
) -> ApiResult<Vec<String>> {
    let judged = judged(conn, media_id, notice_reason).await?;
    // Locked before the takedown clears any cover (media rows before works).
    let cover_copies = cover_copies(conn, media_id, judged).await?;
    let (mut prefixes, copies_of) = take_down_one(conn, store, media_id, admin_id, judged).await?;
    for copy in cover_copies {
        prefixes.extend(take_down_one(conn, store, copy, admin_id, judged).await?.0);
    }
    if let Some((owner, sha256)) = copies_of {
        let copies = sqlx::query_scalar!(
            "SELECT id FROM media
             WHERE kind = 'avatar' AND removed_at IS NULL AND uploaded_by = $1 AND sha256 = $2
               AND id <> $3
             ORDER BY id
             FOR UPDATE",
            owner,
            sha256,
            media_id
        )
        .fetch_all(&mut *conn)
        .await?;
        for copy in copies {
            prefixes.extend(take_down_one(conn, store, copy, admin_id, judged).await?.0);
        }
    }
    Ok(prefixes)
}

/// What a takedown judged (#74, security review of the cover search).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judged {
    /// The picture itself: offensive, a copyright matter, another reason, or none given. A
    /// cover's bytes are refused as a cover anywhere from now on (`blocks_covers`).
    Picture,
    /// Only that the cover shows another book: every report or notice behind the takedown said
    /// `wrong_book`. The picture may be right for its own book, so Open Library fetches of it
    /// are not refused, and only its copies on the same book go with it.
    WrongBook,
}

/// What the takedown of `media_id` judges: [`Judged::WrongBook`] when every reason behind it
/// (the picture's open reports, which the takedown resolves, and a notice's) is `wrong_book`,
/// else [`Judged::Picture`], also when no reason is given.
async fn judged(
    conn: &mut PgConnection,
    media_id: i64,
    notice: Option<&str>,
) -> sqlx::Result<Judged> {
    let mut reasons = sqlx::query_scalar!(
        "SELECT reason FROM reports WHERE media_id = $1 AND status = 'open'",
        media_id
    )
    .fetch_all(conn)
    .await?;
    reasons.extend(notice.map(str::to_owned));
    Ok(
        if !reasons.is_empty() && reasons.iter().all(|reason| reason == "wrong_book") {
            Judged::WrongBook
        } else {
            Judged::Picture
        },
    )
}

/// The live covers a cover's takedown takes down with it, locked `FOR UPDATE` in id order: the
/// same bytes as received, uploaded by the same reader or from Open Library (no uploader); with
/// [`Judged::WrongBook`] only those that the same book (a work showing the picture, or the work
/// of an edition showing it) shows on itself or an edition. None for a profile picture, or a
/// picture already removed.
async fn cover_copies(
    conn: &mut PgConnection,
    media_id: i64,
    judged: Judged,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT c.id AS "id!" FROM media m JOIN media c ON c.sha256 = m.sha256 AND c.id <> m.id
           WHERE m.id = $1 AND m.kind = 'cover' AND m.removed_at IS NULL
             AND c.kind = 'cover' AND c.removed_at IS NULL
             AND (c.uploaded_by IS NULL OR c.uploaded_by = m.uploaded_by)
             AND ($2 OR c.id IN (
                 SELECT w.cover_media_id FROM works w WHERE w.id IN (
                     SELECT b.id FROM works b WHERE b.cover_media_id = $1
                     UNION SELECT e.work_id FROM editions e WHERE e.cover_media_id = $1)
                 UNION
                 SELECT e.cover_media_id FROM editions e WHERE e.work_id IN (
                     SELECT b.id FROM works b WHERE b.cover_media_id = $1
                     UNION SELECT e.work_id FROM editions e WHERE e.cover_media_id = $1)))
           ORDER BY c.id
           FOR UPDATE OF c"#,
        media_id,
        judged == Judged::Picture
    )
    .fetch_all(conn)
    .await
}

/// [`take_down_locked`] of one picture. Also returns, for a profile picture this call took down,
/// its owner and its hash as received, to find the copies.
async fn take_down_one(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
    media_id: i64,
    admin_id: i64,
    judged: Judged,
) -> ApiResult<(Vec<String>, Option<(i64, Vec<u8>)>)> {
    let Some(row) = sqlx::query!(
        r#"SELECT kind, public_id, storage_key, variants, sha256, uploaded_by,
                  removed_at IS NOT NULL AS "removed!"
           FROM media WHERE id = $1 FOR UPDATE"#,
        media_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok((Vec::new(), None));
    };
    let copies_of = match (&row.uploaded_by, &row.sha256) {
        (Some(owner), Some(sha256)) if row.kind == "avatar" && !row.removed => {
            Some((*owner, sha256.clone()))
        }
        _ => None,
    };
    let mut prefixes = Vec::new();
    if !row.removed {
        let stored = match &row.storage_key {
            Some(prefix) => stored_variants(store, prefix, &row.variants).await?,
            None => Vec::new(),
        };
        let blocks_covers = row.kind == "cover" && judged == Judged::Picture;
        record_removed_hashes(conn, media_id, row.sha256, stored, blocks_covers).await?;
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
    Ok((prefixes, copies_of))
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
/// and fetching them now would download the removed picture again. `blocks_covers`: the
/// takedown judged a cover's picture, so Open Library fetches and the cover search refuse the
/// bytes too (a hash recorded once without it gains it from a later such takedown).
async fn record_removed_hashes(
    conn: &mut PgConnection,
    media_id: i64,
    original: Option<Vec<u8>>,
    stored: Vec<(String, Bytes)>,
    blocks_covers: bool,
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
        "INSERT INTO media_removed_hashes (sha256, media_id, variant, blocks_covers)
         SELECT DISTINCT ON (sha256) sha256, $1, variant, $4
         FROM UNNEST($2::bytea[], $3::text[]) WITH ORDINALITY AS h (sha256, variant, n)
         ORDER BY sha256, n
         ON CONFLICT (sha256) DO UPDATE SET
             blocks_covers = media_removed_hashes.blocks_covers OR EXCLUDED.blocks_covers",
        media_id,
        &hashes,
        &variants as &[Option<String>],
        blocks_covers
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The [`ReportOut`]s of `ids`, in that order (missing ids are left out).
pub async fn report_outs(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<Vec<ReportOut>> {
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!", r.target_kind AS "target_kind!", r.reason AS "reason!",
                  r.details AS "details!", r.status AS "status!", r.via AS "via!", r.work_id,
                  r.edition_id, r.target_user_id, r.created_by, r.created_at AS "created_at!",
                  r.resolved_by, r.resolved_at, r.closed_on_arrival AS "closed_on_arrival!",
                  m.public_id AS "public_id?", m.removed_at IS NOT NULL AS "removed?",
                  CASE r.target_kind
                      WHEN 'avatar' THEN r.media_id IS NULL
                      WHEN 'work' THEN r.target_work_id IS DISTINCT FROM r.work_id
                      ELSE false
                  END AS "gone!"
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
        .flat_map(|row| [row.created_by, row.resolved_by, row.target_user_id])
        .flatten()
        .collect();
    let users = user_summaries(conn, &user_ids).await?;
    let mut by_id: HashMap<i64, ReportOut> = rows
        .into_iter()
        .map(|row| {
            let removed = row.removed.unwrap_or(false);
            let shown = row.public_id.as_deref().filter(|_| !removed);
            let avatar = row.target_kind == "avatar";
            let (cover_url, cover_thumb_url) = cover_urls(shown.filter(|_| !avatar));
            let avatar_url = avatar_url(shown.filter(|_| avatar));
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
                target_kind: row.target_kind,
                reason: row.reason,
                details: row.details,
                status: row.status,
                via: row.via,
                media_public_id: row.public_id,
                removed,
                cover_url,
                cover_thumb_url,
                avatar_url,
                target_user: row.target_user_id.and_then(|id| users.get(&id).cloned()),
                book,
                created_by: row.created_by.and_then(|id| users.get(&id).cloned()),
                created_at: row.created_at,
                resolved_by: row.resolved_by.and_then(|id| users.get(&id).cloned()),
                resolved_at: row.resolved_at,
                closed_on_arrival: row.closed_on_arrival,
                gone: row.gone,
            };
            (row.id, out)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}
