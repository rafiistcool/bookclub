//! Favourites: up to three works per user, in order (endpoint-map §2,
//! docs/phase1/spec/shelf.md §1.9–§1.12).
//!
//! The routes live under `/api/auth/me/favorites` but belong to the shelf. Every write locks
//! the user's row first, so two concurrent adds for the last slot give one 200 and one 409.
//! Positions are always 1..n: a removal (or a merge that dropped a favourite) is compacted
//! before the next add, so the legacy gap bug cannot happen.

use std::collections::HashSet;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::lock_user;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{BookRef, book_outs, follow_merges, follow_merges_many};
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError, ErrorBody, is_unique_violation, msg};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};

/// A user has at most this many favourites.
pub const MAX_FAVORITES: usize = 3;

/// One favourite.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FavoriteOut {
    /// 1 to 3.
    pub position: i32,
    pub book: BookOut,
}

/// The caller's favourites, by position.
#[derive(Debug, Serialize, ToSchema)]
pub struct FavoritesOut {
    pub items: Vec<FavoriteOut>,
}

/// The new list of favourites, in order. An omitted list clears them.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct FavoritesIn {
    /// At most three distinct work ids (legacy `book_ids`).
    #[serde(default)]
    pub work_ids: Vec<i64>,
}

impl Validate for FavoritesIn {
    /// The legacy validator: the first failure wins, and every message is the app's own (no
    /// generic length error), in this order.
    fn validate(&mut self, v: &mut Validator) {
        let distinct: HashSet<i64> = self.work_ids.iter().copied().collect();
        if self.work_ids.len() > MAX_FAVORITES {
            v.fail(msg::AT_MOST_3_FAVOURITES);
        } else if distinct.len() != self.work_ids.len() {
            v.fail(msg::FAVOURITE_TWICE);
        } else if self.work_ids.iter().any(|&id| id <= 0) {
            v.fail(msg::BOOK_NOT_IN_CATALOG);
        }
    }
}

/// The caller's favourites.
#[utoipa::path(
    get,
    path = "/api/auth/me/favorites",
    operation_id = "shelf_list_favorites",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = FavoritesOut, description = "The favourites, by position"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list_favorites(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<FavoritesOut>> {
    let mut conn = state.db.acquire().await?;
    let items = favorite_outs(&mut conn, user.id).await?;
    Ok(Json(FavoritesOut { items }))
}

/// Replaces the caller's favourites with `work_ids`, in that order. The works need not be on
/// the caller's shelf.
#[utoipa::path(
    put,
    path = "/api/auth/me/favorites",
    operation_id = "shelf_replace_favorites",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = FavoritesIn,
    responses(
        (status = 200, body = FavoritesOut, description = "The new favourites"),
        (status = 400, body = ErrorBody, description = "`At most 3 favourites`, `Each book can only be a favourite once`, `That book is not in the catalog`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn replace_favorites(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<FavoritesIn>,
) -> ApiResult<Json<FavoritesOut>> {
    let mut tx = state.db.begin().await?;
    let work_ids = current_works(&mut tx, &body.work_ids).await?;
    lock_user(&mut tx, user.id).await?;
    replace(&mut tx, user.id, &work_ids).await?;
    let items = favorite_outs(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(FavoritesOut { items }))
}

/// Appends a work to the caller's favourites. A work that already is one leaves the list as it
/// is (200).
#[utoipa::path(
    post,
    path = "/api/auth/me/favorites/{work_id}",
    operation_id = "shelf_add_favorite",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = i64, Path, description = "The work (numeric id)")),
    responses(
        (status = 200, body = FavoritesOut, description = "The favourites"),
        (status = 400, body = ErrorBody, description = "`That book is not in the catalog`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 409, body = ErrorBody, description = "`You already have 3 favourites`")
    )
)]
pub async fn add_favorite(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(work_id): ApiPath<i64>,
) -> ApiResult<Json<FavoritesOut>> {
    let mut tx = state.db.begin().await?;
    let work_id = follow_merges(&mut tx, work_id)
        .await?
        .ok_or_else(|| AppError::bad_request(msg::BOOK_NOT_IN_CATALOG))?;
    lock_user(&mut tx, user.id).await?;
    let current = favorite_works(&mut tx, user.id).await?;
    if !current.contains(&work_id) {
        if current.len() >= MAX_FAVORITES {
            return Err(AppError::conflict(msg::ALREADY_3_FAVOURITES));
        }
        compact(&mut tx, user.id).await?;
        let position = i32::try_from(current.len() + 1).unwrap_or(i32::MAX);
        let inserted = sqlx::query!(
            "INSERT INTO user_favorites (user_id, work_id, position) VALUES ($1, $2, $3)",
            user.id,
            work_id,
            position
        )
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            // Unreachable under the user lock; kept as the legacy race answer, never a 500.
            if is_unique_violation(&error, "user_favorites_pkey")
                || is_unique_violation(&error, "user_favorites_position_key")
            {
                drop(tx);
                return lost_race(&state, user.id, work_id).await;
            }
            return Err(error.into());
        }
    }
    let items = favorite_outs(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(FavoritesOut { items }))
}

/// After a refused insert: the list when the work is a favourite now, else 409.
async fn lost_race(state: &AppState, user_id: i64, work_id: i64) -> ApiResult<Json<FavoritesOut>> {
    let mut conn = state.db.acquire().await?;
    if favorite_works(&mut conn, user_id).await?.contains(&work_id) {
        let items = favorite_outs(&mut conn, user_id).await?;
        Ok(Json(FavoritesOut { items }))
    } else {
        Err(AppError::conflict(msg::ALREADY_3_FAVOURITES))
    }
}

/// Removes a work from the caller's favourites and closes the gap. A work that is not one
/// (or does not exist) leaves the list as it is (200).
#[utoipa::path(
    delete,
    path = "/api/auth/me/favorites/{work_id}",
    operation_id = "shelf_remove_favorite",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = i64, Path, description = "The work (numeric id)")),
    responses(
        (status = 200, body = FavoritesOut, description = "The favourites"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn remove_favorite(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(work_id): ApiPath<i64>,
) -> ApiResult<Json<FavoritesOut>> {
    let mut tx = state.db.begin().await?;
    let work_id = follow_merges(&mut tx, work_id).await?.unwrap_or(work_id);
    lock_user(&mut tx, user.id).await?;
    let removed = sqlx::query!(
        "DELETE FROM user_favorites WHERE user_id = $1 AND work_id = $2",
        user.id,
        work_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed > 0 {
        compact(&mut tx, user.id).await?;
    }
    let items = favorite_outs(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(FavoritesOut { items }))
}

/// The current work of each id, in order: 400 `That book is not in the catalog` for an unknown
/// id, `Each book can only be a favourite once` when two ids are (merged into) one work.
async fn current_works(conn: &mut PgConnection, work_ids: &[i64]) -> ApiResult<Vec<i64>> {
    if work_ids.is_empty() {
        return Ok(Vec::new());
    }
    let current = follow_merges_many(conn, work_ids).await?;
    let resolved: Vec<i64> = work_ids
        .iter()
        .map(|id| current.get(id).copied())
        .collect::<Option<_>>()
        .ok_or_else(|| AppError::bad_request(msg::BOOK_NOT_IN_CATALOG))?;
    let distinct: HashSet<i64> = resolved.iter().copied().collect();
    if distinct.len() != resolved.len() {
        return Err(AppError::bad_request(msg::FAVOURITE_TWICE));
    }
    Ok(resolved)
}

/// Replaces the user's favourites with `work_ids` at positions 1..n (one transaction, under the
/// caller's user lock).
async fn replace(conn: &mut PgConnection, user_id: i64, work_ids: &[i64]) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM user_favorites WHERE user_id = $1", user_id)
        .execute(&mut *conn)
        .await?;
    let positions: Vec<i32> = (1..=work_ids.len())
        .map(|p| i32::try_from(p).unwrap_or(i32::MAX))
        .collect();
    sqlx::query!(
        "INSERT INTO user_favorites (user_id, work_id, position)
         SELECT $1, work_id, position FROM unnest($2::int8[], $3::int4[]) AS f (work_id, position)",
        user_id,
        work_ids,
        &positions
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Renumbers the user's favourites 1..n in their order. The position key is deferrable, so one
/// statement can shift them.
async fn compact(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE user_favorites AS f SET position = r.rank
         FROM (SELECT work_id, row_number() OVER (ORDER BY position, work_id)::int4 AS rank
               FROM user_favorites WHERE user_id = $1) AS r
         WHERE f.user_id = $1 AND f.work_id = r.work_id AND f.position <> r.rank",
        user_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The user's favourite work ids, by position.
async fn favorite_works(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        "SELECT work_id FROM user_favorites WHERE user_id = $1 ORDER BY position, work_id",
        user_id
    )
    .fetch_all(conn)
    .await
}

/// The user's favourites, by position. Callers check that the viewer may see this user.
pub async fn favorite_outs(
    conn: &mut PgConnection,
    user_id: i64,
) -> sqlx::Result<Vec<FavoriteOut>> {
    let rows = sqlx::query!(
        "SELECT work_id, position FROM user_favorites WHERE user_id = $1 ORDER BY position, work_id",
        user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = rows.iter().map(|row| BookRef::work(row.work_id)).collect();
    let mut books = book_outs(conn, &refs).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let book = books.remove(&BookRef::work(row.work_id))?;
            Some(FavoriteOut {
                position: row.position,
                book,
            })
        })
        .collect())
}
