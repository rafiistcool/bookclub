//! Suspending accounts (#55, Q15): `POST /api/admin/users/{username}/suspend` and `/unsuspend`,
//! `GET /api/admin/users/suspended`, and what a suspension does to sign-in.

mod common;

use std::net::IpAddr;
use std::time::Duration;

use bookclub_server::admin::notify;
use bookclub_server::admin::{self, users::lift_suspension};
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
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE unsuspended_at IS NOT NULL")
            .await,
        0,
        "lifting no suspension records none"
    );
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
/// A suspension through the API also ends the admin rights; this one is set in the database, as
/// a suspension from before that or by hand would be.
#[sqlx::test]
async fn suspended_admins_are_not_notified(db: PgPool) {
    let app = TestApp::new(db).await;
    app.admin("root").await;
    let grace = app.admin("grace").await;
    let ada = app.user("ada").await;
    app.exec("UPDATE users SET suspended_at = now() WHERE username = 'grace'")
        .await;
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

const INVALID: &str = "That invite code is not valid";

/// A club invite of `club`, made by `admin` (a club admin): its code.
async fn club_link(admin: &TestClient, club: &TestClub) -> String {
    let res = admin
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await;
    res.assert_status(201);
    res.json()["code"].as_str().unwrap().to_owned()
}

/// A sign-up invite minted by the instance admin `admin`: its code.
async fn signup_invite(admin: &TestClient, body: Value) -> String {
    let res = admin.post("/api/admin/invites", body).await;
    res.assert_status(201);
    res.json()["code"].as_str().unwrap().to_owned()
}

async fn register(app: &TestApp, username: &str, code: &str) -> TestResponse {
    app.anon()
        .post(
            "/api/auth/register",
            json!({"username": username, "email": format!("{username}@example.test"),
                   "password": PASSWORD, "invite_code": code}),
        )
        .await
}

async fn accept(client: &TestClient, code: &str) -> TestResponse {
    client
        .post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
}

async fn revoked(app: &TestApp, code: &str) -> bool {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM club_invites WHERE code = '{code}' AND revoked_at IS NOT NULL"
    ))
    .await
        == 1
}

/// A suspension revokes every active invite the account knows (security review, ADM2 merge),
/// so a second account can neither sign up with one (`SIGNUP_MODE=invite`) nor rejoin the club
/// with one: a suspended club owner's links, every invite of a club they administer, the link a
/// plain member redeemed, and, for a suspended instance admin, every sign-up invite (they list
/// them all). Invites they never knew keep working.
#[sqlx::test]
async fn a_suspension_revokes_the_invites_the_account_knows(db: PgPool) {
    let app = TestApp::with_env(db, &[("SIGNUP_MODE", "invite")]).await;
    let root = app.admin("root").await;
    // Mallory owns the club Ada and Bob are in; Bob joined through a link that still works.
    let mallory = app.user("mallory").await;
    let club = app.club(&mallory, "Lesekreis").await;
    let hers = club_link(&mallory, &club).await;
    let ada = app.user("ada").await;
    app.join(&club, &ada, "admin").await;
    let adas = club_link(&ada, &club).await;
    let bob = app.user("bob").await;
    accept(&bob, &adas).await.assert_status(200);
    // A club Mallory never belonged to.
    let cleo = app.user("cleo").await;
    let other = app.club(&cleo, "Elsewhere").await;
    let elsewhere = club_link(&cleo, &other).await;

    suspend(&root, "mallory").await.assert_status(200);
    assert!(revoked(&app, &hers).await, "her own link");
    assert!(
        revoked(&app, &adas).await,
        "she listed every invite of her club"
    );
    assert!(!revoked(&app, &elsewhere).await, "another club");
    register(&app, "mallory2", &hers)
        .await
        .assert_detail(400, INVALID);
    accept(&app.user("mallory3").await, &adas)
        .await
        .assert_detail(400, INVALID);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships WHERE club_id = {}",
            club.id
        ))
        .await,
        3,
        "Mallory, Ada and Bob"
    );
    accept(&app.user("dan").await, &elsewhere)
        .await
        .assert_status(200);

    // A plain member: the link he redeemed, not the ones he never saw.
    let fresh = club_link(&ada, &club).await;
    let bobs_way_in = club_link(&ada, &club).await;
    app.exec(&format!(
        "INSERT INTO club_invite_uses (invite_id, user_id)
         SELECT id, {} FROM club_invites WHERE code = '{bobs_way_in}'",
        bob.id()
    ))
    .await;
    suspend(&root, "bob").await.assert_status(200);
    assert!(revoked(&app, &bobs_way_in).await, "the link he used");
    assert!(!revoked(&app, &fresh).await, "never his to see");

    // An instance admin lists every sign-up invite: all of them go, and the one that would
    // never expire too.
    let eve = app.admin("eve").await;
    let forever = signup_invite(&eve, json!({"max_uses": 100})).await;
    let roots = signup_invite(&root, json!({})).await;
    suspend(&root, "eve").await.assert_status(200);
    assert!(revoked(&app, &forever).await);
    assert!(revoked(&app, &roots).await);
    register(&app, "eve2", &forever)
        .await
        .assert_detail(400, INVALID);
    drop(eve);
}

/// A suspension ends the instance admin rights for good: lifting it (as `takedown.md` §7 does
/// for an export or a deletion) gives back the sign-in, not the admin routes, so a suspended
/// admin cannot turn around and suspend the operator (security review, ADM2 merge). The
/// operator's commands lift a suspension and end admin rights without the admin page.
#[sqlx::test]
async fn a_suspension_ends_the_admin_rights_for_good(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let eve = app.admin("eve").await;
    let res = suspend(&root, "eve").await;
    res.assert_status(200);
    assert_eq!(res.json()["is_admin"], false);
    let res = unsuspend(&root, "eve").await;
    res.assert_status(200);
    assert_eq!(res.json()["is_admin"], false);
    let browser = app.anon();
    browser
        .post(
            "/api/auth/login",
            json!({"username": "eve", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    browser.get("/api/auth/me").await.assert_status(200);
    browser
        .post("/api/admin/users/root/suspend", json!({}))
        .await
        .assert_detail(404, "Not found");
    root.get("/api/admin/summary").await.assert_status(200);

    // From the host: `bookclub-server unsuspend` and `revoke-admin`.
    let mut conn = app.db.acquire().await.unwrap();
    suspend(&root, "eve").await.assert_status(200);
    assert_eq!(
        lift_suspension(&mut conn, " EVE ").await.unwrap(),
        Some(eve.id())
    );
    assert_eq!(lift_suspension(&mut conn, "nobody").await.unwrap(), None);
    login(&app, "eve", PASSWORD).await.assert_status(204);
    assert!(admin::make_admin(&mut conn, "eve").await.unwrap());
    assert!(admin::revoke_admin(&mut conn, "Eve").await.unwrap());
    assert!(!admin::revoke_admin(&mut conn, "nobody").await.unwrap());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE is_admin")
            .await,
        1,
        "root only"
    );
}

/// The acting admin's row is read again under the lock: an admin whom another admin suspended
/// while their own suspension request waited decides nothing. Two admins suspending each other
/// at the same moment therefore cannot both succeed and leave no admin.
#[sqlx::test]
async fn an_admin_suspended_meanwhile_suspends_nobody(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let eve = app.admin("eve").await;
    // Eve's request locks root's row (the lower id), then waits for her own, which this
    // transaction holds while it suspends her.
    let mut other = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(eve.id())
        .execute(&mut *other)
        .await
        .unwrap();
    let hers = tokio::spawn({
        let eve = eve.clone();
        async move { suspend(&eve, "root").await }
    });
    until_a_lock_wait(&app.db).await;
    sqlx::query("UPDATE users SET suspended_at = now(), is_admin = false WHERE id = $1")
        .bind(eve.id())
        .execute(&mut *other)
        .await
        .unwrap();
    other.commit().await.unwrap();
    hers.await.unwrap().assert_detail(404, "Not found");
    root.get("/api/admin/summary").await.assert_status(200);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE suspended_at IS NOT NULL")
            .await,
        1,
        "eve only"
    );
}

/// Waits up to 10 s until a session of this test's database waits for a lock.
async fn until_a_lock_wait(db: &PgPool) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no session waited for a lock");
}

/// `bookclub-server unsuspend <name>` lifts a suspension and `revoke-admin <name>` ends admin
/// rights from the host, for when no admin can use the admin page; an unknown name fails.
#[sqlx::test]
async fn the_operator_commands_lift_a_suspension_and_end_admin_rights(db: PgPool) {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let name = db.connect_options().get_database().unwrap().to_owned();
    let mut url = url::Url::parse(&base).unwrap();
    url.set_path(&name);
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_bookclub-server"))
            .args(args)
            .env_clear()
            .env("DATABASE_URL", url.as_str())
            .env("DEV_MODE", "true")
            .env("RUST_LOG", "error")
            .output()
            .unwrap()
    };
    let app = TestApp::new(db).await;
    app.admin("root").await;
    app.admin("eve").await;
    app.exec("UPDATE users SET suspended_at = now() WHERE username = 'root'")
        .await;

    let output = run(&["unsuspend", "ROOT"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "root can sign in again\n"
    );
    login(&app, "root", PASSWORD).await.assert_status(204);
    let output = run(&["revoke-admin", "eve"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "eve is no longer an instance admin\n"
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE is_admin")
            .await,
        1
    );
    for command in ["unsuspend", "revoke-admin"] {
        let output = run(&[command, "nobody"]);
        assert!(!output.status.success(), "{command}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("no user named \"nobody\""),
            "{command}"
        );
    }
}
