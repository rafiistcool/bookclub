//! The next-up vote (#45, owner CLUB; endpoint-map §9, docs/phase1/spec/club.md §1.17–§1.22,
//! §2.8, §2.9).
//!
//! - `ballot`: reading the vote, the deadline, nominations and ballots.
//! - `close`: applying a nomination by hand and the lazy auto-close at the deadline.
//! - `out`: `VoteOut` and its loader.
//! - `suggestions`: candidates for the next vote.
//!
//! **Electorate.** The club's current members: `member_count` and `not_voted` cover them, and
//! only their ballots count (`votes`, `voters`, `voted_count`, the leader, the auto-close and the
//! `vote_closed` payload). A member who leaves keeps their ballot row, which counts again if they
//! rejoin. A nomination stays credited to whoever made it.
//!
//! **Locks.** Every vote write takes the club row (`picks::lock_club_writes`, `FOR NO KEY
//! UPDATE`) and re-reads the caller's role under it, then the open vote row `FOR UPDATE`, in
//! that order (a club delete holds the club row, and its cascade then locks the vote rows). So
//! two first nominations create one vote, the limit of 6, the current-pick check and the
//! duplicate check cannot race, and a concurrent auto-close applies once.

pub mod ballot;
pub mod close;
pub mod out;
pub mod suggestions;

use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::clubs::extract::role_in;
use crate::error::{ApiResult, AppError, msg};
use crate::picks::lock_club_writes;

/// At most this many nominations per vote.
pub const NOMINATION_LIMIT: i64 = 6;

/// 404 on `PATCH …/vote` without an open vote.
pub const NOMINATE_FIRST: &str = "Nominate a book first";
/// 400 on `PATCH …/vote` with a deadline that is not in the future.
pub const DEADLINE_NOT_FUTURE: &str = "The deadline has to be in the future";
/// 400 for a seventh nomination.
pub const VOTE_FULL: &str = "This vote already has enough books";
/// 400 for nominating the current pick.
pub const ALREADY_THE_PICK: &str = "That book is already the club pick";
/// 400 for nominating a book twice in one vote.
pub const ALREADY_NOMINATED: &str = "That book is already nominated";
/// 404 on cast and apply without an open vote.
pub const NO_OPEN_VOTE: &str = "No open next-up vote";
/// 404 for a `nomination_id` that is not in the club's open vote.
pub const NO_NOMINATION: &str = "No nomination with that id";

/// The vote routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ballot::get_vote, ballot::set_deadline))
        .routes(routes!(ballot::nominate))
        .routes(routes!(ballot::cast))
        .routes(routes!(close::apply))
        .routes(routes!(suggestions::suggestions))
}

/// The club's open vote.
#[derive(Debug, Clone, Copy)]
pub struct OpenVote {
    pub id: i64,
    pub closes_at: Option<OffsetDateTime>,
}

/// The club's open vote, locked `FOR UPDATE`. Callers hold the club lock already.
pub async fn lock_open_vote(
    conn: &mut PgConnection,
    club_id: i64,
) -> sqlx::Result<Option<OpenVote>> {
    sqlx::query_as!(
        OpenVote,
        "SELECT id, closes_at FROM next_up_votes
         WHERE club_id = $1 AND ended_at IS NULL
         FOR UPDATE",
        club_id
    )
    .fetch_optional(conn)
    .await
}

/// Locks the club row ([`lock_club_writes`]) and checks that the caller is still a member: 404
/// `No such club` when the club is gone or the caller was removed meanwhile.
pub async fn lock_as_member(conn: &mut PgConnection, club_id: i64, user_id: i64) -> ApiResult<()> {
    if !lock_club_writes(conn, club_id).await? || role_in(conn, user_id, club_id).await?.is_none() {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    Ok(())
}

/// The work of nomination `nomination_id` in the vote; `None` for a missing nomination or one of
/// another vote (another club's included).
pub async fn nomination_work(
    conn: &mut PgConnection,
    vote_id: i64,
    nomination_id: i64,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "SELECT work_id FROM next_up_nominations WHERE vote_id = $1 AND id = $2",
        vote_id,
        nomination_id
    )
    .fetch_optional(conn)
    .await
}

/// A nomination with the number of ballots the club's current members cast for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub nomination_id: i64,
    pub work_id: i64,
    pub nominated_by: Option<i64>,
    pub votes: i64,
}

/// The vote's nominations in creation order (`created_at, id`) with their current-member votes.
pub async fn tallies(conn: &mut PgConnection, vote_id: i64) -> sqlx::Result<Vec<Tally>> {
    let rows = sqlx::query!(
        r#"SELECT n.id, n.work_id, n.nominated_by,
                  (SELECT count(*)
                   FROM next_up_ballots b
                   JOIN club_memberships m ON m.club_id = b.club_id AND m.user_id = b.user_id
                   WHERE b.vote_id = n.vote_id AND b.nomination_id = n.id) AS "votes!"
           FROM next_up_nominations n
           WHERE n.vote_id = $1
           ORDER BY n.created_at, n.id"#,
        vote_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Tally {
            nomination_id: row.id,
            work_id: row.work_id,
            nominated_by: row.nominated_by,
            votes: row.votes,
        })
        .collect())
}

/// The leader: the nomination with strictly the most votes, ties going to the earliest one;
/// `None` when nobody voted. `tallies` are in creation order.
pub fn leader(tallies: &[Tally]) -> Option<&Tally> {
    let mut best: Option<&Tally> = None;
    for tally in tallies {
        if tally.votes > best.map_or(0, |b| b.votes) {
            best = Some(tally);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(nomination_id: i64, votes: i64) -> Tally {
        Tally {
            nomination_id,
            work_id: nomination_id * 10,
            nominated_by: None,
            votes,
        }
    }

    fn lead(rows: &[(i64, i64)]) -> Option<i64> {
        let tallies: Vec<Tally> = rows.iter().map(|&(id, votes)| tally(id, votes)).collect();
        leader(&tallies).map(|t| t.nomination_id)
    }

    #[test]
    fn the_leader_has_strictly_the_most_votes_and_ties_go_to_the_earliest() {
        assert_eq!(lead(&[]), None);
        assert_eq!(lead(&[(1, 0), (2, 0)]), None);
        assert_eq!(lead(&[(1, 1), (2, 1)]), Some(1));
        assert_eq!(lead(&[(1, 1), (2, 2), (3, 2)]), Some(2));
        assert_eq!(lead(&[(7, 0), (3, 1)]), Some(3));
    }
}
