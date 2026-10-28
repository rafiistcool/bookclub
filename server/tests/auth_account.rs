//! The signed-in user's account: `GET /api/auth/me`, preferences, notification preferences
//! and the password change (endpoint-map §2).

mod common;

use std::net::IpAddr;
use std::time::Duration;

use common::*;
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

/// How long a test waits for an answer that must come without verifying a password.
const WAIT: Duration = Duration::from_secs(10);

async fn login(client: &TestClient, username: &str, password: &str) -> TestResponse {
    client
        .post(
            "/api/auth/login",
            json!({"username": username, "password": password}),
        )
        .await
}

fn change(current: &str, new: &str) -> serde_json::Value {
    json!({"current_password": current, "new_password": new})
}

/// Takes every permit of the password semaphore: until the guard drops, no password is hashed
/// or verified, and requests that need one wait.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

#[sqlx::test]
async fn me_is_the_full_user_out(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.admin("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&grace, "Thursday Readers").await;
    app.join(&club, &ada, "admin").await;
    let older = app.club(&ada, "Older").await;
    app.exec(&format!(
        "UPDATE club_memberships SET joined_at = now() - interval '1 day' WHERE club_id = {}",
        older.id
    ))
    .await;
    let me = ada.get("/api/auth/me").await;
    me.assert_status(200);
    assert_eq!(
        me.json(),
        json!({
            "id": ada.id(),
            "username": "ada",
            "email": "ada@example.test",
            "email_verified": true,
            "pending_email": null,
            "needs_email_prompt": false,
            "needs_terms": false,
            "theme": "paper",
            "color_mode": "system",
            "locale": "en",
            "timezone": "UTC",
            "avatar_url": null,
            "is_admin": true,
            "clubs": [
                {"slug": club.slug, "name": "Thursday Readers", "role": "admin", "timezone": "UTC", "member_count": 2},
                {"slug": older.slug, "name": "Older", "role": "owner", "timezone": "UTC", "member_count": 1},
            ],
        })
    );
}

/// `pending_email` shows an email change waiting for confirmation on a verified account;
/// imported accounts without an address get the one-time prompt.
#[sqlx::test]
async fn pending_email_and_the_email_prompt(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({id}, 'verify_email', 'old@example.test', sha256('a'), now() + interval '1 hour'),
                ({id}, 'verify_email', 'new@example.test', sha256('b'), now() + interval '2 hours'),
                ({id}, 'verify_email', 'used@example.test', sha256('c'), now() + interval '3 hours')",
        id = ada.id()
    ))
    .await;
    app.exec(
        "UPDATE email_tokens SET created_at = now() - interval '1 minute' WHERE email = 'old@example.test';
         UPDATE email_tokens SET used_at = now() WHERE email = 'used@example.test'",
    )
    .await;
    let me = ada.get("/api/auth/me").await.json();
    assert_eq!(me["email"], "ada@example.test");
    assert_eq!(me["pending_email"], "new@example.test");

    // An unverified account's own address is not "pending": email_verified says it.
    let grace = app.user_unverified("grace").await;
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({}, 'verify_email', 'grace@example.test', sha256('d'), now() + interval '1 hour')",
        grace.id()
    ))
    .await;
    let me = grace.get("/api/auth/me").await.json();
    assert_eq!(me["email_verified"], false);
    assert_eq!(me["pending_email"], serde_json::Value::Null);

    app.exec(&format!(
        "UPDATE users SET email = NULL, email_verified_at = NULL WHERE id = {}",
        grace.id()
    ))
    .await;
    assert_eq!(
        grace.get("/api/auth/me").await.json()["needs_email_prompt"],
        true
    );
    app.exec(&format!(
        "UPDATE users SET email_prompt_dismissed_at = now() WHERE id = {}",
        grace.id()
    ))
    .await;
    assert_eq!(
        grace.get("/api/auth/me").await.json()["needs_email_prompt"],
        false
    );
}

/// parity: test_auth.py::test_me_carries_default_preferences
#[sqlx::test]
async fn me_carries_default_preferences(db: PgPool) {
    let app = TestApp::new(db).await;
    let me = app.user("ada").await.get("/api/auth/me").await.json();
    assert_eq!(me["theme"], "paper");
    assert_eq!(me["color_mode"], "system");
}

/// parity: test_auth.py::test_update_theme_alone_leaves_color_mode
#[sqlx::test]
async fn updating_the_theme_alone_leaves_color_mode(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let before = ada.get("/api/auth/me").await.json();
    let res = ada
        .patch("/api/auth/me/preferences", json!({"theme": "forest"}))
        .await;
    res.assert_status(200);
    let mut expected = before;
    expected["theme"] = json!("forest");
    assert_eq!(res.json(), expected);
}

/// parity: test_auth.py::test_update_color_mode_alone_leaves_theme
#[sqlx::test]
async fn updating_color_mode_alone_leaves_the_theme(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch("/api/auth/me/preferences", json!({"color_mode": "dark"}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["theme"], "paper");
    assert_eq!(res.json()["color_mode"], "dark");
}

/// parity: test_auth.py::test_update_both_preferences
#[sqlx::test]
async fn updating_both_preferences(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch(
            "/api/auth/me/preferences",
            json!({"theme": "ink", "color_mode": "light"}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["theme"], "ink");
    assert_eq!(res.json()["color_mode"], "light");
}

/// parity: test_auth.py::test_empty_preferences_payload_is_a_no_op
#[sqlx::test]
async fn an_empty_preferences_payload_is_a_no_op(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.patch("/api/auth/me/preferences", json!({"theme": "slate"}))
        .await
        .assert_status(200);
    let res = ada.patch("/api/auth/me/preferences", json!({})).await;
    res.assert_status(200);
    assert_eq!(res.json()["theme"], "slate");
    // null also means "unchanged".
    let res = ada
        .patch(
            "/api/auth/me/preferences",
            json!({"theme": null, "locale": null}),
        )
        .await;
    assert_eq!(res.json()["theme"], "slate");
}

/// parity: test_auth.py::test_preferences_persist_across_sessions
#[sqlx::test]
async fn preferences_persist_across_sessions(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.patch(
        "/api/auth/me/preferences",
        json!({"theme": "slate", "color_mode": "dark"}),
    )
    .await
    .assert_status(200);
    ada.post("/api/auth/logout", json!(null)).await;
    login(&ada, "ada", PASSWORD).await.assert_status(204);
    let me = ada.get("/api/auth/me").await.json();
    assert_eq!(me["theme"], "slate");
    assert_eq!(me["color_mode"], "dark");
}

/// parity: test_auth.py::test_invalid_theme_rejected
#[sqlx::test]
async fn an_invalid_theme_is_rejected(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch("/api/auth/me/preferences", json!({"theme": "neon"}))
        .await;
    res.assert_detail(400, "theme must be one of: paper, slate, forest, ink");
    assert!(res.detail().contains("paper"));
    assert_eq!(ada.get("/api/auth/me").await.json()["theme"], "paper");
}

/// parity: test_auth.py::test_invalid_color_mode_rejected
#[sqlx::test]
async fn an_invalid_color_mode_is_rejected(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch("/api/auth/me/preferences", json!({"color_mode": "sepia"}))
        .await;
    res.assert_detail(400, "color_mode must be one of: light, dark, system");
    assert_eq!(ada.get("/api/auth/me").await.json()["color_mode"], "system");
}

/// parity: test_auth.py::test_preferences_require_a_session
#[sqlx::test]
async fn preferences_require_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.post("/api/auth/logout", json!(null)).await;
    ada.patch("/api/auth/me/preferences", json!({"theme": "ink"}))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_auth.py::test_update_locale
#[sqlx::test]
async fn updating_the_locale(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch("/api/auth/me/preferences", json!({"locale": "de"}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["locale"], "de");
    assert_eq!(res.json()["theme"], "paper");
    ada.post("/api/auth/logout", json!(null)).await;
    login(&ada, "ada", PASSWORD).await.assert_status(204);
    assert_eq!(ada.get("/api/auth/me").await.json()["locale"], "de");
}

/// parity: test_auth.py::test_invalid_locale_rejected
#[sqlx::test]
async fn an_invalid_locale_is_rejected(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .patch("/api/auth/me/preferences", json!({"locale": "fr"}))
        .await;
    res.assert_detail(400, "locale must be one of: en, de");
    assert_eq!(ada.get("/api/auth/me").await.json()["locale"], "en");
}

#[sqlx::test]
async fn preference_rules(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    // Every message, in the order theme, color_mode, locale, timezone; nothing changes.
    ada.patch(
        "/api/auth/me/preferences",
        json!({"timezone": "Mars/Olympus", "locale": "fr", "color_mode": "sepia", "theme": "neon"}),
    )
    .await
    .assert_detail(
        400,
        "theme must be one of: paper, slate, forest, ink; color_mode must be one of: light, dark, system; \
         locale must be one of: en, de; That is not a valid time zone",
    );
    // Values are stripped and lowercased; zones are validated case-sensitively.
    let res = ada
        .patch(
            "/api/auth/me/preferences",
            json!({"theme": " Forest ", "timezone": " Europe/Berlin "}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["theme"], "forest");
    assert_eq!(res.json()["timezone"], "Europe/Berlin");
    ada.patch(
        "/api/auth/me/preferences",
        json!({"timezone": "europe/berlin"}),
    )
    .await
    .assert_detail(400, "That is not a valid time zone");
    ada.patch("/api/auth/me/preferences", json!({"theme": 3}))
        .await
        .assert_detail(400, "Input should be a valid string");
    // A missing body is 400 (auth spec §1.5); `{}` is fine.
    ada.request(
        axum::http::Method::PATCH,
        "/api/auth/me/preferences",
        Some("application/json"),
        Vec::new(),
    )
    .await
    .assert_detail(400, "Field required");
}

/// An old zone name (a tzdb link such as `Asia/Calcutta`) is accepted and stored as the
/// current one; a wrong-case or made-up name stays refused and changes nothing.
#[sqlx::test]
async fn an_old_zone_name_is_stored_as_the_current_one(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let stored = async || -> String {
        sqlx::query_scalar("SELECT timezone FROM users WHERE username = 'ada'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let res = ada
        .patch(
            "/api/auth/me/preferences",
            json!({"timezone": "Asia/Calcutta"}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["timezone"], "Asia/Kolkata");
    assert_eq!(stored().await, "Asia/Kolkata");
    assert_eq!(
        ada.get("/api/auth/me").await.json()["timezone"],
        "Asia/Kolkata"
    );
    for bad in ["asia/calcutta", "Asia/Atlantis"] {
        ada.patch("/api/auth/me/preferences", json!({"timezone": bad}))
            .await
            .assert_detail(400, "That is not a valid time zone");
    }
    assert_eq!(stored().await, "Asia/Kolkata");
}

/// parity: test_notes_and_meeting.py::test_notification_prefs_default_on_and_patch
#[sqlx::test]
async fn notification_prefs_default_on_and_patch(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    assert_eq!(
        ada.get("/api/auth/notifications").await.json(),
        json!({"notify_meeting": true, "notify_pick": true, "notify_note": true})
    );
    let updated = ada
        .patch("/api/auth/notifications", json!({"notify_note": false}))
        .await;
    updated.assert_status(200);
    assert_eq!(
        updated.json(),
        json!({"notify_meeting": true, "notify_pick": true, "notify_note": false})
    );
    assert_eq!(
        ada.get("/api/auth/notifications").await.json()["notify_note"],
        false
    );
}

/// Reasoned change (endpoint-map §2): strict JSON booleans, where pydantic accepted `"no"`.
#[sqlx::test]
async fn notification_prefs_take_strict_booleans(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for value in [json!("no"), json!(0)] {
        ada.patch("/api/auth/notifications", json!({"notify_pick": value}))
            .await
            .assert_detail(400, "Input should be a valid boolean");
    }
    app.anon()
        .get("/api/auth/notifications")
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .patch("/api/auth/notifications", json!({}))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_notes_and_meeting.py::test_change_password
#[sqlx::test]
async fn change_password(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.patch("/api/auth/password", change("nope-nope", "newpassword1"))
        .await
        .assert_detail(400, "Current password is wrong");
    ada.patch("/api/auth/password", change(PASSWORD, "short"))
        .await
        .assert_detail(400, "New password must be at least 8 characters");
    ada.patch("/api/auth/password", change(PASSWORD, PASSWORD))
        .await
        .assert_detail(400, "Pick a different password");
    let ok = ada
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await;
    ok.assert_status(204);
    assert!(ok.body.is_empty());

    ada.post("/api/auth/logout", json!(null)).await;
    login(&ada, "ada", PASSWORD).await.assert_status(401);
    login(&ada, "ada", "newpassword1").await.assert_status(204);
}

/// parity: test_notes_and_meeting.py::test_change_password_requires_auth
#[sqlx::test]
async fn change_password_requires_auth(db: PgPool) {
    let app = TestApp::new(db).await;
    app.anon()
        .patch("/api/auth/password", change("a", "bbbbbbbbb"))
        .await
        .assert_detail(401, "Not signed in");
}

/// New (endpoint-map §2): a password change deletes every other session and bearer token and
/// voids open reset links and a pending address change; the current session stays. An
/// unverified account keeps the link that verifies its own address.
#[sqlx::test]
async fn change_password_revokes_other_sessions_and_reset_links(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let laptop = app.anon();
    login(&laptop, "ada", PASSWORD).await.assert_status(204);
    let token = app
        .anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
        )
        .await
        .json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let phone = app.anon().with_bearer(&token);
    phone.get("/api/auth/me").await.assert_status(200);
    let other = app.user("grace").await;
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({ada}, 'reset_password', 'ada@example.test', sha256('r'), now() + interval '1 hour'),
                ({ada}, 'verify_email', 'new@example.test', sha256('v'), now() + interval '1 hour'),
                ({grace}, 'reset_password', 'grace@example.test', sha256('g'), now() + interval '1 hour');
         UPDATE users SET password_changed_at = now() - interval '1 day' WHERE id = {ada}",
        ada = ada.id(),
        grace = other.id()
    ))
    .await;

    ada.patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_status(204);
    ada.get("/api/auth/me").await.assert_status(200);
    laptop.get("/api/auth/me").await.assert_status(401);
    phone.get("/api/auth/me").await.assert_status(401);
    other.get("/api/auth/me").await.assert_status(200);
    let app = &app;
    let open = |user: i64, purpose: &'static str| async move {
        app.scalar_i64(&format!(
            "SELECT count(*) FROM email_tokens WHERE user_id = {user} AND purpose = '{purpose}' AND used_at IS NULL"
        ))
        .await
    };
    assert_eq!(open(ada.id(), "reset_password").await, 0);
    assert_eq!(open(ada.id(), "verify_email").await, 0);
    assert_eq!(open(other.id(), "reset_password").await, 1);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM users WHERE id = {} AND password_changed_at > now() - interval '1 minute'",
            ada.id()
        ))
        .await,
        1
    );
}

/// Security review 2026-10-28 #7: a copy of the session cookie (a shared browser profile, an
/// extension) is the same `sessions` row, so the session list never shows it, and a change that
/// kept the current session kept the copy alive. The change now gives the cookie session a new
/// secret with the same label: the copy is signed out, the device that changed stays signed in.
#[sqlx::test]
async fn a_password_change_renews_the_cookie_and_signs_out_copies(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let laptop = app.anon().with_header("user-agent", "Laptop browser");
    login(&laptop, "ada", PASSWORD).await.assert_status(204);
    let old = laptop.cookie().unwrap();
    let copy = app.anon();
    copy.set_cookie(Some(&old));
    copy.get("/api/auth/me").await.assert_status(200);

    let changed = laptop
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await;
    changed.assert_status(204);
    assert_eq!(
        changed.set_cookies().len(),
        1,
        "{:?}",
        changed.set_cookies()
    );
    let new = laptop.cookie().unwrap();
    assert_ne!(new, old);
    laptop.get("/api/auth/me").await.assert_status(200);
    copy.get("/api/auth/me").await.assert_status(401);
    let sessions = laptop.get("/api/auth/sessions").await.json();
    let items = sessions["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{sessions}");
    assert_eq!(items[0]["label"], "Laptop browser");
    assert_eq!(items[0]["current"], true);
}

/// A bearer token keeps its secret through a password change (the app holds it); every other
/// session still goes.
#[sqlx::test]
async fn a_password_change_by_bearer_token_keeps_the_token(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let created = app
        .anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
        )
        .await;
    created.assert_status(201);
    let phone = app
        .anon()
        .with_bearer(created.json()["token"].as_str().unwrap());
    let changed = phone
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await;
    changed.assert_status(204);
    assert!(changed.set_cookies().is_empty());
    phone.get("/api/auth/me").await.assert_status(200);
    ada.get("/api/auth/me").await.assert_status(401);
}

/// An unverified account's open link verifies its own address, not a change someone else
/// started: a password change leaves it open.
#[sqlx::test]
async fn change_password_keeps_the_link_of_an_unverified_address(db: PgPool) {
    let app = TestApp::new(db).await;
    let cleo = app.user_unverified("cleo").await;
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({}, 'verify_email', 'cleo@example.test', sha256('c'), now() + interval '1 hour')",
        cleo.id()
    ))
    .await;
    cleo.patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_status(204);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM email_tokens
             WHERE user_id = {} AND purpose = 'verify_email' AND used_at IS NULL",
            cleo.id()
        ))
        .await,
        1
    );
}

/// Security review 2026-10-28 #3: the current-password check has buckets of its own. Failed
/// logins by name, which anyone can send, never block a signed-in owner's password change, and
/// wrong current passwords no longer block the login.
#[sqlx::test]
async fn failed_logins_and_current_password_checks_do_not_block_each_other(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for _ in 0..10 {
        login(&app.anon(), "ada", "nope-nope")
            .await
            .assert_detail(401, "Wrong username or password");
    }
    login(&app.anon(), "ada", PASSWORD)
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    ada.patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_status(204);

    let grace = app.user("grace").await;
    for _ in 0..10 {
        grace
            .patch("/api/auth/password", change("nope-nope", "newpassword1"))
            .await
            .assert_detail(400, "Current password is wrong");
    }
    grace
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    login(&app.anon(), "grace", PASSWORD)
        .await
        .assert_status(204);
}

/// Security review 2026-10-28 #1: every check counts, right or wrong, and a right password
/// clears nothing, so a session cannot run argon2 without limit by sending its own password
/// again and again. Ten checks per session in ten minutes; another session has its own ten.
#[sqlx::test]
async fn current_password_checks_count_per_session_and_a_right_one_clears_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for _ in 0..5 {
        ada.patch("/api/auth/password", change("nope-nope", "newpassword1"))
            .await
            .assert_detail(400, "Current password is wrong");
    }
    for _ in 0..5 {
        ada.patch("/api/auth/password", change(PASSWORD, PASSWORD))
            .await
            .assert_detail(400, "Pick a different password");
    }
    let refused = ada
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await;
    refused.assert_detail(429, "Too many attempts. Wait a few minutes.");
    assert!(refused.header("retry-after").is_some());
    let laptop = app.anon();
    login(&laptop, "ada", PASSWORD).await.assert_status(204);
    laptop
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_status(204);
}

/// Security review 2026-10-28 #1: 30 checks per account an hour across its sessions, so new
/// sessions do not multiply the argon2 work one account can cause.
#[sqlx::test]
async fn current_password_checks_are_limited_per_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut sessions = vec![ada];
    for _ in 0..3 {
        let device = app.anon();
        login(&device, "ada", PASSWORD).await.assert_status(204);
        sessions.push(device);
    }
    for device in &sessions[..3] {
        for _ in 0..10 {
            device
                .patch("/api/auth/password", change(PASSWORD, PASSWORD))
                .await
                .assert_detail(400, "Pick a different password");
        }
    }
    sessions[3]
        .patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
}

/// Of many concurrent wrong current passwords, at most ten are verified. No password can be
/// verified while the test holds the semaphore, so twenty refusals must arrive before any
/// verdict (see auth_login.rs).
#[sqlx::test]
async fn concurrent_wrong_current_passwords_verify_at_most_ten(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let held = hold_passwords(&app).await;
    let mut pending: FuturesUnordered<_> = (0..30u16)
        .map(|n| {
            let ip: IpAddr = format!("2001:db8:{n:x}::1").parse().unwrap();
            let client = ada.clone().with_ip(ip);
            async move {
                client
                    .patch("/api/auth/password", change("nope-nope", "newpassword1"))
                    .await
            }
        })
        .collect();
    for n in 0..20 {
        let res = timeout(WAIT, pending.next())
            .await
            .unwrap_or_else(|_| panic!("only {n} attempts were refused before verifying"))
            .unwrap();
        res.assert_detail(429, "Too many attempts. Wait a few minutes.");
    }
    drop(held);
    for _ in 0..10 {
        let res = timeout(WAIT, pending.next()).await.unwrap().unwrap();
        res.assert_detail(400, "Current password is wrong");
    }
    assert!(pending.next().await.is_none());
}

/// Refused changes leave every session, bearer token and reset link of the user alone. A right
/// current password counts like a wrong one (§5.4): it clears nothing.
#[sqlx::test]
async fn refused_changes_keep_sessions_and_reset_links(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let laptop = app.anon();
    login(&laptop, "ada", PASSWORD).await.assert_status(204);
    let created = app
        .anon()
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
        )
        .await;
    created.assert_status(201);
    let phone = app
        .anon()
        .with_bearer(created.json()["token"].as_str().unwrap());
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({}, 'reset_password', 'ada@example.test', sha256('r'), now() + interval '1 hour')",
        ada.id()
    ))
    .await;
    let stored = || async {
        sqlx::query_as::<_, (String, bool)>(
            "SELECT password_hash, password_changed_at < now() - interval '1 hour'
             FROM users WHERE id = $1",
        )
        .bind(ada.id())
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    app.exec(&format!(
        "UPDATE users SET password_changed_at = now() - interval '1 day' WHERE id = {}",
        ada.id()
    ))
    .await;
    let before = stored().await;
    assert!(before.1);

    for _ in 0..9 {
        ada.patch("/api/auth/password", change("nope-nope", "newpassword1"))
            .await
            .assert_detail(400, "Current password is wrong");
    }
    ada.patch("/api/auth/password", change(PASSWORD, PASSWORD))
        .await
        .assert_detail(400, "Pick a different password");
    ada.patch("/api/auth/password", change(PASSWORD, "short"))
        .await
        .assert_detail(400, "New password must be at least 8 characters");
    // Nine wrong and one right check: the session's ten are used up. The short password was
    // refused by the body check, before any password was checked.
    ada.patch("/api/auth/password", change(PASSWORD, "newpassword1"))
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");

    for client in [&ada, &laptop, &phone] {
        client.get("/api/auth/me").await.assert_status(200);
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM email_tokens
             WHERE user_id = {} AND purpose = 'reset_password' AND used_at IS NULL",
            ada.id()
        ))
        .await,
        1
    );
    assert_eq!(stored().await, before, "nothing was changed");
}

/// A change verified against a hash that a password reset (or another change) replaced
/// meanwhile does not overwrite it (§5.4).
#[sqlx::test]
async fn a_change_racing_a_reset_does_not_overwrite_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let reset = app.state.passwords.hash("resetpassword1").await.unwrap();
    let key = format!("u:{}", ada.id());
    // 29 earlier checks: the change's own check fills the account's bucket, which shows that it
    // has read the old hash.
    for _ in 0..29 {
        app.state.limits.password_check_user.hit(&key);
    }
    let held = hold_passwords(&app).await;
    let changing = tokio::spawn({
        let ada = ada.clone();
        async move {
            ada.patch("/api/auth/password", change(PASSWORD, "newpassword1"))
                .await
        }
    });
    timeout(WAIT, async {
        while !app.state.limits.password_check_user.is_limited(&key) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the change never reached the password check");
    app.exec(&format!(
        "UPDATE users SET password_hash = '{reset}' WHERE id = {}",
        ada.id()
    ))
    .await;
    drop(held);
    changing
        .await
        .unwrap()
        .assert_detail(400, "Current password is wrong");
    login(&app.anon(), "ada", "resetpassword1")
        .await
        .assert_status(204);
    login(&app.anon(), "ada", "newpassword1")
        .await
        .assert_status(401);
}

const TERMS_CHANGED: &str = "The terms of use have changed. Reload the page and try again.";

/// #53: an account that has not accepted the current terms (an imported one, or one from
/// before a new version) gets `needs_terms` on every `UserOut`, also from other modules' routes,
/// until `POST /api/auth/terms` records the current version.
#[sqlx::test]
async fn needs_terms_flips_after_accepting_the_terms(db: PgPool) {
    let legal = legal_dir("2");
    let app = TestApp::with_env(db, &[("LEGAL_DIR", legal.path().to_str().unwrap())]).await;
    let ada = app.user("ada").await;
    assert_eq!(ada.get("/api/auth/me").await.json()["needs_terms"], true);
    let preferences = ada
        .patch("/api/auth/me/preferences", json!({"locale": "de"}))
        .await;
    assert_eq!(preferences.json()["needs_terms"], true);
    // MED's avatar route builds its UserOut with the same function.
    let avatar = ada.delete("/api/auth/me/avatar").await;
    avatar.assert_status(200);
    assert_eq!(avatar.json()["needs_terms"], true);

    for stale in [json!({"terms_version": "1"}), json!({"terms_version": ""})] {
        ada.post("/api/auth/terms", stale)
            .await
            .assert_detail(400, TERMS_CHANGED);
    }
    ada.post("/api/auth/terms", json!({}))
        .await
        .assert_detail(400, "Field required");
    let accepted = ada
        .post("/api/auth/terms", json!({"terms_version": "2"}))
        .await;
    accepted.assert_status(200);
    assert_eq!(accepted.json()["needs_terms"], false);
    assert_eq!(accepted.json()["username"], "ada");
    assert_eq!(ada.get("/api/auth/me").await.json()["needs_terms"], false);

    // Accepting again keeps the first time.
    app.exec("UPDATE users SET terms_accepted_at = '2026-11-01T00:00:00Z' WHERE username = 'ada'")
        .await;
    ada.post("/api/auth/terms", json!({"terms_version": "2"}))
        .await
        .assert_status(200);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users
             WHERE terms_version = '2' AND terms_accepted_at = '2026-11-01T00:00:00Z'"
        )
        .await,
        1
    );

    // An account that accepted an older version is asked again.
    app.exec("UPDATE users SET terms_version = '1' WHERE username = 'ada'")
        .await;
    assert_eq!(ada.get("/api/auth/me").await.json()["needs_terms"], true);
}

/// #53: without the operator's terms nothing is asked, and there is nothing to accept.
#[sqlx::test]
async fn without_terms_nobody_is_asked(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    assert_eq!(ada.get("/api/auth/me").await.json()["needs_terms"], false);
    ada.post("/api/auth/terms", json!({"terms_version": "placeholder"}))
        .await
        .assert_detail(400, TERMS_CHANGED);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE terms_version IS NOT NULL")
            .await,
        0
    );
}
