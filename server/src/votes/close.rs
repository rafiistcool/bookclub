//! Closing the vote: applying a nomination by hand (`POST …/vote/apply`) and the lazy
//! auto-close at the deadline (docs/phase1/spec/club.md §1.21, §2.8).
//!
//! Both make the winner the club pick through `picks::current::set_pick` (a different work ends
//! the current pick and records `pick_set`; the same work changes only what is given), end the
//! vote, and record `vote_closed`. The auto-close credits the winning nominator, also when they
//! have left the club; when their account is gone, the pick's `set_by` and the events' actor are
//! NULL (work-plan appendix "club" 9).

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::out::{VoteOut, vote_out};
use super::{NO_NOMINATION, NO_OPEN_VOTE, leader, lock_open_vote, nomination_work, tallies};
use crate::AppState;
use crate::activity::record::{EventKind, NewEvent, record};
use crate::clubs::extract::ClubAdmin;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate};
use crate::picks::current::{PickWrite, set_pick};
use crate::picks::out::{ClubPickOut, current_row, pick_outs};
use crate::picks::{club_zone, lock_as_admin, lock_club_writes, lock_users};
use crate::tz;

/// Makes a nomination of the open vote the club pick.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VoteApplyIn {
    /// A nomination of the club's open vote (any, whatever its votes).
    pub nomination_id: i64,
    /// The meeting of the new pick, as in `ClubPickSetIn`. Absent, `null` or `""`: none (and,
    /// when the nominated book is already the current pick, its meeting is cleared).
    #[serde(default)]
    pub meeting_at: Option<String>,
}

impl Validate for VoteApplyIn {}

/// The new pick and the vote, which is then always the "no open vote" shape.
#[derive(Debug, Serialize, ToSchema)]
pub struct VoteApplyOut {
    pub pick: ClubPickOut,
    pub vote: VoteOut,
}

/// The winner of a vote being closed.
struct Winner {
    nomination_id: i64,
    work_id: i64,
    votes: i64,
}

/// Ends vote `vote_id` with `winner`: the winner's work becomes the pick (`set_pick` with
/// `actor` and `meeting_at`), the vote ends, and `vote_closed` is recorded with `{"votes"}`
/// (plus `"auto": true` for the auto-close). Returns the pick's id. The caller holds the club
/// and vote locks.
async fn close_vote(
    conn: &mut PgConnection,
    club_id: i64,
    vote_id: i64,
    winner: Winner,
    actor: Option<i64>,
    meeting_at: Option<Option<OffsetDateTime>>,
    auto: bool,
) -> ApiResult<i64> {
    let pick_id = set_pick(
        conn,
        club_id,
        PickWrite {
            work_id: winner.work_id,
            actor,
            note: None,
            meeting_at,
        },
    )
    .await?;
    sqlx::query!(
        "UPDATE next_up_votes SET ended_at = clock_timestamp(), winner_nomination_id = $2
         WHERE id = $1",
        vote_id,
        winner.nomination_id
    )
    .execute(&mut *conn)
    .await?;
    let mut payload = Map::new();
    payload.insert("votes".into(), Value::from(winner.votes));
    if auto {
        payload.insert("auto".into(), Value::Bool(true));
    }
    record(
        conn,
        NewEvent {
            club_id: Some(club_id),
            actor_id: actor,
            kind: EventKind::VoteClosed,
            work_id: Some(winner.work_id),
            pick_id: Some(pick_id),
            payload,
        },
    )
    .await?;
    Ok(pick_id)
}

/// The lazy auto-close (runs at the start of `GET …/vote`, nominate and cast), in the caller's
/// transaction. When the open vote's deadline has passed:
/// - with a leader, the leader's work becomes the pick, credited to its nominator (no meeting;
///   on the same work as the current pick, nothing of the pick changes), and the vote ends;
/// - without one (no ballots), the deadline is cleared and the vote stays open.
///
/// Takes the club row, then the user rows of every nominator of the open vote (the leader's
/// nominator becomes the pick's `set_by` and the events' actor; a work merge locks users before
/// votes, see `picks` on work merges), then the vote row, and checks the deadline again under
/// the lock, so two concurrent requests close the vote once. Nominations are added only under
/// the club lock (a merge re-points or drops them but adds no nominator), so the leader's
/// nominator is among the locked users. Returns whether it wrote anything.
pub async fn maybe_auto_close(conn: &mut PgConnection, club_id: i64) -> ApiResult<bool> {
    let due = sqlx::query_scalar!(
        "SELECT id FROM next_up_votes
         WHERE club_id = $1 AND ended_at IS NULL AND closes_at <= now()",
        club_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    if due.is_none() || !lock_club_writes(conn, club_id).await? {
        return Ok(false);
    }
    let nominators = sqlx::query_scalar!(
        r#"SELECT DISTINCT n.nominated_by AS "nominated_by!"
           FROM next_up_nominations n
           JOIN next_up_votes v ON v.id = n.vote_id
           WHERE v.club_id = $1 AND v.ended_at IS NULL AND n.nominated_by IS NOT NULL"#,
        club_id
    )
    .fetch_all(&mut *conn)
    .await?;
    lock_users(conn, &nominators).await?;
    let vote = sqlx::query_scalar!(
        "SELECT id FROM next_up_votes
         WHERE club_id = $1 AND ended_at IS NULL AND closes_at <= now()
         FOR UPDATE",
        club_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(vote_id) = vote else {
        return Ok(false);
    };
    let tallies = tallies(conn, vote_id).await?;
    let Some(winner) = leader(&tallies) else {
        sqlx::query!(
            "UPDATE next_up_votes SET closes_at = NULL WHERE id = $1",
            vote_id
        )
        .execute(&mut *conn)
        .await?;
        return Ok(true);
    };
    // An account deletion locks every club it writes in (this one, through the nomination)
    // before the user row, so the club lock already orders it with this close, and the
    // nominator's row is held `FOR KEY SHARE` (above) until the commit. A deleted account is no
    // actor (NULL).
    let actor = match winner.nominated_by {
        Some(user_id) => {
            sqlx::query_scalar!("SELECT id FROM users WHERE id = $1", user_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let winner = Winner {
        nomination_id: winner.nomination_id,
        work_id: winner.work_id,
        votes: winner.votes,
    };
    close_vote(conn, club_id, vote_id, winner, actor, None, true).await?;
    Ok(true)
}

/// [`maybe_auto_close`] in a transaction of its own, committed before the request's own work
/// (parity: a nomination refused with 400 afterwards does not undo the auto-close).
pub async fn run_auto_close(state: &AppState, club_id: i64) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    maybe_auto_close(&mut tx, club_id).await?;
    tx.commit().await?;
    Ok(())
}

/// Makes a nomination of the open vote the club pick (admins), whatever its votes: the current
/// pick ends, the new one is set by the caller with the given meeting, the vote ends and
/// `vote_closed` is recorded with the nomination's votes. When the nominated book is already the
/// current pick, only its meeting changes (cleared when none is given) and no `pick_set` is
/// recorded. Does not run the auto-close.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/vote/apply",
    operation_id = "votes_apply",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = VoteApplyIn,
    responses(
        (status = 200, body = VoteApplyOut, description = "The new pick; no vote is open any more"),
        (status = 400, body = ErrorBody, description = "`meeting_at must be a date and time`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No open next-up vote`, `No nomination with that id`")
    )
)]
pub async fn apply(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VoteApplyIn>,
) -> ApiResult<Json<VoteApplyOut>> {
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, club_id, member.user.id).await?;
    let vote = lock_open_vote(&mut tx, club_id)
        .await?
        .ok_or_else(|| AppError::not_found(NO_OPEN_VOTE))?;
    let work_id = nomination_work(&mut tx, vote.id, body.nomination_id)
        .await?
        .ok_or_else(|| AppError::not_found(NO_NOMINATION))?;
    let (zone, _) = club_zone(&member.club.timezone);
    let meeting_at = tz::parse_meeting(body.meeting_at.as_deref(), &zone)
        .map_err(|()| AppError::bad_request(msg::MEETING_AT_FORMAT))?;
    let votes = tallies(&mut tx, vote.id)
        .await?
        .into_iter()
        .find(|tally| tally.nomination_id == body.nomination_id)
        .map_or(0, |tally| tally.votes);
    let winner = Winner {
        nomination_id: body.nomination_id,
        work_id,
        votes,
    };
    close_vote(
        &mut tx,
        club_id,
        vote.id,
        winner,
        Some(member.user.id),
        Some(meeting_at),
        false,
    )
    .await?;
    let row = current_row(&mut tx, club_id)
        .await?
        .ok_or_else(|| AppError::internal("the applied pick is not current"))?;
    let pick = pick_outs(&mut tx, club_id, member.user.id, &zone, vec![row])
        .await?
        .pop()
        .ok_or_else(|| AppError::internal("the applied pick has no output"))?;
    let vote = vote_out(&mut tx, &member.club, member.user.id).await?;
    tx.commit().await?;
    Ok(Json(VoteApplyOut { pick, vote }))
}
