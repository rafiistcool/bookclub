//! Login, logout and the login limits (endpoint-map §2, docs/phase1/conventions.md §5.1, §5.4,
//! §10.1).

mod common;

use std::net::IpAddr;
use std::time::Duration;

use common::*;
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

/// How long a test waits for an answer that must come without verifying a password.
const WAIT: Duration = Duration::from_secs(10);

/// Made by the Python app's `argon2.PasswordHasher()` (argon2-cffi 25.1: argon2id, v=19,
/// m=65536, t=3, p=4) for "correct horse battery staple" (backend/.venv, 2026-10-27).
const LEGACY_HASH: &str = "$argon2id$v=19$m=65536,t=3,p=4$HriveBINAQZ51LoIBI0BgA$KL9lKfOZE02xEwFAY8SIj1fwtdBbdbNdM6KT3FTsUSw";
/// The same hasher's hash of "pässwörd1".
const LEGACY_UMLAUT_HASH: &str = "$argon2id$v=19$m=65536,t=3,p=4$NFXBxWeVqjJzxkA8rUU40Q$DRwLtlwftPydnyd3qMfZg9PiA5oCw6OHTSs92t1Lth4";

fn credentials(username: &str, password: &str) -> Value {
    json!({"username": username, "password": password})
}

async fn login(client: &TestClient, username: &str, password: &str) -> TestResponse {
    client
        .post("/api/auth/login", credentials(username, password))
        .await
}

fn ip(n: u16) -> IpAddr {
    format!("2001:db8:{n:x}::1").parse().unwrap()
}

/// Takes every permit of the password semaphore: until the guard drops, no password is hashed
/// or verified, and requests that need one wait.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

/// Waits until the login failure key `key` is blocked, i.e. until the request that made the
/// tenth hit has looked the account up and waits to verify the password.
async fn until_limited(app: &TestApp, key: &str) {
    timeout(WAIT, async {
        while !app.state.limits.login_failures.is_limited(key) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the request never reached the password check");
}

/// parity: test_auth.py::test_me_requires_session
#[sqlx::test]
async fn me_requires_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let res = app.anon().get("/api/auth/me").await;
    res.assert_detail(401, "Not signed in");
    assert!(res.set_cookies().is_empty());
}

/// parity: test_auth.py::test_login_and_logout
#[sqlx::test]
async fn login_and_logout(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let logout = ada.post("/api/auth/logout", json!(null)).await;
    logout.assert_status(204);
    assert!(logout.body.is_empty());
    let clearing = logout.set_cookies();
    assert_eq!(clearing.len(), 1);
    assert!(
        clearing[0].starts_with("bookclub_session=;"),
        "{clearing:?}"
    );
    assert!(clearing[0].contains("Max-Age=0"));
    assert!(ada.cookie().is_none());
    ada.get("/api/auth/me").await.assert_status(401);

    let res = login(&ada, "ada", PASSWORD).await;
    res.assert_status(204);
    assert!(res.body.is_empty());
    assert_eq!(ada.get("/api/auth/me").await.json()["username"], "ada");
    login(&ada, "ada", "wrong-password")
        .await
        .assert_detail(401, "Wrong username or password");
}

/// Logout deletes the server session: a copy of the cookie stops working.
#[sqlx::test]
async fn logout_revokes_the_session_on_the_server(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let copy = app.anon();
    copy.set_cookie(ada.cookie().as_deref());
    copy.get("/api/auth/me").await.assert_status(200);
    ada.post("/api/auth/logout", json!(null))
        .await
        .assert_status(204);
    copy.get("/api/auth/me").await.assert_status(401);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM sessions").await, 0);
}

/// Without a valid session, logout answers 204 and sends no cookie (auth spec §1.3, probed).
#[sqlx::test]
async fn logout_without_a_session_sends_no_cookie(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    let res = anon.post("/api/auth/logout", json!(null)).await;
    res.assert_status(204);
    assert!(res.set_cookies().is_empty());

    anon.set_cookie(Some("not-a-session"));
    let res = anon.post("/api/auth/logout", json!(null)).await;
    res.assert_status(204);
    assert!(res.set_cookies().is_empty());

    let ada = app.user("ada").await;
    app.exec("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .await;
    let res = ada.post("/api/auth/logout", json!(null)).await;
    res.assert_status(204);
    assert!(res.set_cookies().is_empty());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM sessions").await, 0);
}

/// parity: test_auth.py::test_login_throttle
///
/// Also: the right password is refused while the account is blocked (auth spec §1.2).
#[sqlx::test]
async fn ten_failures_block_the_account(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let anon = app.anon();
    for _ in 0..10 {
        login(&anon, "ada", "nope")
            .await
            .assert_detail(401, "Wrong username or password");
    }
    let blocked = login(&anon, "ada", "nope").await;
    blocked.assert_detail(429, "Too many attempts. Wait a few minutes.");
    let wait: u64 = blocked.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=600).contains(&wait), "{wait}");
    login(&anon, "ada", PASSWORD)
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    // The email address names the same account and bucket.
    login(&anon, "ada@example.test", PASSWORD)
        .await
        .assert_status(429);
}

/// A success clears the failures, so the next ten failures are counted from zero.
#[sqlx::test]
async fn a_success_clears_the_failures(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let anon = app.anon();
    for _ in 0..9 {
        login(&anon, "ada", "nope").await.assert_status(401);
    }
    login(&anon, "ada", PASSWORD).await.assert_status(204);
    for _ in 0..10 {
        login(&anon, "ada", "nope").await.assert_status(401);
    }
    login(&anon, "ada", "nope").await.assert_status(429);
}

/// Unknown names accumulate failures too (auth spec §2.3), in a hashed bucket.
#[sqlx::test]
async fn unknown_names_are_limited_too(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let anon = app.anon();
    for _ in 0..10 {
        login(&anon, "nobody", "nope").await.assert_status(401);
    }
    login(&anon, "Nobody ", "nope").await.assert_status(429);
    // A real account is not in that bucket.
    login(&anon, "ada", PASSWORD).await.assert_status(204);
}

#[sqlx::test]
async fn login_by_verified_email_only(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    app.user_unverified("grace").await;
    login(&app.anon(), " ADA@example.TEST ", PASSWORD)
        .await
        .assert_status(204);
    login(&app.anon(), "grace@example.test", PASSWORD)
        .await
        .assert_detail(401, "Wrong username or password");
    login(&app.anon(), "grace", PASSWORD)
        .await
        .assert_status(204);
}

/// 30 attempts per IP address in ten minutes, whatever the account.
#[sqlx::test]
async fn login_attempts_are_limited_per_ip(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let here = app.anon().with_ip(ip(1));
    for n in 0..30 {
        let res = login(&here, &format!("someone{n}"), "nope").await;
        assert_ne!(res.status.as_u16(), 429, "attempt {n}");
    }
    login(&here, "ada", PASSWORD)
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    login(&app.anon().with_ip(ip(2)), "ada", PASSWORD)
        .await
        .assert_status(204);
}

/// `check_and_hit` before verifying (§10.1): of many concurrent wrong logins for one account,
/// at most ten get to verify a password. No password can be verified while the test holds the
/// semaphore, so thirty refusals must arrive before any verdict: a limit that counted only
/// after a failed verify would let all forty wait for the semaphore.
#[sqlx::test]
async fn concurrent_wrong_logins_verify_at_most_ten_passwords(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let held = hold_passwords(&app).await;
    let mut pending: FuturesUnordered<_> = (0..40)
        .map(|n| {
            let client = app.anon().with_ip(ip(n));
            async move { login(&client, "ada", "nope").await }
        })
        .collect();
    for n in 0..30 {
        let res = timeout(WAIT, pending.next())
            .await
            .unwrap_or_else(|_| panic!("only {n} attempts were refused before verifying"))
            .unwrap();
        res.assert_detail(429, "Too many attempts. Wait a few minutes.");
    }
    drop(held);
    for _ in 0..10 {
        let res = timeout(WAIT, pending.next()).await.unwrap().unwrap();
        res.assert_detail(401, "Wrong username or password");
    }
    assert!(pending.next().await.is_none());
}

/// A login (or token creation) whose password was verified while a password change or reset
/// committed creates no session (§5.4): its transaction sees the new hash and answers 401, so
/// the change's revocation cannot miss the new session or token.
#[sqlx::test]
async fn a_login_racing_a_password_change_creates_no_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let changed = app.state.passwords.hash("newpassword1").await.unwrap();
    let key = format!("u:{}", ada.id());
    // Eight earlier failures: the two attempts' own hits block the key, which shows that both
    // have read the old hash.
    for _ in 0..8 {
        app.state.limits.login_failures.hit(&key);
    }
    let held = hold_passwords(&app).await;
    let browser = app.anon();
    let cookie_login = tokio::spawn({
        let browser = browser.clone();
        async move { login(&browser, "ada", PASSWORD).await }
    });
    let app_login = tokio::spawn({
        let client = app.anon().with_ip(ip(1));
        async move {
            client
                .post(
                    "/api/auth/tokens",
                    json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
                )
                .await
        }
    });
    until_limited(&app, &key).await;
    // The change commits while both attempts wait to verify the old password.
    app.exec(&format!(
        "UPDATE users SET password_hash = '{changed}', password_changed_at = now() WHERE id = {id};
         DELETE FROM sessions WHERE user_id = {id};",
        id = ada.id()
    ))
    .await;
    drop(held);
    for res in [cookie_login.await.unwrap(), app_login.await.unwrap()] {
        res.assert_detail(401, "Wrong username or password");
        assert!(res.set_cookies().is_empty());
    }
    assert!(browser.cookie().is_none());
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE user_id = {}",
            ada.id()
        ))
        .await,
        0
    );
    login(&app.anon(), "ada", "newpassword1")
        .await
        .assert_status(204);
}

/// A NUL character, which Postgres `text` cannot store, is 400 and never a 500.
#[sqlx::test]
async fn nul_characters_in_credentials_are_400(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    for body in [
        json!({"username": "ada\u{0}", "password": PASSWORD}),
        json!({"username": "ada@example.test\u{0}", "password": PASSWORD}),
        json!({"username": "ada", "password": "pass\u{0}word1"}),
    ] {
        app.anon()
            .post("/api/auth/login", body.clone())
            .await
            .assert_detail(400, "Invalid value");
        let mut token = body;
        token["label"] = json!("phone");
        app.anon()
            .post("/api/auth/tokens", token)
            .await
            .assert_detail(400, "Invalid value");
    }
    app.anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "ph\u{0}one"}),
        )
        .await
        .assert_detail(400, "Invalid value");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM sessions").await, 1);
}

/// A legacy argon2-cffi hash (m=65536, t=3, p=4) verifies unchanged and is rehashed with the
/// current parameters inside the login transaction.
#[sqlx::test]
async fn a_legacy_hash_logs_in_and_is_rehashed(db: PgPool) {
    let app = TestApp::new(db).await;
    app.exec(&format!(
        "INSERT INTO users (username, password_hash) VALUES
            ('ada', '{LEGACY_HASH}'), ('grace', '{LEGACY_UMLAUT_HASH}')"
    ))
    .await;
    login(&app.anon(), "ada", "correct horse battery")
        .await
        .assert_status(401);
    let stored = |name: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE username = $1")
                .bind(name)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        stored("ada").await,
        LEGACY_HASH,
        "a failure rehashes nothing"
    );

    login(&app.anon(), "ada", "correct horse battery staple")
        .await
        .assert_status(204);
    let rehashed = stored("ada").await;
    assert!(
        rehashed.starts_with("$argon2id$v=19$m=8,t=1,p=1$"),
        "{rehashed}"
    );
    login(&app.anon(), "ada", "correct horse battery staple")
        .await
        .assert_status(204);
    assert_eq!(stored("ada").await, rehashed, "a current hash stays");

    login(&app.anon(), "grace", "pässwörd1")
        .await
        .assert_status(204);
    assert_ne!(stored("grace").await, LEGACY_UMLAUT_HASH);
}

/// An unparsable stored hash is a wrong password, never a 500.
#[sqlx::test]
async fn a_garbage_hash_is_a_mismatch(db: PgPool) {
    let app = TestApp::new(db).await;
    app.exec("INSERT INTO users (username, password_hash) VALUES ('ada', 'hash')")
        .await;
    login(&app.anon(), "ada", "hash")
        .await
        .assert_detail(401, "Wrong username or password");
}

/// Logging in while signed in switches the session; the old one is deleted.
#[sqlx::test]
async fn logging_in_while_signed_in_switches_the_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.user("grace").await;
    login(&ada, "grace", PASSWORD).await.assert_status(204);
    assert_eq!(ada.get("/api/auth/me").await.json()["username"], "grace");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE user_id = {}",
            ada.id()
        ))
        .await,
        0
    );
}

/// Signed-out requests are 401 before the body is looked at (conventions §4.2).
#[sqlx::test]
async fn auth_comes_before_the_body(db: PgPool) {
    let app = TestApp::new(db).await;
    let res = app
        .anon()
        .request(
            axum::http::Method::PATCH,
            "/api/auth/me/preferences",
            Some("application/json"),
            b"{not json".to_vec(),
        )
        .await;
    res.assert_detail(401, "Not signed in");
    login(&app.anon(), "", "")
        .await
        .assert_detail(401, "Wrong username or password");
    app.anon()
        .post("/api/auth/login", json!({"username": "ada"}))
        .await
        .assert_detail(400, "Field required");
}
