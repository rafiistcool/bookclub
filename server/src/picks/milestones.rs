//! The reading schedule of a pick (endpoint-map §9, docs/phase1/spec/club.md §1.13–§1.16, §2.7,
//! §5.5): a title, an optional range in percent or chapters, and an optional due time.
//!
//! Changes from the legacy app: `chapter_from`/`chapter_to` are `range_from`/`range_to` with a
//! `unit` (`percent`, `chapter`, or null for a title-only checkpoint); PATCH changes only the
//! keys present; the backwards check also runs on PATCH. The closed-pick check stays create-only.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::current::rfc3339;
use super::{NO_PICK_WITH_ID, PICK_CLOSED, club_zone, lock_as_admin};
use crate::AppState;
use crate::activity::record::{EventKind, NewEvent, record};
use crate::clubs::extract::{ClubAdmin, ClubMember};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, Patch, Validate, Validator};
use crate::text::{char_len, py_strip};
use crate::tz;

pub const NO_MILESTONE: &str = "No milestone with that id";
pub const TITLE_NEEDED: &str = "Give the milestone a title";
pub const TITLE_TOO_LONG: &str = "Keep the title to 120 characters";
pub const RANGE_BACKWARDS: &str = "Chapter range is backwards";
pub const PERCENT_TOO_HIGH: &str = "Percent milestones go up to 100";
pub const RANGE_NEEDS_UNIT: &str = "A range needs a unit";
const MAX_TITLE_CHARS: usize = 120;
const MAX_PERCENT: i32 = 100;

/// What a milestone's range counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum MilestoneUnit {
    Percent,
    Chapter,
}

impl MilestoneUnit {
    /// The stored and serialised name.
    pub fn as_str(self) -> &'static str {
        match self {
            MilestoneUnit::Percent => "percent",
            MilestoneUnit::Chapter => "chapter",
        }
    }
}

/// A new milestone.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MilestoneIn {
    /// 1 to 120 characters after trimming.
    pub title: String,
    /// Needed for a range; null for a title-only checkpoint.
    #[serde(default)]
    pub unit: Option<MilestoneUnit>,
    /// Inclusive start, ≥ 0; null = "up to `range_to`".
    #[serde(default)]
    pub range_from: Option<i32>,
    /// Inclusive end, ≥ 0 and not below `range_from`; at most 100 for percent.
    #[serde(default)]
    pub range_to: Option<i32>,
    /// In the club's zone unless it carries an offset; null or `""` for none.
    #[serde(default)]
    pub due_at: Option<String>,
}

impl Validate for MilestoneIn {
    fn validate(&mut self, v: &mut Validator) {
        self.title = milestone_title(&self.title, v);
        not_negative(self.range_from, v);
        not_negative(self.range_to, v);
        for message in shape_errors(self.unit, self.range_from, self.range_to) {
            v.fail(message);
        }
    }
}

/// Milestone changes: only the keys present change; `null` clears `unit`, a range end or
/// `due_at` (a title cannot be cleared).
#[derive(Debug, Deserialize, ToSchema)]
pub struct MilestonePatchIn {
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub title: Patch<String>,
    #[serde(default)]
    #[schema(value_type = Option<MilestoneUnit>)]
    pub unit: Patch<MilestoneUnit>,
    #[serde(default)]
    #[schema(value_type = Option<i32>)]
    pub range_from: Patch<i32>,
    #[serde(default)]
    #[schema(value_type = Option<i32>)]
    pub range_to: Patch<i32>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub due_at: Patch<String>,
}

impl Validate for MilestonePatchIn {
    fn validate(&mut self, v: &mut Validator) {
        match &mut self.title {
            Patch::Missing => {}
            Patch::Null => v.fail(TITLE_NEEDED),
            Patch::Value(title) => *title = milestone_title(title, v),
        }
        if let Patch::Value(from) = self.range_from {
            not_negative(Some(from), v);
        }
        if let Patch::Value(to) = self.range_to {
            not_negative(Some(to), v);
        }
    }
}

/// The trimmed title, recording `Give the milestone a title` or `Keep the title to 120
/// characters`.
fn milestone_title(raw: &str, v: &mut Validator) -> String {
    let title = py_strip(raw).to_owned();
    if title.is_empty() {
        v.fail(TITLE_NEEDED);
    } else if char_len(&title) > MAX_TITLE_CHARS {
        v.fail(TITLE_TOO_LONG);
    }
    title
}

fn not_negative(value: Option<i32>, v: &mut Validator) {
    v.range_i64(value.map(i64::from), 0, i64::MAX);
}

/// The unit and range rules that do not depend on order: a range needs a unit, and percent
/// ranges end at 100.
fn shape_errors(
    unit: Option<MilestoneUnit>,
    from: Option<i32>,
    to: Option<i32>,
) -> Vec<&'static str> {
    let mut errors = Vec::new();
    let has_range = from.is_some() || to.is_some();
    if unit.is_none() && has_range {
        errors.push(RANGE_NEEDS_UNIT);
    }
    if unit == Some(MilestoneUnit::Percent)
        && [from, to]
            .into_iter()
            .flatten()
            .any(|end| end > MAX_PERCENT)
    {
        errors.push(PERCENT_TOO_HIGH);
    }
    errors
}

fn is_backwards(from: Option<i32>, to: Option<i32>) -> bool {
    matches!((from, to), (Some(from), Some(to)) if to < from)
}

/// `due_at` in the club's zone; 400 `meeting_at must be a date and time` (the legacy message,
/// shared with the meeting and the vote deadline).
fn parse_due(value: Option<&str>, zone: &TimeZone) -> ApiResult<Option<OffsetDateTime>> {
    tz::parse_meeting(value, zone).map_err(|()| AppError::bad_request(msg::MEETING_AT_FORMAT))
}

/// One step of a pick's reading schedule.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MilestoneOut {
    pub id: i64,
    pub pick_id: i64,
    pub title: String,
    pub unit: Option<MilestoneUnit>,
    pub range_from: Option<i32>,
    pub range_to: Option<i32>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
    /// `YYYY-MM-DDTHH:MM` in the club's zone.
    pub due_local: Option<String>,
    /// E.g. `Sat 05 Jan 2030, 20:00 UTC`, in the club's zone (always English).
    pub due_label: Option<String>,
    /// 0-based, dense.
    pub position: i32,
    /// Live club posts tied to the milestone.
    pub note_count: i64,
    /// The due time has come.
    pub passed: bool,
}

/// A pick's milestones, in schedule order.
#[derive(Debug, Serialize, ToSchema)]
pub struct MilestoneListOut {
    pub pick_id: i64,
    pub items: Vec<MilestoneOut>,
    pub timezone: String,
}

/// The milestones of `pick_id` (or just `only`) in `(position, id)` order.
async fn milestone_outs(
    conn: &mut PgConnection,
    club_id: i64,
    pick_id: i64,
    only: Option<i64>,
    zone: &TimeZone,
) -> sqlx::Result<Vec<MilestoneOut>> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.pick_id, m.title, m.unit AS "unit: MilestoneUnit", m.range_from,
                  m.range_to, m.due_at, m.position,
                  (SELECT count(*) FROM posts p
                   WHERE p.club_id = m.club_id AND p.pick_id = m.pick_id
                     AND p.milestone_id = m.id AND p.deleted_at IS NULL) AS "note_count!",
                  (m.due_at IS NOT NULL AND m.due_at <= now()) AS "passed!"
           FROM milestones m
           WHERE m.club_id = $1 AND m.pick_id = $2 AND ($3::int8 IS NULL OR m.id = $3)
           ORDER BY m.position, m.id"#,
        club_id,
        pick_id,
        only
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| MilestoneOut {
            id: row.id,
            pick_id: row.pick_id,
            title: row.title,
            unit: row.unit,
            range_from: row.range_from,
            range_to: row.range_to,
            due_at: row.due_at,
            due_local: row.due_at.map(|at| tz::local_minute(at, zone)),
            due_label: row.due_at.map(|at| tz::label(at, zone)),
            position: row.position,
            note_count: row.note_count,
            passed: row.passed,
        })
        .collect())
}

async fn milestone_out(
    conn: &mut PgConnection,
    club_id: i64,
    pick_id: i64,
    milestone_id: i64,
    zone: &TimeZone,
) -> ApiResult<MilestoneOut> {
    milestone_outs(conn, club_id, pick_id, Some(milestone_id), zone)
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(NO_MILESTONE))
}

/// The pick `pick_id` of the club: its work and whether it has ended. 404 `No club pick with
/// that id` for a missing pick and for another club's.
async fn club_pick(conn: &mut PgConnection, club_id: i64, pick_id: i64) -> ApiResult<(i64, bool)> {
    let row = sqlx::query!(
        r#"SELECT work_id, (ended_at IS NOT NULL) AS "ended!" FROM club_picks
           WHERE club_id = $1 AND id = $2"#,
        club_id,
        pick_id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_PICK_WITH_ID))?;
    Ok((row.work_id, row.ended))
}

/// The pick's milestones in schedule order.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/picks/{pick_id}/milestones",
    operation_id = "picks_list_milestones",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "A pick of the club, current or ended")
    ),
    responses(
        (status = 200, body = MilestoneListOut, description = "The milestones"),
        (status = 400, body = ErrorBody, description = "A `pick_id` that is not a number"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`")
    )
)]
pub async fn list_milestones(
    member: ClubMember,
    ApiPath(path): ApiPath<(String, i64)>,
    State(state): State<AppState>,
) -> ApiResult<Json<MilestoneListOut>> {
    let (_, pick_id) = path;
    let mut conn = state.db.acquire().await?;
    club_pick(&mut conn, member.club.id, pick_id).await?;
    let (zone, timezone) = club_zone(&member.club.timezone);
    let items = milestone_outs(&mut conn, member.club.id, pick_id, None, &zone).await?;
    Ok(Json(MilestoneListOut {
        pick_id,
        items,
        timezone,
    }))
}

/// Adds a milestone at the end of the schedule (admins) and records `milestone_added`. An ended
/// pick takes no new milestones.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/picks/{pick_id}/milestones",
    operation_id = "picks_create_milestone",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "The current pick")
    ),
    request_body = MilestoneIn,
    responses(
        (status = 201, body = MilestoneOut, description = "The new milestone"),
        (status = 400, body = ErrorBody, description = "`Give the milestone a title`, `Keep the title to 120 characters`, `A range needs a unit`, `Percent milestones go up to 100`, `That pick is closed`, `Chapter range is backwards`, `meeting_at must be a date and time`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`")
    )
)]
pub async fn create_milestone(
    ClubAdmin(member): ClubAdmin,
    ApiPath(path): ApiPath<(String, i64)>,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<MilestoneIn>,
) -> ApiResult<(StatusCode, Json<MilestoneOut>)> {
    let (_, pick_id) = path;
    let (zone, _) = club_zone(&member.club.timezone);
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    let (work_id, ended) = club_pick(&mut tx, member.club.id, pick_id).await?;
    if ended {
        return Err(AppError::bad_request(PICK_CLOSED));
    }
    if is_backwards(body.range_from, body.range_to) {
        return Err(AppError::bad_request(RANGE_BACKWARDS));
    }
    let due_at = parse_due(body.due_at.as_deref(), &zone)?;
    let id = sqlx::query_scalar!(
        "INSERT INTO milestones
             (club_id, pick_id, title, unit, range_from, range_to, due_at, position, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7,
                 (SELECT count(*)::int4 FROM milestones WHERE pick_id = $2), $8)
         RETURNING id",
        member.club.id,
        pick_id,
        body.title,
        body.unit.map(MilestoneUnit::as_str),
        body.range_from,
        body.range_to,
        due_at,
        member.user.id
    )
    .fetch_one(&mut *tx)
    .await?;
    let mut payload = Map::new();
    payload.insert("title".into(), Value::String(body.title.clone()));
    if let Some(at) = due_at {
        payload.insert("due_at".into(), Value::String(rfc3339(at)?));
    }
    record(
        &mut tx,
        NewEvent {
            club_id: Some(member.club.id),
            actor_id: Some(member.user.id),
            kind: EventKind::MilestoneAdded,
            work_id: Some(work_id),
            pick_id: Some(pick_id),
            payload,
        },
    )
    .await?;
    let out = milestone_out(&mut tx, member.club.id, pick_id, id, &zone).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Changes the keys present (admins), also on an ended pick. The range rules apply to the
/// result. No event.
#[utoipa::path(
    patch,
    path = "/api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}",
    operation_id = "picks_update_milestone",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "A pick of the club"),
        ("milestone_id" = i64, Path, description = "A milestone of that pick")
    ),
    request_body = MilestonePatchIn,
    responses(
        (status = 200, body = MilestoneOut, description = "The milestone"),
        (status = 400, body = ErrorBody, description = "`Give the milestone a title`, `Keep the title to 120 characters`, `A range needs a unit`, `Percent milestones go up to 100`, `Chapter range is backwards`, `meeting_at must be a date and time`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`, `No milestone with that id`")
    )
)]
pub async fn update_milestone(
    ClubAdmin(member): ClubAdmin,
    ApiPath(path): ApiPath<(String, i64, i64)>,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<MilestonePatchIn>,
) -> ApiResult<Json<MilestoneOut>> {
    let (_, pick_id, milestone_id) = path;
    let (zone, _) = club_zone(&member.club.timezone);
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    club_pick(&mut tx, member.club.id, pick_id).await?;
    let row = sqlx::query!(
        r#"SELECT title, unit AS "unit: MilestoneUnit", range_from, range_to, due_at
           FROM milestones WHERE club_id = $1 AND pick_id = $2 AND id = $3"#,
        member.club.id,
        pick_id,
        milestone_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(NO_MILESTONE))?;
    let title = match body.title {
        Patch::Value(title) => title,
        Patch::Missing | Patch::Null => row.title,
    };
    let unit = body.unit.into_option().unwrap_or(row.unit);
    let range_from = body.range_from.into_option().unwrap_or(row.range_from);
    let range_to = body.range_to.into_option().unwrap_or(row.range_to);
    let shape = shape_errors(unit, range_from, range_to);
    if !shape.is_empty() {
        return Err(AppError::bad_request(shape.join("; ")));
    }
    if is_backwards(range_from, range_to) {
        return Err(AppError::bad_request(RANGE_BACKWARDS));
    }
    let due_at = match body.due_at.as_option() {
        None => row.due_at,
        Some(value) => parse_due(value.map(String::as_str), &zone)?,
    };
    sqlx::query!(
        "UPDATE milestones
         SET title = $2, unit = $3, range_from = $4, range_to = $5, due_at = $6
         WHERE id = $1",
        milestone_id,
        title,
        unit.map(MilestoneUnit::as_str),
        range_from,
        range_to,
        due_at
    )
    .execute(&mut *tx)
    .await?;
    let out = milestone_out(&mut tx, member.club.id, pick_id, milestone_id, &zone).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Deletes a milestone (admins), also on an ended pick. Posts tied to it stay, without the
/// milestone; the rest of the schedule is renumbered densely.
#[utoipa::path(
    delete,
    path = "/api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}",
    operation_id = "picks_delete_milestone",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "A pick of the club"),
        ("milestone_id" = i64, Path, description = "A milestone of that pick")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, body = ErrorBody, description = "An id that is not a number"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`, `No milestone with that id`")
    )
)]
pub async fn delete_milestone(
    ClubAdmin(member): ClubAdmin,
    ApiPath(path): ApiPath<(String, i64, i64)>,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let (_, pick_id, milestone_id) = path;
    let mut tx = state.db.begin().await?;
    lock_as_admin(&mut tx, member.club.id, member.user.id).await?;
    club_pick(&mut tx, member.club.id, pick_id).await?;
    // posts_milestone_fk detaches the posts (ON DELETE SET NULL (milestone_id)).
    let deleted = sqlx::query!(
        "DELETE FROM milestones WHERE club_id = $1 AND pick_id = $2 AND id = $3",
        member.club.id,
        pick_id,
        milestone_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(AppError::not_found(NO_MILESTONE));
    }
    sqlx::query!(
        "UPDATE milestones m SET position = r.rn - 1
         FROM (SELECT id, (row_number() OVER (ORDER BY position, id))::int4 AS rn
               FROM milestones WHERE pick_id = $1) r
         WHERE m.id = r.id AND m.position <> r.rn - 1",
        pick_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_shapes() {
        use MilestoneUnit::{Chapter, Percent};
        assert!(shape_errors(None, None, None).is_empty());
        assert_eq!(shape_errors(None, Some(1), None), [RANGE_NEEDS_UNIT]);
        assert_eq!(shape_errors(None, None, Some(1)), [RANGE_NEEDS_UNIT]);
        assert!(shape_errors(Some(Chapter), None, None).is_empty());
        assert!(shape_errors(Some(Chapter), Some(1), Some(400)).is_empty());
        assert!(shape_errors(Some(Percent), Some(0), Some(100)).is_empty());
        assert_eq!(
            shape_errors(Some(Percent), None, Some(101)),
            [PERCENT_TOO_HIGH]
        );
        assert_eq!(
            shape_errors(Some(Percent), Some(150), None),
            [PERCENT_TOO_HIGH]
        );
        assert!(is_backwards(Some(9), Some(2)));
        assert!(!is_backwards(Some(2), Some(2)));
        assert!(!is_backwards(None, Some(2)));
        assert!(!is_backwards(Some(9), None));
    }
}
