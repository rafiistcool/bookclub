//! Friends (#57, docs/phase3/work-plan.md §4): friend requests, friendships, blocks and the
//! caller's personal club invitations, all under `/api/friends` (tag `friends`).
//!
//! - `requests`: send, accept, decline or cancel a request, and remove a friend (§4.2).
//! - `blocks`: block and unblock (§4.3).
//! - `out`: [`out::FriendsOut`], the answer of every route, and its loader.
//! - here: the routes, `GET /api/friends`, declining a club invitation, and the helpers the
//!   friend invites of `clubs::invites` share (§4.6).
//!
//! A friend is the other's audience for personal data through the policy functions
//! (`bc_can_see_user`, `bc_visible_owners`, `bc_friends`; migration 3100), and the home feed
//! shows a friend's personal events from the friendship on (`activity::feed`).
//!
//! **One row per pair** (`friendships_pair_key`): pending from `requester_id` to
//! `addressee_id` while `accepted_at` is NULL, then a friendship. A block is a `user_blocks` row
//! of its own and deletes the pair's row. A request to the caller from an account the caller
//! blocked is *hidden*: it stays, the sender sees it as outgoing, and the caller's lists leave it
//! out.
//!
//! **Lock order** of every write about a pair (send, accept, decline or cancel, remove, block,
//! unblock), taken by [`lock_pair`]:
//!
//! 1. the pair's turn, a transaction-level advisory lock (`PAIR_LOCK_SPACE`), so the writes of
//!    one pair run one after the other: a request and a block by the same account, or an
//!    unblock and the other's request, cannot interleave;
//! 2. both accounts' rows `FOR KEY SHARE`, in id order, so an account deletion either waits for
//!    the write or has finished before it (the write then sees the account gone). The deletion
//!    locks the account `FOR UPDATE` before its cascade reaches the pair's rows, so it cannot
//!    hold one of them while it waits for the write;
//! 3. the `user_blocks` row, then the pair's `friendships` row, then the pair's invite rows.
//!
//! No pair write locks a club. A friend invite (`clubs::invites::create_invite`) holds the club
//! and then takes the friend's account row `FOR KEY SHARE` and the pair's `friendships` row `FOR
//! SHARE` ([`locked_friend`]), which conflicts with the `DELETE` of an unfriend or a block: an
//! invite cannot slip past either, and neither waits for a club.

pub mod blocks;
pub mod out;
pub mod requests;

use axum::Json;
use axum::extract::rejection::RawPathParamsRejection;
use axum::extract::{FromRequestParts, RawPathParams, State};
use axum::http::request::Parts;
use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::rules::normalize_username;
use crate::clubs::invites::NO_SUCH_INVITE;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::ApiPath;
use out::{FriendsOut, friends_out};

/// The first key of the two-key advisory lock space of a pair's turn ([`lock_pair`]); beside
/// ingest's `0x6263_0100`, the reports' `0x6263_0200` and `jobs::lock_ids`.
const PAIR_LOCK_SPACE: i32 = 0x6263_0300;

/// The friend routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(requests::send))
        .routes(routes!(requests::accept))
        .routes(routes!(requests::delete_request))
        .routes(routes!(requests::remove))
        .routes(routes!(blocks::block, blocks::unblock))
        .routes(routes!(decline_club_invite))
}

/// The `{username}` of a friend route, normalised like every username lookup
/// (`auth::rules::normalize_username`). `None` when it cannot name an account: a NUL character
/// (`%00`, which Postgres could not compare) or a path that is not UTF-8. Each route answers
/// `None` as it answers an unknown name, never with 400 (the club extractors' rule for `{slug}`).
pub struct PathName(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for PathName {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let params = match RawPathParams::from_request_parts(parts, state).await {
            Ok(params) => params,
            Err(RawPathParamsRejection::InvalidUtf8InPathParam(_)) => return Ok(PathName(None)),
            Err(rejection) => return Err(AppError::internal(rejection.body_text())),
        };
        let name = params
            .iter()
            .find(|(key, _)| *key == "username")
            .map(|(_, value)| normalize_username(value))
            .ok_or_else(|| AppError::internal("PathName on a route without {username}"))?;
        Ok(PathName((!name.contains('\0')).then_some(name)))
    }
}

/// The id of the account with this normalised username; `None` for an unknown name (deleted
/// accounts are gone) and for a name with a NUL character.
pub(crate) async fn account_id(
    conn: &mut PgConnection,
    username: Option<&str>,
) -> sqlx::Result<Option<i64>> {
    let Some(username) = username.filter(|name| !name.contains('\0')) else {
        return Ok(None);
    };
    sqlx::query_scalar!("SELECT id FROM users WHERE username = $1", username)
        .fetch_optional(conn)
        .await
}

/// Takes the locks of a write about the pair `me`, `other` (the module's lock order, steps 1
/// and 2): the pair's turn, then both accounts' rows `FOR KEY SHARE` in id order. False when one
/// of the accounts no longer exists; the write then answers as for an unknown name.
pub(crate) async fn lock_pair(conn: &mut PgConnection, me: i64, other: i64) -> sqlx::Result<bool> {
    let key = format!("{}:{}", me.min(other), me.max(other));
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        PAIR_LOCK_SPACE,
        key
    )
    .execute(&mut *conn)
    .await?;
    let found = sqlx::query_scalar!(
        "SELECT id FROM users WHERE id IN ($1, $2) ORDER BY id FOR KEY SHARE",
        me,
        other
    )
    .fetch_all(conn)
    .await?;
    Ok(found.len() == 2)
}

/// Revokes the open friend invites the pair made for each other (an unfriend or a block, §4.2,
/// §4.3): bound to one of them and created by the other, not yet revoked, expired or used.
pub(crate) async fn revoke_pair_invites(
    conn: &mut PgConnection,
    a: i64,
    b: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE club_invites SET revoked_at = clock_timestamp()
         WHERE revoked_at IS NULL
           AND ((created_by = $1 AND invitee_id = $2) OR (created_by = $2 AND invitee_id = $1))
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR use_count < max_uses)",
        a,
        b
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The id of the caller's friend with this normalised username, for a friend invite into a club
/// (§4.6). Runs under the club's lock: it takes the friend's account row `FOR KEY SHARE` and the
/// pair's accepted `friendships` row `FOR SHARE`, so a concurrent unfriend, block or deletion
/// either finished before or waits for the invite. 404 `No friend with that name` for an
/// unknown name, the caller themselves, a pending request and a block in either direction (a
/// block ends the friendship), alike.
pub async fn locked_friend(conn: &mut PgConnection, me: i64, username: &str) -> ApiResult<i64> {
    let not_friends = || AppError::not_found(msg::NO_FRIEND);
    let friend = account_id(conn, Some(username))
        .await?
        .filter(|id| *id != me)
        .ok_or_else(not_friends)?;
    sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR KEY SHARE", friend)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(not_friends)?;
    sqlx::query_scalar!(
        r#"SELECT 1 AS "one!" FROM friendships
           WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
             AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)
             AND accepted_at IS NOT NULL
           FOR SHARE"#,
        me,
        friend
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(not_friends)?;
    Ok(friend)
}

/// The caller's friends (with up to three books each is currently reading), the requests to and
/// from the caller, the accounts the caller blocked, and the caller's open friend invites into
/// clubs. Nobody else's friends, requests or blocks are ever listed.
#[utoipa::path(
    get,
    path = "/api/friends",
    operation_id = "friends_list",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = FriendsOut, description = "The caller's friends, requests, blocks and club invitations"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<FriendsOut>> {
    let mut conn = state.db.acquire().await?;
    Ok(Json(friends_out(&mut conn, user.id).await?))
}

/// Declines a friend's invite into a club: it is revoked and can no longer be redeemed. Only
/// the account the invite was made for can decline it; declining twice is harmless.
#[utoipa::path(
    delete,
    path = "/api/friends/club-invites/{invite_id}",
    operation_id = "friends_decline_club_invite",
    tag = "friends",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("invite_id" = i64, Path, description = "The invite's id (`FriendClubInviteOut.id`)")),
    responses(
        (status = 200, body = FriendsOut, description = "Declined; the caller's friends"),
        (status = 400, body = ErrorBody, description = "The id is not an integer"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such invite` (also one made for someone else)")
    )
)]
pub async fn decline_club_invite(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(invite_id): ApiPath<i64>,
) -> ApiResult<Json<FriendsOut>> {
    let mut tx = state.db.begin().await?;
    // Only an open invite is revoked: a used or expired one keeps its history.
    let found = sqlx::query_scalar!(
        "UPDATE club_invites
         SET revoked_at = CASE WHEN revoked_at IS NULL
                                    AND (expires_at IS NULL OR expires_at > now())
                                    AND (max_uses IS NULL OR use_count < max_uses)
                               THEN clock_timestamp() ELSE revoked_at END
         WHERE id = $1 AND invitee_id = $2
         RETURNING id",
        invite_id,
        user.id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if found.is_none() {
        return Err(AppError::not_found(NO_SUCH_INVITE));
    }
    let out = friends_out(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
