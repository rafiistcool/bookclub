//! Club picks, meetings and milestones (#45, owner CLUB; endpoint-map §9,
//! docs/phase1/spec/club.md §1.1–§1.4, §1.12–§1.16, §2.3, §2.5, §2.7, §2.12).
//!
//! - `current`: the current pick (`GET`, `PUT`, `PATCH`, `DELETE /api/clubs/{slug}/pick`), the
//!   history, and [`current::set_pick`], which the vote's apply and auto-close reuse.
//! - `out`: `ClubPickOut` and its loader (readers are club members, read through
//!   `bc_visible_shelf`).
//! - `ics`: the meeting download and its RFC 5545 rendering.
//! - `milestones`: the reading schedule of a pick.
//!
//! **Locks.** Every pick and milestone write takes the club row `FOR NO KEY UPDATE` first
//! ([`lock_as_admin`]) and re-reads the caller's role under it. That serialises the writes per
//! club (two concurrent `PUT pick` calls run one after the other, so one current pick is left)
//! without blocking the foreign-key checks (`FOR KEY SHARE`) of other club writes. Membership
//! writes take the same row `FOR UPDATE`, so a role cannot change while a pick write runs. Code
//! that also locks a vote row takes the club row first, then the vote row: a club delete holds
//! the club row and its cascade then locks the vote rows.

pub mod current;
pub mod ics;
pub mod milestones;
pub mod out;

use jiff::tz::TimeZone;
use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::clubs::extract::{Role, role_in};
use crate::error::{ApiResult, AppError, msg};
use crate::tz;

/// 404 on `PATCH /api/clubs/{slug}/pick` without a current pick.
pub const NO_PICK_YET: &str = "No club pick yet";
/// 404 for a `{pick_id}` that is not a pick of this club.
pub const NO_PICK_WITH_ID: &str = "No club pick with that id";
/// 400 when a milestone is added to an ended pick.
pub const PICK_CLOSED: &str = "That pick is closed";
/// 404 on the meeting download without a current pick or meeting.
pub const NO_MEETING: &str = "No meeting scheduled";

/// The pick routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            current::get_pick,
            current::put_pick,
            current::patch_pick,
            current::clear_pick
        ))
        .routes(routes!(current::history))
        .routes(routes!(ics::meeting_ics))
        .routes(routes!(
            milestones::list_milestones,
            milestones::create_milestone
        ))
        .routes(routes!(
            milestones::update_milestone,
            milestones::delete_milestone
        ))
}

/// Locks the club row for a pick or milestone write (`FOR NO KEY UPDATE`). False when the club
/// no longer exists.
pub async fn lock_club_writes(conn: &mut PgConnection, club_id: i64) -> sqlx::Result<bool> {
    let found = sqlx::query_scalar!(
        "SELECT id FROM clubs WHERE id = $1 FOR NO KEY UPDATE",
        club_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(found.is_some())
}

/// [`lock_club_writes`], then the caller's role as it is now: 404 `No such club` when the club
/// is gone or the caller is no longer a member, 403 `Only club admins can do that` when they are
/// no longer an admin. The role the extractor read before the lock never decides a write.
pub async fn lock_as_admin(conn: &mut PgConnection, club_id: i64, user_id: i64) -> ApiResult<()> {
    if !lock_club_writes(conn, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    match role_in(conn, user_id, club_id).await? {
        None => Err(AppError::not_found(msg::NO_SUCH_CLUB)),
        Some(role) if role < Role::Admin => Err(AppError::forbidden(msg::ONLY_ADMINS)),
        Some(_) => Ok(()),
    }
}

/// The club's zone and the name every `timezone` response field carries: the stored name, or
/// `UTC` when the stored value is not a zone (a corrupt row; writes validate it).
pub fn club_zone(stored: &str) -> (TimeZone, String) {
    let zone = tz::zone_or_utc(stored);
    let name = zone.iana_name().unwrap_or("UTC").to_owned();
    (zone, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corrupt_club_zone_reads_as_utc() {
        let (zone, name) = club_zone("Not/AZone");
        assert_eq!(name, "UTC");
        assert_eq!(zone, TimeZone::UTC);
        let (_, name) = club_zone("America/New_York");
        assert_eq!(name, "America/New_York");
    }
}
