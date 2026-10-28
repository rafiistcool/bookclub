//! `VoteOut` and its loader (docs/phase1/spec/club.md §1.17, endpoint-map §9).

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::{NOMINATION_LIMIT, leader, tallies};
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::clubs::extract::ClubCtx;
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError};
use crate::picks::club_zone;
use crate::tz;

/// The club's open next-up vote, or the "no open vote" shape (`vote_id: null`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoteOut {
    /// Null when no vote is open; the first nomination opens one.
    pub vote_id: Option<i64>,
    /// Most votes first, then by id.
    pub nominations: Vec<VoteNominationOut>,
    /// The nomination the caller's ballot is on.
    pub my_vote_id: Option<i64>,
    /// Always 6.
    pub nomination_limit: i64,
    /// Fewer than 6 nominations (true when no vote is open).
    pub can_nominate: bool,
    /// The club's IANA zone, in which `closes_local` and `closes_label` are given.
    pub timezone: String,
    /// The deadline, if any; at the deadline the leader becomes the club pick.
    #[serde(with = "time::serde::rfc3339::option")]
    pub closes_at: Option<OffsetDateTime>,
    /// The deadline in the club's zone, `YYYY-MM-DDTHH:MM`.
    pub closes_local: Option<String>,
    /// The deadline in the club's zone, e.g. `Tue 01 Jan 2030, 18:00 UTC` (always English).
    pub closes_label: Option<String>,
    /// Club members without a ballot, by username; empty when no vote is open.
    pub not_voted: Vec<String>,
    /// Club members with a ballot.
    pub voted_count: i64,
    /// The club's members.
    pub member_count: i64,
    /// The nomination in the lead (most votes, ties to the earliest); null when nobody voted.
    pub leader_id: Option<i64>,
}

/// One nominated book.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoteNominationOut {
    pub id: i64,
    /// The work, without an edition context.
    pub book: BookOut,
    /// The nominator's username; null when that account is gone.
    pub nominated_by: Option<String>,
    /// The number of club members whose ballot is on this nomination.
    pub votes: i64,
    /// Their usernames, sorted.
    pub voters: Vec<String>,
    /// The caller's ballot is on this nomination.
    pub mine: bool,
}

/// The club's open vote as `viewer` sees it (the "no open vote" shape when none is open).
/// Reads without locks; inside a write transaction it sees that transaction's writes.
pub async fn vote_out(conn: &mut PgConnection, club: &ClubCtx, viewer: i64) -> ApiResult<VoteOut> {
    let (zone, timezone) = club_zone(&club.timezone);
    let members = sqlx::query!(
        "SELECT u.id, u.username FROM club_memberships m
         JOIN users u ON u.id = m.user_id
         WHERE m.club_id = $1
         ORDER BY u.username",
        club.id
    )
    .fetch_all(&mut *conn)
    .await?;
    let member_count = members.len() as i64;
    let vote = sqlx::query!(
        "SELECT id, closes_at FROM next_up_votes WHERE club_id = $1 AND ended_at IS NULL",
        club.id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(vote) = vote else {
        return Ok(VoteOut {
            vote_id: None,
            nominations: Vec::new(),
            my_vote_id: None,
            nomination_limit: NOMINATION_LIMIT,
            can_nominate: true,
            timezone,
            closes_at: None,
            closes_local: None,
            closes_label: None,
            not_voted: Vec::new(),
            voted_count: 0,
            member_count,
            leader_id: None,
        });
    };

    let tallies = tallies(conn, vote.id).await?;
    let leader_id = leader(&tallies).map(|tally| tally.nomination_id);
    let nominators: HashMap<i64, Option<String>> = sqlx::query!(
        r#"SELECT n.id AS "id!", u.username AS "username?"
           FROM next_up_nominations n
           LEFT JOIN users u ON u.id = n.nominated_by
           WHERE n.vote_id = $1"#,
        vote.id
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.id, row.username))
    .collect();
    // The ballots of current members (the same rule as `tallies`).
    let ballots = sqlx::query!(
        "SELECT b.nomination_id, b.user_id, u.username
         FROM next_up_ballots b
         JOIN club_memberships m ON m.club_id = b.club_id AND m.user_id = b.user_id
         JOIN users u ON u.id = b.user_id
         WHERE b.vote_id = $1",
        vote.id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut voters: HashMap<i64, Vec<String>> = HashMap::new();
    let mut my_vote_id = None;
    for ballot in &ballots {
        voters
            .entry(ballot.nomination_id)
            .or_default()
            .push(ballot.username.clone());
        if ballot.user_id == viewer {
            my_vote_id = Some(ballot.nomination_id);
        }
    }

    let refs: Vec<BookRef> = tallies.iter().map(|t| BookRef::work(t.work_id)).collect();
    let books = book_outs(conn, &refs).await?;
    let mut nominations = Vec::with_capacity(tallies.len());
    for tally in &tallies {
        let book = books
            .get(&BookRef::work(tally.work_id))
            .cloned()
            .ok_or_else(|| {
                AppError::internal(format!(
                    "nomination {} is missing its book",
                    tally.nomination_id
                ))
            })?;
        let mut names = voters.remove(&tally.nomination_id).unwrap_or_default();
        names.sort();
        nominations.push(VoteNominationOut {
            id: tally.nomination_id,
            book,
            nominated_by: nominators.get(&tally.nomination_id).cloned().flatten(),
            votes: names.len() as i64,
            mine: my_vote_id == Some(tally.nomination_id),
            voters: names,
        });
    }
    nominations.sort_by_key(|n| (-n.votes, n.id));

    let voted: std::collections::HashSet<i64> = ballots.iter().map(|b| b.user_id).collect();
    let not_voted: Vec<String> = members
        .iter()
        .filter(|member| !voted.contains(&member.id))
        .map(|member| member.username.clone())
        .collect();
    Ok(VoteOut {
        vote_id: Some(vote.id),
        can_nominate: (nominations.len() as i64) < NOMINATION_LIMIT,
        nominations,
        my_vote_id,
        nomination_limit: NOMINATION_LIMIT,
        timezone,
        closes_at: vote.closes_at,
        closes_local: vote.closes_at.map(|at| tz::local_minute(at, &zone)),
        closes_label: vote.closes_at.map(|at| tz::label(at, &zone)),
        voted_count: member_count - not_voted.len() as i64,
        not_voted,
        member_count,
        leader_id,
    })
}
