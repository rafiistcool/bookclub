//! `POST /api/shelf/spine-colors` (endpoint-map §6, docs/phase1/spec/shelf.md §1.6): the
//! backfill of spine colours for the caller's shelf, a batch of 12 books per call.
//!
//! Colours are sampled when a cover is stored (an upload or an Open Library fetch); this route
//! catches the rest: covers whose bytes were never fetched, and stored pictures without a
//! colour (imported ones). The cover of a shelf entry is the one its `BookOut` shows: the
//! entry's edition's, else the work's. A taken-down picture is never pending and never fetched.
//!
//! Each picture settles as a colour, as `''` (no usable cover: Open Library has none, a refused
//! redirect, too large, undecodable), or stays NULL when the fetch should be tried again later
//! (throttled, a transport error, an unexpected status). After such a failure `remaining` is 0,
//! so the client stops asking while the cover host is down.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::State;
use futures::future::join_all;
use serde::Serialize;
use utoipa::ToSchema;

use super::color::dominant_color;
use super::service::CoverOutcome;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{ApiResult, ErrorBody};
use crate::outbound::RetryPolicy;

/// Books resolved per call.
pub const SPINE_COLOR_BATCH: usize = 12;

/// The colours sampled by one call.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SpineColorsOut {
    /// `#rrggbb` by work id (`BookOut.id`, as a string key); only books that got a colour.
    pub colors: BTreeMap<String, String>,
    /// Books still waiting after this batch; 0 after a fetch that should be retried later.
    pub remaining: i64,
}

/// A pending shelf book: its work and the picture its cover shows.
struct Pending {
    work_id: i64,
    media_id: i64,
    storage_key: Option<String>,
}

/// Samples the cover colours of the caller's shelf books that have none yet, 12 at a time.
#[utoipa::path(
    post,
    path = "/api/shelf/spine-colors",
    operation_id = "media_spine_colors",
    tag = "media",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = SpineColorsOut, description = "The colours of this batch; the client asks again while `remaining > 0`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn spine_colors(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<SpineColorsOut>> {
    let pending = {
        let mut conn = state.db.acquire().await?;
        sqlx::query_as!(
            Pending,
            r#"SELECT s.work_id, m.id AS media_id, m.storage_key
               FROM shelf_entries s
               JOIN works w ON w.id = s.work_id
               LEFT JOIN editions e ON e.id = s.edition_id AND e.work_id = s.work_id
               LEFT JOIN media em ON em.id = e.cover_media_id AND em.removed_at IS NULL
               LEFT JOIN media wm ON wm.id = w.cover_media_id AND wm.removed_at IS NULL
               JOIN media m ON m.id = COALESCE(em.id, wm.id)
               WHERE s.user_id = $1 AND m.dominant_color IS NULL
               ORDER BY s.work_id"#,
            user.id
        )
        .fetch_all(&mut *conn)
        .await?
    };
    let batch = &pending[..pending.len().min(SPINE_COLOR_BATCH)];

    // Two books can show one picture; each picture is resolved once.
    let mut pictures: Vec<&Pending> = Vec::new();
    for book in batch {
        if !pictures.iter().any(|seen| seen.media_id == book.media_id) {
            pictures.push(book);
        }
    }
    let results = join_all(pictures.iter().map(|picture| resolve(&state, picture))).await;
    let mut settled: HashMap<i64, Option<String>> = HashMap::new();
    for (picture, result) in pictures.iter().zip(results) {
        let color = result.unwrap_or_else(|error| {
            tracing::warn!(media_id = picture.media_id, %error, "sampling a spine colour failed");
            None
        });
        settled.insert(picture.media_id, color);
    }

    let mut colors = BTreeMap::new();
    let mut unresolved = false;
    for book in batch {
        match settled.get(&book.media_id).cloned().flatten() {
            None => unresolved = true,
            Some(color) if !color.is_empty() => {
                colors.insert(book.work_id.to_string(), color);
            }
            Some(_) => {}
        }
    }
    let remaining = if unresolved {
        0
    } else {
        (pending.len() - batch.len()) as i64
    };
    Ok(Json(SpineColorsOut { colors, remaining }))
}

/// The settled colour of one picture (`""` for none), or `None` to try again later.
async fn resolve(state: &AppState, picture: &Pending) -> anyhow::Result<Option<String>> {
    if picture.storage_key.is_none() {
        match state
            .media_service
            .ensure_cover(&state.db, picture.media_id, RetryPolicy::MEDIA)
            .await
        {
            CoverOutcome::Stored => {}
            // Settled: the fetch recorded `''`, or the picture is gone (and never sampled).
            CoverOutcome::NoCover | CoverOutcome::Removed => {
                settle(state, picture.media_id, "").await?;
                return Ok(Some(String::new()));
            }
            CoverOutcome::Busy | CoverOutcome::Failed => return Ok(None),
        }
    }
    let row = sqlx::query!(
        r#"SELECT storage_key, variants, dominant_color, removed_at IS NOT NULL AS "removed!"
           FROM media WHERE id = $1"#,
        picture.media_id
    )
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row.filter(|row| !row.removed) else {
        return Ok(Some(String::new()));
    };
    if let Some(color) = row.dominant_color {
        return Ok(Some(color));
    }
    // A stored picture without a colour: sample its largest variant.
    let color = match (&row.storage_key, row.variants.first()) {
        (Some(prefix), Some(variant)) => {
            match state.media.get(&format!("{prefix}/{variant}")).await? {
                Some(bytes) => dominant_color(&bytes).unwrap_or_default(),
                None => String::new(),
            }
        }
        _ => String::new(),
    };
    settle(state, picture.media_id, &color).await?;
    Ok(Some(color))
}

/// Stores a sampled colour unless the picture was taken down or sampled meanwhile.
async fn settle(state: &AppState, media_id: i64, color: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE media SET dominant_color = $2
         WHERE id = $1 AND dominant_color IS NULL AND removed_at IS NULL",
        media_id,
        color
    )
    .execute(&state.db)
    .await?;
    Ok(())
}
