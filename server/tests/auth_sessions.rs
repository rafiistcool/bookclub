//! Server-side sessions, bearer tokens, the session list and the origin check
//! (docs/phase1/conventions.md §5.1, §5.2, §5.3, §5.8; endpoint-map §2).

mod common;

use axum::Router;
use axum::http::Method;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use bookclub_server::AppState;
use bookclub_server::auth::extract::{InstanceAdmin, MaybeUser, VerifiedUser};
use bookclub_server::auth::session::maintain_cookie;
use common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

async fn token(app: &TestApp, username: &str, label: &str) -> Value {
    let res = app
        .anon()
        .post(
            "/api/auth/tokens",
            json!({"username": username, "password": PASSWORD, "label": label}),
        )
        .await;
    res.assert_status(201);
    res.json()
}

/// Seconds from now until the session's `expires_at`.
async fn expires_in(app: &TestApp, session: i64) -> i64 {
    app.scalar_i64(&format!(
        "SELECT extract(epoch FROM expires_at - now())::bigint FROM sessions WHERE id = {session}"
    ))
    .await
}

async fn session_of(app: &TestApp, user: &TestClient) -> i64 {
    app.scalar_i64(&format!(
        "SELECT id FROM sessions WHERE user_id = {} AND kind = 'cookie'",
        user.id()
    ))
    .await
}

const DAY: i64 = 86_400;

#[sqlx::test]
async fn the_cookie_is_random_and_stored_only_as_a_hash(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let anon = app.anon();
    let res = anon
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await;
    res.assert_status(204);
    let cookie = res.set_cookies().pop().unwrap();
    for part in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=34560000"] {
        assert!(cookie.contains(part), "{cookie} lacks {part}");
    }
    let value = anon.cookie().unwrap();
    assert_ne!(Some(value.clone()), ada.cookie());
    let secret = BASE64_URL_SAFE_NO_PAD.decode(&value).unwrap();
    assert_eq!(secret.len(), 32);
    let hash = Sha256::digest(&secret).to_vec();
    let stored: (Vec<u8>, String, i64) = sqlx::query_as(
        "SELECT token_hash, kind, extract(epoch FROM expires_at - now())::bigint
         FROM sessions WHERE token_hash = $1",
    )
    .bind(&hash)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(stored.1, "cookie");
    assert!(
        (30 * DAY - 60..=30 * DAY).contains(&stored.2),
        "{}",
        stored.2
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE encode(token_hash, 'escape') LIKE '%{value}%'"
        ))
        .await,
        0
    );
}

/// parity: test_selfhost.py::test_session_cookie_not_secure_on_plain_http
///
/// Reasoned change: `Secure` comes from `COOKIE_SECURE` (default `!DEV_MODE`), not from the
/// request scheme.
#[sqlx::test]
async fn dev_mode_cookies_are_not_secure(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let res = app
        .anon()
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await;
    let cookie = res.set_cookies().pop().unwrap();
    assert!(cookie.starts_with("bookclub_session="));
    assert!(!cookie.to_lowercase().contains("secure"), "{cookie}");
}

/// parity: test_selfhost.py::test_session_cookie_secure_on_https_and_forwarded_proto
#[sqlx::test]
async fn production_cookies_are_secure(db: PgPool) {
    let app = TestApp::with_env(db, &PRODUCTION_ENV).await;
    app.user("ada").await;
    let res = app
        .anon()
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await;
    res.assert_status(204);
    let cookie = res.set_cookies().pop().unwrap();
    assert!(cookie.starts_with("__Host-bookclub_session="), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(
        cookie.contains("Path=/") && !cookie.contains("Domain"),
        "{cookie}"
    );
}

/// Cookie tossing (§5.1): a `Secure` session cookie is `__Host-bookclub_session`, which a page
/// on a sibling subdomain cannot set. A planted `bookclub_session` for the parent domain, or an
/// encoded spelling of the `__Host-` name, is ignored; a second `__Host-` cookie signs the
/// request in as nobody instead of as the attacker.
#[sqlx::test]
async fn a_planted_session_cookie_signs_nobody_in(db: PgPool) {
    let app = TestApp::with_env(db, &PRODUCTION_ENV).await;
    let ada = app.user("ada").await;
    let mallory = app.user("mallory").await;
    let planted = mallory.cookie().unwrap();
    for tossed in [
        format!("bookclub_session={planted}"),
        format!("%5F%5FHost-bookclub_session={planted}"),
        format!("__Host-bookclub%5Fsession={planted}"),
    ] {
        let res = ada
            .clone()
            .with_header("cookie", &tossed)
            .get("/api/auth/me")
            .await;
        res.assert_status(200);
        assert_eq!(res.json()["username"], "ada", "{tossed}");
        app.anon()
            .with_header("cookie", &tossed)
            .get("/api/auth/me")
            .await
            .assert_detail(401, "Not signed in");
    }
    let res = ada
        .clone()
        .with_header("cookie", &format!("__Host-bookclub_session={planted}"))
        .get("/api/auth/me")
        .await;
    res.assert_detail(401, "Not signed in");
}

/// parity: test_selfhost.py::test_trusted_proxy_denies_forwarded_proto
///
/// `X-Forwarded-Proto` never decides `Secure`, not even from a trusted proxy;
/// `COOKIE_SECURE=false` wins. (Python ignored it from an untrusted client only.)
#[sqlx::test]
async fn forwarded_proto_does_not_make_cookies_secure(db: PgPool) {
    let mut env = PRODUCTION_ENV.to_vec();
    env.push(("COOKIE_SECURE", "false"));
    env.push(("TRUSTED_PROXIES", "127.0.0.1"));
    let app = TestApp::with_env(db, &env).await;
    app.user("ada").await;
    let untrusted = app.anon();
    let proxy = app.anon().with_ip("127.0.0.1".parse().unwrap());
    for client in [untrusted, proxy] {
        let res = client
            .with_header("x-forwarded-proto", "https")
            .post(
                "/api/auth/login",
                json!({"username": "ada", "password": PASSWORD}),
            )
            .await;
        res.assert_status(204);
        let cookie = res.set_cookies().pop().unwrap().to_lowercase();
        assert!(cookie.contains("bookclub_session="));
        assert!(!cookie.contains("secure"), "{cookie}");
        assert!(cookie.contains("httponly"));
        assert!(cookie.contains("samesite=lax"));
    }
}

/// Idle expiry slides (§5.1): a session used after more than an hour moves its expiry to 30
/// days from now, and the cookie is sent again; a fresh session is left alone.
#[sqlx::test]
async fn cookie_sessions_slide(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let id = session_of(&app, &ada).await;
    let res = ada.get("/api/auth/me").await;
    res.assert_status(200);
    assert!(
        res.set_cookies().is_empty(),
        "a fresh session is not touched"
    );

    app.exec(&format!(
        "UPDATE sessions SET last_used_at = now() - interval '2 hours', expires_at = now() + interval '1 day'
         WHERE id = {id}"
    ))
    .await;
    let before = ada.cookie();
    let res = ada.get("/api/auth/me").await;
    res.assert_status(200);
    let renewed = res.set_cookies();
    assert_eq!(renewed.len(), 1, "{renewed:?}");
    assert!(renewed[0].contains("Max-Age=34560000"));
    assert_eq!(ada.cookie(), before, "the same session");
    assert!((30 * DAY - 60..=30 * DAY).contains(&expires_in(&app, id).await));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE id = {id} AND last_used_at > now() - interval '1 minute'"
        ))
        .await,
        1
    );
    // Less than an hour later nothing is written.
    app.exec(&format!(
        "UPDATE sessions SET last_used_at = now() - interval '59 minutes', expires_at = now() + interval '1 day'
         WHERE id = {id}"
    ))
    .await;
    ada.get("/api/auth/me").await.assert_status(200);
    assert!(expires_in(&app, id).await <= DAY);
}

/// A cookie that resolves to no live session is 401 with a clearing `Set-Cookie`.
#[sqlx::test]
async fn dead_cookies_are_cleared(db: PgPool) {
    let app = TestApp::new(db).await;
    let expired = app.user("expired").await;
    app.exec(&format!(
        "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE user_id = {}",
        expired.id()
    ))
    .await;
    let deleted = app.user("deleted").await;
    app.exec(&format!("DELETE FROM users WHERE id = {}", deleted.id()))
        .await;
    let garbage = app.anon();
    garbage.set_cookie(Some("not-base64!"));
    let unknown = app.anon();
    unknown.set_cookie(Some(&BASE64_URL_SAFE_NO_PAD.encode([9u8; 32])));
    for client in [&expired, &deleted, &garbage, &unknown] {
        let res = client.get("/api/auth/me").await;
        res.assert_detail(401, "Not signed in");
        let cleared = res.set_cookies();
        assert_eq!(cleared.len(), 1, "{cleared:?}");
        assert!(cleared[0].starts_with("bookclub_session=;"));
        assert!(cleared[0].contains("Max-Age=0"));
        assert!(client.cookie().is_none());
    }
}

#[sqlx::test]
async fn bearer_tokens(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let created = token(&app, " ADA ", " Ada's phone ").await;
    let secret = created["token"].as_str().unwrap();
    assert!(secret.starts_with("bc_"), "{secret}");
    assert_eq!(
        BASE64_URL_SAFE_NO_PAD.decode(&secret[3..]).unwrap().len(),
        32
    );
    let session = &created["session"];
    assert_eq!(session["kind"], "bearer");
    assert_eq!(session["label"], "Ada's phone");
    assert_eq!(session["current"], false);
    let created_at = session["created_at"].as_str().unwrap();
    assert!(created_at.ends_with('Z'), "{created_at}");
    let id = session["id"].as_i64().unwrap();
    assert!((90 * DAY - 60..=90 * DAY).contains(&expires_in(&app, id).await));

    let phone = app.anon().with_bearer(secret);
    let me = phone.get("/api/auth/me").await;
    me.assert_status(200);
    assert_eq!(me.json()["id"], ada.id());
    assert!(me.set_cookies().is_empty());

    // Bearer sessions slide by 90 days.
    app.exec(&format!(
        "UPDATE sessions SET last_used_at = now() - interval '2 hours', expires_at = now() + interval '1 day'
         WHERE id = {id}"
    ))
    .await;
    let me = phone.get("/api/auth/me").await;
    assert!(
        me.set_cookies().is_empty(),
        "no cookie for a bearer session"
    );
    assert!((90 * DAY - 60..=90 * DAY).contains(&expires_in(&app, id).await));

    // Labels are cut to 200 characters.
    let long = token(&app, "ada", &"x".repeat(250)).await;
    assert_eq!(
        long["session"]["label"].as_str().unwrap().chars().count(),
        200
    );
}

/// Token creation has the login errors and limits.
#[sqlx::test]
async fn tokens_need_the_right_password(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let wrong = json!({"username": "ada", "password": "nope", "label": "x"});
    for _ in 0..10 {
        app.anon()
            .post("/api/auth/tokens", wrong.clone())
            .await
            .assert_detail(401, "Wrong username or password");
    }
    app.anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "x"}),
        )
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    app.anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_detail(400, "Field required");
}

/// The bearer token is read first; a bad one is 401 even next to a valid cookie, and that
/// cookie is not cleared.
#[sqlx::test]
async fn the_bearer_token_wins_over_the_cookie(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user("grace").await;
    app.user("ada").await;
    let ada_token = token(&app, "ada", "phone").await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let both = grace.clone().with_bearer(&ada_token);
    assert_eq!(both.get("/api/auth/me").await.json()["username"], "ada");
    for bad in ["bc_nope", "nope", &ada_token[3..]] {
        let res = grace.clone().with_bearer(bad).get("/api/auth/me").await;
        res.assert_detail(401, "Not signed in");
        assert!(res.set_cookies().is_empty());
    }
    assert!(grace.cookie().is_some());
    grace.get("/api/auth/me").await.assert_status(200);
    // A cookie value is not a bearer token, and a bearer token is not a cookie.
    let as_bearer = grace.cookie().unwrap();
    app.anon()
        .with_bearer(&format!("bc_{as_bearer}"))
        .get("/api/auth/me")
        .await
        .assert_status(401);
    let as_cookie = app.anon();
    as_cookie.set_cookie(Some(&ada_token[3..]));
    as_cookie.get("/api/auth/me").await.assert_status(401);
}

/// Logout with a bearer token deletes that token's session and sends no cookie (§5.1); the
/// user's cookie session stays.
#[sqlx::test]
async fn logout_with_a_bearer_token(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let created = token(&app, "ada", "phone").await;
    let id = created["session"]["id"].as_i64().unwrap();
    let phone = app.anon().with_bearer(created["token"].as_str().unwrap());
    let res = phone.post("/api/auth/logout", json!(null)).await;
    res.assert_status(204);
    assert!(res.set_cookies().is_empty());
    phone.get("/api/auth/me").await.assert_status(401);
    assert_eq!(
        app.scalar_i64(&format!("SELECT count(*) FROM sessions WHERE id = {id}"))
            .await,
        0
    );
    ada.get("/api/auth/me").await.assert_status(200);
}

/// Token creation is the one session-creating route that leaves the caller's cookie session
/// alone (§5.1): an app signed in from a browser does not sign the browser out.
#[sqlx::test]
async fn creating_a_token_keeps_the_cookie_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let cookie = ada.cookie();
    let res = ada
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
        )
        .await;
    res.assert_status(201);
    assert!(res.set_cookies().is_empty());
    assert_eq!(ada.cookie(), cookie);
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE user_id = {}",
            ada.id()
        ))
        .await,
        2
    );
}

/// A cookie session's label is the browser's User-Agent, cut to 200 characters.
#[sqlx::test]
async fn cookie_sessions_are_labelled_with_the_user_agent(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let firefox = app.anon().with_header("user-agent", "Firefox/140");
    firefox
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    let items = firefox.get("/api/auth/sessions").await.json()["items"].clone();
    let current: Vec<&Value> = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["current"] == true)
        .collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["kind"], "cookie");
    assert_eq!(current[0]["label"], "Firefox/140");

    let long = app.anon().with_header("user-agent", &"x".repeat(250));
    long.post(
        "/api/auth/login",
        json!({"username": "ada", "password": PASSWORD}),
    )
    .await
    .assert_status(204);
    let items = long.get("/api/auth/sessions").await.json()["items"].clone();
    let label = items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["current"] == true)
        .unwrap()["label"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(label, "x".repeat(200));
}

#[sqlx::test]
async fn the_session_list(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app
        .user("ada")
        .await
        .with_header("user-agent", "Firefox/140");
    let phone_token = token(&app, "ada", "Phone").await;
    let grace = app.user("grace").await;
    app.exec(
        "UPDATE sessions SET last_used_at = now() - interval '5 minutes' WHERE kind = 'bearer'",
    )
    .await;
    // An expired session is not listed.
    app.exec(&format!(
        "INSERT INTO sessions (user_id, kind, token_hash, expires_at)
         VALUES ({}, 'cookie', sha256('old'), now() - interval '1 second')",
        ada.id()
    ))
    .await;
    let list = ada.get("/api/auth/sessions").await;
    list.assert_status(200);
    let items = list.json()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0]["kind"], "cookie");
    assert_eq!(items[0]["current"], true);
    assert_eq!(items[1]["kind"], "bearer");
    assert_eq!(items[1]["label"], "Phone");
    assert_eq!(items[1]["current"], false);
    assert_eq!(items[1]["id"], phone_token["session"]["id"]);
    for item in &items {
        let mut keys: Vec<&str> = item
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "created_at",
                "current",
                "id",
                "kind",
                "label",
                "last_used_at"
            ]
        );
    }
    // Through the token, the bearer session is the current one.
    let phone = app
        .anon()
        .with_bearer(phone_token["token"].as_str().unwrap());
    let via_phone = phone.get("/api/auth/sessions").await.json();
    let current: Vec<&Value> = via_phone["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["current"] == true)
        .collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["kind"], "bearer");
    // Only the caller's sessions.
    assert_eq!(
        grace.get("/api/auth/sessions").await.json()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.anon()
        .get("/api/auth/sessions")
        .await
        .assert_detail(401, "Not signed in");
}

#[sqlx::test]
async fn revoking_sessions(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let phone_token = token(&app, "ada", "Phone").await;
    let phone = app
        .anon()
        .with_bearer(phone_token["token"].as_str().unwrap());
    let phone_id = phone_token["session"]["id"].as_i64().unwrap();
    let grace_session = session_of(&app, &grace).await;

    ada.delete(&format!("/api/auth/sessions/{grace_session}"))
        .await
        .assert_detail(404, "No such session");
    grace.get("/api/auth/me").await.assert_status(200);
    ada.delete("/api/auth/sessions/999999")
        .await
        .assert_detail(404, "No such session");
    ada.delete("/api/auth/sessions/abc").await.assert_detail(
        400,
        "Input should be a valid integer, unable to parse string as an integer",
    );

    let res = ada.delete(&format!("/api/auth/sessions/{phone_id}")).await;
    res.assert_status(204);
    assert!(res.set_cookies().is_empty());
    phone.get("/api/auth/me").await.assert_status(401);
    ada.get("/api/auth/me").await.assert_status(200);

    // Revoking the current cookie session logs out. The session is stale, so the request
    // slides it first; the handler's clearing cookie wins over the renewal (`maintain_cookie`).
    let own = session_of(&app, &ada).await;
    app.exec(&format!(
        "UPDATE sessions SET last_used_at = now() - interval '2 hours' WHERE id = {own}"
    ))
    .await;
    let res = ada.delete(&format!("/api/auth/sessions/{own}")).await;
    res.assert_status(204);
    let cookies = res.set_cookies();
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert!(cookies[0].starts_with("bookclub_session=;"), "{cookies:?}");
    assert!(cookies[0].contains("Max-Age=0"));
    assert!(ada.cookie().is_none());
    ada.get("/api/auth/me").await.assert_status(401);
}

/// A router with one route per optional or restricting extractor (§5.3), behind the cookie
/// layer as in `lib.rs`. Each route answers the caller's username, or `anonymous`.
fn extractor_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/maybe",
            get(|MaybeUser(user): MaybeUser| async move {
                user.map_or_else(|| "anonymous".to_owned(), |user| user.username)
            }),
        )
        .route(
            "/verified",
            get(|VerifiedUser(user): VerifiedUser| async move { user.username }),
        )
        .route(
            "/admin",
            get(|InstanceAdmin(user): InstanceAdmin| async move { user.username }),
        )
        .layer(from_fn_with_state(state.clone(), maintain_cookie))
        .with_state(state)
}

/// `MaybeUser`, `VerifiedUser` and `InstanceAdmin` for every kind of caller. A dead cookie is
/// cleared only with a 401: `MaybeUser` answers as usual and leaves it (§5.1).
#[sqlx::test]
async fn optional_verified_and_admin_extractors(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user_unverified("grace").await;
    let root = app.admin("root").await;
    let dead = app.user("dead").await;
    app.exec(&format!(
        "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE user_id = {}",
        dead.id()
    ))
    .await;
    let router = extractor_routes(app.state.clone());
    let client = |who: Option<&TestClient>| {
        let client = TestClient::for_router(router.clone());
        client.set_cookie(who.and_then(TestClient::cookie).as_deref());
        client
    };
    let answer = |res: &TestResponse, expected: &str| {
        res.assert_status(200);
        assert_eq!(res.text(), expected);
        assert!(res.set_cookies().is_empty(), "{:?}", res.set_cookies());
    };

    let anon = client(None);
    answer(&anon.get("/maybe").await, "anonymous");
    anon.get("/verified")
        .await
        .assert_detail(401, "Not signed in");
    anon.get("/admin").await.assert_detail(401, "Not signed in");

    let verified = client(Some(&ada));
    answer(&verified.get("/maybe").await, "ada");
    answer(&verified.get("/verified").await, "ada");
    verified.get("/admin").await.assert_detail(404, "Not found");

    let unverified = client(Some(&grace));
    answer(&unverified.get("/maybe").await, "grace");
    unverified
        .get("/verified")
        .await
        .assert_detail(403, "Verify your email address first");
    unverified
        .get("/admin")
        .await
        .assert_detail(404, "Not found");

    let admin = client(Some(&root));
    answer(&admin.get("/admin").await, "root");
    answer(&admin.get("/verified").await, "root");

    let stale = client(Some(&dead));
    answer(&stale.get("/maybe").await, "anonymous");
    assert_eq!(stale.cookie(), dead.cookie(), "the dead cookie is kept");
    for path in ["/verified", "/admin"] {
        let stale = client(Some(&dead));
        let res = stale.get(path).await;
        res.assert_detail(401, "Not signed in");
        let cleared = res.set_cookies();
        assert_eq!(cleared.len(), 1, "{path}: {cleared:?}");
        assert!(cleared[0].starts_with("bookclub_session=;"));
        assert!(cleared[0].contains("Max-Age=0"));
        assert!(stale.cookie().is_none());
    }
}

/// When the session lookup fails, `MaybeUser` answers 500 instead of treating the caller as
/// anonymous, and the cookie is kept. A request without a session needs no lookup.
#[tokio::test]
async fn maybe_user_does_not_hide_a_failed_lookup() {
    let router = extractor_routes(test_state(unreachable_db()));
    let anon = TestClient::for_router(router.clone());
    let res = anon.get("/maybe").await;
    res.assert_status(200);
    assert_eq!(res.text(), "anonymous");

    let signed_in = TestClient::for_router(router);
    signed_in.set_cookie(Some(&BASE64_URL_SAFE_NO_PAD.encode([7u8; 32])));
    let res = signed_in.get("/maybe").await;
    res.assert_detail(500, "Internal Server Error");
    assert!(res.set_cookies().is_empty());
    assert!(signed_in.cookie().is_some());
}

/// The origin check (§5.8) with a real session: a cookie request from a foreign origin is
/// refused before it reaches the handler; same-origin and bearer requests pass.
#[sqlx::test]
async fn cross_site_cookie_requests_are_refused(db: PgPool) {
    let app = TestApp::with_env(db, &PRODUCTION_ENV).await;
    let ada = app.user("ada").await;
    let change = json!({"theme": "ink"});
    let evil = ada.clone().with_header("origin", "https://evil.example");
    evil.patch("/api/auth/me/preferences", change.clone())
        .await
        .assert_detail(403, "Cross-site request refused");
    assert_eq!(ada.get("/api/auth/me").await.json()["theme"], "paper");
    evil.post("/api/auth/logout", json!(null))
        .await
        .assert_detail(403, "Cross-site request refused");
    // Reads are not refused.
    evil.get("/api/auth/me").await.assert_status(200);

    let own = ada
        .clone()
        .with_header("origin", "https://books.example.com");
    own.patch("/api/auth/me/preferences", change.clone())
        .await
        .assert_status(200);

    let phone_token = token(&app, "ada", "Phone").await;
    app.anon()
        .with_bearer(phone_token["token"].as_str().unwrap())
        .with_header("origin", "https://evil.example")
        .patch("/api/auth/me/preferences", json!({"theme": "slate"}))
        .await
        .assert_status(200);
    // Without any cookie there is nothing to forge: login from anywhere is answered.
    app.anon()
        .with_header("origin", "https://evil.example")
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
}

/// A request with a malformed JSON body and no session is 401; with a session the body is
/// checked (conventions §4.2).
#[sqlx::test]
async fn extractor_order(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .request(
            Method::PATCH,
            "/api/auth/password",
            Some("application/json"),
            b"{".to_vec(),
        )
        .await;
    res.assert_detail(400, "JSON decode error");
}

/// `deploy/hosted/revoke-access.sql`, which the runbook runs on a database restored after a
/// compromise of the VM (security review, Phase 2 Wave 2): every cookie session and app token,
/// every open club and sign-up invite, every unused email link and every pending friend request
/// stop working, while the password still signs in and accepted friendships stay (reviewed by
/// hand). Run against the current schema, so the file cannot go stale.
#[sqlx::test]
async fn the_revocation_after_a_compromise_ends_every_reusable_access(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    // A pending request turns into a friendship with one click by the account it is for, like an
    // open invite into a membership (FRIENDS security review): a planted one from every account
    // to an account the attacker keeps would make it everyone's audience.
    let planted = app.user("planted").await;
    let dora = app.user("dora").await;
    dora.post("/api/friends/requests", json!({"username": "planted"}))
        .await
        .assert_status(200);
    for (from, to) in [(&ada, "dora"), (&dora, "ada")] {
        from.post("/api/friends/requests", json!({"username": to}))
            .await
            .assert_status(200);
    }
    let phone = app
        .anon()
        .with_bearer(token(&app, "ada", "phone").await["token"].as_str().unwrap());
    let res = ada
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await;
    res.assert_status(201);
    let link = res.json()["code"].as_str().unwrap().to_owned();
    app.exec(
        "INSERT INTO club_invites (club_id, code, max_uses, expires_at)
         VALUES (NULL, 'SIGNUPCODE', 5, now() + interval '7 days')",
    )
    .await;
    app.anon()
        .post(
            "/api/auth/password-reset",
            json!({"email": "ada@example.test"}),
        )
        .await
        .assert_status(204);
    app.settle().await;
    let reset = app.last_link_token("ada@example.test");
    ada.get("/api/auth/me").await.assert_status(200);
    phone.get("/api/auth/me").await.assert_status(200);

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../deploy/hosted/revoke-access.sql");
    app.exec(&std::fs::read_to_string(&path).unwrap()).await;

    ada.get("/api/auth/me").await.assert_status(401);
    phone.get("/api/auth/me").await.assert_status(401);
    let bob = app.user("bob").await;
    bob.post(&format!("/api/invites/{link}/accept"), json!({}))
        .await
        .assert_detail(400, "That invite code is not valid");
    app.anon()
        .post(
            "/api/auth/register",
            json!({"username": "cleo", "email": "cleo@example.test",
                   "password": PASSWORD, "invite_code": "SIGNUPCODE"}),
        )
        .await
        .assert_detail(400, "That invite code is not valid");
    app.anon()
        .post(
            "/api/auth/password-reset/confirm",
            json!({"token": reset, "new_password": "a whole new passphrase"}),
        )
        .await
        .assert_detail(400, "That link has expired or was already used");
    let again = app.anon();
    again
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    again.get("/api/auth/me").await.assert_status(200);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM friendships WHERE accepted_at IS NULL")
            .await,
        0
    );
    let friends = again.get("/api/friends").await;
    friends.assert_status(200);
    assert_eq!(friends.json()["friends"][0]["user"]["username"], "dora");
    planted.get("/api/auth/me").await.assert_status(401);
    let planted = app.anon();
    planted
        .post(
            "/api/auth/login",
            json!({"username": "planted", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    planted
        .post("/api/friends/requests/dora/accept", json!({}))
        .await
        .assert_detail(404, "No such friend request");
}
