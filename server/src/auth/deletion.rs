//! `DELETE /api/auth/me`: account deletion (endpoint-map §2, docs/phase1/conventions.md §5.6).

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::SET_COOKIE;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::account::verify_current_password;
use super::extract::CurrentUser;
use super::session;
use crate::AppState;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate};

/// 409: the account owns a club that has other members.
pub const TRANSFER_CLUBS_FIRST: &str = "Transfer or delete your clubs first";

/// The password, to confirm the deletion.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AccountDeleteIn {
    pub password: String,
}

impl Validate for AccountDeleteIn {}

/// Deletes the signed-in account and clears the cookie.
///
/// - Clubs the user owns alone are deleted; owning a club with other members is 409.
/// - Posts that others replied to become anonymous tombstones; every other post is deleted.
/// - Every avatar the user uploaded is deleted with its bytes; cover photos stay as catalog
///   contributions, without the uploader.
/// - Personal data, sessions, memberships and the user's activity events go with the account;
///   picks, nominations, invites and catalog rows keep existing without the name.
///
/// A wrong password counts as a login failure (429 after ten).
#[utoipa::path(
    delete,
    path = "/api/auth/me",
    operation_id = "auth_delete_me",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = AccountDeleteIn,
    responses(
        (status = 204, description = "Deleted; `Set-Cookie` clears the session cookie"),
        (status = 400, body = ErrorBody, description = "`Current password is wrong`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 409, body = ErrorBody, description = "`Transfer or delete your clubs first`"),
        (status = 429, body = ErrorBody, description = "`Too many attempts. Wait a few minutes.`")
    )
)]
pub async fn delete_account(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<AccountDeleteIn>,
) -> ApiResult<Response> {
    let verified_hash = verify_current_password(&state, user.id, &body.password).await?;
    let mut tx = state.db.begin().await?;
    let prefixes = delete_user(&mut tx, user.id, &verified_hash).await?;
    tx.commit().await.map_err(|error| {
        // A membership write made the user owner of another club meanwhile: the deferred
        // `clubs_have_owner` check refuses to leave that club without an owner.
        if is_owner_check(&error) {
            AppError::conflict(TRANSFER_CLUBS_FIRST)
        } else {
            error.into()
        }
    })?;
    for prefix in prefixes {
        if let Err(error) = state.media.delete_prefix(&prefix).await {
            tracing::warn!(error = %format!("{error:#}"), prefix, "deleting avatar bytes failed");
        }
    }
    let cookie = session::clear_cookie(&state.config);
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response())
}

/// Deletes the account in the caller's transaction (§5.6) and returns the storage prefixes of
/// the deleted avatar rows, whose bytes the caller deletes after commit.
///
/// `verified_hash` is the hash the password was checked against: a password change or reset
/// that committed since then wins (400 `Current password is wrong`).
pub async fn delete_user(
    conn: &mut PgConnection,
    user_id: i64,
    verified_hash: &str,
) -> ApiResult<Vec<String>> {
    // Locking the account first makes every concurrent write that references it (a new post,
    // reaction, shelf entry) wait and then fail, instead of slipping in after the deletes below.
    let current = sqlx::query_scalar!(
        "SELECT password_hash FROM users WHERE id = $1 FOR UPDATE",
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(AppError::unauthorized)?;
    if current != verified_hash {
        return Err(AppError::bad_request(msg::CURRENT_PASSWORD_WRONG));
    }
    delete_owned_clubs(conn, user_id).await?;
    delete_posts(conn, user_id).await?;
    let prefixes = sqlx::query_scalar!(
        "DELETE FROM media
         WHERE kind = 'avatar'
           AND (uploaded_by = $1 OR id = (SELECT avatar_media_id FROM users WHERE id = $1))
         RETURNING storage_key",
        user_id
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .flatten()
    .collect();
    sqlx::query!("DELETE FROM users WHERE id = $1", user_id)
        .execute(&mut *conn)
        .await?;
    Ok(prefixes)
}

/// Deletes the clubs the user owns alone; 409 when one has other members. The club rows are
/// locked first (in id order, like every membership write), so a concurrent join either
/// committed before (and is counted) or fails afterwards.
async fn delete_owned_clubs(conn: &mut PgConnection, user_id: i64) -> ApiResult<()> {
    let owned = sqlx::query_scalar!(
        "SELECT c.id FROM clubs c
         JOIN club_memberships m ON m.club_id = c.id
         WHERE m.user_id = $1 AND m.role = 'owner'
         ORDER BY c.id
         FOR UPDATE OF c",
        user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    if owned.is_empty() {
        return Ok(());
    }
    // A new statement, so it sees every membership committed before the locks were granted.
    let shared = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM club_memberships WHERE club_id = ANY($1) AND user_id <> $2
           ) AS "shared!""#,
        &owned,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    if shared {
        return Err(AppError::conflict(TRANSFER_CLUBS_FIRST));
    }
    sqlx::query!("DELETE FROM clubs WHERE id = ANY($1)", &owned)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Deletes the user's posts that nobody else replied to (with the user's own replies under
/// them), then turns the rest, which others replied to, into tombstones. Reactions on deleted
/// posts cascade; `author_id` becomes NULL when the user row goes.
///
/// The tombstone step covers every post still left, so a reply deleted by its author between
/// the two statements cannot leave a live post without an author (`posts_live_has_author`).
async fn delete_posts(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM posts p
         WHERE p.author_id = $1
           AND NOT EXISTS (
               SELECT 1 FROM posts r WHERE r.parent_id = p.id AND r.author_id IS DISTINCT FROM $1)",
        user_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "UPDATE posts
         SET body = '', spoiler_upto = NULL, deleted_at = coalesce(deleted_at, now())
         WHERE author_id = $1",
        user_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The deferred `clubs_have_owner` / `club_memberships_keep_owner` check failed at COMMIT.
fn is_owner_check(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|db| db.constraint() == Some("clubs_have_owner"))
}
