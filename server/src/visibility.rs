//! Who may see whose personal data (#46, docs/phase1/conventions.md §6). Frozen API; the SQL
//! policy functions live in the migration. Wave 1 step 3 implements the database checks; until
//! then they deny.

use sqlx::PgConnection;

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

/// The viewer's relation to `owner`. Step 3 implements it with `bc_can_see_user`.
pub async fn relation(_conn: &mut PgConnection, viewer: i64, owner: i64) -> sqlx::Result<Relation> {
    Ok(if viewer == owner {
        Relation::Myself
    } else {
        Relation::Stranger
    })
}

/// Resolves `?username=` for personal reads: 404 `No member with that name` when unknown or a
/// stranger. Step 3 implements it.
pub async fn visible_user(
    _conn: &mut PgConnection,
    _viewer: i64,
    _username: &str,
) -> ApiResult<UserSummaryRow> {
    Err(AppError::not_found(msg::NO_MEMBER))
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

/// `Ok` when the viewer may see the post, else 404 `No entry with that id`. Tombstones count as
/// missing unless `allow_tombstone`. Step 3 implements it with `bc_can_see_post`.
pub async fn post_access(
    _conn: &mut PgConnection,
    _viewer: i64,
    _post_id: i64,
    _allow_tombstone: bool,
) -> ApiResult<PostAccess> {
    Err(AppError::not_found(msg::NO_ENTRY))
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
