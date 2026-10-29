//! The response types of the club routes and their loaders (endpoint-map §3).

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::extract::Role;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::config::Config;
use crate::dto::BookOut;
use crate::error::{ApiResult, AppError, msg};
use crate::media::shared::urls::avatar_url;

/// How many currently-reading books a member card previews.
const PREVIEW_BOOKS: i64 = 3;

/// A club the caller belongs to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ClubOut {
    /// The immutable URL key: `/c/{slug}`, `/api/clubs/{slug}`.
    pub slug: String,
    pub name: String,
    /// IANA zone for the club's times.
    pub timezone: String,
    /// The caller's role.
    pub role: Role,
    pub member_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// The current pick; null when there is none.
    pub current_pick: Option<ClubPickSummary>,
}

/// The current pick on a club card.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ClubPickSummary {
    pub id: i64,
    pub book: BookOut,
    #[serde(with = "time::serde::rfc3339::option")]
    pub meeting_at: Option<OffsetDateTime>,
}

/// The caller's clubs, newest join first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClubList {
    pub items: Vec<ClubOut>,
}

/// One member of a club, as the member list shows them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberOut {
    pub username: String,
    pub role: Role,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
    /// This member is the caller.
    pub you: bool,
    /// `/media/{public_id}/a`.
    pub avatar_url: Option<String>,
    pub currently_reading_count: i64,
    /// At most three, in shelf order.
    pub currently_reading_preview: Vec<BookOut>,
}

/// The members of a club: the owner, the admins, the members, each by username.
#[derive(Debug, Serialize, ToSchema)]
pub struct MemberList {
    pub items: Vec<MemberOut>,
}

/// A club invite, or a sign-up invite minted by an instance admin.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteOut {
    pub id: i64,
    /// 10 Crockford base32 characters (legacy codes may differ).
    pub code: String,
    /// `{PUBLIC_URL}/join/{code}`.
    pub url: String,
    /// The username of whoever created it; null when that account is gone or the CLI made it.
    pub created_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// Null for a link without a use limit.
    pub max_uses: Option<i32>,
    pub use_count: i32,
    /// Who redeemed it, oldest first, one entry per use (deleted accounts are left out).
    pub used_by: Vec<String>,
    pub revoked: bool,
    /// Not revoked, not expired and not used up: it can still be redeemed.
    pub active: bool,
    /// A friend invite (`kind: "friend"`): only the friend it was made for can redeem it.
    pub personal: bool,
    /// The username of the friend a personal invite is for, shown only to the invite's creator
    /// (another admin would learn whom the creator is friends with); null otherwise.
    pub invitee: Option<String>,
}

/// Invites, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct InviteList {
    pub items: Vec<InviteOut>,
}

/// The join link of an invite code.
pub fn invite_url(config: &Config, code: &str) -> String {
    format!("{}/join/{code}", config.public_url)
}

/// The clubs of `user_id` as [`ClubOut`], newest join first; with `only`, just that club (empty
/// when the user is not a member of it).
pub async fn club_outs(
    conn: &mut PgConnection,
    user_id: i64,
    only: Option<i64>,
) -> sqlx::Result<Vec<ClubOut>> {
    let rows = sqlx::query!(
        r#"SELECT c.slug, c.name, c.timezone, c.created_at, m.role AS "role: Role",
                  (SELECT count(*) FROM club_memberships x WHERE x.club_id = c.id) AS "member_count!",
                  p.id AS "pick_id?", p.work_id AS "pick_work_id?", p.meeting_at AS "meeting_at?"
           FROM club_memberships m
           JOIN clubs c ON c.id = m.club_id
           LEFT JOIN club_picks p ON p.club_id = c.id AND p.ended_at IS NULL
           WHERE m.user_id = $1 AND ($2::int8 IS NULL OR c.id = $2)
           ORDER BY m.joined_at DESC, c.id DESC"#,
        user_id,
        only
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = rows
        .iter()
        .filter_map(|row| row.pick_work_id.map(BookRef::work))
        .collect();
    let books = book_outs(conn, &refs).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let current_pick = match (row.pick_id, row.pick_work_id) {
                (Some(id), Some(work_id)) => {
                    books
                        .get(&BookRef::work(work_id))
                        .map(|book| ClubPickSummary {
                            id,
                            book: book.clone(),
                            meeting_at: row.meeting_at,
                        })
                }
                _ => None,
            };
            ClubOut {
                slug: row.slug,
                name: row.name,
                timezone: row.timezone,
                role: row.role,
                member_count: row.member_count,
                created_at: row.created_at,
                current_pick,
            }
        })
        .collect())
}

/// One club of `user_id` as [`ClubOut`]; 404 `No such club` when the user is not a member.
pub async fn club_out(conn: &mut PgConnection, user_id: i64, club_id: i64) -> ApiResult<ClubOut> {
    club_outs(conn, user_id, Some(club_id))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_CLUB))
}

/// The members of `club_id` as `viewer` sees them, ordered owner, admins, members, then
/// username (byte order); with `only`, just that member.
///
/// The currently-reading counts and previews read other members' shelves, so they go through
/// `bc_visible_shelf` (conventions §6.2), restricted to this club's members: one query for every
/// member, numbered with `row_number()`.
pub async fn member_outs(
    conn: &mut PgConnection,
    club_id: i64,
    viewer: i64,
    only: Option<i64>,
) -> sqlx::Result<Vec<MemberOut>> {
    let members = sqlx::query!(
        r#"SELECT u.id, u.username, m.role AS "role: Role", m.joined_at,
                  med.public_id AS "avatar_public_id?"
           FROM club_memberships m
           JOIN users u ON u.id = m.user_id
           LEFT JOIN media med ON med.id = u.avatar_media_id AND med.removed_at IS NULL
           WHERE m.club_id = $1 AND ($2::int8 IS NULL OR m.user_id = $2)
           ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, u.username"#,
        club_id,
        only
    )
    .fetch_all(&mut *conn)
    .await?;
    let reading = sqlx::query!(
        r#"SELECT r.user_id AS "user_id!", r.work_id AS "work_id!", r.edition_id,
                  r.total AS "total!"
           FROM (
               SELECT s.user_id, s.work_id, s.edition_id,
                      row_number() OVER (PARTITION BY s.user_id ORDER BY s.position, s.id) AS rn,
                      count(*) OVER (PARTITION BY s.user_id) AS total
               FROM bc_visible_shelf($2) s
               JOIN club_memberships m ON m.club_id = $1 AND m.user_id = s.user_id
               WHERE s.status = 'currently_reading' AND ($3::int8 IS NULL OR s.user_id = $3)
           ) r
           WHERE r.rn <= $4
           ORDER BY r.user_id, r.rn"#,
        club_id,
        viewer,
        only,
        PREVIEW_BOOKS
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = reading
        .iter()
        .map(|row| BookRef {
            work_id: row.work_id,
            edition_id: row.edition_id,
        })
        .collect();
    let books = book_outs(conn, &refs).await?;
    let mut previews: HashMap<i64, (i64, Vec<BookOut>)> = HashMap::new();
    for row in &reading {
        let entry = previews
            .entry(row.user_id)
            .or_insert_with(|| (row.total, Vec::new()));
        let key = BookRef {
            work_id: row.work_id,
            edition_id: row.edition_id,
        };
        if let Some(book) = books.get(&key) {
            entry.1.push(book.clone());
        }
    }
    Ok(members
        .into_iter()
        .map(|row| {
            let (count, preview) = previews.remove(&row.id).unwrap_or_default();
            MemberOut {
                username: row.username,
                role: row.role,
                joined_at: row.joined_at,
                you: row.id == viewer,
                avatar_url: avatar_url(row.avatar_public_id.as_deref()),
                currently_reading_count: count,
                currently_reading_preview: preview,
            }
        })
        .collect())
}

/// Invites as [`InviteOut`], newest first (`created_at DESC, id DESC`): the club's invites
/// for `Some(club_id)`, the sign-up invites for `None`; with `only`, just that invite. For the
/// sign-up invites of the instance admins, which are never personal: [`club_invite_outs`] names
/// a personal invite's friend to its creator.
pub async fn invite_outs(
    conn: &mut PgConnection,
    config: &Config,
    club_id: Option<i64>,
    only: Option<i64>,
) -> sqlx::Result<Vec<InviteOut>> {
    club_invite_outs(conn, config, None, club_id, only).await
}

/// [`invite_outs`] as `viewer` sees them: `invitee` is filled for the personal invites `viewer`
/// created, and null for everyone else's.
pub async fn club_invite_outs(
    conn: &mut PgConnection,
    config: &Config,
    viewer: Option<i64>,
    club_id: Option<i64>,
    only: Option<i64>,
) -> sqlx::Result<Vec<InviteOut>> {
    let rows = sqlx::query!(
        r#"SELECT i.id, i.code, u.username AS "created_by?", i.created_at, i.expires_at,
                  i.max_uses, i.use_count, i.revoked_at IS NOT NULL AS "revoked!",
                  (i.revoked_at IS NULL AND (i.expires_at IS NULL OR i.expires_at > now())
                   AND (i.max_uses IS NULL OR i.use_count < i.max_uses)) AS "active!",
                  ARRAY(SELECT v.username
                        FROM club_invite_uses x JOIN users v ON v.id = x.user_id
                        WHERE x.invite_id = i.id
                        ORDER BY x.used_at, x.id) AS "used_by!",
                  i.invitee_id IS NOT NULL AS "personal!",
                  CASE WHEN i.created_by = $3 THEN f.username END AS "invitee?"
           FROM club_invites i
           LEFT JOIN users u ON u.id = i.created_by
           LEFT JOIN users f ON f.id = i.invitee_id
           WHERE i.club_id IS NOT DISTINCT FROM $1 AND ($2::int8 IS NULL OR i.id = $2)
           ORDER BY i.created_at DESC, i.id DESC"#,
        club_id,
        only,
        viewer
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| InviteOut {
            id: row.id,
            url: invite_url(config, &row.code),
            code: row.code,
            created_by: row.created_by,
            created_at: row.created_at,
            expires_at: row.expires_at,
            max_uses: row.max_uses,
            use_count: row.use_count,
            used_by: row.used_by,
            revoked: row.revoked,
            active: row.active,
            personal: row.personal,
            invitee: row.invitee,
        })
        .collect())
}

/// One invite as [`InviteOut`] (see [`invite_outs`]).
pub async fn invite_out(
    conn: &mut PgConnection,
    config: &Config,
    club_id: Option<i64>,
    invite_id: i64,
) -> ApiResult<InviteOut> {
    club_invite_out(conn, config, None, club_id, invite_id).await
}

/// One invite as `viewer` sees it (see [`club_invite_outs`]).
pub async fn club_invite_out(
    conn: &mut PgConnection,
    config: &Config,
    viewer: Option<i64>,
    club_id: Option<i64>,
    invite_id: i64,
) -> ApiResult<InviteOut> {
    club_invite_outs(conn, config, viewer, club_id, Some(invite_id))
        .await?
        .pop()
        .ok_or_else(|| AppError::internal(format!("invite {invite_id} vanished")))
}
