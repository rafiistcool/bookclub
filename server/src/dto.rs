//! The shared DTOs of endpoint-map §0 (docs/phase1/conventions.md §20). Frozen: every type has
//! exactly one definition, here; packages import them and never define a same-named schema.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::ToSchema;

use crate::clubs::extract::Role;
use crate::media::shared::urls::avatar_url;

/// A shelf entry's status. Stored as text; sorts alphabetically, as the legacy app did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ShelfStatus {
    WantToRead,
    CurrentlyReading,
    Finished,
    DidNotFinish,
}

/// Who may see a personal item: the owner's audience, or the owner only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Audience,
    OnlyMe,
}

/// Where a catalog row came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CatalogSource {
    Openlibrary,
    Dnb,
    User,
    Legacy,
}

/// One work, edition-aware: with an edition context, the edition's own title, pages, cover and
/// spine colour win.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BookOut {
    /// The work id.
    pub id: i64,
    /// `"/works/OL…W"` for Open Library works, else null.
    pub ol_work_key: Option<String>,
    pub title: String,
    pub authors: String,
    pub year: Option<i32>,
    /// Same-origin `/media/{public_id}/l`.
    pub cover_url: Option<String>,
    /// `/media/{public_id}/s`.
    pub cover_thumb_url: Option<String>,
    /// The media public id behind the cover URLs (the key for "Report cover").
    pub cover_public_id: Option<String>,
    pub pages: Option<i32>,
    pub spine_color: Option<String>,
    pub source: CatalogSource,
    /// `source == "user"`: the "Added by a reader" label.
    pub custom: bool,
}

/// A translator, illustrator or editor of an edition.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContributorOut {
    pub name: String,
    /// `"translator"`, `"illustrator"` or `"editor"`.
    pub role: String,
}

/// One edition of a work.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EditionOut {
    pub id: i64,
    pub work_id: i64,
    pub isbn13: Option<String>,
    /// Derived from `isbn13`; null for 979 ISBNs.
    pub isbn10: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub publisher: String,
    pub language: Option<String>,
    pub format: Option<String>,
    pub pages: Option<i32>,
    pub publication_year: Option<i32>,
    /// `YYYY-MM-DD`.
    pub planned_publication_date: Option<time::Date>,
    pub contributors: Vec<ContributorOut>,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    pub source: CatalogSource,
}

/// One search, trending, subject or new-releases hit.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SearchHit {
    pub id: i64,
    pub ol_work_key: Option<String>,
    pub title: String,
    pub authors: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub cover_thumb_url: Option<String>,
    pub cover_public_id: Option<String>,
    pub isbn: Option<String>,
    pub source: CatalogSource,
    pub custom: bool,
    pub on_shelf: bool,
    pub shelf_id: Option<i64>,
    /// The viewer's clubs whose current pick is this work.
    pub club_picks: Vec<ClubRef>,
    /// Set on new-releases hits (HAR), null elsewhere. `YYYY-MM-DD`.
    pub planned_publication_date: Option<time::Date>,
}

/// A page of search hits.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SearchPage {
    pub items: Vec<SearchHit>,
    pub page: i32,
    pub has_more: bool,
}

/// The signed-in user's own account.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserOut {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    /// An email change waiting for confirmation: the address of the newest open verification
    /// link, only while the account's own address is verified; else null.
    pub pending_email: Option<String>,
    pub needs_email_prompt: bool,
    pub theme: String,
    pub color_mode: String,
    pub locale: String,
    pub timezone: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub clubs: Vec<ClubSummary>,
}

/// Anyone else, and the owner block of a shelf.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: i64,
    pub username: String,
    /// `/media/{public_id}/a`.
    pub avatar_url: Option<String>,
}

/// Row form for loaders (not a schema): `visibility::visible_user`, [`user_summaries`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSummaryRow {
    pub id: i64,
    pub username: String,
    pub avatar_public_id: Option<String>,
}

impl From<UserSummaryRow> for UserSummary {
    fn from(row: UserSummaryRow) -> Self {
        UserSummary {
            id: row.id,
            avatar_url: avatar_url(row.avatar_public_id.as_deref()),
            username: row.username,
        }
    }
}

/// A club the viewer belongs to, by slug and name.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ClubRef {
    pub slug: String,
    pub name: String,
}

/// A club in `UserOut.clubs`.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ClubSummary {
    pub slug: String,
    pub name: String,
    pub role: Role,
    pub timezone: String,
    pub member_count: i64,
}

/// One emoji's reactions on a post.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReactionOut {
    pub emoji: String,
    /// Everyone who reacted.
    pub count: i64,
    pub mine: bool,
    /// Only the names the viewer may see.
    pub users: Vec<String>,
}

/// [`UserSummary`] for each existing id. A removed avatar has no URL.
///
/// Checks nothing: it names any user id it is given. Callers pass only ids the viewer may see
/// (conventions §6.3: reaction users, parent authors and reader lists come from
/// `bc_visible_owners` or the club's members, never from the row alone).
pub async fn user_summaries(
    conn: &mut PgConnection,
    ids: &[i64],
) -> sqlx::Result<HashMap<i64, UserSummary>> {
    let rows = sqlx::query_as!(
        UserSummaryRow,
        r#"SELECT u.id, u.username, m.public_id AS "avatar_public_id?"
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.id = ANY($1)"#,
        ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.into())).collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use time::macros::date;

    use super::*;

    #[test]
    fn enums_serialise_as_snake_case() {
        assert_eq!(
            serde_json::to_value(ShelfStatus::DidNotFinish).unwrap(),
            json!("did_not_finish")
        );
        assert_eq!(
            serde_json::to_value(Visibility::OnlyMe).unwrap(),
            json!("only_me")
        );
        assert_eq!(
            serde_json::to_value(CatalogSource::Openlibrary).unwrap(),
            json!("openlibrary")
        );
        let status: ShelfStatus = serde_json::from_value(json!("currently_reading")).unwrap();
        assert_eq!(status, ShelfStatus::CurrentlyReading);
    }

    #[test]
    fn dates_serialise_as_iso_dates() {
        let edition = EditionOut {
            id: 1,
            work_id: 2,
            isbn13: Some("9783161484100".into()),
            isbn10: Some("3161484100".into()),
            title: None,
            subtitle: None,
            publisher: String::new(),
            language: Some("ger".into()),
            format: Some("hardcover".into()),
            pages: None,
            publication_year: Some(2026),
            planned_publication_date: Some(date!(2026 - 09 - 01)),
            contributors: vec![],
            cover_url: None,
            cover_thumb_url: None,
            cover_public_id: None,
            source: CatalogSource::Dnb,
        };
        let value = serde_json::to_value(edition).unwrap();
        assert_eq!(value["planned_publication_date"], "2026-09-01");
        assert_eq!(value["title"], serde_json::Value::Null);
        assert_eq!(value["source"], "dnb");
    }

    #[test]
    fn summary_rows_get_avatar_urls() {
        let summary: UserSummary = UserSummaryRow {
            id: 3,
            username: "ada".into(),
            avatar_public_id: Some("abcdefghijklmnopqrstuvwxyz".into()),
        }
        .into();
        assert_eq!(
            summary.avatar_url.as_deref(),
            Some("/media/abcdefghijklmnopqrstuvwxyz/a")
        );
        let summary: UserSummary = UserSummaryRow {
            id: 4,
            username: "bob".into(),
            avatar_public_id: None,
        }
        .into();
        assert_eq!(summary.avatar_url, None);
    }
}
