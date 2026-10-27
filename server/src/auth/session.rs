//! Server-side sessions (docs/phase1/conventions.md §5.1, §5.2): cookie sessions for the web
//! and bearer tokens for the apps live in `sessions`, which keeps only the SHA-256 of each
//! 32-byte secret.
//!
//! The cookie value is `base64url_nopad(secret)`, a bearer token `bc_` plus the same encoding.
//! Validity is decided here, not by the browser: the cookie's Max-Age is the browser maximum,
//! and the row's `expires_at` slides on use.

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::{SECURE_SESSION_COOKIE, SESSION_COOKIE, random};
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
    /// The session cookie of the web app ([`cookie_name`]); idle expiry 30 days.
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
    /// The session cookie; `None` when its value does not decode to 32 bytes, or when the
    /// request carries the cookie more than once.
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

/// The session cookie's name: `__Host-bookclub_session` when the cookie is `Secure`, so the
/// browser accepts it only from this host (a page on a sibling subdomain cannot plant one);
/// plain `bookclub_session` otherwise, because the prefix requires `Secure`.
pub fn cookie_name(config: &Config) -> &'static str {
    if config.cookie_secure {
        SECURE_SESSION_COOKIE
    } else {
        SESSION_COOKIE
    }
}

/// Reads the bearer token (first) or the session cookie of a request.
///
/// The cookie is found by its name exactly as sent, never percent-decoded: the browser checks
/// the `__Host-` prefix on the raw name, so `%5F%5FHost-bookclub_session` is another cookie. A
/// request that carries the session cookie twice (one planted next to the real one) presents
/// a malformed cookie: it is not signed in as either.
pub fn presented(config: &Config, headers: &HeaderMap) -> Presented {
    if let Some(token) = bearer_token(headers) {
        return Presented::Bearer(token.strip_prefix(BEARER_PREFIX).and_then(decode_secret));
    }
    let name = cookie_name(config);
    let mut values = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .filter(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim());
    match (values.next(), values.next()) {
        (None, _) => Presented::Nothing,
        (Some(value), None) => Presented::Cookie(decode_secret(value)),
        (Some(_), Some(_)) => Presented::Cookie(None),
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
    let cookie = Cookie::build((cookie_name(config), value))
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
    if sets_session_cookie(&state.config, response.headers()) {
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

fn sets_session_cookie(config: &Config, headers: &HeaderMap) -> bool {
    let prefix = format!("{}=", cookie_name(config));
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
        assert!(set.starts_with("__Host-bookclub_session=abc;"), "{set}");
        assert!(!set.contains("Domain"), "{set}");
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
        let clear = clear_cookie(&config(true));
        assert!(
            clear
                .to_str()
                .unwrap()
                .starts_with("__Host-bookclub_session=;")
        );
    }

    /// Cookie tossing: a page on a sibling subdomain can set `bookclub_session` (or an encoded
    /// spelling of the `__Host-` name) for the parent domain, but never the `__Host-` cookie
    /// itself. Only that raw name counts, and a second copy of it is refused.
    #[test]
    fn only_the_raw_host_cookie_counts() {
        let secret = [7u8; 32];
        let encoded = BASE64_URL_SAFE_NO_PAD.encode(secret);
        let planted = BASE64_URL_SAFE_NO_PAD.encode([9u8; 32]);
        let cookie = |value: String| {
            let mut headers = HeaderMap::new();
            headers.insert(COOKIE, HeaderValue::from_str(&value).unwrap());
            headers
        };
        let secure = config(true);
        let real = cookie(format!("theme=x; __Host-bookclub_session={encoded}"));
        assert_eq!(presented(&secure, &real), Presented::Cookie(Some(secret)));
        for tossed in [
            format!("bookclub_session={planted}"),
            format!("%5F%5FHost-bookclub_session={planted}"),
            format!("__Host-bookclub%5Fsession={planted}"),
        ] {
            assert_eq!(presented(&secure, &cookie(tossed)), Presented::Nothing);
        }
        let twice = cookie(format!(
            "__Host-bookclub_session={encoded}; __Host-bookclub_session={planted}"
        ));
        assert_eq!(presented(&secure, &twice), Presented::Cookie(None));
        // Two Cookie headers (HTTP/2 sends one per cookie) count the same.
        let mut split = cookie(format!("__Host-bookclub_session={encoded}"));
        split.append(
            COOKIE,
            HeaderValue::from_str(&format!("__Host-bookclub_session={planted}")).unwrap(),
        );
        assert_eq!(presented(&secure, &split), Presented::Cookie(None));

        let plain = config(false);
        let dev = cookie(format!("bookclub_session={encoded}"));
        assert_eq!(presented(&plain, &dev), Presented::Cookie(Some(secret)));
        let doubled = cookie(format!(
            "bookclub_session={planted}; bookclub_session={encoded}"
        ));
        assert_eq!(presented(&plain, &doubled), Presented::Cookie(None));
        let encoded_name = cookie(format!("bookclub%5Fsession={planted}"));
        assert_eq!(presented(&plain, &encoded_name), Presented::Nothing);
    }

    #[test]
    fn the_bearer_header_wins_over_the_cookie() {
        let secret = [7u8; 32];
        let encoded = BASE64_URL_SAFE_NO_PAD.encode(secret);
        let cookie = format!("theme=x; bookclub_session={encoded}");
        let mut with_cookie = HeaderMap::new();
        with_cookie.insert("cookie", HeaderValue::from_str(&cookie).unwrap());
        let config = config(false);
        assert_eq!(
            presented(&config, &with_cookie),
            Presented::Cookie(Some(secret))
        );

        let mut both = with_cookie.clone();
        let bearer = format!("bearer bc_{encoded}");
        both.insert(AUTHORIZATION, HeaderValue::from_str(&bearer).unwrap());
        assert_eq!(presented(&config, &both), Presented::Bearer(Some(secret)));
        assert_eq!(
            presented(&config, &both).secret(),
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
            assert_eq!(presented(&config(false), &headers), expected, "{headers:?}");
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
