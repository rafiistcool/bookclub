//! `GET /api/books/new-releases` (endpoint-map §5): upcoming and recent titles from the local
//! catalog, which the harvester fills. No remote calls.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use sqlx::PgConnection;
use time::{Date, Duration};
use utoipa::IntoParams;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::hits::{ListedHit, annotated_hits};
use crate::catalog::shared::books::BookRef;
use crate::dto::SearchPage;
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

/// One page of new releases around `today`, annotated for `viewer` like search hits
/// (`catalog::hits::annotated_hits`). A deleted DNB header withdraws every edition of its record
/// (`ingest::withdraw_dnb_edition`), so a cancelled multi-ISBN announcement leaves the list with
/// all its editions.
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

    let hits: Vec<ListedHit> = rows
        .iter()
        .map(|row| ListedHit {
            book: BookRef {
                work_id: row.work_id,
                edition_id: Some(row.edition_id),
            },
            isbn: row.isbn13.clone(),
            planned_publication_date: Some(row.planned),
        })
        .collect();
    let items = annotated_hits(conn, viewer, &hits).await?;
    Ok(SearchPage {
        items,
        page: params.page,
        has_more,
    })
}
