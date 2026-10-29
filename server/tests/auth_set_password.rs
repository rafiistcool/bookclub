//! Mail off (MOVE, the private move #52; docs/phase3/work-plan.md §5.2):
//! `bookclub-server set-password <username>`, the password reset without mail, does in one
//! transaction what the reset confirm does, with a generated password
//! (`auth::reset::set_password`); and `EMAIL_TRANSPORT=none` sends nothing while sign-up and the
//! reset request carry on.

mod common;

use std::net::IpAddr;
use std::time::Duration;

use bookclub_server::auth::mail;
use bookclub_server::auth::reset::set_password;
use common::*;
use serde_json::json;
use sqlx::PgPool;

const LINK_EXPIRED: &str = "That link has expired or was already used";

fn ip(n: u16) -> IpAddr {
    format!("2001:db8:{n:x}::1").parse().unwrap()
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

/// `set_password` for `username` with the test hasher.
async fn set(app: &TestApp, username: &str) -> Option<String> {
    let mut conn = app.db.acquire().await.unwrap();
    set_password(&mut conn, &app.state.passwords, username)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_new_password_signs_in_and_the_old_one_does_not(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    app.user("bob").await;
    app.exec("UPDATE users SET password_changed_at = '2026-01-01T00:00:00Z'")
        .await;

    // Names are normalised like every username lookup.
    let password = set(&app, "  Ada ").await.expect("ada exists");
    assert_eq!(password.len(), 20);
    assert!(
        password
            .bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte)),
        "{password}"
    );
    assert_eq!(login(&app, "ada", PASSWORD).await, 401);
    assert_eq!(login(&app, "ada", &password).await, 204);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users
             WHERE username = 'ada' AND password_changed_at > now() - interval '1 minute'"
        )
        .await,
        1
    );

    // Another account is untouched, and every run makes a new password.
    assert_eq!(login(&app, "bob", PASSWORD).await, 204);
    let again = set(&app, "ada").await.unwrap();
    assert_ne!(again, password);
    assert_eq!(login(&app, "ada", &password).await, 401);
    assert_eq!(login(&app, "ada", &again).await, 204);
}

#[sqlx::test]
async fn the_accounts_earlier_sessions_end(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
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
    ada.get("/api/auth/me").await.assert_status(200);
    phone.get("/api/auth/me").await.assert_status(200);

    set(&app, "ada").await.unwrap();
    ada.get("/api/auth/me").await.assert_status(401);
    phone.get("/api/auth/me").await.assert_status(401);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE user_id = {}",
            ada.id()
        ))
        .await,
        0
    );
    bob.get("/api/auth/me").await.assert_status(200);
}

/// The open reset link and a pending address change are voided, as by the reset confirm.
#[sqlx::test]
async fn an_open_reset_link_is_voided(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.anon()
        .post(
            "/api/auth/password-reset",
            json!({"email": "ada@example.test"}),
        )
        .await
        .assert_status(204);
    app.settle().await;
    let link = app.last_link_token("ada@example.test");
    {
        let mut conn = app.db.acquire().await.unwrap();
        mail::issue_verify_token(&mut conn, ada.id(), "ada@new.example.test")
            .await
            .unwrap();
    }
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM email_tokens WHERE used_at IS NULL")
            .await,
        2
    );

    set(&app, "ada").await.unwrap();
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM email_tokens WHERE used_at IS NULL")
            .await,
        0
    );
    app.anon()
        .post(
            "/api/auth/password-reset/confirm",
            json!({"token": link, "new_password": "a new password"}),
        )
        .await
        .assert_detail(400, LINK_EXPIRED);
}

#[sqlx::test]
async fn an_unknown_name_gives_none(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    assert_eq!(set(&app, "nobody").await, None);
    assert_eq!(set(&app, "").await, None);
    assert_eq!(login(&app, "ada", PASSWORD).await, 204);
}

/// A suspended account gets the password and stays suspended.
#[sqlx::test]
async fn a_suspended_account_stays_suspended(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
    app.exec("UPDATE users SET suspended_at = now() WHERE username = 'ada'")
        .await;
    let password = set(&app, "ada").await.unwrap();
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users WHERE username = 'ada' AND suspended_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(login(&app, "ada", &password).await, 403);
}

/// A production configuration with mail off and no SMTP account: sign-up with an invite
/// answers 201 although no mail leaves, and a reset request's link is voided at once, so the
/// next request is not held back by a link nobody received.
#[sqlx::test]
async fn with_mail_off_sign_up_works_and_reset_links_are_voided(db: PgPool) {
    let env: Vec<(&str, &str)> = PRODUCTION_ENV
        .iter()
        .filter(|(key, _)| !matches!(*key, "SMTP_URL" | "EMAIL_FROM"))
        .copied()
        .chain([("EMAIL_TRANSPORT", "none")])
        .collect();
    let state = test_state_with(db.clone(), test_config(&env));
    let client = TestClient::for_router(bookclub_server::app(state));
    sqlx::query(
        "INSERT INTO club_invites (club_id, code, max_uses, expires_at)
         VALUES (NULL, 'MAILOFF', 1, now() + interval '7 days')",
    )
    .execute(&db)
    .await
    .unwrap();
    client
        .post(
            "/api/auth/register",
            json!({
                "username": "ada",
                "email": "ada@example.test",
                "password": PASSWORD,
                "invite_code": "MAILOFF",
            }),
        )
        .await
        .assert_status(201);

    sqlx::query("UPDATE users SET email_verified_at = now() WHERE username = 'ada'")
        .execute(&db)
        .await
        .unwrap();
    client
        .post(
            "/api/auth/password-reset",
            json!({"email": "ada@example.test"}),
        )
        .await
        .assert_status(204);
    // The request answers before its background work: wait for the link to be issued and
    // voided.
    let voided = "SELECT count(*) FROM email_tokens
                  WHERE purpose = 'reset_password' AND used_at IS NOT NULL";
    let mut waited = Duration::ZERO;
    while sqlx::query_scalar::<_, i64>(voided)
        .fetch_one(&db)
        .await
        .unwrap()
        == 0
    {
        assert!(waited < Duration::from_secs(5), "no voided reset link");
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += Duration::from_millis(20);
    }
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_tokens
         WHERE purpose = 'reset_password' AND used_at IS NULL",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(open, 0);
}
