//! Background jobs: the advisory-lock runner, the hourly auth cleanup with the retention
//! periods of Q14 (docs/legal/privacy-facts.md §7), and shutdown (docs/phase1/conventions.md
//! §16).

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bookclub_server::media::shared::MediaStore;
use bookclub_server::{auth, jobs};
use bytes::Bytes;
use common::{TestApp, test_state};
use sqlx::{AssertSqlSafe, PgConnection, PgPool};
use tokio_util::sync::CancellationToken;

/// A lock key no real job uses.
const LOCK: i64 = 0x6263_ffff;

async fn insert_user(db: &PgPool, username: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
        .bind(username)
        .fetch_one(db)
        .await
        .unwrap()
}

/// A session that expires `seconds` from now (negative: already expired).
async fn insert_session(db: &PgPool, user_id: i64, seconds: f64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO sessions (user_id, kind, token_hash, expires_at)
         VALUES ($1, 'cookie', sha256(random()::text::bytea), now() + make_interval(secs => $2))
         RETURNING id",
    )
    .bind(user_id)
    .bind(seconds)
    .fetch_one(db)
    .await
    .unwrap()
}

/// An email token that expires `seconds` from now, used or not.
async fn insert_token(db: &PgPool, user_id: i64, seconds: f64, used: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, expires_at, used_at)
         VALUES ($1, 'reset_password', 'ada@example.com', sha256(random()::text::bytea),
                 now() + make_interval(secs => $2), CASE WHEN $3 THEN now() END)
         RETURNING id",
    )
    .bind(user_id)
    .bind(seconds)
    .bind(used)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn ids(db: &PgPool, query: &'static str) -> Vec<i64> {
    sqlx::query_scalar(query).fetch_all(db).await.unwrap()
}

const TRY_LOCK: &str = "SELECT pg_try_advisory_lock($1)";
const UNLOCK: &str = "SELECT pg_advisory_unlock($1)";

/// Runs one of the advisory-lock functions on `LOCK`.
async fn advisory(conn: &mut PgConnection, sql: &'static str) -> bool {
    sqlx::query_scalar(sql)
        .bind(LOCK)
        .fetch_one(conn)
        .await
        .unwrap()
}

/// Whether `conn` (a session that does not hold the lock) can take it, waiting up to 5 s for a
/// closed session to end. Releases it again.
async fn lock_is_free(conn: &mut PgConnection) -> bool {
    for _ in 0..50 {
        if advisory(conn, TRY_LOCK).await {
            assert!(advisory(conn, UNLOCK).await);
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn explode() -> anyhow::Result<()> {
    panic!("the job exploded")
}

#[sqlx::test]
async fn cleanup_deletes_only_expired_sessions_and_spent_tokens(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    insert_session(&db, ada, -60.0).await;
    let live = insert_session(&db, ada, 86_400.0).await;
    insert_token(&db, ada, 3600.0, true).await;
    insert_token(&db, ada, -60.0, false).await;
    let open = insert_token(&db, ada, 3600.0, false).await;

    let mut conn = db.acquire().await.unwrap();
    auth::cleanup(&mut conn).await.unwrap();

    assert_eq!(ids(&db, "SELECT id FROM sessions").await, [live]);
    assert_eq!(ids(&db, "SELECT id FROM email_tokens").await, [open]);
}

#[sqlx::test]
async fn run_locked_skips_while_another_session_holds_the_lock(db: PgPool) {
    let mut holder = db.acquire().await.unwrap();
    assert!(advisory(&mut holder, TRY_LOCK).await);

    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let done = jobs::run_locked(&db, LOCK, "test", move |_conn| {
        Box::pin(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
    })
    .await;
    assert!(!done);
    assert!(!ran.load(Ordering::SeqCst));

    assert!(advisory(&mut holder, UNLOCK).await);
    let flag = ran.clone();
    let done = jobs::run_locked(&db, LOCK, "test", move |_conn| {
        Box::pin(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
    })
    .await;
    assert!(done);
    assert!(ran.load(Ordering::SeqCst));
    // The pooled connection gave the lock back.
    assert!(lock_is_free(&mut holder).await);
}

#[sqlx::test]
async fn run_locked_releases_the_lock_when_the_job_fails(db: PgPool) {
    let mut other = db.acquire().await.unwrap();

    let done = jobs::run_locked(&db, LOCK, "test", |_conn| {
        Box::pin(async move { Err(anyhow::anyhow!("nothing to do")) })
    })
    .await;
    assert!(!done);
    assert!(lock_is_free(&mut other).await);

    // A failed statement leaves the transaction aborted, so the unlock fails too: the
    // connection is closed instead, which ends the lock with the session.
    let done = jobs::run_locked(&db, LOCK, "test", |conn| {
        Box::pin(async move {
            sqlx::query("BEGIN").execute(&mut *conn).await?;
            sqlx::query("SELECT 1 / 0").execute(&mut *conn).await?;
            Ok(())
        })
    })
    .await;
    assert!(!done);
    assert!(lock_is_free(&mut other).await);
}

#[sqlx::test]
async fn run_locked_survives_a_panicking_job(db: PgPool) {
    let mut other = db.acquire().await.unwrap();

    let done = jobs::run_locked(&db, LOCK, "test", |conn| {
        Box::pin(async move {
            sqlx::query("BEGIN").execute(&mut *conn).await?;
            explode()
        })
    })
    .await;
    assert!(!done);
    assert!(lock_is_free(&mut other).await);

    // The pool still works, and the next run takes the lock.
    let done = jobs::run_locked(&db, LOCK, "test", |_conn| Box::pin(async move { Ok(()) })).await;
    assert!(done);
}

#[sqlx::test]
async fn background_jobs_clean_up_at_start_and_stop_on_shutdown(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    insert_session(&db, ada, -60.0).await;

    let shutdown = CancellationToken::new();
    let mut background = jobs::spawn_all(test_state(db.clone()), shutdown.clone());

    // The hourly cleanup runs once right away.
    let mut cleaned = false;
    for _ in 0..50 {
        if ids(&db, "SELECT id FROM sessions").await.is_empty() {
            cleaned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(cleaned, "the expired session is still there");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = background.join_next().await {
            result.expect("a background job failed");
        }
    })
    .await
    .expect("background jobs did not stop within 5 s");
}

// Retention (Q14): decided reports and edit requests after 3 years, empty sign-ups after 30 days.

/// Runs the hourly cleanup once.
async fn run_cleanup(db: &PgPool) {
    let mut conn = db.acquire().await.unwrap();
    auth::cleanup(&mut conn).await.unwrap();
}

/// A sign-up from `days` days ago that typed an address, never verified it, never signed in
/// again and holds nothing: an empty sign-up once `days` is above 30.
async fn sign_up(db: &PgPool, username: &str, days: i32) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, created_at, password_changed_at)
         VALUES ($1, $1 || '@example.test', 'x', now() - make_interval(days => $2),
                 now() - make_interval(days => $2))
         RETURNING id",
    )
    .bind(username)
    .bind(days)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn user_ids(db: &PgPool) -> BTreeSet<i64> {
    sqlx::query_scalar("SELECT id FROM users")
        .fetch_all(db)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// Runs `sql` (one or more statements) with every `{name}` of `values` replaced by its id.
async fn exec(db: &PgPool, sql: &str, values: &[(&str, i64)]) {
    let mut sql = sql.to_owned();
    for (name, value) in values {
        sql = sql.replace(&format!("{{{name}}}"), &value.to_string());
    }
    sqlx::raw_sql(AssertSqlSafe(sql.clone()))
        .execute(db)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

/// A cover report that is `status`, filed `filed` ago (a Postgres interval) and, unless it is
/// open, decided `decided` ago by `admin`.
async fn report(
    db: &PgPool,
    (cover, work, admin): (i64, i64, i64),
    status: &str,
    filed: &str,
    decided: Option<&str>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO reports (target_kind, media_id, work_id, reason, status, created_at,
                              resolved_by, resolved_at)
         VALUES ('cover', $1, $2, 'copyright', $3, now() - $5::interval,
                 CASE WHEN $6::interval IS NOT NULL THEN $4::bigint END, now() - $6::interval)
         RETURNING id",
    )
    .bind(cover)
    .bind(work)
    .bind(status)
    .bind(admin)
    .bind(filed)
    .bind(decided)
    .fetch_one(db)
    .await
    .unwrap()
}

/// A correction request that is `status`, filed and decided as in [`report`].
async fn edit_request(
    db: &PgPool,
    (work, admin): (i64, i64),
    status: &str,
    filed: &str,
    decided: Option<&str>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO catalog_edit_requests (kind, work_id, payload, status, created_at,
                                            decided_by, decided_at)
         VALUES ('work_edit', $1, '{\"title\": \"Kirke\"}', $2, now() - $4::interval,
                 CASE WHEN $5::interval IS NOT NULL THEN $3::bigint END, now() - $5::interval)
         RETURNING id",
    )
    .bind(work)
    .bind(status)
    .bind(admin)
    .bind(filed)
    .bind(decided)
    .fetch_one(db)
    .await
    .unwrap()
}

/// The period runs from the decision, not from the filing: a notice filed 5 years ago and
/// decided 2 years ago stays.
#[sqlx::test]
async fn cleanup_deletes_decisions_three_years_after_they_were_made(db: PgPool) {
    let app = TestApp::new(db).await;
    let db = &app.db;
    let root = app.admin("root").await.id();
    let work = app.work("Circe").await.id;
    let cover: i64 = sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source, removed_at)
         VALUES (repeat('a', 26), 'cover', 'upload', now() - interval '4 years')
         RETURNING id",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let target = (cover, work, root);
    // Filed and decided a day past the period, and a month inside it; filed long before the
    // period but decided inside it; open items are kept however old they are.
    let past = ("3 years 2 days", Some("3 years 1 day"));
    let inside = ("2 years 11 months 1 day", Some("2 years 11 months"));
    let late = ("5 years", Some("2 years"));
    let open = ("5 years", None);

    report(db, target, "removed", past.0, past.1).await;
    report(db, target, "dismissed", past.0, past.1).await;
    let kept_reports = vec![
        report(db, target, "removed", inside.0, inside.1).await,
        report(db, target, "dismissed", inside.0, inside.1).await,
        report(db, target, "removed", late.0, late.1).await,
        report(db, target, "dismissed", late.0, late.1).await,
        report(db, target, "open", open.0, open.1).await,
    ];
    edit_request(db, (work, root), "approved", past.0, past.1).await;
    edit_request(db, (work, root), "rejected", past.0, past.1).await;
    let kept_requests = vec![
        edit_request(db, (work, root), "approved", inside.0, inside.1).await,
        edit_request(db, (work, root), "rejected", inside.0, inside.1).await,
        edit_request(db, (work, root), "approved", late.0, late.1).await,
        edit_request(db, (work, root), "rejected", late.0, late.1).await,
        edit_request(db, (work, root), "open", open.0, open.1).await,
    ];

    run_cleanup(db).await;

    assert_eq!(
        ids(db, "SELECT id FROM reports ORDER BY id").await,
        kept_reports
    );
    assert_eq!(
        ids(db, "SELECT id FROM catalog_edit_requests ORDER BY id").await,
        kept_requests
    );
    // The takedown itself stays: the picture is still removed.
    assert_eq!(ids(db, "SELECT id FROM media").await, [cover]);
}

#[sqlx::test]
async fn cleanup_deletes_sign_ups_that_stayed_empty_for_30_days(db: PgPool) {
    let app = TestApp::new(db).await;
    let db = &app.db;
    let keeper = app.user("keeper").await.id();
    let readers = app.club(&app.user("owner").await, "Readers").await.id;

    // Deleted: signed up 31 days ago, never verified the address, holds nothing. An app token
    // last used 31 days ago and the use of a sign-up invite are traces, not data.
    let gone = sign_up(db, "gone", 31).await;
    exec(
        db,
        "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
             VALUES ({gone}, 'bearer', sha256('gone'::bytea), now() - interval '31 days',
                     now() + interval '59 days');
         INSERT INTO club_invites (club_id, code, created_by, max_uses, use_count)
             VALUES (NULL, 'BETA', {keeper}, 5, 1);
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {gone} FROM club_invites WHERE code = 'BETA';",
        &[("gone", gone), ("keeper", keeper)],
    )
    .await;
    // Deleted too: its last sign-in, 31 days ago, ended with a logout.
    let left_long_ago = sign_up(db, "left_long_ago", 60).await;
    exec(
        db,
        "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
             VALUES ({u}, 'bearer', sha256('left'::bytea), now() - interval '31 days',
                     now() + interval '59 days');
         DELETE FROM sessions WHERE user_id = {u};",
        &[("u", left_long_ago)],
    )
    .await;

    // Kept: inside the period, signed in or active within it, verified, imported, an admin, or
    // holding data.
    let kept = [
        ("signed up 29 days ago", sign_up(db, "young", 29).await, ""),
        (
            "a session used 29 days ago",
            sign_up(db, "session", 60).await,
            "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
             VALUES ({u}, 'cookie', sha256('session'::bytea), now() - interval '29 days',
                     now() + interval '1 day')",
        ),
        (
            "a sign-in 10 days ago that ended with a logout",
            sign_up(db, "logged_out", 60).await,
            "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
             VALUES ({u}, 'cookie', sha256('logged_out'::bytea), now() - interval '10 days',
                     now() + interval '20 days');
             DELETE FROM sessions WHERE user_id = {u}",
        ),
        (
            "the password changed 10 days ago",
            sign_up(db, "password", 60).await,
            "UPDATE users SET password_changed_at = now() - interval '10 days' WHERE id = {u}",
        ),
        (
            "the terms accepted 10 days ago",
            sign_up(db, "terms", 60).await,
            "UPDATE users SET terms_version = '2026-11-01',
                              terms_accepted_at = now() - interval '10 days'
             WHERE id = {u}",
        ),
        (
            "a verification link sent yesterday",
            sign_up(db, "link", 60).await,
            "INSERT INTO email_tokens (user_id, purpose, email, token_hash, created_at, expires_at)
             VALUES ({u}, 'verify_email', 'link@example.test', sha256('link'::bytea),
                     now() - interval '1 day', now() + interval '1 day')",
        ),
        (
            "a verified address",
            sign_up(db, "verified", 60).await,
            "UPDATE users SET email_verified_at = now() - interval '59 days' WHERE id = {u}",
        ),
        (
            "an imported account that never added an address",
            sign_up(db, "imported", 700).await,
            "UPDATE users SET email = NULL WHERE id = {u}",
        ),
        (
            "an imported account with a typed address and a membership",
            sign_up(db, "imported_typed", 700).await,
            "INSERT INTO club_memberships (club_id, user_id, role, joined_at)
             VALUES ({readers}, {u}, 'member', now() - interval '700 days')",
        ),
        (
            "an instance admin",
            sign_up(db, "admin", 60).await,
            "UPDATE users SET is_admin = true WHERE id = {u}",
        ),
        (
            "a profile picture",
            sign_up(db, "avatar", 60).await,
            "INSERT INTO media (public_id, kind, source) VALUES (repeat('b', 26), 'avatar', 'legacy');
             UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE kind = 'avatar')
             WHERE id = {u}",
        ),
        (
            "a shelf entry",
            sign_up(db, "reader", 60).await,
            "INSERT INTO shelf_entries (user_id, work_id, status)
             SELECT {u}, id, 'finished' FROM works",
        ),
    ];
    app.work("Circe").await;
    for (_, user, sql) in &kept {
        exec(db, sql, &[("u", *user), ("readers", readers)]).await;
    }

    run_cleanup(db).await;

    let left = user_ids(db).await;
    assert!(!left.contains(&gone), "the empty sign-up is still there");
    assert!(
        !left.contains(&left_long_ago),
        "the sign-up that logged out 31 days ago is still there"
    );
    for (what, user, _) in kept {
        assert!(left.contains(&user), "an account with {what} was deleted");
    }
    // The account went the way of `DELETE /api/auth/me`: its sessions with it, and the sign-up
    // invite keeps its use without the name.
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(gone)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    let uses: Vec<Option<i64>> = sqlx::query_scalar("SELECT user_id FROM club_invite_uses")
        .fetch_all(db)
        .await
        .unwrap();
    assert_eq!(uses, [None]);
    // Run again: nothing more to delete.
    run_cleanup(db).await;
    assert_eq!(user_ids(db).await, left);
}

/// Every foreign key to `users` whose row means the account holds data, with SQL that gives the
/// account `{u}` such a row. The other names are the fixture rows of
/// [`every_reference_to_an_account_is_data_or_a_trace_of_signing_in`].
const DATA: [(&str, &str); 26] = [
    (
        "shelf_entries.user_id",
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({u}, {work}, 'finished')",
    ),
    (
        "physical_shelf_layouts.user_id",
        "INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ({u}, '{\"rows\": [[1]]}')",
    ),
    (
        "user_favorites.user_id",
        "INSERT INTO user_favorites (user_id, work_id, position) VALUES ({u}, {work}, 1)",
    ),
    (
        "search_history.user_id",
        "INSERT INTO search_history (user_id, query, normalized_query)
         VALUES ({u}, 'Circe', 'circe')",
    ),
    (
        "quotes.user_id",
        "INSERT INTO quotes (user_id, work_id, body, page) VALUES ({u}, {work}, 'A line', 12)",
    ),
    (
        "import_jobs.user_id",
        "INSERT INTO import_jobs (user_id, status, total) VALUES ({u}, 'done', 3)",
    ),
    (
        "posts.author_id",
        "INSERT INTO posts (work_id, author_id, body) VALUES ({work}, {u}, 'my diary')",
    ),
    (
        "post_reactions.user_id",
        "INSERT INTO post_reactions (post_id, user_id, emoji) VALUES ({post}, {u}, '🔥')",
    ),
    (
        "clubs.created_by",
        "UPDATE clubs SET created_by = {u} WHERE id = {second}",
    ),
    (
        "club_memberships.user_id",
        "INSERT INTO club_memberships (club_id, user_id, role) VALUES ({club}, {u}, 'member')",
    ),
    (
        "club_memberships.invited_by",
        "UPDATE club_memberships SET invited_by = {u} WHERE club_id = {second}",
    ),
    (
        "club_removals.user_id",
        "INSERT INTO club_removals (club_id, user_id) VALUES ({club}, {u})",
    ),
    (
        "club_invites.created_by",
        "INSERT INTO club_invites (club_id, code, created_by, max_uses)
         VALUES ({club}, 'BY{u}', {u}, 5)",
    ),
    (
        "club_invite_uses.user_id",
        "INSERT INTO club_invite_uses (invite_id, user_id) VALUES ({invite}, {u})",
    ),
    (
        "club_picks.set_by",
        "INSERT INTO club_picks (club_id, work_id, set_by, started_at, ended_at)
         VALUES ({club}, {other}, {u}, now() - interval '9 days', now() - interval '2 days')",
    ),
    (
        "milestones.created_by",
        "INSERT INTO milestones (club_id, pick_id, title, created_by)
         VALUES ({club}, {pick}, 'Part one', {u})",
    ),
    (
        "next_up_nominations.nominated_by",
        "INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
         VALUES ({club}, {vote}, {other}, {u})",
    ),
    (
        "next_up_ballots.user_id",
        "INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
         VALUES ({club}, {vote}, {u}, {nomination})",
    ),
    (
        "events.actor_id",
        "INSERT INTO events (club_id, actor_id, kind) VALUES ({club}, {u}, 'member_joined')",
    ),
    (
        "works.created_by",
        "UPDATE works SET created_by = {u} WHERE id = {added}",
    ),
    (
        "editions.created_by",
        "UPDATE editions SET created_by = {u} WHERE id = {edition}",
    ),
    (
        "media.uploaded_by",
        "UPDATE media SET uploaded_by = {u} WHERE id = {cover}",
    ),
    (
        "catalog_edit_requests.created_by",
        "INSERT INTO catalog_edit_requests (kind, work_id, created_by)
         VALUES ('work_edit', {work}, {u})",
    ),
    (
        "catalog_edit_requests.decided_by",
        "INSERT INTO catalog_edit_requests (kind, work_id, status, decided_by, decided_at)
         VALUES ('work_edit', {work}, 'rejected', {u}, now())",
    ),
    (
        "reports.created_by",
        "INSERT INTO reports (target_kind, media_id, work_id, reason, created_by)
         VALUES ('cover', {cover}, {work}, 'other', {u})",
    ),
    (
        "reports.resolved_by",
        "INSERT INTO reports (target_kind, media_id, work_id, reason, status, resolved_by,
                              resolved_at)
         VALUES ('cover', {cover}, {work}, 'other', 'dismissed', {u}, now())",
    ),
];

/// The foreign keys to `users` that only record signing up or signing in, why, and SQL that
/// gives the account `{u}` such a row more than 30 days old: an account with nothing else is
/// still an empty sign-up.
const TRACES: [(&str, &str, &str); 3] = [
    (
        "sessions.user_id",
        "signing in; the purge reads `last_used_at`",
        "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
         VALUES ({u}, 'bearer', sha256('s{u}'::bytea), now() - interval '31 days',
                 now() + interval '59 days')",
    ),
    (
        "email_tokens.user_id",
        "the verification link of signing up; the purge reads `created_at` (a link this old \
         is expired and gone in practice, so this one is kept unexpired to reach the purge)",
        "INSERT INTO email_tokens (user_id, purpose, email, token_hash, created_at, expires_at)
         VALUES ({u}, 'verify_email', 'trace@example.test', sha256('t{u}'::bytea),
                 now() - interval '31 days', now() + interval '1 day')",
    ),
    (
        "club_invite_uses.user_id",
        "the use of a sign-up invite (`club_id` NULL), which only unlocked the sign-up; the use \
         of a club invite is data",
        "INSERT INTO club_invite_uses (invite_id, user_id) VALUES ({beta}, {u})",
    ),
];

/// The purge of empty sign-ups (`auth::empty_sign_ups`) must consider every foreign key to
/// `users`. The schema's list comes from `pg_constraint`, as in `auth_export.rs`: a new column
/// fails here until it is in [`DATA`] (and the purge has a `NOT EXISTS` for it, or the account
/// below is deleted) or in [`TRACES`] with a reason.
#[sqlx::test]
async fn every_reference_to_an_account_is_data_or_a_trace_of_signing_in(db: PgPool) {
    let in_schema: BTreeSet<String> = sqlx::query_scalar(
        "SELECT c.conrelid::regclass::text || '.' || a.attname
         FROM pg_constraint c
         CROSS JOIN LATERAL unnest(c.conkey) AS k(attnum)
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
         WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass",
    )
    .fetch_all(&db)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let named: BTreeSet<String> = DATA
        .iter()
        .map(|(column, _)| column.to_string())
        .chain(TRACES.iter().map(|(column, _, _)| column.to_string()))
        .collect();
    let unconsidered: Vec<&String> = in_schema.difference(&named).collect();
    assert!(
        unconsidered.is_empty(),
        "the purge of empty sign-ups does not consider these foreign keys to users: \
         {unconsidered:?}. Add a NOT EXISTS for each to auth::empty_sign_ups and a row to DATA, \
         or name it in TRACES with the reason"
    );
    let gone: Vec<&String> = named.difference(&in_schema).collect();
    assert!(gone.is_empty(), "no longer in the schema: {gone:?}");
    assert!(TRACES.iter().all(|(_, reason, _)| !reason.is_empty()));

    let app = TestApp::new(db).await;
    let db = &app.db;
    let keeper = app.user("keeper").await;
    let club = app.club(&keeper, "Readers").await.id;
    let second = app.club(&keeper, "Second").await.id;
    let work = app.work("Circe").await;
    let other = app.work("Ulysses").await.id;
    let added = app.work("Dune").await.id;
    let edition = app.edition(&work, "9780316556347").await.id;
    let mut values = vec![
        ("keeper", keeper.id()),
        ("club", club),
        ("second", second),
        ("work", work.id),
        ("other", other),
        ("added", added),
        ("edition", edition),
    ];
    exec(
        db,
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ({club}, {work}, {keeper});
         INSERT INTO next_up_votes (club_id) VALUES ({club});
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {club}, id, {work}, {keeper} FROM next_up_votes;
         INSERT INTO posts (work_id, author_id, body) VALUES ({work}, {keeper}, 'keeper diary');
         INSERT INTO club_invites (club_id, code, created_by, max_uses)
             VALUES ({club}, 'CLUB', {keeper}, 100), (NULL, 'BETA', {keeper}, 100);
         INSERT INTO media (public_id, kind, source)
             VALUES (repeat('a', 26), 'cover', 'upload');",
        &values,
    )
    .await;
    for (name, sql) in [
        ("pick", "SELECT id FROM club_picks"),
        ("vote", "SELECT id FROM next_up_votes"),
        ("nomination", "SELECT id FROM next_up_nominations"),
        ("post", "SELECT id FROM posts"),
        ("invite", "SELECT id FROM club_invites WHERE code = 'CLUB'"),
        ("beta", "SELECT id FROM club_invites WHERE code = 'BETA'"),
        ("cover", "SELECT id FROM media"),
    ] {
        values.push((name, app.scalar_i64(sql).await));
    }

    let control = sign_up(db, "control", 31).await;
    let mut with_data = Vec::new();
    for (index, (column, sql)) in DATA.iter().enumerate() {
        let user = sign_up(db, &format!("data_{index}"), 31).await;
        values.push(("u", user));
        exec(db, sql, &values).await;
        values.pop();
        with_data.push((*column, user));
    }
    let mut with_traces = Vec::new();
    for (index, (column, _, sql)) in TRACES.iter().enumerate() {
        let user = sign_up(db, &format!("trace_{index}"), 31).await;
        values.push(("u", user));
        exec(db, sql, &values).await;
        values.pop();
        with_traces.push((*column, user));
    }

    // Kept or deleted by the rule itself, not because a deletion failed and was logged: every
    // account with data is no empty sign-up, and every account with only a trace is one.
    let mut conn = db.acquire().await.unwrap();
    assert!(
        auth::is_empty_sign_up(&mut conn, control).await.unwrap(),
        "the control account is no empty sign-up"
    );
    let mut candidates = Vec::new();
    for (column, user) in &with_data {
        if auth::is_empty_sign_up(&mut conn, *user).await.unwrap() {
            candidates.push(*column);
        }
    }
    assert!(
        candidates.is_empty(),
        "accounts with data count as empty sign-ups; add a NOT EXISTS to auth::empty_sign_ups \
         for {candidates:?}"
    );
    let mut not_candidates = Vec::new();
    for (column, user) in &with_traces {
        if !auth::is_empty_sign_up(&mut conn, *user).await.unwrap() {
            not_candidates.push(*column);
        }
    }
    assert!(
        not_candidates.is_empty(),
        "accounts with only these traces do not count as empty sign-ups: {not_candidates:?}"
    );
    drop(conn);

    run_cleanup(db).await;

    let left = user_ids(db).await;
    assert!(!left.contains(&control), "the purge did not run");
    let deleted: Vec<&str> = with_data
        .iter()
        .filter(|(_, user)| !left.contains(user))
        .map(|(column, _)| *column)
        .collect();
    assert!(
        deleted.is_empty(),
        "accounts with data were purged; add a NOT EXISTS to auth::empty_sign_ups for {deleted:?}"
    );
    let kept: Vec<&str> = with_traces
        .iter()
        .filter(|(_, user)| left.contains(user))
        .map(|(column, _)| *column)
        .collect();
    assert!(
        kept.is_empty(),
        "accounts with only these traces were kept: {kept:?}"
    );
}

/// Waits up to 5 s until a session of this database waits for a lock.
async fn wait_for_a_lock_wait(db: &PgPool) {
    for _ in 0..250 {
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
    panic!("nothing waits for a lock after 5 s");
}

/// Data written while the purge is at an account keeps the account: the purge picks it (the
/// shelf entry is not committed yet), waits for the writer's lock on the user row, and looks
/// again once it has the row.
#[sqlx::test]
async fn data_written_while_the_purge_waits_keeps_the_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = app.work("Circe").await.id;
    let ada = sign_up(&app.db, "ada", 31).await;

    let mut writer = app.db.begin().await.unwrap();
    sqlx::query("INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, 'finished')")
        .bind(ada)
        .bind(work)
        .execute(&mut *writer)
        .await
        .unwrap();
    let purge = tokio::spawn({
        let db = app.db.clone();
        async move {
            let mut conn = db.acquire().await.unwrap();
            auth::purge_empty_sign_ups(&mut conn).await.unwrap()
        }
    });
    wait_for_a_lock_wait(&app.db).await;
    writer.commit().await.unwrap();

    assert_eq!(purge.await.unwrap(), 0);
    assert!(user_ids(&app.db).await.contains(&ada));
    assert_eq!(
        ids(&app.db, "SELECT user_id FROM shelf_entries").await,
        [ada]
    );
}

#[sqlx::test]
async fn the_purge_deletes_a_batch_per_run(db: PgPool) {
    sqlx::query(
        "INSERT INTO users (username, email, password_hash, created_at, password_changed_at)
         SELECT 'bulk' || i, 'bulk' || i || '@example.test', 'x',
                now() - interval '31 days', now() - interval '31 days'
         FROM generate_series(1, $1 + 1) AS i",
    )
    .bind(auth::PURGE_BATCH)
    .execute(&db)
    .await
    .unwrap();

    let mut conn = db.acquire().await.unwrap();
    let first = auth::purge_empty_sign_ups(&mut conn).await.unwrap();
    assert_eq!(first, auth::PURGE_BATCH as u64);
    assert_eq!(auth::purge_empty_sign_ups(&mut conn).await.unwrap(), 1);
    assert!(user_ids(&db).await.is_empty());
}

/// An account whose deletion fails is logged and kept, and the purge goes on with the next.
#[sqlx::test]
async fn a_failing_deletion_keeps_that_account_and_the_purge_goes_on(db: PgPool) {
    let stuck = sign_up(&db, "stuck", 31).await;
    let gone = sign_up(&db, "gone", 31).await;
    assert!(stuck < gone, "the failing account comes first");
    exec(
        &db,
        "CREATE FUNCTION refuse_delete() RETURNS trigger LANGUAGE plpgsql
             AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$;
         CREATE TRIGGER refuse_delete BEFORE DELETE ON users
             FOR EACH ROW WHEN (OLD.id = {stuck}) EXECUTE FUNCTION refuse_delete();",
        &[("stuck", stuck)],
    )
    .await;

    let mut conn = db.acquire().await.unwrap();
    assert_eq!(auth::purge_empty_sign_ups(&mut conn).await.unwrap(), 1);
    assert_eq!(user_ids(&db).await, BTreeSet::from([stuck]));
}

/// Every column of `users`, and how the purge of empty sign-ups reads it. A column that records
/// a sign-in or a state that keeps the account (a suspension) needs a condition in
/// `auth::empty_sign_ups` and a kept case in
/// [`cleanup_deletes_sign_ups_that_stayed_empty_for_30_days`] before it is named here.
const USERS_COLUMNS: [(&str, &str); 20] = [
    ("id", "the account"),
    ("username", "no bearing"),
    (
        "email",
        "an empty sign-up has one; an account without one is imported",
    ),
    (
        "email_verified_at",
        "an empty sign-up never verified its address",
    ),
    (
        "email_prompt_dismissed_at",
        "no bearing: an empty sign-up has an address, so it sees no prompt",
    ),
    (
        "password_hash",
        "read under the lock, for deletion::delete_user",
    ),
    ("password_changed_at", "a sign-in: a change needs one"),
    ("is_admin", "an instance admin is never purged"),
    (
        "theme",
        "a preference; changing it needs a session, whose use counts",
    ),
    ("color_mode", "a preference, as theme"),
    ("locale", "a preference, as theme"),
    ("timezone", "a preference, as theme"),
    ("notify_meeting", "a preference, as theme"),
    ("notify_pick", "a preference, as theme"),
    ("notify_note", "a preference, as theme"),
    (
        "avatar_media_id",
        "data: an empty sign-up has no profile picture",
    ),
    ("created_at", "the sign-up"),
    ("terms_version", "set with terms_accepted_at"),
    ("terms_accepted_at", "a sign-in: accepting needs one"),
    (
        "last_seen_at",
        "a sign-in: the last use of a session that has ended",
    ),
];

/// A new column of `users` is not a foreign key, so
/// [`every_reference_to_an_account_is_data_or_a_trace_of_signing_in`] does not see it. This
/// fails until it is in [`USERS_COLUMNS`].
#[sqlx::test]
async fn every_column_of_users_is_considered_by_the_purge(db: PgPool) {
    let in_schema: BTreeSet<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = 'users'",
    )
    .fetch_all(&db)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let named: BTreeSet<String> = USERS_COLUMNS
        .iter()
        .map(|(column, _)| column.to_string())
        .collect();
    let unconsidered: Vec<&String> = in_schema.difference(&named).collect();
    assert!(
        unconsidered.is_empty(),
        "the purge of empty sign-ups does not consider these columns of users: \
         {unconsidered:?}. A sign-in or a state that keeps the account needs a condition in \
         auth::empty_sign_ups and a kept case in \
         cleanup_deletes_sign_ups_that_stayed_empty_for_30_days; then name it in USERS_COLUMNS"
    );
    let gone: Vec<&String> = named.difference(&in_schema).collect();
    assert!(gone.is_empty(), "no longer in the schema: {gone:?}");
    assert!(USERS_COLUMNS.iter().all(|(_, how)| !how.is_empty()));
}

/// Using an existing session while the purge is at the account keeps the account. The use
/// updates only the session row, which the purge locks after the account and before its second
/// look.
#[sqlx::test]
async fn a_session_used_while_the_purge_waits_keeps_the_account(db: PgPool) {
    let ada = sign_up(&db, "ada", 31).await;
    exec(
        &db,
        "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
         VALUES ({u}, 'bearer', sha256('ada'::bytea), now() - interval '31 days',
                 now() + interval '59 days')",
        &[("u", ada)],
    )
    .await;

    // What `auth::extract` does with a session last used more than an hour ago.
    let mut touch = db.begin().await.unwrap();
    sqlx::query("UPDATE sessions SET last_used_at = now() WHERE user_id = $1")
        .bind(ada)
        .execute(&mut *touch)
        .await
        .unwrap();
    let purge = tokio::spawn({
        let db = db.clone();
        async move {
            let mut conn = db.acquire().await.unwrap();
            auth::purge_empty_sign_ups(&mut conn).await.unwrap()
        }
    });
    wait_for_a_lock_wait(&db).await;
    touch.commit().await.unwrap();

    assert_eq!(purge.await.unwrap(), 0);
    assert!(user_ids(&db).await.contains(&ada));
}

/// A session used `used` ago (a Postgres interval) that expires in `expires`.
async fn session_used(db: &PgPool, user_id: i64, used: &str, expires: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO sessions (user_id, kind, token_hash, last_used_at, expires_at)
         VALUES ($1, 'cookie', sha256(random()::text::bytea), now() - $2::interval,
                 now() + $3::interval)
         RETURNING id",
    )
    .bind(user_id)
    .bind(used)
    .bind(expires)
    .fetch_one(db)
    .await
    .unwrap()
}

/// How many hours ago `users.last_seen_at` of the account is, rounded.
async fn hours_since_seen(db: &PgPool, user_id: i64) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT round(extract(epoch FROM now() - last_seen_at) / 3600)::bigint
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn delete_session(conn: &mut PgConnection, id: i64) {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await
        .unwrap();
}

/// A session that ends before it expires leaves its last use on the account
/// (`users.last_seen_at`, migration 2201), so a sign-in outlives its logout.
#[sqlx::test]
async fn a_session_that_ends_early_leaves_its_last_use_on_the_account(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let mut conn = db.acquire().await.unwrap();
    assert_eq!(hours_since_seen(&db, ada).await, None);

    // A logout, then the logout of an older session: the newer use stays.
    let session = session_used(&db, ada, "10 days", "20 days").await;
    delete_session(&mut conn, session).await;
    assert_eq!(hours_since_seen(&db, ada).await, Some(240));
    let session = session_used(&db, ada, "20 days", "10 days").await;
    delete_session(&mut conn, session).await;
    assert_eq!(hours_since_seen(&db, ada).await, Some(240));

    // An expired session leaves nothing: the hourly cleanup of expired rows writes nothing.
    let session = session_used(&db, ada, "2 hours", "-1 hour").await;
    delete_session(&mut conn, session).await;
    assert_eq!(hours_since_seen(&db, ada).await, Some(240));

    // While another transaction holds the account's row, a logout does not wait for it (that
    // could deadlock with a password change), and leaves nothing.
    let mut holder = db.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(ada)
        .execute(&mut *holder)
        .await
        .unwrap();
    let session = session_used(&db, ada, "1 hour", "1 day").await;
    sqlx::query("SET lock_timeout = '2s'")
        .execute(&mut *conn)
        .await
        .unwrap();
    delete_session(&mut conn, session).await;
    holder.commit().await.unwrap();
    assert_eq!(hours_since_seen(&db, ada).await, Some(240));

    // A transaction that holds the row itself (a password change ending the other sessions)
    // leaves the use.
    let session = session_used(&db, ada, "2 hours", "1 day").await;
    let mut change = db.begin().await.unwrap();
    sqlx::query("UPDATE users SET password_changed_at = now() WHERE id = $1")
        .bind(ada)
        .execute(&mut *change)
        .await
        .unwrap();
    delete_session(&mut change, session).await;
    change.commit().await.unwrap();
    assert_eq!(hours_since_seen(&db, ada).await, Some(2));

    // Deleting the account deletes its live sessions with it; the trigger finds no account.
    session_used(&db, ada, "1 hour", "1 day").await;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(ada)
        .execute(&mut *conn)
        .await
        .unwrap();
    assert!(user_ids(&db).await.is_empty());
}

/// Every foreign key to `media`: `auth::delete_rejected_cover_photos` keeps a photo that any of
/// them references (other than a request that is due). A new one fails
/// [`rejected_cover_photos_go_with_their_request`] until the function considers it.
const MEDIA_REFERENCES: [&str; 6] = [
    "catalog_edit_requests.media_id",
    "editions.cover_media_id",
    "media_removed_hashes.media_id",
    "reports.media_id",
    "users.avatar_media_id",
    "works.cover_media_id",
];

/// A reader's photo stored as a cover upload is: a media row with its bytes under its prefix.
async fn stored_photo(app: &TestApp, uploader: i64, letter: char) -> (i64, String) {
    let public_id = letter.to_string().repeat(26);
    let prefix = format!("covers/{}/{public_id}", &public_id[..2]);
    let id = sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants,
                            uploaded_by)
         VALUES ($1, 'cover', 'upload', $2, 'image/webp', '{l,s}', $3)
         RETURNING id",
    )
    .bind(&public_id)
    .bind(&prefix)
    .bind(uploader)
    .fetch_one(&app.db)
    .await
    .unwrap();
    for variant in ["l", "s"] {
        app.media
            .put(
                &format!("{prefix}/{variant}"),
                Bytes::from_static(b"webp"),
                "image/webp",
            )
            .await
            .unwrap();
    }
    (id, prefix)
}

/// A `cover_replace` request for `photo` that is `status`, filed and decided as in [`report`].
async fn cover_request(
    app: &TestApp,
    (work, photo, uploader, admin): (i64, i64, i64, i64),
    status: &str,
    filed: &str,
    decided: Option<&str>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO catalog_edit_requests (kind, work_id, media_id, status, created_by,
                                            created_at, decided_by, decided_at)
         VALUES ('cover_replace', $1, $2, $3, $4, now() - $6::interval,
                 CASE WHEN $7::interval IS NOT NULL THEN $5::bigint END, now() - $7::interval)
         RETURNING id",
    )
    .bind(work)
    .bind(photo)
    .bind(status)
    .bind(uploader)
    .bind(admin)
    .bind(filed)
    .bind(decided)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// The photo of a rejected cover request goes with the request, 3 years after the decision:
/// its row, its files, and the request. A photo that something else references stays.
#[sqlx::test]
async fn rejected_cover_photos_go_with_their_request(db: PgPool) {
    let in_schema: BTreeSet<String> = sqlx::query_scalar(
        "SELECT c.conrelid::regclass::text || '.' || a.attname
         FROM pg_constraint c
         CROSS JOIN LATERAL unnest(c.conkey) AS k(attnum)
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
         WHERE c.contype = 'f' AND c.confrelid = 'media'::regclass",
    )
    .fetch_all(&db)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(
        in_schema,
        MEDIA_REFERENCES.iter().map(|c| c.to_string()).collect(),
        "auth::delete_rejected_cover_photos must keep a photo that a new foreign key to media \
         references; then update MEDIA_REFERENCES"
    );

    let app = TestApp::new(db).await;
    let root = app.admin("root").await.id();
    let ada = app.user("ada").await.id();
    let work = app.work("Circe").await.id;
    let past = ("3 years 2 days", Some("3 years 1 day"));
    let inside = ("2 years 11 months 1 day", Some("2 years 11 months"));

    // Deleted: rejected a day past the period.
    let (due_photo, due_prefix) = stored_photo(&app, ada, 'c').await;
    let due = cover_request(
        &app,
        (work, due_photo, ada, root),
        "rejected",
        past.0,
        past.1,
    )
    .await;
    // Kept: rejected inside the period, and still open after 5 years.
    let (young_photo, young_prefix) = stored_photo(&app, ada, 'd').await;
    let young = cover_request(
        &app,
        (work, young_photo, ada, root),
        "rejected",
        inside.0,
        inside.1,
    )
    .await;
    let (open_photo, open_prefix) = stored_photo(&app, ada, 'e').await;
    let open = cover_request(&app, (work, open_photo, ada, root), "open", "5 years", None).await;
    // Kept with its request: a report names the photo.
    let (reported_photo, reported_prefix) = stored_photo(&app, ada, 'f').await;
    let reported = cover_request(
        &app,
        (work, reported_photo, ada, root),
        "rejected",
        past.0,
        past.1,
    )
    .await;
    report(
        &app.db,
        (reported_photo, work, root),
        "dismissed",
        "1 year 1 day",
        Some("1 year"),
    )
    .await;
    // The photo stays as the cover, and the request goes: approved past the period.
    let (cover_photo, cover_prefix) = stored_photo(&app, ada, 'g').await;
    exec(
        &app.db,
        "UPDATE works SET cover_media_id = {photo} WHERE id = {work}",
        &[("photo", cover_photo), ("work", work)],
    )
    .await;
    cover_request(
        &app,
        (work, cover_photo, ada, root),
        "approved",
        past.0,
        past.1,
    )
    .await;
    // A taken-down photo has no files left, and its row stays; the request goes.
    let removed_photo: i64 = sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source, uploaded_by, removed_at)
         VALUES (repeat('h', 26), 'cover', 'upload', $1, now() - interval '3 years 1 day')
         RETURNING id",
    )
    .bind(ada)
    .fetch_one(&app.db)
    .await
    .unwrap();
    cover_request(
        &app,
        (work, removed_photo, ada, root),
        "rejected",
        past.0,
        past.1,
    )
    .await;

    let mut conn = app.db.acquire().await.unwrap();
    auth::cleanup(&mut conn).await.unwrap();
    // The cleanup leaves the due request to the photo's deletion.
    assert_eq!(
        ids(&app.db, "SELECT id FROM catalog_edit_requests ORDER BY id").await,
        [due, young, open, reported]
    );
    let deleted = auth::delete_rejected_cover_photos(&mut conn, app.media.as_ref())
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    assert_eq!(
        ids(&app.db, "SELECT id FROM catalog_edit_requests ORDER BY id").await,
        [young, open, reported]
    );
    assert_eq!(
        ids(&app.db, "SELECT id FROM media ORDER BY id").await,
        [
            young_photo,
            open_photo,
            reported_photo,
            cover_photo,
            removed_photo
        ]
    );
    let keys = app.media.keys();
    assert!(
        !keys.iter().any(|key| key.starts_with(&due_prefix)),
        "the rejected photo's files are still there: {keys:?}"
    );
    for prefix in [young_prefix, open_prefix, reported_prefix, cover_prefix] {
        assert!(
            keys.iter().any(|key| key.starts_with(&prefix)),
            "{prefix} was deleted"
        );
    }
    // Run again: nothing more to delete.
    auth::cleanup(&mut conn).await.unwrap();
    assert_eq!(
        auth::delete_rejected_cover_photos(&mut conn, app.media.as_ref())
            .await
            .unwrap(),
        0
    );
}

/// The hourly job runs `delete_rejected_cover_photos` after `cleanup`: its run at startup deletes
/// a due photo with its request and its files.
#[sqlx::test]
async fn background_jobs_delete_the_photos_of_rejected_cover_requests(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await.id();
    let ada = app.user("ada").await.id();
    let work = app.work("Circe").await.id;
    let (photo, prefix) = stored_photo(&app, ada, 'c').await;
    cover_request(
        &app,
        (work, photo, ada, root),
        "rejected",
        "3 years 2 days",
        Some("3 years 1 day"),
    )
    .await;

    let shutdown = CancellationToken::new();
    let mut background = jobs::spawn_all(app.state.clone(), shutdown.clone());

    // The row goes first, the files after the commit.
    let mut deleted = false;
    for _ in 0..50 {
        let files_left = app.media.keys().iter().any(|key| key.starts_with(&prefix));
        if !files_left && ids(&app.db, "SELECT id FROM media").await.is_empty() {
            deleted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(deleted, "the rejected photo or its files are still there");
    assert!(
        ids(&app.db, "SELECT id FROM catalog_edit_requests")
            .await
            .is_empty()
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = background.join_next().await {
            result.expect("a background job failed");
        }
    })
    .await
    .expect("background jobs did not stop within 5 s");
}

/// A report written while the deletion is at the photo keeps it: the deletion locks the photo
/// and looks again once the report committed.
#[sqlx::test]
async fn a_report_written_while_the_photo_is_deleted_keeps_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await.id();
    let ada = app.user("ada").await.id();
    let work = app.work("Circe").await.id;
    let (photo, prefix) = stored_photo(&app, ada, 'c').await;
    cover_request(
        &app,
        (work, photo, ada, root),
        "rejected",
        "3 years 2 days",
        Some("3 years 1 day"),
    )
    .await;

    let mut writer = app.db.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO reports (target_kind, media_id, work_id, reason) VALUES ('cover', $1, $2, 'other')",
    )
    .bind(photo)
    .bind(work)
    .execute(&mut *writer)
    .await
    .unwrap();
    let deletion = tokio::spawn({
        let db = app.db.clone();
        let store = app.media.clone();
        async move {
            let mut conn = db.acquire().await.unwrap();
            auth::delete_rejected_cover_photos(&mut conn, store.as_ref())
                .await
                .unwrap()
        }
    });
    wait_for_a_lock_wait(&app.db).await;
    writer.commit().await.unwrap();

    assert_eq!(deletion.await.unwrap(), 0);
    assert_eq!(ids(&app.db, "SELECT id FROM media").await, [photo]);
    assert!(app.media.keys().iter().any(|key| key.starts_with(&prefix)));
}
