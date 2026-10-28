//! Who may see whose personal data (#46, docs/phase1/conventions.md §6). Frozen API; the SQL
//! policy functions live in the migration (`bc_can_see_user`, `bc_can_see_post`, …), and every
//! database check here goes through them.

use sqlx::PgConnection;

use crate::auth::rules::normalize_username;
use crate::clubs::extract::Role;
use crate::dto::UserSummaryRow;
pub use crate::dto::Visibility;
use crate::error::{ApiResult, AppError, msg};

/// The viewer's relation to the owner of personal data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The viewer is the owner.
    Myself,
    /// They share at least one club.
    Audience,
    /// Anyone else.
    Stranger,
}

impl Relation {
    /// Whether the viewer may see an item with this visibility.
    pub fn sees(self, item: Visibility) -> bool {
        match self {
            Relation::Myself => true,
            Relation::Audience => item == Visibility::Audience,
            Relation::Stranger => false,
        }
    }

    /// Whether the viewer may see the owner's profile and shelf at all.
    pub fn sees_profile(self) -> bool {
        matches!(self, Relation::Myself | Relation::Audience)
    }
}

/// The viewer's relation to `owner`: `Myself`, `Audience` when they share a club
/// (`bc_can_see_user`), else `Stranger` (also for an unknown owner).
pub async fn relation(conn: &mut PgConnection, viewer: i64, owner: i64) -> sqlx::Result<Relation> {
    if viewer == owner {
        return Ok(Relation::Myself);
    }
    let sees = sqlx::query_scalar!(
        r#"SELECT bc_can_see_user($1, $2) AS "sees!""#,
        viewer,
        owner
    )
    .fetch_one(conn)
    .await?;
    Ok(if sees {
        Relation::Audience
    } else {
        Relation::Stranger
    })
}

/// Resolves `?username=` for personal reads. The name is stripped and lowercased like every
/// username lookup. 404 `No member with that name` for an unknown name and for a stranger
/// alike, so the answer never tells them apart; a name with a NUL character is unknown too.
/// The viewer resolves to themselves.
pub async fn visible_user(
    conn: &mut PgConnection,
    viewer: i64,
    username: &str,
) -> ApiResult<UserSummaryRow> {
    let username = normalize_username(username);
    if username.contains('\0') {
        return Err(AppError::not_found(msg::NO_MEMBER));
    }
    sqlx::query_as!(
        UserSummaryRow,
        r#"SELECT u.id, u.username, m.public_id AS "avatar_public_id?"
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.username = $1 AND bc_can_see_user($2, u.id)"#,
        username,
        viewer
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_MEMBER))
}

/// What a viewer may know about one post they can see.
#[derive(Debug, Clone)]
pub struct PostAccess {
    pub post_id: i64,
    pub work_id: i64,
    pub author_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub club_id: Option<i64>,
    pub pick_id: Option<i64>,
    /// The viewer's role in the post's club (club posts only).
    pub club_role: Option<Role>,
    pub deleted: bool,
}

/// `Ok` when the viewer may see the post (`bc_can_see_post`), else 404 `No entry with that id`.
/// Tombstones count as missing unless `allow_tombstone`. `club_role` is the viewer's role in the
/// post's club, for moderation checks.
pub async fn post_access(
    conn: &mut PgConnection,
    viewer: i64,
    post_id: i64,
    allow_tombstone: bool,
) -> ApiResult<PostAccess> {
    let access = sqlx::query_as!(
        PostAccess,
        r#"SELECT p.id AS post_id, p.work_id, p.author_id, p.parent_id, p.club_id, p.pick_id,
                  m.role AS "club_role?: Role", p.deleted_at IS NOT NULL AS "deleted!"
           FROM posts p
           LEFT JOIN club_memberships m ON m.club_id = p.club_id AND m.user_id = $2
           WHERE p.id = $1 AND bc_can_see_post($2, p.id)"#,
        post_id,
        viewer
    )
    .fetch_optional(conn)
    .await?;
    match access {
        Some(access) if allow_tombstone || !access.deleted => Ok(access),
        _ => Err(AppError::not_found(msg::NO_ENTRY)),
    }
}

/// Redacts an `only_me` rating and take for everyone but the owner.
pub fn redact_rating(
    is_owner: bool,
    vis: Visibility,
    rating: Option<i32>,
    take: String,
) -> (Option<i32>, String) {
    if is_owner || vis == Visibility::Audience {
        (rating, take)
    } else {
        (None, String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relations_see_what_the_rules_say() {
        assert!(Relation::Myself.sees(Visibility::OnlyMe));
        assert!(Relation::Myself.sees(Visibility::Audience));
        assert!(Relation::Audience.sees(Visibility::Audience));
        assert!(!Relation::Audience.sees(Visibility::OnlyMe));
        assert!(!Relation::Stranger.sees(Visibility::Audience));
        assert!(Relation::Audience.sees_profile());
        assert!(!Relation::Stranger.sees_profile());
    }

    #[test]
    fn only_me_ratings_are_redacted_for_others() {
        assert_eq!(
            redact_rating(false, Visibility::OnlyMe, Some(5), "Loved it".into()),
            (None, String::new())
        );
        assert_eq!(
            redact_rating(true, Visibility::OnlyMe, Some(5), "Loved it".into()),
            (Some(5), "Loved it".to_owned())
        );
        assert_eq!(
            redact_rating(false, Visibility::Audience, Some(4), "Fine".into()),
            (Some(4), "Fine".to_owned())
        );
    }
}
