//! Blocking (docs/phase3/work-plan.md §4.3).
//!
//! A block is silent: the blocked account is never told, and no answer it gets differs from the
//! answers of an account that was simply never accepted. Its requests are still stored and show
//! as sent on its side, but never reach the blocker. A block ends the friendship and the pair's
//! open friend invites, and nothing else: in a club both are in, now or later, they are each
//! other's audience as before. A block files no report.
//!
//! Unblocking deletes the block and the requests the blocked account sent meanwhile, so it never
//! surfaces an old request. It acts only on the caller's own rows and always answers 200.

use axum::Json;
use axum::extract::State;

use super::out::{FriendsOut, friends_out};
use super::{PathName, account_id, lock_pair, revoke_pair_invites};
use crate::AppState;
use crate::auth::account::account_key;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, AppError, ErrorBody, msg};

/// Blocks `username`: the friendship ends, pending requests in either direction go, the pair's
/// open friend invites are revoked, and later requests from them stay hidden from the caller.
/// Blocking twice is harmless. Counts against the 20 requests and blocks a day, unless the
/// caller and that account already have a request or a friendship (blocking any number of
/// requesters is free).
#[utoipa::path(
    put,
    path = "/api/friends/blocks/{username}",
    operation_id = "friends_block",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The account to block")),
    responses(
        (status = 200, body = FriendsOut, description = "Blocked; the caller's friends"),
        (status = 400, body = ErrorBody, description = "`That is your own account`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No account with that name`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn block(
    user: CurrentUser,
    State(state): State<AppState>,
    PathName(name): PathName,
) -> ApiResult<Json<FriendsOut>> {
    let count = || {
        state
            .limits
            .friend_request_user
            .check_and_hit(&account_key(user.id))
            .map_err(|wait| wait.into_error(msg::SLOW_DOWN))
    };
    let unknown = || AppError::not_found(msg::NO_ACCOUNT);
    let mut tx = state.db.begin().await?;
    let Some(other) = account_id(&mut tx, name.as_deref()).await? else {
        count()?;
        return Err(unknown());
    };
    if other == user.id {
        return Err(AppError::bad_request(msg::SELF_ACCOUNT));
    }
    if !lock_pair(&mut tx, user.id, other).await? {
        count()?;
        return Err(unknown());
    }
    let disclosed = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM friendships
               WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
                 AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)
           ) AS "disclosed!""#,
        user.id,
        other
    )
    .fetch_one(&mut *tx)
    .await?;
    if !disclosed {
        count()?;
    }
    sqlx::query!(
        "INSERT INTO user_blocks (user_id, blocked_user_id) VALUES ($1, $2)
         ON CONFLICT (user_id, blocked_user_id) DO NOTHING",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM friendships
         WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
           AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?;
    revoke_pair_invites(&mut tx, user.id, other).await?;
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Unblocks `username` and drops the requests they sent while blocked. Always 200 with the
/// caller's friends: for an account the caller never blocked and for an unknown name too.
#[utoipa::path(
    delete,
    path = "/api/friends/blocks/{username}",
    operation_id = "friends_unblock",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The account to unblock")),
    responses(
        (status = 200, body = FriendsOut, description = "Not blocked (any more); the caller's friends"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn unblock(
    user: CurrentUser,
    State(state): State<AppState>,
    PathName(name): PathName,
) -> ApiResult<Json<FriendsOut>> {
    let mut tx = state.db.begin().await?;
    let other = account_id(&mut tx, name.as_deref())
        .await?
        .filter(|id| *id != user.id);
    if let Some(other) = other
        && lock_pair(&mut tx, user.id, other).await?
    {
        let unblocked = sqlx::query!(
            "DELETE FROM user_blocks WHERE user_id = $1 AND blocked_user_id = $2",
            user.id,
            other
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if unblocked > 0 {
            // While the block stood, the pair's only row could be a hidden request from them.
            sqlx::query!(
                "DELETE FROM friendships
                 WHERE requester_id = $1 AND addressee_id = $2 AND accepted_at IS NULL",
                other,
                user.id
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
