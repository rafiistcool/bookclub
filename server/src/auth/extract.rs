//! Who is calling (docs/phase1/conventions.md §5.3). Frozen: handlers put one of these first in
//! their argument list, so a signed-out request is 401 before its body is looked at.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

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

/// The user when signed in, `None` otherwise. Never rejects.
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

    async fn from_request_parts(
        _parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Wave 1 step 3 resolves the bearer token or the session cookie (§5.1, §5.2).
        Err(AppError::unauthorized())
    }
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(
            CurrentUser::from_request_parts(parts, state).await.ok(),
        ))
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
