//! The `/api/admin` router: every route is guarded by one `InstanceAdmin` route layer in
//! `lib.rs` (docs/phase1/conventions.md §2).

pub mod invites;

use sqlx::PgConnection;

/// Makes the user an instance admin (`bookclub-server make-admin`). The name is normalised like
/// every username lookup. Returns false when no such user exists.
pub async fn make_admin(conn: &mut PgConnection, username: &str) -> sqlx::Result<bool> {
    let username = username.trim().to_lowercase();
    let updated = sqlx::query!(
        "UPDATE users SET is_admin = true WHERE username = $1",
        username
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(updated > 0)
}
