//! Password reset by email (endpoint-map §2, docs/phase1/conventions.md §5.5): request a link,
//! then confirm it with a new password. Without mail, `bookclub-server set-password` does the
//! confirm step's work from the host ([`set_password`]).

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;

use super::account::{account_key, email_login_key};
use super::login::too_many;
use super::mail::{self, LINK_EXPIRED, MailKind, TokenPurpose};
use super::password::Passwords;
use super::{random, rules};
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
/// Five requests per IP address an hour (429). Silently, nothing is sent while the account has
/// an open link (one an hour at most), and at most three links go to an account an hour.
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
            Ok(Some((email, token))) => {
                if !mail::deliver(&state, email, MailKind::Link).await {
                    // A link nobody received must not hold back the next request.
                    let voided =
                        async { mail::void_token(&mut *state.db.acquire().await?, &token).await };
                    if let Err(error) = voided.await {
                        tracing::warn!(%error, "voiding an unsent reset link failed");
                    }
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "password reset request failed"),
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Issues the reset token for the verified account holding `email` and returns the mail with
/// its link (and the token). Nothing is issued while the account has an open reset link (its
/// mail went out: one link an hour at most, so reset requests, which anyone can send, cannot
/// use up the address's mail limit faster than its owner can use a link), when the address's
/// mail limit would drop the mail, or over the account's limit.
async fn reset_mail(state: &AppState, email: &str) -> sqlx::Result<Option<(Email, String)>> {
    let mut tx = state.db.begin().await?;
    let Some(holder) = mail::verified_holder(&mut tx, email).await? else {
        return Ok(None);
    };
    // The user row first, then its tokens, like every email flow; two requests take turns.
    let open = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM email_tokens
               WHERE user_id = u.id AND purpose = 'reset_password'
                 AND used_at IS NULL AND expires_at > now()) AS "open!"
           FROM users u WHERE u.id = $1 FOR UPDATE"#,
        holder.user_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if open != Some(false)
        || state
            .limits
            .email_target
            .is_limited(&mail::target_key(&holder.email))
        || state
            .limits
            .reset_account
            .check_and_hit(&account_key(holder.user_id))
            .is_err()
    {
        return Ok(None);
    }
    let token = mail::issue_reset_token(&mut tx, holder.user_id, &holder.email).await?;
    tx.commit().await?;
    let mail = templates::reset_password(
        mail::site(&state.config),
        Recipient {
            to: &holder.email,
            username: &holder.username,
            locale: &holder.locale,
        },
        &mail::reset_link(&state.config, &token),
    );
    Ok(Some((mail, token)))
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
        (status = 400, body = ErrorBody, description = "`That link has expired or was already used`, `New password must be at least 8 characters`"),
        (status = 503, body = ErrorBody, description = "`The server is busy. Try again in a moment.`")
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
    replace_password(&mut tx, user_id, &new_hash).await?;
    tx.commit().await?;
    // The link proves the mailbox; failed guesses by someone else no longer lock the owner out.
    state.limits.login_failures.clear(&account_key(user_id));
    state.limits.login_failures.clear(&email_login_key(user_id));
    Ok(StatusCode::NO_CONTENT)
}

/// Stores `new_hash` as the account's password and ends what the old one opened: every session
/// and bearer token is deleted, and every open reset link and a pending address change are
/// voided. Runs inside the caller's transaction, which holds the user row (`FOR UPDATE`): the
/// reset confirm and [`set_password`].
async fn replace_password(
    conn: &mut PgConnection,
    user_id: i64,
    new_hash: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET password_hash = $2, password_changed_at = now() WHERE id = $1",
        user_id,
        new_hash
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", user_id)
        .execute(&mut *conn)
        .await?;
    mail::void_open_tokens(conn, user_id, TokenPurpose::ResetPassword).await?;
    // A pending address change may be the work of whoever knew the old password: the owner
    // starts it again if it was theirs.
    mail::void_open_tokens(conn, user_id, TokenPurpose::VerifyEmail).await?;
    Ok(())
}

/// The characters of a generated password: the invite codes' alphabet (Crockford base32, no
/// I, L, O or U), so it reads out and types without doubt.
const PASSWORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The length of a generated password: 20 characters of 5 bits each, 100 random bits.
const PASSWORD_LENGTH: usize = 20;

/// The password reset without mail (`bookclub-server set-password <username>`, the private move
/// #52): a new random password ([`PASSWORD_LENGTH`] characters of [`PASSWORD_ALPHABET`]),
/// hashed by `passwords` (the CLI passes `Passwords::new()`, as the server's sign-up uses), and
/// in one transaction what the reset confirm does ([`replace_password`]). Returns the password,
/// or `None` when no account has that name (normalised like every username lookup). A suspended
/// account gets the password and stays suspended.
///
/// The running server's in-memory login-failure buckets stay as they are (another process); they
/// empty within 10 minutes.
pub async fn set_password(
    conn: &mut PgConnection,
    passwords: &Passwords,
    username: &str,
) -> anyhow::Result<Option<String>> {
    let username = rules::normalize_username(username);
    let password = generate_password();
    // Hashed before the transaction, as in the confirm, so the row lock waits for no argon2.
    let new_hash = passwords.hash(&password).await?;
    let mut tx = conn.begin().await?;
    let Some(user_id) = sqlx::query_scalar!(
        "SELECT id FROM users WHERE username = $1 FOR UPDATE",
        username
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    replace_password(&mut tx, user_id, &new_hash).await?;
    tx.commit().await?;
    Ok(Some(password))
}

fn generate_password() -> String {
    let mut bytes = [0u8; PASSWORD_LENGTH];
    random::fill(&mut bytes);
    // 256 is a multiple of 32: every character is equally likely.
    bytes
        .iter()
        .map(|byte| PASSWORD_ALPHABET[usize::from(*byte) % PASSWORD_ALPHABET.len()] as char)
        .collect()
}

fn link_expired() -> AppError {
    AppError::bad_request(LINK_EXPIRED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_passwords_are_twenty_crockford_characters() {
        let a = generate_password();
        let b = generate_password();
        assert_eq!(a.len(), 20);
        assert!(
            a.bytes().all(|byte| PASSWORD_ALPHABET.contains(&byte)),
            "{a}"
        );
        assert!(rules::long_enough(&a));
        assert_ne!(a, b);
    }
}
