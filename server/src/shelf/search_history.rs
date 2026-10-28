//! The caller's search history: the last 20 Discover searches, deduplicated by Unicode case
//! fold (endpoint-map §7, docs/phase1/spec/diary.md §1.16–§1.19, §2.6 and R25).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::lock_user;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};
use crate::text::{casefold, char_len, collapse_ws};

pub const SEARCH_LENGTH: &str = "Search must contain between 1 and 300 characters";
/// A user keeps at most this many searches.
pub const HISTORY_LIMIT: i64 = 20;
const MAX_QUERY_CHARS: usize = 300;

/// A search to remember.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchHistoryIn {
    /// Whitespace runs collapse to one space; 1 to 300 characters after that.
    pub query: String,
}

impl Validate for SearchHistoryIn {
    fn validate(&mut self, v: &mut Validator) {
        self.query = collapse_ws(&self.query);
        v.check(
            (1..=MAX_QUERY_CHARS).contains(&char_len(&self.query)),
            SEARCH_LENGTH,
        );
    }
}

/// One remembered search.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHistoryOut {
    pub id: i64,
    /// The latest spelling of the search.
    pub query: String,
    #[serde(with = "time::serde::rfc3339")]
    pub last_used_at: OffsetDateTime,
}

/// The caller's searches, most recent first.
#[utoipa::path(
    get,
    path = "/api/search-history",
    operation_id = "shelf_list_search_history",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = Vec<SearchHistoryOut>, description = "Most recent first"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list_history(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<SearchHistoryOut>>> {
    let mut conn = state.db.acquire().await?;
    Ok(Json(history(&mut conn, user.id, None).await?))
}

/// Remembers a search. A search that differs only by case (Unicode case fold) or spacing from
/// a remembered one keeps its id and takes the new spelling. Only the 20 most recent stay: the
/// upsert and the pruning run in one transaction under the user lock.
#[utoipa::path(
    post,
    path = "/api/search-history",
    operation_id = "shelf_remember_search",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = SearchHistoryIn,
    responses(
        (status = 200, body = Vec<SearchHistoryOut>, description = "The updated history, most recent first"),
        (status = 400, body = ErrorBody, description = "`Search must contain between 1 and 300 characters`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn remember_search(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<SearchHistoryIn>,
) -> ApiResult<Json<Vec<SearchHistoryOut>>> {
    let mut tx = state.db.begin().await?;
    // Two searches remembered at once would each prune what they see and could leave 21 rows.
    lock_user(&mut tx, user.id).await?;
    sqlx::query!(
        "INSERT INTO search_history (user_id, query, normalized_query, last_used_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT ON CONSTRAINT search_history_user_query_key
         DO UPDATE SET query = EXCLUDED.query, last_used_at = EXCLUDED.last_used_at",
        user.id,
        body.query,
        casefold(&body.query)
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM search_history WHERE id IN (
             SELECT id FROM search_history WHERE user_id = $1
             ORDER BY last_used_at DESC, id DESC OFFSET $2)",
        user.id,
        HISTORY_LIMIT
    )
    .execute(&mut *tx)
    .await?;
    let items = history(&mut tx, user.id, Some(HISTORY_LIMIT)).await?;
    tx.commit().await?;
    Ok(Json(items))
}

/// Forgets every search of the caller.
#[utoipa::path(
    delete,
    path = "/api/search-history",
    operation_id = "shelf_clear_search_history",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "Cleared"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn clear_history(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    sqlx::query!("DELETE FROM search_history WHERE user_id = $1", user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Forgets one of the caller's searches.
#[utoipa::path(
    delete,
    path = "/api/search-history/{entry_id}",
    operation_id = "shelf_delete_search",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("entry_id" = i64, Path, description = "The search history entry")),
    responses(
        (status = 204, description = "Forgotten"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Search history entry not found` (also another user's)")
    )
)]
pub async fn delete_history_entry(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(entry_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let deleted = sqlx::query!(
        "DELETE FROM search_history WHERE id = $1 AND user_id = $2",
        entry_id,
        user.id
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(AppError::not_found(msg::SEARCH_HISTORY_NOT_FOUND));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The user's searches, most recent first, at most `limit`.
async fn history(
    conn: &mut PgConnection,
    user_id: i64,
    limit: Option<i64>,
) -> sqlx::Result<Vec<SearchHistoryOut>> {
    sqlx::query_as!(
        SearchHistoryOut,
        "SELECT id, query, last_used_at FROM search_history
         WHERE user_id = $1
         ORDER BY last_used_at DESC, id DESC
         LIMIT $2",
        user_id,
        limit
    )
    .fetch_all(conn)
    .await
}
