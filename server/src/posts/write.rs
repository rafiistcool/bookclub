//! Writing a post, shared by the diary route and the pick routes (docs/phase1/spec/diary.md
//! §1.2, docs/phase1/spec/club.md §1.6 and §1.8, conventions §6.3).
//!
//! - A reply's parent is checked through `visibility::post_access` (an invisible parent answers
//!   like a missing one) and then locked `FOR KEY SHARE`, so a concurrent delete either sees the
//!   reply (and leaves a tombstone) or removes the parent first (and the reply is refused).
//! - **Lock order: club row, then work, then post.** For a club post the club row comes first
//!   (`FOR KEY SHARE`). A club delete and an account deletion lock the club rows before they
//!   reach the posts (conventions §5.3, §5.6); a reply or a reaction that locked the post first
//!   and then needed the club row for its foreign keys would deadlock with them. The work comes
//!   next ([`lock_work`], `FOR SHARE`): `merge_works` locks the works, then users, then the
//!   posts it moves (docs/phase1/notes/cat.md, "For SHELF, POSTS and CLUB"), so a write that
//!   held a post and then waited for a user row would deadlock with it, and a post inserted
//!   during a merge would stay on the merged-away work.
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
use crate::catalog::shared::books::require_work;
use crate::dto::{ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, msg};
use crate::visibility::{PostAccess, post_access};

/// How often a work lock follows a merge that committed while it waited. Merges are admin
/// actions, so a second round is already rare; the bound only keeps a loop finite.
pub const WORK_LOCK_TRIES: usize = 8;

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
    /// The thread's work, locked (see [`lock_work`]); the reply is written on it.
    pub work_id: i64,
    pub club_id: Option<i64>,
    pub pick_id: Option<i64>,
}

/// Locks the work a post is written on `FOR SHARE` and returns its current id (404 `No such
/// book.` when it does not exist). The lock conflicts with the `FOR NO KEY UPDATE` that
/// `merge_works` holds on both works: a write waits for a merge in flight and then follows
/// `merged_into_id` to the winner, and a merge that starts later waits for the write and then
/// moves its row. Taken after the club row and before any post or user row.
pub async fn lock_work(conn: &mut PgConnection, work_id: i64) -> ApiResult<i64> {
    let mut current = work_id;
    for _ in 0..WORK_LOCK_TRIES {
        let locked = sqlx::query_scalar!(
            "SELECT id FROM works WHERE id = $1 AND merged_into_id IS NULL FOR SHARE",
            current
        )
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = locked {
            return Ok(id);
        }
        current = require_work(conn, current).await?;
    }
    Err(AppError::internal(format!(
        "work {work_id} was merged away {WORK_LOCK_TRIES} times while a post waited for it"
    )))
}

/// Locks the row of the club `club_id` `FOR KEY SHARE` (nothing for a diary entry). A club that
/// is gone stays unlocked; the caller's next read answers for it.
async fn lock_club_row(conn: &mut PgConnection, club_id: Option<i64>) -> sqlx::Result<()> {
    if let Some(club_id) = club_id {
        sqlx::query_scalar!("SELECT id FROM clubs WHERE id = $1 FOR KEY SHARE", club_id)
            .fetch_optional(conn)
            .await?;
    }
    Ok(())
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

/// `visibility::post_access` in the lock order of writes on an existing post: the club row of
/// a club post (`FOR KEY SHARE`), then the post's work ([`lock_work`]), then the access is read
/// again. A removal, a club delete or a merge that held one of those locks may have changed the
/// answer; the access returned is the one under the locks, and its `work_id` is the locked
/// work. Callers lock the post itself next.
pub async fn access_in_lock_order(
    conn: &mut PgConnection,
    viewer: i64,
    post_id: i64,
    allow_tombstone: bool,
) -> ApiResult<PostAccess> {
    let mut access = post_access(conn, viewer, post_id, allow_tombstone).await?;
    lock_club_row(conn, access.club_id).await?;
    for _ in 0..WORK_LOCK_TRIES {
        let work_id = lock_work(conn, access.work_id).await?;
        access = post_access(conn, viewer, post_id, allow_tombstone).await?;
        // A merge that committed while the request waited moved the post to the work
        // `lock_work` followed to, and no merge moves it while that lock is held.
        if access.work_id == work_id {
            return Ok(access);
        }
    }
    Err(AppError::internal(format!(
        "post {post_id} kept moving to another work"
    )))
}

/// Checks the parent of a new reply on the thread of `work_id`, in the legacy order: missing,
/// invisible or not on this thread (another work, or `on_thread` fails) → 400 `That entry is
/// not on this book`; itself a reply → 400 `Reply to the entry itself, not to a reply`; a
/// tombstone → 400 `That entry was deleted`. A club post also needs the caller's membership of
/// **its** club. Locks in the order of the module doc: the parent's club, then the thread's
/// work (a merge in flight moves the parent with it, so the reply follows to the winner), then
/// the parent `FOR KEY SHARE` until the transaction ends.
pub async fn reply_parent(
    conn: &mut PgConnection,
    viewer: i64,
    parent_id: i64,
    work_id: i64,
    on_thread: impl FnOnce(&Parent) -> bool,
) -> ApiResult<Parent> {
    let not_here = || AppError::bad_request(NOT_ON_THIS_BOOK);
    let as_parent = |result: ApiResult<PostAccess>| match result {
        Ok(access) => Ok(access),
        Err(error) if error.status == StatusCode::NOT_FOUND => Err(not_here()),
        Err(error) => Err(error),
    };
    let first = as_parent(post_access(conn, viewer, parent_id, true).await)?;
    lock_club_row(conn, first.club_id).await?;
    let work_id = lock_work(conn, work_id).await?;
    // Read again under the locks: it sees a removal or a club delete that held the club row.
    let checked = as_parent(post_access(conn, viewer, parent_id, true).await)?;
    if checked.club_id.is_some() && checked.club_role.is_none() {
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
    if parent.work_id != work_id || !on_thread(&parent) {
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
/// The caller holds the work's lock ([`lock_work`]) and, for a milestone, the milestone's
/// (`FOR KEY SHARE`). A club post records `note_posted`; one on a pick that has ended is 400
/// `That pick is closed`.
pub async fn insert_post(
    conn: &mut PgConnection,
    author: &CurrentUser,
    new: NewPost,
) -> ApiResult<i64> {
    let (progress_at, status_at) = my_position(conn, author.id, new.work_id).await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO posts (work_id, author_id, parent_id, club_id, pick_id, milestone_id,
                            visibility, body, spoiler_upto, progress_at, status_at)
         SELECT $1::int8, $2::int8, $3::int8, $4::int8, $5::int8, $6::int8,
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
