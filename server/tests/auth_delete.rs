//! `DELETE /api/auth/me` (endpoint-map §2, docs/phase1/conventions.md §5.6).

mod common;

use std::net::IpAddr;
use std::time::Duration;

use bookclub_server::media::shared::{MediaStore, NewMedia, register, urls};
use bytes::Bytes;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

const TRANSFER_CLUBS_FIRST: &str = "Transfer or delete your clubs first";

/// How long a test waits for a step that must not wait for the other transaction.
const WAIT: Duration = Duration::from_secs(5);

fn ip(n: u16) -> IpAddr {
    format!("2001:db8:{n:x}::1").parse().unwrap()
}

async fn delete_me(client: &TestClient, password: &str) -> TestResponse {
    client
        .delete_json("/api/auth/me", json!({"password": password}))
        .await
}

/// Stores the bytes of an uploaded picture and registers its media row (`uploaded_by` =
/// `uploader`). Returns the row id and its storage prefix.
async fn upload(app: &TestApp, uploader: Option<i64>, kind: &str, seed: u8) -> (i64, String) {
    let public_id = urls::new_public_id();
    let variants: &[&str] = if kind == "avatar" {
        &["a"]
    } else {
        &["s", "l"]
    };
    for variant in variants {
        app.media
            .put(
                &urls::object_key(kind, &public_id, variant),
                Bytes::from(vec![seed; 16]),
                "image/webp",
            )
            .await
            .unwrap();
    }
    let mut conn = app.db.acquire().await.unwrap();
    let id = register(
        &mut conn,
        &public_id,
        &NewMedia {
            kind,
            source: "upload",
            content_type: "image/webp",
            variants,
            width: 256,
            height: 256,
            byte_size: 16,
            sha256: [seed; 32],
            dominant_color: None,
            uploaded_by: uploader,
            created_at: None,
        },
    )
    .await
    .unwrap();
    (id, urls::storage_prefix(kind, &public_id))
}

fn stored_under(app: &TestApp, prefix: &str) -> usize {
    app.media
        .keys()
        .iter()
        .filter(|key| key.starts_with(&format!("{prefix}/")))
        .count()
}

/// A pick of `club` on `work_id`, set by `set_by`.
async fn pick(app: &TestApp, club: &TestClub, work_id: i64, set_by: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(club.id)
    .bind(work_id)
    .bind(set_by)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A post (POSTS arrives in Wave 2, so rows are inserted directly): a diary entry when `club`
/// is None, else a post in the pick's discussion. `parent` makes it a reply.
async fn post(
    app: &TestApp,
    work_id: i64,
    author: i64,
    club: Option<(i64, i64)>,
    parent: Option<i64>,
    body: &str,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO posts (work_id, author_id, club_id, pick_id, parent_id, body, spoiler_upto)
         VALUES ($1, $2, $3, $4, $5, $6, 40) RETURNING id",
    )
    .bind(work_id)
    .bind(author)
    .bind(club.map(|(club_id, _)| club_id))
    .bind(club.map(|(_, pick_id)| pick_id))
    .bind(parent)
    .bind(body)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    app.scalar_i64(sql).await
}

/// Takes every permit of the password semaphore: until the guard drops, no password is hashed
/// or verified, and requests that need one wait.
async fn hold_passwords(app: &TestApp) -> OwnedSemaphorePermit {
    let permits = app.state.passwords.permits.clone();
    let all = u32::try_from(permits.available_permits()).unwrap();
    permits.acquire_many_owned(all).await.unwrap()
}

/// Waits until a statement of this test's database waits for a lock held by another
/// transaction.
async fn wait_for_a_lock(app: &TestApp, who: &str) {
    for attempt in 0.. {
        let waiting = count(
            app,
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .await;
        if waiting > 0 {
            return;
        }
        assert!(attempt < 500, "{who} never waited for the lock");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Makes a new picture `user`'s avatar (`uploaded_by` = `uploader`). Returns the row id and its
/// storage prefix.
async fn avatar_of(app: &TestApp, user: &TestClient, uploader: Option<i64>) -> (i64, String) {
    let (avatar, prefix) = upload(app, uploader, "avatar", 5).await;
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = {avatar} WHERE id = {}",
        user.id()
    ))
    .await;
    (avatar, prefix)
}

#[sqlx::test]
async fn deletion_removes_personal_data_and_every_avatar(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let id = ada.id();
    let work = app.work("Circe").await;

    // A second browser, an app token and an open verification link.
    app.anon()
        .with_ip(ip(1))
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    let token = app
        .anon()
        .with_ip(ip(1))
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
    for (client, address) in [(&ada, "new@example.test"), (&bob, "bob2@example.test")] {
        client
            .put(
                "/api/auth/me/email",
                json!({"email": address, "current_password": PASSWORD}),
            )
            .await
            .assert_status(204);
    }
    for owner in [id, bob.id()] {
        app.exec(&format!(
            "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({owner}, {w}, 'want_to_read');
             INSERT INTO quotes (user_id, work_id, body) VALUES ({owner}, {w}, 'A line');
             INSERT INTO user_favorites (user_id, work_id, position) VALUES ({owner}, {w}, 1);
             INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ({owner}, '{{\"rows\": []}}');
             INSERT INTO search_history (user_id, query, normalized_query) VALUES ({owner}, 'Circe', 'circe');
             INSERT INTO import_jobs (user_id, status) VALUES ({owner}, 'done');",
            w = work.id
        ))
        .await;
    }

    // Ada replaced her avatar twice and left all three rows behind; she also photographed a
    // cover. Bob has an avatar of his own.
    let mut avatars = Vec::new();
    for seed in 1..=3 {
        avatars.push(upload(&app, Some(id), "avatar", seed).await);
    }
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = {} WHERE id = {id}",
        avatars[2].0
    ))
    .await;
    let (cover, cover_prefix) = upload(&app, Some(id), "cover", 9).await;
    app.exec(&format!(
        "UPDATE works SET cover_media_id = {cover} WHERE id = {}",
        work.id
    ))
    .await;
    let (bobs_avatar, bobs_prefix) = upload(&app, Some(bob.id()), "avatar", 7).await;
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = {bobs_avatar} WHERE id = {}",
        bob.id()
    ))
    .await;

    delete_me(&ada, "wrong password")
        .await
        .assert_detail(400, "Current password is wrong");
    assert_eq!(
        count(&app, &format!("SELECT count(*) FROM users WHERE id = {id}")).await,
        1
    );
    let old_cookie = ada.cookie();

    let res = delete_me(&ada, PASSWORD).await;
    res.assert_status(204);
    let cookies = res.set_cookies();
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert!(
        cookies[0].starts_with("bookclub_session=;"),
        "{}",
        cookies[0]
    );
    assert!(cookies[0].contains("Max-Age=0"));
    assert!(ada.cookie().is_none());

    assert_eq!(
        count(&app, &format!("SELECT count(*) FROM users WHERE id = {id}")).await,
        0
    );
    for table in [
        "sessions",
        "email_tokens",
        "shelf_entries",
        "quotes",
        "user_favorites",
        "physical_shelf_layouts",
        "search_history",
        "import_jobs",
    ] {
        assert_eq!(
            count(
                &app,
                &format!("SELECT count(*) FROM {table} WHERE user_id = {id}")
            )
            .await,
            0,
            "{table}"
        );
        assert_eq!(
            count(
                &app,
                &format!("SELECT count(*) FROM {table} WHERE user_id = {}", bob.id())
            )
            .await,
            1,
            "{table} of another user"
        );
    }
    let avatar_ids: Vec<String> = avatars.iter().map(|(id, _)| id.to_string()).collect();
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM media WHERE id IN ({})",
                avatar_ids.join(", ")
            )
        )
        .await,
        0
    );
    for (_, prefix) in &avatars {
        assert_eq!(stored_under(&app, prefix), 0, "{prefix}");
    }
    // The cover photo is a catalog contribution: it stays, without its uploader.
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM media WHERE id = {cover} AND uploaded_by IS NULL")
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM works WHERE cover_media_id = {cover}")
        )
        .await,
        1
    );
    assert_eq!(stored_under(&app, &cover_prefix), 2);
    assert_eq!(stored_under(&app, &bobs_prefix), 1);
    bob.get("/api/auth/me").await.assert_status(200);

    // Every session is gone, and the name no longer signs in.
    ada.set_cookie(old_cookie.as_deref());
    ada.get("/api/auth/me").await.assert_status(401);
    phone.get("/api/auth/me").await.assert_status(401);
    app.anon()
        .with_ip(ip(2))
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(401);
}

/// §5.6: the user's posts that others replied to become anonymous tombstones (without the
/// reading state they were written at); every other post is deleted, with its reactions. Other
/// users' replies stay visible to the audience of their authors.
#[sqlx::test]
async fn posts_with_replies_by_others_become_tombstones(db: PgPool) {
    let app = TestApp::new(db).await;
    let bob = app.user("bob").await;
    let ada = app.user("ada").await;
    let cleo = app.user("cleo").await;
    let (a, b) = (ada.id(), bob.id());
    let readers = app.club(&bob, "Readers").await;
    app.join(&readers, &ada, "member").await;
    app.join(&readers, &cleo, "member").await;
    let work = app.work("Circe").await;
    let pick_id = pick(&app, &readers, work.id, b).await;
    let club = Some((readers.id, pick_id));
    let w = work.id;

    let answered = post(&app, w, a, club, None, "ada asks").await;
    let bobs_reply = post(&app, w, b, club, Some(answered), "bob answers").await;
    let alone = post(&app, w, a, club, None, "nobody answers").await;
    let self_thread = post(&app, w, a, club, None, "ada talks").await;
    let self_reply = post(&app, w, a, club, Some(self_thread), "to herself").await;
    let bobs_post = post(&app, w, b, club, None, "bob posts").await;
    let adas_reply = post(&app, w, a, club, Some(bobs_post), "ada replies").await;
    let diary = post(&app, w, a, None, None, "ada's diary").await;
    let diary_reply = post(&app, w, b, None, Some(diary), "bob's diary reply").await;
    let private = post(&app, w, a, None, None, "only for ada").await;
    app.exec(&format!(
        "UPDATE posts SET visibility = 'only_me' WHERE id = {private};
         UPDATE posts SET progress_at = 42, status_at = 'currently_reading'
             WHERE id IN ({answered}, {diary});
         INSERT INTO post_reactions (post_id, user_id, emoji) VALUES
             ({alone}, {b}, '🔥'), ({bobs_post}, {a}, '📚'), ({answered}, {b}, '👍');"
    ))
    .await;

    delete_me(&ada, PASSWORD).await.assert_status(204);

    let survivors: Vec<i64> = sqlx::query_scalar("SELECT id FROM posts ORDER BY id")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(
        survivors,
        [answered, bobs_reply, bobs_post, diary, diary_reply],
        "deleted: {alone} {self_thread} {self_reply} {adas_reply} {private}"
    );
    for tombstone in [answered, diary] {
        let row: (Option<i64>, String, Option<i32>, bool, bool) = sqlx::query_as(
            "SELECT author_id, body, spoiler_upto, deleted_at IS NOT NULL,
                    progress_at IS NULL AND status_at IS NULL
             FROM posts WHERE id = $1",
        )
        .bind(tombstone)
        .fetch_one(&app.db)
        .await
        .unwrap();
        assert_eq!(
            row,
            (None, String::new(), None, true, true),
            "post {tombstone}"
        );
    }
    for reply in [bobs_reply, diary_reply] {
        assert_eq!(
            count(
                &app,
                &format!(
                    "SELECT count(*) FROM posts
                     WHERE id = {reply} AND author_id = {b} AND body <> '' AND deleted_at IS NULL"
                )
            )
            .await,
            1
        );
    }
    // Reactions on deleted posts and by Ada are gone; Bob's on the tombstone stays.
    assert_eq!(count(&app, "SELECT count(*) FROM post_reactions").await, 1);
    // The diary tombstone stays visible to whoever sees a reply's author, so Bob's reply keeps
    // its anchor for the same audience; Cleo shares a club with Bob.
    for (viewer, sees) in [(b, true), (cleo.id(), true)] {
        let visible: bool = sqlx::query_scalar("SELECT bc_can_see_post($1, $2)")
            .bind(viewer)
            .bind(diary)
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(visible, sees, "viewer {viewer}");
    }
    let stranger = app.user("stranger").await;
    let visible: bool = sqlx::query_scalar("SELECT bc_can_see_post($1, $2)")
        .bind(stranger.id())
        .bind(diary)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!visible);
}

/// Other users' `reaction` events keep only the author's id, so after the deletion they name
/// nobody, also once someone registers the freed username. The user's own events are deleted.
#[sqlx::test]
async fn reaction_events_of_others_show_no_author_afterwards(db: PgPool) {
    let app = TestApp::new(db).await;
    let bob = app.user("bob").await;
    let ada = app.user("ada").await;
    let (a, b) = (ada.id(), bob.id());
    let readers = app.club(&bob, "Readers").await;
    app.join(&readers, &ada, "member").await;
    let work = app.work("Circe").await;
    let pick_id = pick(&app, &readers, work.id, b).await;
    let adas_post = post(&app, work.id, a, Some((readers.id, pick_id)), None, "hi").await;
    app.exec(&format!(
        "INSERT INTO events (club_id, actor_id, kind, pick_id, work_id, payload) VALUES
             ({c}, {b}, 'reaction', {pick_id}, {w},
              jsonb_build_object('emoji', '🔥', 'post_id', {adas_post}, 'author_id', {a})),
             ({c}, {a}, 'member_joined', NULL, NULL, '{{}}'),
             ({c}, {a}, 'note_posted', {pick_id}, {w}, jsonb_build_object('post_id', {adas_post}));
         INSERT INTO events (actor_id, kind, work_id) VALUES ({a}, 'shelf_added', {w});",
        c = readers.id,
        w = work.id
    ))
    .await;
    let author_of_reaction = "SELECT count(*) FROM events e
         LEFT JOIN users u ON u.id = (e.payload->>'author_id')::bigint
         WHERE e.kind = 'reaction' AND u.username IS NULL";

    delete_me(&ada, PASSWORD).await.assert_status(204);

    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM events WHERE actor_id = {a}")
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM events
                 WHERE actor_id = {b} AND kind = 'reaction' AND (payload->>'author_id')::bigint = {a}"
            )
        )
        .await,
        1
    );
    assert_eq!(count(&app, author_of_reaction).await, 1);
    let new_ada = app.user("ada").await;
    assert_ne!(new_ada.id(), a, "identity ids are never reused");
    assert_eq!(count(&app, author_of_reaction).await, 1);
}

/// Owning a club with other members blocks the deletion; a club the user owns alone goes with
/// the account.
#[sqlx::test]
async fn owned_clubs_must_be_handed_over_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let readers = app.club(&ada, "Readers").await;
    app.join(&readers, &bob, "admin").await;
    let solo = app.club(&ada, "Solo").await;
    let work = app.work("Circe").await;
    let solo_pick = pick(&app, &solo, work.id, ada.id()).await;
    post(
        &app,
        work.id,
        ada.id(),
        Some((solo.id, solo_pick)),
        None,
        "notes",
    )
    .await;

    delete_me(&ada, PASSWORD)
        .await
        .assert_detail(409, TRANSFER_CLUBS_FIRST);
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(count(&app, "SELECT count(*) FROM clubs").await, 2);
    assert_eq!(count(&app, "SELECT count(*) FROM posts").await, 1);

    // Ada hands Readers over; then both her solo club and her account go.
    app.exec(&format!(
        "BEGIN;
         UPDATE club_memberships SET role = 'member' WHERE club_id = {c} AND user_id = {a};
         UPDATE club_memberships SET role = 'owner' WHERE club_id = {c} AND user_id = {b};
         COMMIT;",
        c = readers.id,
        a = ada.id(),
        b = bob.id()
    ))
    .await;
    delete_me(&ada, PASSWORD).await.assert_status(204);
    let clubs: Vec<i64> = sqlx::query_scalar("SELECT id FROM clubs")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(clubs, [readers.id]);
    assert_eq!(count(&app, "SELECT count(*) FROM club_picks").await, 0);
    assert_eq!(count(&app, "SELECT count(*) FROM posts").await, 0);
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM club_memberships WHERE club_id = {}",
                readers.id
            )
        )
        .await,
        1
    );
}

/// The club records and catalog rows the user made outlive the account, without the name.
#[sqlx::test]
async fn shared_records_outlive_the_account_without_the_name(db: PgPool) {
    let app = TestApp::new(db).await;
    let bob = app.user("bob").await;
    let ada = app.user("ada").await;
    let cleo = app.user("cleo").await;
    let (a, b) = (ada.id(), bob.id());
    let readers = app.club(&bob, "Readers").await;
    app.join(&readers, &ada, "admin").await;
    app.join(&readers, &cleo, "member").await;
    let other = app.club(&bob, "Other").await;
    let work = app.work("Circe").await;
    let edition = app.edition(&work, "9780316556347").await;
    let pick_id = pick(&app, &readers, work.id, a).await;
    let c = readers.id;
    app.exec(&format!(
        "UPDATE clubs SET created_by = {a} WHERE id = {c};
         UPDATE club_memberships SET invited_by = {a} WHERE user_id = {cleo};
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({c}, 'ADAS', {a}, 5);
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({o}, 'BOBS', {b}, 5);
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {a} FROM club_invites WHERE code = 'BOBS';
         INSERT INTO club_removals (club_id, user_id) VALUES ({o}, {a});
         INSERT INTO milestones (club_id, pick_id, title, created_by) VALUES ({c}, {pick_id}, 'Part one', {a});
         INSERT INTO next_up_votes (club_id) VALUES ({c});
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {c}, id, {w}, {a} FROM next_up_votes;
         INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
             SELECT {c}, vote_id, {a}, id FROM next_up_nominations;
         UPDATE works SET created_by = {a} WHERE id = {w};
         UPDATE editions SET created_by = {a} WHERE id = {e};
         INSERT INTO catalog_edit_requests (kind, work_id, created_by, decided_by)
             VALUES ('work_edit', {w}, {a}, {b}), ('work_edit', {w}, {b}, {a});
         INSERT INTO reports (target_kind, work_id, reason, created_by, resolved_by)
             VALUES ('cover', {w}, 'other', {a}, {b}), ('cover', {w}, 'other', {b}, {a});",
        cleo = cleo.id(),
        o = other.id,
        w = work.id,
        e = edition.id
    ))
    .await;

    delete_me(&ada, PASSWORD).await.assert_status(204);

    for (sql, expected) in [
        ("SELECT count(*) FROM clubs WHERE created_by IS NULL", 1),
        (
            "SELECT count(*) FROM club_memberships WHERE invited_by IS NULL",
            3,
        ),
        (
            "SELECT count(*) FROM club_invites WHERE code = 'ADAS' AND created_by IS NULL",
            1,
        ),
        (
            "SELECT count(*) FROM club_invite_uses WHERE user_id IS NULL",
            1,
        ),
        ("SELECT count(*) FROM club_removals", 0),
        ("SELECT count(*) FROM club_picks WHERE set_by IS NULL", 1),
        (
            "SELECT count(*) FROM milestones WHERE created_by IS NULL",
            1,
        ),
        (
            "SELECT count(*) FROM next_up_nominations WHERE nominated_by IS NULL",
            1,
        ),
        ("SELECT count(*) FROM next_up_ballots", 0),
        ("SELECT count(*) FROM works WHERE created_by IS NULL", 1),
        ("SELECT count(*) FROM editions WHERE created_by IS NULL", 1),
        (
            "SELECT count(*) FROM catalog_edit_requests WHERE created_by IS NULL OR decided_by IS NULL",
            2,
        ),
        (
            "SELECT count(*) FROM reports WHERE created_by IS NULL OR resolved_by IS NULL",
            2,
        ),
    ] {
        assert_eq!(count(&app, sql).await, expected, "{sql}");
    }
    // The invite Ada created stays usable for the club.
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM club_invites
             WHERE code = 'ADAS' AND revoked_at IS NULL AND use_count = 0"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn a_wrong_password_counts_as_a_login_failure(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.anon()
        .delete_json("/api/auth/me", json!({"password": PASSWORD}))
        .await
        .assert_detail(401, "Not signed in");
    ada.delete_json("/api/auth/me", json!({}))
        .await
        .assert_detail(400, "Field required");
    for _ in 0..10 {
        delete_me(&ada, "wrong password")
            .await
            .assert_detail(400, "Current password is wrong");
    }
    delete_me(&ada, PASSWORD)
        .await
        .assert_detail(429, "Too many attempts. Wait a few minutes.");
    ada.get("/api/auth/me").await.assert_status(200);
}

/// An app deletes its account with its bearer token; the token dies with it.
#[sqlx::test]
async fn a_bearer_token_can_delete_the_account(db: PgPool) {
    let app = TestApp::new(db).await;
    app.user("ada").await;
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
    let res = delete_me(&phone, PASSWORD).await;
    res.assert_status(204);
    // The clearing cookie goes to every caller (§5.6).
    let cookies = res.set_cookies();
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert!(
        cookies[0].starts_with("bookclub_session=;") && cookies[0].contains("Max-Age=0"),
        "{}",
        cookies[0]
    );
    phone.get("/api/auth/me").await.assert_status(401);
    assert_eq!(count(&app, "SELECT count(*) FROM users").await, 0);
}

/// A transfer that makes the user owner of a club while the deletion runs is caught by the
/// deferred `clubs_have_owner` check at COMMIT, and answered like any owned club: 409, and
/// nothing is deleted. The avatar bytes go only after COMMIT, so they stay too.
#[sqlx::test]
async fn becoming_an_owner_during_the_deletion_is_409(db: PgPool) {
    let app = TestApp::new(db).await;
    let bob = app.user("bob").await;
    let ada = app.user("ada").await;
    let readers = app.club(&bob, "Readers").await;
    app.join(&readers, &ada, "admin").await;
    let (avatar, prefix) = avatar_of(&app, &ada, Some(ada.id())).await;

    // Bob hands the club to Ada in a transaction that is still open.
    let mut transfer = app.db.begin().await.unwrap();
    for (user, role) in [(bob.id(), "member"), (ada.id(), "owner")] {
        sqlx::query("UPDATE club_memberships SET role = $3 WHERE club_id = $1 AND user_id = $2")
            .bind(readers.id)
            .bind(user)
            .bind(role)
            .execute(&mut *transfer)
            .await
            .unwrap();
    }
    let client = ada.clone();
    let deleting = tokio::spawn(async move { delete_me(&client, PASSWORD).await });
    // The deletion did not see the transfer and now waits for Ada's membership row.
    wait_for_a_lock(&app, "the deletion").await;
    transfer.commit().await.unwrap();

    deleting
        .await
        .unwrap()
        .assert_detail(409, TRANSFER_CLUBS_FIRST);
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM club_memberships
                 WHERE club_id = {} AND user_id = {} AND role = 'owner'",
                readers.id,
                ada.id()
            )
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM media WHERE id = {avatar}")
        )
        .await,
        1
    );
    assert_eq!(stored_under(&app, &prefix), 1);
}

/// A join that commits while the deletion waits for the club row is counted: 409, and nothing
/// is deleted. The join locks the club and then needs the inviter's row for `invited_by`; the
/// deletion locks in the same order (§5.3, §5.6), so the two cannot deadlock.
#[sqlx::test]
async fn a_join_during_the_deletion_is_409(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let solo = app.club(&ada, "Solo").await;

    // Bob accepts Ada's invite: the club row first, like every membership write.
    let mut join = app.db.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(solo.id)
        .execute(&mut *join)
        .await
        .unwrap();
    let client = ada.clone();
    let deleting = tokio::spawn(async move { delete_me(&client, PASSWORD).await });
    wait_for_a_lock(&app, "the deletion").await;
    // The membership's foreign key needs Ada's row, which the deletion does not hold yet.
    timeout(
        WAIT,
        sqlx::query(
            "INSERT INTO club_memberships (club_id, user_id, role, invited_by)
             VALUES ($1, $2, 'member', $3)",
        )
        .bind(solo.id)
        .bind(bob.id())
        .bind(ada.id())
        .execute(&mut *join),
    )
    .await
    .expect("the join waited for the deletion")
    .unwrap();
    join.commit().await.unwrap();

    deleting
        .await
        .unwrap()
        .assert_detail(409, TRANSFER_CLUBS_FIRST);
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM club_memberships WHERE club_id = {}",
                solo.id
            )
        )
        .await,
        2
    );
}

/// A reply written while the deletion runs keeps its parent. The deletion locks the user's
/// posts before it decides which to delete, so a reply in flight commits first and its parent
/// becomes a tombstone.
#[sqlx::test]
async fn a_reply_written_during_the_deletion_keeps_its_parent(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let work = app.work("Circe").await;
    let diary = post(&app, work.id, ada.id(), None, None, "ada's diary").await;

    // Bob's reply is written, so its foreign key holds the parent, but not committed.
    let mut reply = app.db.begin().await.unwrap();
    let reply_id: i64 = sqlx::query_scalar(
        "INSERT INTO posts (work_id, author_id, parent_id, body)
         VALUES ($1, $2, $3, 'bob replies') RETURNING id",
    )
    .bind(work.id)
    .bind(bob.id())
    .bind(diary)
    .fetch_one(&mut *reply)
    .await
    .unwrap();
    let client = ada.clone();
    let deleting = tokio::spawn(async move { delete_me(&client, PASSWORD).await });
    wait_for_a_lock(&app, "the deletion").await;
    reply.commit().await.unwrap();

    deleting.await.unwrap().assert_status(204);
    let parent: (Option<i64>, String, bool) =
        sqlx::query_as("SELECT author_id, body, deleted_at IS NOT NULL FROM posts WHERE id = $1")
            .bind(diary)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(parent, (None, String::new(), true));
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) FROM posts
                 WHERE id = {reply_id} AND parent_id = {diary} AND author_id = {}",
                bob.id()
            )
        )
        .await,
        1
    );
}

/// A deletion verified against a hash that a password reset replaced meanwhile deletes nothing
/// (§5.4): the reset wins.
#[sqlx::test]
async fn a_deletion_racing_a_reset_deletes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (avatar, prefix) = avatar_of(&app, &ada, Some(ada.id())).await;
    let reset = app.state.passwords.hash("resetpassword1").await.unwrap();
    let key = format!("u:{}", ada.id());
    // Nine earlier failures: the deletion's own hit blocks the key, which shows that it has
    // read the old hash.
    for _ in 0..9 {
        app.state.limits.login_failures.hit(&key);
    }
    let held = hold_passwords(&app).await;
    let deleting = tokio::spawn({
        let ada = ada.clone();
        async move { delete_me(&ada, PASSWORD).await }
    });
    timeout(WAIT, async {
        while !app.state.limits.login_failures.is_limited(&key) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the deletion never reached the password check");
    app.exec(&format!(
        "UPDATE users SET password_hash = '{reset}' WHERE id = {}",
        ada.id()
    ))
    .await;
    drop(held);

    let res = deleting.await.unwrap();
    res.assert_detail(400, "Current password is wrong");
    assert!(res.set_cookies().is_empty());
    ada.get("/api/auth/me").await.assert_status(200);
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM sessions WHERE user_id = {}", ada.id())
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM media WHERE id = {avatar}")
        )
        .await,
        1
    );
    assert_eq!(stored_under(&app, &prefix), 1);
}

/// The current avatar goes with the account even when no uploader is recorded on it; another
/// user's avatar without an uploader stays.
#[sqlx::test]
async fn the_current_avatar_goes_even_without_an_uploader(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let (avatar, prefix) = avatar_of(&app, &ada, None).await;
    let (bobs_avatar, bobs_prefix) = upload(&app, None, "avatar", 6).await;
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = {bobs_avatar} WHERE id = {}",
        bob.id()
    ))
    .await;

    delete_me(&ada, PASSWORD).await.assert_status(204);
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM media WHERE id = {avatar}")
        )
        .await,
        0
    );
    assert_eq!(stored_under(&app, &prefix), 0);
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM media WHERE id = {bobs_avatar}")
        )
        .await,
        1
    );
    assert_eq!(stored_under(&app, &bobs_prefix), 1);
}

/// Every kind of row a user can leave in a club they do not belong to (or belong to), as SQL
/// with `{c}` the club, `{a}` the user, `{o}` the club's owner and `{w}` a work.
const CLUB_TRACES: &[(&str, &str)] = &[
    (
        "member",
        "INSERT INTO club_memberships (club_id, user_id, role) VALUES ({c}, {a}, 'member')",
    ),
    (
        "creator",
        "UPDATE clubs SET created_by = {a} WHERE id = {c}",
    ),
    (
        "inviter of a member",
        "UPDATE club_memberships SET invited_by = {a} WHERE club_id = {c} AND user_id = {o}",
    ),
    (
        "removed",
        "INSERT INTO club_removals (club_id, user_id) VALUES ({c}, {a})",
    ),
    (
        "invite",
        "INSERT INTO club_invites (club_id, code, created_by, max_uses)
         VALUES ({c}, 'CODE{c}', {a}, 5)",
    ),
    (
        "invite use",
        "WITH i AS (INSERT INTO club_invites (club_id, code, max_uses) VALUES ({c}, 'USE{c}', 5)
                    RETURNING id)
         INSERT INTO club_invite_uses (invite_id, user_id) SELECT id, {a} FROM i",
    ),
    (
        "pick",
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ({c}, {w}, {a})",
    ),
    (
        "milestone",
        "WITH p AS (INSERT INTO club_picks (club_id, work_id) VALUES ({c}, {w}) RETURNING id)
         INSERT INTO milestones (club_id, pick_id, title, created_by)
         SELECT {c}, id, 'Part one', {a} FROM p",
    ),
    (
        "post",
        "WITH p AS (INSERT INTO club_picks (club_id, work_id) VALUES ({c}, {w}) RETURNING id)
         INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
         SELECT {w}, {a}, {c}, id, 'club talk' FROM p",
    ),
    (
        "reaction",
        "WITH p AS (INSERT INTO club_picks (club_id, work_id) VALUES ({c}, {w}) RETURNING id),
              q AS (INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
                    SELECT {w}, {o}, {c}, id, 'club talk' FROM p RETURNING id)
         INSERT INTO post_reactions (post_id, user_id, emoji) SELECT id, {a}, '🔥' FROM q",
    ),
    (
        "nomination",
        "WITH v AS (INSERT INTO next_up_votes (club_id) VALUES ({c}) RETURNING id)
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
         SELECT {c}, id, {w}, {a} FROM v",
    ),
    (
        "ballot",
        "WITH v AS (INSERT INTO next_up_votes (club_id) VALUES ({c}) RETURNING id),
              n AS (INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
                    SELECT {c}, id, {w}, {o} FROM v RETURNING id, vote_id)
         INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
         SELECT {c}, vote_id, {a}, id FROM n",
    ),
    (
        "event",
        "INSERT INTO events (club_id, actor_id, kind) VALUES ({c}, {a}, 'member_joined')",
    ),
];

/// §5.6: the deletion locks every club it writes in before the account row, also a club the
/// user no longer belongs to. A club delete, a join or a removal holds the club and then needs
/// those rows or the account row; the deletion reaches them through its cascade, so the club
/// lock must come first or the two could wait for each other. Each case holds the one club the
/// user left a row in: the deletion waits for it, and the account row is still free meanwhile.
#[sqlx::test]
async fn the_deletion_locks_every_club_it_writes_in_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = app.work("Circe").await;
    for (n, (trace, sql)) in CLUB_TRACES.iter().enumerate() {
        let owner = app.user(&format!("owner{n}")).await;
        let leaver = app.user(&format!("leaver{n}")).await;
        let club = app.club(&owner, "Readers").await;
        app.exec(
            &sql.replace("{c}", &club.id.to_string())
                .replace("{a}", &leaver.id().to_string())
                .replace("{o}", &owner.id().to_string())
                .replace("{w}", &work.id.to_string()),
        )
        .await;

        let mut hold = app.db.begin().await.unwrap();
        sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
            .bind(club.id)
            .execute(&mut *hold)
            .await
            .unwrap();
        let client = leaver.clone();
        let deleting = tokio::spawn(async move { delete_me(&client, PASSWORD).await });
        wait_for_a_lock(&app, &format!("the deletion ({trace})")).await;
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE NOWAIT")
            .bind(leaver.id())
            .execute(&mut *hold)
            .await
            .unwrap_or_else(|error| {
                panic!("{trace}: the deletion locked the account before the club: {error}")
            });
        hold.rollback().await.unwrap();
        deleting.await.unwrap().assert_status(204);
        assert_eq!(
            count(
                &app,
                &format!("SELECT count(*) FROM users WHERE id = {}", leaver.id())
            )
            .await,
            0,
            "{trace}"
        );
    }
}
