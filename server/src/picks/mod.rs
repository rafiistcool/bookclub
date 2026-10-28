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
//!
//! **Work merges** (`catalog::merge_works`, docs/phase1/notes/cat.md "For SHELF, POSTS and
//! CLUB") lock the two works, then the users with the loser on their shelf `FOR UPDATE`, then
//! the votes that nominated the loser, and then re-point picks, nominations and events. A club
//! write keeps that order:
//! 1. a write that stores a new `work_id` (`PUT pick`, nominate) locks the work first
//!    ([`lock_work`]), so a merge in flight finishes before it and the write stores the winner;
//! 2. the club row, then the user rows the write references ([`lock_users`]: the caller in
//!    [`lock_as_admin`] and `votes::lock_as_member`, every nominator in the auto-close);
//! 3. only then a vote row or the current pick's row.
//!
//! Without step 2 a write that held a vote or pick row would reach the user row only through
//! the foreign-key check of its insert (a ballot, a nomination, a pick's `set_by`, an event's
//! actor) and deadlock with a merge that holds that user and waits for the vote or pick.

pub mod current;
pub mod ics;
pub mod milestones;
pub mod out;

use jiff::tz::TimeZone;
use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::catalog::shared::books::require_work;
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
/// no longer an admin. The role the extractor read before the lock never decides a write. Then
/// the caller's user row ([`lock_users`]), before any vote or pick row.
pub async fn lock_as_admin(conn: &mut PgConnection, club_id: i64, user_id: i64) -> ApiResult<()> {
    if !lock_club_writes(conn, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    match role_in(conn, user_id, club_id).await? {
        None => return Err(AppError::not_found(msg::NO_SUCH_CLUB)),
        Some(role) if role < Role::Admin => return Err(AppError::forbidden(msg::ONLY_ADMINS)),
        Some(_) => {}
    }
    lock_users(conn, &[user_id]).await?;
    Ok(())
}

/// How many merges [`lock_work`] follows that commit while it waits, before it gives up.
const MERGE_HOPS: usize = 4;

/// Locks work `work_id`, or the work it was merged into, `FOR SHARE` and returns its id; 404
/// `No such book.` for an unknown id. The lock conflicts with the `FOR NO KEY UPDATE` of a merge
/// (docs/phase1/notes/cat.md "For SHELF, POSTS and CLUB"): a merge in flight finishes first,
/// and the write then stores the winner instead of leaving a row on the merged-away loser (a
/// second nomination of one book, a pick of a work that no longer exists on its own). A merge
/// that starts later waits for the write and moves its rows. Taken before the club row.
pub async fn lock_work(conn: &mut PgConnection, work_id: i64) -> ApiResult<i64> {
    let mut current = require_work(conn, work_id).await?;
    for _ in 0..MERGE_HOPS {
        let locked = sqlx::query_scalar!(
            "SELECT id FROM works WHERE id = $1 AND merged_into_id IS NULL FOR SHARE",
            current
        )
        .fetch_optional(&mut *conn)
        .await?;
        if locked.is_some() {
            return Ok(current);
        }
        current = require_work(conn, current).await?;
    }
    Err(AppError::internal(format!(
        "work {work_id} kept being merged while it was locked"
    )))
}

/// Locks the rows of `user_ids` `FOR KEY SHARE`, in id order: the users a club write
/// references, before it locks a vote or pick row (see the module docs on work merges). A
/// missing id is skipped.
pub async fn lock_users(conn: &mut PgConnection, user_ids: &[i64]) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        "SELECT id FROM users WHERE id = ANY($1) ORDER BY id FOR KEY SHARE",
        user_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(())
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
