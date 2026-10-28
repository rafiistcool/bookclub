//! The origin check (docs/phase1/conventions.md §5.8): a cookie-authenticated unsafe request
//! from a foreign `Origin` is refused, on top of `SameSite=Lax` and the JSON content type.

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::config::Config;
use crate::error::{AppError, msg};

/// Middleware: 403 `Cross-site request refused` for the requests [`refuses`] names.
pub async fn check(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if refuses(&state.config, request.method(), request.headers()) {
        return AppError::forbidden(msg::CROSS_SITE_REFUSED).into_response();
    }
    next.run(request).await
}

/// True for an unsafe-method request that may authenticate by cookie (no bearer token, and any
/// `Cookie` header) and carries an `Origin` other than `PUBLIC_URL`, a `CORS_ORIGINS` entry, or
/// the Vite origin in DEV_MODE.
///
/// Any cookie counts, not only one this check would parse as `bookclub_session`: the check fails
/// closed instead of depending on how the session extractor decodes cookie names
/// (`bookclub%5Fsession` is the same cookie to a percent-decoding parser).
pub fn refuses(config: &Config, method: &Method, headers: &HeaderMap) -> bool {
    if method.is_safe() || has_bearer(headers) || !headers.contains_key(COOKIE) {
        return false;
    }
    let Some(origin) = headers.get(ORIGIN) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return true;
    };
    !config
        .allowed_origins()
        .iter()
        .any(|allowed| allowed == origin)
}

fn has_bearer(headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.get(..7))
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer "))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn config(dev: bool) -> Config {
        let pairs = [
            ("DATABASE_URL", "postgres://x"),
            ("DEV_MODE", if dev { "true" } else { "false" }),
            ("PUBLIC_URL", "https://books.example.com"),
            ("TRUSTED_PROXIES", "none"),
            ("CONTACT_EMAIL", "ops@example.com"),
            ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
            ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
            ("CORS_ORIGINS", "capacitor://localhost"),
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

    const COOKIE_ONLY: (&str, &str) = ("cookie", "theme=x; bookclub_session=abc");

    #[test]
    fn foreign_origins_with_a_cookie_are_refused() {
        let config = config(false);
        let evil = headers(&[COOKIE_ONLY, ("origin", "https://evil.example")]);
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(refuses(&config, &method, &evil), "{method}");
        }
        let null = headers(&[COOKIE_ONLY, ("origin", "null")]);
        assert!(refuses(&config, &Method::POST, &null));
    }

    #[test]
    fn allowed_requests_pass() {
        let config = config(false);
        let evil = headers(&[COOKIE_ONLY, ("origin", "https://evil.example")]);
        assert!(!refuses(&config, &Method::GET, &evil));
        assert!(!refuses(&config, &Method::OPTIONS, &evil));
        let own = headers(&[COOKIE_ONLY, ("origin", "https://books.example.com")]);
        assert!(!refuses(&config, &Method::POST, &own));
        let app = headers(&[COOKIE_ONLY, ("origin", "capacitor://localhost")]);
        assert!(!refuses(&config, &Method::POST, &app));
        let no_origin = headers(&[COOKIE_ONLY]);
        assert!(!refuses(&config, &Method::POST, &no_origin));
        let no_cookie = headers(&[("origin", "https://evil.example")]);
        assert!(!refuses(&config, &Method::POST, &no_cookie));
        let bearer = headers(&[
            COOKIE_ONLY,
            ("authorization", "Bearer bc_abc"),
            ("origin", "https://evil.example"),
        ]);
        assert!(!refuses(&config, &Method::POST, &bearer));
    }

    /// The check does not depend on how the session extractor parses cookie names.
    #[test]
    fn any_cookie_from_a_foreign_origin_is_refused() {
        let config = config(false);
        for cookie in [
            "bookclub%5Fsession=planted",
            " bookclub_session =planted",
            "theme=x",
        ] {
            let mut evil = headers(&[("origin", "https://evil.example")]);
            evil.append("cookie", HeaderValue::from_str(cookie).unwrap());
            assert!(refuses(&config, &Method::POST, &evil), "{cookie}");
        }
        let mut undecodable = headers(&[("origin", "https://evil.example")]);
        undecodable.append(
            "cookie",
            HeaderValue::from_bytes(b"bookclub_session=\xff").unwrap(),
        );
        assert!(refuses(&config, &Method::POST, &undecodable));
    }

    #[test]
    fn the_vite_origin_passes_only_in_dev_mode() {
        let vite = headers(&[COOKIE_ONLY, ("origin", "http://localhost:5173")]);
        assert!(refuses(&config(false), &Method::POST, &vite));
        assert!(!refuses(&config(true), &Method::POST, &vite));
    }
}
