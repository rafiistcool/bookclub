//! Invites (endpoint-map §2, §3; docs/phase1/conventions.md §5.5, §13): the redemption seam that
//! sign-up (`POST /api/auth/register`) and `POST /api/invites/{code}/accept` share, and the
//! club's invite routes (create, list, revoke).
//!
//! Sign-up calls [`consume`] and, for a club invite, [`join`], inside its own transaction, so a
//! failed join rolls the new account and the invite use back.
//!
//! **Lock order** (every membership write, §5.3): the club row, then the inviter's user row (the
//! membership's `invited_by` foreign key needs it, and an account deletion locks it `FOR UPDATE`
//! before it clears the invite's `created_by`), then the invite row. [`consume`] takes them in
//! that order before its atomic `UPDATE`, so a join cannot deadlock with a club delete, a member
//! write or the inviter's account deletion (which locks the club first, §5.6).
//!
//! Creating and revoking an invite also lock the club and re-read the caller's role, so an admin
//! removed meanwhile gets no code to walk back in with. Invites and removals are dated with
//! `clock_timestamp()` under that lock: their dates follow the order of the writes, which the
//! removal rule of [`join`] compares.
//!
//! **Friend invites** (FRIENDS #57, docs/phase3/work-plan.md §4.6): `kind: "friend"` makes a
//! single-use invite bound to one friend of the admin (`club_invites.invitee_id`). Only that
//! account can redeem it ([`consume`]), so a forwarded code is useless and sign-up never takes
//! one. Under the club lock, the friend's account row and the pair's friendship are locked next
//! (`friends::locked_friend`), so an unfriend or a block, which revoke the pair's open invites,
//! cannot pass the new invite. Every friend invite counts against the caller's
//! `friend_invite_user` (20 a day, unknown names and repeats included): a friend's decline does
//! not stop a new invite, as a declined request does not stop a new request, and the limit
//! bounds both alike.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::MAX_MEMBERS;
use super::extract::{ClubAdmin, ClubMember, Role, role_in};
use super::out::{InviteList, InviteOut, club_invite_out, club_invite_outs};
use super::{lock_club, locked_role};
use crate::AppState;
use crate::activity::{self, EventKind, NewEvent};
use crate::auth::account::account_key;
use crate::auth::random;
use crate::auth::rules::normalize_username;
use crate::error::{ApiResult, AppError, ErrorBody, is_unique_violation, msg};
use crate::extract::{ApiJson, ApiPath, FIELD_REQUIRED, Validate, Validator};
use crate::friends::locked_friend;
use crate::text::is_py_space;

pub const ALREADY_IN_CLUB: &str = "You are already in this club";
pub const NO_SUCH_INVITE: &str = "No such invite";

/// Crockford base32, the legacy invite alphabet (no I, L, O, U).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_CHARS: usize = 10;
const CODE_ATTEMPTS: usize = 8;

/// An invite whose use was just counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedInvite {
    pub id: i64,
    /// `None` for a sign-up invite minted by an instance admin (it only unlocks sign-up).
    pub club_id: Option<i64>,
    /// Becomes `club_memberships.invited_by`.
    pub created_by: Option<i64>,
    /// A removed member cannot use an invite created before the removal.
    pub created_at: OffsetDateTime,
}

/// A typed code as stored: whitespace and `-` removed, uppercased (the legacy
/// `normalize_invite_code`, without Crockford decoding).
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !is_py_space(*c) && *c != '-')
        .collect::<String>()
        .to_uppercase()
}

/// A fresh code: 10 random Crockford base32 characters (50 bits).
pub fn new_code() -> String {
    let mut bytes = [0u8; CODE_CHARS];
    random::fill(&mut bytes);
    // 256 is a multiple of 32, so the remainder is uniform.
    bytes
        .iter()
        .map(|b| CROCKFORD[usize::from(*b) % CROCKFORD.len()] as char)
        .collect()
}

/// Counts one use of the invite with this normalised `code` and logs it for `user_id`
/// (`club_invite_uses`), atomically: an unknown, revoked, expired or used-up invite is `None`
/// and nothing changes, and so is a friend invite bound to another account (sign-up passes the
/// new account's id, so a bound code never signs anyone up). Of concurrent redemptions of a
/// single-use invite exactly one succeeds.
///
/// It first locks the invite's club and its creator's row (the lock order of the module docs).
/// Callers run it inside their transaction and roll back on any later failure. The accept route
/// checks "already in this club" (409, invite not consumed) before calling it.
pub async fn consume(
    conn: &mut PgConnection,
    code: &str,
    user_id: i64,
) -> sqlx::Result<Option<ConsumedInvite>> {
    let Some(target) = sqlx::query!(
        "SELECT club_id, created_by FROM club_invites WHERE code = $1",
        code
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    if let Some(club_id) = target.club_id {
        lock_club(conn, club_id).await?;
    }
    if let Some(inviter) = target.created_by {
        // A deleted inviter returns no row; the UPDATE below then reads created_by NULL.
        sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR KEY SHARE", inviter)
            .fetch_optional(&mut *conn)
            .await?;
    }
    let Some(invite) = sqlx::query_as!(
        ConsumedInvite,
        "UPDATE club_invites SET use_count = use_count + 1
         WHERE code = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR use_count < max_uses)
           AND (invitee_id IS NULL OR invitee_id = $2)
         RETURNING id, club_id, created_by, created_at",
        code,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query!(
        "INSERT INTO club_invite_uses (invite_id, user_id) VALUES ($1, $2)",
        invite.id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(Some(invite))
}

/// Makes `user_id` a member of `club_id` through the consumed `invite`, inside the caller's
/// transaction:
///
/// 1. locks the club row (a no-op after [`consume`]);
/// 2. refuses an invite created before the user's removal from this club (400 `That invite
///    code is not valid`);
/// 3. refuses a full club (409 `This club is full`, 20 members), counted under the lock;
/// 4. inserts the membership with `invited_by` = the invite's creator, deletes the removal row
///    and records `member_joined`.
///
/// A user who is already a member gets 409 `You are already in this club`.
pub async fn join(
    conn: &mut PgConnection,
    invite: &ConsumedInvite,
    club_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    if !lock_club(conn, club_id).await? {
        return Err(AppError::bad_request(msg::INVITE_INVALID));
    }
    let removed_at = sqlx::query_scalar!(
        "SELECT removed_at FROM club_removals WHERE club_id = $1 AND user_id = $2",
        club_id,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    if removed_at.is_some_and(|removed_at| invite.created_at <= removed_at) {
        return Err(AppError::bad_request(msg::INVITE_INVALID));
    }
    let members = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM club_memberships WHERE club_id = $1"#,
        club_id
    )
    .fetch_one(&mut *conn)
    .await?;
    if members >= MAX_MEMBERS {
        return Err(AppError::conflict(msg::CLUB_FULL));
    }
    sqlx::query!(
        "INSERT INTO club_memberships (club_id, user_id, role, invited_by)
         VALUES ($1, $2, 'member', $3)",
        club_id,
        user_id,
        invite.created_by
    )
    .execute(&mut *conn)
    .await
    .map_err(|error| {
        if is_unique_violation(&error, "club_memberships_pkey") {
            AppError::conflict(ALREADY_IN_CLUB)
        } else {
            error.into()
        }
    })?;
    sqlx::query!(
        "DELETE FROM club_removals WHERE club_id = $1 AND user_id = $2",
        club_id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    activity::record(
        conn,
        NewEvent {
            club_id: Some(club_id),
            actor_id: Some(user_id),
            kind: EventKind::MemberJoined,
            work_id: None,
            pick_id: None,
            payload: Default::default(),
        },
    )
    .await?;
    Ok(())
}

/// Inserts an invite under a fresh code and returns its id. `club_id` `None` is a sign-up
/// invite. `expires_in_days` `None` means no expiry, `max_uses` `None` no use limit; the schema
/// requires at least one of them.
///
/// `created_at` is the time of the insert, not of the transaction start: a club invite is
/// inserted under the club lock, so its date and a removal's follow the order of the two writes.
pub async fn insert_invite(
    conn: &mut PgConnection,
    club_id: Option<i64>,
    created_by: Option<i64>,
    max_uses: Option<i32>,
    expires_in_days: Option<i32>,
) -> sqlx::Result<i64> {
    insert_bound_invite(conn, club_id, created_by, max_uses, expires_in_days, None).await
}

/// [`insert_invite`], bound to `invitee` when given: a friend invite only that account can
/// redeem (the schema then requires a club and `max_uses` 1, `club_invites_bound`).
pub async fn insert_bound_invite(
    conn: &mut PgConnection,
    club_id: Option<i64>,
    created_by: Option<i64>,
    max_uses: Option<i32>,
    expires_in_days: Option<i32>,
    invitee: Option<i64>,
) -> sqlx::Result<i64> {
    for _ in 0..CODE_ATTEMPTS {
        let id = sqlx::query_scalar!(
            "INSERT INTO club_invites
                 (club_id, code, created_by, max_uses, created_at, expires_at, invitee_id)
             SELECT $1, $2, $3, $4, at, at + $5::int4 * interval '1 day', $6
             FROM clock_timestamp() AS at
             ON CONFLICT (code) DO NOTHING
             RETURNING id",
            club_id,
            new_code(),
            created_by,
            max_uses,
            expires_in_days,
            invitee
        )
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    Err(sqlx::Error::Protocol(format!(
        "no free invite code after {CODE_ATTEMPTS} attempts"
    )))
}

/// The invite presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteKind {
    /// One use; expires in 30 days unless `expires_in_days` says otherwise.
    SingleUse,
    /// A shareable link without a use limit; expires in 7 days unless `expires_in_days` says
    /// otherwise.
    Link,
    /// A personal invite for one friend of the caller (`username`): one use, only by that
    /// friend; expires in 30 days unless `expires_in_days` says otherwise.
    Friend,
}

/// A new club invite.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InviteIn {
    pub kind: InviteKind,
    /// 1 to 30; default 30 for `single_use` and `friend`, 7 for `link`.
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    /// The friend a `friend` invite is for (required there, refused for the other kinds).
    #[serde(default)]
    pub username: Option<String>,
}

impl Validate for InviteIn {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(self.expires_in_days, 1, 30);
        match (self.kind, self.username.as_mut()) {
            (InviteKind::Friend, Some(name)) => *name = normalize_username(name),
            (InviteKind::Friend, None) => v.fail(FIELD_REQUIRED),
            (_, Some(_)) => v.fail(msg::ONLY_FRIEND_INVITE_NAMES),
            (_, None) => {}
        }
    }
}

/// Every invite of the club, newest first (admins). A personal invite (`personal`) names the
/// friend it is for (`invitee`) to its creator only.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/invites",
    operation_id = "clubs_list_invites",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = InviteList, description = "The club's invites"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn list_invites(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<InviteList>> {
    let mut conn = state.db.acquire().await?;
    let items = club_invite_outs(
        &mut conn,
        &state.config,
        Some(member.user.id),
        Some(member.club.id),
        None,
    )
    .await?;
    Ok(Json(InviteList { items }))
}

/// Creates an invite to the club (admins): a single-use code, a shareable link, or a personal
/// invite for one friend of the caller (`kind: "friend"`, `username`), which only that friend can
/// redeem through `/join/{code}`. A second friend invite for the same friend, while the caller's
/// first one is still open, answers with the first one. 20 friend invites per account a day,
/// unknown names and repeats included.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/invites",
    operation_id = "clubs_create_invite",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = InviteIn,
    responses(
        (status = 201, body = InviteOut, description = "The new invite (or the caller's open friend invite for that friend)"),
        (status = 400, body = ErrorBody, description = "An invalid kind or expiry, `Field required` (a friend invite without `username`), `Only a friend invite names a person`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No friend with that name`"),
        (status = 409, body = ErrorBody, description = "`That friend is already in this club`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (friend invites only)")
    )
)]
pub async fn create_invite(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<InviteIn>,
) -> ApiResult<(StatusCode, Json<InviteOut>)> {
    let (max_uses, default_days) = match body.kind {
        InviteKind::SingleUse | InviteKind::Friend => (Some(1), 30),
        InviteKind::Link => (None, 7),
    };
    // Validated to 1..=30 above.
    let days = body.expires_in_days.unwrap_or(default_days) as i32;
    if body.kind == InviteKind::Friend {
        state
            .limits
            .friend_invite_user
            .check_and_hit(&account_key(member.user.id))
            .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    }
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, &member).await?;
    let invitee = match (body.kind, body.username.as_deref()) {
        (InviteKind::Friend, Some(username)) => {
            Some(bound_friend(&mut tx, &member, username).await?)
        }
        _ => None,
    };
    let open = match invitee {
        Some(friend) => open_friend_invite(&mut tx, club_id, member.user.id, friend).await?,
        None => None,
    };
    let id = match open {
        Some(id) => id,
        None => {
            insert_bound_invite(
                &mut tx,
                Some(club_id),
                Some(member.user.id),
                max_uses,
                Some(days),
                invitee,
            )
            .await?
        }
    };
    let out = club_invite_out(
        &mut tx,
        &state.config,
        Some(member.user.id),
        Some(club_id),
        id,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// The friend a `kind: "friend"` invite is for, under the club lock: 404 `No friend with that
/// name` unless the caller and `username` are friends (`friends::locked_friend`), then 409 `That
/// friend is already in this club`.
async fn bound_friend(
    conn: &mut PgConnection,
    member: &ClubMember,
    username: &str,
) -> ApiResult<i64> {
    let friend = locked_friend(conn, member.user.id, username).await?;
    if role_in(conn, friend, member.club.id).await?.is_some() {
        return Err(AppError::conflict(msg::FRIEND_IN_CLUB));
    }
    Ok(friend)
}

/// The caller's newest friend invite for `friend` into the club that can still be redeemed. Only
/// the caller's own: answering with a co-admin's would tell the caller whom that admin is
/// friends with.
async fn open_friend_invite(
    conn: &mut PgConnection,
    club_id: i64,
    created_by: i64,
    friend: i64,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "SELECT id FROM club_invites
         WHERE club_id = $1 AND created_by = $2 AND invitee_id = $3 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR use_count < max_uses)
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        club_id,
        created_by,
        friend
    )
    .fetch_optional(conn)
    .await
}

/// Revokes an invite of the club (admins). Revoking twice is harmless.
#[utoipa::path(
    delete,
    path = "/api/clubs/{slug}/invites/{invite_id}",
    operation_id = "clubs_revoke_invite",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("invite_id" = i64, Path, description = "The invite's id")
    ),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, body = ErrorBody, description = "The id is not an integer"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No such invite` (also another club's)")
    )
)]
pub async fn revoke_invite(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiPath(path): ApiPath<(String, i64)>,
) -> ApiResult<StatusCode> {
    let (_, invite_id) = path;
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, &member).await?;
    let revoked = sqlx::query_scalar!(
        "UPDATE club_invites SET revoked_at = coalesce(revoked_at, now())
         WHERE club_id = $1 AND id = $2
         RETURNING id",
        member.club.id,
        invite_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if revoked.is_none() {
        return Err(AppError::not_found(NO_SUCH_INVITE));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Locks the club and checks that the caller is still an admin there: 404 `No such club` for
/// someone removed since the extractor read the role, 403 `Only club admins can do that` for
/// someone demoted.
async fn lock_as_admin(conn: &mut PgConnection, member: &ClubMember) -> ApiResult<()> {
    if locked_role(conn, member.user.id, member.club.id).await? < Role::Admin {
        return Err(AppError::forbidden(msg::ONLY_ADMINS));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_lose_spaces_and_dashes_and_are_uppercased() {
        assert_eq!(normalize_code(" ab-cd\u{3000}ef\t"), "ABCDEF");
        assert_eq!(normalize_code("dev-only"), "DEVONLY");
        assert_eq!(normalize_code("Oil"), "OIL", "no Crockford decoding");
        assert_eq!(normalize_code(" - "), "");
    }

    #[test]
    fn new_codes_are_ten_crockford_characters() {
        let a = new_code();
        assert_eq!(a.len(), CODE_CHARS);
        assert!(a.bytes().all(|b| CROCKFORD.contains(&b)), "{a}");
        assert_eq!(normalize_code(&a), a);
        assert_ne!(a, new_code());
    }
}
