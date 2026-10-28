//! The `/api/admin` router: every route is guarded by one `InstanceAdmin` route layer in
//! `lib.rs` (docs/phase1/conventions.md §2).
//!
//! - [`invites`]: closed-beta sign-up invites.
//! - [`summary`]: `GET /api/admin/summary`, both queues at a glance.
//! - [`queue`]: the `?status=&page=` query and the overdue threshold both queues share.
//! - [`notify`]: the job that mails instance admins about new reports and catalog requests.
//!
//! The queues themselves live with their data: `catalog::admin` (catalog requests) and
//! `media::reports` (reports and takedowns).

pub mod invites;
pub mod notify;
pub mod queue;
pub mod summary;

use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;

/// The admin routes of this module; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(invites::admin_routes())
        .routes(routes!(summary::summary))
}

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
