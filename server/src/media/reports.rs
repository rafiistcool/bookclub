//! Notice and takedown (endpoint-map §6): any signed-in reader reports a cover, an instance
//! admin removes or dismisses it.
//!
//! **A takedown** ([`take_down`]), in one transaction: the media row is locked, the hashes of
//! its original bytes and of every stored variant go into `media_removed_hashes`, the row gets
//! `removed_at` (and loses `storage_key`), then `ingest::clear_media` clears every cover that
//! showed it, then the picture's open reports are resolved as `removed`. That order (media row,
//! catalog rows, reports) is the one ingest, `set_cover` and `merge_works` lock in, so a takedown
//! waits for an ingest that is attaching the picture and then clears it too. The bytes are
//! deleted after the commit. The row stays, with its hash: `ingest::ol_cover_media` never hands
//! it out again, no fetch or backfill downloads it again, `/media/…` answers 410, and an upload
//! of the same file, or of a variant a reader saved from `/media/…`, is refused.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::delete_prefixes;
use super::service::sha256_all;
use super::shared::MediaStore;
use super::shared::urls::{cover_urls, storage_prefix};
use crate::AppState;
use crate::auth::extract::{CurrentUser, InstanceAdmin};
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::catalog::shared::ingest::clear_media;
use crate::dto::{BookOut, UserSummary, user_summaries};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};
use crate::text::{py_strip, truncate_chars};

/// Longest `details` of a report (silently cut, like request notes).
pub const MAX_DETAILS_CHARS: usize = 2000;
/// The pydantic enum message for an unknown `reason`.
pub const REASON_CHOICES: &str =
    "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'";
/// The pydantic enum message for an unknown `action`.
pub const ACTION_CHOICES: &str = "Input should be 'remove' or 'dismiss'";
/// 409 for a report an admin already resolved (admin only).
pub const REPORT_RESOLVED: &str = "This report was already resolved";

const REASONS: [&str; 4] = ["copyright", "wrong_book", "offensive", "other"];

/// A reader's report of a cover.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportIn {
    /// `copyright`, `wrong_book`, `offensive` or `other`.
    pub reason: String,
    /// What is wrong; at most 2000 characters.
    #[serde(default)]
    pub details: Option<String>,
}

impl Validate for ReportIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(REASONS.contains(&self.reason.as_str()), REASON_CHOICES);
        if let Some(details) = &mut self.details {
            *details = truncate_chars(py_strip(details), MAX_DETAILS_CHARS).to_owned();
        }
    }
}

/// The report is in the admin queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportCreated {
    pub id: i64,
}

/// A report in the admin queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportOut {
    pub id: i64,
    /// `copyright`, `wrong_book`, `offensive` or `other`.
    pub reason: String,
    pub details: String,
    /// `open`, `removed` or `dismissed`.
    pub status: String,
    /// The reported picture.
    pub media_public_id: Option<String>,
    /// The picture was taken down (by this report or otherwise).
    pub removed: bool,
    /// The picture, null once it was taken down.
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    /// The book that showed the picture when it was reported (as it is now).
    pub book: Option<BookOut>,
    pub created_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub resolved_by: Option<UserSummary>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
}

/// The open reports, oldest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportList {
    pub items: Vec<ReportOut>,
}

/// What the admin decided.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportResolveIn {
    /// `remove` takes the picture down; `dismiss` keeps it.
    pub action: String,
}

impl Validate for ReportResolveIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            matches!(self.action.as_str(), "remove" | "dismiss"),
            ACTION_CHOICES,
        );
    }
}

/// Reports a cover (`cover_public_id` of a book or edition): copyright, the wrong book,
/// offensive, or something else. 20 per day.
#[utoipa::path(
    post,
    path = "/api/media/{public_id}/reports",
    operation_id = "media_report",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("public_id" = String, Path, description = "The cover's `cover_public_id`")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`: no such cover"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 per day)")
    )
)]
pub async fn report(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(public_id): ApiPath<String>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    let mut conn = state.db.acquire().await?;
    let media_id = sqlx::query_scalar!(
        "SELECT id FROM media WHERE public_id = $1 AND kind = 'cover'",
        public_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    state
        .limits
        .report_user
        .check_and_hit(&format!("u:{}", user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    // The book that shows the picture: an edition (and its work) first, else a work, else the
    // book a cover_replace request proposed it for.
    let (work_id, edition_id) = sqlx::query!(
        r#"SELECT (SELECT e.work_id FROM editions e WHERE e.cover_media_id = $1
                   ORDER BY e.id LIMIT 1) AS edition_work,
                  (SELECT e.id FROM editions e WHERE e.cover_media_id = $1
                   ORDER BY e.id LIMIT 1) AS edition,
                  (SELECT w.id FROM works w WHERE w.cover_media_id = $1
                   ORDER BY w.id LIMIT 1) AS work,
                  (SELECT r.work_id FROM catalog_edit_requests r WHERE r.media_id = $1
                   ORDER BY r.id LIMIT 1) AS requested_work,
                  (SELECT r.edition_id FROM catalog_edit_requests r WHERE r.media_id = $1
                   ORDER BY r.id LIMIT 1) AS requested_edition"#,
        media_id
    )
    .fetch_one(&mut *conn)
    .await
    .map(|row| match (row.edition_work, row.edition, row.work) {
        (Some(work), Some(edition), _) => (Some(work), Some(edition)),
        (_, _, Some(work)) => (Some(work), None),
        _ => (row.requested_work, row.requested_edition),
    })?;
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, media_id, work_id, edition_id, reason, details,
                              created_by)
         VALUES ('cover', $1, $2, $3, $4, $5, $6) RETURNING id",
        media_id,
        work_id,
        edition_id,
        body.reason,
        body.details.unwrap_or_default(),
        user.id
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok((StatusCode::CREATED, Json(ReportCreated { id })))
}

/// The open reports, oldest first.
#[utoipa::path(
    get,
    path = "/api/admin/reports",
    operation_id = "media_admin_list_reports",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = ReportList, description = "The open reports, oldest first"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` for everyone but instance admins")
    )
)]
pub async fn list_reports(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<ReportList>> {
    let mut conn = state.db.acquire().await?;
    let ids =
        sqlx::query_scalar!("SELECT id FROM reports WHERE status = 'open' ORDER BY created_at, id")
            .fetch_all(&mut *conn)
            .await?;
    Ok(Json(ReportList {
        items: report_outs(&mut conn, &ids).await?,
    }))
}

/// Removes the reported picture (everywhere it is shown, for good) or dismisses the report.
#[utoipa::path(
    post,
    path = "/api/admin/reports/{report_id}/resolve",
    operation_id = "media_admin_resolve_report",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("report_id" = i64, Path, description = "The report id")),
    request_body = ReportResolveIn,
    responses(
        (status = 200, body = ReportOut, description = "The resolved report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'remove' or 'dismiss'`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`"),
        (status = 409, body = ErrorBody, description = "`This report was already resolved`")
    )
)]
pub async fn resolve_report(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(report_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportResolveIn>,
) -> ApiResult<Json<ReportOut>> {
    let report = sqlx::query!(
        "SELECT media_id, status FROM reports WHERE id = $1",
        report_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if report.status != "open" {
        return Err(AppError::conflict(REPORT_RESOLVED));
    }
    let mut tx = state.db.begin().await?;
    let mut prefixes = Vec::new();
    let status = if body.action == "remove" {
        if let Some(media_id) = report.media_id {
            // Resolves the picture's open reports, this one included.
            prefixes = take_down_locked(&mut tx, state.media.as_ref(), media_id, admin.id).await?;
        }
        "removed"
    } else {
        "dismissed"
    };
    sqlx::query!(
        "UPDATE reports SET status = $3, resolved_by = $2, resolved_at = now()
         WHERE id = $1 AND status = 'open'",
        report_id,
        admin.id,
        status
    )
    .execute(&mut *tx)
    .await?;
    // Another admin may have resolved it since it was read; then this answer changes nothing
    // (the rollback undoes a takedown too).
    let ours = sqlx::query_scalar!(
        r#"SELECT COALESCE(resolved_by = $2 AND resolved_at = now(), false) AS "ours!"
           FROM reports WHERE id = $1"#,
        report_id,
        admin.id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if !ours {
        return Err(AppError::conflict(REPORT_RESOLVED));
    }
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    let mut conn = state.db.acquire().await?;
    let out = report_outs(&mut conn, &[report_id])
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    Ok(Json(out))
}

/// Takes a picture down directly, without a report: same effect as resolving one with
/// `remove`. Taking down a removed picture changes nothing.
#[utoipa::path(
    delete,
    path = "/api/admin/media/{public_id}",
    operation_id = "media_admin_take_down",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("public_id" = String, Path, description = "The picture's public id")),
    responses(
        (status = 204, description = "The picture is removed"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found`")
    )
)]
pub async fn take_down(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(public_id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    let media_id = sqlx::query_scalar!("SELECT id FROM media WHERE public_id = $1", public_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    let mut tx = state.db.begin().await?;
    let prefixes = take_down_locked(&mut tx, state.media.as_ref(), media_id, admin.id).await?;
    tx.commit().await?;
    delete_prefixes(state.media.as_ref(), &prefixes).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The takedown inside the caller's transaction (see the module docs): locks the media row,
/// records the hashes it must not come back under, sets `removed_at` and clears `storage_key`,
/// clears every cover that shows it, and resolves the picture's open reports as `removed` by
/// `admin_id`. Returns the storage prefixes to delete after the commit (empty when the picture
/// was removed already).
pub async fn take_down_locked(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
    media_id: i64,
    admin_id: i64,
) -> ApiResult<Vec<String>> {
    let Some(row) = sqlx::query!(
        r#"SELECT kind, public_id, storage_key, variants, sha256,
                  removed_at IS NOT NULL AS "removed!"
           FROM media WHERE id = $1 FOR UPDATE"#,
        media_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(Vec::new());
    };
    let mut prefixes = Vec::new();
    if !row.removed {
        let stored = match &row.storage_key {
            Some(prefix) => stored_variants(store, prefix, &row.variants).await?,
            None => Vec::new(),
        };
        record_removed_hashes(conn, media_id, row.sha256, stored).await?;
        sqlx::query!(
            "UPDATE media SET removed_at = now(), storage_key = NULL WHERE id = $1",
            media_id
        )
        .execute(&mut *conn)
        .await?;
        prefixes.push(storage_prefix(&row.kind, &row.public_id));
        prefixes.extend(row.storage_key);
    }
    clear_media(conn, media_id).await?;
    sqlx::query!(
        "UPDATE reports SET status = 'removed', resolved_by = $2, resolved_at = now()
         WHERE media_id = $1 AND status = 'open'",
        media_id,
        admin_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(prefixes)
}

/// The bytes of every stored variant, as `/media/{public_id}/{variant}` served them. A variant
/// whose bytes are missing is skipped (nothing can have been served from it).
async fn stored_variants(
    store: &dyn MediaStore,
    prefix: &str,
    variants: &[String],
) -> anyhow::Result<Vec<(String, Bytes)>> {
    let mut stored = Vec::new();
    for variant in variants {
        let key = format!("{prefix}/{variant}");
        match store.get(&key).await? {
            Some(bytes) => stored.push((variant.clone(), bytes)),
            None => tracing::warn!(key, "a stored variant of a picture taken down has no bytes"),
        }
    }
    Ok(stored)
}

/// Records the hashes a taken-down picture must not come back under (`media_removed_hashes`,
/// checked by every upload): the bytes as received (`original`, a copy of `media.sha256` that
/// outlives the row) and every stored variant, the bytes readers actually saw and saved. An
/// Open Library cover taken down before its first fetch has neither: we never had its bytes,
/// and fetching them now would download the removed picture again.
async fn record_removed_hashes(
    conn: &mut PgConnection,
    media_id: i64,
    original: Option<Vec<u8>>,
    stored: Vec<(String, Bytes)>,
) -> anyhow::Result<()> {
    let (names, bytes): (Vec<String>, Vec<Bytes>) = stored.into_iter().unzip();
    let mut hashes: Vec<Vec<u8>> = sha256_all(bytes)
        .await?
        .into_iter()
        .map(|hash| hash.to_vec())
        .collect();
    let mut variants: Vec<Option<String>> = names.into_iter().map(Some).collect();
    if let Some(original) = original {
        hashes.insert(0, original);
        variants.insert(0, None);
    }
    sqlx::query!(
        "INSERT INTO media_removed_hashes (sha256, media_id, variant)
         SELECT sha256, $1, variant FROM UNNEST($2::bytea[], $3::text[]) AS h (sha256, variant)
         ON CONFLICT (sha256) DO NOTHING",
        media_id,
        &hashes,
        &variants as &[Option<String>]
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The [`ReportOut`]s of `ids`, in that order (missing ids are left out).
pub async fn report_outs(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<Vec<ReportOut>> {
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!", r.reason AS "reason!", r.details AS "details!",
                  r.status AS "status!", r.work_id, r.edition_id, r.created_by,
                  r.created_at AS "created_at!", r.resolved_by, r.resolved_at,
                  m.public_id AS "public_id?", m.removed_at IS NOT NULL AS "removed?"
           FROM reports r
           LEFT JOIN media m ON m.id = r.media_id
           WHERE r.id = ANY($1)"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = rows
        .iter()
        .filter_map(|row| {
            row.work_id.map(|work_id| BookRef {
                work_id,
                edition_id: row.edition_id,
            })
        })
        .collect();
    let books = book_outs(conn, &refs).await?;
    let user_ids: Vec<i64> = rows
        .iter()
        .flat_map(|row| [row.created_by, row.resolved_by])
        .flatten()
        .collect();
    let users = user_summaries(conn, &user_ids).await?;
    let mut by_id: HashMap<i64, ReportOut> = rows
        .into_iter()
        .map(|row| {
            let removed = row.removed.unwrap_or(false);
            let shown = row.public_id.as_deref().filter(|_| !removed);
            let (cover_url, cover_thumb_url) = cover_urls(shown);
            let book = row.work_id.and_then(|work_id| {
                books
                    .get(&BookRef {
                        work_id,
                        edition_id: row.edition_id,
                    })
                    .cloned()
            });
            let out = ReportOut {
                id: row.id,
                reason: row.reason,
                details: row.details,
                status: row.status,
                media_public_id: row.public_id,
                removed,
                cover_url,
                cover_thumb_url,
                book,
                created_by: row.created_by.and_then(|id| users.get(&id).cloned()),
                created_at: row.created_at,
                resolved_by: row.resolved_by.and_then(|id| users.get(&id).cloned()),
                resolved_at: row.resolved_at,
            };
            (row.id, out)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}
