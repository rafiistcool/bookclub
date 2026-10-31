//! The response types of the friend routes and their loader (docs/phase3/work-plan.md §4.5).
//!
//! Every friend route answers with the caller's whole [`FriendsOut`], so the app replaces its
//! state with the answer. Nothing here is ever served to anyone but the caller: no friend list,
//! request or block of another account, and no count of friends.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::catalog::shared::books::{BookRef, book_outs};
use crate::dto::{BookOut, ClubRef, UserSummary, UserSummaryRow};

/// How many currently-reading books a friend's card previews (as a club member's card does).
const PREVIEW_BOOKS: i64 = 3;

/// The caller's friends, requests, blocks and personal club invitations.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FriendsOut {
    /// By username.
    pub friends: Vec<FriendOut>,
    /// Requests to the caller, newest first. Requests from accounts the caller blocked are left
    /// out.
    pub incoming: Vec<FriendRequestOut>,
    /// Requests the caller sent and nobody answered yet, newest first.
    pub outgoing: Vec<FriendRequestOut>,
    /// The accounts the caller blocked, newest first.
    pub blocked: Vec<BlockedOut>,
    /// Friend invites into clubs for the caller that can still be redeemed, to clubs the caller
    /// is not in, newest first.
    pub club_invites: Vec<FriendClubInviteOut>,
}

/// One friend.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FriendOut {
    pub user: UserSummary,
    /// When the friendship began.
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
    pub currently_reading_count: i64,
    /// At most three, in shelf order.
    pub currently_reading: Vec<FriendReadingOut>,
}

/// A book a friend is currently reading.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FriendReadingOut {
    pub book: BookOut,
    /// Percent; null when the friend has not set it.
    pub progress: Option<i32>,
}

/// A pending friend request.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FriendRequestOut {
    /// The other account. `avatar_url` is null unless the caller can see that account (a club
    /// mate): a reader only sees pictures they could report.
    pub user: UserSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// An account the caller blocked.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlockedOut {
    pub username: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A friend invite into a club for the caller.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FriendClubInviteOut {
    /// The invite's id, for `DELETE /api/friends/club-invites/{invite_id}`.
    pub id: i64,
    pub club: ClubRef,
    /// The username of the friend who invited; null when that account is gone.
    pub invited_by: Option<String>,
    /// The invite code, for `/join/{code}`. Only the caller can redeem it.
    pub code: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// The caller's [`FriendsOut`].
///
/// The currently-reading previews read the friends' shelves through `bc_visible_shelf` (an
/// `only_me` rating never shows), one windowed query for every friend, as the club's member list
/// does (`clubs::out::member_outs`).
pub async fn friends_out(conn: &mut PgConnection, viewer: i64) -> sqlx::Result<FriendsOut> {
    let friends = sqlx::query!(
        r#"SELECT u.id, u.username, m.public_id AS "avatar_public_id?", f.since AS "since!"
           FROM bc_friends($1) f
           JOIN users u ON u.id = f.user_id
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           ORDER BY u.username"#,
        viewer
    )
    .fetch_all(&mut *conn)
    .await?;
    let reading = sqlx::query!(
        r#"SELECT r.user_id AS "user_id!", r.work_id AS "work_id!", r.edition_id, r.progress,
                  r.total AS "total!"
           FROM (
               SELECT s.user_id, s.work_id, s.edition_id, s.progress,
                      row_number() OVER (PARTITION BY s.user_id ORDER BY s.position, s.id) AS rn,
                      count(*) OVER (PARTITION BY s.user_id) AS total
               FROM bc_visible_shelf($1) s
               JOIN bc_friends($1) f ON f.user_id = s.user_id
               WHERE s.status = 'currently_reading'
           ) r
           WHERE r.rn <= $2
           ORDER BY r.user_id, r.rn"#,
        viewer,
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
    let mut previews: HashMap<i64, (i64, Vec<FriendReadingOut>)> = HashMap::new();
    for row in &reading {
        let entry = previews
            .entry(row.user_id)
            .or_insert_with(|| (row.total, Vec::new()));
        let key = BookRef {
            work_id: row.work_id,
            edition_id: row.edition_id,
        };
        if let Some(book) = books.get(&key) {
            entry.1.push(FriendReadingOut {
                book: book.clone(),
                progress: row.progress,
            });
        }
    }
    let friends = friends
        .into_iter()
        .map(|row| {
            let (count, reading) = previews.remove(&row.id).unwrap_or_default();
            FriendOut {
                user: UserSummaryRow {
                    id: row.id,
                    username: row.username,
                    avatar_public_id: row.avatar_public_id,
                }
                .into(),
                since: row.since,
                currently_reading_count: count,
                currently_reading: reading,
            }
        })
        .collect();

    // A requester's or addressee's picture only when the viewer can see them (a club mate).
    let requests = sqlx::query!(
        r#"SELECT f.requester_id = $1 AS "outgoing!", u.id, u.username, f.created_at,
                  CASE WHEN bc_can_see_user($1, u.id) THEN m.public_id END AS "avatar_public_id?"
           FROM friendships f
           JOIN users u ON u.id = CASE WHEN f.requester_id = $1 THEN f.addressee_id
                                       ELSE f.requester_id END
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE (f.requester_id = $1 OR f.addressee_id = $1) AND f.accepted_at IS NULL
             AND NOT (f.addressee_id = $1 AND EXISTS (
                 SELECT 1 FROM user_blocks b
                 WHERE b.user_id = $1 AND b.blocked_user_id = f.requester_id))
           ORDER BY f.created_at DESC, u.username"#,
        viewer
    )
    .fetch_all(&mut *conn)
    .await?;
    let (mut incoming, mut outgoing) = (Vec::new(), Vec::new());
    for row in requests {
        let out = FriendRequestOut {
            user: UserSummaryRow {
                id: row.id,
                username: row.username,
                avatar_public_id: row.avatar_public_id,
            }
            .into(),
            created_at: row.created_at,
        };
        if row.outgoing {
            outgoing.push(out);
        } else {
            incoming.push(out);
        }
    }

    let blocked = sqlx::query_as!(
        BlockedOut,
        "SELECT u.username, b.created_at
         FROM user_blocks b
         JOIN users u ON u.id = b.blocked_user_id
         WHERE b.user_id = $1
         ORDER BY b.created_at DESC, u.username",
        viewer
    )
    .fetch_all(&mut *conn)
    .await?;

    // Bound invites always expire (`kind: "friend"` sets 1 to 30 days), so `expires_at > now()`
    // is the whole expiry check.
    let club_invites = sqlx::query!(
        r#"SELECT i.id, c.slug, c.name, u.username AS "invited_by?", i.code, i.created_at,
                  i.expires_at AS "expires_at!"
           FROM club_invites i
           JOIN clubs c ON c.id = i.club_id
           LEFT JOIN users u ON u.id = i.created_by
           WHERE i.invitee_id = $1 AND i.revoked_at IS NULL AND i.expires_at > now()
             AND (i.max_uses IS NULL OR i.use_count < i.max_uses)
             AND NOT EXISTS (SELECT 1 FROM club_memberships x
                             WHERE x.club_id = i.club_id AND x.user_id = $1)
           ORDER BY i.created_at DESC, i.id DESC"#,
        viewer
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| FriendClubInviteOut {
        id: row.id,
        club: ClubRef {
            slug: row.slug,
            name: row.name,
        },
        invited_by: row.invited_by,
        code: row.code,
        created_at: row.created_at,
        expires_at: row.expires_at,
    })
    .collect();

    Ok(FriendsOut {
        friends,
        incoming,
        outgoing,
        blocked,
        club_invites,
    })
}
