//! `POST /api/auth/login` and `POST /api/auth/logout` (endpoint-map §2), plus the credential
//! check that bearer token creation shares.

use std::net::IpAddr;

use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::account::{account_key, email_login_key};
use super::password::Verdict;
use super::rules;
use super::session::{self, SessionKind};
use crate::AppState;
use crate::client_ip::ClientIp;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::ratelimit::{RetryAfter, ip_key, unknown_identifier_key};

/// Username or verified email address, and password.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginIn {
    /// A username, or a verified email address (it contains `@`). Stripped and lowercased.
    pub username: String,
    pub password: String,
}

impl Validate for LoginIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.username = rules::normalize_username(&self.username);
    }
}

/// An account whose password was just verified.
pub(crate) struct Authenticated {
    pub user_id: i64,
    /// The stored hash the password was verified against.
    pub stored: String,
    /// A hash with the current parameters when `stored` uses old ones.
    pub rehash: Option<String>,
}

/// Signs in with a cookie session. Ten failures per account and identifier kind (or per unknown
/// name) in ten minutes, and thirty attempts per IP address, answer 429 before any password is
/// checked. A suspended account gets 403 `This account is suspended`, only once its password
/// matched.
#[utoipa::path(
    post,
    path = "/api/auth/login",
    operation_id = "auth_login",
    tag = "auth",
    request_body = LoginIn,
    responses(
        (status = 204, description = "Signed in; `Set-Cookie` carries the session"),
        (status = 400, body = ErrorBody, description = "Malformed body"),
        (status = 401, body = ErrorBody, description = "`Wrong username or password`"),
        (status = 403, body = ErrorBody, description = "`This account is suspended` (the password matched)"),
        (status = 429, body = ErrorBody, description = "`Too many attempts. Wait a few minutes.`"),
        (status = 503, body = ErrorBody, description = "`The server is busy. Try again in a moment.`")
    )
)]
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginIn>,
) -> ApiResult<Response> {
    let account = authenticate(&state, ip, &body.username, &body.password).await?;
    let mut tx = state.db.begin().await?;
    confirm(&mut tx, &account).await?;
    let new_session = session::create(
        &mut tx,
        account.user_id,
        SessionKind::Cookie,
        &session::device_label(&headers),
    )
    .await?;
    // Signing in while signed in switches the session.
    if let Some((secret, SessionKind::Cookie)) =
        session::presented(&state.config, &headers).secret()
    {
        session::delete_by_secret(&mut tx, &secret, SessionKind::Cookie).await?;
    }
    tx.commit().await?;
    let cookie = session::set_cookie(&state.config, &new_session.secret);
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response())
}

/// Deletes the session the request presents. The cookie is cleared only when a live cookie
/// session existed (parity: no `Set-Cookie` otherwise). Never fails for a missing session.
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    operation_id = "auth_logout",
    tag = "auth",
    responses((status = 204, description = "Signed out"))
)]
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let Some((secret, kind)) = session::presented(&state.config, &headers).secret() else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let live = session::delete_by_secret(&mut *state.db.acquire().await?, &secret, kind).await?;
    if live && kind == SessionKind::Cookie {
        let cookie = session::clear_cookie(&state.config);
        return Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Checks a username (or verified email) and password under the login limits (§10.1):
///
/// 1. 30 attempts per IP address (`ip_key`) per 10 minutes.
/// 2. 10 failures per identifier per 10 minutes, counted with `check_and_hit` **before** the
///    password is verified, so concurrent guesses cannot pass together; a success clears the
///    key. The key is `u:<id>` for a username, `u:<id>:email` for a verified address and
///    `unknown_identifier_key` (in `login_unknown`) for an identifier that names no account.
///    Every identifier thus fills its own bucket, whether or not it names an account: neither a
///    filled bucket nor a fresh one tells an address in use from a free one, or links a username
///    to its address.
///
/// An unknown account verifies against a dummy hash, so it takes as long as a real one. The
/// caller creates the session in a transaction that starts with [`confirm`].
pub(crate) async fn authenticate(
    state: &AppState,
    ip: IpAddr,
    identifier: &str,
    password: &str,
) -> ApiResult<Authenticated> {
    state
        .limits
        .login_ip
        .check_and_hit(&ip_key(ip))
        .map_err(too_many)?;
    let account = find_account(&mut *state.db.acquire().await?, identifier).await?;
    let (failures, key) = match &account {
        Some((id, _)) if identifier.contains('@') => {
            (&state.limits.login_failures, email_login_key(*id))
        }
        Some((id, _)) => (&state.limits.login_failures, account_key(*id)),
        None => (
            &state.limits.login_unknown,
            unknown_identifier_key(identifier),
        ),
    };
    failures.check_and_hit(&key).map_err(too_many)?;
    let stored = account.as_ref().map(|(_, hash)| hash.as_str());
    let verdict = state.passwords.verify(password, stored).await?;
    let (Verdict::Match { rehash }, Some((user_id, stored))) = (verdict, account) else {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            msg::WRONG_CREDENTIALS,
        ));
    };
    failures.clear(&key);
    let rehash = if rehash {
        Some(state.passwords.hash(password).await?)
    } else {
        None
    };
    Ok(Authenticated {
        user_id,
        stored,
        rehash,
    })
}

/// The first step of the transaction that creates the session of an [`authenticate`]d login:
/// locks the user row and checks that the verified hash is still the stored one and that the
/// account is not suspended (403 `This account is suspended`, #55), then stores the rehash.
/// Login and bearer token creation share it. A suspension that commits first is seen here; one
/// that commits later waits for this transaction and deletes the new session.
///
/// The password was verified outside any transaction. A password change or reset that
/// committed since then answers 401 `Wrong username or password` here; one that commits later
/// waits for this transaction, so its session revocation sees the new session. Either way no
/// session made with the old password outlives the change. The lock is `FOR NO KEY UPDATE`,
/// not `FOR SHARE`, so two logins of one account that both rehash cannot deadlock.
pub(crate) async fn confirm(conn: &mut PgConnection, account: &Authenticated) -> ApiResult<()> {
    let current = sqlx::query!(
        r#"SELECT password_hash, suspended_at IS NOT NULL AS "suspended!"
           FROM users WHERE id = $1 FOR NO KEY UPDATE"#,
        account.user_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(current) = current.filter(|row| row.password_hash == account.stored) else {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            msg::WRONG_CREDENTIALS,
        ));
    };
    if current.suspended {
        return Err(AppError::forbidden(msg::ACCOUNT_SUSPENDED));
    }
    if let Some(new) = &account.rehash {
        sqlx::query!(
            "UPDATE users SET password_hash = $2 WHERE id = $1",
            account.user_id,
            new
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The account a login identifier names: an address (it contains `@`) only when verified,
/// compared like `users_verified_email_key`; otherwise the username.
async fn find_account(
    conn: &mut PgConnection,
    identifier: &str,
) -> sqlx::Result<Option<(i64, String)>> {
    let row = if identifier.contains('@') {
        sqlx::query!(
            "SELECT id, password_hash FROM users
             WHERE lower(email) = lower($1) AND email_verified_at IS NOT NULL",
            identifier
        )
        .fetch_optional(conn)
        .await?
        .map(|row| (row.id, row.password_hash))
    } else {
        sqlx::query!(
            "SELECT id, password_hash FROM users WHERE username = $1",
            identifier
        )
        .fetch_optional(conn)
        .await?
        .map(|row| (row.id, row.password_hash))
    };
    Ok(row)
}

/// 429 `Too many attempts. Wait a few minutes.` with `Retry-After`.
pub(crate) fn too_many(wait: RetryAfter) -> AppError {
    wait.into_error(msg::TOO_MANY_ATTEMPTS)
}
