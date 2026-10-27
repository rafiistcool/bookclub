//! Edit and merge requests (endpoint-map §4): readers never change catalog rows directly. A
//! correction of a work or an edition, or "these two are the same book", goes to the admin
//! queue (`catalog_edit_requests`, [`super::admin`]). 50 per day, shared by the three routes.
//!
//! A proposed edit is checked with the rules its approval applies (`ingest::apply_admin_fields`
//! in a transaction that is rolled back), so only approvable requests reach the queue.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;

use super::MERGE_INTO_ITSELF;
use super::detail::require_local_work;
use super::shared::books::require_work;
use super::shared::ingest::{CoverTarget, apply_admin_fields};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, FIELD_REQUIRED, Validate, Validator};
use crate::text::{py_strip, truncate_chars};

/// Longest note on a request (silently cut).
pub const MAX_NOTE_CHARS: usize = 2000;

/// A proposed correction.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EditRequestIn {
    /// The proposed values. Works: `title`, `subtitle`, `authors`, `year`, `description`,
    /// `subjects`, `pages`. Editions: `title`, `subtitle`, `publisher`, `language`, `format`,
    /// `pages`, `publication_year`, `planned_publication_date` (`YYYY-MM-DD`). Null clears a
    /// field.
    #[schema(value_type = Object)]
    pub fields: Map<String, Value>,
    /// Why; at most 2000 characters.
    #[serde(default)]
    pub note: String,
}

impl Validate for EditRequestIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(!self.fields.is_empty(), FIELD_REQUIRED);
        self.note = truncate_chars(py_strip(&self.note), MAX_NOTE_CHARS).to_owned();
    }
}

/// "These two are the same book."
#[derive(Debug, Deserialize, ToSchema)]
pub struct MergeRequestIn {
    /// The work that stays; the work in the path is merged into it.
    pub into_work_id: i64,
    /// Why; at most 2000 characters.
    #[serde(default)]
    pub note: String,
}

impl Validate for MergeRequestIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.note = truncate_chars(py_strip(&self.note), MAX_NOTE_CHARS).to_owned();
    }
}

/// A request in the admin queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditRequestCreated {
    pub id: i64,
    /// `open`.
    pub status: String,
}

/// Counts one edit or merge request (50 per day, shared).
fn hit_request_limit(state: &AppState, user_id: i64) -> ApiResult<()> {
    state
        .limits
        .edit_request_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))
}

/// Checks `fields` with the rules of their approval, writing nothing: 400 `Unknown field: <key>`
/// or `Invalid value for <key>`.
async fn check_fields(
    conn: &mut PgConnection,
    target: CoverTarget,
    fields: &Map<String, Value>,
) -> ApiResult<()> {
    let mut tx = conn.begin().await?;
    let checked = apply_admin_fields(&mut tx, target, fields).await;
    tx.rollback().await?;
    checked
}

struct NewRequest<'a> {
    kind: &'a str,
    work_id: i64,
    edition_id: Option<i64>,
    payload: Value,
    note: &'a str,
    created_by: i64,
}

async fn insert_request(conn: &mut PgConnection, request: NewRequest<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, payload, note, created_by)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        request.kind,
        request.work_id,
        request.edition_id,
        request.payload,
        request.note,
        request.created_by
    )
    .fetch_one(conn)
    .await
}

fn created(id: i64) -> (StatusCode, Json<EditRequestCreated>) {
    (
        StatusCode::CREATED,
        Json(EditRequestCreated {
            id,
            status: "open".to_owned(),
        }),
    )
}

/// Proposes a correction of a book's facts. An admin applies it; no source overwrites an
/// approved value afterwards. 50 per day, shared with edition and merge requests.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/edit-requests",
    operation_id = "catalog_request_work_edit",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body = EditRequestIn,
    responses(
        (status = 201, body = EditRequestCreated, description = "The request is in the admin queue"),
        (status = 400, body = ErrorBody, description = "`Field required` (no fields), `Unknown field: <key>` or `Invalid value for <key>`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (50 per day)")
    )
)]
pub async fn request_work_edit(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<EditRequestIn>,
) -> ApiResult<(StatusCode, Json<EditRequestCreated>)> {
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    check_fields(&mut conn, CoverTarget::Work(work_id), &body.fields).await?;
    hit_request_limit(&state, user.id)?;
    let id = insert_request(
        &mut conn,
        NewRequest {
            kind: "work_edit",
            work_id,
            edition_id: None,
            payload: Value::Object(body.fields),
            note: &body.note,
            created_by: user.id,
        },
    )
    .await?;
    Ok(created(id))
}

/// Proposes a correction of an edition's facts. Same queue and limit as book edits.
#[utoipa::path(
    post,
    path = "/api/books/editions/{edition_id}/edit-requests",
    operation_id = "catalog_request_edition_edit",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("edition_id" = i64, Path, description = "The edition id")),
    request_body = EditRequestIn,
    responses(
        (status = 201, body = EditRequestCreated, description = "The request is in the admin queue"),
        (status = 400, body = ErrorBody, description = "`Field required` (no fields), `Unknown field: <key>` or `Invalid value for <key>`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (50 per day)")
    )
)]
pub async fn request_edition_edit(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(edition_id): ApiPath<i64>,
    ApiJson(body): ApiJson<EditRequestIn>,
) -> ApiResult<(StatusCode, Json<EditRequestCreated>)> {
    let mut conn = state.db.acquire().await?;
    let work_id = sqlx::query_scalar!("SELECT work_id FROM editions WHERE id = $1", edition_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    check_fields(&mut conn, CoverTarget::Edition(edition_id), &body.fields).await?;
    hit_request_limit(&state, user.id)?;
    let id = insert_request(
        &mut conn,
        NewRequest {
            kind: "edition_edit",
            work_id,
            edition_id: Some(edition_id),
            payload: Value::Object(body.fields),
            note: &body.note,
            created_by: user.id,
        },
    )
    .await?;
    Ok(created(id))
}

/// "These two are the same book": proposes merging the book in the path into `into_work_id`
/// (for example a DNB record and an Open Library record of one work, or a legacy bare-facts
/// book and its real record). Same queue and limit as edits.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/merge-requests",
    operation_id = "catalog_request_merge",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "The work to merge away: a work id, `OL…W`, or a legacy key")),
    request_body = MergeRequestIn,
    responses(
        (status = 201, body = EditRequestCreated, description = "The request is in the admin queue"),
        (status = 400, body = ErrorBody, description = "`A book cannot be merged into itself`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.` (either book)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (50 per day)")
    )
)]
pub async fn request_merge(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<MergeRequestIn>,
) -> ApiResult<(StatusCode, Json<EditRequestCreated>)> {
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let into = require_work(&mut conn, body.into_work_id).await?;
    if into == work_id {
        return Err(AppError::bad_request(MERGE_INTO_ITSELF));
    }
    hit_request_limit(&state, user.id)?;
    let id = insert_request(
        &mut conn,
        NewRequest {
            kind: "work_merge",
            work_id,
            edition_id: None,
            payload: json!({ "into_work_id": into }),
            note: &body.note,
            created_by: user.id,
        },
    )
    .await?;
    Ok(created(id))
}
