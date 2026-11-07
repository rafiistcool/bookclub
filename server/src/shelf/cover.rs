//! The personal cover choice (docs/phase3/notes/cover-choice.md): a reader picks the cover a book
//! on their shelf shows. `PUT /api/shelf/{entry_id}/cover` takes a candidate's key (from
//! `GET /api/books/works/{work_id}/cover-candidates`), `DELETE` goes back to the standard cover;
//! the photo of the reader's own copy is `PUT …/cover/photo` (`media::cover`).
//!
//! The choice is `shelf_entries.cover_media_id`. Wherever an entry's book shows (the shelf,
//! Home, the book page for the reader, the shelf as friends and club mates see it, the year in
//! review), [`personal_covers`] loads it and [`apply_personal_cover`] puts it into the entry's
//! [`BookOut`]: the personal cover wins over the edition's and the work's. `BookRef` and
//! `book_outs` stay as they are (frozen): the choice belongs to an entry, not to a book.
//!
//! A key names a picture the server already knows:
//! - `ol:<cover_id>`: an Open Library cover whose (lazy) media row exists, which the candidate
//!   list creates;
//! - `media:<public_id>`: the cover of the entry's work or one of its editions, an Open Library
//!   cover's row, or a personal photo the reader uploaded.
//!
//! In each case the picture must be usable: a cover, not taken down, not gone at Open Library,
//! and not with the bytes of a picture a takedown judged. Anything else is 400
//! [`COVER_NOT_AVAILABLE`]. Lock order (conventions §7.2): the media row `FOR SHARE` first, then
//! the entry, so a takedown waits for the choice and its `ingest::clear_references` then clears
//! it, or ran first and the choice finds the picture unusable.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::entries::{NOT_ON_SHELF, ShelfItemOut, item_out};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};
use crate::media::shared::urls::cover_urls;

/// 400: the key names no picture this reader may choose for this book (unknown, taken down,
/// gone, or another book's or reader's). Mapped in apiErrors.ts.
pub const COVER_NOT_AVAILABLE: &str = "This cover cannot be chosen";

/// Longest key accepted (`media:` and 26 characters; `ol:` and a cover id).
const MAX_KEY_CHARS: usize = 64;

/// A personal cover as the loaders put it into a [`BookOut`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalCover {
    pub public_id: String,
    /// `#rrggbb`; `None` when not sampled or without a usable colour.
    pub spine_color: Option<String>,
}

/// The usable personal covers of the entries `entry_ids`, by entry id. Callers pass only the
/// entries the viewer may see (read through `bc_visible_shelf`, or the viewer's own). A removed
/// or gone picture never shows (a takedown also clears the reference).
pub async fn personal_covers(
    conn: &mut PgConnection,
    entry_ids: &[i64],
) -> sqlx::Result<HashMap<i64, PersonalCover>> {
    if entry_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"SELECT s.id, m.public_id, m.dominant_color
           FROM shelf_entries s
           JOIN media m ON m.id = s.cover_media_id
           WHERE s.id = ANY($1) AND m.kind = 'cover' AND m.removed_at IS NULL
             AND m.ol_gone_at IS NULL"#,
        entry_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let cover = PersonalCover {
                public_id: row.public_id,
                spine_color: row.dominant_color.filter(|color| !color.is_empty()),
            };
            (row.id, cover)
        })
        .collect())
}

/// Puts a personal cover into `book`: its URLs, public id and spine colour replace the edition's
/// or the work's. Nothing else of the book changes.
pub fn apply_personal_cover(book: &mut BookOut, cover: &PersonalCover) {
    let (cover_url, cover_thumb_url) = cover_urls(Some(&cover.public_id));
    book.cover_url = cover_url;
    book.cover_thumb_url = cover_thumb_url;
    book.cover_public_id = Some(cover.public_id.clone());
    book.spine_color = cover.spine_color.clone();
}

/// The candidate a reader chooses as their cover.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CoverChoiceIn {
    /// A candidate's `key` from `GET /api/books/works/{work_id}/cover-candidates`: `ol:<cover
    /// id>` or `media:<public id>`.
    pub key: String,
}

impl Validate for CoverChoiceIn {
    fn validate(&mut self, v: &mut Validator) {
        self.key = self.key.trim().to_owned();
        v.check(
            self.key.chars().count() <= MAX_KEY_CHARS,
            COVER_NOT_AVAILABLE,
        );
    }
}

/// A parsed candidate key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverKey {
    /// `ol:<cover_id>`, an Open Library cover id above 0.
    OpenLibrary(i64),
    /// `media:<public_id>`, 26 characters of `[a-z0-9]`.
    Media(String),
}

impl CoverKey {
    /// `None` for anything that is not one of the two forms.
    pub fn parse(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("ol:") {
            let valid = !id.is_empty() && id.len() <= 18 && id.bytes().all(|b| b.is_ascii_digit());
            return valid
                .then(|| id.parse::<i64>().ok())
                .flatten()
                .filter(|&id| id > 0)
                .map(CoverKey::OpenLibrary);
        }
        let id = key.strip_prefix("media:")?;
        let valid = id.len() == 26
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        valid.then(|| CoverKey::Media(id.to_owned()))
    }

    /// The key as the candidate list writes it.
    pub fn format(&self) -> String {
        match self {
            CoverKey::OpenLibrary(id) => format!("ol:{id}"),
            CoverKey::Media(public_id) => format!("media:{public_id}"),
        }
    }
}

/// Chooses a cover for one of the caller's books: a candidate of the cover list. It shows
/// wherever the entry's book shows, also to the friends and club mates who see the shelf.
#[utoipa::path(
    put,
    path = "/api/shelf/{entry_id}/cover",
    operation_id = "shelf_choose_cover",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("entry_id" = i64, Path, description = "The shelf entry")),
    request_body = CoverChoiceIn,
    responses(
        (status = 200, body = ShelfItemOut, description = "The entry with its chosen cover (`personal_cover` true)"),
        (status = 400, body = ErrorBody, description = "`This cover cannot be chosen` (an unknown, taken-down or gone picture, or another book's or reader's), or a non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Book not on your shelf` (also another user's entry)")
    )
)]
pub async fn choose_cover(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(entry_id): ApiPath<i64>,
    ApiJson(body): ApiJson<CoverChoiceIn>,
) -> ApiResult<Json<ShelfItemOut>> {
    let key = CoverKey::parse(&body.key);
    let mut tx = state.db.begin().await?;
    let owned = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM shelf_entries WHERE id = $1 AND user_id = $2) AS "owned!""#,
        entry_id,
        user.id
    )
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        return Err(AppError::not_found(NOT_ON_SHELF));
    }
    let key = key.ok_or_else(|| AppError::bad_request(COVER_NOT_AVAILABLE))?;
    let media_id = lock_choosable(&mut tx, user.id, entry_id, &key)
        .await?
        .ok_or_else(|| AppError::bad_request(COVER_NOT_AVAILABLE))?;
    set_entry_cover(&mut tx, user.id, entry_id, Some(media_id)).await?;
    let item = item_out(&mut tx, user.id, entry_id).await?;
    tx.commit().await?;
    Ok(Json(item))
}

/// Goes back to the standard cover (the edition's, else the work's). A personal photo the
/// entry showed is deleted 7 days later unless it is chosen again (the hourly sweeper).
#[utoipa::path(
    delete,
    path = "/api/shelf/{entry_id}/cover",
    operation_id = "shelf_reset_cover",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("entry_id" = i64, Path, description = "The shelf entry")),
    responses(
        (status = 200, body = ShelfItemOut, description = "The entry with its standard cover (`personal_cover` false)"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Book not on your shelf` (also another user's entry)")
    )
)]
pub async fn reset_cover(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(entry_id): ApiPath<i64>,
) -> ApiResult<Json<ShelfItemOut>> {
    let mut tx = state.db.begin().await?;
    set_entry_cover(&mut tx, user.id, entry_id, None).await?;
    let item = item_out(&mut tx, user.id, entry_id).await?;
    tx.commit().await?;
    Ok(Json(item))
}

/// Writes the entry's choice (`None` = the standard cover); 404 `Book not on your shelf` when
/// it is not the user's entry. `updated_at` stays: the choice is no reading activity.
pub async fn set_entry_cover(
    conn: &mut PgConnection,
    user_id: i64,
    entry_id: i64,
    media_id: Option<i64>,
) -> ApiResult<()> {
    let changed = sqlx::query!(
        "UPDATE shelf_entries SET cover_media_id = $3 WHERE id = $1 AND user_id = $2",
        entry_id,
        user_id,
        media_id
    )
    .execute(conn)
    .await?
    .rows_affected();
    if changed == 1 {
        Ok(())
    } else {
        Err(AppError::not_found(NOT_ON_SHELF))
    }
}

/// The media row `key` names when the user may choose it for the entry, locked `FOR SHARE`
/// until the transaction ends; `None` otherwise.
async fn lock_choosable(
    conn: &mut PgConnection,
    user_id: i64,
    entry_id: i64,
    key: &CoverKey,
) -> sqlx::Result<Option<i64>> {
    let (ol_cover_id, public_id) = match key {
        CoverKey::OpenLibrary(id) => (Some(*id), None),
        CoverKey::Media(public_id) => (None, Some(public_id.as_str())),
    };
    sqlx::query_scalar!(
        r#"SELECT m.id FROM media m
           WHERE (m.ol_cover_id = $1 OR m.public_id = $2)
             AND m.kind = 'cover' AND m.removed_at IS NULL AND m.ol_gone_at IS NULL
             AND NOT EXISTS (SELECT 1 FROM media_removed_hashes h
                             WHERE h.sha256 = m.sha256 AND h.blocks_covers)
             AND (m.source = 'openlibrary'
                  OR (m.personal AND m.uploaded_by = $3)
                  OR m.id IN (SELECT w.cover_media_id FROM shelf_entries s
                              JOIN works w ON w.id = s.work_id WHERE s.id = $4)
                  OR m.id IN (SELECT e.cover_media_id FROM shelf_entries s
                              JOIN editions e ON e.work_id = s.work_id WHERE s.id = $4))
           FOR SHARE OF m"#,
        ol_cover_id,
        public_id,
        user_id,
        entry_id
    )
    .fetch_optional(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_in_their_two_forms_only() {
        assert_eq!(CoverKey::parse("ol:123"), Some(CoverKey::OpenLibrary(123)));
        let public_id = "a".repeat(26);
        assert_eq!(
            CoverKey::parse(&format!("media:{public_id}")),
            Some(CoverKey::Media(public_id.clone()))
        );
        for bad in [
            "",
            "ol:",
            "ol:0",
            "ol:-1",
            "ol:12a",
            "ol: 12",
            "ol:99999999999999999999",
            "media:",
            "media:ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "media:aaaa",
            "isbn:9783462050493",
            "https://covers.openlibrary.org/b/id/1-L.jpg",
        ] {
            assert_eq!(CoverKey::parse(bad), None, "{bad}");
        }
        assert_eq!(CoverKey::OpenLibrary(7).format(), "ol:7");
        assert_eq!(
            CoverKey::Media(public_id.clone()).format(),
            format!("media:{public_id}")
        );
    }
}
