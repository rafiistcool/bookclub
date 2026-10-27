//! `GET /api/config`, the production configuration and the rate-limit keys
//! (endpoint-map §1, docs/phase1/conventions.md §10.1, §10.2, §11). Ports of the self-hosting
//! tests that still apply to the hosted service.

mod common;

use std::net::IpAddr;

use axum::http::Method;
use bookclub_server::config::Config;
use bookclub_server::outbound::user_agent;
use bookclub_server::ratelimit::unknown_identifier_key;
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn config_from(pairs: &[(&str, &str)]) -> anyhow::Result<Config> {
    let pairs: Vec<(String, String)> = std::iter::once(("DATABASE_URL", "postgres://x"))
        .chain(pairs.iter().copied())
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Config::from_lookup(|key| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    })
}

async fn preflight(client: &TestClient, origin: &str) -> TestResponse {
    client
        .clone()
        .with_header("origin", origin)
        .with_header("access-control-request-method", "GET")
        .request(Method::OPTIONS, "/api/health", None, Vec::new())
        .await
}

#[sqlx::test]
async fn config_is_public_branding(db: PgPool) {
    let app = TestApp::new(db).await;
    let res = app.anon().get("/api/config").await;
    res.assert_status(200);
    assert_eq!(
        res.json(),
        json!({
            "name": "Bookclub",
            "theme": "#b44a2a",
            "theme_dark": "#8e361c",
            "public_url": "http://localhost:5173",
            "signup": "invite",
        })
    );
}

/// parity: test_selfhost.py::test_branded_app_config_cors_and_user_agent
///
/// Reasoned changes: the name comes from `SERVICE_NAME`; the accent colours are fixed (no
/// `BOOKCLUB_THEME`); the manifest is a static file; the public origin needs no CORS (the SPA
/// is same-origin).
#[sqlx::test]
async fn config_follows_the_service_settings(db: PgPool) {
    let mut env = PRODUCTION_ENV.to_vec();
    env.extend([
        ("SERVICE_NAME", "Thursday Readers"),
        ("PUBLIC_URL", "https://books.example.com/app"),
        ("SIGNUP_MODE", "open"),
        ("CORS_ORIGINS", "capacitor://localhost"),
    ]);
    let app = TestApp::with_env(db, &env).await;
    let body = app.anon().get("/api/config").await.json();
    assert_eq!(body["name"], "Thursday Readers");
    assert_eq!(body["public_url"], "https://books.example.com");
    assert_eq!(body["signup"], "open");
    assert_eq!(body["theme"], "#b44a2a");

    let allowed = preflight(&app.anon(), "capacitor://localhost").await;
    assert_eq!(
        allowed.header("access-control-allow-origin"),
        Some("capacitor://localhost")
    );
    let denied = preflight(&app.anon(), "http://localhost:5173").await;
    assert_ne!(
        denied.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
}

/// parity: test_selfhost.py::test_public_config_and_cors_origins
///
/// `PUBLIC_URL` is reduced to its origin, goes into the outbound User-Agent, and must be an
/// http(s) URL with a host.
#[test]
fn public_url_is_an_origin() {
    let mut env = PRODUCTION_ENV.to_vec();
    env.push(("PUBLIC_URL", "https://books.example.com/app"));
    let config = config_from(&env).unwrap();
    assert_eq!(config.public_url, "https://books.example.com");
    assert_eq!(config.allowed_origins(), vec!["https://books.example.com"]);
    assert!(config.cors_allowed_origins().is_empty());
    let ua = user_agent(&config);
    assert!(ua.contains("https://books.example.com"), "{ua}");
    assert!(ua.contains("ops@example.com"), "{ua}");

    let mut dev = vec![
        ("DEV_MODE", "true"),
        ("PUBLIC_URL", "https://books.example.com"),
    ];
    let config = config_from(&dev).unwrap();
    assert!(
        config
            .allowed_origins()
            .contains(&"http://localhost:5173".to_owned())
    );
    assert!(
        config
            .allowed_origins()
            .contains(&"https://books.example.com".to_owned())
    );
    dev.push(("PUBLIC_URL", "not-a-url"));
    let error = config_from(&dev).unwrap_err();
    assert!(error.to_string().contains("PUBLIC_URL"), "{error}");
}

/// parity: test_selfhost.py::test_debug_cors_keeps_vite
#[sqlx::test]
async fn dev_mode_cors_keeps_vite(db: PgPool) {
    let app = TestApp::new(db).await;
    assert_eq!(
        app.anon().get("/api/config").await.json()["name"],
        "Bookclub"
    );
    let res = preflight(&app.anon(), "http://localhost:5173").await;
    assert_eq!(
        res.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
    assert_eq!(res.header("access-control-allow-credentials"), Some("true"));
}

/// parity: test_selfhost.py::test_production_app_registers_and_hides_docs
///
/// Reasoned changes: health answers `{"status":"ok"}`; `GET /api/openapi.json` is public
/// (the API contract); the sign-up needs an email and an invite from the database.
#[sqlx::test]
async fn a_production_app_registers(db: PgPool) {
    let app = TestApp::with_env(db, &PRODUCTION_ENV).await;
    let anon = app.anon();
    assert_eq!(
        anon.get("/api/health").await.json(),
        json!({"status": "ok"})
    );
    anon.get("/api/openapi.json").await.assert_status(200);
    let vite = preflight(&anon, "http://localhost:5173").await;
    assert_eq!(vite.header("access-control-allow-origin"), None);

    app.exec("INSERT INTO club_invites (club_id, code, max_uses) VALUES (NULL, 'SELFHOST1', 1)")
        .await;
    let res = anon
        .post(
            "/api/auth/register",
            json!({
                "username": "ada",
                "email": "ada@example.test",
                "password": PASSWORD,
                "invite_code": "SELFHOST1",
            }),
        )
        .await;
    res.assert_status(201);
    let cookie = res.set_cookies().pop().unwrap().to_lowercase();
    assert!(cookie.contains("httponly"));
    assert!(cookie.contains("samesite=lax"));
    assert!(cookie.contains("secure"));
    assert_eq!(anon.get("/api/auth/me").await.json()["username"], "ada");
    // Production links use PUBLIC_URL.
    assert!(
        app.sent_emails()[0]
            .text
            .contains("https://books.example.com/verify-email#token=")
    );
}

/// parity: test_selfhost.py::test_production_refuses_demo_secret_and_invite
///
/// Reasoned change: there are no secrets or bootstrap invites to refuse any more. Startup
/// refuses the production settings that would be unsafe instead: no `TRUSTED_PROXIES` (one
/// client could lock everyone behind the proxy out of sign-up), and the log mail transport,
/// which writes reset links to the log.
#[test]
fn production_startup_refuses_unsafe_settings() {
    let without_proxies: Vec<(&str, &str)> = PRODUCTION_ENV
        .iter()
        .copied()
        .filter(|(key, _)| *key != "TRUSTED_PROXIES")
        .collect();
    let error = config_from(&without_proxies).unwrap_err();
    assert!(error.to_string().contains("TRUSTED_PROXIES"), "{error}");

    let mut direct = without_proxies.clone();
    direct.push(("TRUSTED_PROXIES", "none"));
    assert!(config_from(&direct).unwrap().trusted_proxies.is_empty());

    let mut dev = without_proxies.clone();
    dev.push(("DEV_MODE", "true"));
    assert!(config_from(&dev).is_ok(), "DEV_MODE needs no proxy list");

    let mut log_mail = PRODUCTION_ENV.to_vec();
    log_mail.push(("EMAIL_TRANSPORT", "log"));
    let error = config_from(&log_mail).unwrap_err();
    assert!(error.to_string().contains("EMAIL_TRANSPORT"), "{error}");
}

/// The login limit per IP keys IPv6 clients by their /64, so a client with a /64 cannot mint
/// a fresh key per request.
#[sqlx::test]
async fn ipv6_addresses_in_one_slash_64_share_the_login_bucket(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let a: IpAddr = "2001:db8:5:6::1".parse().unwrap();
    let b: IpAddr = "2001:db8:5:6:ffff::2".parse().unwrap();
    let other: IpAddr = "2001:db8:5:7::1".parse().unwrap();
    for n in 0..30 {
        let client = app.anon().with_ip(if n % 2 == 0 { a } else { b });
        client
            .post(
                "/api/auth/login",
                json!({"username": format!("ghost{n}"), "password": "nope"}),
            )
            .await;
    }
    let login = json!({"username": "ada", "password": PASSWORD});
    app.anon()
        .with_ip(b)
        .post("/api/auth/login", login.clone())
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    app.anon()
        .with_ip(other)
        .post("/api/auth/login", login.clone())
        .await
        .assert_status(204);
    // IPv4 addresses are keyed one by one, also when IPv4-mapped.
    let v4: IpAddr = "203.0.113.7".parse().unwrap();
    let mapped: IpAddr = "::ffff:203.0.113.7".parse().unwrap();
    for _ in 0..30 {
        app.anon()
            .with_ip(v4)
            .post(
                "/api/auth/login",
                json!({"username": "ghost", "password": "nope"}),
            )
            .await;
    }
    app.anon()
        .with_ip(mapped)
        .post("/api/auth/login", login.clone())
        .await
        .assert_status(429);
    app.anon()
        .with_ip("203.0.113.8".parse().unwrap())
        .post("/api/auth/login", login)
        .await
        .assert_status(204);
}

/// Names that name no account share at most 65,536 failure buckets (the first 4 hex digits of
/// their SHA-256), so attacker-chosen names cannot flush the buckets of real accounts; two
/// unknown names in one bucket share its limit.
#[sqlx::test]
async fn unknown_names_land_in_hashed_buckets(db: PgPool) {
    let app = TestApp::new(db).await;
    let target = unknown_identifier_key("nobody");
    let twin = (0..)
        .map(|n| format!("ghost{n}"))
        .find(|name| unknown_identifier_key(name) == target)
        .unwrap();
    let elsewhere = (0..)
        .map(|n| format!("ghost{n}"))
        .find(|name| unknown_identifier_key(name) != target)
        .unwrap();
    for n in 0..10u16 {
        let ip: IpAddr = format!("198.51.100.{n}").parse().unwrap();
        app.anon()
            .with_ip(ip)
            .post(
                "/api/auth/login",
                json!({"username": "nobody", "password": "nope"}),
            )
            .await
            .assert_status(401);
    }
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": twin, "password": "nope"}),
        )
        .await
        .assert_status(429);
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": elsewhere, "password": "nope"}),
        )
        .await
        .assert_status(401);
    // The bucket space is bounded.
    let buckets: std::collections::HashSet<String> = (0..300_000)
        .map(|n| unknown_identifier_key(&format!("name{n}")))
        .collect();
    assert!(buckets.len() <= 65_536, "{}", buckets.len());
}
