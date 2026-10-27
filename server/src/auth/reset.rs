//! Password reset by email (endpoint-map §2, docs/phase1/conventions.md §5.5): request a link,
//! then confirm it with a new password.

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::account::{account_key, email_login_key};
use super::login::too_many;
use super::mail::{self, LINK_EXPIRED, TokenPurpose};
use super::rules;
use crate::AppState;
use crate::client_ip::ClientIp;
use crate::email::Email;
use crate::email::templates::{self, Recipient};
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, Validate, Validator};
use crate::ratelimit::ip_key;
use crate::text::py_strip;

/// The address to send a reset link to.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetRequestIn {
    /// Compared case-insensitively with verified account addresses only.
    pub email: String,
}

impl Validate for ResetRequestIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.email = py_strip(&self.email).to_owned();
    }
}

/// The token from a `/reset-password#token=…` link and the new password.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetConfirmIn {
    pub token: String,
    /// At least 8 characters; never trimmed.
    pub new_password: String,
}

impl Validate for ResetConfirmIn {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            rules::long_enough(&self.new_password),
            rules::NEW_PASSWORD_RULE,
        );
    }
}

/// Sends a reset link (valid one hour) when a verified account holds the address. The answer
/// is always 204, and it does not wait for the lookup or the mail: the work runs in a
/// background task, so neither the answer nor its timing tells whether an account exists.
///
/// Five requests per IP address an hour (429). Three links per account an hour; over that the
/// request is silently dropped.
#[utoipa::path(
    post,
    path = "/api/auth/password-reset",
    operation_id = "auth_request_password_reset",
    tag = "auth",
    request_body = ResetRequestIn,
    responses(
        (status = 204, description = "Accepted, whether or not an account holds the address"),
        (status = 400, body = ErrorBody, description = "Malformed body"),
        (status = 429, body = ErrorBody, description = "`Too many attempts. Wait a few minutes.`")
    )
)]
pub async fn request_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<ResetRequestIn>,
) -> ApiResult<StatusCode> {
    state
        .limits
        .reset_ip
        .check_and_hit(&ip_key(ip))
        .map_err(too_many)?;
    tokio::spawn(async move {
        match reset_mail(&state, &body.email).await {
            Ok(Some(email)) => mail::deliver(&state, email).await,
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "password reset request failed"),
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Issues the reset token for the verified account holding `email`, unless the account's limit
/// is reached, and returns the mail with its link.
async fn reset_mail(state: &AppState, email: &str) -> sqlx::Result<Option<Email>> {
    let mut tx = state.db.begin().await?;
    let Some(holder) = mail::verified_holder(&mut tx, email).await? else {
        return Ok(None);
    };
    if state
        .limits
        .reset_account
        .check_and_hit(&account_key(holder.user_id))
        .is_err()
    {
        return Ok(None);
    }
    let token = mail::issue_reset_token(&mut tx, holder.user_id, &holder.email).await?;
    tx.commit().await?;
    Ok(Some(templates::reset_password(
        mail::site(&state.config),
        Recipient {
            to: &holder.email,
            username: &holder.username,
            locale: &holder.locale,
        },
        &mail::reset_link(&state.config, &token),
    )))
}

/// Sets a new password with a reset link. The link must be open and still for the account's
/// verified address. Every session and bearer token of the account is deleted, and every open
/// reset link and a pending address change are voided; the user then logs in. A right link
/// also clears the account's login failures.
#[utoipa::path(
    post,
    path = "/api/auth/password-reset/confirm",
    operation_id = "auth_confirm_password_reset",
    tag = "auth",
    request_body = ResetConfirmIn,
    responses(
        (status = 204, description = "The password is set; every session is signed out"),
        (status = 400, body = ErrorBody, description = "`That link has expired or was already used`, `New password must be at least 8 characters`")
    )
)]
pub async fn confirm_reset(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ResetConfirmIn>,
) -> ApiResult<StatusCode> {
    let hash = mail::link_token_hash(&body.token).ok_or_else(link_expired)?;
    // Checked before hashing, so a made-up token costs no argon2 work.
    let user_id = sqlx::query_scalar!(
        "SELECT t.user_id FROM email_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND t.purpose = 'reset_password'
           AND t.used_at IS NULL AND t.expires_at > now()
           AND u.email = t.email AND u.email_verified_at IS NOT NULL",
        hash
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(link_expired)?;
    let new_hash = state.passwords.hash(&body.new_password).await?;

    let mut tx = state.db.begin().await?;
    // The user row first, then the token, like every email flow.
    let account = sqlx::query!(
        r#"SELECT email, email_verified_at IS NOT NULL AS "verified!" FROM users
           WHERE id = $1 FOR UPDATE"#,
        user_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(link_expired)?;
    let token_email = sqlx::query_scalar!(
        "UPDATE email_tokens SET used_at = now()
         WHERE token_hash = $1 AND user_id = $2 AND purpose = 'reset_password'
           AND used_at IS NULL AND expires_at > now()
         RETURNING email",
        hash,
        user_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(link_expired)?;
    if !account.verified || account.email.as_deref() != Some(&token_email) {
        return Err(link_expired());
    }
    sqlx::query!(
        "UPDATE users SET password_hash = $2, password_changed_at = now() WHERE id = $1",
        user_id,
        new_hash
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?;
    mail::void_open_tokens(&mut tx, user_id, TokenPurpose::ResetPassword).await?;
    // A pending address change may be the work of whoever knew the old password: the owner
    // starts it again if it was theirs.
    mail::void_open_tokens(&mut tx, user_id, TokenPurpose::VerifyEmail).await?;
    tx.commit().await?;
    // The link proves the mailbox; failed guesses by someone else no longer lock the owner out.
    state.limits.login_failures.clear(&account_key(user_id));
    state.limits.login_failures.clear(&email_login_key(user_id));
    Ok(StatusCode::NO_CONTENT)
}

fn link_expired() -> AppError {
    AppError::bad_request(LINK_EXPIRED)
}
