//! The account's email address (endpoint-map §2, docs/phase1/conventions.md §5.5): verification
//! links, resend, `PUT /api/auth/me/email` and the "add your email" prompt.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bookclub_server::email::{Email, EmailSender};
use bookclub_server::ratelimit::RateLimits;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

const LINK_EXPIRED: &str = "That link has expired or was already used";
const INVALID_EMAIL: &str = "Enter a valid email address";
const SLOW_DOWN: &str = "Slow down a little. Try again later.";

/// How long a test waits for an answer that must not wait for anything.
const WAIT: Duration = Duration::from_secs(5);

async fn verify(app: &TestApp, token: &str) -> TestResponse {
    app.anon()
        .post("/api/auth/verify-email", json!({"token": token}))
        .await
}

/// `POST /api/auth/verify-email/resend`, then waits for the mail, which goes out after the
/// answer.
async fn resend(app: &TestApp, client: &TestClient) -> TestResponse {
    let res = client
        .post("/api/auth/verify-email/resend", json!({}))
        .await;
    app.settle().await;
    res
}

async fn change_email(client: &TestClient, email: &str) -> TestResponse {
    client
        .put(
            "/api/auth/me/email",
            json!({"email": email, "current_password": PASSWORD}),
        )
        .await
}

async fn me(client: &TestClient) -> Value {
    let res = client.get("/api/auth/me").await;
    res.assert_status(200);
    res.json()
}

/// The open (unused, unexpired) tokens of a user, as `purpose:email`, oldest first.
async fn open_tokens(app: &TestApp, user_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT purpose || ':' || email FROM email_tokens
         WHERE user_id = $1 AND used_at IS NULL AND expires_at > now()
         ORDER BY created_at, id",
    )
    .bind(user_id)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// Takes every permit of the password semaphore: until the guard drops, no password is hashed
/// or verified, and requests that need one wait.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

/// A mail transport that keeps every message waiting until [`GatedEmail::open`], then records
/// its recipient.
struct GatedEmail {
    gate: Semaphore,
    sent: Mutex<Vec<String>>,
}

impl GatedEmail {
    fn closed() -> Self {
        GatedEmail {
            gate: Semaphore::new(0),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn open(&self) {
        self.gate.add_permits(1000);
    }

    fn recipients(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl EmailSender for GatedEmail {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        let _permit = self.gate.acquire().await?;
        self.sent.lock().unwrap().push(msg.to);
        Ok(())
    }
}

/// An account imported from the legacy instance: no address at all.
async fn imported(app: &TestApp, username: &str) -> TestClient {
    let client = app.user_unverified(username).await;
    app.exec(&format!(
        "UPDATE users SET email = NULL WHERE id = {}",
        client.id()
    ))
    .await;
    client
}

#[sqlx::test]
async fn a_link_verifies_the_address_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    resend(&app, &ada).await.assert_status(204);
    let mails = app.email.sent_to("ada@example.test");
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].subject, "Confirm your email address for Bookclub");
    let token = app.last_link_token("ada@example.test");
    assert!(
        mails[0]
            .text
            .contains(&format!("http://localhost:5173/verify-email#token={token}")),
        "the token travels in the fragment: {}",
        mails[0].text
    );
    assert!(!mails[0].text.contains('?'));

    let anon = app.anon();
    anon.post("/api/auth/verify-email", json!({"token": token}))
        .await
        .assert_status(204);
    assert!(anon.cookie().is_none(), "verifying signs nobody in");
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["email_verified"], true);
    assert_eq!(account["pending_email"], Value::Null);
    assert!(open_tokens(&app, ada.id()).await.is_empty());

    verify(&app, &token).await.assert_detail(400, LINK_EXPIRED);
    // The verified address now signs in.
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "ADA@example.test", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
}

#[sqlx::test]
async fn malformed_unknown_expired_and_reset_tokens_are_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    for token in ["", "short", "%%%", &"A".repeat(43)] {
        verify(&app, token).await.assert_detail(400, LINK_EXPIRED);
    }
    app.anon()
        .post("/api/auth/verify-email", json!({}))
        .await
        .assert_detail(400, "Field required");

    resend(&app, &ada).await.assert_status(204);
    let token = app.last_link_token("ada@example.test");
    app.exec("UPDATE email_tokens SET expires_at = now() - interval '1 second'")
        .await;
    verify(&app, &token).await.assert_detail(400, LINK_EXPIRED);

    // A reset link is not a verification link.
    let grace = app.user("grace").await;
    let reset = reset_link(&app, "grace@example.test").await;
    verify(&app, &reset).await.assert_detail(400, LINK_EXPIRED);
    assert_eq!(
        open_tokens(&app, grace.id()).await,
        ["reset_password:grace@example.test"]
    );
    assert_eq!(me(&ada).await["email_verified"], false);
}

/// Every new link voids the previous one, so at most one is open.
#[sqlx::test]
async fn a_new_link_kills_the_old_one(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    resend(&app, &ada).await.assert_status(204);
    let first = app.last_link_token("ada@example.test");
    resend(&app, &ada).await.assert_status(204);
    let second = app.last_link_token("ada@example.test");
    assert_ne!(first, second);
    assert_eq!(
        open_tokens(&app, ada.id()).await,
        ["verify_email:ada@example.test"]
    );
    verify(&app, &first).await.assert_detail(400, LINK_EXPIRED);
    verify(&app, &second).await.assert_status(204);
}

/// Changing the address of an unverified account replaces it at once and kills the old link.
#[sqlx::test]
async fn changing_the_address_kills_the_old_link(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    resend(&app, &ada).await.assert_status(204);
    let old = app.last_link_token("ada@example.test");
    change_email(&ada, " new@example.test ")
        .await
        .assert_status(204);
    let account = me(&ada).await;
    assert_eq!(account["email"], "new@example.test");
    assert_eq!(account["email_verified"], false);
    assert_eq!(account["pending_email"], Value::Null);
    // No "email changed" notice: the old address was never verified.
    assert_eq!(app.email.sent_to("ada@example.test").len(), 1);
    let new = app.last_link_token("new@example.test");
    verify(&app, &old).await.assert_detail(400, LINK_EXPIRED);
    verify(&app, &new).await.assert_status(204);
    let account = me(&ada).await;
    assert_eq!(account["email"], "new@example.test");
    assert_eq!(account["email_verified"], true);
}

/// Verify accepts only the newest link, and only for the address still pending.
#[sqlx::test]
async fn only_the_newest_link_for_the_pending_address_verifies(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    resend(&app, &ada).await.assert_status(204);
    let first = app.last_link_token("ada@example.test");
    resend(&app, &ada).await.assert_status(204);
    let second = app.last_link_token("ada@example.test");
    // Reopen the voided link: it is still not the newest.
    app.exec("UPDATE email_tokens SET used_at = NULL").await;
    verify(&app, &first).await.assert_detail(400, LINK_EXPIRED);

    // The address changed behind the link's back.
    app.exec(&format!(
        "UPDATE users SET email = 'moved@example.test' WHERE id = {}",
        ada.id()
    ))
    .await;
    verify(&app, &second).await.assert_detail(400, LINK_EXPIRED);
    assert_eq!(me(&ada).await["email_verified"], false);
}

/// The newest link is the one issued last. `created_at` is the start of the issuing
/// transaction, so a request that began first but waited for the account lock issues the later
/// link with the earlier time; that link is still the newest, for verify and `pending_email`.
#[sqlx::test]
async fn the_link_issued_last_is_the_newest_whatever_its_time(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    change_email(&ada, "first@example.test")
        .await
        .assert_status(204);
    change_email(&ada, "second@example.test")
        .await
        .assert_status(204);
    // The voided first link carries the later time, and is open again.
    app.exec(
        "UPDATE email_tokens SET created_at = now() + interval '1 minute', used_at = NULL
         WHERE email = 'first@example.test'",
    )
    .await;
    assert_eq!(me(&ada).await["pending_email"], "second@example.test");
    verify(&app, &app.last_link_token("first@example.test"))
        .await
        .assert_detail(400, LINK_EXPIRED);
    verify(&app, &app.last_link_token("second@example.test"))
        .await
        .assert_status(204);
    assert_eq!(me(&ada).await["email"], "second@example.test");
}

/// A verified address stays until the new one is verified; the old address gets a notice.
#[sqlx::test]
async fn a_verified_address_stays_until_the_new_one_is_verified(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["email_verified"], true);
    assert_eq!(account["pending_email"], "new@example.test");

    let notices = app.email.sent_to("ada@example.test");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].subject,
        "Your Bookclub email address is being changed"
    );
    assert!(notices[0].text.contains("new@example.test"));
    assert!(!notices[0].text.contains("#token="));
    let token = app.last_link_token("new@example.test");

    // Until then, the old address signs in and the new one does not.
    let login = |name: &'static str| {
        let client = app.anon();
        async move {
            client
                .post(
                    "/api/auth/login",
                    json!({"username": name, "password": PASSWORD}),
                )
                .await
                .status
                .as_u16()
        }
    };
    assert_eq!(login("ada@example.test").await, 204);
    assert_eq!(login("new@example.test").await, 401);

    verify(&app, &token).await.assert_status(204);
    let account = me(&ada).await;
    assert_eq!(account["email"], "new@example.test");
    assert_eq!(account["email_verified"], true);
    assert_eq!(account["pending_email"], Value::Null);
    assert_eq!(login("new@example.test").await, 204);
    assert_eq!(login("ada@example.test").await, 401);
}

/// Verification voids every open reset link of the account, in the same transaction.
#[sqlx::test]
async fn verification_voids_reset_links(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let link = app.last_link_token("new@example.test");
    // The old address is still the verified one, so it can still ask for a reset link.
    let reset = reset_link(&app, "ada@example.test").await;
    assert_eq!(
        open_tokens(&app, ada.id()).await,
        [
            "verify_email:new@example.test",
            "reset_password:ada@example.test"
        ]
    );
    verify(&app, &link).await.assert_status(204);
    assert!(open_tokens(&app, ada.id()).await.is_empty());
    app.anon()
        .post(
            "/api/auth/password-reset/confirm",
            json!({"token": reset, "new_password": "brand new pw"}),
        )
        .await
        .assert_detail(400, LINK_EXPIRED);
}

/// An unverified address is only what someone typed: another account can verify it, and the
/// first account to verify it keeps it.
#[sqlx::test]
async fn an_unverified_address_does_not_stop_another_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user_unverified("grace").await;
    change_email(&grace, "shared@example.test")
        .await
        .assert_status(204);
    let graces_link = app.last_link_token("shared@example.test");
    let ada = app.user("ada").await;
    change_email(&ada, "Shared@example.test")
        .await
        .assert_status(204);
    let adas_link = app.last_link_token("shared@example.test");
    assert_ne!(graces_link, adas_link);

    verify(&app, &adas_link).await.assert_status(204);
    assert_eq!(me(&ada).await["email"], "Shared@example.test");
    verify(&app, &graces_link)
        .await
        .assert_detail(409, "That email is already in use");
    let account = me(&grace).await;
    assert_eq!(account["email"], "shared@example.test");
    assert_eq!(account["email_verified"], false);
    // The refused link stays open: it was not used.
    assert_eq!(
        open_tokens(&app, grace.id()).await,
        ["verify_email:shared@example.test"]
    );
}

/// No enumeration (§5.5): an address a verified account holds gets the same answer and the same
/// account state as a free one. The holder gets a notice and nobody gets a link.
#[sqlx::test]
async fn a_taken_address_answers_like_a_free_one(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user("grace").await;
    app.exec("UPDATE users SET locale = 'de' WHERE username = 'grace'")
        .await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let taken = change_email(&ada, "Grace@Example.TEST").await;
    let free = change_email(&bob, "free@example.test").await;
    taken.assert_status(204);
    free.assert_status(204);
    assert_eq!(taken.body, free.body);
    assert_eq!(taken.header("content-type"), free.header("content-type"));

    let shape = |mut account: Value| {
        for key in ["id", "username", "email"] {
            account.as_object_mut().unwrap().remove(key);
        }
        account["pending_email"] = json!(account["pending_email"].is_string());
        account
    };
    assert_eq!(shape(me(&ada).await), shape(me(&bob).await));
    assert_eq!(me(&ada).await["pending_email"], "Grace@Example.TEST");

    let to_grace = app.email.sent_to("grace@example.test");
    assert_eq!(to_grace.len(), 1);
    assert!(!to_grace[0].text.contains("#token="));
    assert!(
        to_grace[0]
            .text
            .contains("sie einem anderen Konto bei Bookclub hinzuzufügen"),
        "{}",
        to_grace[0].text
    );
    assert_eq!(app.email.sent_to("free@example.test").len(), 1);
    // Both old addresses get the same "email changed" notice.
    assert_eq!(app.email.sent_to("ada@example.test").len(), 1);
    assert_eq!(app.email.sent_to("bob@example.test").len(), 1);
    assert_eq!(me(&grace).await["email"], "grace@example.test");

    // An unverified account that names the address keeps it, unverified; no link either.
    let cleo = app.user_unverified("cleo").await;
    change_email(&cleo, "grace@example.test")
        .await
        .assert_status(204);
    assert_eq!(me(&cleo).await["email"], "grace@example.test");
    let to_grace = app.email.sent_to("grace@example.test");
    assert_eq!(to_grace.len(), 2);
    assert!(to_grace.iter().all(|mail| !mail.text.contains("#token=")));
}

/// Wrong passwords count against the session's current-password checks, not the login
/// (security review 2026-10-28 #3).
#[sqlx::test]
async fn a_wrong_password_changes_nothing_and_counts_as_a_check(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let wrong = json!({"email": "new@example.test", "current_password": "wrong password"});
    for _ in 0..10 {
        ada.put("/api/auth/me/email", wrong.clone())
            .await
            .assert_detail(400, "Current password is wrong");
    }
    let blocked = change_email(&ada, "new@example.test").await;
    blocked.assert_detail(429, "Too many attempts. Wait a few minutes.");
    assert!(blocked.header("retry-after").is_some());
    // Not the login's bucket.
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    assert_eq!(me(&ada).await["pending_email"], Value::Null);
    assert!(open_tokens(&app, ada.id()).await.is_empty());
    assert!(app.sent_emails().is_empty());
}

/// Security review 2026-10-28 #1: the per-account limit of 3 changes an hour is checked before
/// the password, so a refused change costs no argon2 work (the password semaphore is held, and
/// the answer still comes), and it does not use up a current-password check.
#[sqlx::test]
async fn the_change_limit_is_checked_before_the_password(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for n in 0..3 {
        change_email(&ada, &format!("new{n}@example.test"))
            .await
            .assert_status(204);
    }
    let held = hold_passwords(&app).await;
    for _ in 0..10 {
        timeout(WAIT, change_email(&ada, "late@example.test"))
            .await
            .expect("the refusal waited for the password semaphore")
            .assert_detail(429, "Slow down a little. Try again later.");
    }
    drop(held);
    ada.patch(
        "/api/auth/password",
        json!({"current_password": PASSWORD, "new_password": "newpassword1"}),
    )
    .await
    .assert_status(204);
}

#[sqlx::test]
async fn invalid_addresses_are_refused_before_the_password(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for email in [
        "",
        "nope",
        "ada@localhost",
        "x<victim@example.test>",
        "\"a\"@b.de",
    ] {
        ada.put(
            "/api/auth/me/email",
            json!({"email": email, "current_password": "wrong password"}),
        )
        .await
        .assert_detail(400, INVALID_EMAIL);
    }
    // None of them counted as a login failure.
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    app.anon()
        .put(
            "/api/auth/me/email",
            json!({"email": "new@example.test", "current_password": PASSWORD}),
        )
        .await
        .assert_detail(401, "Not signed in");
}

/// Resend and `PUT /me/email` share three per account and hour.
#[sqlx::test]
async fn email_change_and_resend_share_a_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user_unverified("ada").await;
    resend(&app, &ada).await.assert_status(204);
    change_email(&ada, "two@example.test")
        .await
        .assert_status(204);
    resend(&app, &ada).await.assert_status(204);
    let limited = resend(&app, &ada).await;
    limited.assert_detail(429, SLOW_DOWN);
    assert!(limited.header("retry-after").is_some());
    change_email(&ada, "three@example.test")
        .await
        .assert_detail(429, SLOW_DOWN);
    assert_eq!(me(&ada).await["email"], "two@example.test");
    assert_eq!(app.sent_emails().len(), 3);
    // Another account has its own bucket.
    let bob = app.user_unverified("bob").await;
    resend(&app, &bob).await.assert_status(204);
}

/// Mail to one address counts against 6 a day across accounts; over it nothing is sent and the
/// answer does not change.
/// Security review 2026-10-28 #8: the verify page asks before it verifies. The preview names
/// the account and the address, changes nothing and can be read again; the link still works
/// afterwards, and a used, unknown or replaced link has no preview.
#[sqlx::test]
async fn a_preview_names_the_account_and_changes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let token = app.last_link_token("new@example.test");
    let preview = |token: String| {
        let anon = app.anon();
        async move {
            anon.post("/api/auth/verify-email/preview", json!({"token": token}))
                .await
        }
    };
    for _ in 0..2 {
        let res = preview(token.clone()).await;
        res.assert_status(200);
        assert_eq!(
            res.json(),
            json!({"username": "ada", "email": "new@example.test"})
        );
    }
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["pending_email"], "new@example.test");
    verify(&app, &token).await.assert_status(204);
    preview(token).await.assert_detail(400, LINK_EXPIRED);
    preview("nope".into())
        .await
        .assert_detail(400, LINK_EXPIRED);

    let cleo = app.user_unverified("cleo").await;
    for address in ["cleo1@example.test", "cleo2@example.test"] {
        change_email(&cleo, address).await.assert_status(204);
    }
    preview(app.last_link_token("cleo1@example.test"))
        .await
        .assert_detail(400, LINK_EXPIRED);
    preview(app.last_link_token("cleo2@example.test"))
        .await
        .assert_status(200);
}

/// Security review 2026-10-28 #14: links and each kind of notice have per-address buckets of
/// their own. A flood of links to an address (reset requests, which anyone can send) cannot
/// silence the notice that its account's address is being changed.
#[sqlx::test]
async fn link_floods_do_not_silence_the_security_notices(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for _ in 0..6 {
        app.state.limits.email_target.hit("ada@example.test");
    }
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let to_old = app.email.sent_to("ada@example.test");
    assert_eq!(to_old.len(), 1);
    assert!(
        to_old[0].text.contains("new@example.test"),
        "{}",
        to_old[0].text
    );
    assert_eq!(app.email.sent_to("new@example.test").len(), 1);
}

#[sqlx::test]
async fn mail_to_one_address_is_silently_limited(db: PgPool) {
    let app = TestApp::new(db).await;
    for n in 0..7 {
        let user = app.user(&format!("user{n}")).await;
        let address = if n % 2 == 0 {
            "shared@example.test"
        } else {
            "SHARED@example.test"
        };
        change_email(&user, address).await.assert_status(204);
        assert_eq!(me(&user).await["pending_email"], address);
    }
    assert_eq!(app.email.sent_to("shared@example.test").len(), 6);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM email_tokens WHERE purpose = 'verify_email'")
            .await,
        7
    );
}

/// Imported accounts have no address: they add one here, or dismiss the prompt.
#[sqlx::test]
async fn an_imported_user_adds_an_address(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = imported(&app, "ada").await;
    let account = me(&ada).await;
    assert_eq!(account["email"], Value::Null);
    assert_eq!(account["needs_email_prompt"], true);
    resend(&app, &ada)
        .await
        .assert_detail(400, "Add an email address first");

    change_email(&ada, "ada@example.test")
        .await
        .assert_status(204);
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["needs_email_prompt"], false);
    verify(&app, &app.last_link_token("ada@example.test"))
        .await
        .assert_status(204);
    assert_eq!(me(&ada).await["email_verified"], true);
}

#[sqlx::test]
async fn dismissing_the_prompt_keeps_the_first_time(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = imported(&app, "ada").await;
    ada.post("/api/auth/me/email-prompt/dismiss", json!({}))
        .await
        .assert_status(204);
    assert_eq!(me(&ada).await["needs_email_prompt"], false);
    app.exec(
        "UPDATE users SET email_prompt_dismissed_at = '2026-01-01T00:00:00Z' WHERE username = 'ada'",
    )
    .await;
    ada.post("/api/auth/me/email-prompt/dismiss", json!({}))
        .await
        .assert_status(204);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users
             WHERE email_prompt_dismissed_at = '2026-01-01T00:00:00Z'"
        )
        .await,
        1
    );
    app.anon()
        .post("/api/auth/me/email-prompt/dismiss", json!({}))
        .await
        .assert_detail(401, "Not signed in");
}

/// Changing back to the account's own verified address cancels the pending change.
#[sqlx::test]
async fn the_own_verified_address_cancels_a_pending_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let link = app.last_link_token("new@example.test");
    let sent = app.sent_emails().len();
    change_email(&ada, "ADA@example.test")
        .await
        .assert_status(204);
    assert_eq!(app.sent_emails().len(), sent, "nothing is sent");
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["pending_email"], Value::Null);
    verify(&app, &link).await.assert_detail(400, LINK_EXPIRED);
}

#[sqlx::test]
async fn resend_on_a_verified_account_renews_only_a_pending_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    // Nothing to verify: nothing is sent and nothing counts against the limit.
    for _ in 0..4 {
        resend(&app, &ada).await.assert_status(204);
    }
    assert!(app.sent_emails().is_empty());

    change_email(&ada, "new@example.test")
        .await
        .assert_status(204);
    let first = app.last_link_token("new@example.test");
    resend(&app, &ada).await.assert_status(204);
    let second = app.last_link_token("new@example.test");
    assert_ne!(first, second);
    assert_eq!(app.email.sent_to("new@example.test").len(), 2);
    verify(&app, &first).await.assert_detail(400, LINK_EXPIRED);
    verify(&app, &second).await.assert_status(204);
    assert_eq!(me(&ada).await["email"], "new@example.test");
}

/// Resend never sends a link to an address a verified account holds, and the answer is the
/// same.
#[sqlx::test]
async fn resend_to_a_taken_address_sends_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("grace").await;
    let ada = app.user_unverified("ada").await;
    app.exec(&format!(
        "UPDATE users SET email = 'grace@example.test' WHERE id = {}",
        ada.id()
    ))
    .await;
    resend(&app, &ada).await.assert_status(204);
    assert!(app.sent_emails().is_empty());
}

/// Resend answers before its mail goes out (§5.5): only a free address gets a link, so the time
/// a send takes would tell whether a verified account holds the address. A free and a held
/// address both answer while the transport holds every mail.
#[sqlx::test]
async fn resend_answers_before_the_mail_goes_out(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("grace").await;
    app.user_unverified("ada").await;
    let eve = app.user_unverified("eve").await;
    app.exec(&format!(
        "UPDATE users SET email = 'grace@example.test' WHERE id = {}",
        eve.id()
    ))
    .await;
    // The same app with a transport that holds every mail until the test opens it, and its
    // own limits, whose reference count shows when the background mail is done.
    let email = Arc::new(GatedEmail::closed());
    let limits = Arc::new(RateLimits::new());
    let mut state = app.state.clone();
    state.email = email.clone();
    state.limits = limits.clone();
    let router = bookclub_server::app(state);
    let idle = Arc::strong_count(&limits);

    for name in ["ada", "eve"] {
        let client = TestClient::for_router(router.clone());
        client
            .post(
                "/api/auth/login",
                json!({"username": name, "password": PASSWORD}),
            )
            .await
            .assert_status(204);
        timeout(
            WAIT,
            client.post("/api/auth/verify-email/resend", json!({})),
        )
        .await
        .unwrap_or_else(|_| panic!("the answer to {name} waited for the mail"))
        .assert_status(204);
    }
    assert!(email.recipients().is_empty());
    email.open();
    timeout(WAIT, async {
        while Arc::strong_count(&limits) > idle {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the mail was never sent");
    assert_eq!(email.recipients(), ["ada@example.test"]);
}

/// An email change verified against a hash that a password reset replaced meanwhile changes
/// nothing (§5.4): the reset wins.
#[sqlx::test]
async fn an_email_change_racing_a_reset_changes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec(&format!(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at)
         VALUES ({}, 'reset_password', 'ada@example.test', sha256('r'), now() + interval '1 hour')",
        ada.id()
    ))
    .await;
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
        async move { change_email(&ada, "new@example.test").await }
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
    let account = me(&ada).await;
    assert_eq!(account["email"], "ada@example.test");
    assert_eq!(account["pending_email"], Value::Null);
    assert_eq!(
        open_tokens(&app, ada.id()).await,
        ["reset_password:ada@example.test"],
        "no new link, and the open reset link stays open"
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM email_tokens").await, 1);
    assert!(app.sent_emails().is_empty());
}

/// Asks for a reset link for `email` and returns its token (the mail goes out in the
/// background, after the answer).
async fn reset_link(app: &TestApp, email: &str) -> String {
    app.anon()
        .post("/api/auth/password-reset", json!({"email": email}))
        .await
        .assert_status(204);
    app.settle().await;
    app.last_link_token(email)
}
