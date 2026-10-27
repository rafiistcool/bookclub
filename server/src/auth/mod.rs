//! Accounts and auth (#39): sessions, bearer tokens, passwords, email flows, account deletion
//! and export (docs/phase1/conventions.md §5).

pub mod extract;
pub mod origin;
pub mod password;
pub mod random;

use sqlx::PgConnection;
use utoipa_axum::router::OpenApiRouter;

use crate::AppState;
use crate::dto::UserOut;
use crate::error::{ApiResult, AppError};

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "bookclub_session";

/// The auth routes, plus `GET /api/config`. Wave 1 step 3 fills them in.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

/// The full `UserOut` of `user_id`, including `clubs` (frozen; MED's avatar endpoints return it).
/// Wave 1 step 3 implements it.
pub async fn user_out(_conn: &mut PgConnection, user_id: i64) -> ApiResult<UserOut> {
    Err(AppError::internal(format!(
        "auth::user_out is not implemented yet (user {user_id})"
    )))
}

/// The hourly cleanup job: deletes expired sessions and used or expired email tokens.
pub async fn cleanup(conn: &mut PgConnection) -> sqlx::Result<()> {
    let sessions = sqlx::query!("DELETE FROM sessions WHERE expires_at < now()")
        .execute(&mut *conn)
        .await?
        .rows_affected();
    let tokens =
        sqlx::query!("DELETE FROM email_tokens WHERE used_at IS NOT NULL OR expires_at < now()")
            .execute(&mut *conn)
            .await?
            .rows_affected();
    tracing::info!(
        sessions,
        tokens,
        "deleted expired sessions and email tokens"
    );
    Ok(())
}
