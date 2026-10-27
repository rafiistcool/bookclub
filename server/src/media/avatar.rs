//! `PUT` and `DELETE /api/auth/me/avatar` (endpoint-map §2, docs/phase1/spec/auth.md §1.6–1.7):
//! the profile picture, a 256×256 WebP without metadata, served at `/media/{public_id}/a`.
//!
//! An upload stores the new picture under a new public id, then, in one transaction under the
//! user-row lock, deletes the user's previous avatar rows, registers the new one and points
//! `users.avatar_media_id` at it; the old bytes are deleted after the commit. A picture taken
//! down by an admin keeps its row (and hash), so the same file cannot come back.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use bytes::Bytes;
use sqlx::PgConnection;

use super::pipeline::{PictureKind, sha256};
use super::service::store_variants;
use super::shared::urls::{new_public_id, storage_prefix};
use super::shared::{NewMedia, is_removed_hash, register};
use super::upload::{PictureUpload, picture_error, read_picture};
use super::{PICTURE_REMOVED, delete_prefixes};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::user_out;
use crate::dto::UserOut;
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::ApiMultipart;

/// Uploads a profile picture: centre-cropped, 256×256, WebP, metadata (EXIF, GPS) dropped. The
/// previous picture is deleted.
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
        (status = 413, body = ErrorBody, description = "`That picture is too large (50 MB max)` (by `Content-Length`, before the body is read)")
    )
)]
pub async fn put_avatar(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<Json<UserOut>> {
    let (data, _) = read_picture(&headers, multipart).await?;
    refuse_removed(&state, &data).await?;
    let processed = state
        .media_service
        .process(PictureKind::Avatar, data)
        .await
        .map_err(picture_error)?;
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

/// 400 `This picture was removed` when an admin took down a picture with these bytes.
pub async fn refuse_removed(state: &AppState, data: &Bytes) -> ApiResult<()> {
    let hash = sha256(data);
    let mut conn = state.db.acquire().await?;
    if is_removed_hash(&mut conn, &hash).await? {
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

/// Deletes every avatar row of the user that was not taken down (the current one and any a
/// crash left behind) and returns their storage prefixes.
async fn delete_avatars(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<Vec<String>> {
    let rows = sqlx::query!(
        "DELETE FROM media
         WHERE kind = 'avatar' AND removed_at IS NULL
           AND (uploaded_by = $1 OR id = (SELECT avatar_media_id FROM users WHERE id = $1))
         RETURNING public_id, storage_key",
        user_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .flat_map(|row| {
            let mut prefixes = vec![storage_prefix(PictureKind::Avatar.as_str(), &row.public_id)];
            prefixes.extend(row.storage_key);
            prefixes
        })
        .collect())
}
