//! Invite redemption shared by sign-up (`POST /api/auth/register`) and
//! `POST /api/invites/{code}/accept` (endpoint-map §2, §3; docs/phase1/conventions.md §13).
//!
//! Sign-up calls [`consume`] and, for a club invite, [`join`], inside its own transaction, so a
//! failed join rolls the new account and the invite use back. Wave 1 step 5 implements [`join`]
//! and the club invite routes.

use sqlx::PgConnection;
use time::OffsetDateTime;

use crate::error::{ApiResult, AppError, msg};
use crate::text::is_py_space;

/// An invite whose use was just counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedInvite {
    pub id: i64,
    /// `None` for a sign-up invite minted by an instance admin (it only unlocks sign-up).
    pub club_id: Option<i64>,
    /// Becomes `club_memberships.invited_by`.
    pub created_by: Option<i64>,
    /// A removed member cannot use an invite created before the removal.
    pub created_at: OffsetDateTime,
}

/// A typed code as stored: whitespace and `-` removed, uppercased (the legacy
/// `normalize_invite_code`, without Crockford decoding).
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !is_py_space(*c) && *c != '-')
        .collect::<String>()
        .to_uppercase()
}

/// Counts one use of the invite with this normalised `code` and logs it for `user_id`
/// (`club_invite_uses`), atomically: an unknown, revoked, expired or used-up invite is `None`
/// and nothing changes. Of concurrent redemptions of a single-use invite exactly one succeeds.
///
/// Callers run it inside their transaction and roll back on any later failure. The accept route
/// checks "already in this club" (409, invite not consumed) before calling it.
pub async fn consume(
    conn: &mut PgConnection,
    code: &str,
    user_id: i64,
) -> sqlx::Result<Option<ConsumedInvite>> {
    let Some(invite) = sqlx::query_as!(
        ConsumedInvite,
        "UPDATE club_invites SET use_count = use_count + 1
         WHERE code = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR use_count < max_uses)
         RETURNING id, club_id, created_by, created_at",
        code
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query!(
        "INSERT INTO club_invite_uses (invite_id, user_id) VALUES ($1, $2)",
        invite.id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(Some(invite))
}

/// Makes `user_id` a member of `club_id` through the consumed `invite`, inside the caller's
/// transaction: locks the club row, refuses an invite created before the user's removal (400
/// `That invite code is not valid`) and a full club (409 `This club is full`), inserts the
/// membership with `invited_by`, deletes the removal row and records `member_joined`.
///
/// Wave 1 step 3 stub: refuses every club invite with 400 `That invite code is not valid`, so
/// sign-up works with sign-up invites only until step 5 implements the join.
pub async fn join(
    _conn: &mut PgConnection,
    _invite: &ConsumedInvite,
    _club_id: i64,
    _user_id: i64,
) -> ApiResult<()> {
    Err(AppError::bad_request(msg::INVITE_INVALID))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_lose_spaces_and_dashes_and_are_uppercased() {
        assert_eq!(normalize_code(" ab-cd\u{3000}ef\t"), "ABCDEF");
        assert_eq!(normalize_code("dev-only"), "DEVONLY");
        assert_eq!(normalize_code("Oil"), "OIL", "no Crockford decoding");
        assert_eq!(normalize_code(" - "), "");
    }
}
