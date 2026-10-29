//! `POST /api/books/works/{work_id}/cover` (endpoint-map §6): a reader's photo of their copy.
//!
//! The photo goes through the pipeline (EXIF and GPS dropped, `l` and `s` WebP, spine colour)
//! and is stored under a new public id. Then, in one transaction:
//! 1. the work row is locked (`FOR UPDATE`, following merges), the order `merge_works` uses, so a
//!    merge either sees the new cover or the upload lands on the surviving work;
//! 2. the media row is registered;
//! 3. `ingest::set_cover(…, Source::User, only_if_empty = true)` makes it the cover of the
//!    edition (with `edition_id`) or of the work, but only where there is none
//!    (`… AND cover_media_id IS NULL`): 200 `{book}`;
//! 4. when that sets nothing (a cover exists, or a concurrent first upload won), the upload
//!    becomes a `cover_replace` request for the admin queue (CAT's approve applies it): 202
//!    `{request_id}`.
//!
//! An upload whose bytes match a taken-down picture, the file it was uploaded or fetched as or
//! one of the WebP variants it was served as, is refused (400 `This picture was removed`) before
//! anything is stored. 30 uploads per user and day.
//!
//! `PUT /api/admin/works/{work_id}/cover` (#74): an instance admin's picture becomes the cover
//! of the work or edition at once, replacing any cover (`set_cover(…, Source::Admin, false)`),
//! through the same steps.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::pipeline::PictureKind;
use super::service::store_variants;
use super::shared::urls::{new_public_id, storage_prefix};
use super::shared::{NewMedia, register};
use super::upload::{CoverUpload, picture_error, read_picture, upload_turn};
use super::{avatar::refuse_removed, delete_prefixes};
use crate::AppState;
use crate::auth::extract::{CurrentUser, InstanceAdmin};
use crate::catalog::shared::books::{BookRef, book_out, follow_merges, require_edition_of};
use crate::catalog::shared::ingest::{CoverTarget, Source, set_cover};
use crate::catalog::shared::work_ref::{WorkRef, resolve_local};
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiMultipart, ApiPath, INTEGER_PARSE};
use crate::ratelimit::GatePass;

/// Attempts at locking a work that a concurrent merge moves away.
const MERGE_ATTEMPTS: usize = 3;

/// The photo became the cover.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverSet {
    /// The book with its new cover (in the edition's context when one was named).
    pub book: BookOut,
}

/// A cover existed, so the photo waits in the admin queue as a `cover_replace` request.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverRequested {
    pub request_id: i64,
}

/// Uploads a photo of the reader's copy as the book's cover. The first cover of an edition (or
/// of a work, without `edition_id`) is set at once; otherwise an admin decides.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/cover",
    operation_id = "media_upload_cover",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body(content = CoverUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, body = CoverSet, description = "The photo is the cover now"),
        (status = 202, body = CoverRequested, description = "A cover existed: the photo is a `cover_replace` request"),
        (status = 400, body = ErrorBody, description = "`Use a JPEG, PNG, WebP, or GIF`, `That file is not a picture`, `That picture has too many pixels`, `That picture is too large (50 MB max)`, `This picture was removed`, `That edition belongs to another book` or an `edition_id` that is not a number"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 413, body = ErrorBody, description = "`That picture is too large (50 MB max)` (by `Content-Length`, before the body is read)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (30 uploads per day, or another upload of the account still running)")
    )
)]
pub async fn upload_cover(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    headers: HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<Response> {
    let received = receive(&state, user.id, &raw, &headers, multipart).await?;
    let (work_id, edition_id) = (received.work_id, received.edition_id);
    let applied = async {
        let mut tx = state.db.begin().await?;
        let work_id = lock_current_work(&mut tx, work_id).await?;
        if let Some(edition_id) = edition_id {
            require_edition_of(&mut tx, work_id, edition_id).await?;
        }
        let media_id = register(&mut tx, &received.public_id, &received.media).await?;
        let target = match edition_id {
            Some(edition_id) => CoverTarget::Edition(edition_id),
            None => CoverTarget::Work(work_id),
        };
        let outcome = if set_cover(&mut tx, target, media_id, Source::User, true).await? {
            Applied::Set(work_id)
        } else {
            Applied::Requested(
                file_cover_request(&mut tx, work_id, edition_id, media_id, user.id).await?,
            )
        };
        tx.commit().await?;
        ApiResult::Ok(outcome)
    }
    .await;
    let applied = match applied {
        Ok(applied) => applied,
        Err(error) => {
            received.discard(&state).await;
            return Err(error);
        }
    };
    match applied {
        Applied::Set(work_id) => {
            let mut conn = state.db.acquire().await?;
            let book = book_out(
                &mut conn,
                BookRef {
                    work_id,
                    edition_id,
                },
            )
            .await?
            .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
            Ok((StatusCode::OK, Json(CoverSet { book })).into_response())
        }
        Applied::Requested(request_id) => {
            Ok((StatusCode::ACCEPTED, Json(CoverRequested { request_id })).into_response())
        }
    }
}

/// An instance admin sets a book's or an edition's cover directly (#74): the picture becomes
/// the cover at once, replacing any cover, with provenance `admin` (no source overwrites it
/// afterwards). The same pipeline and limits as a reader's photo; also for a book a reader
/// added.
#[utoipa::path(
    put,
    path = "/api/admin/works/{work_id}/cover",
    operation_id = "media_admin_set_cover",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    request_body(content = CoverUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, body = CoverSet, description = "The picture is the cover now"),
        (status = 400, body = ErrorBody, description = "`Use a JPEG, PNG, WebP, or GIF`, `That file is not a picture`, `That picture has too many pixels`, `That picture is too large (50 MB max)`, `This picture was removed`, `That edition belongs to another book` or an `edition_id` that is not a number"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`, or `Not found` (not an instance admin)"),
        (status = 413, body = ErrorBody, description = "`That picture is too large (50 MB max)` (by `Content-Length`, before the body is read)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (30 uploads per day, or another upload of the account still running)")
    )
)]
pub async fn admin_set_cover(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
    headers: HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<Json<CoverSet>> {
    let received = receive(&state, admin.id, &raw, &headers, multipart).await?;
    let edition_id = received.edition_id;
    let applied = async {
        let mut tx = state.db.begin().await?;
        let work_id = lock_current_work(&mut tx, received.work_id).await?;
        if let Some(edition_id) = edition_id {
            require_edition_of(&mut tx, work_id, edition_id).await?;
        }
        let media_id = register(&mut tx, &received.public_id, &received.media).await?;
        let target = match edition_id {
            Some(edition_id) => CoverTarget::Edition(edition_id),
            None => CoverTarget::Work(work_id),
        };
        if !set_cover(&mut tx, target, media_id, Source::Admin, false).await? {
            return Err(AppError::not_found(msg::NO_SUCH_BOOK));
        }
        tx.commit().await?;
        ApiResult::Ok(work_id)
    }
    .await;
    let work_id = match applied {
        Ok(work_id) => work_id,
        Err(error) => {
            received.discard(&state).await;
            return Err(error);
        }
    };
    let mut conn = state.db.acquire().await?;
    let book = book_out(
        &mut conn,
        BookRef {
            work_id,
            edition_id,
        },
    )
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    Ok(Json(CoverSet { book }))
}

/// A photo both uploads took in: the book it is for, and the picture processed, checked
/// against takedowns and stored under a new public id (not registered yet).
struct Received {
    /// The account's one upload at a time, held until the request ends.
    _turn: GatePass,
    work_id: i64,
    edition_id: Option<i64>,
    public_id: String,
    media: NewMedia<'static>,
}

impl Received {
    /// Deletes the stored bytes again (the transaction that would register them failed).
    async fn discard(&self, state: &AppState) {
        let prefix = storage_prefix(PictureKind::Cover.as_str(), &self.public_id);
        delete_prefixes(state.media.as_ref(), &[prefix]).await;
    }
}

/// The steps the reader's and the admin's upload share, in the legacy order: one upload per
/// account at a time and none over the daily limit (checked before the body is read, counted
/// after the body and the book checks), the body, the book and edition, the pipeline (metadata
/// stripped, WebP `l`/`s`), the takedown hashes, the store.
async fn receive(
    state: &AppState,
    user_id: i64,
    raw: &str,
    headers: &HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<Received> {
    let turn = upload_turn(&state.limits, user_id)?;
    state
        .limits
        .upload_user
        .check(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (data, fields) = read_picture(headers, multipart).await?;
    let edition_id = match fields.0.get("edition_id").map(|value| value.trim()) {
        None | Some("") => None,
        Some(value) => Some(
            value
                .parse::<i64>()
                .map_err(|_| AppError::bad_request(INTEGER_PARSE))?,
        ),
    };
    let work_id = {
        let mut conn = state.db.acquire().await?;
        let work_id = match WorkRef::parse(raw) {
            Some(work_ref) => resolve_local(&mut conn, &work_ref).await?,
            None => None,
        }
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
        if let Some(edition_id) = edition_id {
            require_edition_of(&mut conn, work_id, edition_id).await?;
        }
        work_id
    };
    state
        .limits
        .upload_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let processed = state
        .media_service
        .process(PictureKind::Cover, data)
        .await
        .map_err(picture_error)?;
    refuse_removed(state, &processed.sha256).await?;
    let public_id = new_public_id();
    store_variants(
        state.media.as_ref(),
        PictureKind::Cover,
        &public_id,
        &processed,
    )
    .await
    .map_err(AppError::internal)?;
    let media = NewMedia {
        kind: PictureKind::Cover.as_str(),
        source: "upload",
        content_type: "image/webp",
        variants: PictureKind::Cover.variants(),
        width: processed.width as i32,
        height: processed.height as i32,
        byte_size: processed.byte_size(),
        sha256: processed.sha256,
        dominant_color: processed.dominant_color.clone(),
        uploaded_by: Some(user_id),
        created_at: None,
    };
    Ok(Received {
        _turn: turn,
        work_id,
        edition_id,
        public_id,
        media,
    })
}

enum Applied {
    /// The cover of this work (or of its edition) is the photo now.
    Set(i64),
    /// The `cover_replace` request filed instead.
    Requested(i64),
}

/// Locks the work `FOR UPDATE` while it is not merged away, following merges; 404 `No such
/// book.` when it is gone.
async fn lock_current_work(conn: &mut PgConnection, mut work_id: i64) -> ApiResult<i64> {
    for _ in 0..MERGE_ATTEMPTS {
        let locked = sqlx::query_scalar!(
            "SELECT id FROM works WHERE id = $1 AND merged_into_id IS NULL FOR UPDATE",
            work_id
        )
        .fetch_optional(&mut *conn)
        .await?;
        if locked.is_some() {
            return Ok(work_id);
        }
        work_id = follow_merges(conn, work_id)
            .await?
            .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    }
    Err(AppError::not_found(msg::NO_SUCH_BOOK))
}

/// A `cover_replace` request as CAT's approve expects it: the current work, the edition when
/// one was named, the photo, and an empty payload.
async fn file_cover_request(
    conn: &mut PgConnection,
    work_id: i64,
    edition_id: Option<i64>,
    media_id: i64,
    created_by: i64,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, media_id, created_by)
         VALUES ('cover_replace', $1, $2, $3, $4) RETURNING id",
        work_id,
        edition_id,
        media_id,
        created_by
    )
    .fetch_one(conn)
    .await
}
