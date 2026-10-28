//! The Goodreads CSV import (endpoint-map §7, docs/phase1/spec/shelf.md §1.3, §2.7 and §6
//! risks 18–20): the parser, the background job and its two routes.
//!
//! `POST /api/shelf/import` parses the file at once (a parse error is a synchronous 400), stores
//! the parsed rows in an `import_jobs` row and answers 202. A spawned task then looks the rows
//! up one after the other through `state.lookup` and shelves the hits. Each row is committed
//! together with the job's progress, under the user's lock, so a restarted server resumes at
//! `processed` ([`resume`]) and never shelves a row twice. The import records no events.
//!
//! All jobs share [`LOOKUP_SLOTS`]: at most two row lookups (and so two pool connections held
//! through remote calls) run at once. A throttled row waits and is tried again before it is
//! skipped ([`lookup_row`]).

use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::multipart::MultipartError;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use sqlx::types::Json as SqlJson;
use time::{Date, Month, OffsetDateTime};
use tokio::sync::Semaphore;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::entries::{NewEntry, insert_entry};
use super::ordering::{StatusFields, apply_finish_fields, apply_reading_dates};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::follow_merges;
use crate::catalog::shared::isbn::parse_isbn;
use crate::catalog::shared::lookup::{CatalogLookup, LookupError};
use crate::dto::{ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, ErrorBody, is_unique_violation};
use crate::extract::{ApiMultipart, ApiPath, FIELD_REQUIRED};
use crate::outbound::RetryPolicy;
use crate::text::{py_strip, truncate_chars};
use crate::tz;

/// The largest file the import reads.
pub const MAX_IMPORT_BYTES: usize = 1_500_000;
/// Rows after this many are dropped with one `stopped_after_400` skip.
pub const MAX_IMPORT_ROWS: usize = 400;
/// `skips` lists at most this many; `skipped` counts all of them.
pub const SKIP_LIST_LIMIT: usize = 25;
/// The route's body limit (conventions §4.6): the file plus the multipart framing.
const IMPORT_BODY_LIMIT: usize = 2 * 1024 * 1024;
/// The longest `Title` and `Author` a row keeps (the search box's limit), so the lookup's
/// full-text step never gets an unbounded string from the file.
pub const MAX_LOOKUP_CHARS: usize = 300;

/// Row lookups that may run at once, across all import jobs. Each holds a pool connection
/// through its remote calls, and Open Library's 3 requests per second are shared anyway, so more
/// would only take connections from the API.
static LOOKUP_SLOTS: Semaphore = Semaphore::const_new(2);
/// The longest a throttled row waits in total before it is skipped as `catalog_unavailable`:
/// the throttle wait of background jobs (conventions §10.3, `RetryPolicy::JOB`).
const THROTTLE_BUDGET: Duration = RetryPolicy::JOB.max_wait;
/// The pause before a throttled row is tried again while Open Library has no cooldown running
/// (the throttle's queue was full) ...
const QUEUE_PAUSE: Duration = Duration::from_millis(250);
/// ... and how often a row is tried again that way.
const QUEUE_RETRIES: u32 = 2;
/// Added to a running cooldown before the row is tried again.
const COOLDOWN_MARGIN: Duration = Duration::from_millis(100);

pub const FILE_TOO_LARGE: &str = "That file is too large. Keep the export under 1.5 MB.";
pub const NOT_UTF8: &str = "The file must be a UTF-8 Goodreads CSV export";
pub const NOT_GOODREADS: &str = "That does not look like a Goodreads export";
pub const IMPORT_RUNNING: &str = "An import is already running";
pub const NO_SUCH_IMPORT: &str = "No such import";
/// `error` of a job that stopped on an unexpected error (logged); the rows done so far stay.
pub const IMPORT_FAILED: &str = "The import stopped unexpectedly. Try again.";

/// The import routes. The upload route raises the body limit to 2 MiB.
pub fn routes() -> OpenApiRouter<AppState> {
    let upload = OpenApiRouter::new()
        .routes(routes!(start_import))
        .layer(DefaultBodyLimit::max(IMPORT_BODY_LIMIT));
    OpenApiRouter::new()
        .merge(upload)
        .routes(routes!(get_import))
}

/// Why a row was not imported. `reason` carries the legacy English text of each code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportSkipCode {
    /// `Stopped after 400 rows` (title `""`): the rest of the file was not read.
    #[serde(rename = "stopped_after_400")]
    StoppedAfter400,
    /// `No title` (title `"Untitled"`).
    NoTitle,
    /// `Unknown shelf`: not to-read, currently-reading or read.
    UnknownShelf,
    /// `No match`: every catalog source answered and none knew the book.
    NoMatch,
    /// `Catalog unavailable`: a source was throttled or down and nothing matched.
    CatalogUnavailable,
    /// `Already on your shelf`, also for a book an earlier row of the file added.
    AlreadyOnShelf,
}

impl ImportSkipCode {
    /// The legacy English reason.
    pub fn reason(self) -> &'static str {
        match self {
            ImportSkipCode::StoppedAfter400 => "Stopped after 400 rows",
            ImportSkipCode::NoTitle => "No title",
            ImportSkipCode::UnknownShelf => "Unknown shelf",
            ImportSkipCode::NoMatch => "No match",
            ImportSkipCode::CatalogUnavailable => "Catalog unavailable",
            ImportSkipCode::AlreadyOnShelf => "Already on your shelf",
        }
    }
}

/// One skipped row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ImportSkipOut {
    /// The row's title; `""` for `stopped_after_400`, `"Untitled"` for `no_title`.
    pub title: String,
    pub code: ImportSkipCode,
    /// The legacy English text of `code`.
    pub reason: String,
}

impl ImportSkipOut {
    pub fn new(title: impl Into<String>, code: ImportSkipCode) -> Self {
        Self {
            title: title.into(),
            code,
            reason: code.reason().to_owned(),
        }
    }
}

/// Where an import job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Queued,
    Running,
    Done,
    Failed,
}

/// A Goodreads import and its progress.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportJobOut {
    pub id: i64,
    pub status: ImportStatus,
    /// The rows to look up: the file's rows without the ones skipped while parsing.
    pub total: i32,
    /// The rows looked up so far.
    pub processed: i32,
    pub imported: i32,
    /// Every skip so far, the rows skipped while parsing included.
    pub skipped: i32,
    /// The first 25 skips: the parse skips in file order, then the lookup skips in file order.
    pub skips: Vec<ImportSkipOut>,
    /// Why a `failed` job stopped.
    pub error: Option<String>,
}

/// An `import_jobs` row as the API shows it.
struct JobRow {
    id: i64,
    status: ImportStatus,
    total: i32,
    processed: i32,
    imported: i32,
    skipped: i32,
    skips: SqlJson<Vec<ImportSkipOut>>,
    error: Option<String>,
}

impl From<JobRow> for ImportJobOut {
    fn from(row: JobRow) -> Self {
        ImportJobOut {
            id: row.id,
            status: row.status,
            total: row.total,
            processed: row.processed,
            imported: row.imported,
            skipped: row.skipped,
            skips: row.skips.0,
            error: row.error,
        }
    }
}

/// The upload of `POST /api/shelf/import` (documentation only).
#[derive(ToSchema)]
pub struct GoodreadsUpload {
    /// The Goodreads "Export Library" CSV, UTF-8, at most 1.5 MB.
    #[schema(value_type = String, format = Binary)]
    pub file: Vec<u8>,
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// One parsed row to look up, as stored in `import_jobs.rows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodreadsRow {
    /// At most [`MAX_LOOKUP_CHARS`] characters.
    pub title: String,
    /// The `Author` column only (Additional Authors are ignored), at most [`MAX_LOOKUP_CHARS`]
    /// characters.
    pub authors: String,
    /// `ISBN13`, else `ISBN`, when it is a valid ISBN (normalised to 13 digits).
    pub isbn13: Option<String>,
    pub status: ShelfStatus,
    /// `My Rating` 1–5 of a `read` row (Goodreads writes 0 for "not rated").
    pub rating: Option<i32>,
    /// `Date Read` of a `read` row.
    pub read_on: Option<Date>,
}

/// A parsed export: the rows to look up and the rows skipped while parsing, both in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExport {
    pub rows: Vec<GoodreadsRow>,
    pub skips: Vec<ImportSkipOut>,
}

/// `parse_goodreads_csv`: checks the size, the encoding (UTF-8, a BOM allowed) and the headers
/// (`Title` and `Exclusive Shelf`, trimmed and case-insensitive), then maps every row. The error
/// is the 400 detail.
///
/// The CSV dialect is Python's `excel` one as the `csv` crate reads it: blank lines are skipped
/// and do not count, a quote inside an unquoted field (`="0316556343"`) is a literal character,
/// short rows read missing cells as `""` and extra cells are ignored.
pub fn parse_goodreads_csv(raw: &[u8]) -> Result<ParsedExport, &'static str> {
    if raw.len() > MAX_IMPORT_BYTES {
        return Err(FILE_TOO_LARGE);
    }
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    let text = std::str::from_utf8(raw).map_err(|_| NOT_UTF8)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = reader.records();
    let headers: Vec<String> = match records.next() {
        Some(Ok(record)) => record.iter().map(str::to_owned).collect(),
        _ => return Err(NOT_GOODREADS),
    };
    let names: Vec<String> = headers
        .iter()
        .map(|name| py_strip(name).to_lowercase())
        .collect();
    if !names.iter().any(|name| name == "exclusive shelf")
        || !names.iter().any(|name| name == "title")
    {
        return Err(NOT_GOODREADS);
    }
    let columns = Columns::new(&headers);

    let mut parsed = ParsedExport {
        rows: Vec::new(),
        skips: Vec::new(),
    };
    for (index, record) in records.enumerate() {
        let record = record.map_err(|_| NOT_GOODREADS)?;
        if index >= MAX_IMPORT_ROWS {
            parsed
                .skips
                .push(ImportSkipOut::new("", ImportSkipCode::StoppedAfter400));
            break;
        }
        match parse_row(&columns, &record) {
            Ok(row) => parsed.rows.push(row),
            Err(skip) => parsed.skips.push(skip),
        }
    }
    Ok(parsed)
}

/// The column of each cell the import reads.
struct Columns {
    title: Option<usize>,
    author: Option<usize>,
    shelf: Option<usize>,
    isbn13: Option<usize>,
    isbn: Option<usize>,
    rating: Option<usize>,
    date_read: Option<usize>,
}

impl Columns {
    fn new(headers: &[String]) -> Self {
        Self {
            title: column(headers, "Title"),
            author: column(headers, "Author"),
            shelf: column(headers, "Exclusive Shelf"),
            isbn13: column(headers, "ISBN13"),
            isbn: column(headers, "ISBN"),
            rating: column(headers, "My Rating"),
            date_read: column(headers, "Date Read"),
        }
    }
}

/// `_cell`'s key lookup: the header spelled exactly so, else one that matches ignoring case
/// (not trimmed). With duplicate headers the last column wins, like Python's row dict.
fn column(headers: &[String], name: &str) -> Option<usize> {
    headers
        .iter()
        .rposition(|header| header == name)
        .or_else(|| {
            let lower = name.to_lowercase();
            headers
                .iter()
                .rposition(|header| header.to_lowercase() == lower)
        })
}

/// The trimmed cell, `""` when the row is too short or the column is missing.
fn cell(record: &csv::StringRecord, column: Option<usize>) -> &str {
    column
        .and_then(|index| record.get(index))
        .map(py_strip)
        .unwrap_or_default()
}

/// A `Title` or `Author` cell cut to [`MAX_LOOKUP_CHARS`] characters.
fn lookup_text(value: &str) -> &str {
    py_strip(truncate_chars(value, MAX_LOOKUP_CHARS))
}

fn parse_row(columns: &Columns, record: &csv::StringRecord) -> Result<GoodreadsRow, ImportSkipOut> {
    let title = lookup_text(cell(record, columns.title));
    if title.is_empty() {
        return Err(ImportSkipOut::new("Untitled", ImportSkipCode::NoTitle));
    }
    let status = match cell(record, columns.shelf).to_lowercase().as_str() {
        "to-read" => ShelfStatus::WantToRead,
        "currently-reading" => ShelfStatus::CurrentlyReading,
        "read" => ShelfStatus::Finished,
        _ => return Err(ImportSkipOut::new(title, ImportSkipCode::UnknownShelf)),
    };
    let isbn = Some(clean_isbn(cell(record, columns.isbn13)))
        .filter(|isbn| !isbn.is_empty())
        .unwrap_or_else(|| clean_isbn(cell(record, columns.isbn)));
    let finished = status == ShelfStatus::Finished;
    Ok(GoodreadsRow {
        title: title.to_owned(),
        authors: lookup_text(cell(record, columns.author)).to_owned(),
        isbn13: parse_isbn(&isbn).map(|isbn| isbn.as_str().to_owned()),
        status,
        rating: finished
            .then(|| parse_rating(cell(record, columns.rating)))
            .flatten(),
        read_on: finished
            .then(|| parse_date_read(cell(record, columns.date_read)))
            .flatten(),
    })
}

/// `clean_isbn`: unwraps the `="…"` formula Goodreads writes, keeps `[0-9Xx]` and returns the
/// result uppercased when it has 10 or 13 characters, else `""`. No checksum check (the lookup
/// checks it).
pub fn clean_isbn(value: &str) -> String {
    let unquote = |text: &str| -> String { text.trim_matches('"').trim_matches('\'').to_owned() };
    let mut text = unquote(py_strip(value));
    if let Some(rest) = text.strip_prefix('=') {
        text = unquote(py_strip(rest));
    }
    let kept: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .collect();
    if matches!(kept.len(), 10 | 13) {
        kept.to_uppercase()
    } else {
        String::new()
    }
}

/// `My Rating`: 1 to 5; Goodreads writes 0 for a book without a rating.
fn parse_rating(value: &str) -> Option<i32> {
    value
        .parse::<i32>()
        .ok()
        .filter(|rating| (1..=5).contains(rating))
}

/// `Date Read`: `YYYY/MM/DD` (the Goodreads export) or `YYYY-MM-DD`, within the personal date
/// range (`tz::personal_date_in_range`).
fn parse_date_read(value: &str) -> Option<Date> {
    let mut parts = value.split(['/', '-']);
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 {
        return None;
    }
    let month = Month::try_from(month.parse::<u8>().ok()?).ok()?;
    let date = Date::from_calendar_date(year.parse().ok()?, month, day.parse().ok()?).ok()?;
    tz::personal_date_in_range(date).then_some(date)
}

// ---------------------------------------------------------------------------------------------
// Routes

/// Starts importing a Goodreads export into the caller's shelf. The file is parsed at once; the
/// lookups run in the background (poll `GET /api/shelf/import/{job_id}`).
#[utoipa::path(
    post,
    path = "/api/shelf/import",
    operation_id = "shelf_start_import",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body(content = GoodreadsUpload, content_type = "multipart/form-data"),
    responses(
        (status = 202, body = ImportJobOut, description = "The queued import"),
        (status = 400, body = ErrorBody, description = "`Field required` (no `file` part), `That file is too large. Keep the export under 1.5 MB.`, `The file must be a UTF-8 Goodreads CSV export`, `That does not look like a Goodreads export`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 409, body = ErrorBody, description = "`An import is already running`, with that import as `job`")
    )
)]
pub async fn start_import(
    user: CurrentUser,
    State(state): State<AppState>,
    upload: ApiMultipart,
) -> ApiResult<(StatusCode, Json<ImportJobOut>)> {
    let raw = read_upload(upload).await?;
    let parsed = parse_goodreads_csv(&raw).map_err(AppError::bad_request)?;
    // The job row and its task start together in a task of their own. A client that goes away
    // drops this handler at an await; were the INSERT awaited here, the row could commit with
    // no task to run it, and the user's imports would answer 409 until the next restart.
    let user_id = user.id;
    let job = tokio::spawn(async move {
        let job = create_job(&state, user_id, &parsed).await?;
        tokio::spawn(run(state, job.id));
        Ok::<_, AppError>(job)
    })
    .await
    .map_err(AppError::internal)??;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// One of the caller's imports.
#[utoipa::path(
    get,
    path = "/api/shelf/import/{job_id}",
    operation_id = "shelf_get_import",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("job_id" = i64, Path, description = "The import")),
    responses(
        (status = 200, body = ImportJobOut, description = "The import and its progress"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such import` (also another user's)")
    )
)]
pub async fn get_import(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(job_id): ApiPath<i64>,
) -> ApiResult<Json<ImportJobOut>> {
    let mut conn = state.db.acquire().await?;
    let job = sqlx::query_as!(
        JobRow,
        r#"SELECT id, status AS "status: ImportStatus", total, processed, imported, skipped,
                  skips AS "skips: SqlJson<Vec<ImportSkipOut>>", error
           FROM import_jobs
           WHERE id = $1 AND user_id = $2"#,
        job_id,
        user.id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_SUCH_IMPORT))?;
    Ok(Json(job.into()))
}

/// The `file` part, read up to just over [`MAX_IMPORT_BYTES`] (enough to tell "too large"). No
/// `file` part, or a body that is not multipart, is 400 `Field required`; a body over the route
/// limit is the same 400 as a large file.
async fn read_upload(ApiMultipart(mut multipart): ApiMultipart) -> ApiResult<Vec<u8>> {
    while let Some(mut field) = multipart.next_field().await.map_err(upload_error)? {
        if field.name() != Some("file") {
            continue;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(upload_error)? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_IMPORT_BYTES {
                break;
            }
        }
        return Ok(bytes);
    }
    Err(AppError::bad_request(FIELD_REQUIRED))
}

fn upload_error(error: MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::bad_request(FILE_TOO_LARGE)
    } else {
        AppError::bad_request(FIELD_REQUIRED)
    }
}

/// Stores the parsed export as a queued job. 409 `An import is already running` (with that
/// import as `job`) when the user has a queued or running one (`import_jobs_one_active`).
async fn create_job(
    state: &AppState,
    user_id: i64,
    parsed: &ParsedExport,
) -> ApiResult<ImportJobOut> {
    let rows = serde_json::to_value(&parsed.rows).map_err(AppError::internal)?;
    let skips = serde_json::to_value(
        parsed
            .skips
            .iter()
            .take(SKIP_LIST_LIMIT)
            .collect::<Vec<_>>(),
    )
    .map_err(AppError::internal)?;
    let mut conn = state.db.acquire().await?;
    let inserted = sqlx::query_as!(
        JobRow,
        r#"INSERT INTO import_jobs (user_id, rows, total, skipped, skips)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, status AS "status: ImportStatus", total, processed, imported, skipped,
                     skips AS "skips: SqlJson<Vec<ImportSkipOut>>", error"#,
        user_id,
        rows,
        count(parsed.rows.len()),
        count(parsed.skips.len()),
        skips
    )
    .fetch_one(&mut *conn)
    .await;
    match inserted {
        Ok(job) => Ok(job.into()),
        Err(error) if is_unique_violation(&error, "import_jobs_one_active") => {
            let active = sqlx::query_as!(
                JobRow,
                r#"SELECT id, status AS "status: ImportStatus", total, processed, imported, skipped,
                          skips AS "skips: SqlJson<Vec<ImportSkipOut>>", error
                   FROM import_jobs
                   WHERE user_id = $1 AND status IN ('queued', 'running')"#,
                user_id
            )
            .fetch_optional(&mut *conn)
            .await?;
            let conflict = AppError::conflict(IMPORT_RUNNING);
            Err(match active {
                Some(job) => conflict.with("job", ImportJobOut::from(job)),
                None => conflict,
            })
        }
        Err(error) => Err(error.into()),
    }
}

/// A count as stored (the export is at most 1.5 MB, so it always fits).
fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

// ---------------------------------------------------------------------------------------------
// The job

/// Starts every `queued` or `running` import again after a restart, each at its `processed`
/// row. Called once at startup by `jobs::spawn_all`.
pub async fn resume(state: AppState) {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM import_jobs WHERE status IN ('queued', 'running') ORDER BY id"
    )
    .fetch_all(&state.db)
    .await;
    let ids = match ids {
        Ok(ids) => ids,
        Err(error) => {
            tracing::error!(%error, "cannot list the Goodreads imports to resume");
            return;
        }
    };
    for job_id in ids {
        tracing::info!(job_id, "resuming a Goodreads import");
        tokio::spawn(run(state.clone(), job_id));
    }
}

/// Runs import `job_id` to its end. An error or a panic is logged and marks the job `failed`;
/// the rows committed so far stay on the shelf.
async fn run(state: AppState, job_id: i64) {
    let error = match AssertUnwindSafe(process(&state, job_id))
        .catch_unwind()
        .await
    {
        Ok(Ok(())) => return,
        Ok(Err(error)) => error.to_string(),
        Err(_) => "the import task panicked".to_owned(),
    };
    tracing::error!(job_id, %error, "Goodreads import failed");
    let failed = sqlx::query!(
        "UPDATE import_jobs
         SET status = 'failed', error = $2, rows = '[]', finished_at = now()
         WHERE id = $1 AND status IN ('queued', 'running')",
        job_id,
        IMPORT_FAILED
    )
    .execute(&state.db)
    .await;
    if let Err(error) = failed {
        tracing::error!(job_id, %error, "cannot mark the Goodreads import as failed");
    }
}

/// A job this task has claimed.
struct Claimed {
    user_id: i64,
    rows: SqlJson<Vec<GoodreadsRow>>,
    processed: i32,
    timezone: String,
}

/// What the lookup of one row found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    Work {
        work_id: i64,
        edition_id: Option<i64>,
    },
    Skip(ImportSkipCode),
}

async fn process(state: &AppState, job_id: i64) -> sqlx::Result<()> {
    let claimed = sqlx::query_as!(
        Claimed,
        r#"UPDATE import_jobs AS j SET status = 'running'
           FROM users AS u
           WHERE j.id = $1 AND j.status IN ('queued', 'running') AND u.id = j.user_id
           RETURNING j.user_id, j.rows AS "rows: SqlJson<Vec<GoodreadsRow>>", j.processed,
                     u.timezone"#,
        job_id
    )
    .fetch_optional(&state.db)
    .await?;
    let Some(job) = claimed else {
        return Ok(());
    };
    let zone = tz::zone_or_utc(&job.timezone);
    let start = usize::try_from(job.processed).unwrap_or(usize::MAX);
    for (index, row) in job.rows.0.iter().enumerate().skip(start) {
        let found = lookup_row(state, row).await?;
        let mut tx = state.db.begin().await?;
        if !import_row(&mut tx, job_id, job.user_id, index, row, found, &zone).await? {
            // The job is gone (the account was deleted) or another task took it over.
            return Ok(());
        }
        tx.commit().await?;
    }
    sqlx::query!(
        "UPDATE import_jobs SET status = 'done', rows = '[]', finished_at = now()
         WHERE id = $1 AND status = 'running' AND processed >= total",
        job_id
    )
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Looks a row up, holding one of the [`LOOKUP_SLOTS`] and a pool connection only while its
/// lookups run. A throttled row is tried again: after Open Library's cooldown when one is running
/// and the wait fits into [`THROTTLE_BUDGET`], else up to [`QUEUE_RETRIES`] times after
/// [`QUEUE_PAUSE`]. When that runs out, the row is skipped as `catalog_unavailable`.
async fn lookup_row(state: &AppState, row: &GoodreadsRow) -> sqlx::Result<Found> {
    let started = Instant::now();
    let mut queue_retries = 0;
    loop {
        let found = {
            // The semaphore is never closed; without a slot the lookup would still be correct.
            let _slot = LOOKUP_SLOTS.acquire().await.ok();
            let mut conn = state.db.acquire().await?;
            find(state.lookup.as_ref(), &mut conn, row).await?
        };
        if let Some(found) = found {
            return Ok(found);
        }
        let pause = match state.outbound.ol_api.cooldown_remaining() {
            Some(left) => left + COOLDOWN_MARGIN,
            None if queue_retries < QUEUE_RETRIES => {
                queue_retries += 1;
                QUEUE_PAUSE
            }
            None => break,
        };
        if started.elapsed() + pause > THROTTLE_BUDGET {
            break;
        }
        tokio::time::sleep(pause).await;
    }
    Ok(Found::Skip(ImportSkipCode::CatalogUnavailable))
}

/// `lookup_work`: the ISBN first (local editions, then the remote route), then title and
/// author; the first hit wins. Only an ISBN hit names the edition. `None` when a lookup was
/// throttled and nothing hit (worth another try); `catalog_unavailable` when a lookup was
/// unavailable and nothing hit; `no_match` when every lookup answered.
async fn find(
    lookup: &dyn CatalogLookup,
    conn: &mut PgConnection,
    row: &GoodreadsRow,
) -> sqlx::Result<Option<Found>> {
    let (mut throttled, mut unavailable) = (false, false);
    let mut note = |error: LookupError| match error {
        LookupError::Throttled => {
            throttled = true;
            Ok(())
        }
        LookupError::Unavailable => {
            unavailable = true;
            Ok(())
        }
        LookupError::Db(error) => Err(error),
    };
    if let Some(isbn) = row.isbn13.as_deref().and_then(parse_isbn) {
        match lookup.resolve_isbn(conn, &isbn).await {
            Ok(Some(hit)) => {
                return Ok(Some(Found::Work {
                    work_id: hit.work_id,
                    edition_id: hit.edition_id,
                }));
            }
            Ok(None) => {}
            Err(error) => note(error)?,
        }
    }
    match lookup.resolve_text(conn, &row.title, &row.authors).await {
        Ok(Some(hit)) => {
            return Ok(Some(Found::Work {
                work_id: hit.work_id,
                edition_id: None,
            }));
        }
        Ok(None) => {}
        Err(error) => note(error)?,
    }
    Ok(if throttled {
        None
    } else if unavailable {
        Some(Found::Skip(ImportSkipCode::CatalogUnavailable))
    } else {
        Some(Found::Skip(ImportSkipCode::NoMatch))
    })
}

/// Shelves row `index` (or records its skip) and advances the job, in the caller's
/// transaction under the user's lock. `false` when the job is gone or no longer at `index`.
async fn import_row(
    conn: &mut PgConnection,
    job_id: i64,
    user_id: i64,
    index: usize,
    row: &GoodreadsRow,
    found: Found,
    zone: &jiff::tz::TimeZone,
) -> sqlx::Result<bool> {
    // The user first, then the job: account deletion locks the user before its rows go.
    let user = sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", user_id)
        .fetch_optional(&mut *conn)
        .await?;
    let processed = sqlx::query_scalar!(
        "SELECT processed FROM import_jobs WHERE id = $1 AND status = 'running' FOR UPDATE",
        job_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    if user.is_none() || processed != Some(count(index)) {
        return Ok(false);
    }
    let skip = match found {
        Found::Work {
            work_id,
            edition_id,
        } => shelve(conn, user_id, row, work_id, edition_id, zone).await?,
        Found::Skip(code) => Some(code),
    };
    let skip = skip
        .map(|code| serde_json::to_value(ImportSkipOut::new(row.title.clone(), code)))
        .transpose()
        .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
    sqlx::query!(
        "UPDATE import_jobs
         SET processed = processed + 1,
             imported = imported + $2,
             skipped = skipped + $3,
             skips = CASE WHEN $4::jsonb IS NULL OR jsonb_array_length(skips) >= $5 THEN skips
                          ELSE skips || jsonb_build_array($4::jsonb) END
         WHERE id = $1",
        job_id,
        i32::from(skip.is_none()),
        i32::from(skip.is_some()),
        skip,
        count(SKIP_LIST_LIMIT)
    )
    .execute(&mut *conn)
    .await?;
    Ok(true)
}

/// Puts the row's book on the shelf at the top of its column, with the row's rating and reading
/// date; `Some(already_on_shelf)` when the user has the work already.
async fn shelve(
    conn: &mut PgConnection,
    user_id: i64,
    row: &GoodreadsRow,
    work_id: i64,
    edition_id: Option<i64>,
    zone: &jiff::tz::TimeZone,
) -> sqlx::Result<Option<ImportSkipCode>> {
    let Some(work_id) = follow_merges(conn, work_id).await? else {
        return Ok(Some(ImportSkipCode::NoMatch));
    };
    let edition_id = match edition_id {
        Some(edition_id) => {
            sqlx::query_scalar!(
                "SELECT id FROM editions WHERE id = $1 AND work_id = $2",
                edition_id,
                work_id
            )
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let now = OffsetDateTime::now_utc();
    let mut fields = StatusFields::empty();
    let rating = row.rating.filter(|rating| (1..=5).contains(rating));
    apply_finish_fields(&mut fields, row.status, rating, None, None);
    apply_reading_dates(&mut fields, row.status, false, now);
    if row.status == ShelfStatus::Finished
        && let Some(date) = row.read_on
        && tz::personal_date_in_range(date)
        && date <= tz::today(zone)
    {
        let finished = tz::noon_local_utc(date, zone);
        fields.finished_at = Some(finished);
        if fields.started_at.is_some_and(|started| started > finished) {
            fields.started_at = Some(finished);
        }
    }
    let entry = NewEntry {
        user_id,
        work_id,
        edition_id,
        status: row.status,
        rating_visibility: Visibility::Audience,
        fields: &fields,
        now,
    };
    Ok(match insert_entry(conn, &entry).await? {
        Some(_) => None,
        None => Some(ImportSkipCode::AlreadyOnShelf),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn parse(csv: &str) -> ParsedExport {
        parse_goodreads_csv(csv.as_bytes()).unwrap()
    }

    #[test]
    fn clean_isbn_keeps_isbn_characters_only() {
        assert_eq!(clean_isbn(" '=\"0-316-55634-x\"' "), "031655634X");
        assert_eq!(clean_isbn("12345"), "");
        assert_eq!(clean_isbn("=\"978031655634\""), "");
    }

    #[test]
    fn headers_are_trimmed_and_case_insensitive() {
        let parsed = parse(" title ,EXCLUSIVE SHELF\nDune,to-read\n");
        // The header check trims, but the cell lookup does not: " title " is not "Title".
        assert_eq!(
            parsed.skips,
            vec![ImportSkipOut::new("Untitled", ImportSkipCode::NoTitle)]
        );
        let parsed = parse("TITLE,Exclusive Shelf\nDune,TO-READ\n");
        assert_eq!(parsed.rows[0].title, "Dune");
        assert_eq!(parsed.rows[0].status, ShelfStatus::WantToRead);
    }

    #[test]
    fn rejects_what_is_not_an_export() {
        assert_eq!(parse_goodreads_csv(b""), Err(NOT_GOODREADS));
        assert_eq!(parse_goodreads_csv(b"Title\nDune\n"), Err(NOT_GOODREADS));
        assert_eq!(parse_goodreads_csv(b"Title,Shelf\xff\n"), Err(NOT_UTF8));
        let large = vec![b'x'; MAX_IMPORT_BYTES + 1];
        assert_eq!(parse_goodreads_csv(&large), Err(FILE_TOO_LARGE));
    }

    #[test]
    fn bom_blank_lines_short_and_long_rows() {
        let parsed = parse(
            "\u{feff}Title,Exclusive Shelf,Author\n\nDune,read\n\nEmma,to-read,Austen,extra,cells\n",
        );
        assert_eq!(parsed.skips, vec![]);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].authors, "");
        assert_eq!(parsed.rows[1].authors, "Austen");
    }

    #[test]
    fn long_titles_and_authors_are_cut_for_the_lookup() {
        // Two bytes per character: 600 KB in one cell, twice in the file.
        let title = "é".repeat(300_000);
        // 299 letters, a space, then more: the cut ends on the space, which is trimmed.
        let author = format!("{} Austen", "a".repeat(MAX_LOOKUP_CHARS - 1));
        let parsed = parse(&format!(
            "Title,Author,Exclusive Shelf\n{title},{author},to-read\n{title},,dnf\n"
        ));
        assert_eq!(parsed.rows[0].title, "é".repeat(MAX_LOOKUP_CHARS));
        assert_eq!(parsed.rows[0].authors, "a".repeat(MAX_LOOKUP_CHARS - 1));
        assert_eq!(
            parsed.skips,
            vec![ImportSkipOut::new(
                "é".repeat(MAX_LOOKUP_CHARS),
                ImportSkipCode::UnknownShelf
            )]
        );
    }

    #[test]
    fn stops_after_400_rows_with_one_skip() {
        let mut csv = String::from("Title,Exclusive Shelf\n");
        for n in 0..405 {
            csv.push_str(&format!("Book {n},to-read\n"));
        }
        let parsed = parse(&csv);
        assert_eq!(parsed.rows.len(), 400);
        assert_eq!(
            parsed.skips,
            vec![ImportSkipOut::new("", ImportSkipCode::StoppedAfter400)]
        );
    }

    #[test]
    fn rating_and_date_read_only_for_read_rows() {
        let parsed = parse(
            "Title,Exclusive Shelf,My Rating,Date Read\n\
             A,read,4,2024/02/03\n\
             B,read,0,\n\
             C,to-read,5,2024/02/03\n\
             D,read,6,2024-13-01\n\
             E,read,x,0000/01/01\n",
        );
        let got: Vec<_> = parsed
            .rows
            .iter()
            .map(|row| (row.title.as_str(), row.rating, row.read_on))
            .collect();
        assert_eq!(
            got,
            vec![
                ("A", Some(4), Some(time::macros::date!(2024 - 02 - 03))),
                ("B", None, None),
                ("C", None, None),
                ("D", None, None),
                ("E", None, None),
            ]
        );
    }

    #[test]
    fn isbn13_column_wins_and_bad_checksums_are_dropped() {
        let parsed = parse(
            "Title,Exclusive Shelf,ISBN,ISBN13\n\
             A,read,=\"0316556343\",=\"\"\n\
             B,read,=\"0316556343\",=\"9780316556347\"\n\
             C,read,,=\"9780316556340\"\n",
        );
        let got: Vec<_> = parsed
            .rows
            .iter()
            .map(|row| row.isbn13.as_deref())
            .collect();
        assert_eq!(
            got,
            vec![Some("9780316556347"), Some("9780316556347"), None]
        );
    }

    #[test]
    fn skip_codes_serialise_with_the_documented_names() {
        let codes: Vec<Value> = [
            ImportSkipCode::StoppedAfter400,
            ImportSkipCode::NoTitle,
            ImportSkipCode::UnknownShelf,
            ImportSkipCode::NoMatch,
            ImportSkipCode::CatalogUnavailable,
            ImportSkipCode::AlreadyOnShelf,
        ]
        .into_iter()
        .map(|code| serde_json::to_value(code).unwrap())
        .collect();
        assert_eq!(
            codes,
            vec![
                "stopped_after_400",
                "no_title",
                "unknown_shelf",
                "no_match",
                "catalog_unavailable",
                "already_on_shelf"
            ]
        );
    }
}
