//! A club pick's discussion (endpoint-map §8, docs/phase1/spec/club.md §1.5–§1.8): the club
//! posts on one pick, oldest first, with their replies. Any member may read and post; posts
//! on a pick that has ended stay readable but take no new posts.
//!
//! The club comes only from the `ClubMember` extractor, and the pick and the milestone are
//! looked up `WHERE club_id = $club AND id = $id`, so another club's ids answer like missing
//! ones. A write locks the club row `FOR KEY SHARE` first and checks the membership again
//! under that lock (`write::lock_club_member`), then the pick's work (`write::lock_work`),
//! then the milestone (`FOR KEY SHARE`), then the parent of a reply.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::out::{PostOut, my_position, post_out, post_outs};
use super::write::{
    NewPost, WORK_LOCK_TRIES, insert_post, lock_club_member, lock_work, reply_parent,
};
use super::{
    MILESTONE_NOT_ON_PICK, NO_PICK_WITH_ID, NO_PICK_YET, PICK_CLOSED, check_spoiler, checked_body,
    spoiler,
};
use crate::AppState;
use crate::clubs::extract::ClubMember;
use crate::dto::{ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};

/// One pick's discussion.
#[derive(Debug, Serialize, ToSchema)]
pub struct PickThreadOut {
    pub pick_id: i64,
    /// Whether the pick is still current (an ended pick takes no new posts).
    pub can_post: bool,
    /// The caller's progress in the pick's work (100 when finished), for the spoiler shield.
    pub my_progress: Option<i32>,
    pub my_status: Option<ShelfStatus>,
    /// The pick's posts, oldest first, each with its replies.
    pub items: Vec<PostOut>,
    /// The club's time zone.
    pub timezone: String,
}

/// A new post in a pick's discussion, or a reply to one.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PickPostIn {
    /// Stripped, 1 to 1000 characters.
    pub body: String,
    /// 0 to 100: the percentage of the book the post is safe up to; null for no spoiler flag.
    #[serde(default)]
    pub spoiler_upto: Option<i64>,
    /// A milestone of this pick the post is about.
    #[serde(default)]
    pub milestone_id: Option<i64>,
    /// A post of this pick's discussion to reply to.
    #[serde(default)]
    pub parent_id: Option<i64>,
}

impl Validate for PickPostIn {
    fn validate(&mut self, v: &mut Validator) {
        self.body = checked_body(&self.body, v);
        check_spoiler(self.spoiler_upto, v);
    }
}

/// A pick of the caller's club.
struct Pick {
    id: i64,
    work_id: i64,
    open: bool,
}

/// The club's current pick; 404 `No club pick yet`.
async fn current_pick(conn: &mut PgConnection, club_id: i64) -> ApiResult<Pick> {
    sqlx::query_as!(
        Pick,
        r#"SELECT id, work_id, true AS "open!"
           FROM club_picks
           WHERE club_id = $1 AND ended_at IS NULL"#,
        club_id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_PICK_YET))
}

/// One of the club's picks, current or ended; 404 `No club pick with that id` (also for
/// another club's pick).
async fn club_pick(conn: &mut PgConnection, club_id: i64, pick_id: i64) -> ApiResult<Pick> {
    sqlx::query_as!(
        Pick,
        r#"SELECT id, work_id, ended_at IS NULL AS "open!"
           FROM club_picks
           WHERE club_id = $1 AND id = $2"#,
        club_id,
        pick_id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_PICK_WITH_ID))
}

/// The pick's posts as the caller sees them.
async fn thread(
    conn: &mut PgConnection,
    member: &ClubMember,
    pick: Pick,
) -> ApiResult<PickThreadOut> {
    let roots = sqlx::query_scalar!(
        r#"SELECT p.id
           FROM posts p
           JOIN bc_visible_posts($1) AS v ON v.id = p.id
           WHERE p.club_id = $2 AND p.pick_id = $3 AND p.parent_id IS NULL
           ORDER BY p.created_at, p.id"#,
        member.user.id,
        member.club.id,
        pick.id
    )
    .fetch_all(&mut *conn)
    .await?;
    let items = post_outs(conn, &member.user, &roots, true).await?;
    let (my_progress, my_status) = my_position(conn, member.user.id, pick.work_id).await?;
    Ok(PickThreadOut {
        pick_id: pick.id,
        can_post: pick.open,
        my_progress,
        my_status,
        items,
        timezone: member.club.timezone.clone(),
    })
}

/// Locks the pick's work against a merge (`write::lock_work`) and reads the pick's work again
/// under that lock: a merge that committed while the request waited has re-pointed the pick to
/// the work `lock_work` followed to. A pick deleted meanwhile keeps the work read before; the
/// insert then answers `That pick is closed`.
async fn lock_pick_work(conn: &mut PgConnection, pick: &mut Pick) -> ApiResult<()> {
    for _ in 0..WORK_LOCK_TRIES {
        let locked = lock_work(conn, pick.work_id).await?;
        let current = sqlx::query_scalar!("SELECT work_id FROM club_picks WHERE id = $1", pick.id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(locked);
        pick.work_id = current;
        if current == locked {
            return Ok(());
        }
    }
    Err(AppError::internal(format!(
        "pick {} kept moving to another work",
        pick.id
    )))
}

/// Writes a post on `pick`, in the legacy order of checks: the pick has ended → 400 `That
/// pick is closed`; a milestone of another pick (or none) → 400 `That milestone is not on this
/// pick`; then the parent of a reply, which must be a post of this pick.
async fn write(
    conn: &mut PgConnection,
    member: &ClubMember,
    mut pick: Pick,
    body: PickPostIn,
) -> ApiResult<PostOut> {
    if !pick.open {
        return Err(AppError::bad_request(PICK_CLOSED));
    }
    lock_pick_work(conn, &mut pick).await?;
    if let Some(milestone_id) = body.milestone_id {
        // FOR KEY SHARE: a delete in flight is waited for and then answers as missing; a delete
        // that starts later waits for this post and then detaches it (ON DELETE SET NULL).
        sqlx::query_scalar!(
            "SELECT id FROM milestones WHERE club_id = $1 AND pick_id = $2 AND id = $3
             FOR KEY SHARE",
            member.club.id,
            pick.id,
            milestone_id
        )
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::bad_request(MILESTONE_NOT_ON_PICK))?;
    }
    if let Some(parent_id) = body.parent_id {
        reply_parent(conn, member.user.id, parent_id, pick.work_id, |parent| {
            parent.club_id == Some(member.club.id) && parent.pick_id == Some(pick.id)
        })
        .await?;
    }
    let id = insert_post(
        conn,
        &member.user,
        NewPost {
            work_id: pick.work_id,
            parent_id: body.parent_id,
            club_id: Some(member.club.id),
            pick_id: Some(pick.id),
            milestone_id: body.milestone_id,
            visibility: Visibility::Audience,
            body: body.body,
            spoiler_upto: spoiler(body.spoiler_upto),
        },
    )
    .await?;
    post_out(conn, &member.user, id).await
}

/// The current pick's discussion.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/pick/posts",
    operation_id = "posts_current_pick_thread",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = PickThreadOut, description = "The discussion"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick yet`")
    )
)]
pub async fn current_pick_thread(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<PickThreadOut>> {
    let mut conn = state.db.acquire().await?;
    let pick = current_pick(&mut conn, member.club.id).await?;
    Ok(Json(thread(&mut conn, &member, pick).await?))
}

/// Posts on the current pick. Records `note_posted`.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/pick/posts",
    operation_id = "posts_write_current_pick",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = PickPostIn,
    responses(
        (status = 201, body = PostOut, description = "The new post"),
        (status = 400, body = ErrorBody, description = "Validation, `Write a short take, quote, or meeting note`, `Keep it to 1000 characters`, `That milestone is not on this pick`, `That entry is not on this book`, `Reply to the entry itself, not to a reply`, `That entry was deleted`, `That pick is closed`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick yet`")
    )
)]
pub async fn write_current_pick(
    member: ClubMember,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PickPostIn>,
) -> ApiResult<(StatusCode, Json<PostOut>)> {
    let mut tx = state.db.begin().await?;
    lock_club_member(&mut tx, member.club.id, member.user.id).await?;
    let pick = current_pick(&mut tx, member.club.id).await?;
    let post = write(&mut tx, &member, pick, body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(post)))
}

/// Any pick's discussion, current or ended.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/picks/{pick_id}/posts",
    operation_id = "posts_pick_thread",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "One of the club's picks")
    ),
    responses(
        (status = 200, body = PickThreadOut, description = "The discussion"),
        (status = 400, body = ErrorBody, description = "A non-numeric pick id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`")
    )
)]
pub async fn pick_thread(
    member: ClubMember,
    State(state): State<AppState>,
    ApiPath(path): ApiPath<(String, i64)>,
) -> ApiResult<Json<PickThreadOut>> {
    let mut conn = state.db.acquire().await?;
    let pick = club_pick(&mut conn, member.club.id, path.1).await?;
    Ok(Json(thread(&mut conn, &member, pick).await?))
}

/// Posts on one of the club's picks; an ended pick is 400 `That pick is closed`. Records
/// `note_posted`.
#[utoipa::path(
    post,
    path = "/api/clubs/{slug}/picks/{pick_id}/posts",
    operation_id = "posts_write_pick",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        ("pick_id" = i64, Path, description = "One of the club's picks")
    ),
    request_body = PickPostIn,
    responses(
        (status = 201, body = PostOut, description = "The new post"),
        (status = 400, body = ErrorBody, description = "Validation, a non-numeric pick id, `Write a short take, quote, or meeting note`, `Keep it to 1000 characters`, `That pick is closed`, `That milestone is not on this pick`, `That entry is not on this book`, `Reply to the entry itself, not to a reply`, `That entry was deleted`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No club pick with that id`")
    )
)]
pub async fn write_pick(
    member: ClubMember,
    State(state): State<AppState>,
    ApiPath(path): ApiPath<(String, i64)>,
    ApiJson(body): ApiJson<PickPostIn>,
) -> ApiResult<(StatusCode, Json<PostOut>)> {
    let mut tx = state.db.begin().await?;
    lock_club_member(&mut tx, member.club.id, member.user.id).await?;
    let pick = club_pick(&mut tx, member.club.id, path.1).await?;
    let post = write(&mut tx, &member, pick, body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(post)))
}
