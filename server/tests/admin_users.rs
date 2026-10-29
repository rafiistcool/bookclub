//! Suspending accounts (#55, Q15): `POST /api/admin/users/{username}/suspend` and `/unsuspend`,
//! `GET /api/admin/users/suspended`, and what a suspension does to sign-in.

mod common;

use std::net::IpAddr;
use std::time::Duration;

use bookclub_server::admin::notify;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

async fn suspend(admin: &TestClient, username: &str) -> TestResponse {
    admin
        .post(&format!("/api/admin/users/{username}/suspend"), json!({}))
        .await
}

async fn unsuspend(admin: &TestClient, username: &str) -> TestResponse {
    admin
        .post(&format!("/api/admin/users/{username}/unsuspend"), json!({}))
        .await
}

async fn login(app: &TestApp, username: &str, password: &str) -> TestResponse {
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": username, "password": password}),
        )
        .await
}

async fn bearer_token(app: &TestApp, username: &str) -> TestResponse {
    app.anon()
        .post(
            "/api/auth/tokens",
            json!({"username": username, "password": PASSWORD, "label": "phone"}),
        )
        .await
}

async fn sessions_of(app: &TestApp, user: &TestClient) -> i64 {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM sessions WHERE user_id = {}",
        user.id()
    ))
    .await
}

/// Acceptance (#55): after a suspension the user's cookie and bearer token get 401, every
/// session is gone, and login (and a new bearer token) is refused with a clear message once the
/// password matched; a wrong password still reads as one. The content stays. Unsuspending
/// restores login.
#[sqlx::test]
async fn a_suspension_signs_the_account_out_until_it_is_lifted(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let token = bearer_token(&app, "ada").await.json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let phone = app.anon().with_bearer(&token);
    phone.get("/api/auth/me").await.assert_status(200);
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(sessions_of(&app, &ada).await, 2);

    let res = suspend(&root, "ada").await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["username"], "ada");
    assert_eq!(out["is_admin"], false);
    assert!(out["suspended_at"].is_string());
    assert!(out.get("email").is_none(), "the admin sees no address");

    let res = ada.get("/api/auth/me").await;
    res.assert_detail(401, "Not signed in");
    phone
        .get("/api/auth/me")
        .await
        .assert_detail(401, "Not signed in");
    assert_eq!(sessions_of(&app, &ada).await, 0);
    login(&app, "ada", PASSWORD)
        .await
        .assert_detail(403, "This account is suspended");
    login(&app, "ada@example.test", PASSWORD)
        .await
        .assert_detail(403, "This account is suspended");
    bearer_token(&app, "ada")
        .await
        .assert_detail(403, "This account is suspended");
    login(&app, "ada", "wrong password")
        .await
        .assert_detail(401, "Wrong username or password");
    assert_eq!(sessions_of(&app, &ada).await, 0, "no session was made");
    // The content stays.
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships WHERE club_id = {} AND user_id = {}",
            club.id,
            ada.id()
        ))
        .await,
        1
    );

    let res = unsuspend(&root, "ada").await;
    res.assert_status(200);
    assert_eq!(res.json()["suspended_at"], Value::Null);
    let res = login(&app, "ada", PASSWORD).await;
    res.assert_status(204);
    assert_eq!(sessions_of(&app, &ada).await, 1);
    bearer_token(&app, "ada").await.assert_status(201);
}

/// A session that slipped past the suspension (made by a login that raced it) is refused where
/// the caller is resolved, and its cookie is cleared, as for a dead session. Admin routes refuse
/// a suspended admin as well.
#[sqlx::test]
async fn a_suspended_account_has_no_live_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let ada = app.user("ada").await;
    let grace = app.admin("grace").await;
    for who in [&ada, &grace] {
        app.exec(&format!(
            "UPDATE users SET suspended_at = now() WHERE id = {}",
            who.id()
        ))
        .await;
    }
    assert_eq!(sessions_of(&app, &ada).await, 1, "left in place on purpose");
    let res = ada.get("/api/auth/me").await;
    res.assert_detail(401, "Not signed in");
    assert!(
        res.set_cookies()
            .iter()
            .any(|cookie| cookie.contains("Max-Age=0")),
        "{:?}",
        res.set_cookies()
    );
    grace
        .get("/api/admin/summary")
        .await
        .assert_detail(401, "Not signed in");
    root.get("/api/admin/summary").await.assert_status(200);
}

/// An admin cannot suspend their own account (400); an unknown name is 404 `No account with that
/// name`; names are normalised like every username lookup; suspending twice keeps the first
/// time, and unsuspending an account that is not suspended changes nothing.
#[sqlx::test]
async fn suspensions_are_checked(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let grace = app.admin("grace").await;
    app.user("ada").await;
    suspend(&root, "root")
        .await
        .assert_detail(400, "You cannot suspend your own account");
    suspend(&root, "%20ROOT%20")
        .await
        .assert_detail(400, "You cannot suspend your own account");
    suspend(&root, "nobody")
        .await
        .assert_detail(404, "No account with that name");
    unsuspend(&root, "nobody")
        .await
        .assert_detail(404, "No account with that name");
    let first = suspend(&root, "ADA").await.json()["suspended_at"].clone();
    app.exec("UPDATE users SET suspended_at = suspended_at - interval '1 hour'")
        .await;
    let again = suspend(&root, "ada").await.json()["suspended_at"].clone();
    assert_ne!(again, first, "moved back an hour above");
    let stored = app
        .scalar_i64(
            "SELECT count(*) FROM users
             WHERE suspended_at < now() - interval '59 minutes' AND username = 'ada'",
        )
        .await;
    assert_eq!(stored, 1, "the first suspension's time stays");
    let res = unsuspend(&root, "grace").await;
    res.assert_status(200);
    assert_eq!(res.json()["suspended_at"], Value::Null);
    // Another admin can be suspended; then they cannot use the admin routes.
    suspend(&root, "grace").await.assert_status(200);
    grace
        .get("/api/admin/summary")
        .await
        .assert_detail(401, "Not signed in");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE suspended_at IS NOT NULL")
            .await,
        2
    );
}

/// The admin page lists the suspended accounts, the latest suspension first, without their
/// addresses.
#[sqlx::test]
async fn the_admin_page_lists_suspended_accounts(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for name in ["ada", "bob", "cleo"] {
        app.user(name).await;
    }
    let list = root.get("/api/admin/users/suspended").await;
    list.assert_status(200);
    assert_eq!(list.json(), json!({"items": []}));
    suspend(&root, "bob").await.assert_status(200);
    suspend(&root, "ada").await.assert_status(200);
    app.exec("UPDATE users SET suspended_at = now() - interval '1 day' WHERE username = 'bob'")
        .await;
    let items = root.get("/api/admin/users/suspended").await.json()["items"].clone();
    let names: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ada", "bob"]);
    let keys: Vec<&String> = items[0].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "avatar_url",
            "created_at",
            "id",
            "is_admin",
            "suspended_at",
            "username"
        ]
    );
    unsuspend(&root, "ada").await.assert_status(200);
    let items = root.get("/api/admin/users/suspended").await.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["username"], "bob");
}

/// A suspended admin gets no admin notification mail (the job mails only admins who can act).
#[sqlx::test]
async fn suspended_admins_are_not_notified(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let grace = app.admin("grace").await;
    let ada = app.user("ada").await;
    suspend(&root, "grace").await.assert_status(200);
    app.exec(&format!(
        "INSERT INTO reports (target_kind, reason, created_by) VALUES ('cover', 'other', {})",
        ada.id()
    ))
    .await;
    let mut conn = app.db.acquire().await.unwrap();
    let run = notify::run(&app.state, &mut conn).await.unwrap();
    assert_eq!(run.report_mails, 1);
    assert_eq!(app.email.sent_to("root@example.test").len(), 1);
    assert!(app.email.sent_to("grace@example.test").is_empty());
    drop(grace);
}

/// Takes every password verification slot, so a login waits right before it verifies the
/// password until the permit is dropped.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

/// Waits until the login failure key `key` is blocked, i.e. until the request that made the
/// tenth hit has looked the account up and waits to verify the password.
async fn until_limited(app: &TestApp, key: &str) {
    timeout(Duration::from_secs(10), async {
        while !app.state.limits.login_failures.is_limited(key) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the request never reached the password check");
}

/// A login and a bearer token request racing a suspension create no session (`login::confirm`):
/// both read the account before the suspension and wait to verify the password; the suspension
/// commits meanwhile; the transaction that would create the session locks the user row, sees
/// the suspension and answers 403. A check of `suspended_at` before the password verification,
/// outside that transaction, would let both through and leave a live session behind.
#[sqlx::test]
async fn a_login_racing_a_suspension_creates_no_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let ada = app.user("ada").await;
    let key = format!("u:{}", ada.id());
    // Eight earlier failures: the two attempts' own hits block the key, which shows that both
    // have looked the account up.
    for _ in 0..8 {
        app.state.limits.login_failures.hit(&key);
    }
    let held = hold_passwords(&app).await;
    let browser = app.anon();
    let cookie_login = tokio::spawn({
        let browser = browser.clone();
        async move {
            browser
                .post(
                    "/api/auth/login",
                    json!({"username": "ada", "password": PASSWORD}),
                )
                .await
        }
    });
    let app_login = tokio::spawn({
        let client = app.anon().with_ip(IpAddr::from([10, 0, 0, 1]));
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
    suspend(&root, "ada").await.assert_status(200);
    drop(held);
    for res in [cookie_login.await.unwrap(), app_login.await.unwrap()] {
        res.assert_detail(403, "This account is suspended");
        assert!(res.set_cookies().is_empty());
    }
    assert!(browser.cookie().is_none());
    assert_eq!(sessions_of(&app, &ada).await, 0);
}
