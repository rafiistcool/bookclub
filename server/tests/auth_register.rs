//! `POST /api/auth/register` (endpoint-map §2, docs/phase1/conventions.md §5.5).

mod common;

use std::net::IpAddr;

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const USERNAME_RULE: &str =
    "Username must be 2\u{2013}32 characters: lowercase letters, numbers, underscore";

/// A sign-up invite (club_id NULL) with `max_uses` uses, or a 7-day link without a limit.
async fn signup_invite(app: &TestApp, code: &str, max_uses: Option<i32>) {
    let max_uses = max_uses.map_or("NULL".to_owned(), |n| n.to_string());
    app.exec(&format!(
        "INSERT INTO club_invites (club_id, code, max_uses, expires_at)
         VALUES (NULL, '{code}', {max_uses}, now() + interval '7 days')"
    ))
    .await;
}

fn body(username: &str, invite: &str) -> Value {
    json!({
        "username": username,
        "email": format!("{username}@example.test"),
        "password": PASSWORD,
        "invite_code": invite,
    })
}

fn ip(n: u16) -> IpAddr {
    format!("2001:db8:{n:x}::1").parse().unwrap()
}

async fn login(client: &TestClient, username: &str) -> TestResponse {
    client
        .post(
            "/api/auth/login",
            json!({"username": username, "password": PASSWORD}),
        )
        .await
}

/// parity: test_auth.py::test_register_consumes_invite_and_sets_session
///
/// Reasoned change: `UserOut` gained `email`, `email_verified`, `pending_email`,
/// `needs_email_prompt`, `timezone`, `is_admin` and `clubs`; the body needs an email.
#[sqlx::test]
async fn register_consumes_the_invite_and_signs_in(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", Some(1)).await;
    let client = app.anon();
    let res = client
        .post("/api/auth/register", body("ada", "testinvite"))
        .await;
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    assert_eq!(
        res.json(),
        json!({
            "id": id,
            "username": "ada",
            "email": "ada@example.test",
            "email_verified": false,
            "pending_email": null,
            "needs_email_prompt": false,
            "needs_terms": false,
            "theme": "paper",
            "color_mode": "system",
            "locale": "en",
            "timezone": "UTC",
            "avatar_url": null,
            "is_admin": false,
            "clubs": [],
        })
    );
    assert!(client.cookie().is_some());
    let me = client.get("/api/auth/me").await;
    me.assert_status(200);
    assert_eq!(me.json()["username"], "ada");
    assert_eq!(
        app.scalar_i64("SELECT use_count::bigint FROM club_invites WHERE code = 'TESTINVITE'")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_invite_uses WHERE user_id = {id}"
        ))
        .await,
        1
    );
}

/// parity: test_auth.py::test_invalid_invite_does_not_create_user
#[sqlx::test]
async fn an_invalid_invite_creates_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let client = app.anon();
    client
        .post("/api/auth/register", body("ada", "NOPE"))
        .await
        .assert_detail(400, "That invite code is not valid");
    assert!(client.cookie().is_none());
    for table in ["users", "sessions", "email_tokens", "club_invite_uses"] {
        assert_eq!(
            app.scalar_i64(&format!("SELECT count(*) FROM {table}"))
                .await,
            0,
            "{table}"
        );
    }
    assert!(app.sent_emails().is_empty());
    login(&client, "ada").await.assert_status(401);
}

/// parity: test_auth.py::test_invite_cannot_be_reused
#[sqlx::test]
async fn a_single_use_invite_cannot_be_reused(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", Some(1)).await;
    let client = app.anon();
    client
        .post("/api/auth/register", body("ada", "TESTINVITE"))
        .await
        .assert_status(201);
    client
        .post("/api/auth/logout", json!(null))
        .await
        .assert_status(204);
    let again = client
        .post("/api/auth/register", body("grace", "TESTINVITE"))
        .await;
    again.assert_detail(400, "That invite code is not valid");
    assert!(again.detail().to_lowercase().contains("invite"));
    assert!(client.cookie().is_none());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE username = 'grace'")
            .await,
        0
    );
    assert_eq!(
        app.scalar_i64("SELECT use_count::bigint FROM club_invites WHERE code = 'TESTINVITE'")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        1
    );
}

/// parity: test_auth.py::test_invite_cannot_be_used_twice_concurrently
#[sqlx::test]
async fn a_single_use_invite_admits_one_of_two_concurrent_sign_ups(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", Some(1)).await;
    let (ada, grace) = (app.anon(), app.anon());
    let (a, g) = tokio::join!(
        ada.post("/api/auth/register", body("ada", "TESTINVITE")),
        grace.post("/api/auth/register", body("grace", "TESTINVITE")),
    );
    let mut statuses = vec![a.status.as_u16(), g.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, vec![201, 400]);
    let loser = if a.status == 400 { &a } else { &g };
    loser.assert_detail(400, "That invite code is not valid");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 1);
    assert_eq!(
        app.scalar_i64("SELECT use_count::bigint FROM club_invites WHERE code = 'TESTINVITE'")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        1
    );
    let ada_ok = login(&app.anon(), "ada").await.status == 204;
    let grace_ok = login(&app.anon(), "grace").await.status == 204;
    assert!(ada_ok ^ grace_ok);
}

/// parity: test_auth.py::test_username_taken
///
/// The taken check runs before the invite check, so a bad invite still gives 409.
#[sqlx::test]
async fn a_taken_username_is_409_before_the_invite_is_checked(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "FIRST", Some(1)).await;
    signup_invite(&app, "SECOND", Some(1)).await;
    let client = app.anon();
    client
        .post("/api/auth/register", body("ada", "FIRST"))
        .await
        .assert_status(201);
    client.post("/api/auth/logout", json!(null)).await;
    for invite in ["SECOND", "NOPE"] {
        client
            .post("/api/auth/register", body("Ada", invite))
            .await
            .assert_detail(409, "That username is already taken");
    }
    assert_eq!(
        app.scalar_i64("SELECT use_count::bigint FROM club_invites WHERE code = 'SECOND'")
            .await,
        0
    );
}

/// Two concurrent sign-ups of one name with two valid invites: the unique index decides, and
/// the loser gets 409, not 500.
#[sqlx::test]
async fn concurrent_sign_ups_of_one_name_give_one_409(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "FIRST", Some(1)).await;
    signup_invite(&app, "SECOND", Some(1)).await;
    let (one, two) = (app.anon(), app.anon());
    let (a, b) = tokio::join!(
        one.post("/api/auth/register", body("ada", "FIRST")),
        two.post("/api/auth/register", body("ada", "SECOND")),
    );
    let mut statuses = vec![a.status.as_u16(), b.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, vec![201, 409]);
    let loser = if a.status == 409 { &a } else { &b };
    assert_eq!(loser.detail(), "That username is already taken");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 1);
    assert_eq!(
        app.scalar_i64("SELECT sum(use_count)::bigint FROM club_invites")
            .await,
        1
    );
}

/// parity: test_auth.py::test_bad_username_and_short_password
#[sqlx::test]
async fn bad_usernames_and_short_passwords_are_400(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", Some(1)).await;
    let client = app.anon();
    let mut spaced = body("ada", "TESTINVITE");
    spaced["username"] = json!("Ada Lovelace");
    client
        .post("/api/auth/register", spaced)
        .await
        .assert_detail(400, USERNAME_RULE);
    let mut short = body("ada", "TESTINVITE");
    short["password"] = json!("short");
    client
        .post("/api/auth/register", short)
        .await
        .assert_detail(400, "Password must be at least 8 characters");
    // Every message, in field order.
    let all = json!({
        "username": "a",
        "email": "nope",
        "password": "pässwö",
        "invite_code": "TESTINVITE",
        "timezone": "Mars/Olympus",
        "locale": "fr",
    });
    client.post("/api/auth/register", all).await.assert_detail(
        400,
        &format!(
            "{USERNAME_RULE}; Enter a valid email address; Password must be at least 8 characters; \
             That is not a valid time zone; locale must be one of: en, de"
        ),
    );
    // Passwords count characters, not bytes, and are not trimmed.
    let mut umlauts = body("ada", "TESTINVITE");
    umlauts["password"] = json!("pässwörd");
    client
        .post("/api/auth/register", umlauts)
        .await
        .assert_status(201);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 1);
}

#[sqlx::test]
async fn fields_are_normalised(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "ABCDE12345", Some(1)).await;
    let res = app
        .anon()
        .post(
            "/api/auth/register",
            json!({
                "username": "  Ada_1 ",
                "email": " Ada@Example.test ",
                "password": " password1 ",
                "invite_code": " abcde-12345 ",
                "timezone": " Europe/Berlin ",
                "locale": " DE ",
            }),
        )
        .await;
    res.assert_status(201);
    let user = res.json();
    assert_eq!(user["username"], "ada_1");
    assert_eq!(user["email"], "Ada@Example.test");
    assert_eq!(user["timezone"], "Europe/Berlin");
    assert_eq!(user["locale"], "de");
    // The password keeps its spaces.
    login(&app.anon(), "ada_1").await.assert_status(401);
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "ada_1", "password": " password1 "}),
        )
        .await
        .assert_status(204);
}

/// An old zone name (a tzdb link that ICU browsers still report) signs up like the current
/// one, which is what is stored. Case still matters, and a made-up name stays refused.
#[sqlx::test]
async fn an_old_zone_name_is_stored_as_the_current_one(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", None).await;
    for bad in ["asia/calcutta", "Asia/Atlantis"] {
        let mut wrong = body("ada", "TESTINVITE");
        wrong["timezone"] = json!(bad);
        app.anon()
            .post("/api/auth/register", wrong)
            .await
            .assert_detail(400, "That is not a valid time zone");
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
    let mut old = body("ada", "TESTINVITE");
    old["timezone"] = json!(" Asia/Calcutta ");
    let res = app.anon().post("/api/auth/register", old).await;
    res.assert_status(201);
    assert_eq!(res.json()["timezone"], "Asia/Kolkata");
    let stored: String = sqlx::query_scalar("SELECT timezone FROM users WHERE username = 'ada'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, "Asia/Kolkata");
}

#[sqlx::test]
async fn invite_mode_needs_a_code(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut no_code = body("ada", "");
    no_code.as_object_mut().unwrap().remove("invite_code");
    app.anon()
        .post("/api/auth/register", no_code)
        .await
        .assert_detail(400, "Sign-up needs an invite code right now");
    for blank in ["", " - ", "\t"] {
        app.anon()
            .post("/api/auth/register", body("ada", blank))
            .await
            .assert_detail(400, "Sign-up needs an invite code right now");
    }
    let mut null_code = body("ada", "");
    null_code["invite_code"] = Value::Null;
    app.anon()
        .post("/api/auth/register", null_code)
        .await
        .assert_detail(400, "Sign-up needs an invite code right now");
}

#[sqlx::test]
async fn open_mode_needs_no_code_but_checks_a_given_one(db: PgPool) {
    let app = TestApp::with_env(db, &[("SIGNUP_MODE", "open")]).await;
    let mut no_code = body("ada", "");
    no_code.as_object_mut().unwrap().remove("invite_code");
    app.anon()
        .post("/api/auth/register", no_code)
        .await
        .assert_status(201);
    app.anon()
        .post("/api/auth/register", body("grace", "NOPE"))
        .await
        .assert_detail(400, "That invite code is not valid");
}

#[sqlx::test]
async fn expired_and_revoked_invites_are_invalid(db: PgPool) {
    let app = TestApp::new(db).await;
    app.exec(
        "INSERT INTO club_invites (club_id, code, max_uses, expires_at, revoked_at)
         VALUES (NULL, 'EXPIRED', NULL, now() - interval '1 second', NULL),
                (NULL, 'REVOKED', 5, NULL, now())",
    )
    .await;
    for code in ["EXPIRED", "REVOKED"] {
        app.anon()
            .post("/api/auth/register", body("ada", code))
            .await
            .assert_detail(400, "That invite code is not valid");
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
}

#[sqlx::test]
async fn sign_up_sends_a_verification_link_in_the_fragment(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", None).await;
    let res = app
        .anon()
        .post("/api/auth/register", body("ada", "TESTINVITE"))
        .await;
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    let mails = app.sent_emails();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "ada@example.test");
    assert_eq!(mails[0].subject, "Confirm your email address for Bookclub");
    assert!(
        mails[0]
            .text
            .contains("http://localhost:5173/verify-email#token="),
        "{}",
        mails[0].text
    );
    let token = app.last_link_token("ada@example.test");
    let secret = BASE64_URL_SAFE_NO_PAD.decode(&token).unwrap();
    assert_eq!(secret.len(), 32);
    let row: (Vec<u8>, String, String, bool) = sqlx::query_as(
        "SELECT token_hash, purpose, email,
                expires_at BETWEEN now() + interval '47 hours' AND now() + interval '49 hours'
         FROM email_tokens WHERE user_id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        row.0,
        Sha256::digest(&secret).to_vec(),
        "only the hash is stored"
    );
    assert_eq!(row.1, "verify_email");
    assert_eq!(row.2, "ada@example.test");
    assert!(row.3, "valid for 48 hours");

    // A German account gets German mail.
    let mut german = body("grace", "TESTINVITE");
    german["locale"] = json!("de");
    app.anon()
        .post("/api/auth/register", german)
        .await
        .assert_status(201);
    let mail = app.email.sent_to("grace@example.test").pop().unwrap();
    assert!(mail.subject.starts_with("Bestätige"), "{}", mail.subject);
}

/// No enumeration (§5.5): an address a verified account holds gets the same answer as a free
/// one; the holder gets a notice, nobody gets a link, and the new account keeps an unverified
/// address.
#[sqlx::test]
async fn an_address_of_a_verified_account_gets_a_notice_not_a_link(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", None).await;
    let _grace = app.user("grace").await;
    app.exec("UPDATE users SET locale = 'de' WHERE username = 'grace'")
        .await;

    let mut taken = body("ada", "TESTINVITE");
    taken["email"] = json!("Grace@Example.TEST");
    let res = app.anon().post("/api/auth/register", taken).await;
    let free = app
        .anon()
        .post("/api/auth/register", body("bob", "TESTINVITE"))
        .await;
    res.assert_status(201);
    free.assert_status(201);
    let shape = |value: &Value| {
        let mut value = value.clone();
        for key in ["id", "username", "email"] {
            value.as_object_mut().unwrap().remove(key);
        }
        value
    };
    assert_eq!(shape(&res.json()), shape(&free.json()));
    assert_eq!(res.json()["email"], "Grace@Example.TEST");
    assert_eq!(res.json()["email_verified"], false);

    let ada = res.json()["id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM email_tokens WHERE user_id = {ada}"
        ))
        .await,
        0
    );
    let to_grace = app.email.sent_to("grace@example.test");
    assert_eq!(to_grace.len(), 1);
    assert!(!to_grace[0].text.contains("#token="));
    assert!(to_grace[0].text.starts_with("Hallo grace"));
    assert!(to_grace[0].subject.starts_with("Jemand wollte"));
    assert_eq!(app.email.sent_to("bob@example.test").len(), 1);
}

/// An unverified address is just what someone typed: it blocks nobody.
#[sqlx::test]
async fn an_unverified_address_does_not_block_a_sign_up(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", None).await;
    app.user_unverified("grace").await;
    let mut same = body("ada", "TESTINVITE");
    same["email"] = json!("grace@example.test");
    app.anon()
        .post("/api/auth/register", same)
        .await
        .assert_status(201);
    let mails = app.email.sent_to("grace@example.test");
    assert_eq!(mails.len(), 1);
    assert!(mails[0].text.contains("/verify-email#token="));
}

/// Five sign-ups per IP address per hour; IPv6 addresses count by their /64.
#[sqlx::test]
async fn sign_ups_are_limited_per_ip(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "LINK", None).await;
    let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
    let same_64: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
    for n in 0..5 {
        app.anon()
            .with_ip(first)
            .post("/api/auth/register", body(&format!("user{n}"), "LINK"))
            .await
            .assert_status(201);
    }
    let refused = app
        .anon()
        .with_ip(same_64)
        .post("/api/auth/register", body("user5", "LINK"))
        .await;
    refused.assert_detail(429, "Too many attempts. Wait a few minutes.");
    let wait: u64 = refused.header("retry-after").unwrap().parse().unwrap();
    assert!((3590..=3600).contains(&wait), "{wait}");
    app.anon()
        .with_ip(ip(3))
        .post("/api/auth/register", body("user5", "LINK"))
        .await
        .assert_status(201);
}

/// The order of §5.5: malformed bodies are refused before the limit and cost nothing; every
/// validated attempt counts, also one refused for a taken name, a missing or an invalid
/// invite code, so invite codes and names cannot be probed for free.
#[sqlx::test]
async fn every_validated_sign_up_attempt_counts_against_the_ip_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "LINK", None).await;
    app.user("taken").await;
    let here = app.anon().with_ip(ip(7));
    for _ in 0..6 {
        let mut malformed = body("ada", "LINK");
        malformed["email"] = json!("nope");
        here.post("/api/auth/register", malformed)
            .await
            .assert_detail(400, "Enter a valid email address");
    }
    here.post("/api/auth/register", json!({"username": "ada"}))
        .await
        .assert_detail(400, "Field required");
    for code in ["NOPE", "WRONG", "NOTHING"] {
        here.post("/api/auth/register", body("ada", code))
            .await
            .assert_detail(400, "That invite code is not valid");
    }
    here.post("/api/auth/register", body("taken", "LINK"))
        .await
        .assert_detail(409, "That username is already taken");
    here.post("/api/auth/register", body("ada", ""))
        .await
        .assert_detail(400, "Sign-up needs an invite code right now");
    let refused = here.post("/api/auth/register", body("ada", "LINK")).await;
    refused.assert_detail(429, "Too many attempts. Wait a few minutes.");
    assert!(refused.header("retry-after").is_some());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE username = 'ada'")
            .await,
        0
    );
    assert!(app.email.sent_to("ada@example.test").is_empty());
}

/// Only a plain address is accepted (§5.5). Spellings that a mail library delivers to a
/// verified account's mailbox are refused, so they can neither get a verification link to it
/// past the "address in use" notice nor bypass the per-address mail limit.
#[sqlx::test]
async fn mailbox_spellings_of_an_address_are_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", None).await;
    app.user("grace").await;
    for spelling in [
        "<grace@example.test>",
        "x<grace@example.test>",
        "Grace <grace@example.test>",
        "\"grace\"@example.test",
        "(1)grace@example.test",
        "grace(1)@example.test",
        "a,grace@example.test",
    ] {
        let mut attempt = body("ada", "TESTINVITE");
        attempt["email"] = json!(spelling);
        app.anon()
            .post("/api/auth/register", attempt)
            .await
            .assert_detail(400, "Enter a valid email address");
    }
    let mut nul = body("ada", "TESTINVITE");
    nul["email"] = json!("gr\u{0}ace@example.test");
    app.anon()
        .post("/api/auth/register", nul)
        .await
        .assert_detail(400, "Invalid value");
    app.anon()
        .post("/api/auth/register", body("ada", "TEST\u{0}INVITE"))
        .await
        .assert_detail(400, "Invalid value");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 1);
    assert!(app.sent_emails().is_empty());

    // The plain address itself gets the notice, as before.
    let mut plain = body("ada", "TESTINVITE");
    plain["email"] = json!("grace@example.test");
    app.anon()
        .post("/api/auth/register", plain)
        .await
        .assert_status(201);
    let to_grace = app.email.sent_to("grace@example.test");
    assert_eq!(to_grace.len(), 1);
    assert!(!to_grace[0].text.contains("#token="));
}

/// Every mail to one address counts against 6 a day; over it the mail is silently dropped and
/// the answer does not change.
#[sqlx::test]
async fn mails_to_one_address_are_silently_limited(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "LINK", None).await;
    for n in 0..7u16 {
        let mut same = body(&format!("user{n}"), "LINK");
        same["email"] = json!(if n % 2 == 0 {
            "shared@example.test"
        } else {
            "Shared@Example.test"
        });
        app.anon()
            .with_ip(ip(n))
            .post("/api/auth/register", same)
            .await
            .assert_status(201);
    }
    assert_eq!(app.email.sent_to("shared@example.test").len(), 6);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM email_tokens").await, 7);
}

/// Registering while signed in switches the session, as in the legacy app; the old session
/// row is deleted.
#[sqlx::test]
async fn registering_while_signed_in_switches_the_session(db: PgPool) {
    let app = TestApp::new(db).await;
    signup_invite(&app, "TESTINVITE", Some(1)).await;
    let grace = app.user("grace").await;
    let old = grace.cookie().unwrap();
    grace
        .post("/api/auth/register", body("ada", "TESTINVITE"))
        .await
        .assert_status(201);
    assert_ne!(grace.cookie().unwrap(), old);
    assert_eq!(grace.get("/api/auth/me").await.json()["username"], "ada");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM sessions WHERE user_id = {}",
            grace.id()
        ))
        .await,
        0
    );
}

/// A club invite creates the account and the membership in one transaction: the new member is
/// `invited_by` the invite's creator, the use is logged, and the club records `member_joined`.
#[sqlx::test]
async fn a_club_invite_signs_up_and_joins_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let invite = owner
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "single_use"}),
        )
        .await;
    invite.assert_status(201);
    let code = invite.json()["code"].as_str().unwrap().to_owned();

    let res = app
        .anon()
        .post("/api/auth/register", body("ada", &code.to_lowercase()))
        .await;
    res.assert_status(201);
    assert_eq!(
        res.json()["clubs"],
        json!([{"slug": club.slug, "name": "Readers", "role": "member", "timezone": "UTC", "member_count": 2}])
    );
    let ada = res.json()["id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships
             WHERE club_id = {} AND user_id = {ada} AND role = 'member' AND invited_by = {}",
            club.id,
            owner.id()
        ))
        .await,
        1
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events
             WHERE club_id = {} AND actor_id = {ada} AND kind = 'member_joined' AND payload = '{{}}'",
            club.id
        ))
        .await,
        1
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_invite_uses u JOIN club_invites i ON i.id = u.invite_id
             WHERE i.code = '{code}' AND i.use_count = 1 AND u.user_id = {ada}"
        ))
        .await,
        1
    );
}

/// A full club refuses a sign-up with its invite: 409, and the account, the session and the
/// invite use are rolled back.
#[sqlx::test]
async fn a_full_club_rolls_the_sign_up_back(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    app.exec(&format!(
        "WITH extra AS (
             INSERT INTO users (username, password_hash)
             SELECT 'member' || n, 'x' FROM generate_series(1, 19) AS n
             RETURNING id
         )
         INSERT INTO club_memberships (club_id, user_id, role) SELECT {}, id, 'member' FROM extra;
         INSERT INTO club_invites (club_id, code, created_by, expires_at)
         VALUES ({}, 'CLUBLINK', {}, now() + interval '7 days')",
        club.id,
        club.id,
        owner.id()
    ))
    .await;
    app.anon()
        .post("/api/auth/register", body("ada", "CLUBLINK"))
        .await
        .assert_detail(409, "This club is full");
    for (sql, expected) in [
        ("SELECT count(*) FROM users WHERE username = 'ada'", 0),
        ("SELECT count(*) FROM sessions", 1),
        ("SELECT count(*) FROM email_tokens", 0),
        (
            "SELECT use_count::bigint FROM club_invites WHERE code = 'CLUBLINK'",
            0,
        ),
        ("SELECT count(*) FROM club_invite_uses", 0),
        ("SELECT count(*) FROM events", 0),
    ] {
        assert_eq!(app.scalar_i64(sql).await, expected, "{sql}");
    }
}

const TERMS_CHANGED: &str = "The terms of use have changed. Reload the page and try again.";

/// #53: whenever the operator's terms are configured, a sign-up must send their current
/// version; a missing or old one is 400 and creates nothing. The accepted version is stored
/// with the time of the sign-up.
#[sqlx::test]
async fn a_sign_up_accepts_the_current_terms(db: PgPool) {
    let legal = legal_dir("2026-11-01");
    let app = TestApp::with_env(
        db,
        &[
            ("SIGNUP_MODE", "open"),
            ("LEGAL_DIR", legal.path().to_str().unwrap()),
        ],
    )
    .await;
    let mut sign_up = body("ada", "");
    sign_up.as_object_mut().unwrap().remove("invite_code");
    for terms in [
        None,
        Some(json!("2026-10-01")),
        Some(json!("")),
        Some(Value::Null),
    ] {
        let mut attempt = sign_up.clone();
        if let Some(terms) = terms {
            attempt["terms_version"] = terms;
        }
        app.anon()
            .with_ip(ip(1))
            .post("/api/auth/register", attempt)
            .await
            .assert_detail(400, TERMS_CHANGED);
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
    assert!(app.sent_emails().is_empty());

    sign_up["terms_version"] = json!("2026-11-01");
    let res = app
        .anon()
        .with_ip(ip(2))
        .post("/api/auth/register", sign_up)
        .await;
    res.assert_status(201);
    assert_eq!(res.json()["needs_terms"], false);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users
             WHERE username = 'ada' AND terms_version = '2026-11-01'
               AND terms_accepted_at > now() - interval '1 minute'"
        )
        .await,
        1
    );
}

/// #53: without the operator's terms (here the DEV_MODE placeholders) there is nothing to
/// accept: a given version is ignored and nothing is stored.
#[sqlx::test]
async fn without_terms_a_sign_up_stores_no_acceptance(db: PgPool) {
    let app = TestApp::with_env(db, &[("SIGNUP_MODE", "open")]).await;
    let mut sign_up = body("ada", "");
    sign_up.as_object_mut().unwrap().remove("invite_code");
    sign_up["terms_version"] = json!("placeholder");
    let res = app.anon().post("/api/auth/register", sign_up).await;
    res.assert_status(201);
    assert_eq!(res.json()["needs_terms"], false);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users WHERE terms_version IS NULL AND terms_accepted_at IS NULL"
        )
        .await,
        1
    );
}
