//! The minimal admin queue of catalog requests and the direct merge (endpoint-map §4; #54
//! builds the UI). Every route sits under `/api/admin/`, behind the `InstanceAdmin` route layer.
//!
//! Approving applies a request through the sanctioned writers only: `ingest::apply_admin_fields`
//! for edits (provenance `admin`, so no source overwrites them), `ingest::set_cover(…, Admin)`
//! for a `cover_replace`, and [`super::merge_works`] for a `work_merge`.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::detail::{BookDetailOut, book_detail, require_local_work};
use super::merge::merge_works;
use super::requests::MAX_NOTE_CHARS;
use super::shared::books::{BookRef, book_outs, follow_merges, require_work};
use super::shared::ingest::{CoverTarget, Source, apply_admin_fields, set_cover};
use super::{MERGE_INTO_ITSELF, PICTURE_REMOVED, REQUEST_DECIDED};
use crate::AppState;
use crate::auth::extract::InstanceAdmin;
use crate::dto::{BookOut, UserSummary, user_summaries};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};
use crate::media::shared::urls::cover_urls;
use crate::text::{py_strip, truncate_chars};

/// One request of the catalog queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditRequestOut {
    pub id: i64,
    /// `work_edit`, `edition_edit`, `cover_replace` or `work_merge`.
    pub kind: String,
    /// `open`, `approved` or `rejected`.
    pub status: String,
    /// The work (for `work_merge`: the work to merge away).
    pub work_id: Option<i64>,
    pub edition_id: Option<i64>,
    /// The work as it is now, in the edition's context when the request names one.
    pub book: Option<BookOut>,
    /// `work_edit`/`edition_edit`: the proposed fields; `work_merge`: `{"into_work_id": n}`;
    /// `cover_replace`: `{}`.
    #[schema(value_type = Object)]
    pub payload: Value,
    /// `cover_replace`: the proposed picture (null once it was taken down).
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    pub note: String,
    pub created_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub decided_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
    pub decision_note: String,
}

/// The open requests, oldest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditRequestList {
    pub items: Vec<EditRequestOut>,
}

/// Why a request is rejected.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EditRequestRejectIn {
    /// At most 2000 characters.
    #[serde(default)]
    pub note: String,
}

impl Validate for EditRequestRejectIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.note = truncate_chars(py_strip(&self.note), MAX_NOTE_CHARS).to_owned();
    }
}

/// A direct merge.
#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkMergeIn {
    /// The work that stays.
    pub into_work_id: i64,
}

impl Validate for WorkMergeIn {}

/// The [`EditRequestOut`]s of `ids`, in that order (missing ids are left out).
pub async fn edit_request_outs(
    conn: &mut PgConnection,
    ids: &[i64],
) -> sqlx::Result<Vec<EditRequestOut>> {
    let rows = sqlx::query!(
        r#"SELECT r.id, r.kind, r.status, r.work_id, r.edition_id, r.payload, r.note,
                  r.created_by, r.created_at, r.decided_by, r.decided_at, r.decision_note,
                  m.public_id AS "cover?"
           FROM catalog_edit_requests r
           LEFT JOIN media m ON m.id = r.media_id AND m.removed_at IS NULL
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
    let mut books = book_outs(conn, &refs).await?;
    let user_ids: Vec<i64> = rows
        .iter()
        .flat_map(|row| [row.created_by, row.decided_by])
        .flatten()
        .collect();
    let users = user_summaries(conn, &user_ids).await?;
    let mut by_id: HashMap<i64, EditRequestOut> = rows
        .into_iter()
        .map(|row| {
            let book = row.work_id.and_then(|work_id| {
                books.remove(&BookRef {
                    work_id,
                    edition_id: row.edition_id,
                })
            });
            let (cover_url, cover_thumb_url) = cover_urls(row.cover.as_deref());
            let out = EditRequestOut {
                id: row.id,
                kind: row.kind,
                status: row.status,
                work_id: row.work_id,
                edition_id: row.edition_id,
                book,
                payload: row.payload,
                cover_url,
                cover_thumb_url,
                cover_public_id: row.cover,
                note: row.note,
                created_by: row.created_by.and_then(|id| users.get(&id).cloned()),
                created_at: row.created_at,
                decided_by: row.decided_by.and_then(|id| users.get(&id).cloned()),
                decided_at: row.decided_at,
                decision_note: row.decision_note,
            };
            (row.id, out)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

async fn edit_request_out(conn: &mut PgConnection, id: i64) -> ApiResult<EditRequestOut> {
    edit_request_outs(conn, &[id])
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))
}

/// An open request, locked by the caller's transaction.
struct OpenRequest {
    kind: String,
    work_id: Option<i64>,
    edition_id: Option<i64>,
    media_id: Option<i64>,
    payload: Value,
}

/// The open request `id`, locked: 404 `Not found` when missing, 409 when already decided.
async fn lock_open_request(conn: &mut PgConnection, id: i64) -> ApiResult<OpenRequest> {
    let row = sqlx::query!(
        "SELECT kind, status, work_id, edition_id, media_id, payload
         FROM catalog_edit_requests WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if row.status != "open" {
        return Err(AppError::conflict(REQUEST_DECIDED));
    }
    Ok(OpenRequest {
        kind: row.kind,
        work_id: row.work_id,
        edition_id: row.edition_id,
        media_id: row.media_id,
        payload: row.payload,
    })
}

/// The two current works of a merge: 404 `No such book.` for either, 400 when they are one.
async fn merge_pair(conn: &mut PgConnection, loser: i64, winner: i64) -> ApiResult<(i64, i64)> {
    let loser = require_work(conn, loser).await?;
    let winner = require_work(conn, winner).await?;
    if loser == winner {
        return Err(AppError::bad_request(MERGE_INTO_ITSELF));
    }
    Ok((loser, winner))
}

/// Applies an open request (locked by the caller's transaction).
async fn apply_request(conn: &mut PgConnection, request: &OpenRequest) -> ApiResult<()> {
    let OpenRequest {
        kind,
        work_id,
        edition_id,
        media_id,
        payload,
    } = request;
    let (work_id, edition_id, media_id) = (*work_id, *edition_id, *media_id);
    let fields = payload.as_object().cloned().unwrap_or_default();
    match kind.as_str() {
        "work_edit" => {
            let work = match work_id {
                Some(id) => follow_merges(conn, id).await?,
                None => None,
            }
            .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            apply_admin_fields(conn, CoverTarget::Work(work), &fields).await
        }
        "edition_edit" => {
            let edition = edition_id.ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            apply_admin_fields(conn, CoverTarget::Edition(edition), &fields).await
        }
        "cover_replace" => {
            let media_id = media_id.ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
            let usable = sqlx::query_scalar!(
                r#"SELECT removed_at IS NULL AS "usable!" FROM media WHERE id = $1"#,
                media_id
            )
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false);
            if !usable {
                return Err(AppError::bad_request(PICTURE_REMOVED));
            }
            let target = match (edition_id, work_id) {
                (Some(edition), _) => CoverTarget::Edition(edition),
                (None, Some(work)) => CoverTarget::Work(
                    follow_merges(conn, work)
                        .await?
                        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?,
                ),
                (None, None) => return Err(AppError::not_found(msg::NO_SUCH_BOOK)),
            };
            set_cover(conn, target, media_id, Source::Admin, false).await?;
            Ok(())
        }
        "work_merge" => {
            let into = fields
                .get("into_work_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            let loser = work_id.ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            let (loser, winner) = merge_pair(conn, loser, into).await?;
            merge_works(conn, loser, winner).await?;
            Ok(())
        }
        other => Err(AppError::internal(format!("unknown request kind {other}"))),
    }
}

/// The open catalog requests (edits, cover replacements, merges), oldest first.
#[utoipa::path(
    get,
    path = "/api/admin/edit-requests",
    operation_id = "catalog_admin_list_edit_requests",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = EditRequestList, description = "The open requests"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (not an instance admin)")
    )
)]
pub async fn list_edit_requests(
    InstanceAdmin(_admin): InstanceAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<EditRequestList>> {
    let mut conn = state.db.acquire().await?;
    let ids = sqlx::query_scalar!(
        "SELECT id FROM catalog_edit_requests WHERE status = 'open' ORDER BY created_at, id"
    )
    .fetch_all(&mut *conn)
    .await?;
    let items = edit_request_outs(&mut conn, &ids).await?;
    Ok(Json(EditRequestList { items }))
}

/// Approves a request and applies it: edits with provenance `admin` (no source overwrites them
/// afterwards), a cover replacement as the admin's cover, a merge through `merge_works`.
#[utoipa::path(
    post,
    path = "/api/admin/edit-requests/{request_id}/approve",
    operation_id = "catalog_admin_approve_edit_request",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("request_id" = i64, Path, description = "The request id")),
    responses(
        (status = 200, body = EditRequestOut, description = "The approved request"),
        (status = 400, body = ErrorBody, description = "`Unknown field: <key>`, `Invalid value for <key>`, `This picture was removed` or `A book cannot be merged into itself`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (no such request, or not an instance admin), or `No such book.`"),
        (status = 409, body = ErrorBody, description = "`This request was already decided`")
    )
)]
pub async fn approve_edit_request(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(request_id): ApiPath<i64>,
) -> ApiResult<Json<EditRequestOut>> {
    let mut conn = state.db.acquire().await?;
    let mut tx = conn.begin().await?;
    let request = lock_open_request(&mut tx, request_id).await?;
    // Decided first: a merge rejects the open requests on the work it merges away, this one
    // excepted.
    sqlx::query!(
        "UPDATE catalog_edit_requests
         SET status = 'approved', decided_by = $2, decided_at = now()
         WHERE id = $1",
        request_id,
        admin.id
    )
    .execute(&mut *tx)
    .await?;
    apply_request(&mut tx, &request).await?;
    tx.commit().await?;
    Ok(Json(edit_request_out(&mut conn, request_id).await?))
}

/// Rejects a request; nothing changes in the catalog.
#[utoipa::path(
    post,
    path = "/api/admin/edit-requests/{request_id}/reject",
    operation_id = "catalog_admin_reject_edit_request",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("request_id" = i64, Path, description = "The request id")),
    request_body = EditRequestRejectIn,
    responses(
        (status = 200, body = EditRequestOut, description = "The rejected request"),
        (status = 400, body = ErrorBody, description = "A malformed body"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (no such request, or not an instance admin)"),
        (status = 409, body = ErrorBody, description = "`This request was already decided`")
    )
)]
pub async fn reject_edit_request(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(request_id): ApiPath<i64>,
    ApiJson(body): ApiJson<EditRequestRejectIn>,
) -> ApiResult<Json<EditRequestOut>> {
    let mut conn = state.db.acquire().await?;
    let mut tx = conn.begin().await?;
    lock_open_request(&mut tx, request_id).await?;
    sqlx::query!(
        "UPDATE catalog_edit_requests
         SET status = 'rejected', decided_by = $2, decided_at = now(), decision_note = $3
         WHERE id = $1",
        request_id,
        admin.id,
        body.note
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(edit_request_out(&mut conn, request_id).await?))
}

/// Merges a book into another without a request: shelves, favourites, nominations and layouts
/// are reconciled, every reference moves to the book that stays, and the merged book's links
/// redirect there. Answers with the book that stays.
#[utoipa::path(
    post,
    path = "/api/admin/works/{work_id}/merge",
    operation_id = "catalog_admin_merge_work",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "The work to merge away: a work id, `OL…W`, or a legacy key")),
    request_body = WorkMergeIn,
    responses(
        (status = 200, body = BookDetailOut, description = "The work that stays"),
        (status = 400, body = ErrorBody, description = "`A book cannot be merged into itself`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.` (either book), or `Not found` (not an instance admin)")
    )
)]
pub async fn merge_work(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<WorkMergeIn>,
) -> ApiResult<Json<BookDetailOut>> {
    let mut conn = state.db.acquire().await?;
    let loser = require_local_work(&mut conn, &raw).await?;
    let mut tx = conn.begin().await?;
    let (loser, winner) = merge_pair(&mut tx, loser, body.into_work_id).await?;
    merge_works(&mut tx, loser, winner).await?;
    tx.commit().await?;
    Ok(Json(book_detail(&mut conn, admin.id, winner).await?))
}
