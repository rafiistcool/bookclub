//! Writing a post, shared by the diary route and the pick routes (docs/phase1/spec/diary.md
//! §1.2, docs/phase1/spec/club.md §1.6 and §1.8, conventions §6.3).
//!
//! - A reply's parent is checked through `visibility::post_access` (an invisible parent answers
//!   like a missing one) and then locked `FOR KEY SHARE`, so a concurrent delete either sees the
//!   reply (and leaves a tombstone) or removes the parent first (and the reply is refused).
//! - **Lock order:** for a club post the club row comes first (`FOR KEY SHARE`, see
//!   [`access_in_lock_order`]), then the post. A club delete and an account deletion lock the
//!   club rows before they reach the posts (conventions §5.3, §5.6); a reply or a reaction that
//!   locked the post first and then needed the club row for its foreign keys would deadlock with
//!   them.
//! - The insert itself refuses a club post on a pick that has ended (`That pick is closed`), so
//!   a pick that closes while the request runs cannot get a new post.
//! - Every new club post records `note_posted` `{post_id}`; diary entries record nothing.

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgConnection;

use super::out::my_position;
use super::{ENTRY_DELETED, NOT_ON_THIS_BOOK, PICK_CLOSED, REPLY_TO_ENTRY};
use crate::activity::{EventKind, NewEvent, record};
use crate::auth::extract::CurrentUser;
use crate::dto::{ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, msg};
use crate::visibility::{PostAccess, post_access};

/// A post to insert; the caller has checked the work, the pick, the milestone and the parent.
#[derive(Debug, Clone)]
pub struct NewPost {
    pub work_id: i64,
    pub parent_id: Option<i64>,
    /// Set for club posts, together with `pick_id`.
    pub club_id: Option<i64>,
    pub pick_id: Option<i64>,
    pub milestone_id: Option<i64>,
    pub visibility: Visibility,
    pub body: String,
    pub spoiler_upto: Option<i32>,
}

/// The parent of a reply, locked for the rest of the transaction.
#[derive(Debug, Clone)]
pub struct Parent {
    pub work_id: i64,
    pub club_id: Option<i64>,
    pub pick_id: Option<i64>,
}

/// Locks the club row `FOR KEY SHARE` for a new club post and checks again, under that lock,
/// that the caller is still a member: 404 `No such club` when the club was deleted or the caller
/// removed while the request waited. Without it, a post racing a club delete would fail its
/// foreign key (a 500), and one racing a removal would land after it.
pub async fn lock_club_member(
    conn: &mut PgConnection,
    club_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    let gone = || AppError::not_found(msg::NO_SUCH_CLUB);
    sqlx::query_scalar!("SELECT id FROM clubs WHERE id = $1 FOR KEY SHARE", club_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(gone)?;
    // A statement of its own: it sees a removal committed while the lock was awaited (a join in
    // the locking statement would be re-checked against its old snapshot).
    let member = sqlx::query_scalar!(
        r#"SELECT bc_is_member($2, $1) AS "member!""#,
        club_id,
        user_id
    )
    .fetch_one(conn)
    .await?;
    if member { Ok(()) } else { Err(gone()) }
}

/// `visibility::post_access` in the lock order of club writes: for a club post, the club row is
/// locked `FOR KEY SHARE` first and the access is read again after that lock (a removal or a
/// club delete that held it may have changed the answer). Callers lock the post itself next.
pub async fn access_in_lock_order(
    conn: &mut PgConnection,
    viewer: i64,
    post_id: i64,
    allow_tombstone: bool,
) -> ApiResult<PostAccess> {
    let access = post_access(conn, viewer, post_id, allow_tombstone).await?;
    let Some(club_id) = access.club_id else {
        return Ok(access);
    };
    sqlx::query_scalar!("SELECT id FROM clubs WHERE id = $1 FOR KEY SHARE", club_id)
        .fetch_optional(&mut *conn)
        .await?;
    post_access(conn, viewer, post_id, allow_tombstone).await
}

/// Checks the parent of a new reply, in the legacy order: missing, invisible or not on this
/// thread (`on_thread`) → 400 `That entry is not on this book`; itself a reply → 400 `Reply to
/// the entry itself, not to a reply`; a tombstone → 400 `That entry was deleted`. A club post
/// also needs the caller's membership of **its** club. The parent row stays locked
/// `FOR KEY SHARE` until the transaction ends.
pub async fn reply_parent(
    conn: &mut PgConnection,
    viewer: i64,
    parent_id: i64,
    on_thread: impl FnOnce(&Parent) -> bool,
) -> ApiResult<Parent> {
    let not_here = || AppError::bad_request(NOT_ON_THIS_BOOK);
    let access = match access_in_lock_order(conn, viewer, parent_id, true).await {
        Ok(access) => access,
        Err(error) if error.status == StatusCode::NOT_FOUND => return Err(not_here()),
        Err(error) => return Err(error),
    };
    if access.club_id.is_some() && access.club_role.is_none() {
        return Err(not_here());
    }
    let row = sqlx::query!(
        r#"SELECT work_id, parent_id, club_id, pick_id, deleted_at IS NOT NULL AS "deleted!"
           FROM posts
           WHERE id = $1
           FOR KEY SHARE"#,
        parent_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(not_here)?;
    let parent = Parent {
        work_id: row.work_id,
        club_id: row.club_id,
        pick_id: row.pick_id,
    };
    if !on_thread(&parent) {
        return Err(not_here());
    }
    if row.parent_id.is_some() {
        return Err(AppError::bad_request(REPLY_TO_ENTRY));
    }
    if row.deleted {
        return Err(AppError::bad_request(ENTRY_DELETED));
    }
    Ok(parent)
}

/// The stored text of a status.
fn status_text(status: ShelfStatus) -> &'static str {
    match status {
        ShelfStatus::WantToRead => "want_to_read",
        ShelfStatus::CurrentlyReading => "currently_reading",
        ShelfStatus::Finished => "finished",
        ShelfStatus::DidNotFinish => "did_not_finish",
    }
}

/// The stored text of a visibility.
pub fn visibility_text(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Audience => "audience",
        Visibility::OnlyMe => "only_me",
    }
}

/// Inserts the post with a snapshot of the author's position in the work and returns its id.
/// A club post records `note_posted`; one on a pick that has ended is 400 `That pick is
/// closed`. A milestone that vanished meanwhile is dropped (as its delete would have done).
pub async fn insert_post(
    conn: &mut PgConnection,
    author: &CurrentUser,
    new: NewPost,
) -> ApiResult<i64> {
    let (progress_at, status_at) = my_position(conn, author.id, new.work_id).await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO posts (work_id, author_id, parent_id, club_id, pick_id, milestone_id,
                            visibility, body, spoiler_upto, progress_at, status_at)
         SELECT $1::int8, $2::int8, $3::int8, $4::int8, $5::int8,
                (SELECT ms.id FROM milestones ms
                 WHERE ms.id = $6 AND ms.pick_id = $5 AND ms.club_id = $4),
                $7::text, $8::text, $9::int4, $10::int4, $11::text
         WHERE $4::int8 IS NULL
            OR EXISTS (SELECT 1 FROM club_picks cp
                       WHERE cp.club_id = $4 AND cp.id = $5 AND cp.ended_at IS NULL)
         RETURNING id",
        new.work_id,
        author.id,
        new.parent_id,
        new.club_id,
        new.pick_id,
        new.milestone_id,
        visibility_text(new.visibility),
        new.body,
        new.spoiler_upto,
        progress_at,
        status_at.map(status_text)
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::bad_request(PICK_CLOSED))?;
    if let Some(club_id) = new.club_id {
        let payload = json!({ "post_id": id });
        record(
            conn,
            NewEvent {
                club_id: Some(club_id),
                actor_id: Some(author.id),
                kind: EventKind::NotePosted,
                work_id: Some(new.work_id),
                pick_id: new.pick_id,
                payload: payload.as_object().cloned().unwrap_or_default(),
            },
        )
        .await?;
    }
    Ok(id)
}
