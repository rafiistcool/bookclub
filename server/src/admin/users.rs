//! Suspending accounts (#55, Q15): `POST /api/admin/users/{username}/suspend` and
//! `/unsuspend`, and `GET /api/admin/users/suspended` for the admin page.
//!
//! A suspension, in one transaction:
//!
//! - sets `users.suspended_at`, and deletes every session and bearer token of the account. From
//!   then on the account cannot sign in: login and bearer token creation answer 403 `This account
//!   is suspended` once the password matched (`auth::login::confirm`), and a session that slipped
//!   through is refused where the caller is resolved (`auth::extract`), as if signed out;
//! - ends the account's instance admin rights (`users.is_admin = false`). Lifting the suspension
//!   does not bring them back; only `bookclub-server make-admin` does. Otherwise the
//!   unsuspension of `docs/legal/takedown.md` §7 (export, deletion) would hand a suspended admin
//!   the admin routes again, with which they could suspend the operator (security review, ADM2
//!   merge);
//! - revokes the active invites the account knows ([`revoke_known_invites`]), so a second
//!   account cannot walk back in with one: the rule of a club removal (`clubs::members`), for
//!   every club, plus every sign-up invite when the account was an instance admin.
//!
//! Its content stays as it is. Unsuspending clears the column; the account then signs in again.
//! An admin cannot suspend their own account (there would be nobody left to undo it on a
//! one-admin instance).
//!
//! **Lock order:** the acting admin's row and the target's, in id order (`FOR NO KEY UPDATE`,
//! the login's lock), then the target's sessions, then the invites. Both rows are re-read under
//! the lock: an admin suspended or demoted since the route guard let them through decides
//! nothing (404 `Not found`, as the guard answers), so two admins who suspend each other at the
//! same moment cannot both succeed. No club row is locked: a redemption locks the inviter's row
//! only `FOR KEY SHARE`, which this lock does not block, and then the invite row, which the
//! revocation waits for.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::extract::InstanceAdmin;
use crate::auth::rules::normalize_username;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::ApiPath;
use crate::media::shared::urls::avatar_url;

/// 404 for a username no account has (admin only).
pub const NO_ACCOUNT: &str = "No account with that name";
/// 400 when an admin tries to suspend their own account.
pub const CANNOT_SUSPEND_SELF: &str = "You cannot suspend your own account";

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(suspend))
        .routes(routes!(unsuspend))
        .routes(routes!(list_suspended))
}

/// An account as the admin page shows it: no address, no personal data beyond the name.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminUserOut {
    pub id: i64,
    pub username: String,
    /// `/media/{public_id}/a`; null without a picture or once it was taken down.
    pub avatar_url: Option<String>,
    /// An instance admin.
    pub is_admin: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the account was suspended; null while it may sign in.
    #[serde(with = "time::serde::rfc3339::option")]
    pub suspended_at: Option<OffsetDateTime>,
}

/// The suspended accounts, the latest suspension first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminUserList {
    pub items: Vec<AdminUserOut>,
}

/// Suspends an account: it can no longer sign in, every session and bearer token it has is
/// deleted, its instance admin rights end, and the invites it knows are revoked. Its content
/// stays. Suspending a suspended account keeps the first time.
#[utoipa::path(
    post,
    path = "/api/admin/users/{username}/suspend",
    operation_id = "admin_suspend_user",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The account's username")),
    responses(
        (status = 200, body = AdminUserOut, description = "The account, suspended"),
        (status = 400, body = ErrorBody, description = "`You cannot suspend your own account`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No account with that name`; `Not found` for everyone but instance admins")
    )
)]
pub async fn suspend(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(username): ApiPath<String>,
) -> ApiResult<Json<AdminUserOut>> {
    let username = normalize_username(&username);
    let mut tx = state.db.begin().await?;
    let user_id = sqlx::query_scalar!("SELECT id FROM users WHERE username = $1", username)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found(NO_ACCOUNT))?;
    if user_id == admin.id {
        return Err(AppError::bad_request(CANNOT_SUSPEND_SELF));
    }
    // Both rows, in id order (see the module docs). The target's first lock is the login's
    // (`auth::login::confirm`): a login that is creating a session finishes, and its session
    // is deleted below.
    let mut target = None;
    let mut acting = None;
    for id in sorted_pair(admin.id, user_id) {
        let row = sqlx::query!(
            r#"SELECT is_admin, suspended_at IS NOT NULL AS "suspended!"
               FROM users WHERE id = $1 FOR NO KEY UPDATE"#,
            id
        )
        .fetch_optional(&mut *tx)
        .await?;
        if id == user_id {
            target = row;
        } else {
            acting = row;
        }
    }
    // Suspended, demoted or deleted since the guard read the session.
    if !acting.is_some_and(|row| row.is_admin && !row.suspended) {
        return Err(AppError::not_found(msg::NOT_FOUND));
    }
    let target = target.ok_or_else(|| AppError::not_found(NO_ACCOUNT))?;
    sqlx::query!(
        "UPDATE users SET suspended_at = coalesce(suspended_at, now()), is_admin = false
         WHERE id = $1",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    let ended = sqlx::query!("DELETE FROM sessions WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let revoked = revoke_known_invites(&mut tx, user_id, target.is_admin).await?;
    let out = admin_user_out(&mut tx, user_id).await?;
    tx.commit().await?;
    tracing::info!(
        admin = admin.id,
        user = user_id,
        sessions = ended,
        invites = revoked,
        "account suspended"
    );
    Ok(Json(out))
}

/// The two ids in ascending order, the order the users rows are locked in.
fn sorted_pair(a: i64, b: i64) -> [i64; 2] {
    if a <= b { [a, b] } else { [b, a] }
}

/// Revokes every active invite a suspended account knows, so that a second account cannot use
/// one to sign up or to rejoin a club (security review, ADM2 merge): the rule of a club removal
/// (`clubs::members::revoke_known_invites`) for each club the account belongs to — every invite of a club it
/// owns or administers, and of the others the invites created up to its demotion
/// (`admin_until`) — plus the invites it created or redeemed anywhere, and, for an instance
/// admin (who lists them all, `GET /api/admin/invites`), every sign-up invite. Expired and
/// used-up invites keep their state. A link someone forwarded to them is beyond what the server
/// knows. Returns how many were revoked.
async fn revoke_known_invites(
    conn: &mut PgConnection,
    user_id: i64,
    instance_admin: bool,
) -> sqlx::Result<u64> {
    Ok(sqlx::query!(
        "UPDATE club_invites i SET revoked_at = clock_timestamp()
         WHERE i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > now())
           AND (i.max_uses IS NULL OR i.use_count < i.max_uses)
           AND (i.created_by = $1
                OR i.id IN (SELECT u.invite_id FROM club_invite_uses u WHERE u.user_id = $1)
                OR (i.club_id IS NULL AND $2)
                OR EXISTS (SELECT 1 FROM club_memberships m
                           WHERE m.club_id = i.club_id AND m.user_id = $1
                             AND (m.role <> 'member' OR i.created_at <= m.admin_until)))",
        user_id,
        instance_admin
    )
    .execute(conn)
    .await?
    .rows_affected())
}

/// Lifts a suspension: the account signs in again (without the admin rights the suspension
/// ended). An account that is not suspended stays as it is.
#[utoipa::path(
    post,
    path = "/api/admin/users/{username}/unsuspend",
    operation_id = "admin_unsuspend_user",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("username" = String, Path, description = "The account's username")),
    responses(
        (status = 200, body = AdminUserOut, description = "The account, no longer suspended"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No account with that name`; `Not found` for everyone but instance admins")
    )
)]
pub async fn unsuspend(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(username): ApiPath<String>,
) -> ApiResult<Json<AdminUserOut>> {
    let mut conn = state.db.acquire().await?;
    let user_id = lift_suspension(&mut conn, &username)
        .await?
        .ok_or_else(|| AppError::not_found(NO_ACCOUNT))?;
    tracing::info!(admin = admin.id, user = user_id, "account unsuspended");
    Ok(Json(admin_user_out(&mut conn, user_id).await?))
}

/// Lifts the suspension of the account named `username` (normalised like every username
/// lookup), for `/unsuspend` and `bookclub-server unsuspend`: it signs in again. Admin rights a
/// suspension ended stay ended. Returns the account's id, `None` for an unknown name; an
/// account that is not suspended stays as it is.
pub async fn lift_suspension(conn: &mut PgConnection, username: &str) -> sqlx::Result<Option<i64>> {
    let username = normalize_username(username);
    // The end of a suspension counts as a sign-in for the purge of empty sign-ups
    // (`auth::empty_sign_ups`): the account could not sign in while it was suspended.
    sqlx::query_scalar!(
        "UPDATE users
         SET suspended_at = NULL,
             unsuspended_at = CASE WHEN suspended_at IS NULL THEN unsuspended_at ELSE now() END
         WHERE username = $1
         RETURNING id",
        username
    )
    .fetch_optional(conn)
    .await
}

/// Every suspended account, the latest suspension first.
#[utoipa::path(
    get,
    path = "/api/admin/users/suspended",
    operation_id = "admin_list_suspended_users",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = AdminUserList, description = "The suspended accounts"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` for everyone but instance admins")
    )
)]
pub async fn list_suspended(
    InstanceAdmin(_admin): InstanceAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<AdminUserList>> {
    let mut conn = state.db.acquire().await?;
    let rows = sqlx::query!(
        r#"SELECT u.id, u.username, m.public_id AS "avatar_public_id?", u.is_admin, u.created_at,
                  u.suspended_at
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.suspended_at IS NOT NULL
           ORDER BY u.suspended_at DESC, u.id DESC"#
    )
    .fetch_all(&mut *conn)
    .await?;
    let items = rows
        .into_iter()
        .map(|row| AdminUserOut {
            id: row.id,
            username: row.username,
            avatar_url: avatar_url(row.avatar_public_id.as_deref()),
            is_admin: row.is_admin,
            created_at: row.created_at,
            suspended_at: row.suspended_at,
        })
        .collect();
    Ok(Json(AdminUserList { items }))
}

/// The [`AdminUserOut`] of an existing account.
async fn admin_user_out(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<AdminUserOut> {
    let row = sqlx::query!(
        r#"SELECT u.id, u.username, m.public_id AS "avatar_public_id?", u.is_admin, u.created_at,
                  u.suspended_at
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.id = $1"#,
        user_id
    )
    .fetch_one(conn)
    .await?;
    Ok(AdminUserOut {
        id: row.id,
        username: row.username,
        avatar_url: avatar_url(row.avatar_public_id.as_deref()),
        is_admin: row.is_admin,
        created_at: row.created_at,
        suspended_at: row.suspended_at,
    })
}
