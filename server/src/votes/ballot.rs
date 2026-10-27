//! Reading the vote, its deadline, nominations and ballots (docs/phase1/spec/club.md
//! §1.17–§1.20, §2.8).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::Map;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::close::run_auto_close;
use super::out::{VoteOut, vote_out};
use super::{
    ALREADY_NOMINATED, ALREADY_THE_PICK, DEADLINE_NOT_FUTURE, NO_NOMINATION, NO_OPEN_VOTE,
    NOMINATE_FIRST, NOMINATION_LIMIT, VOTE_FULL, lock_as_member, lock_open_vote, nomination_work,
};
use crate::AppState;
use crate::activity::record::{EventKind, NewEvent, record};
use crate::catalog::shared::books::require_work;
use crate::clubs::extract::{ClubAdmin, ClubMember};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate};
use crate::picks::{club_zone, lock_as_admin};
use crate::tz;

/// Sets or clears the deadline.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VoteDeadlineIn {
    /// In the club's zone unless it carries an offset (`2030-01-01T18:00`, `…Z`). Absent,
    /// `null`, `""` or whitespace clear the deadline.
    #[serde(default)]
    pub closes_at: Option<String>,
}

impl Validate for VoteDeadlineIn {}

/// Nominates a book for the next pick.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VoteNominateIn {
    /// A work id from the catalog (`BookOut.id`).
    pub work_id: i64,
}

impl Validate for VoteNominateIn {}

/// Casts (or moves) the caller's ballot.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VoteCastIn {
    /// A nomination of the club's open vote.
    pub nomination_id: i64,
}

impl Validate for VoteCastIn {}

/// The club's open vote. Applies the leader first when the deadline has passed (or, with no
/// ballots, clears the deadline).
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/vote",
    operation_id = "votes_get",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = VoteOut, description = "The open vote, or `vote_id: null`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn get_vote(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<VoteOut>> {
    run_auto_close(&state, member.club.id).await?;
    let mut conn = state.db.acquire().await?;
    Ok(Json(
        vote_out(&mut conn, &member.club, member.user.id).await?,
    ))
}

/// Sets or clears the open vote's deadline (admins). Does not run the auto-close.
#[utoipa::path(
    patch,
    path = "/api/clubs/{slug}/vote",
    operation_id = "votes_set_deadline",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = VoteDeadlineIn,
    responses(
        (status = 200, body = VoteOut, description = "The open vote"),
        (status = 400, body = ErrorBody, description = "`meeting_at must be a date and time`, `The deadline has to be in the future`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `Nominate a book first`")
    )
)]
pub async fn set_deadline(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VoteDeadlineIn>,
) -> ApiResult<Json<VoteOut>> {
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    let vote = lock_open_vote(&mut tx, member.club.id)
        .await?
        .ok_or_else(|| AppError::not_found(NOMINATE_FIRST))?;
    let (zone, _) = club_zone(&member.club.timezone);
    let closes_at = tz::parse_meeting(body.closes_at.as_deref(), &zone)
        .map_err(|()| AppError::bad_request(msg::MEETING_AT_FORMAT))?;
    if closes_at.is_some_and(|at| at <= OffsetDateTime::now_utc()) {
        return Err(AppError::bad_request(DEADLINE_NOT_FUTURE));
    }
    sqlx::query!(
        "UPDATE next_up_votes SET closes_at = $2 WHERE id = $1",
        vote.id,
        closes_at
    )
    .execute(&mut *tx)
    .await?;
    let out = vote_out(&mut tx, &member.club, member.user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Nominates a book (members) and records `nominated`. The first nomination opens a vote. At
/// most 6 books per vote; not the current pick (past picks are fine); each book once.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/vote/nominations",
    operation_id = "votes_nominate",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = VoteNominateIn,
    responses(
        (status = 201, body = VoteOut, description = "The open vote with the new nomination"),
        (status = 400, body = ErrorBody, description = "`This vote already has enough books`, `That book is already the club pick`, `That book is already nominated`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No such book.`")
    )
)]
pub async fn nominate(
    member: ClubMember,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VoteNominateIn>,
) -> ApiResult<(StatusCode, Json<VoteOut>)> {
    run_auto_close(&state, member.club.id).await?;
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    let work_id = require_work(&mut tx, body.work_id).await?;
    lock_as_member(&mut tx, club_id, member.user.id).await?;
    let vote_id = match lock_open_vote(&mut tx, club_id).await? {
        Some(vote) => vote.id,
        None => {
            sqlx::query_scalar!(
                "INSERT INTO next_up_votes (club_id) VALUES ($1) RETURNING id",
                club_id
            )
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM next_up_nominations WHERE vote_id = $1"#,
        vote_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if count >= NOMINATION_LIMIT {
        return Err(AppError::bad_request(VOTE_FULL));
    }
    let is_pick = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM club_picks
                          WHERE club_id = $1 AND ended_at IS NULL AND work_id = $2) AS "is_pick!""#,
        club_id,
        work_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if is_pick {
        return Err(AppError::bad_request(ALREADY_THE_PICK));
    }
    let nominated = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM next_up_nominations
                          WHERE vote_id = $1 AND work_id = $2) AS "nominated!""#,
        vote_id,
        work_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if nominated {
        return Err(AppError::bad_request(ALREADY_NOMINATED));
    }
    sqlx::query!(
        "INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
         VALUES ($1, $2, $3, $4)",
        club_id,
        vote_id,
        work_id,
        member.user.id
    )
    .execute(&mut *tx)
    .await?;
    record(
        &mut tx,
        NewEvent {
            club_id: Some(club_id),
            actor_id: Some(member.user.id),
            kind: EventKind::Nominated,
            work_id: Some(work_id),
            pick_id: None,
            payload: Map::new(),
        },
    )
    .await?;
    let out = vote_out(&mut tx, &member.club, member.user.id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Casts the caller's ballot (members), or moves it to another nomination. `voted` is recorded
/// on the first ballot only.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/vote/cast",
    operation_id = "votes_cast",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = VoteCastIn,
    responses(
        (status = 200, body = VoteOut, description = "The open vote with the ballot"),
        (status = 400, body = ErrorBody, description = "`Field required` and other body errors"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No open next-up vote`, `No nomination with that id`")
    )
)]
pub async fn cast(
    member: ClubMember,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VoteCastIn>,
) -> ApiResult<Json<VoteOut>> {
    run_auto_close(&state, member.club.id).await?;
    let club_id = member.club.id;
    let mut tx = state.db.begin().await?;
    lock_as_member(&mut tx, club_id, member.user.id).await?;
    let vote = lock_open_vote(&mut tx, club_id)
        .await?
        .ok_or_else(|| AppError::not_found(NO_OPEN_VOTE))?;
    let work_id = nomination_work(&mut tx, vote.id, body.nomination_id)
        .await?
        .ok_or_else(|| AppError::not_found(NO_NOMINATION))?;
    let inserted = sqlx::query_scalar!(
        r#"INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (vote_id, user_id) DO UPDATE SET nomination_id = EXCLUDED.nomination_id
           RETURNING (xmax = 0) AS "inserted!""#,
        club_id,
        vote.id,
        member.user.id,
        body.nomination_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if inserted {
        record(
            &mut tx,
            NewEvent {
                club_id: Some(club_id),
                actor_id: Some(member.user.id),
                kind: EventKind::Voted,
                work_id: Some(work_id),
                pick_id: None,
                payload: Map::new(),
            },
        )
        .await?;
    }
    let out = vote_out(&mut tx, &member.club, member.user.id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
