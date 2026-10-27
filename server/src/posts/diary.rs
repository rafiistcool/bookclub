//! A work's thread, writing a diary entry or a reply, and the diary feed (endpoint-map §8,
//! docs/phase1/spec/diary.md §1.1, §1.2, §1.6, §2.2, R1, R4 and R9).
//!
//! - The work thread holds the diary entries on the work the viewer may see and the club posts
//!   on it from the viewer's clubs, roots and replies ordered `created_at, id`, tombstones
//!   included as anchors.
//! - The feed pages by id (`id < before`, newest first) and leaves tombstones out (their
//!   replies stay). With `?club=` it shows that club's posts and the diary entries of its
//!   current members.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::{IntoParams, ToSchema};

use super::out::{PostOut, my_position, positions, post_out, post_outs};
use super::write::{NewPost, insert_post, reply_parent};
use super::{CANNOT_BE_PRIVATE, check_spoiler, checked_body, spoiler};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::catalog::shared::work_ref::{WorkRef, resolve_local};
use crate::clubs::extract::member_by_slug;
use crate::dto::{BookOut, ClubRef, ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, ApiQuery, Validate, Validator};

/// One work's thread.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiaryOut {
    /// The work's current id (a merged work answers with the work it was merged into).
    pub work_id: i64,
    /// The viewer's progress in the work (100 when finished), for the spoiler shield.
    pub my_progress: Option<i32>,
    pub my_status: Option<ShelfStatus>,
    /// Diary entries and the viewer's clubs' posts on the work, oldest first, each with its
    /// visible replies.
    pub items: Vec<PostOut>,
    /// The viewer's time zone.
    pub timezone: String,
}

/// A new diary entry, or a reply to an entry or to a club post.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DiaryEntryIn {
    /// Stripped, 1 to 1000 characters.
    pub body: String,
    /// 0 to 100: the percentage of the book the post is safe up to; null for no spoiler flag.
    #[serde(default)]
    pub spoiler_upto: Option<i64>,
    /// The entry to reply to. A reply takes its parent's scope: a reply to a club post is a
    /// club post of that club.
    #[serde(default)]
    pub parent_id: Option<i64>,
    /// Who sees a new entry; default `audience`. A reply is always `audience`.
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

impl Validate for DiaryEntryIn {
    fn validate(&mut self, v: &mut Validator) {
        self.body = checked_body(&self.body, v);
        check_spoiler(self.spoiler_upto, v);
        if self.parent_id.is_some() && self.visibility == Some(Visibility::OnlyMe) {
            v.fail(CANNOT_BE_PRIVATE);
        }
    }
}

/// `?before=&limit=&club=` of the diary feed.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FeedQuery {
    /// Only posts with a smaller id (the last item's `entry.id`); at least 1.
    #[serde(default)]
    pub before: Option<i64>,
    /// 1 to 50, default 10.
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// A club's slug: that club's posts and its current members' diary entries. A club the
    /// caller is not in is 404 `No such club`. Empty means no filter.
    #[serde(default)]
    pub club: Option<String>,
}

fn default_limit() -> i64 {
    10
}

impl Validate for FeedQuery {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(self.before, 1, i64::MAX);
        v.range_i64(Some(self.limit), 1, 50);
    }
}

/// One post of the feed, with its work and the viewer's position in it.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiaryFeedItemOut {
    /// The post, without replies.
    pub entry: PostOut,
    /// The work, without an edition context.
    pub book: BookOut,
    /// For a reply: the parent's author, when the viewer may see them; else null.
    pub parent_author: Option<String>,
    pub my_progress: Option<i32>,
    pub my_status: Option<ShelfStatus>,
    /// The club of a club post.
    pub club: Option<ClubRef>,
}

/// A page of the diary feed, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiaryFeedOut {
    pub items: Vec<DiaryFeedItemOut>,
    pub has_more: bool,
    /// The viewer's time zone.
    pub timezone: String,
}

/// The local work a `{work_id}` names: a numeric id, `OL…W` or a legacy key. 404 `No such
/// book.` for anything else and for a work the catalog does not have.
async fn resolve_work(conn: &mut PgConnection, raw: &str) -> ApiResult<i64> {
    let not_found = || AppError::not_found(msg::NO_SUCH_BOOK);
    let work_ref = WorkRef::parse(raw).ok_or_else(not_found)?;
    resolve_local(conn, &work_ref).await?.ok_or_else(not_found)
}

/// The diary entries on a work the caller may see, plus the posts of the caller's clubs on it.
#[utoipa::path(
    get,
    path = "/api/books/works/{work_id}/posts",
    operation_id = "posts_work_thread",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W` or a legacy key")),
    responses(
        (status = 200, body = DiaryOut, description = "The thread"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn work_thread(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(work): ApiPath<String>,
) -> ApiResult<Json<DiaryOut>> {
    let mut conn = state.db.acquire().await?;
    let work_id = resolve_work(&mut conn, &work).await?;
    let roots = sqlx::query_scalar!(
        r#"SELECT p.id
           FROM posts p
           JOIN bc_visible_posts($1) AS v ON v.id = p.id
           WHERE p.work_id = $2 AND p.parent_id IS NULL
           ORDER BY p.created_at, p.id"#,
        user.id,
        work_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let items = post_outs(&mut conn, &user, &roots, true).await?;
    let (my_progress, my_status) = my_position(&mut conn, user.id, work_id).await?;
    Ok(Json(DiaryOut {
        work_id,
        my_progress,
        my_status,
        items,
        timezone: user.timezone,
    }))
}

/// Writes a diary entry, or a reply. A reply to a club post is a club post (it needs
/// membership of that club and records `note_posted`); a diary entry records no event.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/posts",
    operation_id = "posts_write_entry",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W` or a legacy key")),
    request_body = DiaryEntryIn,
    responses(
        (status = 201, body = PostOut, description = "The new post"),
        (status = 400, body = ErrorBody, description = "Validation, `Write a short take, quote, or meeting note`, `Keep it to 1000 characters`, `Replies and club posts cannot be private`, `That entry is not on this book`, `Reply to the entry itself, not to a reply`, `That entry was deleted`, `That pick is closed`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn write_entry(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(work): ApiPath<String>,
    ApiJson(body): ApiJson<DiaryEntryIn>,
) -> ApiResult<(StatusCode, Json<PostOut>)> {
    let mut tx = state.db.begin().await?;
    let work_id = resolve_work(&mut tx, &work).await?;
    let new = match body.parent_id {
        None => NewPost {
            work_id,
            parent_id: None,
            club_id: None,
            pick_id: None,
            milestone_id: None,
            visibility: body.visibility.unwrap_or(Visibility::Audience),
            body: body.body,
            spoiler_upto: spoiler(body.spoiler_upto),
        },
        Some(parent_id) => {
            let parent =
                reply_parent(&mut tx, user.id, parent_id, |p| p.work_id == work_id).await?;
            NewPost {
                work_id,
                parent_id: Some(parent_id),
                club_id: parent.club_id,
                pick_id: parent.pick_id,
                milestone_id: None,
                visibility: Visibility::Audience,
                body: body.body,
                spoiler_upto: spoiler(body.spoiler_upto),
            }
        }
    };
    let id = insert_post(&mut tx, &user, new).await?;
    let post = post_out(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(post)))
}

/// The newest posts the caller may see across every work: their own and their audience's diary
/// entries and their clubs' posts (`?club=`: one club's).
#[utoipa::path(
    get,
    path = "/api/diary",
    operation_id = "posts_feed",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(FeedQuery),
    responses(
        (status = 200, body = DiaryFeedOut, description = "A page of the feed"),
        (status = 400, body = ErrorBody, description = "`before` below 1, `limit` outside 1..50"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn feed(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<FeedQuery>,
) -> ApiResult<Json<DiaryFeedOut>> {
    let mut conn = state.db.acquire().await?;
    let club_id = match query.club.as_deref().filter(|slug| !slug.is_empty()) {
        Some(slug) => Some(member_by_slug(&mut conn, &user, slug).await?.club.id),
        None => None,
    };
    let rows = sqlx::query!(
        r#"SELECT p.id, p.work_id
           FROM posts p
           JOIN bc_visible_posts($1) AS v ON v.id = p.id
           WHERE p.deleted_at IS NULL
             AND ($2::int8 IS NULL OR p.id < $2)
             AND ($3::int8 IS NULL
                  OR p.club_id = $3
                  OR (p.club_id IS NULL AND EXISTS (
                          SELECT 1 FROM club_memberships m
                          WHERE m.club_id = $3 AND m.user_id = p.author_id)))
           ORDER BY p.id DESC
           LIMIT $4"#,
        user.id,
        query.before,
        club_id,
        query.limit + 1
    )
    .fetch_all(&mut *conn)
    .await?;
    let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
    let has_more = rows.len() > limit;
    let rows = &rows[..rows.len().min(limit)];
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let work_ids: Vec<i64> = rows.iter().map(|row| row.work_id).collect();
    let entries = post_outs(&mut conn, &user, &ids, false).await?;
    let refs: Vec<BookRef> = work_ids.iter().copied().map(BookRef::work).collect();
    let books = book_outs(&mut conn, &refs).await?;
    let mine = positions(&mut conn, user.id, &work_ids).await?;
    let parent_authors = parent_authors(&mut conn, user.id, &ids).await?;
    let work_of: HashMap<i64, i64> = rows.iter().map(|row| (row.id, row.work_id)).collect();
    let items = entries
        .into_iter()
        .filter_map(|entry| {
            let work_id = *work_of.get(&entry.id)?;
            let book = books.get(&BookRef::work(work_id))?.clone();
            let (my_progress, my_status) = mine.get(&work_id).copied().unwrap_or((None, None));
            Some(DiaryFeedItemOut {
                parent_author: parent_authors.get(&entry.id).cloned(),
                club: entry.club.clone(),
                entry,
                book,
                my_progress,
                my_status,
            })
        })
        .collect();
    Ok(Json(DiaryFeedOut {
        items,
        has_more,
        timezone: user.timezone,
    }))
}

/// For each reply among `ids`: its parent's author, when the viewer may see that user
/// (`bc_visible_owners`).
async fn parent_authors(
    conn: &mut PgConnection,
    viewer: i64,
    ids: &[i64],
) -> sqlx::Result<HashMap<i64, String>> {
    let rows = sqlx::query!(
        r#"SELECT p.id, u.username
           FROM posts p
           JOIN posts parent ON parent.id = p.parent_id
           JOIN users u ON u.id = parent.author_id
           JOIN bc_visible_owners($1) AS o ON o.user_id = parent.author_id
           WHERE p.id = ANY($2)"#,
        viewer,
        ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.username)).collect())
}
