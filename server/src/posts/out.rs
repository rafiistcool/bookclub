//! `PostOut` and the loaders behind every post endpoint (endpoint-map §8, conventions §6.3,
//! docs/phase1/spec/diary.md §1.0 and R2, R9–R12).
//!
//! Every loader reads through the policy functions, so a caller can hand it any ids:
//! - posts through `bc_visible_posts` (a post the viewer may not see is left out),
//! - the live `author_rating` through `bc_visible_shelf` (an `only_me` rating is null for
//!   everyone but its owner),
//! - reaction names through `bc_visible_owners` (`count` counts everyone, `users` names only the
//!   viewer and their audience, which includes the members of the viewer's clubs).

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::auth::extract::CurrentUser;
use crate::dto::{ClubRef, ReactionOut, ShelfStatus, Visibility};
use crate::error::{ApiResult, AppError, msg};
use crate::media::shared::urls::avatar_url;
use crate::tz;

/// One post (a diary entry or a club post) as the viewer sees it: the legacy `DiaryEntryOut`
/// plus the scope fields.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostOut {
    pub id: i64,
    /// The author's username; null only for a tombstone left by a deleted account.
    pub author: Option<String>,
    /// `/media/{public_id}/a`; null without a picture, once it was taken down, or when the viewer
    /// shares no club with the author any more.
    pub author_avatar_url: Option<String>,
    /// Whether the viewer wrote it.
    pub mine: bool,
    /// Empty for a tombstone.
    pub body: String,
    /// A tombstone: deleted by its author, kept because it has replies.
    pub deleted: bool,
    /// Diary roots only; replies and club posts are always `audience`.
    pub visibility: Visibility,
    /// The percentage of the book the post is safe up to; null for no spoiler flag and for a
    /// tombstone.
    pub spoiler_upto: Option<i32>,
    /// The author's progress when writing (100 when finished).
    pub progress_at: Option<i32>,
    /// The author's shelf status when writing.
    pub status_at: Option<ShelfStatus>,
    /// The author's current rating of the work while they have it as finished; null when the
    /// rating is `only_me` and the viewer is not the author.
    pub author_rating: Option<i32>,
    pub parent_id: Option<i64>,
    /// The club of a club post; null for a diary entry.
    pub club: Option<ClubRef>,
    /// The pick a club post belongs to.
    pub pick_id: Option<i64>,
    /// The milestone a club post is about.
    pub milestone_id: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// `created_at` in the club's zone for a club post, in the viewer's zone for a diary entry,
    /// e.g. `Tue 27 Oct 2026, 10:58 CET`.
    pub created_label: String,
    /// Changed after writing (never for a tombstone).
    pub edited: bool,
    /// Grouped by emoji, the most used first; empty for a tombstone.
    pub reactions: Vec<ReactionOut>,
    /// The visible replies, oldest first, where the endpoint includes them; always empty for a
    /// reply.
    #[schema(no_recursion)]
    pub replies: Vec<PostOut>,
}

/// Where a reader is in a book: `(progress, status)` (legacy `_position`). Finished is 100,
/// reading is the progress (possibly null), want-to-read and DNF have no progress.
pub fn position(
    status: Option<ShelfStatus>,
    progress: Option<i32>,
) -> (Option<i32>, Option<ShelfStatus>) {
    match status {
        None => (None, None),
        Some(ShelfStatus::Finished) => (Some(100), status),
        Some(ShelfStatus::CurrentlyReading) => (progress, status),
        Some(ShelfStatus::WantToRead | ShelfStatus::DidNotFinish) => (None, status),
    }
}

/// The user's own position in each of these works (their own shelf; works they do not have are
/// left out).
pub async fn positions(
    conn: &mut PgConnection,
    user_id: i64,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, (Option<i32>, Option<ShelfStatus>)>> {
    let rows = sqlx::query!(
        r#"SELECT work_id, status AS "status: ShelfStatus", progress
           FROM shelf_entries
           WHERE user_id = $1 AND work_id = ANY($2)"#,
        user_id,
        work_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.work_id, position(Some(row.status), row.progress)))
        .collect())
}

/// The user's own position in one work.
pub async fn my_position(
    conn: &mut PgConnection,
    user_id: i64,
    work_id: i64,
) -> sqlx::Result<(Option<i32>, Option<ShelfStatus>)> {
    Ok(positions(conn, user_id, &[work_id])
        .await?
        .remove(&work_id)
        .unwrap_or((None, None)))
}

/// A post row as the viewer may see it.
struct PostRow {
    id: i64,
    author_id: Option<i64>,
    author: Option<String>,
    author_avatar: Option<String>,
    parent_id: Option<i64>,
    club_id: Option<i64>,
    club_slug: Option<String>,
    club_name: Option<String>,
    club_timezone: Option<String>,
    pick_id: Option<i64>,
    milestone_id: Option<i64>,
    visibility: Visibility,
    body: String,
    spoiler_upto: Option<i32>,
    progress_at: Option<i32>,
    status_at: Option<ShelfStatus>,
    deleted: bool,
    created_at: OffsetDateTime,
    updated_at: Option<OffsetDateTime>,
    author_rating: Option<i32>,
}

/// The rows of the posts among `ids` the viewer may see. The author's profile picture only when
/// the viewer can see the author (`bc_can_see_user`: they share a club), as the report route
/// accepts it: a member removed from the club, or who left it, keeps their name on their old
/// posts, but a picture they set since would show to people who could not report it (security
/// review, ADM2 merge). Reactions hide the names of such people the same way.
async fn load_rows(
    conn: &mut PgConnection,
    viewer: i64,
    ids: &[i64],
) -> sqlx::Result<HashMap<i64, PostRow>> {
    let rows = sqlx::query_as!(
        PostRow,
        r#"SELECT p.id, p.author_id, u.username AS "author?",
                  CASE WHEN bc_can_see_user($1, p.author_id) THEN m.public_id END
                      AS "author_avatar?",
                  p.parent_id, p.club_id, c.slug AS "club_slug?", c.name AS "club_name?",
                  c.timezone AS "club_timezone?", p.pick_id, p.milestone_id,
                  p.visibility AS "visibility: Visibility", p.body, p.spoiler_upto,
                  p.progress_at, p.status_at AS "status_at: ShelfStatus",
                  p.deleted_at IS NOT NULL AS "deleted!", p.created_at, p.updated_at,
                  (SELECT s.rating
                   FROM bc_visible_shelf($1) AS s
                   WHERE s.user_id = p.author_id AND s.work_id = p.work_id
                     AND s.status = 'finished') AS author_rating
           FROM posts p
           JOIN bc_visible_posts($1) AS v ON v.id = p.id
           LEFT JOIN users u ON u.id = p.author_id
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           LEFT JOIN clubs c ON c.id = p.club_id
           WHERE p.id = ANY($2)"#,
        viewer,
        ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// One reaction: who reacted, and their name only when the viewer may see them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReactionRow {
    post_id: i64,
    emoji: String,
    user_id: i64,
    username: Option<String>,
}

/// The reactions on these posts, grouped per post.
async fn load_reactions(
    conn: &mut PgConnection,
    viewer: i64,
    ids: &[i64],
) -> sqlx::Result<HashMap<i64, Vec<ReactionOut>>> {
    let rows = sqlx::query_as!(
        ReactionRow,
        r#"SELECT r.post_id AS "post_id!", r.emoji AS "emoji!", r.user_id AS "user_id!",
                  CASE WHEN o.user_id IS NOT NULL THEN u.username END AS "username?"
           FROM post_reactions r
           JOIN users u ON u.id = r.user_id
           LEFT JOIN bc_visible_owners($1) AS o ON o.user_id = r.user_id
           WHERE r.post_id = ANY($2)"#,
        viewer,
        ids
    )
    .fetch_all(conn)
    .await?;
    let mut per_post: HashMap<i64, Vec<ReactionRow>> = HashMap::new();
    for row in rows {
        per_post.entry(row.post_id).or_default().push(row);
    }
    Ok(per_post
        .into_iter()
        .map(|(post_id, rows)| (post_id, aggregate(rows, viewer)))
        .collect())
}

/// Groups reactions by emoji (legacy `_reactions_out`): `count` counts everyone, `mine` is
/// exact, `users` holds the visible names in byte order; the groups are sorted by count
/// (descending), then emoji (code points).
fn aggregate(rows: Vec<ReactionRow>, viewer: i64) -> Vec<ReactionOut> {
    let mut groups: HashMap<String, ReactionOut> = HashMap::new();
    for row in rows {
        let group = groups
            .entry(row.emoji.clone())
            .or_insert_with(|| ReactionOut {
                emoji: row.emoji,
                count: 0,
                mine: false,
                users: Vec::new(),
            });
        group.count += 1;
        group.mine |= row.user_id == viewer;
        group.users.extend(row.username);
    }
    let mut out: Vec<ReactionOut> = groups.into_values().collect();
    for group in &mut out {
        group.users.sort();
    }
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.emoji.cmp(&b.emoji)));
    out
}

/// The visible replies to these roots, oldest first: `(reply id, root id)`.
async fn reply_ids(
    conn: &mut PgConnection,
    viewer: i64,
    root_ids: &[i64],
) -> sqlx::Result<Vec<(i64, i64)>> {
    let rows = sqlx::query!(
        r#"SELECT p.id, p.parent_id AS "parent_id!"
           FROM posts p
           JOIN bc_visible_posts($1) AS v ON v.id = p.id
           WHERE p.parent_id = ANY($2)
           ORDER BY p.created_at, p.id"#,
        viewer,
        root_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.parent_id))
        .collect())
}

/// The posts `ids` as the viewer sees them, in that order; posts the viewer may not see are
/// left out. With `with_replies`, each root carries its visible replies (oldest first);
/// otherwise every `replies` is empty (the feed).
pub async fn post_outs(
    conn: &mut PgConnection,
    viewer: &CurrentUser,
    ids: &[i64],
    with_replies: bool,
) -> sqlx::Result<Vec<PostOut>> {
    let replies = if with_replies {
        reply_ids(conn, viewer.id, ids).await?
    } else {
        Vec::new()
    };
    let mut all: Vec<i64> = ids.to_vec();
    all.extend(replies.iter().map(|(id, _)| *id));
    let mut rows = load_rows(conn, viewer.id, &all).await?;
    let mut reactions = load_reactions(conn, viewer.id, &all).await?;
    let zone = tz::zone_or_utc(&viewer.timezone);
    let mut build = |id: i64| -> Option<PostOut> {
        let row = rows.remove(&id)?;
        let reactions = if row.deleted {
            Vec::new()
        } else {
            reactions.remove(&id).unwrap_or_default()
        };
        let label = match &row.club_timezone {
            Some(club_zone) => tz::label(row.created_at, &tz::zone_or_utc(club_zone)),
            None => tz::label(row.created_at, &zone),
        };
        let club = match (row.club_slug, row.club_name) {
            (Some(slug), Some(name)) if row.club_id.is_some() => Some(ClubRef { slug, name }),
            _ => None,
        };
        Some(PostOut {
            id: row.id,
            author: row.author,
            author_avatar_url: avatar_url(row.author_avatar.as_deref()),
            mine: row.author_id == Some(viewer.id),
            body: row.body,
            deleted: row.deleted,
            visibility: row.visibility,
            spoiler_upto: row.spoiler_upto,
            progress_at: row.progress_at,
            status_at: row.status_at,
            author_rating: row.author_rating,
            parent_id: row.parent_id,
            club,
            pick_id: row.pick_id,
            milestone_id: row.milestone_id,
            created_at: row.created_at,
            created_label: label,
            edited: row.updated_at.is_some() && !row.deleted,
            reactions,
            replies: Vec::new(),
        })
    };
    let mut nested: HashMap<i64, Vec<PostOut>> = HashMap::new();
    for (reply_id, root_id) in replies {
        if let Some(reply) = build(reply_id) {
            nested.entry(root_id).or_default().push(reply);
        }
    }
    Ok(ids
        .iter()
        .filter_map(|id| {
            let mut post = build(*id)?;
            post.replies = nested.remove(id).unwrap_or_default();
            Some(post)
        })
        .collect())
}

/// One post as the viewer sees it, with its replies (a reply has none); 404 `No entry with that
/// id` when the viewer may not see it.
pub async fn post_out(
    conn: &mut PgConnection,
    viewer: &CurrentUser,
    post_id: i64,
) -> ApiResult<PostOut> {
    post_outs(conn, viewer, &[post_id], true)
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(msg::NO_ENTRY))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reaction(emoji: &str, user_id: i64, username: Option<&str>) -> ReactionRow {
        ReactionRow {
            post_id: 1,
            emoji: emoji.to_owned(),
            user_id,
            username: username.map(str::to_owned),
        }
    }

    #[test]
    fn positions_follow_the_legacy_rule() {
        use ShelfStatus::*;
        assert_eq!(position(None, None), (None, None));
        assert_eq!(position(Some(Finished), None), (Some(100), Some(Finished)));
        assert_eq!(
            position(Some(CurrentlyReading), Some(40)),
            (Some(40), Some(CurrentlyReading))
        );
        assert_eq!(
            position(Some(CurrentlyReading), None),
            (None, Some(CurrentlyReading))
        );
        assert_eq!(position(Some(WantToRead), None), (None, Some(WantToRead)));
        assert_eq!(
            position(Some(DidNotFinish), None),
            (None, Some(DidNotFinish))
        );
    }

    #[test]
    fn reactions_group_by_count_then_code_point() {
        // Probed legacy order for counts 2, 1, 1, 1: fire, heart, thumbs up, books.
        let rows = vec![
            reaction("\u{1f4da}", 1, Some("ada")),
            reaction("\u{1f525}", 2, Some("grace")),
            reaction("\u{2764}\u{fe0f}", 3, Some("tom")),
            reaction("\u{1f525}", 1, Some("ada")),
            reaction("\u{1f44d}", 2, Some("grace")),
        ];
        let out = aggregate(rows, 1);
        let order: Vec<&str> = out.iter().map(|r| r.emoji.as_str()).collect();
        assert_eq!(
            order,
            ["\u{1f525}", "\u{2764}\u{fe0f}", "\u{1f44d}", "\u{1f4da}"]
        );
        assert_eq!(out[0].count, 2);
        assert!(out[0].mine);
        assert_eq!(out[0].users, ["ada", "grace"]);
        assert!(!out[1].mine);
    }

    #[test]
    fn hidden_names_count_but_are_not_listed() {
        let out = aggregate(
            vec![
                reaction("\u{1f525}", 7, None),
                reaction("\u{1f525}", 1, Some("ada")),
            ],
            1,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 2);
        assert_eq!(out[0].users, ["ada"]);
        assert!(out[0].mine);
    }
}
