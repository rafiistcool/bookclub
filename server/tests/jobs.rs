//! Background jobs: the advisory-lock runner, the hourly auth cleanup and shutdown
//! (docs/phase1/conventions.md §16).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bookclub_server::{auth, jobs};
use common::test_state;
use sqlx::{PgConnection, PgPool};
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
