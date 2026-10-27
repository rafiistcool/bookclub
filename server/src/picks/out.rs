//! `ClubPickOut` and its loader (docs/phase1/spec/club.md §2.5, endpoint-map §9).

use std::collections::HashMap;

use jiff::tz::TimeZone;
use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::catalog::shared::books::{BookRef, book_outs};
use crate::dto::{BookOut, ShelfStatus};
use crate::error::{ApiResult, AppError};
use crate::media::shared::urls::avatar_url;
use crate::tz;

/// A club pick as the caller sees it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ClubPickOut {
    pub id: i64,
    /// The work, without an edition context (members may read different editions).
    pub book: BookOut,
    /// The username of whoever set the pick; null when that account is gone.
    pub set_by: Option<String>,
    pub note: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// Null for the current pick.
    #[serde(with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub meeting_at: Option<OffsetDateTime>,
    /// The meeting in the club's zone, `YYYY-MM-DDTHH:MM` (a `datetime-local` value).
    pub meeting_local: Option<String>,
    /// The meeting in the club's zone, e.g. `Sat 22 Aug 2026, 19:00 CEST` (always English).
    pub meeting_label: Option<String>,
    /// The caller's own shelf status for the book (also on ended picks).
    pub on_shelf: Option<ShelfStatus>,
    /// The caller's own shelf entry for the book.
    pub shelf_id: Option<i64>,
    /// Club members with the book on their shelf, the caller first, then by username; empty on
    /// an ended pick. Ratings and takes marked "only me" are hidden from everyone else.
    pub readers: Vec<ClubPickReader>,
    /// The readers currently reading it, in reader order.
    pub reading: Vec<String>,
    /// The readers who finished it, in reader order.
    pub finished: Vec<String>,
}

/// A club member who has the pick on their shelf.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ClubPickReader {
    pub username: String,
    pub status: ShelfStatus,
    /// Null when unrated, or when the rating is "only me" and not the caller's.
    pub rating: Option<i32>,
    /// Empty when there is none, or when it is "only me" and not the caller's.
    pub take: String,
    pub dnf_reason: String,
    pub progress: Option<i32>,
    /// `/media/{public_id}/a`.
    pub avatar_url: Option<String>,
}

/// The current pick (`null` when there is none) and the club's zone.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClubPickCurrentOut {
    pub pick: Option<ClubPickOut>,
    /// The club's IANA zone, in which every label is given.
    pub timezone: String,
}

/// Ended picks, the most recently ended first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClubPickHistoryOut {
    pub items: Vec<ClubPickOut>,
    pub timezone: String,
}

/// A `club_picks` row with its setter's username.
#[derive(Debug, Clone)]
pub struct PickRow {
    pub id: i64,
    pub work_id: i64,
    pub set_by: Option<String>,
    pub note: String,
    pub started_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    pub meeting_at: Option<OffsetDateTime>,
}

/// The club's current pick, if any.
pub async fn current_row(conn: &mut PgConnection, club_id: i64) -> sqlx::Result<Option<PickRow>> {
    sqlx::query_as!(
        PickRow,
        r#"SELECT p.id, p.work_id, u.username AS "set_by?", p.note, p.started_at, p.ended_at,
                  p.meeting_at
           FROM club_picks p
           LEFT JOIN users u ON u.id = p.set_by
           WHERE p.club_id = $1 AND p.ended_at IS NULL"#,
        club_id
    )
    .fetch_optional(conn)
    .await
}

/// The pick `pick_id` of the club; `None` for a missing pick or another club's.
pub async fn pick_row(
    conn: &mut PgConnection,
    club_id: i64,
    pick_id: i64,
) -> sqlx::Result<Option<PickRow>> {
    sqlx::query_as!(
        PickRow,
        r#"SELECT p.id, p.work_id, u.username AS "set_by?", p.note, p.started_at, p.ended_at,
                  p.meeting_at
           FROM club_picks p
           LEFT JOIN users u ON u.id = p.set_by
           WHERE p.club_id = $1 AND p.id = $2"#,
        club_id,
        pick_id
    )
    .fetch_optional(conn)
    .await
}

/// The club's ended picks, `ended_at DESC, id DESC`.
pub async fn history_rows(conn: &mut PgConnection, club_id: i64) -> sqlx::Result<Vec<PickRow>> {
    sqlx::query_as!(
        PickRow,
        r#"SELECT p.id, p.work_id, u.username AS "set_by?", p.note, p.started_at, p.ended_at,
                  p.meeting_at
           FROM club_picks p
           LEFT JOIN users u ON u.id = p.set_by
           WHERE p.club_id = $1 AND p.ended_at IS NOT NULL
           ORDER BY p.ended_at DESC, p.id DESC"#,
        club_id
    )
    .fetch_all(conn)
    .await
}

/// [`ClubPickOut`] for each row of `club_id`, in order, as `viewer` (a member) sees it, with the
/// labels in `zone`.
///
/// Readers are loaded for a current pick only: the shelf entries of the club's members for the
/// work, read through `bc_visible_shelf(viewer)`, so "only me" ratings and takes of others are
/// already redacted (conventions §6.2).
pub async fn pick_outs(
    conn: &mut PgConnection,
    club_id: i64,
    viewer: i64,
    zone: &TimeZone,
    rows: Vec<PickRow>,
) -> ApiResult<Vec<ClubPickOut>> {
    let refs: Vec<BookRef> = rows.iter().map(|row| BookRef::work(row.work_id)).collect();
    let books = book_outs(conn, &refs).await?;
    let work_ids: Vec<i64> = rows.iter().map(|row| row.work_id).collect();
    let own: HashMap<i64, (i64, ShelfStatus)> = sqlx::query!(
        r#"SELECT id, work_id, status AS "status: ShelfStatus"
           FROM shelf_entries
           WHERE user_id = $1 AND work_id = ANY($2)"#,
        viewer,
        &work_ids
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.work_id, (row.id, row.status)))
    .collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let book = books
            .get(&BookRef::work(row.work_id))
            .cloned()
            .ok_or_else(|| AppError::internal(format!("pick {} is missing its book", row.id)))?;
        let readers = match row.ended_at {
            None => readers(conn, club_id, viewer, row.work_id).await?,
            Some(_) => Vec::new(),
        };
        let names = |status: ShelfStatus| -> Vec<String> {
            readers
                .iter()
                .filter(|reader| reader.status == status)
                .map(|reader| reader.username.clone())
                .collect()
        };
        let reading = names(ShelfStatus::CurrentlyReading);
        let finished = names(ShelfStatus::Finished);
        let (shelf_id, on_shelf) = match own.get(&row.work_id) {
            Some((id, status)) => (Some(*id), Some(*status)),
            None => (None, None),
        };
        out.push(ClubPickOut {
            id: row.id,
            book,
            set_by: row.set_by,
            note: row.note,
            started_at: row.started_at,
            ended_at: row.ended_at,
            meeting_at: row.meeting_at,
            meeting_local: row.meeting_at.map(|at| tz::local_minute(at, zone)),
            meeting_label: row.meeting_at.map(|at| tz::label(at, zone)),
            on_shelf,
            shelf_id,
            readers,
            reading,
            finished,
        });
    }
    Ok(out)
}

/// The club members with `work_id` on their shelf, in any status: the viewer first, then by
/// username (byte order).
async fn readers(
    conn: &mut PgConnection,
    club_id: i64,
    viewer: i64,
    work_id: i64,
) -> sqlx::Result<Vec<ClubPickReader>> {
    let rows = sqlx::query!(
        r#"SELECT s.user_id AS "user_id!", u.username, s.status AS "status!: ShelfStatus",
                  s.rating, s.take AS "take!", s.dnf_reason AS "dnf_reason!", s.progress,
                  med.public_id AS "avatar_public_id?"
           FROM bc_visible_shelf($1) s
           JOIN club_memberships m ON m.club_id = $2 AND m.user_id = s.user_id
           JOIN users u ON u.id = s.user_id
           LEFT JOIN media med ON med.id = u.avatar_media_id AND med.removed_at IS NULL
           WHERE s.work_id = $3"#,
        viewer,
        club_id,
        work_id
    )
    .fetch_all(conn)
    .await?;
    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.sort_by(|a, b| {
        (a.user_id != viewer, a.username.as_bytes())
            .cmp(&(b.user_id != viewer, b.username.as_bytes()))
    });
    Ok(rows
        .into_iter()
        .map(|row| ClubPickReader {
            username: row.username,
            status: row.status,
            rating: row.rating,
            take: row.take,
            dnf_reason: row.dnf_reason,
            progress: row.progress,
            avatar_url: avatar_url(row.avatar_public_id.as_deref()),
        })
        .collect())
}
