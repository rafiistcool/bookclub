//! One-time email tokens and mail delivery (docs/phase1/conventions.md §5.5), shared by sign-up
//! and the email flows (verification, resend, email change, password reset).
//!
//! A token is 32 random bytes. The link carries it base64url-encoded in the URL fragment, which
//! browsers never send to a server, a proxy log or a `Referer`; `email_tokens` keeps its SHA-256.

use std::time::Duration;

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::random;
use crate::AppState;
use crate::config::Config;
use crate::email::Email;
use crate::email::templates::Site;
use crate::text::casefold;

/// The longest a request waits for the mail transport (lettre's own SMTP timeout is a minute).
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// What an email token is for (`email_tokens.purpose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TokenPurpose {
    /// Verifies `email_tokens.email`; valid 48 hours.
    VerifyEmail,
    /// Resets the password; valid one hour, sent only to a verified address.
    ResetPassword,
}

/// A verified account that holds an address, with what a notice to it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub user_id: i64,
    pub username: String,
    pub locale: String,
    /// The address as the holder stored it.
    pub email: String,
}

/// Issues a `verify_email` token for `email` and marks every older open one of the user used,
/// so at most one is open (§5.5). Returns the token for [`verify_link`].
pub async fn issue_verify_token(
    conn: &mut PgConnection,
    user_id: i64,
    email: &str,
) -> sqlx::Result<String> {
    void_open_tokens(conn, user_id, TokenPurpose::VerifyEmail).await?;
    issue(conn, user_id, TokenPurpose::VerifyEmail, email).await
}

/// Issues a `reset_password` token for the verified `email`. Returns the token for
/// [`reset_link`].
pub async fn issue_reset_token(
    conn: &mut PgConnection,
    user_id: i64,
    email: &str,
) -> sqlx::Result<String> {
    issue(conn, user_id, TokenPurpose::ResetPassword, email).await
}

async fn issue(
    conn: &mut PgConnection,
    user_id: i64,
    purpose: TokenPurpose,
    email: &str,
) -> sqlx::Result<String> {
    let mut secret = [0u8; 32];
    random::fill(&mut secret);
    sqlx::query!(
        r#"INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
           VALUES ($1, $2, $3, $4,
                   now() + CASE $2 WHEN 'verify_email' THEN interval '48 hours'
                                   ELSE interval '1 hour' END)"#,
        user_id,
        purpose as TokenPurpose,
        email,
        Sha256::digest(secret).to_vec()
    )
    .execute(conn)
    .await?;
    Ok(BASE64_URL_SAFE_NO_PAD.encode(secret))
}

/// Marks every open (unused, unexpired) token of `purpose` of the user used.
pub async fn void_open_tokens(
    conn: &mut PgConnection,
    user_id: i64,
    purpose: TokenPurpose,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE email_tokens SET used_at = now()
         WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()",
        user_id,
        purpose as TokenPurpose
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// `email_tokens.token_hash` of a token from a link; `None` when the token is not base64url of
/// 32 bytes (such a token names nothing).
pub fn link_token_hash(token: &str) -> Option<Vec<u8>> {
    let secret: [u8; 32] = BASE64_URL_SAFE_NO_PAD
        .decode(token.trim())
        .ok()?
        .try_into()
        .ok()?;
    Some(Sha256::digest(secret).to_vec())
}

/// `{PUBLIC_URL}/verify-email#token=…`.
pub fn verify_link(config: &Config, token: &str) -> String {
    format!("{}/verify-email#token={token}", config.public_url)
}

/// `{PUBLIC_URL}/reset-password#token=…`.
pub fn reset_link(config: &Config, token: &str) -> String {
    format!("{}/reset-password#token={token}", config.public_url)
}

/// The template values of this service.
pub fn site(config: &Config) -> Site<'_> {
    Site {
        service: &config.service_name,
        public_url: &config.public_url,
    }
}

/// The verified account holding `email` (compared like `users_verified_email_key`:
/// `lower(email)`), if any.
pub async fn verified_holder(conn: &mut PgConnection, email: &str) -> sqlx::Result<Option<Holder>> {
    sqlx::query_as!(
        Holder,
        r#"SELECT id AS user_id, username, locale, email AS "email!"
           FROM users
           WHERE lower(email) = lower($1) AND email_verified_at IS NOT NULL"#,
        email
    )
    .fetch_optional(conn)
    .await
}

/// Sends a mail after the transaction committed. Every mail counts against the per-address
/// `email_target` limit (6 a day, case-folded, across all accounts); over it the mail is
/// silently dropped. A send failure, or a send slower than [`SEND_TIMEOUT`], is logged and never
/// fails the request.
pub async fn deliver(state: &AppState, email: Email) {
    if state
        .limits
        .email_target
        .check_and_hit(&casefold(email.to.trim()))
        .is_err()
    {
        tracing::info!("mail not sent: the daily limit for this address is reached");
        return;
    }
    match tokio::time::timeout(SEND_TIMEOUT, state.email.send(email)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(error = %format!("{error:#}"), "sending mail failed"),
        Err(_) => tracing::warn!("sending mail timed out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_tokens_hash_only_when_well_formed() {
        let token = BASE64_URL_SAFE_NO_PAD.encode([1u8; 32]);
        assert_eq!(
            link_token_hash(&token),
            Some(Sha256::digest([1u8; 32]).to_vec())
        );
        assert_eq!(
            link_token_hash(&format!(" {token}\n")),
            link_token_hash(&token)
        );
        assert_eq!(link_token_hash("short"), None);
        assert_eq!(link_token_hash(""), None);
        assert_eq!(link_token_hash(&format!("{token}=")), None);
    }
}
