//! Who is calling (docs/phase1/conventions.md §5.3). Frozen: handlers put one of these first in
//! their argument list, so a signed-out request is 401 before its body is looked at.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};

use super::session::{self, CookieAction, CookieSlot, Presented, SessionKind};
use crate::AppState;
use crate::error::{AppError, msg};

/// The signed-in user, from a bearer token (read first) or the session cookie.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub email_verified: bool,
    pub locale: String,
    pub timezone: String,
    /// The `sessions.id` that authenticated this request.
    pub session_id: i64,
}

/// The user when signed in, `None` otherwise (also for a dead cookie, which is then not
/// cleared: only a 401 clears it). Rejects only when the session lookup itself fails (500), so
/// a database error never turns a signed-in caller into an anonymous one.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<CurrentUser>);

/// A signed-in user with a verified email address; 403 `Verify your email address first`.
#[derive(Debug, Clone)]
pub struct VerifiedUser(pub CurrentUser);

/// An instance admin; 401 when signed out, 404 `Not found` for everyone else.
#[derive(Debug, Clone)]
pub struct InstanceAdmin(pub CurrentUser);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    /// One query joins `sessions` and `users`. A session idle for over an hour slides: its
    /// `expires_at` moves to 30 days (cookie) or 90 days (bearer) from now. A cookie that names
    /// no live session gets a clearing `Set-Cookie` with the 401 (`session::maintain_cookie`).
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let presented = session::presented(&parts.headers);
        let slot = parts.extensions.get::<CookieSlot>().cloned();
        let found = match presented.secret() {
            Some((secret, kind)) => resolve(state, &secret, kind).await?,
            None => None,
        };
        let Some((user, slid)) = found else {
            if let (Presented::Cookie(_), Some(slot)) = (presented, &slot) {
                slot.set(CookieAction::Clear);
            }
            return Err(AppError::unauthorized());
        };
        if let (true, Presented::Cookie(Some(secret)), Some(slot)) = (slid, presented, &slot) {
            slot.set(CookieAction::Renew(BASE64_URL_SAFE_NO_PAD.encode(secret)));
        }
        Ok(user)
    }
}

/// The user behind a live session, and whether the session slid just now.
async fn resolve(
    state: &AppState,
    secret: &[u8; 32],
    kind: SessionKind,
) -> Result<Option<(CurrentUser, bool)>, AppError> {
    let mut conn = state.db.acquire().await?;
    let Some(row) = sqlx::query!(
        r#"SELECT s.id AS session_id, s.last_used_at < now() - interval '1 hour' AS "stale!",
                  u.id, u.username, u.is_admin, u.email_verified_at IS NOT NULL AS "email_verified!",
                  u.locale, u.timezone
           FROM sessions s
           JOIN users u ON u.id = s.user_id
           WHERE s.token_hash = $1 AND s.kind = $2 AND s.expires_at > now()"#,
        session::token_hash(secret),
        kind as SessionKind
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    if row.stale {
        sqlx::query!(
            "UPDATE sessions
             SET last_used_at = now(),
                 expires_at = now() + CASE kind WHEN 'cookie' THEN interval '30 days'
                                                ELSE interval '90 days' END
             WHERE id = $1",
            row.session_id
        )
        .execute(&mut *conn)
        .await?;
    }
    let user = CurrentUser {
        id: row.id,
        username: row.username,
        is_admin: row.is_admin,
        email_verified: row.email_verified,
        locale: row.locale,
        timezone: row.timezone,
        session_id: row.session_id,
    };
    Ok(Some((user, row.stale)))
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match CurrentUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeUser(Some(user))),
            Err(error) if error.status == StatusCode::UNAUTHORIZED => Ok(MaybeUser(None)),
            Err(error) => Err(error),
        }
    }
}

impl FromRequestParts<AppState> for VerifiedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.email_verified {
            Ok(VerifiedUser(user))
        } else {
            Err(AppError::forbidden(msg::VERIFY_EMAIL_FIRST))
        }
    }
}

impl FromRequestParts<AppState> for InstanceAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.is_admin {
            Ok(InstanceAdmin(user))
        } else {
            Err(AppError::not_found(msg::NOT_FOUND))
        }
    }
}
