//! Redeeming an invite (endpoint-map §3): the join page's preview `GET /api/invites/{code}` and
//! `POST /api/invites/{code}/accept` for a signed-in user. Sign-up redeems through the same
//! seam (`invites::consume` and `invites::join`).

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use super::extract::role_in;
use super::invites::{ALREADY_IN_CLUB, consume, join, normalize_code};
use super::lock_club;
use super::out::{ClubOut, club_out};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::client_ip::ClientIp;
use crate::dto::ClubRef;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::ApiPath;
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

/// The join page's view of a valid invite.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvitePreview {
    /// The club it joins; null for a sign-up invite.
    pub club: Option<ClubRef>,
    pub kind: InviteTarget,
}

/// What a valid invite code leads to, for the join page. Public. An unknown, revoked, expired
/// or used-up code gets one answer: 404 `That invite code is not valid`. 60 lookups per IP
/// address per hour.
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
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiPath(code): ApiPath<String>,
) -> ApiResult<Json<InvitePreview>> {
    state
        .limits
        .invite_preview_ip
        .check_and_hit(&ip_key(ip))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let row = sqlx::query!(
        r#"SELECT c.slug AS "slug?", c.name AS "name?"
           FROM club_invites i
           LEFT JOIN clubs c ON c.id = i.club_id
           WHERE i.code = $1 AND i.revoked_at IS NULL
             AND (i.expires_at IS NULL OR i.expires_at > now())
             AND (i.max_uses IS NULL OR i.use_count < i.max_uses)"#,
        normalize_code(&code)
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found(msg::INVITE_INVALID))?;
    let preview = match (row.slug, row.name) {
        (Some(slug), Some(name)) => InvitePreview {
            club: Some(ClubRef { slug, name }),
            kind: InviteTarget::Club,
        },
        _ => InvitePreview {
            club: None,
            kind: InviteTarget::Signup,
        },
    };
    Ok(Json(preview))
}

/// Joins the invite's club as a member. One transaction: the club lock, the "already in this
/// club" check (the invite is then not used), the atomic use of the invite, the removal check,
/// the 20-member limit, the membership, the redemption log and the `member_joined` event.
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
        (status = 409, body = ErrorBody, description = "`You are already in this club`, `This club is full`")
    )
)]
pub async fn accept(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(code): ApiPath<String>,
) -> ApiResult<Json<ClubOut>> {
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
