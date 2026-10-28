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
use sha2::{Digest, Sha256};
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
/// parity: test_selfhost.py::test_invalid_theme_warns_and_falls_back
///
/// Reasoned changes: the name comes from `SERVICE_NAME`; the accent colours are fixed (no
/// `BOOKCLUB_THEME`, so there is no invalid theme to warn about: `theme` is always the legacy
/// default `#b44a2a`); the manifest is a static file; the public origin needs no CORS (the SPA
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
/// parity: test_selfhost.py::test_dotenv_only_production_boot
///
/// Reasoned changes: health answers `{"status":"ok"}`; `GET /api/openapi.json` is public
/// (the API contract); the sign-up needs an email and an invite from the database. The
/// configuration comes from the environment only (no `.env` file, no generated
/// `.bootstrap_invite`), so the production boot from the documented variables is this test.
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
/// parity: test_selfhost.py::test_placeholder_detection
/// parity: test_selfhost.py::test_production_generates_and_reuses_secrets
///
/// Reasoned change: there are no secrets or bootstrap invites to detect, refuse, generate or
/// reuse any more (sessions and invites are database rows; `Secure` comes from
/// `COOKIE_SECURE`, not an HTTPS mode). Startup refuses the production settings that would be
/// unsafe instead: no `TRUSTED_PROXIES` (one client could lock everyone behind the proxy out of
/// sign-up), and the log mail transport, which writes reset links to the log.
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

/// Every login identifier fills its own failure bucket, whether it names an account or not
/// (§10.1), so the answers never tell a verified address from a free or unverified one:
///
/// - names that share the first 16 bits of an address's SHA-256 (the old buckets) cannot fill
///   the address's bucket: after ten of them, the address still answers 401;
/// - ten failures of the address itself then make it 429, for every kind of address alike;
/// - ten failures of a username leave its verified address alone, so they do not link the two.
#[sqlx::test]
async fn failure_buckets_tell_no_identifier_from_another(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    app.user_unverified("cleo").await;
    let login = |n: u16, username: String| {
        let client = app
            .anon()
            .with_ip(format!("198.51.100.{n}").parse().unwrap());
        async move {
            client
                .post(
                    "/api/auth/login",
                    json!({"username": username, "password": "nope"}),
                )
                .await
                .status
                .as_u16()
        }
    };
    let prefix = |text: &str| Sha256::digest(text.as_bytes())[..2].to_vec();
    let addresses = [
        "ada@example.test",
        "cleo@example.test",
        "nobody@example.test",
    ];
    // Ten names in each address's old bucket, found in one pass.
    let buckets: Vec<Vec<u8>> = addresses.iter().map(|a| prefix(a)).collect();
    let mut twins: Vec<Vec<String>> = vec![Vec::new(); addresses.len()];
    for i in 0.. {
        let name = format!("ghost{i}");
        let bucket = prefix(&name);
        if let Some(at) = buckets.iter().position(|b| *b == bucket)
            && twins[at].len() < 10
        {
            twins[at].push(name);
        }
        if twins.iter().all(|names| names.len() == 10) {
            break;
        }
    }
    for ((n, address), names) in (0u16..).zip(addresses).zip(twins) {
        for twin in names {
            assert_ne!(
                unknown_identifier_key(&twin),
                unknown_identifier_key(address)
            );
            assert_eq!(login(n, twin).await, 401);
        }
        assert_eq!(login(n, address.to_owned()).await, 401, "{address}");
        for _ in 0..9 {
            assert_eq!(login(10 + n, address.to_owned()).await, 401, "{address}");
        }
        assert_eq!(login(20 + n, address.to_owned()).await, 429, "{address}");
    }

    app.user("bob").await;
    for _ in 0..10 {
        assert_eq!(login(41, "bob".to_owned()).await, 401);
    }
    assert_eq!(login(41, "bob".to_owned()).await, 429);
    assert_eq!(login(42, "bob@example.test".to_owned()).await, 401);
}
