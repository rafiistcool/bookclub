//! Editions of a work (endpoint-map §4): the list, one edition, and a reader adding a missing
//! edition (for example after a failed scan). Adding follows the ISBN rule of user-added books
//! ([`super::custom::claim_isbn`]) and shares their rate limit.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::Connection;
use utoipa::ToSchema;

use super::custom::{
    MAX_PUBLISHER_CHARS, claim_isbn, hit_contribution_limit, insert_user_edition, trimmed,
    validate_isbn, validate_language,
};
use super::detail::require_local_work;
use super::shared::books::edition_outs;
use super::shared::ingest::{EditionRecord, Format, Source};
use super::shared::isbn::parse_isbn;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::EditionOut;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, FIELD_REQUIRED, Validate, Validator};
use crate::text::truncate_chars;

/// pydantic's enum message for a bad `format`.
pub const INVALID_FORMAT: &str =
    "Input should be 'hardcover', 'paperback', 'ebook', 'audiobook' or 'other'";
/// Longest edition title and subtitle.
pub const MAX_EDITION_TITLE_CHARS: usize = 1000;

/// A work's editions, oldest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditionList {
    pub items: Vec<EditionOut>,
}

/// An edition a reader adds. At least one field is needed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NewEditionIn {
    /// An ISBN-10 or ISBN-13; spaces and hyphens are ignored.
    #[serde(default)]
    pub isbn: Option<String>,
    /// Only stored when it differs from the work's title (a translation, a new title).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    /// ISO 639-2/B code, e.g. `ger`.
    #[serde(default)]
    pub language: Option<String>,
    /// `hardcover`, `paperback`, `ebook`, `audiobook` or `other`.
    #[serde(default)]
    pub format: Option<String>,
    /// 1 to 100000.
    #[serde(default)]
    pub pages: Option<i64>,
    /// 1 to 3000.
    #[serde(default)]
    pub publication_year: Option<i64>,
}

impl Validate for NewEditionIn {
    fn validate(&mut self, v: &mut Validator) {
        validate_isbn(&mut self.isbn, v);
        self.title = trimmed(&self.title)
            .map(|text| truncate_chars(&text, MAX_EDITION_TITLE_CHARS).to_owned());
        self.subtitle = trimmed(&self.subtitle)
            .map(|text| truncate_chars(&text, MAX_EDITION_TITLE_CHARS).to_owned());
        self.publisher = trimmed(&self.publisher)
            .map(|text| truncate_chars(&text, MAX_PUBLISHER_CHARS).to_owned());
        validate_language(&mut self.language, v);
        self.format = trimmed(&self.format).map(|format| format.to_lowercase());
        if let Some(format) = &self.format {
            v.check(Format::parse(format).is_some(), INVALID_FORMAT);
        }
        v.range_i64(self.pages, 1, 100_000);
        v.range_i64(self.publication_year, 1, 3000);
        let empty = self.isbn.is_none()
            && self.title.is_none()
            && self.subtitle.is_none()
            && self.publisher.is_none()
            && self.language.is_none()
            && self.format.is_none()
            && self.pages.is_none()
            && self.publication_year.is_none();
        if empty && v.messages().is_empty() {
            v.fail(FIELD_REQUIRED);
        }
    }
}

/// Every edition of a book, oldest first.
#[utoipa::path(
    get,
    path = "/api/books/works/{work_id}/editions",
    operation_id = "catalog_list_editions",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    responses(
        (status = 200, body = EditionList, description = "The editions"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn list_editions(
    _user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<EditionList>> {
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let ids = sqlx::query_scalar!(
        "SELECT id FROM editions WHERE work_id = $1 ORDER BY id",
        work_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut outs = edition_outs(&mut conn, &ids).await?;
    let items = ids.iter().filter_map(|id| outs.remove(id)).collect();
    Ok(Json(EditionList { items }))
}

/// Adds an edition that is missing from a book. With an ISBN the catalog, the DNB (German
/// ISBNs) and Open Library are asked first; an edition they know is 409 with its `work_id` and
/// `edition_id`. 30 per day, shared with adding books.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/editions",
    operation_id = "catalog_add_edition",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body = NewEditionIn,
    responses(
        (status = 201, body = EditionOut, description = "The new edition"),
        (status = 400, body = ErrorBody, description = "`That is not a valid ISBN-10 or ISBN-13`, `Field required` (no field), or a value out of range"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 409, body = ErrorBody, description = "`That ISBN is already in the catalog` (with `work_id` and `edition_id`)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (30 per day), or the ISBN lookup is busy"),
        (status = 502, body = ErrorBody, description = "`Could not look that ISBN up right now. Try again.`")
    )
)]
pub async fn add_edition(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    ApiJson(body): ApiJson<NewEditionIn>,
) -> ApiResult<(StatusCode, Json<EditionOut>)> {
    let work_id = {
        let mut conn = state.db.acquire().await?;
        require_local_work(&mut conn, &raw).await?
    };
    hit_contribution_limit(&state, user.id)?;
    let isbn = body.isbn.as_deref().and_then(parse_isbn);
    if let Some(isbn) = &isbn {
        claim_isbn(&state, isbn).await?;
    }
    let mut conn = state.db.acquire().await?;
    let edition = EditionRecord {
        isbn13: isbn,
        title: body.title,
        subtitle: body.subtitle,
        publisher: body.publisher,
        language: body.language,
        format: body.format.as_deref().and_then(Format::parse),
        pages: body.pages.and_then(|pages| i32::try_from(pages).ok()),
        publication_year: body
            .publication_year
            .and_then(|year| i32::try_from(year).ok()),
        created_by: Some(user.id),
        ..EditionRecord::new(Source::User)
    };
    let mut tx = conn.begin().await?;
    let (_, edition_id) = insert_user_edition(&mut tx, work_id, &edition).await?;
    tx.commit().await?;
    let out = edition_outs(&mut conn, &[edition_id])
        .await?
        .remove(&edition_id)
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// One edition.
#[utoipa::path(
    get,
    path = "/api/books/editions/{edition_id}",
    operation_id = "catalog_get_edition",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("edition_id" = i64, Path, description = "The edition id")),
    responses(
        (status = 200, body = EditionOut, description = "The edition"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn get_edition(
    _user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(edition_id): ApiPath<i64>,
) -> ApiResult<Json<EditionOut>> {
    let mut conn = state.db.acquire().await?;
    edition_outs(&mut conn, &[edition_id])
        .await?
        .remove(&edition_id)
        .map(Json)
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))
}
