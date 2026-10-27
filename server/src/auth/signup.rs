//! `POST /api/auth/register` (endpoint-map §2, docs/phase1/conventions.md §5.5).

use axum::Json;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::mail::{self, Holder};
use super::session::{self, SessionKind};
use super::{rules, user_out};
use crate::AppState;
use crate::client_ip::ClientIp;
use crate::clubs::invites;
use crate::config::SignupMode;
use crate::dto::UserOut;
use crate::email::templates::{self, Attempt, Recipient};
use crate::error::{ApiResult, AppError, ErrorBody, is_unique_violation, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::ratelimit::ip_key;
use crate::text::py_strip;
use crate::tz;

const INVITE_NEEDED: &str = "Sign-up needs an invite code right now";

/// A new account.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterIn {
    /// Stripped and lowercased, then `^[a-z0-9_]{2,32}$`.
    pub username: String,
    pub email: String,
    /// At least 8 characters; never trimmed.
    pub password: String,
    /// Required while `SIGNUP_MODE=invite`: a club invite (it also joins the club) or a sign-up
    /// invite. Whitespace and `-` are ignored, case does not matter.
    #[serde(default)]
    pub invite_code: Option<String>,
    /// IANA zone for personal dates, e.g. from the browser; default `UTC`.
    #[serde(default)]
    pub timezone: Option<String>,
    /// `en` or `de`; default `en`.
    #[serde(default)]
    pub locale: Option<String>,
}

impl Validate for RegisterIn {
    fn validate(&mut self, v: &mut Validator) {
        self.username = rules::normalize_username(&self.username);
        v.check(rules::valid_username(&self.username), rules::USERNAME_RULE);
        self.email = py_strip(&self.email).to_owned();
        v.check(rules::valid_email(&self.email), rules::INVALID_EMAIL);
        v.check(rules::long_enough(&self.password), rules::PASSWORD_RULE);
        if let Some(zone) = &mut self.timezone {
            *zone = py_strip(zone).to_owned();
            v.check(tz::parse_zone(zone).is_ok(), msg::INVALID_TIME_ZONE);
        }
        if let Some(locale) = &mut self.locale {
            v.check(
                rules::allowed_choice(locale, &rules::LOCALES),
                rules::LOCALE_RULE,
            );
        }
    }
}

/// The one mail a sign-up sends after commit.
enum Outgoing {
    /// The address is free (or only typed by unverified accounts): a verification link.
    Verify(String),
    /// A verified account holds the address: that account gets a notice, nobody a link.
    Notice(Holder),
}

/// Creates an account, signs it in and sends the verification mail.
///
/// The answer never says whether the address is in use: when a verified account holds it, the
/// new account keeps the unverified address, no link is sent, and the holder gets the "someone
/// tried to use your address" notice. A club invite also joins the club; a sign-up invite only
/// unlocks sign-up. Called while signed in, it switches the session (the old one is deleted).
#[utoipa::path(
    post,
    path = "/api/auth/register",
    operation_id = "auth_register",
    tag = "auth",
    request_body = RegisterIn,
    responses(
        (status = 201, body = UserOut, description = "Created and signed in; `Set-Cookie` carries the session"),
        (status = 400, body = ErrorBody, description = "Invalid fields, a missing or invalid invite code"),
        (status = 409, body = ErrorBody, description = "`That username is already taken`, or `This club is full`"),
        (status = 429, body = ErrorBody, description = "Five sign-ups per IP address per hour")
    )
)]
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RegisterIn>,
) -> ApiResult<Response> {
    state
        .limits
        .signup_ip
        .check_and_hit(&ip_key(ip))
        .map_err(|wait| wait.into_error(msg::TOO_MANY_ATTEMPTS))?;
    if username_taken(&mut *state.db.acquire().await?, &body.username).await? {
        return Err(AppError::conflict(msg::USERNAME_TAKEN));
    }
    let code = body
        .invite_code
        .as_deref()
        .map(invites::normalize_code)
        .filter(|code| !code.is_empty());
    if code.is_none() && state.config.signup_mode == SignupMode::Invite {
        return Err(AppError::bad_request(INVITE_NEEDED));
    }
    let password_hash = state.passwords.hash(&body.password).await?;

    let mut tx = state.db.begin().await?;
    let user_id = insert_user(&mut tx, &body, &password_hash).await?;
    if let Some(code) = &code {
        let invite = invites::consume(&mut tx, code, user_id)
            .await?
            .ok_or_else(|| AppError::bad_request(msg::INVITE_INVALID))?;
        if let Some(club_id) = invite.club_id {
            invites::join(&mut tx, &invite, club_id, user_id).await?;
        }
    }
    let outgoing = match mail::verified_holder(&mut tx, &body.email).await? {
        Some(holder) => Outgoing::Notice(holder),
        None => Outgoing::Verify(mail::issue_verify_token(&mut tx, user_id, &body.email).await?),
    };
    let new_session = session::create(
        &mut tx,
        user_id,
        SessionKind::Cookie,
        &session::device_label(&headers),
    )
    .await?;
    if let Some((secret, SessionKind::Cookie)) =
        session::presented(&state.config, &headers).secret()
    {
        session::delete_by_secret(&mut tx, &secret, SessionKind::Cookie).await?;
    }
    let user = user_out(&mut tx, user_id).await?;
    tx.commit().await?;

    let site = mail::site(&state.config);
    let email = match &outgoing {
        Outgoing::Verify(token) => templates::verify_email(
            site,
            Recipient {
                to: &body.email,
                username: &user.username,
                locale: &user.locale,
            },
            &mail::verify_link(&state.config, token),
        ),
        Outgoing::Notice(holder) => templates::address_in_use(
            site,
            Recipient {
                to: &holder.email,
                username: &holder.username,
                locale: &holder.locale,
            },
            Attempt::SignUp,
        ),
    };
    mail::deliver(&state, email).await;

    let cookie = session::set_cookie(&state.config, &new_session.secret);
    Ok((StatusCode::CREATED, [(SET_COOKIE, cookie)], Json(user)).into_response())
}

async fn username_taken(conn: &mut PgConnection, username: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM users WHERE username = $1) AS "taken!""#,
        username
    )
    .fetch_one(conn)
    .await
}

/// Inserts the account with an unverified address. A concurrent sign-up of the same name is
/// 409, not 500.
async fn insert_user(
    conn: &mut PgConnection,
    body: &RegisterIn,
    password_hash: &str,
) -> ApiResult<i64> {
    sqlx::query_scalar!(
        "INSERT INTO users (username, email, password_hash, timezone, locale)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id",
        body.username,
        body.email,
        password_hash,
        body.timezone.as_deref().unwrap_or("UTC"),
        body.locale.as_deref().unwrap_or("en")
    )
    .fetch_one(conn)
    .await
    .map_err(|error| {
        if is_unique_violation(&error, "users_username_key") {
            AppError::conflict(msg::USERNAME_TAKEN)
        } else {
            error.into()
        }
    })
}
