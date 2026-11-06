//! Redeeming an invite (endpoint-map §3): the join page's preview `GET /api/invites/{code}` and
//! `POST /api/invites/{code}/accept` for a signed-in user. Sign-up redeems through the same
//! seam (`invites::consume` and `invites::join`).

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::MAX_MEMBERS;
use super::extract::role_in;
use super::invites::{ALREADY_IN_CLUB, consume, join, normalize_code};
use super::lock_club;
use super::out::{ClubOut, club_out};
use crate::AppState;
use crate::auth::account::account_key;
use crate::auth::extract::{CurrentUser, MaybeUser};
use crate::catalog::shared::books::{BookRef, book_out};
use crate::client_ip::ClientIp;
use crate::dto::ClubRef;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::ApiPath;
use crate::media::shared::urls::avatar_url;
use crate::ratelimit::ip_key;

/// What an invite leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteTarget {
    /// Joins a club (and unlocks sign-up).
    Club,
    /// Only unlocks sign-up (a closed-beta invite minted by an instance admin).
    Signup,
}

/// The join page's view of a valid invite. Whoever holds a valid code sees the club's name, its
/// member count, the book it reads now and the member who made the invite; nothing else of the
/// club (no other members, no posts, no ratings) and never whom a personal invite is for.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvitePreview {
    /// The club it joins; null for a sign-up invite.
    pub club: Option<ClubRef>,
    pub kind: InviteTarget,
    /// A personal invite (a club admin's invite for one friend, FRIENDS #57): only the account it
    /// was made for can join with it, and it never makes an account. Whom it is for is not told.
    pub personal: bool,
    /// The member who made a club invite, while they are still in the club and not suspended;
    /// null otherwise, and always null for a sign-up invite.
    pub inviter: Option<InviterOut>,
    /// The book the club reads now; null without a current pick and for a sign-up invite.
    pub pick: Option<InvitePickOut>,
    /// How many members the club has; null for a sign-up invite.
    pub member_count: Option<i64>,
    /// The club has reached the 20-member limit: joining (and signing up with the code) fails
    /// with 409 `This club is full`. False for a sign-up invite.
    pub full: bool,
    /// When the code stops working; null when only its uses limit it.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// Signed in: whether the caller is already in the club. Null when signed out and for a
    /// sign-up invite.
    pub you_are_member: Option<bool>,
    /// Signed in, a personal invite: whether it was made for the caller. Null when signed out
    /// and for any other invite.
    pub for_you: Option<bool>,
}

/// The member who made a club invite.
#[derive(Debug, Serialize, ToSchema)]
pub struct InviterOut {
    pub username: String,
    /// `/media/{public_id}/a`.
    pub avatar_url: Option<String>,
}

/// The book a club reads now, as an invite shows it: the book alone, without its id, the pick's
/// note, meeting or readers.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvitePickOut {
    pub title: String,
    /// Display string; may be empty.
    pub authors: String,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    /// The cover's dominant colour, `#rrggbb`; null when there is no usable one.
    pub spine_color: Option<String>,
}

/// What a valid invite code leads to, for the join page. Public; a session adds
/// `you_are_member` and `for_you`. An unknown, revoked, expired or used-up code gets one answer:
/// 404 `That invite code is not valid`. 60 lookups per IP address per hour.
#[utoipa::path(
    get,
    path = "/api/invites/{code}",
    operation_id = "clubs_preview_invite",
    tag = "clubs",
    params(("code" = String, Path, description = "The invite code; spaces, dashes and case are ignored")),
    responses(
        (status = 200, body = InvitePreview, description = "The invite is valid"),
        (status = 404, body = ErrorBody, description = "`That invite code is not valid`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn preview(
    MaybeUser(viewer): MaybeUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiPath(code): ApiPath<String>,
) -> ApiResult<Json<InvitePreview>> {
    state
        .limits
        .invite_preview_ip
        .check_and_hit(&ip_key(ip))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let viewer_id = viewer.map(|user| user.id);
    let mut conn = state.db.acquire().await?;
    // The inviter only while they are a member of the invite's club and not suspended: leaving
    // keeps a member's invites valid, and they no longer speak for the club.
    let row = sqlx::query!(
        r#"SELECT c.slug AS "slug?", c.name AS "name?", i.invitee_id, i.expires_at,
                  u.username AS "inviter?", m.public_id AS "inviter_avatar?",
                  (SELECT count(*) FROM club_memberships cm WHERE cm.club_id = i.club_id)
                      AS "member_count!",
                  (SELECT p.work_id FROM club_picks p
                   WHERE p.club_id = i.club_id AND p.ended_at IS NULL) AS pick_work_id,
                  EXISTS (SELECT 1 FROM club_memberships cm
                          WHERE cm.club_id = i.club_id AND cm.user_id = $2) AS "you_are_member!"
           FROM club_invites i
           LEFT JOIN clubs c ON c.id = i.club_id
           LEFT JOIN club_memberships im ON im.club_id = i.club_id AND im.user_id = i.created_by
           LEFT JOIN users u ON u.id = im.user_id AND u.suspended_at IS NULL
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE i.code = $1 AND i.revoked_at IS NULL
             AND (i.expires_at IS NULL OR i.expires_at > now())
             AND (i.max_uses IS NULL OR i.use_count < i.max_uses)"#,
        normalize_code(&code),
        viewer_id as Option<i64>
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::INVITE_INVALID))?;
    let (Some(slug), Some(name)) = (row.slug, row.name) else {
        return Ok(Json(InvitePreview {
            club: None,
            kind: InviteTarget::Signup,
            personal: false,
            inviter: None,
            pick: None,
            member_count: None,
            full: false,
            expires_at: row.expires_at,
            you_are_member: None,
            for_you: None,
        }));
    };
    let pick = match row.pick_work_id {
        Some(work_id) => book_out(&mut conn, BookRef::work(work_id))
            .await?
            .map(|book| InvitePickOut {
                title: book.title,
                authors: book.authors,
                cover_url: book.cover_url,
                cover_thumb_url: book.cover_thumb_url,
                spine_color: book.spine_color,
            }),
        None => None,
    };
    Ok(Json(InvitePreview {
        club: Some(ClubRef { slug, name }),
        kind: InviteTarget::Club,
        personal: row.invitee_id.is_some(),
        inviter: row.inviter.map(|username| InviterOut {
            username,
            avatar_url: avatar_url(row.inviter_avatar.as_deref()),
        }),
        pick,
        member_count: Some(row.member_count),
        full: row.member_count >= MAX_MEMBERS,
        expires_at: row.expires_at,
        you_are_member: viewer_id.map(|_| row.you_are_member),
        for_you: match (viewer_id, row.invitee_id) {
            (Some(viewer), Some(invitee)) => Some(viewer == invitee),
            _ => None,
        },
    }))
}

/// Joins the invite's club as a member. One transaction: the club lock, the "already in this
/// club" check (the invite is then not used), the atomic use of the invite, the removal check,
/// the 20-member limit, the membership, the redemption log and the `member_joined` event.
/// 20 attempts per account and hour, valid code or not, so an account cannot guess codes past
/// the preview's limit.
#[utoipa::path(
    post,
    path = "/api/invites/{code}/accept",
    operation_id = "clubs_accept_invite",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("code" = String, Path, description = "The invite code; spaces, dashes and case are ignored")),
    responses(
        (status = 200, body = ClubOut, description = "The club just joined"),
        (status = 400, body = ErrorBody, description = "`That invite code is not valid` (unknown, revoked, expired, used up, a sign-up invite, or created before the caller was removed from the club)"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 409, body = ErrorBody, description = "`You are already in this club`, `This club is full`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn accept(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(code): ApiPath<String>,
) -> ApiResult<Json<ClubOut>> {
    state
        .limits
        .invite_accept_user
        .check_and_hit(&account_key(user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let code = normalize_code(&code);
    let invalid = || AppError::bad_request(msg::INVITE_INVALID);
    let mut tx = state.db.begin().await?;
    let club_id = sqlx::query_scalar!("SELECT club_id FROM club_invites WHERE code = $1", code)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .ok_or_else(invalid)?;
    if !lock_club(&mut tx, club_id).await? {
        return Err(invalid());
    }
    if role_in(&mut tx, user.id, club_id).await?.is_some() {
        return Err(AppError::conflict(ALREADY_IN_CLUB));
    }
    let invite = consume(&mut tx, &code, user.id)
        .await?
        .ok_or_else(invalid)?;
    join(&mut tx, &invite, club_id, user.id).await?;
    let out = club_out(&mut tx, user.id, club_id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
