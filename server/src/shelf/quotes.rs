//! Quotes: lines a reader keeps from a book (endpoint-map §7, docs/phase1/spec/diary.md §1.7–
//! §1.10, §5 "Quotes", R6, R7 and R24).
//!
//! Quotes are personal, each `audience` or `only_me`. Every read goes through
//! `bc_visible_quotes`: a quote the viewer cannot see answers like a missing one (404), and a
//! visible quote of someone else is 403 for edits. A quote names a catalog work and optionally the
//! edition its `page` refers to (default: the author's shelf edition of the work).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};

use super::entries::{NOTHING_TO_UPDATE, visibility_text};
use crate::AppState;
use crate::activity::{EventKind, NewEvent, record};
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{
    BookRef, book_outs, follow_merges, require_edition_of, require_work,
};
use crate::dto::{BookOut, Visibility};
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, ApiQuery, Patch, Validate, Validator};
use crate::text::{char_len, py_strip, truncate_chars};
use crate::tz;
use crate::visibility;

pub const PASTE_THE_LINE: &str = "Paste the line you want to keep";
pub const QUOTE_TOO_LONG: &str = "Keep it to 1000 characters";
pub const NO_QUOTE: &str = "No quote with that id";
pub const EDIT_OWN_QUOTES: &str = "You can only edit your own quotes";
pub const DELETE_OWN_QUOTES: &str = "You can only delete your own quotes";
const MAX_BODY_CHARS: usize = 1000;
const MAX_PAGE: i64 = 20_000;
/// The `quote_added` excerpt: the first 80 characters of the body (stored, and served from
/// the current body by the feeds).
pub const EXCERPT_CHARS: usize = 80;

/// One quote as the viewer sees it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QuoteOut {
    pub id: i64,
    /// The work, with the quote's edition as context.
    pub book: BookOut,
    /// The edition `page` refers to; null when unknown.
    pub edition_id: Option<i64>,
    /// The username of the quote's owner.
    pub author: String,
    /// Whether the viewer owns the quote.
    pub mine: bool,
    pub body: String,
    pub page: Option<i32>,
    pub visibility: Visibility,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// `created_at` in the viewer's time zone, e.g. `Tue 27 Oct 2026, 10:46 CET`.
    pub created_label: String,
}

/// The newest quotes first.
#[derive(Debug, Serialize, ToSchema)]
pub struct QuoteListOut {
    pub items: Vec<QuoteOut>,
}

/// Filters of `GET /api/quotes`; they combine.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QuoteQuery {
    /// Only quotes of this work (a merged work id finds the work it was merged into). An
    /// unknown work gives an empty list. Empty means no filter.
    #[serde(default, deserialize_with = "optional_work_id")]
    pub work: Option<i64>,
    /// Only this user's quotes. An unknown name, or a user outside the caller's audience, gives
    /// an empty list. Empty means no filter.
    #[serde(default)]
    pub username: Option<String>,
    /// 1 to 500, default 100.
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// `?work=`: an empty value is no filter, like an absent one (the legacy `if work:`); any
/// other value must be an integer (400 `Input should be a valid integer, unable to parse
/// string as an integer`).
fn optional_work_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let text = String::deserialize(deserializer)?;
    if text.is_empty() {
        return Ok(None);
    }
    text.parse().map(Some).map_err(de::Error::custom)
}

impl Validate for QuoteQuery {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(Some(self.limit), 1, 500);
    }
}

/// A new quote.
#[derive(Debug, Deserialize, ToSchema)]
pub struct QuoteIn {
    /// The work; a merged work resolves to the work it was merged into.
    pub work_id: i64,
    /// The edition `page` refers to, an edition of that work. Default: the caller's shelf
    /// edition of the work, if any.
    #[serde(default)]
    pub edition_id: Option<i64>,
    /// Trimmed, 1 to 1000 characters.
    pub body: String,
    /// 0 to 20000.
    #[serde(default)]
    pub page: Option<i32>,
    /// Default `audience`.
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

impl Validate for QuoteIn {
    fn validate(&mut self, v: &mut Validator) {
        self.body = checked_body(&self.body, v);
        v.range_i64(self.page.map(i64::from), 0, MAX_PAGE);
    }
}

/// Changes to one of the caller's quotes. `body` and `visibility` treat null as absent; `page`
/// null clears the page.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct QuotePatchIn {
    /// Trimmed, 1 to 1000 characters.
    #[serde(default)]
    pub body: Option<String>,
    /// 0 to 20000; null clears it.
    #[serde(default)]
    #[schema(value_type = Option<i32>)]
    pub page: Patch<i32>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

impl Validate for QuotePatchIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(body) = &self.body {
            self.body = Some(checked_body(body, v));
        }
        if let Patch::Value(page) = self.page {
            v.range_i64(Some(page.into()), 0, MAX_PAGE);
        }
    }
}

impl QuotePatchIn {
    fn is_empty(&self) -> bool {
        self.body.is_none() && self.page.is_missing() && self.visibility.is_none()
    }
}

/// The trimmed body, recording the legacy messages when it is empty or too long.
fn checked_body(raw: &str, v: &mut Validator) -> String {
    let body = py_strip(raw).to_owned();
    if body.is_empty() {
        v.fail(PASTE_THE_LINE);
    } else if char_len(&body) > MAX_BODY_CHARS {
        v.fail(QUOTE_TOO_LONG);
    }
    body
}

/// The quotes the caller may see, newest first: their own, and the `audience` quotes of their
/// audience.
#[utoipa::path(
    get,
    path = "/api/quotes",
    operation_id = "shelf_list_quotes",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(QuoteQuery),
    responses(
        (status = 200, body = QuoteListOut, description = "The quotes"),
        (status = 400, body = ErrorBody, description = "A `limit` outside 1..500, a non-numeric `work`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list_quotes(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<QuoteQuery>,
) -> ApiResult<Json<QuoteListOut>> {
    let empty = || Ok(Json(QuoteListOut { items: Vec::new() }));
    let mut conn = state.db.acquire().await?;
    let work_id = match query.work {
        Some(work) => match follow_merges(&mut conn, work).await? {
            Some(work_id) => Some(work_id),
            None => return empty(),
        },
        None => None,
    };
    let owner_id = match query.username.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => match visibility::visible_user(&mut conn, user.id, name).await {
            Ok(owner) => Some(owner.id),
            Err(error) if error.status == StatusCode::NOT_FOUND => return empty(),
            Err(error) => return Err(error),
        },
        None => None,
    };
    let rows = sqlx::query_as!(
        QuoteRow,
        r#"SELECT q.id AS "id!", q.user_id AS "user_id!", u.username, q.work_id AS "work_id!",
                  q.edition_id, q.body AS "body!", q.page,
                  q.visibility AS "visibility!: Visibility", q.created_at AS "created_at!",
                  q.is_self AS "is_self!"
           FROM bc_visible_quotes($1) AS q
           JOIN users u ON u.id = q.user_id
           WHERE ($2::int8 IS NULL OR q.work_id = $2) AND ($3::int8 IS NULL OR q.user_id = $3)
           ORDER BY q.id DESC
           LIMIT $4"#,
        user.id,
        work_id,
        owner_id,
        query.limit
    )
    .fetch_all(&mut *conn)
    .await?;
    let items = quote_outs(&mut conn, &user, rows).await?;
    Ok(Json(QuoteListOut { items }))
}

/// Keeps a quote from a catalog work. Records `quote_added`.
#[utoipa::path(
    post,
    path = "/api/quotes",
    operation_id = "shelf_add_quote",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = QuoteIn,
    responses(
        (status = 201, body = QuoteOut, description = "The new quote"),
        (status = 400, body = ErrorBody, description = "Validation, `Paste the line you want to keep`, `Keep it to 1000 characters`, `That edition belongs to another book`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn add_quote(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<QuoteIn>,
) -> ApiResult<(StatusCode, Json<QuoteOut>)> {
    let mut tx = state.db.begin().await?;
    let work_id = require_work(&mut tx, body.work_id).await?;
    let edition_id = match body.edition_id {
        Some(edition_id) => {
            require_edition_of(&mut tx, work_id, edition_id).await?;
            Some(edition_id)
        }
        None => sqlx::query_scalar!(
            "SELECT edition_id FROM shelf_entries WHERE user_id = $1 AND work_id = $2",
            user.id,
            work_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .flatten(),
    };
    let visibility = body.visibility.unwrap_or(Visibility::Audience);
    let id = sqlx::query_scalar!(
        "INSERT INTO quotes (user_id, work_id, edition_id, body, page, visibility)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
        user.id,
        work_id,
        edition_id,
        body.body,
        body.page,
        visibility_text(visibility)
    )
    .fetch_one(&mut *tx)
    .await?;
    let payload = json!({
        "page": body.page,
        "excerpt": truncate_chars(&body.body, EXCERPT_CHARS),
        "quote_id": id,
    });
    record(
        &mut tx,
        NewEvent {
            club_id: None,
            actor_id: Some(user.id),
            kind: EventKind::QuoteAdded,
            work_id: Some(work_id),
            pick_id: None,
            payload: payload.as_object().cloned().unwrap_or_default(),
        },
    )
    .await?;
    let quote = quote_out(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(quote)))
}

/// Changes one of the caller's quotes.
#[utoipa::path(
    patch,
    path = "/api/quotes/{quote_id}",
    operation_id = "shelf_update_quote",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("quote_id" = i64, Path, description = "The quote")),
    request_body = QuotePatchIn,
    responses(
        (status = 200, body = QuoteOut, description = "The changed quote"),
        (status = 400, body = ErrorBody, description = "Validation, `Paste the line you want to keep`, `Keep it to 1000 characters`, `Nothing to update`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`You can only edit your own quotes`"),
        (status = 404, body = ErrorBody, description = "`No quote with that id` (also a quote the caller cannot see)")
    )
)]
pub async fn update_quote(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(quote_id): ApiPath<i64>,
    ApiJson(body): ApiJson<QuotePatchIn>,
) -> ApiResult<Json<QuoteOut>> {
    let mut tx = state.db.begin().await?;
    if !visible_quote(&mut tx, user.id, quote_id).await? {
        return Err(AppError::forbidden(EDIT_OWN_QUOTES));
    }
    if body.is_empty() {
        return Err(AppError::bad_request(NOTHING_TO_UPDATE));
    }
    let (page_sent, page) = match body.page {
        Patch::Missing => (false, None),
        Patch::Null => (true, None),
        Patch::Value(page) => (true, Some(page)),
    };
    let updated = sqlx::query_scalar!(
        "UPDATE quotes
         SET body = coalesce($3, body),
             page = CASE WHEN $4 THEN $5 ELSE page END,
             visibility = coalesce($6, visibility),
             updated_at = now()
         WHERE id = $1 AND user_id = $2
         RETURNING id",
        quote_id,
        user.id,
        body.body,
        page_sent,
        page,
        body.visibility.map(visibility_text)
    )
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Err(AppError::not_found(NO_QUOTE));
    }
    let quote = quote_out(&mut tx, &user, quote_id).await?;
    tx.commit().await?;
    Ok(Json(quote))
}

/// Deletes one of the caller's quotes. Its `quote_added` event stays; the feeds hide it.
#[utoipa::path(
    delete,
    path = "/api/quotes/{quote_id}",
    operation_id = "shelf_delete_quote",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("quote_id" = i64, Path, description = "The quote")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`You can only delete your own quotes`"),
        (status = 404, body = ErrorBody, description = "`No quote with that id` (also a quote the caller cannot see)")
    )
)]
pub async fn delete_quote(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(quote_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    if !visible_quote(&mut tx, user.id, quote_id).await? {
        return Err(AppError::forbidden(DELETE_OWN_QUOTES));
    }
    let deleted = sqlx::query!(
        "DELETE FROM quotes WHERE id = $1 AND user_id = $2",
        quote_id,
        user.id
    )
    .execute(&mut *tx)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found(NO_QUOTE));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether the viewer owns the quote; 404 `No quote with that id` when it is missing or the
/// viewer cannot see it (`bc_visible_quotes`).
async fn visible_quote(conn: &mut PgConnection, viewer: i64, quote_id: i64) -> ApiResult<bool> {
    sqlx::query_scalar!(
        r#"SELECT q.is_self AS "is_self!" FROM bc_visible_quotes($1) AS q WHERE q.id = $2"#,
        viewer,
        quote_id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_QUOTE))
}

/// A quote row read through `bc_visible_quotes`.
struct QuoteRow {
    id: i64,
    user_id: i64,
    username: String,
    work_id: i64,
    edition_id: Option<i64>,
    body: String,
    page: Option<i32>,
    visibility: Visibility,
    created_at: OffsetDateTime,
    is_self: bool,
}

/// One quote of the viewer's, as they see it (after a write).
async fn quote_out(
    conn: &mut PgConnection,
    viewer: &CurrentUser,
    quote_id: i64,
) -> ApiResult<QuoteOut> {
    let rows = sqlx::query_as!(
        QuoteRow,
        r#"SELECT q.id AS "id!", q.user_id AS "user_id!", u.username, q.work_id AS "work_id!",
                  q.edition_id, q.body AS "body!", q.page,
                  q.visibility AS "visibility!: Visibility", q.created_at AS "created_at!",
                  q.is_self AS "is_self!"
           FROM bc_visible_quotes($1) AS q
           JOIN users u ON u.id = q.user_id
           WHERE q.id = $2"#,
        viewer.id,
        quote_id
    )
    .fetch_all(&mut *conn)
    .await?;
    quote_outs(conn, viewer, rows)
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(NO_QUOTE))
}

/// The rows as [`QuoteOut`]s, in order; labels in the viewer's zone. A quote whose work is gone
/// is left out (works are never deleted).
async fn quote_outs(
    conn: &mut PgConnection,
    viewer: &CurrentUser,
    rows: Vec<QuoteRow>,
) -> sqlx::Result<Vec<QuoteOut>> {
    let zone = tz::zone_or_utc(&viewer.timezone);
    let refs: Vec<BookRef> = rows
        .iter()
        .map(|row| BookRef {
            work_id: row.work_id,
            edition_id: row.edition_id,
        })
        .collect();
    let books = book_outs(conn, &refs).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let book = books
                .get(&BookRef {
                    work_id: row.work_id,
                    edition_id: row.edition_id,
                })?
                .clone();
            Some(QuoteOut {
                id: row.id,
                book,
                edition_id: row.edition_id,
                author: row.username,
                mine: row.is_self && row.user_id == viewer.id,
                body: row.body,
                page: row.page,
                visibility: row.visibility,
                created_label: tz::label(row.created_at, &zone),
                created_at: row.created_at,
            })
        })
        .collect())
}
