//! The client address behind a reverse proxy (docs/phase1/conventions.md §10.2, #55).
//!
//! With the peer inside `TRUSTED_PROXIES`, the sign-up, login and reset limits count per client
//! that `X-Forwarded-For` names, so two people behind one proxy keep their own budgets. From any
//! other peer the header is ignored and the peer is the client, so a client cannot pick a fresh
//! address per request. `client_ip::resolve` has unit tests; these go through the router with
//! a `ConnectInfo` peer, so they also fail when a handler counts the raw peer.

mod common;

use std::net::IpAddr;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const TOO_MANY: &str = "Too many attempts. Wait a few minutes.";
/// The reverse proxy, inside `TRUSTED_PROXIES`.
const PROXY: &str = "10.0.0.2";
/// A second proxy in front of it (a CDN or a load balancer), also trusted.
const INNER_PROXY: &str = "172.16.0.9";
/// Two people behind the proxy.
const A: &str = "203.0.113.5";
const B: &str = "198.51.100.7";

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

async fn proxied_app(db: PgPool, trusted: &str) -> TestApp {
    let app = TestApp::with_env(db, &[("TRUSTED_PROXIES", trusted)]).await;
    app.exec(
        "INSERT INTO club_invites (club_id, code, max_uses, expires_at)
         VALUES (NULL, 'LINK', NULL, now() + interval '7 days')",
    )
    .await;
    app
}

/// A request from `peer` with `X-Forwarded-For: forwarded`.
fn via(app: &TestApp, peer: &str, forwarded: &str) -> TestClient {
    app.anon()
        .with_ip(ip(peer))
        .with_header("x-forwarded-for", forwarded)
}

fn sign_up_body(username: &str) -> Value {
    json!({
        "username": username,
        "email": format!("{username}@example.test"),
        "password": PASSWORD,
        "invite_code": "LINK",
    })
}

async fn sign_up(client: &TestClient, username: &str) -> TestResponse {
    client
        .post("/api/auth/register", sign_up_body(username))
        .await
}

async fn login(client: &TestClient, username: &str, password: &str) -> TestResponse {
    client
        .post(
            "/api/auth/login",
            json!({"username": username, "password": password}),
        )
        .await
}

async fn reset(client: &TestClient) -> TestResponse {
    client
        .post(
            "/api/auth/password-reset",
            json!({"email": "nobody@example.test"}),
        )
        .await
}

/// Five sign-ups per hour for A; B, behind the same proxy, still signs up.
#[sqlx::test]
async fn sign_ups_count_per_forwarded_client(db: PgPool) {
    let app = proxied_app(db, "10.0.0.0/8").await;
    for n in 0..5 {
        sign_up(&via(&app, PROXY, A), &format!("a{n}"))
            .await
            .assert_status(201);
    }
    sign_up(&via(&app, PROXY, A), "a5")
        .await
        .assert_detail(429, TOO_MANY);
    sign_up(&via(&app, PROXY, B), "b0").await.assert_status(201);
}

/// Thirty login attempts per 10 minutes for A (each for another unknown name, so only the
/// address limit applies); B, behind the same proxy, still signs in.
#[sqlx::test]
async fn logins_count_per_forwarded_client(db: PgPool) {
    let app = proxied_app(db, "10.0.0.0/8").await;
    app.user("ada").await;
    for n in 0..30 {
        login(&via(&app, PROXY, A), &format!("nobody{n}"), "wrong")
            .await
            .assert_detail(401, "Wrong username or password");
    }
    login(&via(&app, PROXY, A), "ada", PASSWORD)
        .await
        .assert_detail(429, TOO_MANY);
    login(&via(&app, PROXY, B), "ada", PASSWORD)
        .await
        .assert_status(204);
}

/// Five reset requests per hour for A; B, behind the same proxy, still asks for a link.
#[sqlx::test]
async fn reset_requests_count_per_forwarded_client(db: PgPool) {
    let app = proxied_app(db, "10.0.0.0/8").await;
    for _ in 0..5 {
        reset(&via(&app, PROXY, A)).await.assert_status(204);
    }
    reset(&via(&app, PROXY, A))
        .await
        .assert_detail(429, TOO_MANY);
    reset(&via(&app, PROXY, B)).await.assert_status(204);
    app.settle().await;
}

/// A peer outside `TRUSTED_PROXIES` is the client, whatever it forwards: a fresh
/// `X-Forwarded-For` per request buys no fresh budget, and a peer cannot spend another's.
#[sqlx::test]
async fn forwarded_for_from_an_untrusted_peer_counts_against_the_peer(db: PgPool) {
    let app = proxied_app(db, "10.0.0.0/8").await;
    let direct = "192.0.2.10";
    for n in 0..5 {
        sign_up(
            &via(&app, direct, &format!("203.0.113.{n}")),
            &format!("d{n}"),
        )
        .await
        .assert_status(201);
    }
    sign_up(&via(&app, direct, "203.0.113.99"), "d5")
        .await
        .assert_detail(429, TOO_MANY);
    // The addresses the peer forwarded were not charged, and another peer has its own budget.
    sign_up(&via(&app, PROXY, "203.0.113.0"), "p0")
        .await
        .assert_status(201);
    sign_up(&via(&app, "192.0.2.11", "203.0.113.99"), "e0")
        .await
        .assert_status(201);
    // A peer that forges B's address until its own budget is gone leaves B's untouched.
    let forger = "192.0.2.12";
    for n in 0..5 {
        sign_up(&via(&app, forger, B), &format!("f{n}"))
            .await
            .assert_status(201);
    }
    sign_up(&via(&app, forger, B), "f5")
        .await
        .assert_detail(429, TOO_MANY);
    sign_up(&via(&app, PROXY, B), "b0").await.assert_status(201);
}

/// With `TRUSTED_PROXIES=none` (clients connect directly) the header never counts.
#[sqlx::test]
async fn without_trusted_proxies_forwarded_for_is_ignored(db: PgPool) {
    let app = proxied_app(db, "none").await;
    let direct = "192.0.2.10";
    for n in 0..5 {
        reset(&via(&app, direct, &format!("203.0.113.{n}")))
            .await
            .assert_status(204);
    }
    reset(&via(&app, direct, "203.0.113.99"))
        .await
        .assert_detail(429, TOO_MANY);
    reset(&via(&app, "192.0.2.11", A)).await.assert_status(204);
    app.settle().await;
}

/// Behind two trusted proxies the client is the right-most untrusted address. Whatever the
/// client writes to the left of it is ignored, so a forged entry buys no fresh budget; the
/// inner proxy's own address is skipped, so not everyone shares its bucket.
#[sqlx::test]
async fn two_trusted_hops_name_the_right_most_untrusted_address(db: PgPool) {
    let app = proxied_app(db, "10.0.0.0/8, 172.16.0.0/12").await;
    for n in 0..5 {
        let forged = format!("192.0.2.{n}, {A}, {INNER_PROXY}");
        sign_up(&via(&app, PROXY, &forged), &format!("a{n}"))
            .await
            .assert_status(201);
    }
    // The chain may also come as several headers, split anywhere: a resolver that read only the
    // last header would see the inner proxy alone, one that read only the first the forged
    // address alone, and either would find a fresh budget.
    let split_before_proxy = app
        .anon()
        .with_ip(ip(PROXY))
        .with_header("x-forwarded-for", &format!("192.0.2.99, {A}"))
        .with_header("x-forwarded-for", INNER_PROXY);
    sign_up(&split_before_proxy, "a5")
        .await
        .assert_detail(429, TOO_MANY);
    let split_after_forged = app
        .anon()
        .with_ip(ip(PROXY))
        .with_header("x-forwarded-for", "192.0.2.98")
        .with_header("x-forwarded-for", &format!("{A}, {INNER_PROXY}"));
    sign_up(&split_after_forged, "a6")
        .await
        .assert_detail(429, TOO_MANY);
    sign_up(&via(&app, PROXY, &format!("{A}, {B}, {INNER_PROXY}")), "b0")
        .await
        .assert_status(201);
}
