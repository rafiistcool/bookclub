//! Password reset by email (endpoint-map §2, docs/phase1/conventions.md §5.5).

mod common;

use std::net::IpAddr;
use std::time::Duration;

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

const LINK_EXPIRED: &str = "That link has expired or was already used";
const NEW_PASSWORD: &str = "a new password";

/// How long a test waits for an answer that must not wait for anything.
const WAIT: Duration = Duration::from_secs(5);

fn ip(n: u16) -> IpAddr {
    format!("2001:db8:{n:x}::1").parse().unwrap()
}

/// `POST /api/auth/password-reset` from `client`, then waits for its background work.
async fn request(app: &TestApp, client: &TestClient, email: &str) -> TestResponse {
    let res = client
        .post("/api/auth/password-reset", json!({"email": email}))
        .await;
    app.settle().await;
    res
}

/// Asks for a reset link for `email` and returns its token.
async fn reset_link(app: &TestApp, email: &str) -> String {
    request(app, &app.anon(), email).await.assert_status(204);
    app.last_link_token(email)
}

async fn confirm(app: &TestApp, token: &str, password: &str) -> TestResponse {
    app.anon()
        .post(
            "/api/auth/password-reset/confirm",
            json!({"token": token, "new_password": password}),
        )
        .await
}

/// Takes every permit of the password semaphore: until the guard drops, no password is hashed
/// or verified, and requests that need one wait.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

async fn login(app: &TestApp, username: &str, password: &str) -> u16 {
    app.anon()
        .with_ip(ip(999))
        .post(
            "/api/auth/login",
            json!({"username": username, "password": password}),
        )
        .await
        .status
        .as_u16()
}

#[sqlx::test]
async fn a_reset_link_sets_the_password_and_signs_everyone_out(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let token = app
        .anon()
        .with_ip(ip(1))
        .post(
            "/api/auth/tokens",
            json!({"username": "ada", "password": PASSWORD, "label": "phone"}),
        )
        .await;
    token.assert_status(201);
    let phone = app
        .anon()
        .with_bearer(token.json()["token"].as_str().unwrap());
    phone.get("/api/auth/me").await.assert_status(200);

    request(&app, &app.anon(), "ADA@example.test")
        .await
        .assert_status(204);
    let mails = app.email.sent_to("ada@example.test");
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "ada@example.test", "the stored spelling");
    assert_eq!(mails[0].subject, "Reset your Bookclub password");
    let link = app.last_link_token("ada@example.test");
    assert!(
        mails[0].text.contains(&format!(
            "http://localhost:5173/reset-password#token={link}"
        )),
        "the token travels in the fragment: {}",
        mails[0].text
    );
    let secret = BASE64_URL_SAFE_NO_PAD.decode(&link).unwrap();
    let row: (Vec<u8>, String, bool) = sqlx::query_as(
        "SELECT token_hash, email,
                expires_at BETWEEN now() + interval '59 minutes' AND now() + interval '61 minutes'
         FROM email_tokens WHERE purpose = 'reset_password'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row.0, Sha256::digest(&secret).to_vec(), "only the hash");
    assert_eq!(row.1, "ada@example.test");
    assert!(row.2, "valid for one hour");

    app.exec("UPDATE users SET password_changed_at = '2026-01-01T00:00:00Z'")
        .await;
    confirm(&app, &link, NEW_PASSWORD).await.assert_status(204);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM sessions").await, 0);
    ada.get("/api/auth/me").await.assert_status(401);
    phone.get("/api/auth/me").await.assert_status(401);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users WHERE password_changed_at > now() - interval '1 minute'"
        )
        .await,
        1
    );
    assert_eq!(login(&app, "ada", PASSWORD).await, 401);
    assert_eq!(login(&app, "ada", NEW_PASSWORD).await, 204);
    confirm(&app, &link, "another password")
        .await
        .assert_detail(400, LINK_EXPIRED);
}

/// The answer never tells whether an account holds the address: unknown and unverified
/// addresses get the same 204 and no mail.
#[sqlx::test]
async fn only_a_verified_address_gets_a_link(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user_unverified("grace").await;
    for email in [
        "grace@example.test",
        "nobody@example.test",
        "not an address",
        "",
    ] {
        let res = request(&app, &app.anon(), email).await;
        res.assert_status(204);
        assert!(res.body.is_empty());
    }
    assert!(app.sent_emails().is_empty());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM email_tokens").await, 0);
    app.anon()
        .post("/api/auth/password-reset", json!({}))
        .await
        .assert_detail(400, "Field required");
}

/// The answer comes before the lookup and the mail (§5.5): while the background work waits for
/// a lock on the account, the request has already answered.
#[sqlx::test]
async fn the_answer_does_not_wait_for_the_lookup_or_the_mail(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    // Issuing the token needs the account row for its foreign key.
    let mut lock = app.db.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(ada.id())
        .execute(&mut *lock)
        .await
        .unwrap();
    timeout(
        WAIT,
        app.anon().post(
            "/api/auth/password-reset",
            json!({"email": "ada@example.test"}),
        ),
    )
    .await
    .expect("the answer waited for the background work")
    .assert_status(204);
    assert!(app.sent_emails().is_empty());
    lock.commit().await.unwrap();
    app.settle().await;
    assert_eq!(app.email.sent_to("ada@example.test").len(), 1);
}

/// Five requests per IP address an hour (429); three links per account an hour, silently.
#[sqlx::test]
async fn requests_are_limited_per_ip_and_silently_per_account(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let client = app.anon().with_ip(ip(1));
    for _ in 0..5 {
        request(&app, &client, "nobody@example.test")
            .await
            .assert_status(204);
    }
    let limited = request(&app, &client, "ada@example.test").await;
    limited.assert_detail(429, "Too many attempts. Wait a few minutes.");
    assert!(limited.header("retry-after").is_some());
    // The same /64 is the same client.
    let neighbour: IpAddr = "2001:db8:1::ffff".parse().unwrap();
    request(&app, &app.anon().with_ip(neighbour), "ada@example.test")
        .await
        .assert_status(429);
    assert!(app.sent_emails().is_empty());

    for n in 2..6 {
        request(&app, &app.anon().with_ip(ip(n)), "ada@example.test")
            .await
            .assert_status(204);
    }
    assert_eq!(app.email.sent_to("ada@example.test").len(), 3);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM email_tokens").await, 3);
}

/// Several links can be open; using one voids the others.
#[sqlx::test]
async fn a_reset_link_dies_when_another_is_used(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let first = reset_link(&app, "ada@example.test").await;
    let second = reset_link(&app, "ada@example.test").await;
    assert_ne!(first, second);
    confirm(&app, &second, NEW_PASSWORD)
        .await
        .assert_status(204);
    confirm(&app, &first, "another password")
        .await
        .assert_detail(400, LINK_EXPIRED);
    assert_eq!(login(&app, "ada", NEW_PASSWORD).await, 204);
}

#[sqlx::test]
async fn a_reset_link_dies_on_a_password_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let link = reset_link(&app, "ada@example.test").await;
    ada.patch(
        "/api/auth/password",
        json!({"current_password": PASSWORD, "new_password": "changed password"}),
    )
    .await
    .assert_status(204);
    confirm(&app, &link, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);
    assert_eq!(login(&app, "ada", "changed password").await, 204);
}

#[sqlx::test]
async fn a_reset_link_dies_on_an_email_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let link = reset_link(&app, "ada@example.test").await;
    ada.put(
        "/api/auth/me/email",
        json!({"email": "new@example.test", "current_password": PASSWORD}),
    )
    .await
    .assert_status(204);
    // The old address is still the verified one: only the voiding kills the link.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE email = 'ada@example.test'")
            .await,
        1
    );
    confirm(&app, &link, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);
    assert_eq!(login(&app, "ada", PASSWORD).await, 204);
}

/// A link is only good while its address is the account's verified address, even when nothing
/// voided it.
#[sqlx::test]
async fn confirm_refuses_a_link_whose_address_is_no_longer_the_verified_one(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let link = reset_link(&app, "ada@example.test").await;
    app.exec("UPDATE users SET email = 'moved@example.test' WHERE username = 'ada'")
        .await;
    confirm(&app, &link, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);
    app.exec(
        "UPDATE users SET email = 'ada@example.test', email_verified_at = NULL
         WHERE username = 'ada'",
    )
    .await;
    confirm(&app, &link, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);
    app.exec("UPDATE users SET email_verified_at = now() WHERE username = 'ada'")
        .await;
    // Refused links stay unused: with the address back, the link works.
    confirm(&app, &link, NEW_PASSWORD).await.assert_status(204);
}

#[sqlx::test]
async fn confirm_checks_the_password_and_the_link(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let link = reset_link(&app, "ada@example.test").await;
    confirm(&app, &link, "short")
        .await
        .assert_detail(400, "New password must be at least 8 characters");
    for token in ["", "nope", &"A".repeat(43)] {
        confirm(&app, token, NEW_PASSWORD)
            .await
            .assert_detail(400, LINK_EXPIRED);
    }
    // A verification link is not a reset link.
    let bob = app.user_unverified("bob").await;
    bob.post("/api/auth/verify-email/resend", json!({}))
        .await
        .assert_status(204);
    app.settle().await;
    let verification = app.last_link_token("bob@example.test");
    confirm(&app, &verification, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);

    let link = reset_link(&app, "ada@example.test").await;
    app.exec(
        "UPDATE email_tokens SET expires_at = now() - interval '1 second'
         WHERE purpose = 'reset_password'",
    )
    .await;
    confirm(&app, &link, NEW_PASSWORD)
        .await
        .assert_detail(400, LINK_EXPIRED);
    assert_eq!(login(&app, "ada", PASSWORD).await, 204);
}

/// The token is checked before the new password is hashed (§5.5): a made-up token is refused
/// while no password can be hashed at all.
#[sqlx::test]
async fn a_made_up_token_costs_no_hashing(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    let held = hold_passwords(&app).await;
    let unknown = BASE64_URL_SAFE_NO_PAD.encode([7u8; 32]);
    timeout(WAIT, confirm(&app, &unknown, NEW_PASSWORD))
        .await
        .expect("the made-up token waited for the password hasher")
        .assert_detail(400, LINK_EXPIRED);
    drop(held);
    assert_eq!(login(&app, "ada", PASSWORD).await, 204);
}

/// The link proves the mailbox: failed guesses by someone else no longer lock the owner out.
#[sqlx::test]
async fn a_reset_clears_the_login_failures(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    for _ in 0..10 {
        assert_eq!(login(&app, "ada", "guess guess").await, 401);
    }
    assert_eq!(login(&app, "ada", PASSWORD).await, 429);
    let link = reset_link(&app, "ada@example.test").await;
    confirm(&app, &link, NEW_PASSWORD).await.assert_status(204);
    assert_eq!(login(&app, "ada", NEW_PASSWORD).await, 204);
}
