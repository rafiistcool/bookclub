//! The account's email address (endpoint-map §2, docs/phase1/conventions.md §5.5): verifying a
//! link, sending a new link, changing the address and dismissing the "add your email" prompt.
//!
//! Every flow here locks the `users` row before it touches the user's `email_tokens` rows, so
//! two flows of one account cannot deadlock.
//!
//! The newest verification link of an account is the one with the highest `id`. Every flow that
//! issues one holds the `users` row lock (sign-up creates the row in the same transaction), and
//! identity values are taken at insert time, so ids follow the order in which links are issued.
//! `created_at` does not: it is the start of the issuing transaction, which can be older than a
//! link another transaction issued and committed while this one waited for the lock.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::account::{account_key, verify_current_password};
use super::extract::CurrentUser;
use super::mail::{self, LINK_EXPIRED, MailKind, TokenPurpose};
use super::rules;
use crate::AppState;
use crate::email::Email;
use crate::email::templates::{self, Attempt, Recipient};
use crate::error::{ApiResult, AppError, ErrorBody, is_unique_violation, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::text::py_strip;

/// 409 on verify: another account verified the address first. Only a link holder sees it.
pub const EMAIL_IN_USE: &str = "That email is already in use";
/// 400 on resend for an account without an address (an imported user).
pub const ADD_EMAIL_FIRST: &str = "Add an email address first";

/// The token from a `/verify-email#token=…` link.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailIn {
    pub token: String,
}

impl Validate for VerifyEmailIn {}

/// What a verification link would confirm, shown before the reader confirms it.
#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyEmailPreview {
    /// The account the address would belong to.
    pub username: String,
    /// The address the link verifies.
    pub email: String,
}

/// A new address for the account, confirmed with the current password.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EmailChangeIn {
    /// A plain `local@domain` address; surrounding spaces are ignored.
    pub email: String,
    pub current_password: String,
}

impl Validate for EmailChangeIn {
    fn validate(&mut self, v: &mut Validator) {
        self.email = py_strip(&self.email).to_owned();
        v.check(rules::valid_email(&self.email), rules::INVALID_EMAIL);
    }
}

/// The account row an email flow works on, locked `FOR UPDATE`.
struct Account {
    username: String,
    locale: String,
    email: Option<String>,
    verified: bool,
    password_hash: String,
    /// The address of the newest open verification link.
    open_change: Option<String>,
}

impl Account {
    fn recipient<'a>(&'a self, to: &'a str) -> Recipient<'a> {
        Recipient {
            to,
            username: &self.username,
            locale: &self.locale,
        }
    }
}

/// The account, locked; `None` when it no longer exists.
async fn lock_account(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<Option<Account>> {
    sqlx::query_as!(
        Account,
        r#"SELECT u.username, u.locale, u.email, u.email_verified_at IS NOT NULL AS "verified!",
                  u.password_hash,
                  (SELECT t.email FROM email_tokens t
                   WHERE t.user_id = u.id AND t.purpose = 'verify_email'
                     AND t.used_at IS NULL AND t.expires_at > now()
                   ORDER BY t.id DESC LIMIT 1) AS open_change
           FROM users u
           WHERE u.id = $1
           FOR UPDATE"#,
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// Verifies the address of a link. Public: the link works on any device and signs nobody in.
///
/// The token must be open (unused, unexpired), the account's newest verification link, and for
/// the address still pending: the account's own address while it is unverified, else the
/// address of its email change. Success makes the token's address the verified account address
/// and voids every other open verification and reset link of the account.
#[utoipa::path(
    post,
    path = "/api/auth/verify-email",
    operation_id = "auth_verify_email",
    tag = "auth",
    request_body = VerifyEmailIn,
    responses(
        (status = 204, description = "The address is verified"),
        (status = 400, body = ErrorBody, description = "`That link has expired or was already used`"),
        (status = 409, body = ErrorBody, description = "`That email is already in use`: another account verified the address first")
    )
)]
pub async fn verify_email(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VerifyEmailIn>,
) -> ApiResult<StatusCode> {
    let hash = mail::link_token_hash(&body.token).ok_or_else(link_expired)?;
    let mut tx = state.db.begin().await?;
    let user_id = sqlx::query_scalar!(
        "SELECT user_id FROM email_tokens WHERE token_hash = $1 AND purpose = 'verify_email'",
        hash
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(link_expired)?;
    let account = lock_account(&mut tx, user_id)
        .await?
        .ok_or_else(link_expired)?;
    let token = sqlx::query!(
        r#"SELECT t.email FROM email_tokens t
           WHERE t.token_hash = $1 AND t.purpose = 'verify_email'
             AND t.used_at IS NULL AND t.expires_at > now()
             AND NOT EXISTS (
                 SELECT 1 FROM email_tokens n
                 WHERE n.user_id = t.user_id AND n.purpose = 'verify_email'
                   AND n.id > t.id)"#,
        hash
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(link_expired)?;
    let still_pending = account.verified || account.email.as_deref() == Some(&token.email);
    if !still_pending {
        return Err(link_expired());
    }
    sqlx::query!(
        "UPDATE users SET email = $2, email_verified_at = now() WHERE id = $1",
        user_id,
        token.email
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if is_unique_violation(&error, "users_verified_email_key") {
            AppError::conflict(EMAIL_IN_USE)
        } else {
            error.into()
        }
    })?;
    // This link, every other open verification link and every open reset link.
    sqlx::query!(
        "UPDATE email_tokens SET used_at = now()
         WHERE user_id = $1 AND used_at IS NULL AND expires_at > now()",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What a verification link would do, without doing it: the account and the address. Public,
/// like the verification itself, and read-only. The verify page shows it and verifies only when
/// the reader confirms, so a link scanner that opens the page, or a click on a link someone
/// else's sign-up sent, never verifies an address for an account its owner does not know.
///
/// The same links are accepted as by `POST /api/auth/verify-email`: open, the account's newest,
/// and for the address still pending.
#[utoipa::path(
    post,
    path = "/api/auth/verify-email/preview",
    operation_id = "auth_preview_verification",
    tag = "auth",
    request_body = VerifyEmailIn,
    responses(
        (status = 200, body = VerifyEmailPreview, description = "The account and the address the link would verify"),
        (status = 400, body = ErrorBody, description = "`That link has expired or was already used`")
    )
)]
pub async fn preview_verification(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<VerifyEmailIn>,
) -> ApiResult<Json<VerifyEmailPreview>> {
    let hash = mail::link_token_hash(&body.token).ok_or_else(link_expired)?;
    let preview = sqlx::query_as!(
        VerifyEmailPreview,
        r#"SELECT u.username, t.email
           FROM email_tokens t
           JOIN users u ON u.id = t.user_id
           WHERE t.token_hash = $1 AND t.purpose = 'verify_email'
             AND t.used_at IS NULL AND t.expires_at > now()
             AND NOT EXISTS (
                 SELECT 1 FROM email_tokens n
                 WHERE n.user_id = t.user_id AND n.purpose = 'verify_email'
                   AND n.id > t.id)
             AND (u.email_verified_at IS NOT NULL OR u.email = t.email)"#,
        hash
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(link_expired)?;
    Ok(Json(preview))
}

/// Sends a new verification link for the pending address (the account's own unverified
/// address, or the address of an email change) and voids the previous link.
///
/// Shares the per-account limit with `PUT /api/auth/me/email` (3 an hour). A verified account
/// with no email change waiting has nothing to verify: 204, nothing is sent and nothing counts.
/// When a verified account holds the pending address, the new link is not sent. The answer does
/// not wait for the mail, which goes out in a background task: only a free address gets one, so
/// the time a send takes would tell whether an address is in use.
#[utoipa::path(
    post,
    path = "/api/auth/verify-email/resend",
    operation_id = "auth_resend_verification",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "A new link is on its way, or nothing waits for verification"),
        (status = 400, body = ErrorBody, description = "`Add an email address first`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn resend_verification(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let account = lock_account(&mut tx, user.id)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    let pending = match (&account.email, account.verified) {
        (None, _) => return Err(AppError::bad_request(ADD_EMAIL_FIRST)),
        (Some(own), false) => own.clone(),
        (Some(_), true) => match &account.open_change {
            Some(change) => change.clone(),
            None => return Ok(StatusCode::NO_CONTENT),
        },
    };
    state
        .limits
        .email_send_user
        .check_and_hit(&account_key(user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let token = mail::issue_verify_token(&mut tx, user.id, &pending).await?;
    let held = mail::verified_holder(&mut tx, &pending).await?.is_some();
    tx.commit().await?;
    if !held {
        let link = mail::verify_link(&state.config, &token);
        let email = templates::verify_email(
            mail::site(&state.config),
            account.recipient(&pending),
            &link,
        );
        tokio::spawn(async move { mail::deliver(&state, email, MailKind::Link).await });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Changes the account's address. The answer is the same whether or not the address is in use.
///
/// - The per-account limit (3 an hour) is checked first, then the current password (under the
///   current-password limits), and only then is the change counted.
/// - Without an address, or with an unverified one, the new address replaces it at once (still
///   unverified) and gets a verification link.
/// - A verified address stays until the new one is verified: the link goes to the new address,
///   `UserOut.pending_email` shows it, and the old address gets an "email changed" notice.
/// - When a verified account holds the new address, nobody gets a link; that account gets the
///   "someone tried to use your address" notice instead. Everything else happens as for a free
///   address, `pending_email` included.
/// - The account's own verified address (case-insensitive) cancels a pending change and sends
///   nothing.
///
/// Every earlier open verification link and every open reset link is voided. Shares the
/// per-account limit with resend (3 an hour); mail to one address is also limited (6 links and
/// 6 of each notice a day, silently).
#[utoipa::path(
    put,
    path = "/api/auth/me/email",
    operation_id = "auth_change_email",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = EmailChangeIn,
    responses(
        (status = 204, description = "Accepted; a verification link is on its way"),
        (status = 400, body = ErrorBody, description = "`Enter a valid email address`, `Current password is wrong`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (3 an hour), or `Too many attempts. Wait a few minutes.` (current-password checks)"),
        (status = 503, body = ErrorBody, description = "`The server is busy. Try again in a moment.`")
    )
)]
pub async fn change_email(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<EmailChangeIn>,
) -> ApiResult<StatusCode> {
    // Checked first without counting, so a request the limit refuses costs no argon2 work and
    // a mistyped password does not use up a change; counted once the password is right.
    let send_key = account_key(user.id);
    state
        .limits
        .email_send_user
        .check(&send_key)
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let verified_hash = verify_current_password(&state, &user, &body.current_password).await?;
    state
        .limits
        .email_send_user
        .check_and_hit(&send_key)
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;

    let mut tx = state.db.begin().await?;
    let account = lock_account(&mut tx, user.id)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    if account.password_hash != verified_hash {
        // A password change or reset committed since the check: it wins.
        return Err(AppError::bad_request(msg::CURRENT_PASSWORD_WRONG));
    }
    mail::void_open_tokens(&mut tx, user.id, TokenPurpose::ResetPassword).await?;
    let own_verified = account.verified
        && sqlx::query_scalar!(
            r#"SELECT lower(email) = lower($2) AS "same!" FROM users WHERE id = $1"#,
            user.id,
            body.email
        )
        .fetch_one(&mut *tx)
        .await?;
    if own_verified {
        mail::void_open_tokens(&mut tx, user.id, TokenPurpose::VerifyEmail).await?;
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    if !account.verified {
        sqlx::query!(
            "UPDATE users SET email = $2 WHERE id = $1",
            user.id,
            body.email
        )
        .execute(&mut *tx)
        .await?;
    }
    let token = mail::issue_verify_token(&mut tx, user.id, &body.email).await?;
    let holder = mail::verified_holder(&mut tx, &body.email).await?;
    tx.commit().await?;

    let site = mail::site(&state.config);
    let mut outgoing: Vec<(Email, MailKind)> = Vec::new();
    if let (true, Some(old)) = (account.verified, &account.email) {
        outgoing.push((
            templates::email_changed(site, account.recipient(old), &body.email),
            MailKind::EmailChanged,
        ));
    }
    outgoing.push(match &holder {
        Some(holder) => (
            templates::address_in_use(
                site,
                Recipient {
                    to: &holder.email,
                    username: &holder.username,
                    locale: &holder.locale,
                },
                Attempt::EmailChange,
            ),
            MailKind::AddressInUse,
        ),
        None => (
            templates::verify_email(
                site,
                account.recipient(&body.email),
                &mail::verify_link(&state.config, &token),
            ),
            MailKind::Link,
        ),
    });
    for (email, kind) in outgoing {
        mail::deliver(&state, email, kind).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Hides the one-time "add your email" prompt of an account without an address
/// (`UserOut.needs_email_prompt` becomes false). The first dismissal's time is kept.
#[utoipa::path(
    post,
    path = "/api/auth/me/email-prompt/dismiss",
    operation_id = "auth_dismiss_email_prompt",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "Dismissed"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn dismiss_email_prompt(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    sqlx::query!(
        "UPDATE users SET email_prompt_dismissed_at = coalesce(email_prompt_dismissed_at, now())
         WHERE id = $1",
        user.id
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn link_expired() -> AppError {
    AppError::bad_request(LINK_EXPIRED)
}
