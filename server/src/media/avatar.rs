//! `PUT` and `DELETE /api/auth/me/avatar` (endpoint-map §2, docs/phase1/spec/auth.md §1.6–1.7):
//! the profile picture, a 256×256 WebP without metadata, served at `/media/{public_id}/a`.
//!
//! An upload stores the new picture under a new public id, then, in one transaction under the
//! user-row lock, deletes the user's previous avatar rows, registers the new one and points
//! `users.avatar_media_id` at it; the old bytes are deleted after the commit. A picture taken
//! down by an admin keeps its row (and hash), and the takedown records the hash of the served
//! `a` variant too, so neither the uploaded file nor the one readers saved can come back.
//!
//! A picture with an open report is held instead of deleted, until an admin decides it
//! (security review, Phase 2 Wave 2): otherwise replacing or deleting it would leave nothing to
//! take down, and the same file could be uploaded again once the report was closed. It leaves
//! the profile at once, and gets a new public id that only the admin queue shows, so its old URL
//! answers 404 like a deleted picture's and tells the owner nothing (`media::reports`, "Held").

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use sqlx::PgConnection;

use super::pipeline::PictureKind;
use super::service::store_variants;
use super::shared::urls::{new_public_id, storage_prefix};
use super::shared::{NewMedia, is_removed_hash, register};
use super::upload::{PictureUpload, picture_error, read_picture, upload_turn};
use super::{PICTURE_REMOVED, delete_prefixes};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::user_out;
use crate::dto::UserOut;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::ApiMultipart;

/// Uploads a profile picture: centre-cropped, 256×256, WebP, metadata (EXIF, GPS) dropped. The
/// previous picture is deleted. Ten uploads per account an hour, and one at a time.
#[utoipa::path(
    put,
    path = "/api/auth/me/avatar",
    operation_id = "media_put_avatar",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body(content = PictureUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, body = UserOut, description = "The account with its new `avatar_url`"),
        (status = 400, body = ErrorBody, description = "`Use a JPEG, PNG, WebP, or GIF`, `That file is not a picture`, `That picture has too many pixels`, `That picture is too large (50 MB max)` or `This picture was removed`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 413, body = ErrorBody, description = "`That picture is too large (50 MB max)` (by `Content-Length`, before the body is read)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (10 an hour, or another upload of the account still running)")
    )
)]
pub async fn put_avatar(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<Json<UserOut>> {
    // Before the body is read: the upload's 50 MiB and its decode are what one account must
    // not multiply (security review 2026-10-28 #4).
    let _turn = upload_turn(&state.limits, user.id)?;
    state
        .limits
        .avatar_user
        .check_and_hit(&format!("u:{}", user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (data, _) = read_picture(&headers, multipart).await?;
    let processed = state
        .media_service
        .process(PictureKind::Avatar, data)
        .await
        .map_err(picture_error)?;
    refuse_removed(&state, &processed.sha256).await?;
    let public_id = new_public_id();
    store_variants(
        state.media.as_ref(),
        PictureKind::Avatar,
        &public_id,
        &processed,
    )
    .await
    .map_err(AppError::internal)?;
    let new_prefix = storage_prefix(PictureKind::Avatar.as_str(), &public_id);

    let replaced = async {
        let mut tx = state.db.begin().await?;
        lock_user(&mut tx, user.id).await?;
        let old = delete_avatars(&mut tx, user.id).await?;
        let media_id = register(
            &mut tx,
            &public_id,
            &NewMedia {
                kind: PictureKind::Avatar.as_str(),
                source: "upload",
                content_type: "image/webp",
                variants: PictureKind::Avatar.variants(),
                width: processed.width as i32,
                height: processed.height as i32,
                byte_size: processed.byte_size(),
                sha256: processed.sha256,
                dominant_color: None,
                uploaded_by: Some(user.id),
                created_at: None,
            },
        )
        .await?;
        sqlx::query!(
            "UPDATE users SET avatar_media_id = $2 WHERE id = $1",
            user.id,
            media_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        ApiResult::Ok(old)
    }
    .await;
    let old = match replaced {
        Ok(old) => old,
        Err(error) => {
            delete_prefixes(state.media.as_ref(), &[new_prefix]).await;
            return Err(error);
        }
    };
    delete_prefixes(state.media.as_ref(), &old).await;
    let mut conn = state.db.acquire().await?;
    Ok(Json(user_out(&mut conn, user.id).await?))
}

/// Deletes the profile picture. Answers the same when there is none.
#[utoipa::path(
    delete,
    path = "/api/auth/me/avatar",
    operation_id = "media_delete_avatar",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = UserOut, description = "The account, `avatar_url` null"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn delete_avatar(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<UserOut>> {
    let mut tx = state.db.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let old = delete_avatars(&mut tx, user.id).await?;
    sqlx::query!(
        "UPDATE users SET avatar_media_id = NULL WHERE id = $1",
        user.id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &old).await;
    let mut conn = state.db.acquire().await?;
    Ok(Json(user_out(&mut conn, user.id).await?))
}

/// 400 `This picture was removed` when an upload with this SHA-256 ([`Processed::sha256`], the
/// bytes as received, hashed on the blocking pool) is a picture an admin took down: its
/// original bytes (`media.sha256` of a removed row) or one of the variants it was served as
/// (`media_removed_hashes`, recorded by the takedown).
///
/// [`Processed::sha256`]: super::pipeline::Processed::sha256
pub async fn refuse_removed(state: &AppState, sha256: &[u8; 32]) -> ApiResult<()> {
    let mut conn = state.db.acquire().await?;
    let served = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM media_removed_hashes WHERE sha256 = $1) AS "removed!""#,
        &sha256[..]
    )
    .fetch_one(&mut *conn)
    .await?;
    if served || is_removed_hash(&mut conn, sha256).await? {
        return Err(AppError::bad_request(PICTURE_REMOVED));
    }
    Ok(())
}

/// Serialises a user's avatar writes; 401 when the account is gone.
async fn lock_user(conn: &mut PgConnection, user_id: i64) -> ApiResult<()> {
    sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", user_id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    Ok(())
}

/// Takes every avatar row of the user that was not taken down (the current one and any a crash
/// left behind or a report holds) off the account. A row with an open report is held for the
/// admin's decision under a new public id (see the module docs); every other row is deleted, and
/// its storage prefixes are returned, to delete after the commit.
async fn delete_avatars(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<Vec<String>> {
    // Locked first: a report being filed holds its picture `FOR SHARE` until it commits, so the
    // next statement, on a new snapshot, sees it.
    let rows = sqlx::query!(
        "SELECT id, public_id, storage_key FROM media
         WHERE kind = 'avatar' AND removed_at IS NULL
           AND (uploaded_by = $1 OR id = (SELECT avatar_media_id FROM users WHERE id = $1))
         ORDER BY id
         FOR UPDATE",
        user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let reported = sqlx::query_scalar!(
        r#"SELECT DISTINCT media_id AS "id!" FROM reports
           WHERE media_id = ANY($1) AND status = 'open'"#,
        &ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut prefixes = Vec::new();
    for row in rows {
        if reported.contains(&row.id) {
            sqlx::query!(
                "UPDATE media SET public_id = $2 WHERE id = $1",
                row.id,
                new_public_id()
            )
            .execute(&mut *conn)
            .await?;
        } else {
            sqlx::query!("DELETE FROM media WHERE id = $1", row.id)
                .execute(&mut *conn)
                .await?;
            prefixes.push(storage_prefix(PictureKind::Avatar.as_str(), &row.public_id));
            prefixes.extend(row.storage_key);
        }
    }
    Ok(prefixes)
}
