//! Edit, delete and react on `/api/posts/{post_id}` (endpoint-map §8, docs/phase1/spec/diary.md
//! §1.3–§1.5, R2, R3, R6–R8 and R12, conventions §6.3).
//!
//! Each starts with `visibility::post_access`: a missing, tombstoned or invisible post (another
//! club's post, a stranger's entry, someone else's `only_me` entry) is 404 `No entry with that
//! id`, so no answer reveals or returns a body the caller may not see.
//!
//! - **Edit:** the author only (403 otherwise); `updated_at` moves on every edit.
//! - **Delete:** the author, or an admin or the owner of **the post's own club** on a club post
//!   (moderation). The row is locked first; a post with replies becomes a tombstone, any other
//!   is deleted. Its reactions go either way.
//! - **React:** a toggle, atomic under races (`DELETE … RETURNING`, then `INSERT … ON CONFLICT
//!   DO NOTHING`). Turning a reaction on records `reaction` for a club post. A club post's club
//!   row and then the post's work are locked before the post (`write::access_in_lock_order`),
//!   so a reaction waits for a merge of the work instead of deadlocking with it, and its event
//!   names the work the post is on after that merge.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use super::out::{PostOut, post_out};
use super::write::{access_in_lock_order, visibility_text};
use super::{
    CANNOT_BE_PRIVATE, DELETE_OWN, EDIT_OWN, NOTHING_TO_UPDATE, PICK_A_REACTION, REACTIONS,
    check_spoiler, checked_body, spoiler,
};
use crate::AppState;
use crate::activity::{EventKind, NewEvent, record};
use crate::auth::extract::CurrentUser;
use crate::clubs::extract::Role;
use crate::dto::Visibility;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, Patch, Validate, Validator};
use crate::text::py_strip;
use crate::visibility::post_access;

/// Changes to one of the caller's posts. `body` and `visibility` treat null as absent;
/// `spoiler_upto` null clears the flag.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PostPatchIn {
    /// Stripped, 1 to 1000 characters.
    #[serde(default)]
    pub body: Option<String>,
    /// 0 to 100; null clears the spoiler flag.
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub spoiler_upto: Patch<i64>,
    /// Who sees a diary entry; only the root of a diary entry can be `only_me`.
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

impl Validate for PostPatchIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(body) = &self.body {
            self.body = Some(checked_body(body, v));
        }
        if let Patch::Value(value) = self.spoiler_upto {
            check_spoiler(Some(value), v);
        }
    }
}

impl PostPatchIn {
    fn is_empty(&self) -> bool {
        self.body.is_none() && self.spoiler_upto.is_missing() && self.visibility.is_none()
    }
}

/// A reaction to toggle.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReactionIn {
    /// One of ❤️ 👍 😂 😮 🔥 📚 (surrounding whitespace is ignored).
    pub emoji: String,
}

impl Validate for ReactionIn {
    fn validate(&mut self, v: &mut Validator) {
        self.emoji = py_strip(&self.emoji).to_owned();
        v.check(REACTIONS.contains(&self.emoji.as_str()), PICK_A_REACTION);
    }
}

/// Edits one of the caller's posts.
#[utoipa::path(
    patch,
    path = "/api/posts/{post_id}",
    operation_id = "posts_update",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("post_id" = i64, Path, description = "The post")),
    request_body = PostPatchIn,
    responses(
        (status = 200, body = PostOut, description = "The edited post, with its replies"),
        (status = 400, body = ErrorBody, description = "Validation, a non-numeric id, `Nothing to update`, `Replies and club posts cannot be private`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`You can only edit your own entries`"),
        (status = 404, body = ErrorBody, description = "`No entry with that id` (also a tombstone or a post the caller cannot see)")
    )
)]
pub async fn update_post(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(post_id): ApiPath<i64>,
    ApiJson(body): ApiJson<PostPatchIn>,
) -> ApiResult<Json<PostOut>> {
    let mut tx = state.db.begin().await?;
    let access = post_access(&mut tx, user.id, post_id, false).await?;
    if access.author_id != Some(user.id) {
        return Err(AppError::forbidden(EDIT_OWN));
    }
    if body.is_empty() {
        return Err(AppError::bad_request(NOTHING_TO_UPDATE));
    }
    let diary_root = access.club_id.is_none() && access.parent_id.is_none();
    if body.visibility == Some(Visibility::OnlyMe) && !diary_root {
        return Err(AppError::bad_request(CANNOT_BE_PRIVATE));
    }
    let (spoiler_sent, spoiler_upto) = match body.spoiler_upto {
        Patch::Missing => (false, None),
        Patch::Null => (true, None),
        Patch::Value(value) => (true, spoiler(Some(value))),
    };
    let updated = sqlx::query_scalar!(
        "UPDATE posts
         SET body = coalesce($3, body),
             spoiler_upto = CASE WHEN $4 THEN $5 ELSE spoiler_upto END,
             visibility = coalesce($6, visibility),
             updated_at = now()
         WHERE id = $1 AND author_id = $2 AND deleted_at IS NULL
         RETURNING id",
        post_id,
        user.id,
        body.body,
        spoiler_sent,
        spoiler_upto,
        body.visibility.map(visibility_text)
    )
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Err(AppError::not_found(msg::NO_ENTRY));
    }
    let post = post_out(&mut tx, &user, post_id).await?;
    tx.commit().await?;
    Ok(Json(post))
}

/// Deletes a post: the author's own, or a club post as an admin or the owner of its club. A
/// post with replies stays as a tombstone.
#[utoipa::path(
    delete,
    path = "/api/posts/{post_id}",
    operation_id = "posts_delete",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("post_id" = i64, Path, description = "The post")),
    responses(
        (status = 204, description = "Deleted, or a tombstone when it has replies"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`You can only delete your own entries`"),
        (status = 404, body = ErrorBody, description = "`No entry with that id` (also a tombstone or a post the caller cannot see)")
    )
)]
pub async fn delete_post(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(post_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let access = post_access(&mut tx, user.id, post_id, false).await?;
    let mine = access.author_id == Some(user.id);
    let moderator =
        access.club_id.is_some() && access.club_role.is_some_and(|role| role >= Role::Admin);
    if !mine && !moderator {
        return Err(AppError::forbidden(DELETE_OWN));
    }
    // The lock waits for a reply or a reaction in flight (they hold the post FOR KEY SHARE),
    // so the reply check below sees every reply committed before the delete.
    let live = sqlx::query_scalar!(
        r#"SELECT deleted_at IS NULL AS "live!" FROM posts WHERE id = $1 FOR UPDATE"#,
        post_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if live != Some(true) {
        return Err(AppError::not_found(msg::NO_ENTRY));
    }
    sqlx::query!("DELETE FROM post_reactions WHERE post_id = $1", post_id)
        .execute(&mut *tx)
        .await?;
    let has_replies = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM posts WHERE parent_id = $1) AS "has_replies!""#,
        post_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if has_replies {
        sqlx::query!(
            "UPDATE posts SET body = '', spoiler_upto = NULL, deleted_at = now() WHERE id = $1",
            post_id
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!("DELETE FROM posts WHERE id = $1", post_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Toggles one of the caller's reactions on a post they can see.
#[utoipa::path(
    post,
    path = "/api/posts/{post_id}/reactions",
    operation_id = "posts_react",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("post_id" = i64, Path, description = "The post")),
    request_body = ReactionIn,
    responses(
        (status = 200, body = PostOut, description = "The post with its reactions (and its replies)"),
        (status = 400, body = ErrorBody, description = "Validation, a non-numeric id, `Pick one of the club reactions`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No entry with that id` (also a tombstone or a post the caller cannot see)")
    )
)]
pub async fn react(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(post_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReactionIn>,
) -> ApiResult<Json<PostOut>> {
    let mut tx = state.db.begin().await?;
    let access = access_in_lock_order(&mut tx, user.id, post_id, false).await?;
    // Held until commit: a concurrent delete waits, and a post deleted meanwhile is a 404.
    let live = sqlx::query_scalar!(
        r#"SELECT deleted_at IS NULL AS "live!" FROM posts WHERE id = $1 FOR KEY SHARE"#,
        post_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if live != Some(true) {
        return Err(AppError::not_found(msg::NO_ENTRY));
    }
    let removed = sqlx::query_scalar!(
        "DELETE FROM post_reactions WHERE post_id = $1 AND user_id = $2 AND emoji = $3
         RETURNING post_id",
        post_id,
        user.id,
        body.emoji
    )
    .fetch_optional(&mut *tx)
    .await?;
    if removed.is_none() {
        let inserted = sqlx::query_scalar!(
            "INSERT INTO post_reactions (post_id, user_id, emoji) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING
             RETURNING post_id",
            post_id,
            user.id,
            body.emoji
        )
        .fetch_optional(&mut *tx)
        .await?;
        if inserted.is_some()
            && let Some(club_id) = access.club_id
        {
            let payload = json!({
                "emoji": body.emoji,
                "post_id": post_id,
                "author_id": access.author_id,
            });
            record(
                &mut tx,
                NewEvent {
                    club_id: Some(club_id),
                    actor_id: Some(user.id),
                    kind: EventKind::Reaction,
                    work_id: Some(access.work_id),
                    pick_id: access.pick_id,
                    payload: payload.as_object().cloned().unwrap_or_default(),
                },
            )
            .await?;
        }
    }
    let post = post_out(&mut tx, &user, post_id).await?;
    tx.commit().await?;
    Ok(Json(post))
}
