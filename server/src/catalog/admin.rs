//! The admin queue of catalog requests, the direct merge and the direct admin edit
//! (endpoint-map §4, #54). Every route sits under `/api/admin/`, behind the `InstanceAdmin` route
//! layer.
//!
//! Approving applies a request through the sanctioned writers only: `ingest::apply_admin_fields`
//! for edits (provenance `admin`, so no source overwrites them), `ingest::set_cover(…, Admin)`
//! for a `cover_replace`, and [`super::merge_works`] for a `work_merge`. The direct admin edit
//! (`PATCH /api/admin/works/{work_id}`, `PATCH /api/admin/editions/{edition_id}`) writes through
//! `apply_admin_fields` too, with the same checks and 400s as an approval.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::detail::{BookDetailOut, book_detail, require_local_work};
use super::merge::merge_works;
use super::requests::MAX_NOTE_CHARS;
use super::shared::books::{
    BookRef, book_outs, edition_outs, follow_merges, follow_merges_many, require_work,
};
use super::shared::ingest::{CoverTarget, Source, apply_admin_fields, set_cover};
use super::{MERGE_INTO_ITSELF, PICTURE_REMOVED, REQUEST_DECIDED};
use crate::AppState;
use crate::auth::extract::InstanceAdmin;
use crate::dto::{BookOut, EditionOut, UserSummary, user_summaries};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, FIELD_REQUIRED, Validate, Validator};
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
    /// `work_edit`/`edition_edit`: the record's value now for each proposed field, keyed like
    /// `payload` (an empty text is null, a list of subjects an array); `{}` for the other kinds
    /// and when the record is gone. A `work_edit` reads the work its book was merged into.
    #[schema(value_type = Object)]
    pub current: Value,
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

/// Fields an instance admin sets directly.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminEditIn {
    /// The new values, with the keys and rules of an edit request (`EditRequestIn.fields`).
    /// Works: `title`, `subtitle`, `authors`, `year`, `description`, `subjects`, `pages`.
    /// Editions: `title`, `subtitle`, `publisher`, `language`, `format`, `pages`,
    /// `publication_year`, `planned_publication_date` (`YYYY-MM-DD`). Null clears a field.
    #[schema(value_type = Object)]
    pub fields: Map<String, Value>,
}

impl Validate for AdminEditIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(!self.fields.is_empty(), FIELD_REQUIRED);
    }
}

/// A text as an edit request writes it: empty is null.
fn text_value(text: String) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    }
}

/// The present values of works and editions under the edit-request keys, by record id.
#[derive(Default)]
struct CurrentValues {
    works: HashMap<i64, Map<String, Value>>,
    editions: HashMap<i64, Map<String, Value>>,
    /// The current work of each requested work id (merges followed).
    merged: HashMap<i64, i64>,
}

impl CurrentValues {
    async fn load(
        conn: &mut PgConnection,
        work_ids: &[i64],
        edition_ids: &[i64],
    ) -> sqlx::Result<Self> {
        let merged = follow_merges_many(conn, work_ids).await?;
        let current: Vec<i64> = merged.values().copied().collect();
        let works = sqlx::query!(
            "SELECT id, title, subtitle, author_names, first_publish_year, description, subjects,
                    pages
             FROM works WHERE id = ANY($1)",
            &current
        )
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| {
            let mut values = Map::new();
            values.insert("title".into(), Value::String(row.title));
            values.insert("subtitle".into(), text_value(row.subtitle));
            values.insert("authors".into(), text_value(row.author_names));
            values.insert("year".into(), json!(row.first_publish_year));
            values.insert("description".into(), text_value(row.description));
            values.insert("subjects".into(), json!(row.subjects));
            values.insert("pages".into(), json!(row.pages));
            (row.id, values)
        })
        .collect();
        let editions = sqlx::query!(
            "SELECT id, title, subtitle, publisher, language, format, pages, publication_year,
                    planned_publication_date
             FROM editions WHERE id = ANY($1)",
            edition_ids
        )
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| {
            let mut values = Map::new();
            values.insert("title".into(), json!(row.title));
            values.insert("subtitle".into(), json!(row.subtitle));
            values.insert("publisher".into(), text_value(row.publisher));
            values.insert("language".into(), json!(row.language));
            values.insert("format".into(), json!(row.format));
            values.insert("pages".into(), json!(row.pages));
            values.insert("publication_year".into(), json!(row.publication_year));
            values.insert(
                "planned_publication_date".into(),
                json!(row.planned_publication_date),
            );
            (row.id, values)
        })
        .collect();
        Ok(CurrentValues {
            works,
            editions,
            merged,
        })
    }

    /// `EditRequestOut.current` of one request.
    fn of(
        &self,
        kind: &str,
        work_id: Option<i64>,
        edition_id: Option<i64>,
        payload: &Value,
    ) -> Value {
        let record = match kind {
            "work_edit" => work_id
                .and_then(|id| self.merged.get(&id))
                .and_then(|id| self.works.get(id)),
            "edition_edit" => edition_id.and_then(|id| self.editions.get(&id)),
            _ => None,
        };
        let (Some(record), Some(proposed)) = (record, payload.as_object()) else {
            return Value::Object(Map::new());
        };
        Value::Object(
            proposed
                .keys()
                .filter_map(|key| Some((key.clone(), record.get(key)?.clone())))
                .collect(),
        )
    }
}

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
    let books = book_outs(conn, &refs).await?;
    let edited_works: Vec<i64> = rows
        .iter()
        .filter(|row| row.kind == "work_edit")
        .filter_map(|row| row.work_id)
        .collect();
    let edited_editions: Vec<i64> = rows
        .iter()
        .filter(|row| row.kind == "edition_edit")
        .filter_map(|row| row.edition_id)
        .collect();
    let current = CurrentValues::load(conn, &edited_works, &edited_editions).await?;
    let user_ids: Vec<i64> = rows
        .iter()
        .flat_map(|row| [row.created_by, row.decided_by])
        .flatten()
        .collect();
    let users = user_summaries(conn, &user_ids).await?;
    let mut by_id: HashMap<i64, EditRequestOut> = rows
        .into_iter()
        .map(|row| {
            let current = current.of(&row.kind, row.work_id, row.edition_id, &row.payload);
            // Cloned, not removed: several requests can name the same work and edition.
            let book = row.work_id.and_then(|work_id| {
                books
                    .get(&BookRef {
                        work_id,
                        edition_id: row.edition_id,
                    })
                    .cloned()
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
                current,
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

/// Locks the picture of a `cover_replace` request `FOR SHARE`, before the request row. A takedown
/// locks the picture and then rejects the open requests that propose it, so an approval that
/// took the request row first would deadlock with it (conventions §7.2: media rows first). The
/// request's `media_id` never changes, so reading it without a lock is safe.
async fn lock_request_media(conn: &mut PgConnection, id: i64) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        "SELECT m.id FROM catalog_edit_requests r JOIN media m ON m.id = r.media_id
         WHERE r.id = $1 FOR SHARE OF m",
        id
    )
    .fetch_optional(conn)
    .await?;
    Ok(())
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

/// Merges a pair that [`merge_pair`] resolved without a lock. A concurrent merge may have
/// merged either work in the meantime: `merge_works` sees that under its lock and answers
/// `RowNotFound` (its savepoint is rolled back). The pair is then resolved again for the
/// answer it has become: 400 when both are one work now (A into B against B into A), else 404
/// `No such book.` (the work to merge away is gone).
async fn merge_resolved(conn: &mut PgConnection, loser: i64, winner: i64) -> ApiResult<()> {
    match merge_works(conn, loser, winner).await {
        Ok(_) => Ok(()),
        Err(sqlx::Error::RowNotFound) => {
            merge_pair(conn, loser, winner).await?;
            Err(AppError::not_found(msg::NO_SUCH_BOOK))
        }
        Err(error) => Err(error.into()),
    }
}

/// The current cover of `target`; `None` when the target does not exist.
async fn cover_of(
    conn: &mut PgConnection,
    target: CoverTarget,
) -> sqlx::Result<Option<Option<i64>>> {
    match target {
        CoverTarget::Work(id) => {
            sqlx::query_scalar!("SELECT cover_media_id FROM works WHERE id = $1", id)
                .fetch_optional(&mut *conn)
                .await
        }
        CoverTarget::Edition(id) => {
            sqlx::query_scalar!("SELECT cover_media_id FROM editions WHERE id = $1", id)
                .fetch_optional(&mut *conn)
                .await
        }
    }
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
            // Locked FOR SHARE before any catalog row (conventions §7.2): a takedown that is
            // under way finishes first and this check sees it; a later one waits for us.
            let usable = sqlx::query_scalar!(
                "SELECT id FROM media WHERE id = $1 AND removed_at IS NULL FOR SHARE",
                media_id
            )
            .fetch_optional(&mut *conn)
            .await?
            .is_some();
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
            if set_cover(conn, target, media_id, Source::Admin, false).await? {
                return Ok(());
            }
            // Not set: the target already shows this picture (fine), the target is gone, or
            // set_cover refused the media row (not a usable cover). The request stays open.
            match cover_of(conn, target).await? {
                Some(Some(current)) if current == media_id => Ok(()),
                Some(_) => Err(AppError::bad_request(PICTURE_REMOVED)),
                None => Err(AppError::not_found(msg::NO_SUCH_BOOK)),
            }
        }
        "work_merge" => {
            let into = fields
                .get("into_work_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            let loser = work_id.ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            let (loser, winner) = merge_pair(conn, loser, into).await?;
            merge_resolved(conn, loser, winner).await
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
    lock_request_media(&mut tx, request_id).await?;
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
    merge_resolved(&mut tx, loser, winner).await?;
    tx.commit().await?;
    Ok(Json(book_detail(&mut conn, admin.id, winner).await?))
}

/// Sets a book's facts directly, without a request: the fields get provenance `admin`, so no
/// refresh, enrichment or harvest overwrites them afterwards. Same keys, rules and 400s as an
/// approved edit request. Answers with the book as it is now.
#[utoipa::path(
    patch,
    path = "/api/admin/works/{work_id}",
    operation_id = "catalog_admin_edit_work",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body = AdminEditIn,
    responses(
        (status = 200, body = BookDetailOut, description = "The book with the new values"),
        (status = 400, body = ErrorBody, description = "`Field required` (no fields), `Unknown field: <key>` or `Invalid value for <key>`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`, or `Not found` (not an instance admin)")
    )
)]
pub async fn edit_work(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<AdminEditIn>,
) -> ApiResult<Json<BookDetailOut>> {
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let mut tx = conn.begin().await?;
    apply_admin_fields(&mut tx, CoverTarget::Work(work_id), &body.fields).await?;
    tx.commit().await?;
    Ok(Json(book_detail(&mut conn, admin.id, work_id).await?))
}

/// Sets an edition's facts directly, without a request: the fields get provenance `admin`, like
/// a book's. Same keys, rules and 400s as an approved edit request. Answers with the edition as
/// it is now.
#[utoipa::path(
    patch,
    path = "/api/admin/editions/{edition_id}",
    operation_id = "catalog_admin_edit_edition",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("edition_id" = i64, Path, description = "The edition id")),
    request_body = AdminEditIn,
    responses(
        (status = 200, body = EditionOut, description = "The edition with the new values"),
        (status = 400, body = ErrorBody, description = "`Field required` (no fields), `Unknown field: <key>` or `Invalid value for <key>`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`, or `Not found` (not an instance admin)")
    )
)]
pub async fn edit_edition(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(edition_id): ApiPath<i64>,
    ApiJson(body): ApiJson<AdminEditIn>,
) -> ApiResult<Json<EditionOut>> {
    let mut conn = state.db.acquire().await?;
    let mut tx = conn.begin().await?;
    apply_admin_fields(&mut tx, CoverTarget::Edition(edition_id), &body.fields).await?;
    tx.commit().await?;
    edition_outs(&mut conn, &[edition_id])
        .await?
        .remove(&edition_id)
        .map(Json)
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))
}
