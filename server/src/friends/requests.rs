//! Friend requests and friendships (docs/phase3/work-plan.md §4.2): send, accept, decline or
//! cancel, and remove. Each write runs in one transaction under [`super::lock_pair`] and answers
//! with the caller's whole [`FriendsOut`].
//!
//! | Action (A about B) | From | To | Answer |
//! |---|---|---|---|
//! | send | none | pending A→B (hidden from B if B blocked A) | 200 |
//! | send | pending A→B, or friends | unchanged | 200 |
//! | send | pending B→A | friends | 200 |
//! | send | A blocked B | unchanged | 409 `You blocked this account` |
//! | accept | pending B→A, not hidden | friends | 200 |
//! | decline | pending B→A, not hidden | none | 200 |
//! | cancel | pending A→B, hidden or not | none | 200 |
//! | remove | friends | none, the pair's open friend invites revoked | 200 |
//!
//! Anything else is 404: `No such friend request` for accept, decline and cancel, `No friend with
//! that name` for remove. A declined request leaves no trace; the sender sees it disappear as
//! after a cancel and may ask again. Requests never expire.
//!
//! Only send tells an unknown name from an existing one, and it is counted (§4.9). Accept,
//! decline, cancel and remove are not, so they answer an unknown name only after the same work
//! as an existing one ([`super::NO_ACCOUNT`]).

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use utoipa::ToSchema;

use super::out::{FriendsOut, friends_out};
use super::{NO_ACCOUNT, PathName, account_id, count_request, lock_pair, revoke_pair_invites};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::rules::normalize_username;
use crate::client_ip::ClientIp;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate, Validator};

/// A friend request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FriendRequestIn {
    /// The other account's exact username (case and surrounding spaces are ignored).
    pub username: String,
}

impl Validate for FriendRequestIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.username = normalize_username(&self.username);
    }
}

/// Asks another account to be friends. Sending to someone who already asked the caller makes
/// them friends at once; sending again, or to a friend, changes nothing. A request to someone
/// who blocked the caller is stored and shows as sent, but never reaches them. 20 requests and
/// blocks per account a day, unknown names and repeats included, and 60 per client address a
/// day across its accounts.
#[utoipa::path(
    post,
    path = "/api/friends/requests",
    operation_id = "friends_send_request",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = FriendRequestIn,
    responses(
        (status = 200, body = FriendsOut, description = "Sent (or now friends); the caller's friends"),
        (status = 400, body = ErrorBody, description = "`Field required`, `That is your own account`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No account with that name`"),
        (status = 409, body = ErrorBody, description = "`You blocked this account`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn send(
    user: CurrentUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<FriendRequestIn>,
) -> ApiResult<Json<FriendsOut>> {
    count_request(&state.limits, user.id, ip)?;
    let mut tx = state.db.begin().await?;
    let unknown = || AppError::not_found(msg::NO_ACCOUNT);
    let other = account_id(&mut tx, Some(&body.username))
        .await?
        .ok_or_else(unknown)?;
    if other == user.id {
        return Err(AppError::bad_request(msg::SELF_ACCOUNT));
    }
    if !lock_pair(&mut tx, user.id, other).await? {
        return Err(unknown());
    }
    let blocked = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM user_blocks WHERE user_id = $1 AND blocked_user_id = $2)
           AS "blocked!""#,
        user.id,
        other
    )
    .fetch_one(&mut *tx)
    .await?;
    if blocked {
        return Err(AppError::conflict(msg::YOU_BLOCKED));
    }
    sqlx::query!(
        "INSERT INTO friendships (requester_id, addressee_id) VALUES ($1, $2)
         ON CONFLICT ((least(requester_id, addressee_id)), (greatest(requester_id, addressee_id)))
         DO NOTHING",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query!(
        "SELECT requester_id, accepted_at FROM friendships
         WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
           AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)
         FOR UPDATE",
        user.id,
        other
    )
    .fetch_one(&mut *tx)
    .await?;
    if row.requester_id == other && row.accepted_at.is_none() {
        // They asked first: the request back accepts theirs. clock_timestamp(), not now(): their
        // request may have been made after this transaction began.
        sqlx::query!(
            "UPDATE friendships SET accepted_at = clock_timestamp()
             WHERE requester_id = $1 AND addressee_id = $2",
            other,
            user.id
        )
        .execute(&mut *tx)
        .await?;
    }
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Accepts a request from `username`: from now on they are friends. A request the caller cannot
/// see (from an account the caller blocked) cannot be accepted.
#[utoipa::path(
    post,
    path = "/api/friends/requests/{username}/accept",
    operation_id = "friends_accept_request",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "Who sent the request")),
    responses(
        (status = 200, body = FriendsOut, description = "Now friends; the caller's friends"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such friend request`")
    )
)]
pub async fn accept(
    user: CurrentUser,
    State(state): State<AppState>,
    PathName(name): PathName,
) -> ApiResult<Json<FriendsOut>> {
    let no_request = || AppError::not_found(msg::NO_FRIEND_REQUEST);
    let mut tx = state.db.begin().await?;
    let other = account_id(&mut tx, name.as_deref())
        .await?
        .filter(|id| *id != user.id)
        .unwrap_or(NO_ACCOUNT);
    // For an unknown name the same statements run with `NO_ACCOUNT`, and change nothing.
    let found = lock_pair(&mut tx, user.id, other).await?;
    let accepted = sqlx::query!(
        "UPDATE friendships SET accepted_at = clock_timestamp()
         WHERE requester_id = $1 AND addressee_id = $2 AND accepted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM user_blocks
                           WHERE user_id = $2 AND blocked_user_id = $1)",
        other,
        user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if !found || accepted == 0 {
        return Err(no_request());
    }
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Declines a request from `username`, or cancels the caller's request to them: the pending
/// request goes, without a trace, and either may ask again.
#[utoipa::path(
    delete,
    path = "/api/friends/requests/{username}",
    operation_id = "friends_delete_request",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The other account of the request")),
    responses(
        (status = 200, body = FriendsOut, description = "Declined or cancelled; the caller's friends"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such friend request`")
    )
)]
pub async fn delete_request(
    user: CurrentUser,
    State(state): State<AppState>,
    PathName(name): PathName,
) -> ApiResult<Json<FriendsOut>> {
    let no_request = || AppError::not_found(msg::NO_FRIEND_REQUEST);
    let mut tx = state.db.begin().await?;
    let other = account_id(&mut tx, name.as_deref())
        .await?
        .filter(|id| *id != user.id)
        .unwrap_or(NO_ACCOUNT);
    // For an unknown name the same statements run with `NO_ACCOUNT`, and change nothing.
    let found = lock_pair(&mut tx, user.id, other).await?;
    // The caller's own request, hidden or not; or one to the caller that the caller can see.
    let deleted = sqlx::query!(
        "DELETE FROM friendships
         WHERE accepted_at IS NULL
           AND ((requester_id = $1 AND addressee_id = $2)
                OR (requester_id = $2 AND addressee_id = $1
                    AND NOT EXISTS (SELECT 1 FROM user_blocks
                                    WHERE user_id = $1 AND blocked_user_id = $2)))",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if !found || deleted == 0 {
        return Err(no_request());
    }
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Ends a friendship. The friend invites into clubs either made for the other and nobody used
/// yet are revoked. The other account is not told; either may ask again.
#[utoipa::path(
    delete,
    path = "/api/friends/list/{username}",
    operation_id = "friends_remove",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The friend")),
    responses(
        (status = 200, body = FriendsOut, description = "No longer friends; the caller's friends"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No friend with that name`")
    )
)]
pub async fn remove(
    user: CurrentUser,
    State(state): State<AppState>,
    PathName(name): PathName,
) -> ApiResult<Json<FriendsOut>> {
    let no_friend = || AppError::not_found(msg::NO_FRIEND);
    let mut tx = state.db.begin().await?;
    let other = account_id(&mut tx, name.as_deref())
        .await?
        .filter(|id| *id != user.id)
        .unwrap_or(NO_ACCOUNT);
    // For an unknown name the same statements run with `NO_ACCOUNT`, and change nothing.
    let found = lock_pair(&mut tx, user.id, other).await?;
    let removed = sqlx::query!(
        "DELETE FROM friendships
         WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
           AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)
           AND accepted_at IS NOT NULL",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if !found || removed == 0 {
        return Err(no_friend());
    }
    revoke_pair_invites(&mut tx, user.id, other).await?;
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
