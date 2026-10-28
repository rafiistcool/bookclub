//! The current pick and the history (endpoint-map §9, docs/phase1/spec/club.md §1.1–§1.4, §2.3).

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::PgConnection;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use super::out::{
    ClubPickCurrentOut, ClubPickHistoryOut, ClubPickOut, current_row, history_rows, pick_outs,
};
use super::{NO_PICK_YET, club_zone, lock_as_admin, lock_club_writes, lock_work};
use crate::AppState;
use crate::activity::record::{EventKind, NewEvent, record};
use crate::clubs::extract::{ClubAdmin, ClubCtx, ClubMember};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Patch, Validate, Validator};
use crate::text::{char_len, py_strip};
use crate::tz;

pub const NOTE_TOO_LONG: &str = "note must be 280 characters or fewer";
const MAX_NOTE_CHARS: usize = 280;

/// Sets the club's pick. The same work as the current pick updates only the keys present; a
/// different work ends the current pick and starts a new one.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClubPickSetIn {
    /// A work id from the catalog (`BookOut.id`).
    pub work_id: i64,
    /// At most 280 characters after trimming. Absent: `""` on a new pick, unchanged on the same
    /// work; `null` clears it.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub note: Patch<String>,
    /// The meeting, in the club's zone unless it carries an offset (`2026-08-22T19:00`,
    /// `2026-08-22`, `…Z`, `…+02:00`). Absent: none on a new pick, unchanged on the same work;
    /// `null` or `""` clears it.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub meeting_at: Patch<String>,
}

impl Validate for ClubPickSetIn {
    fn validate(&mut self, v: &mut Validator) {
        validate_note(&mut self.note, v);
    }
}

/// Edits the current pick's note and meeting; absent keys stay as they are.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClubPickPatchIn {
    /// At most 280 characters after trimming; `null` clears it.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub note: Patch<String>,
    /// As in `ClubPickSetIn`; `null` or `""` clears it.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub meeting_at: Patch<String>,
}

impl Validate for ClubPickPatchIn {
    fn validate(&mut self, v: &mut Validator) {
        validate_note(&mut self.note, v);
    }
}

fn validate_note(note: &mut Patch<String>, v: &mut Validator) {
    if let Patch::Value(text) = note {
        *text = py_strip(text).to_owned();
        v.check(char_len(text) <= MAX_NOTE_CHARS, NOTE_TOO_LONG);
    }
}

/// A note change: `None` when the key was absent, `""` for `null`.
fn note_change(note: Patch<String>) -> Option<String> {
    note.into_option().map(Option::unwrap_or_default)
}

/// A meeting change parsed in the club's zone: `None` when the key was absent, `Some(None)` for
/// `null`, `""` or whitespace. 400 `meeting_at must be a date and time` for anything else that
/// does not parse.
fn meeting_change(
    meeting_at: &Patch<String>,
    club: &ClubCtx,
) -> ApiResult<Option<Option<OffsetDateTime>>> {
    let Some(value) = meeting_at.as_option() else {
        return Ok(None);
    };
    let (zone, _) = club_zone(&club.timezone);
    tz::parse_meeting(value.map(String::as_str), &zone)
        .map(Some)
        .map_err(|()| AppError::bad_request(msg::MEETING_AT_FORMAT))
}

/// What [`set_pick`] writes.
#[derive(Debug, Clone)]
pub struct PickWrite {
    /// The work, already resolved through `require_work` (and locked by [`super::lock_work`]
    /// when it comes from the request).
    pub work_id: i64,
    /// The setter of a new pick and the actor of its `pick_set` event; `None` for a deleted
    /// account (auto-close).
    pub actor: Option<i64>,
    /// `None`: unchanged on the same work, `""` on a new pick.
    pub note: Option<String>,
    /// `None`: unchanged on the same work, no meeting on a new pick.
    pub meeting_at: Option<Option<OffsetDateTime>>,
}

/// Makes `write.work_id` the club's current pick and returns the pick's id.
///
/// - The current pick is of the same work: only the fields given change; `set_by`,
///   `started_at` and the history stay, and no event is recorded.
/// - Otherwise the current pick (if any) ends, a new one starts with `set_by` = the actor, and
///   `pick_set` is recorded with `{"meeting_at"}` when a meeting is set.
///
/// Takes the club-row lock itself ([`lock_club_writes`]), so callers may hold it already.
/// `ended_at` and `started_at` are `clock_timestamp()` under the lock, so a pick never ends
/// before it started, whichever transaction began first. 404 `No such club` when the club is
/// gone.
pub async fn set_pick(conn: &mut PgConnection, club_id: i64, write: PickWrite) -> ApiResult<i64> {
    if !lock_club_writes(conn, club_id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    let current = sqlx::query!(
        "SELECT id, work_id, note, meeting_at FROM club_picks
         WHERE club_id = $1 AND ended_at IS NULL",
        club_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(current) = &current
        && current.work_id == write.work_id
    {
        let note = write.note.unwrap_or_else(|| current.note.clone());
        let meeting_at = write.meeting_at.unwrap_or(current.meeting_at);
        sqlx::query!(
            "UPDATE club_picks SET note = $2, meeting_at = $3 WHERE id = $1",
            current.id,
            note,
            meeting_at
        )
        .execute(&mut *conn)
        .await?;
        return Ok(current.id);
    }
    if let Some(current) = &current {
        sqlx::query!(
            "UPDATE club_picks SET ended_at = clock_timestamp() WHERE id = $1",
            current.id
        )
        .execute(&mut *conn)
        .await?;
    }
    let meeting_at = write.meeting_at.flatten();
    let pick_id = sqlx::query_scalar!(
        "INSERT INTO club_picks (club_id, work_id, set_by, note, meeting_at, started_at)
         VALUES ($1, $2, $3, $4, $5, clock_timestamp())
         RETURNING id",
        club_id,
        write.work_id,
        write.actor,
        write.note.unwrap_or_default(),
        meeting_at
    )
    .fetch_one(&mut *conn)
    .await?;
    let mut payload = Map::new();
    if let Some(at) = meeting_at {
        payload.insert("meeting_at".into(), Value::String(rfc3339(at)?));
    }
    record(
        conn,
        NewEvent {
            club_id: Some(club_id),
            actor_id: write.actor,
            kind: EventKind::PickSet,
            work_id: Some(write.work_id),
            pick_id: Some(pick_id),
            payload,
        },
    )
    .await?;
    Ok(pick_id)
}

/// An event payload timestamp: RFC 3339 in UTC with `Z`.
pub fn rfc3339(at: OffsetDateTime) -> ApiResult<String> {
    at.format(&Rfc3339).map_err(AppError::internal)
}

/// The current pick of the caller's club as [`ClubPickOut`]; `None` without one.
async fn current_out(
    conn: &mut PgConnection,
    member: &ClubMember,
) -> ApiResult<Option<ClubPickOut>> {
    let Some(row) = current_row(conn, member.club.id).await? else {
        return Ok(None);
    };
    let (zone, _) = club_zone(&member.club.timezone);
    Ok(
        pick_outs(conn, member.club.id, member.user.id, &zone, vec![row])
            .await?
            .pop(),
    )
}

/// The club's current pick (no side effects).
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/pick",
    operation_id = "picks_get",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = ClubPickCurrentOut, description = "The current pick, or null"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn get_pick(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<ClubPickCurrentOut>> {
    let mut conn = state.db.acquire().await?;
    let pick = current_out(&mut conn, &member).await?;
    let (_, timezone) = club_zone(&member.club.timezone);
    Ok(Json(ClubPickCurrentOut { pick, timezone }))
}

/// Sets the club's pick (admins). The same work as the current pick updates only the keys
/// present (no history entry, no event); a different work ends the current pick and records
/// `pick_set`.
#[utoipa::path(
    put,
    path = "/api/clubs/{slug}/pick",
    operation_id = "picks_set",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = ClubPickSetIn,
    responses(
        (status = 200, body = ClubPickOut, description = "The current pick"),
        (status = 400, body = ErrorBody, description = "`note must be 280 characters or fewer`, `meeting_at must be a date and time`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No such book.`")
    )
)]
pub async fn put_pick(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ClubPickSetIn>,
) -> ApiResult<Json<ClubPickOut>> {
    let meeting_at = meeting_change(&body.meeting_at, &member.club)?;
    let mut tx = state.db.begin().await?;
    let work_id = lock_work(&mut tx, body.work_id).await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    set_pick(
        &mut tx,
        member.club.id,
        PickWrite {
            work_id,
            actor: Some(member.user.id),
            note: note_change(body.note),
            meeting_at,
        },
    )
    .await?;
    let out = current_out(&mut tx, &member)
        .await?
        .ok_or_else(|| AppError::internal("the pick just set is not current"))?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Edits the current pick's note and meeting (admins) without touching `set_by`.
#[utoipa::path(
    patch,
    path = "/api/clubs/{slug}/pick",
    operation_id = "picks_update",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = ClubPickPatchIn,
    responses(
        (status = 200, body = ClubPickOut, description = "The current pick"),
        (status = 400, body = ErrorBody, description = "`note must be 280 characters or fewer`, `meeting_at must be a date and time`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick yet`")
    )
)]
pub async fn patch_pick(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ClubPickPatchIn>,
) -> ApiResult<Json<ClubPickOut>> {
    let meeting_at = meeting_change(&body.meeting_at, &member.club)?;
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    let current = current_row(&mut tx, member.club.id)
        .await?
        .ok_or_else(|| AppError::not_found(NO_PICK_YET))?;
    let note = note_change(body.note).unwrap_or(current.note);
    let meeting_at = meeting_at.unwrap_or(current.meeting_at);
    sqlx::query!(
        "UPDATE club_picks SET note = $2, meeting_at = $3 WHERE id = $1",
        current.id,
        note,
        meeting_at
    )
    .execute(&mut *tx)
    .await?;
    let out = current_out(&mut tx, &member)
        .await?
        .ok_or_else(|| AppError::internal("the edited pick is not current"))?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Ends the current pick without a successor (admins) and records `pick_cleared`. Without a
/// current pick nothing is written. The answer is always `{"pick": null}`.
#[utoipa::path(
    delete,
    path = "/api/clubs/{slug}/pick",
    operation_id = "picks_clear",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = ClubPickCurrentOut, description = "No current pick any more"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn clear_pick(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<ClubPickCurrentOut>> {
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    let ended = sqlx::query!(
        "UPDATE club_picks SET ended_at = clock_timestamp()
         WHERE club_id = $1 AND ended_at IS NULL
         RETURNING id, work_id",
        member.club.id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(ended) = ended {
        record(
            &mut tx,
            NewEvent {
                club_id: Some(member.club.id),
                actor_id: Some(member.user.id),
                kind: EventKind::PickCleared,
                work_id: Some(ended.work_id),
                pick_id: Some(ended.id),
                payload: Map::new(),
            },
        )
        .await?;
    }
    tx.commit().await?;
    let (_, timezone) = club_zone(&member.club.timezone);
    Ok(Json(ClubPickCurrentOut {
        pick: None,
        timezone,
    }))
}

/// The club's ended picks, the most recently ended first. Readers are empty on ended picks; the
/// caller's own shelf status is still filled in.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/pick/history",
    operation_id = "picks_history",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = ClubPickHistoryOut, description = "Ended picks"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn history(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<ClubPickHistoryOut>> {
    let mut conn = state.db.acquire().await?;
    let rows = history_rows(&mut conn, member.club.id).await?;
    let (zone, timezone) = club_zone(&member.club.timezone);
    let items = pick_outs(&mut conn, member.club.id, member.user.id, &zone, rows).await?;
    Ok(Json(ClubPickHistoryOut { items, timezone }))
}
