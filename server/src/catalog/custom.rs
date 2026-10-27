//! `POST /api/books/custom` (endpoint-map §4, docs/phase1/spec/catalog.md §1.4): a work a reader
//! adds because no source has it. It is a `source='user'` work with `created_by`, every entered
//! field with provenance `user`, plus an edition when an ISBN or edition facts are given, and
//! it is searchable by everyone at once.
//!
//! With an ISBN the sources are asked first ([`claim_isbn`]): a typo must never claim a real
//! ISBN that the catalog, the DNB or Open Library knows. Adding an edition shares the rule and
//! the rate limit (30 per day).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;

use super::detail::{BookDetailOut, book_detail};
use super::isbn_lookup::resolve_isbn;
use super::shared::books::follow_merges;
use super::shared::ingest::{self, EditionRecord, Mode, Source, WorkRecord};
use super::shared::isbn::{Isbn13, parse_isbn};
use super::shared::lookup::local_isbn;
use super::{INVALID_ISBN, ISBN_IN_CATALOG, ISBN_UNAVAILABLE, TITLE_REQUIRED};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::text::{py_strip, truncate_chars};

/// Longest title, authors and description of a user-added book (silently truncated, parity).
pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_AUTHORS_CHARS: usize = 300;
pub const MAX_DESCRIPTION_CHARS: usize = 4000;
pub const MAX_PUBLISHER_CHARS: usize = 300;
/// pydantic's message for a `pattern` constraint, for a language code.
pub const LANGUAGE_PATTERN: &str = "String should match pattern '^[a-z]{3}$'";

/// A book no source knows.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CustomBookIn {
    /// Required; trimmed, at most 500 characters (longer titles are cut).
    pub title: String,
    /// `", "`-separated names; trimmed, at most 300 characters.
    #[serde(default)]
    pub authors: String,
    /// 1 to 3000.
    #[serde(default)]
    pub year: Option<i64>,
    /// Trimmed, at most 4000 characters.
    #[serde(default)]
    pub description: String,
    /// An ISBN-10 or ISBN-13 of the reader's copy; spaces and hyphens are ignored.
    #[serde(default)]
    pub isbn: Option<String>,
    /// The publisher of the reader's copy; trimmed, at most 300 characters.
    #[serde(default)]
    pub publisher: String,
    /// The page count of the reader's copy, 1 to 100000.
    #[serde(default)]
    pub pages: Option<i64>,
    /// ISO 639-2/B code of the reader's copy, e.g. `ger`.
    #[serde(default)]
    pub language: Option<String>,
}

/// Trims an optional text field; blank is absent.
pub(crate) fn trimmed(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(py_strip)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Checks an optional ISBN field in place (blank is absent).
pub(crate) fn validate_isbn(isbn: &mut Option<String>, v: &mut Validator) {
    *isbn = trimmed(isbn);
    if let Some(raw) = isbn {
        v.check(parse_isbn(raw).is_some(), INVALID_ISBN);
    }
}

/// Normalises an optional language code in place (trimmed, lower case; blank is absent).
pub(crate) fn validate_language(language: &mut Option<String>, v: &mut Validator) {
    *language = trimmed(language).map(|code| code.to_lowercase());
    if let Some(code) = language {
        v.check(
            code.len() == 3 && code.bytes().all(|b| b.is_ascii_lowercase()),
            LANGUAGE_PATTERN,
        );
    }
}

impl Validate for CustomBookIn {
    fn validate(&mut self, v: &mut Validator) {
        let title = py_strip(&self.title);
        if title.is_empty() {
            v.fail(TITLE_REQUIRED);
        }
        self.title = truncate_chars(title, MAX_TITLE_CHARS).to_owned();
        self.authors = truncate_chars(py_strip(&self.authors), MAX_AUTHORS_CHARS).to_owned();
        v.range_i64(self.year, 1, 3000);
        self.description =
            truncate_chars(py_strip(&self.description), MAX_DESCRIPTION_CHARS).to_owned();
        validate_isbn(&mut self.isbn, v);
        self.publisher = truncate_chars(py_strip(&self.publisher), MAX_PUBLISHER_CHARS).to_owned();
        v.range_i64(self.pages, 1, 100_000);
        validate_language(&mut self.language, v);
    }
}

/// 409 `That ISBN is already in the catalog`, naming the work and edition that hold it.
pub(crate) fn isbn_conflict(work_id: i64, edition_id: Option<i64>) -> AppError {
    AppError::conflict(ISBN_IN_CATALOG)
        .with("work_id", work_id)
        .with("edition_id", edition_id)
}

/// The rule for an ISBN a reader enters: it must be unknown to the catalog and to every source.
/// A local edition, or a remote hit (which the remote route persists), is 409 with its ids; a
/// lookup without a definite answer is 429 or 502, never a pass. Holds no pool connection while
/// the sources are asked ([`resolve_isbn`]).
pub(crate) async fn claim_isbn(state: &AppState, isbn: &Isbn13) -> ApiResult<()> {
    let local = {
        let mut conn = state.db.acquire().await?;
        local_isbn(&mut conn, isbn).await?
    };
    if let Some(found) = local {
        return Err(isbn_conflict(found.work_id, found.edition_id));
    }
    match resolve_isbn(state, isbn, true, ISBN_UNAVAILABLE).await? {
        Some(found) => Err(isbn_conflict(found.work_id, found.edition_id)),
        None => Ok(()),
    }
}

/// Counts one user-added book or edition (30 per day, shared).
pub(crate) fn hit_contribution_limit(state: &AppState, user_id: i64) -> ApiResult<()> {
    state
        .limits
        .custom_book_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))
}

/// How often [`insert_user_edition`] starts over when the work it wrote to was merged away
/// meanwhile.
const MERGED_RETRIES: usize = 2;

/// Inserts a user edition of `work_id` inside the caller's transaction (in a savepoint) and
/// returns the work it landed on and its id.
///
/// - **A merge in flight:** the work the edition landed on is locked `FOR NO KEY UPDATE`
///   afterwards, like a merge locks it. A merge that started earlier cannot see the new edition
///   when it moves the loser's editions; the lock waits for it, sees the work merged away, and
///   the insert starts over on the surviving work (ingest follows the merge). A merge that
///   starts later waits for this transaction and then moves the edition itself.
/// - **An ISBN conflict:** when the ISBN turned up under another work meanwhile (a concurrent
///   lookup), the answer is 409 and the caller rolls back. A work merged away since the caller
///   resolved it is no conflict.
pub(crate) async fn insert_user_edition(
    conn: &mut PgConnection,
    work_id: i64,
    edition: &EditionRecord,
) -> ApiResult<(i64, i64)> {
    for _ in 0..=MERGED_RETRIES {
        let mut tx = conn.begin().await?;
        let (found_work, edition_id) =
            ingest::upsert_edition(&mut tx, work_id, edition, Mode::Refresh).await?;
        let current = sqlx::query_scalar!(
            r#"SELECT merged_into_id IS NULL AS "current!" FROM works WHERE id = $1
               FOR NO KEY UPDATE"#,
            found_work
        )
        .fetch_one(&mut *tx)
        .await?;
        if !current {
            tx.rollback().await?;
            continue;
        }
        // Without an ISBN (a user edition's only identifier) the edition is always new.
        if found_work != work_id
            && edition.isbn13.is_some()
            && follow_merges(&mut tx, work_id).await? != Some(found_work)
        {
            return Err(isbn_conflict(found_work, Some(edition_id)));
        }
        tx.commit().await?;
        return Ok((found_work, edition_id));
    }
    Err(AppError::internal(format!(
        "work {work_id} was merged away during every attempt to add an edition"
    )))
}

/// Adds a book that no source knows. Never calls Open Library, except to check an ISBN: with an
/// ISBN the catalog, the DNB (German ISBNs) and Open Library are asked first, and a book they
/// know is 409 with its `work_id` and `edition_id`. 30 per day, shared with adding editions.
#[utoipa::path(
    post,
    path = "/api/books/custom",
    operation_id = "catalog_create_custom_book",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = CustomBookIn,
    responses(
        (status = 201, body = BookDetailOut, description = "The new book"),
        (status = 400, body = ErrorBody, description = "`A title is required`, `That is not a valid ISBN-10 or ISBN-13`, or a value out of range"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 409, body = ErrorBody, description = "`That ISBN is already in the catalog` (with `work_id` and `edition_id`)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (30 per day), or the ISBN lookup is busy"),
        (status = 502, body = ErrorBody, description = "`Could not look that ISBN up right now. Try again.`")
    )
)]
pub async fn create_custom_book(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CustomBookIn>,
) -> ApiResult<(StatusCode, Json<BookDetailOut>)> {
    hit_contribution_limit(&state, user.id)?;
    let isbn = body.isbn.as_deref().and_then(parse_isbn);
    if let Some(isbn) = &isbn {
        claim_isbn(&state, isbn).await?;
    }
    let mut conn = state.db.acquire().await?;
    let work = WorkRecord {
        author_names: body.authors,
        first_publish_year: body.year.and_then(|year| i32::try_from(year).ok()),
        description: Some(body.description).filter(|text| !text.is_empty()),
        created_by: Some(user.id),
        ..WorkRecord::new(Source::User, body.title)
    };
    let publisher = Some(body.publisher).filter(|text| !text.is_empty());
    let pages = body.pages.and_then(|pages| i32::try_from(pages).ok());
    let has_edition =
        isbn.is_some() || publisher.is_some() || pages.is_some() || body.language.is_some();
    let mut tx = conn.begin().await?;
    let work_id = ingest::upsert_work(&mut tx, &work, Mode::Refresh).await?;
    if has_edition {
        let edition = EditionRecord {
            isbn13: isbn,
            publisher,
            pages,
            language: body.language,
            created_by: Some(user.id),
            ..EditionRecord::new(Source::User)
        };
        insert_user_edition(&mut tx, work_id, &edition).await?;
    }
    tx.commit().await?;
    let detail = book_detail(&mut conn, user.id, work_id).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}
