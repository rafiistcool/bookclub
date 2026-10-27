//! Clubs (#40): clubs, memberships, roles, invites, join, leave and transfer
//! (docs/phase1/conventions.md §5.3, endpoint-map §3).
//!
//! - `manage`: create, list, read, update and delete clubs.
//! - `members`: the member list, role changes, removal, leaving and the ownership transfer.
//! - `invites`: the redemption seam sign-up shares, and the club's invite routes.
//! - `redeem`: the join-page preview and `POST /api/invites/{code}/accept`.
//! - `out`: the response types and their loaders.
//!
//! Every membership write, and every invite write, locks the club row first ([`lock_club`]) and
//! re-reads the roles it depends on inside its transaction (§5.3).

pub mod extract;
pub mod invites;
pub mod manage;
pub mod members;
pub mod out;
pub mod redeem;

use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::error::{ApiResult, AppError, msg};
use extract::{Role, role_in};

/// At most this many members per club (`This club is full`).
pub const MAX_MEMBERS: i64 = 20;

/// The club routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(manage::list_clubs, manage::create_club))
        .routes(routes!(
            manage::get_club,
            manage::update_club,
            manage::delete_club
        ))
        .routes(routes!(members::list_members))
        .routes(routes!(members::change_role, members::remove_member))
        .routes(routes!(members::leave))
        .routes(routes!(members::transfer))
        .routes(routes!(invites::list_invites, invites::create_invite))
        .routes(routes!(invites::revoke_invite))
        .routes(routes!(redeem::preview))
        .routes(routes!(redeem::accept))
}

/// Locks the club row (`FOR UPDATE`) for a membership write: every join, removal, role change,
/// leave, transfer, club delete and invite create or revoke takes it first, so they run one
/// after the other per club. False when the club no longer exists.
pub async fn lock_club(conn: &mut PgConnection, club_id: i64) -> sqlx::Result<bool> {
    let found = sqlx::query_scalar!("SELECT id FROM clubs WHERE id = $1 FOR UPDATE", club_id)
        .fetch_optional(conn)
        .await?;
    Ok(found.is_some())
}

/// Locks the club and returns the caller's role as it is now; 404 `No such club` when the club is
/// gone or the caller is no longer a member. The role the extractor read before the lock never
/// decides a write (§5.3).
pub async fn locked_role(conn: &mut PgConnection, user_id: i64, club_id: i64) -> ApiResult<Role> {
    if !lock_club(conn, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    role_in(conn, user_id, club_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_CLUB))
}

/// The id and current role of the member with this (normalised) username, inside the caller's
/// transaction; `None` for an unknown name or a non-member.
pub async fn member_named(
    conn: &mut PgConnection,
    club_id: i64,
    username: &str,
) -> sqlx::Result<Option<(i64, Role)>> {
    let row = sqlx::query!(
        r#"SELECT u.id, m.role AS "role: Role"
           FROM users u
           JOIN club_memberships m ON m.user_id = u.id AND m.club_id = $1
           WHERE u.username = $2"#,
        club_id,
        username
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| (row.id, row.role)))
}
