//! Accounts and auth (#39): sessions, bearer tokens, passwords, email flows, account deletion
//! and export (docs/phase1/conventions.md §5, endpoint-map §2).

pub mod account;
pub mod address;
pub mod deletion;
pub mod export;
pub mod extract;
pub mod login;
pub mod mail;
pub mod origin;
pub mod password;
pub mod random;
pub mod reset;
pub mod rules;
pub mod session;
pub mod signup;
pub mod tokens;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::PgConnection;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::dto::{ClubSummary, UserOut};
use crate::error::{ApiResult, AppError};
use crate::media::shared::urls::avatar_url;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "bookclub_session";

/// The service accent colours: the legacy defaults (docs/phase1/spec/auth.md §1.20).
const THEME: &str = "#b44a2a";
const THEME_DARK: &str = "#8e361c";

/// The auth routes, plus `GET /api/config`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(service_config))
        .routes(routes!(signup::register))
        .routes(routes!(login::login))
        .routes(routes!(login::logout))
        .routes(routes!(account::me, deletion::delete_account))
        .routes(routes!(account::update_preferences))
        .routes(routes!(account::change_password))
        .routes(routes!(address::verify_email))
        .routes(routes!(address::resend_verification))
        .routes(routes!(address::change_email))
        .routes(routes!(address::dismiss_email_prompt))
        .routes(routes!(reset::request_reset))
        .routes(routes!(reset::confirm_reset))
        .routes(routes!(export::export_me))
        .routes(routes!(
            account::notifications,
            account::update_notifications
        ))
        .routes(routes!(tokens::create_token))
        .routes(routes!(tokens::list_sessions))
        .routes(routes!(tokens::revoke_session))
}

/// Public service branding for the SPA.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceConfig {
    /// `SERVICE_NAME`.
    pub name: String,
    /// The accent colour, `#rrggbb`.
    pub theme: String,
    /// The accent colour in dark mode.
    pub theme_dark: String,
    /// `PUBLIC_URL`, an origin.
    pub public_url: String,
    /// `"invite"` (sign-up needs an invite code) or `"open"`.
    pub signup: String,
}

/// The service name, accent colours, public origin and sign-up mode. Public; answers from
/// memory.
#[utoipa::path(
    get,
    path = "/api/config",
    operation_id = "auth_get_config",
    tag = "auth",
    responses((status = 200, body = ServiceConfig, description = "Service branding"))
)]
pub async fn service_config(State(state): State<AppState>) -> Json<ServiceConfig> {
    let config = &state.config;
    Json(ServiceConfig {
        name: config.service_name.clone(),
        theme: THEME.to_owned(),
        theme_dark: THEME_DARK.to_owned(),
        public_url: config.public_url.clone(),
        signup: config.signup_mode.as_str().to_owned(),
    })
}

/// The full `UserOut` of `user_id`, including `clubs` (frozen; MED's avatar endpoints return
/// it). A user that no longer exists is 401 `Not signed in`.
///
/// - `pending_email` is the address of the open verification link of an account whose own
///   address is verified (an email change waiting for confirmation); null otherwise.
/// - `needs_email_prompt` is `email IS NULL AND email_prompt_dismissed_at IS NULL`.
/// - `clubs` are ordered newest join first.
pub async fn user_out(conn: &mut PgConnection, user_id: i64) -> ApiResult<UserOut> {
    let Some(row) = sqlx::query!(
        r#"SELECT u.id, u.username, u.email, u.email_verified_at IS NOT NULL AS "email_verified!",
                  (SELECT t.email FROM email_tokens t
                   WHERE t.user_id = u.id AND t.purpose = 'verify_email'
                     AND t.used_at IS NULL AND t.expires_at > now()
                   ORDER BY t.id DESC LIMIT 1) AS open_verification,
                  (u.email IS NULL AND u.email_prompt_dismissed_at IS NULL) AS "needs_email_prompt!",
                  u.theme, u.color_mode, u.locale, u.timezone, u.is_admin,
                  m.public_id AS "avatar_public_id?"
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.id = $1"#,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Err(AppError::unauthorized());
    };
    let clubs = sqlx::query_as!(
        ClubSummary,
        r#"SELECT c.slug, c.name, m.role, c.timezone,
                  (SELECT count(*) FROM club_memberships x WHERE x.club_id = c.id) AS "member_count!"
           FROM club_memberships m
           JOIN clubs c ON c.id = m.club_id
           WHERE m.user_id = $1
           ORDER BY m.joined_at DESC, c.id DESC"#,
        user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(UserOut {
        id: row.id,
        username: row.username,
        pending_email: row.open_verification.filter(|_| row.email_verified),
        email: row.email,
        email_verified: row.email_verified,
        needs_email_prompt: row.needs_email_prompt,
        theme: row.theme,
        color_mode: row.color_mode,
        locale: row.locale,
        timezone: row.timezone,
        avatar_url: avatar_url(row.avatar_public_id.as_deref()),
        is_admin: row.is_admin,
        clubs,
    })
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
