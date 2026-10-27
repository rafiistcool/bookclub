//! Server-side sessions (docs/phase1/conventions.md §5.1, §5.2): cookie sessions for the web
//! and bearer tokens for the apps live in `sessions`, which keeps only the SHA-256 of each
//! 32-byte secret.
//!
//! The cookie value is `base64url_nopad(secret)`, a bearer token `bc_` plus the same encoding.
//! Validity is decided here, not by the browser: the cookie's Max-Age is the browser maximum,
//! and the row's `expires_at` slides on use.

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::{SESSION_COOKIE, random};
use crate::AppState;
use crate::config::Config;
use crate::text::truncate_chars;

/// The cookie's Max-Age: 400 days, the most browsers keep. The server decides validity.
pub const COOKIE_MAX_AGE_SECONDS: i64 = 400 * 24 * 60 * 60;
/// Prefix of every bearer token.
pub const BEARER_PREFIX: &str = "bc_";
/// Longest stored session label (`sessions.label`), in characters.
pub const MAX_LABEL_CHARS: usize = 200;

/// How a session authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// The `bookclub_session` cookie of the web app; idle expiry 30 days.
    Cookie,
    /// An `Authorization: Bearer bc_…` token of the apps; idle expiry 90 days.
    Bearer,
}

/// A session just stored, with the secret the client presents from now on.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: i64,
    /// The cookie value, or the full `bc_…` bearer token.
    pub secret: String,
    /// The stored label, cut to [`MAX_LABEL_CHARS`].
    pub label: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: OffsetDateTime,
}

/// The secret a request presents. A bearer header is read first and wins over the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presented {
    /// No bearer header and no session cookie.
    Nothing,
    /// An `Authorization: Bearer …` header; `None` when it is not a well-formed `bc_` token.
    Bearer(Option<[u8; 32]>),
    /// The session cookie; `None` when its value does not decode to 32 bytes.
    Cookie(Option<[u8; 32]>),
}

impl Presented {
    /// The secret and its session kind, when one was presented and is well-formed.
    pub fn secret(self) -> Option<([u8; 32], SessionKind)> {
        match self {
            Presented::Bearer(Some(raw)) => Some((raw, SessionKind::Bearer)),
            Presented::Cookie(Some(raw)) => Some((raw, SessionKind::Cookie)),
            _ => None,
        }
    }
}

/// Reads the bearer token (first) or the session cookie of a request.
pub fn presented(headers: &HeaderMap) -> Presented {
    if let Some(token) = bearer_token(headers) {
        return Presented::Bearer(token.strip_prefix(BEARER_PREFIX).and_then(decode_secret));
    }
    match CookieJar::from_headers(headers).get(SESSION_COOKIE) {
        Some(cookie) => Presented::Cookie(decode_secret(cookie.value())),
        None => Presented::Nothing,
    }
}

/// The credentials of an `Authorization: Bearer <credentials>` header (scheme case-insensitive).
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, credentials) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| credentials.trim())
}

fn decode_secret(encoded: &str) -> Option<[u8; 32]> {
    BASE64_URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
}

/// `sessions.token_hash` of a secret.
pub fn token_hash(secret: &[u8; 32]) -> Vec<u8> {
    Sha256::digest(secret).to_vec()
}

/// Stores a new session with a fresh secret. `label` is cut to [`MAX_LABEL_CHARS`]: the
/// User-Agent for cookies, the app's label for bearer tokens.
pub async fn create(
    conn: &mut PgConnection,
    user_id: i64,
    kind: SessionKind,
    label: &str,
) -> sqlx::Result<NewSession> {
    let mut secret = [0u8; 32];
    random::fill(&mut secret);
    let label = truncate_chars(label.trim(), MAX_LABEL_CHARS);
    let row = sqlx::query!(
        r#"INSERT INTO sessions (user_id, kind, token_hash, label, expires_at)
           VALUES ($1, $2, $3, $4,
                   now() + CASE $2 WHEN 'cookie' THEN interval '30 days' ELSE interval '90 days' END)
           RETURNING id, label, created_at, last_used_at"#,
        user_id,
        kind as SessionKind,
        token_hash(&secret),
        label
    )
    .fetch_one(conn)
    .await?;
    let encoded = BASE64_URL_SAFE_NO_PAD.encode(secret);
    Ok(NewSession {
        id: row.id,
        secret: match kind {
            SessionKind::Cookie => encoded,
            SessionKind::Bearer => format!("{BEARER_PREFIX}{encoded}"),
        },
        label: row.label,
        created_at: row.created_at,
        last_used_at: row.last_used_at,
    })
}

/// Deletes the session behind a secret. Returns whether a live (unexpired) session was deleted;
/// an expired row is deleted too but does not count.
pub async fn delete_by_secret(
    conn: &mut PgConnection,
    secret: &[u8; 32],
    kind: SessionKind,
) -> sqlx::Result<bool> {
    let live = sqlx::query_scalar!(
        r#"DELETE FROM sessions WHERE token_hash = $1 AND kind = $2
           RETURNING expires_at > now() AS "live!""#,
        token_hash(secret),
        kind as SessionKind
    )
    .fetch_optional(conn)
    .await?;
    Ok(live.unwrap_or(false))
}

/// The label of a new cookie session: the request's User-Agent ([`create`] cuts it to
/// [`MAX_LABEL_CHARS`]).
pub fn device_label(headers: &HeaderMap) -> String {
    headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The `Set-Cookie` value that stores a session cookie.
pub fn set_cookie(config: &Config, value: &str) -> HeaderValue {
    cookie_header(config, value.to_owned(), COOKIE_MAX_AGE_SECONDS)
}

/// The `Set-Cookie` value that deletes the session cookie.
pub fn clear_cookie(config: &Config) -> HeaderValue {
    cookie_header(config, String::new(), 0)
}

fn cookie_header(config: &Config, value: String, max_age: i64) -> HeaderValue {
    let cookie = Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(config.cookie_secure)
        .max_age(time::Duration::seconds(max_age))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("a session cookie is a valid header value")
}

/// What the session middleware should do with the cookie once the response is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CookieAction {
    /// The request's cookie names no live session: clear it on a 401.
    Clear,
    /// The session slid (§5.1): send the cookie again, so the browser's 400 days restart too.
    Renew(String),
}

/// A per-request slot the session extractor fills and [`maintain_cookie`] reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct CookieSlot(Arc<Mutex<Option<CookieAction>>>);

impl CookieSlot {
    pub(crate) fn set(&self, action: CookieAction) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(action);
    }

    fn take(&self) -> Option<CookieAction> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// Middleware (lib.rs, inside the origin check): lets the session extractor answer with a
/// cookie although extractors cannot touch the response.
///
/// - A request whose cookie did not resolve gets a clearing `Set-Cookie` on its 401.
/// - A cookie session that slid gets its cookie again.
///
/// A handler that sets the session cookie itself (login, logout) always wins.
pub async fn maintain_cookie(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let slot = CookieSlot::default();
    request.extensions_mut().insert(slot.clone());
    let mut response = next.run(request).await;
    let Some(action) = slot.take() else {
        return response;
    };
    if sets_session_cookie(response.headers()) {
        return response;
    }
    let header = match action {
        CookieAction::Clear if response.status() == StatusCode::UNAUTHORIZED => {
            clear_cookie(&state.config)
        }
        CookieAction::Clear => return response,
        CookieAction::Renew(value) => set_cookie(&state.config, &value),
    };
    response.headers_mut().append(SET_COOKIE, header);
    response
}

fn sets_session_cookie(headers: &HeaderMap) -> bool {
    let prefix = format!("{SESSION_COOKIE}=");
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.starts_with(&prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(secure: bool) -> Config {
        let pairs = [
            ("DATABASE_URL", "postgres://x"),
            ("DEV_MODE", "true"),
            ("COOKIE_SECURE", if secure { "true" } else { "false" }),
        ];
        Config::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
        .unwrap()
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn cookies_carry_the_documented_attributes() {
        let set = set_cookie(&config(true), "abc");
        let set = set.to_str().unwrap();
        assert!(set.starts_with("bookclub_session=abc;"), "{set}");
        for part in [
            "HttpOnly",
            "SameSite=Lax",
            "Secure",
            "Path=/",
            "Max-Age=34560000",
        ] {
            assert!(set.contains(part), "{set} lacks {part}");
        }
        let clear = clear_cookie(&config(false));
        let clear = clear.to_str().unwrap();
        assert!(clear.starts_with("bookclub_session=;"), "{clear}");
        assert!(clear.contains("Max-Age=0"));
        assert!(!clear.contains("Secure"));
    }

    #[test]
    fn the_bearer_header_wins_over_the_cookie() {
        let secret = [7u8; 32];
        let encoded = BASE64_URL_SAFE_NO_PAD.encode(secret);
        let cookie = format!("theme=x; bookclub_session={encoded}");
        let mut with_cookie = HeaderMap::new();
        with_cookie.insert("cookie", HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(presented(&with_cookie), Presented::Cookie(Some(secret)));

        let mut both = with_cookie.clone();
        let bearer = format!("bearer bc_{encoded}");
        both.insert(AUTHORIZATION, HeaderValue::from_str(&bearer).unwrap());
        assert_eq!(presented(&both), Presented::Bearer(Some(secret)));
        assert_eq!(
            presented(&both).secret(),
            Some((secret, SessionKind::Bearer))
        );
    }

    #[test]
    fn malformed_secrets_are_recognised_but_unusable() {
        let cases = [
            (
                headers(&[("authorization", "Bearer nope")]),
                Presented::Bearer(None),
            ),
            (
                headers(&[("authorization", "Bearer bc_short")]),
                Presented::Bearer(None),
            ),
            (
                headers(&[("cookie", "bookclub_session=%%%")]),
                Presented::Cookie(None),
            ),
            (headers(&[("cookie", "theme=x")]), Presented::Nothing),
            (
                headers(&[("authorization", "Basic YWRhOnB3")]),
                Presented::Nothing,
            ),
            (headers(&[]), Presented::Nothing),
        ];
        for (headers, expected) in cases {
            assert_eq!(presented(&headers), expected, "{headers:?}");
            assert_eq!(expected.secret(), None);
        }
    }

    #[test]
    fn token_hashes_are_sha256() {
        let hash = token_hash(&[0u8; 32]);
        assert_eq!(hash.len(), 32);
        assert_eq!(
            hash[..4],
            [0x66, 0x68, 0x7a, 0xad],
            "SHA-256 of 32 zero bytes"
        );
    }
}
