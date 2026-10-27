//! The signed-in user's own account (endpoint-map §2): `GET /api/auth/me`, preferences,
//! notification preferences and the password change.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::extract::CurrentUser;
use super::login::too_many;
use super::mail::{self, TokenPurpose};
use super::{rules, user_out};
use crate::AppState;
use crate::dto::UserOut;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::text::py_strip;
use crate::tz;

/// Preferences to change; `null` or absent leaves a value as it is. `{}` is a valid no-op.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreferencesIn {
    /// `paper`, `slate`, `forest` or `ink` (case and surrounding spaces ignored).
    #[serde(default)]
    pub theme: Option<String>,
    /// `light`, `dark` or `system`.
    #[serde(default)]
    pub color_mode: Option<String>,
    /// `en` or `de`.
    #[serde(default)]
    pub locale: Option<String>,
    /// IANA zone for personal dates, canonical spelling (`Europe/Berlin`).
    #[serde(default)]
    pub timezone: Option<String>,
}

impl Validate for PreferencesIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(theme) = &mut self.theme {
            v.check(
                rules::allowed_choice(theme, &rules::THEMES),
                rules::THEME_RULE,
            );
        }
        if let Some(mode) = &mut self.color_mode {
            v.check(
                rules::allowed_choice(mode, &rules::COLOR_MODES),
                rules::COLOR_MODE_RULE,
            );
        }
        if let Some(locale) = &mut self.locale {
            v.check(
                rules::allowed_choice(locale, &rules::LOCALES),
                rules::LOCALE_RULE,
            );
        }
        if let Some(zone) = &mut self.timezone {
            *zone = py_strip(zone).to_owned();
            v.check(tz::parse_zone(zone).is_ok(), msg::INVALID_TIME_ZONE);
        }
    }
}

/// Push preferences, kept for Phase 4 (nothing is sent in Phase 1).
#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationsOut {
    pub notify_meeting: bool,
    pub notify_pick: bool,
    pub notify_note: bool,
}

/// Push preferences to change; `null` or absent leaves a value as it is. Strict JSON booleans.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NotificationsIn {
    #[serde(default)]
    pub notify_meeting: Option<bool>,
    #[serde(default)]
    pub notify_pick: Option<bool>,
    #[serde(default)]
    pub notify_note: Option<bool>,
}

impl Validate for NotificationsIn {}

/// The current and the new password.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordChangeIn {
    pub current_password: String,
    /// At least 8 characters.
    pub new_password: String,
}

impl Validate for PasswordChangeIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            rules::long_enough(&self.new_password),
            rules::NEW_PASSWORD_RULE,
        );
    }
}

/// The signed-in user's account.
#[utoipa::path(
    get,
    path = "/api/auth/me",
    operation_id = "auth_get_me",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = UserOut, description = "The account"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn me(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<UserOut>> {
    Ok(Json(
        user_out(&mut *state.db.acquire().await?, user.id).await?,
    ))
}

/// Changes theme, colour mode, locale or time zone. A body is required (`{}` changes nothing);
/// invalid values change nothing and every message is reported.
#[utoipa::path(
    patch,
    path = "/api/auth/me/preferences",
    operation_id = "auth_update_preferences",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = PreferencesIn,
    responses(
        (status = 200, body = UserOut, description = "The updated account"),
        (status = 400, body = ErrorBody, description = "A value outside its list, or an invalid time zone"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn update_preferences(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PreferencesIn>,
) -> ApiResult<Json<UserOut>> {
    let mut conn = state.db.acquire().await?;
    sqlx::query!(
        "UPDATE users
         SET theme = COALESCE($2, theme), color_mode = COALESCE($3, color_mode),
             locale = COALESCE($4, locale), timezone = COALESCE($5, timezone)
         WHERE id = $1",
        user.id,
        body.theme,
        body.color_mode,
        body.locale,
        body.timezone
    )
    .execute(&mut *conn)
    .await?;
    Ok(Json(user_out(&mut conn, user.id).await?))
}

/// The push preferences (all on by default).
#[utoipa::path(
    get,
    path = "/api/auth/notifications",
    operation_id = "auth_get_notifications",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = NotificationsOut, description = "The preferences"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn notifications(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<NotificationsOut>> {
    let prefs = sqlx::query_as!(
        NotificationsOut,
        "SELECT notify_meeting, notify_pick, notify_note FROM users WHERE id = $1",
        user.id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unauthorized)?;
    Ok(Json(prefs))
}

/// Changes push preferences.
#[utoipa::path(
    patch,
    path = "/api/auth/notifications",
    operation_id = "auth_update_notifications",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = NotificationsIn,
    responses(
        (status = 200, body = NotificationsOut, description = "The updated preferences"),
        (status = 400, body = ErrorBody, description = "`Input should be a valid boolean`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn update_notifications(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<NotificationsIn>,
) -> ApiResult<Json<NotificationsOut>> {
    let prefs = sqlx::query_as!(
        NotificationsOut,
        "UPDATE users
         SET notify_meeting = COALESCE($2, notify_meeting),
             notify_pick = COALESCE($3, notify_pick),
             notify_note = COALESCE($4, notify_note)
         WHERE id = $1
         RETURNING notify_meeting, notify_pick, notify_note",
        user.id,
        body.notify_meeting,
        body.notify_pick,
        body.notify_note
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unauthorized)?;
    Ok(Json(prefs))
}

/// Changes the password. It shares the login failure limit (10 wrong current passwords in ten
/// minutes, then 429); a right current password clears it. Every other session and bearer
/// token of the user is deleted, and every open reset link is voided, and so is a pending
/// address change of a verified account; the current session stays. A password change or reset that commits while this one verifies wins: this one then
/// answers `Current password is wrong`.
#[utoipa::path(
    patch,
    path = "/api/auth/password",
    operation_id = "auth_change_password",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = PasswordChangeIn,
    responses(
        (status = 204, description = "Changed"),
        (status = 400, body = ErrorBody, description = "`Current password is wrong`, `Pick a different password`, `New password must be at least 8 characters`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "`Too many attempts. Wait a few minutes.`")
    )
)]
pub async fn change_password(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PasswordChangeIn>,
) -> ApiResult<StatusCode> {
    let stored = verify_current_password(&state, user.id, &body.current_password).await?;
    if body.current_password == body.new_password {
        return Err(AppError::bad_request(msg::PICK_DIFFERENT_PASSWORD));
    }
    let new_hash = state.passwords.hash(&body.new_password).await?;

    let mut tx = state.db.begin().await?;
    // Only over the hash that was verified: a change or reset that committed meanwhile wins.
    let verified = sqlx::query_scalar!(
        r#"UPDATE users SET password_hash = $3, password_changed_at = now()
           WHERE id = $1 AND password_hash = $2
           RETURNING email_verified_at IS NOT NULL AS "verified!""#,
        user.id,
        stored,
        new_hash
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad_request(msg::CURRENT_PASSWORD_WRONG))?;
    sqlx::query!(
        "DELETE FROM sessions WHERE user_id = $1 AND id <> $2",
        user.id,
        user.session_id
    )
    .execute(&mut *tx)
    .await?;
    mail::void_open_tokens(&mut tx, user.id, TokenPurpose::ResetPassword).await?;
    if verified {
        // The open link is a pending address change, which whoever knew the old password may
        // have started; the owner starts it again if it was theirs. An unverified account's
        // link verifies its own address and stays.
        mail::void_open_tokens(&mut tx, user.id, TokenPurpose::VerifyEmail).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks the signed-in user's password before a sensitive change (password, email address,
/// account deletion) and returns the stored hash it was verified against.
///
/// A wrong password counts as a login failure: `login_failures.check_and_hit("u:<id>")` runs
/// **before** verifying, so concurrent guesses cannot pass together (429 `Too many attempts.
/// Wait a few minutes.` once 10 are counted); the right password clears the key. A wrong one is
/// 400 `Current password is wrong`. The caller's write must be guarded by the returned hash
/// (`… WHERE password_hash = <hash>`, or a locked re-read), so a change or reset that commits
/// meanwhile wins.
pub(crate) async fn verify_current_password(
    state: &AppState,
    user_id: i64,
    password: &str,
) -> ApiResult<String> {
    let stored = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    let key = account_key(user_id);
    state
        .limits
        .login_failures
        .check_and_hit(&key)
        .map_err(too_many)?;
    if !state
        .passwords
        .verify(password, Some(&stored))
        .await
        .is_match()
    {
        return Err(AppError::bad_request(msg::CURRENT_PASSWORD_WRONG));
    }
    // The hit above was counted before verifying; the right password proves the user.
    state.limits.login_failures.clear(&key);
    Ok(stored)
}

/// The rate-limit key of an account, `u:<id>`: every per-account limit, and in
/// `login_failures` the logins by username and the current-password check.
pub(crate) fn account_key(user_id: i64) -> String {
    format!("u:{user_id}")
}

/// The `login_failures` key of the logins by an account's verified address, `u:<id>:email`.
/// Apart from the username's, so a filled bucket of one never shows that the other names the
/// same account.
pub(crate) fn email_login_key(user_id: i64) -> String {
    format!("u:{user_id}:email")
}
