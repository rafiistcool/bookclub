//! The member list and the membership writes of one club: role changes, removal, leaving and
//! the ownership transfer (endpoint-map §3, docs/phase1/conventions.md §5.3).
//!
//! Each write locks the club row, re-reads the caller's role and the target's inside the
//! transaction, and puts its guard into the statement itself, so a role the extractor read
//! before the lock never decides anything.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::extract::{ClubAdmin, ClubMember, ClubOwner, Role, role_in};
use super::out::{ClubOut, MemberList, MemberOut, club_out, member_outs};
use super::{lock_club, member_named};
use crate::AppState;
use crate::auth::rules::normalize_username;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, INVALID_VALUE, Validate, Validator};

pub const USE_TRANSFER: &str = "Use transfer to change the owner";
pub const TRANSFER_BEFORE_LEAVING: &str = "Transfer ownership before leaving";
pub const USE_LEAVE: &str = "Use leave to leave the club";

/// A new role for a member.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RoleIn {
    /// `"admin"` or `"member"`. Ownership moves only through the transfer.
    pub role: String,
}

impl Validate for RoleIn {
    fn validate(&mut self, v: &mut Validator) {
        match self.role.as_str() {
            "admin" | "member" => {}
            "owner" => v.fail(USE_TRANSFER),
            _ => v.fail(INVALID_VALUE),
        }
    }
}

/// The member who becomes the owner.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransferIn {
    pub username: String,
}

impl Validate for TransferIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.username = normalize_username(&self.username);
    }
}

/// Every member of the club, the caller included (`you: true`), ordered owner, admins, members,
/// then username. Each card previews up to three books the member is currently reading.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/members",
    operation_id = "clubs_list_members",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = MemberList, description = "The members"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn list_members(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<MemberList>> {
    let mut conn = state.db.acquire().await?;
    let items = member_outs(&mut conn, member.club.id, member.user.id, None).await?;
    Ok(Json(MemberList { items }))
}

/// Makes a member an admin or an admin a member (owner only).
#[utoipa::path(
    patch,
    path = "/api/clubs/{slug}/members/{username}",
    operation_id = "clubs_change_role",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("username" = String, Path, description = "The member's username")
    ),
    request_body = RoleIn,
    responses(
        (status = 200, body = MemberOut, description = "The member with the new role"),
        (status = 400, body = ErrorBody, description = "`Use transfer to change the owner`, or an unknown role"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only the club owner can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No member with that name`")
    )
)]
pub async fn change_role(
    ClubOwner(member): ClubOwner,
    State(state): State<AppState>,
    ApiPath(path): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<RoleIn>,
) -> ApiResult<Json<MemberOut>> {
    let (_, username) = path;
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    let caller = locked_role(&mut tx, member.user.id, club_id).await?;
    if caller != Role::Owner {
        return Err(AppError::forbidden(msg::ONLY_OWNER));
    }
    let (target, _) = target_named(&mut tx, club_id, &username).await?;
    let updated = sqlx::query!(
        "UPDATE club_memberships SET role = $3
         WHERE club_id = $1 AND user_id = $2 AND role <> 'owner'",
        club_id,
        target,
        body.role
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        // The target is the owner (the caller themselves).
        return Err(AppError::bad_request(USE_TRANSFER));
    }
    let out = member_outs(&mut tx, club_id, member.user.id, Some(target))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NO_MEMBER))?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Removes someone from the club. Admins remove members; the owner also removes admins. Nobody
/// removes the owner, and nobody removes themselves (that is `leave`). Invites created before
/// the removal no longer work for that person; a newer invite does.
#[utoipa::path(
    delete,
    path = "/api/clubs/{slug}/members/{username}",
    operation_id = "clubs_remove_member",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("username" = String, Path, description = "The member's username")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 400, body = ErrorBody, description = "`Use leave to leave the club`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`, `Only the club owner can do that` (removing an admin or the owner)"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No member with that name`")
    )
)]
pub async fn remove_member(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiPath(path): ApiPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let (_, username) = path;
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    let caller = locked_role(&mut tx, member.user.id, club_id).await?;
    if caller < Role::Admin {
        return Err(AppError::forbidden(msg::ONLY_ADMINS));
    }
    let (target, _) = target_named(&mut tx, club_id, &username).await?;
    if target == member.user.id {
        return Err(AppError::bad_request(USE_LEAVE));
    }
    // An admin removes members only; the owner anyone but the owner.
    let removed = sqlx::query!(
        "DELETE FROM club_memberships
         WHERE club_id = $1 AND user_id = $2
           AND CASE WHEN $3 THEN role <> 'owner' ELSE role = 'member' END",
        club_id,
        target,
        caller == Role::Owner
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(AppError::forbidden(msg::ONLY_OWNER));
    }
    sqlx::query!(
        "INSERT INTO club_removals (club_id, user_id) VALUES ($1, $2)
         ON CONFLICT (club_id, user_id) DO UPDATE SET removed_at = now()",
        club_id,
        target
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Leaves the club. The owner transfers ownership first (or deletes the club). Leaving records
/// nothing: the same link can let them back in.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/leave",
    operation_id = "clubs_leave",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 204, description = "Left"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`"),
        (status = 409, body = ErrorBody, description = "`Transfer ownership before leaving`")
    )
)]
pub async fn leave(member: ClubMember, State(state): State<AppState>) -> ApiResult<StatusCode> {
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    if !lock_club(&mut tx, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    let left = sqlx::query!(
        "DELETE FROM club_memberships WHERE club_id = $1 AND user_id = $2 AND role <> 'owner'",
        club_id,
        member.user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if left == 0 {
        return Err(match role_in(&mut tx, member.user.id, club_id).await? {
            Some(_) => AppError::conflict(TRANSFER_BEFORE_LEAVING),
            None => AppError::not_found(msg::NO_SUCH_CLUB),
        });
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hands the club to another member (owner only). The old owner becomes an admin. Both updates
/// run in one transaction under the club lock, the demotion first.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/transfer",
    operation_id = "clubs_transfer",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = TransferIn,
    responses(
        (status = 200, body = ClubOut, description = "The club; the caller is now an admin"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only the club owner can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No member with that name`")
    )
)]
pub async fn transfer(
    ClubOwner(member): ClubOwner,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<TransferIn>,
) -> ApiResult<Json<ClubOut>> {
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    locked_role(&mut tx, member.user.id, club_id).await?;
    let demoted = sqlx::query!(
        "UPDATE club_memberships SET role = 'admin'
         WHERE club_id = $1 AND user_id = $2 AND role = 'owner'",
        club_id,
        member.user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if demoted == 0 {
        return Err(AppError::forbidden(msg::ONLY_OWNER));
    }
    let promoted = sqlx::query!(
        "UPDATE club_memberships SET role = 'owner'
         WHERE club_id = $1 AND user_id = (SELECT id FROM users WHERE username = $2)",
        club_id,
        body.username
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if promoted == 0 {
        return Err(AppError::not_found(msg::NO_MEMBER));
    }
    let out = club_out(&mut tx, member.user.id, club_id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Locks the club and returns the caller's current role; 404 `No such club` when the club is
/// gone or the caller is no longer a member.
async fn locked_role(conn: &mut PgConnection, user_id: i64, club_id: i64) -> ApiResult<Role> {
    if !lock_club(conn, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    role_in(conn, user_id, club_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_CLUB))
}

/// The member a path `{username}` names (normalised like every username lookup); 404 `No member
/// with that name` for an unknown name and a non-member alike.
async fn target_named(
    conn: &mut PgConnection,
    club_id: i64,
    username: &str,
) -> ApiResult<(i64, Role)> {
    member_named(conn, club_id, &normalize_username(username))
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_MEMBER))
}
