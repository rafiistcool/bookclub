//! `GET /api/books/new-releases` (endpoint-map §5): upcoming and recent titles from the local
//! catalog, which the harvester fills. No remote calls.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use sqlx::PgConnection;
use time::{Date, Duration};
use utoipa::IntoParams;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::dto::{ClubRef, SearchHit, SearchPage};
use crate::error::{ApiResult, ErrorBody};
use crate::extract::{ApiQuery, Validate, Validator};
use crate::tz;

/// The window starts this many days before today…
pub const DAYS_BACK: i64 = 30;
/// …and ends this many days after it.
pub const DAYS_AHEAD: i64 = 120;
/// Largest page, as for search.
pub const MAX_LIMIT: i64 = 40;
/// pydantic's message for a `language` that is not three lowercase letters.
pub const LANGUAGE_PATTERN: &str = "String should match pattern '^[a-z]{3}$'";

fn german() -> String {
    "ger".to_owned()
}

fn first_page() -> i32 {
    1
}

fn default_limit() -> i32 {
    24
}

/// `GET /api/books/new-releases` parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NewReleasesParams {
    /// The editions' language, an ISO 639-2/B code such as `ger` (the default) or `eng`.
    #[serde(default = "german")]
    pub language: String,
    /// 1 or more.
    #[serde(default = "first_page")]
    pub page: i32,
    /// 1 to 40, default 24.
    #[serde(default = "default_limit")]
    pub limit: i32,
}

impl Validate for NewReleasesParams {
    fn validate(&mut self, v: &mut Validator) {
        let ok = self.language.len() == 3 && self.language.bytes().all(|b| b.is_ascii_lowercase());
        v.check(ok, LANGUAGE_PATTERN);
        v.range_i64(Some(self.page.into()), 1, i64::MAX);
        v.range_i64(Some(self.limit.into()), 1, MAX_LIMIT);
    }
}

/// Upcoming and recent titles: editions planned between today − 30 days and today + 120 days
/// (today in the caller's time zone), not withdrawn, in the given language; one hit per work
/// (its earliest such edition, whose title and cover the hit shows), ordered by that date, then
/// work id. `planned_publication_date` is that date.
#[utoipa::path(
    get,
    path = "/api/books/new-releases",
    operation_id = "harvest_new_releases",
    tag = "harvest",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(NewReleasesParams),
    responses(
        (status = 200, body = SearchPage, description = "A page of new releases; `has_more` set"),
        (status = 400, body = ErrorBody, description = "A parameter error"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn new_releases(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<NewReleasesParams>,
) -> ApiResult<Json<SearchPage>> {
    let today = tz::today(&tz::zone_or_utc(&user.timezone));
    let mut conn = state.db.acquire().await?;
    Ok(Json(
        new_releases_page(&mut conn, user.id, today, &params).await?,
    ))
}

/// One page of new releases around `today`, annotated for `viewer` like search hits.
pub async fn new_releases_page(
    conn: &mut PgConnection,
    viewer: i64,
    today: Date,
    params: &NewReleasesParams,
) -> sqlx::Result<SearchPage> {
    let limit = i64::from(params.limit);
    let offset = (i64::from(params.page) - 1) * limit;
    let rows = sqlx::query!(
        r#"SELECT work_id AS "work_id!", edition_id AS "edition_id!", isbn13,
                  planned AS "planned!"
           FROM (
               SELECT DISTINCT ON (e.work_id)
                      e.work_id, e.id AS edition_id, e.isbn13, e.planned_publication_date AS planned
               FROM editions e
               JOIN works w ON w.id = e.work_id
               WHERE e.planned_publication_date BETWEEN $1 AND $2
                 AND e.withdrawn_at IS NULL
                 AND e.language = $3
                 AND w.merged_into_id IS NULL
               ORDER BY e.work_id, e.planned_publication_date, e.id
           ) firsts
           ORDER BY planned, work_id
           LIMIT $4 OFFSET $5"#,
        today - Duration::days(DAYS_BACK),
        today + Duration::days(DAYS_AHEAD),
        params.language,
        limit + 1,
        offset
    )
    .fetch_all(&mut *conn)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(params.limit as usize)];

    let refs: Vec<BookRef> = rows
        .iter()
        .map(|row| BookRef {
            work_id: row.work_id,
            edition_id: Some(row.edition_id),
        })
        .collect();
    let work_ids: Vec<i64> = rows.iter().map(|row| row.work_id).collect();
    let mut books = book_outs(conn, &refs).await?;
    let shelf = shelf_entries(conn, viewer, &work_ids).await?;
    let mut picks = club_picks(conn, viewer, &work_ids).await?;
    let items = rows
        .iter()
        .zip(&refs)
        .filter_map(|(row, book_ref)| {
            let book = books.remove(book_ref)?;
            let shelf_id = shelf.get(&row.work_id).copied();
            Some(SearchHit {
                id: book.id,
                ol_work_key: book.ol_work_key,
                title: book.title,
                authors: book.authors,
                year: book.year,
                cover_url: book.cover_url,
                cover_thumb_url: book.cover_thumb_url,
                cover_public_id: book.cover_public_id,
                isbn: row.isbn13.clone(),
                source: book.source,
                custom: book.custom,
                on_shelf: shelf_id.is_some(),
                shelf_id,
                club_picks: picks.remove(&row.work_id).unwrap_or_default(),
                planned_publication_date: Some(row.planned),
            })
        })
        .collect();
    Ok(SearchPage {
        items,
        page: params.page,
        has_more,
    })
}

/// The viewer's shelf entry id per work.
async fn shelf_entries(
    conn: &mut PgConnection,
    viewer: i64,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, i64>> {
    let rows = sqlx::query!(
        "SELECT work_id, id FROM shelf_entries WHERE user_id = $1 AND work_id = ANY($2)",
        viewer,
        work_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.work_id, row.id)).collect())
}

/// Per work, the viewer's clubs (`bc_member_clubs`) whose current pick it is, newest membership
/// first: the same annotation as search hits.
async fn club_picks(
    conn: &mut PgConnection,
    viewer: i64,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, Vec<ClubRef>>> {
    let rows = sqlx::query!(
        r#"SELECT p.work_id, c.slug, c.name
           FROM club_picks p
           JOIN bc_member_clubs($1) AS mc ON mc.club_id = p.club_id
           JOIN clubs c ON c.id = p.club_id
           WHERE p.ended_at IS NULL AND p.work_id = ANY($2)
           ORDER BY mc.joined_at DESC, c.id"#,
        viewer,
        work_ids
    )
    .fetch_all(conn)
    .await?;
    let mut picks: HashMap<i64, Vec<ClubRef>> = HashMap::new();
    for row in rows {
        picks.entry(row.work_id).or_default().push(ClubRef {
            slug: row.slug,
            name: row.name,
        });
    }
    Ok(picks)
}
