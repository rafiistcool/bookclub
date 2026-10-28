//! `GET /media/{public_id}/{variant}` (endpoint-map §6): every picture the app shows, from our
//! own origin. The URL is unguessable and immutable (a new picture gets a new public id), so a
//! 200 is cached for a year; every other answer carries `Cache-Control: no-store`, so a cover
//! that failed once is tried again on the next view.
//!
//! Order: the row by public id (404) → `removed_at` (410, never fetched again) → the variant
//! (404) → the stored bytes, or for an Open Library cover without bytes, one coalesced fetch
//! that waits up to 10 s for the `ol_covers` throttle (429 while Open Library is busy, 404 when
//! it has no cover, 502 when it did not answer).

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;

use super::service::CoverOutcome;
use super::{COVER_BUSY, COVER_UNAVAILABLE, PICTURE_REMOVED};
use crate::AppState;
use crate::error::{AppError, msg};
use crate::extract::ApiPath;
use crate::outbound::RetryPolicy;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// One stored picture.
struct Row {
    id: i64,
    kind: String,
    source: String,
    storage_key: Option<String>,
    content_type: Option<String>,
    variants: Vec<String>,
    removed: bool,
}

/// Serves a picture. Public: the id is the secret. Covers have the variants `l` (≤ 800 px wide)
/// and `s` (≤ 240 px wide), avatars `a` (256×256).
#[utoipa::path(
    get,
    path = "/media/{public_id}/{variant}",
    operation_id = "media_get",
    tag = "media",
    params(
        ("public_id" = String, Path, description = "The picture's public id (`cover_public_id`, or the id in `avatar_url`)"),
        ("variant" = String, Path, description = "`l` or `s` for covers, `a` for avatars")
    ),
    responses(
        (status = 200, description = "The image (`image/webp`, or `image/jpeg` for imported avatars), cached for a year", content_type = "image/webp", body = Vec<u8>),
        (status = 404, body = crate::error::ErrorBody, description = "`Not found`: unknown id or variant, or Open Library has no such cover"),
        (status = 410, body = crate::error::ErrorBody, description = "`This picture was removed`"),
        (status = 429, body = crate::error::ErrorBody, description = "`The library is busy. Wait a few seconds and try again.` (Open Library throttled)"),
        (status = 502, body = crate::error::ErrorBody, description = "`Could not load the cover right now. Try again.` (Open Library did not answer)")
    )
)]
pub async fn get_media(
    State(state): State<AppState>,
    path: Result<ApiPath<(String, String)>, AppError>,
) -> Response {
    let Ok(ApiPath((public_id, variant))) = path else {
        return no_store(AppError::not_found(msg::NOT_FOUND));
    };
    match serve(&state, &public_id, &variant).await {
        Ok((content_type, bytes)) => (
            [
                (CONTENT_TYPE, content_type),
                (CACHE_CONTROL, IMMUTABLE.to_owned()),
                (X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => no_store(error),
    }
}

/// Any answer but a 200: never cached, never sniffed.
fn no_store(error: AppError) -> Response {
    (
        [
            (CACHE_CONTROL, "no-store"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        error,
    )
        .into_response()
}

async fn serve(
    state: &AppState,
    public_id: &str,
    variant: &str,
) -> Result<(String, Bytes), AppError> {
    if !valid_public_id(public_id) {
        return Err(AppError::not_found(msg::NOT_FOUND));
    }
    let mut row = load(state, public_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    if row.removed {
        return Err(AppError::new(StatusCode::GONE, PICTURE_REMOVED));
    }
    if !kind_has_variant(&row.kind, variant) {
        return Err(AppError::not_found(msg::NOT_FOUND));
    }
    if row.storage_key.is_none() {
        if row.source != "openlibrary" {
            return Err(AppError::not_found(msg::NOT_FOUND));
        }
        match state
            .media_service
            .ensure_cover(&state.db, row.id, RetryPolicy::MEDIA)
            .await
        {
            CoverOutcome::Stored => {}
            CoverOutcome::Removed => {
                return Err(AppError::new(StatusCode::GONE, PICTURE_REMOVED));
            }
            CoverOutcome::NoCover => return Err(AppError::not_found(msg::NOT_FOUND)),
            CoverOutcome::Busy => {
                let wait = state
                    .outbound
                    .ol_covers
                    .cooldown_remaining()
                    .map(|left| left.as_secs().max(1));
                return Err(AppError::too_many(COVER_BUSY, wait));
            }
            CoverOutcome::Failed => return Err(AppError::bad_gateway(COVER_UNAVAILABLE)),
        }
        row = load(state, public_id)
            .await?
            .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
        if row.removed {
            return Err(AppError::new(StatusCode::GONE, PICTURE_REMOVED));
        }
    }
    let (Some(prefix), Some(content_type)) = (&row.storage_key, &row.content_type) else {
        return Err(AppError::not_found(msg::NOT_FOUND));
    };
    if !row.variants.iter().any(|stored| stored == variant) {
        return Err(AppError::not_found(msg::NOT_FOUND));
    }
    let key = format!("{prefix}/{variant}");
    match state.media.get(&key).await {
        Ok(Some(bytes)) => Ok((content_type.clone(), bytes)),
        Ok(None) => {
            tracing::warn!(key, "a stored picture has no bytes");
            Err(AppError::not_found(msg::NOT_FOUND))
        }
        Err(error) => Err(AppError::internal(format!("media store: {error:#}"))),
    }
}

async fn load(state: &AppState, public_id: &str) -> sqlx::Result<Option<Row>> {
    sqlx::query_as!(
        Row,
        r#"SELECT id, kind, source, storage_key, content_type, variants,
                  removed_at IS NOT NULL AS "removed!"
           FROM media WHERE public_id = $1"#,
        public_id
    )
    .fetch_optional(&state.db)
    .await
}

/// `[a-z0-9]{26}`, the schema's check: anything else names no picture.
fn valid_public_id(public_id: &str) -> bool {
    public_id.len() == 26
        && public_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn kind_has_variant(kind: &str, variant: &str) -> bool {
    match kind {
        "cover" => matches!(variant, "l" | "s"),
        "avatar" => variant == "a",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ids_follow_the_schema() {
        assert!(valid_public_id("abcdefghijklmnopqrstuvwxyz"));
        assert!(valid_public_id("a2345672345672345672345672"));
        assert!(!valid_public_id("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
        assert!(!valid_public_id("abc"));
        assert!(!valid_public_id("abcdefghijklmnopqrstuvwxy\0"));
    }

    #[test]
    fn variants_belong_to_their_kind() {
        assert!(kind_has_variant("cover", "l"));
        assert!(kind_has_variant("cover", "s"));
        assert!(!kind_has_variant("cover", "a"));
        assert!(kind_has_variant("avatar", "a"));
        assert!(!kind_has_variant("avatar", "l"));
    }
}
