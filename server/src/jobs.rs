//! Background jobs (docs/phase1/conventions.md §16). Started by `serve`, never in tests: tests call
//! the job functions directly.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::{Connection, PgConnection, PgPool};
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::{AppState, auth, harvest, media, shelf};

/// `pg_try_advisory_lock` keys, one per periodic job, so a second instance never runs the same
/// job at the same time. Packages with a periodic job reserve a key here.
pub mod lock_ids {
    pub const AUTH_CLEANUP: i64 = 0x6263_0001;
    pub const DNB_HARVEST: i64 = 0x6263_0002;
}

const CLEANUP_EVERY: Duration = Duration::from_secs(3600);

/// Starts every background job; each stops when `shutdown` is cancelled. The caller awaits the
/// returned set after cancelling.
pub fn spawn_all(state: AppState, shutdown: CancellationToken) -> JoinSet<()> {
    let mut jobs = JoinSet::new();
    jobs.spawn(auth_cleanup_loop(state.clone(), shutdown.clone()));
    if state.config.dnb_harvest {
        jobs.spawn(harvest::run_forever(state.clone(), shutdown.clone()));
    }
    jobs.spawn(shelf::goodreads::resume(state.clone()));
    jobs.spawn(media::service::run_prefetch(state, shutdown));
    jobs
}

/// Runs `auth::cleanup` every hour until `shutdown`.
async fn auth_cleanup_loop(state: AppState, shutdown: CancellationToken) {
    let mut every = tokio::time::interval(CLEANUP_EVERY);
    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = every.tick() => {}
        }
        run_locked(&state.db, lock_ids::AUTH_CLEANUP, "auth cleanup", |conn| {
            Box::pin(async move { Ok(auth::cleanup(conn).await?) })
        })
        .await;
    }
}

/// Runs `job` on one connection while holding the advisory lock `lock_id`; skips the run when
/// another session holds it. Logs start, end and duration, and every error; never panics: a
/// panicking job is logged and counts as failed. Returns whether the job ran and succeeded.
pub async fn run_locked<F>(db: &PgPool, lock_id: i64, name: &str, job: F) -> bool
where
    F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, anyhow::Result<()>>,
{
    let mut conn = match db.acquire().await {
        Ok(conn) => conn,
        Err(error) => {
            tracing::error!(job = name, %error, "job: no database connection");
            return false;
        }
    };
    match sqlx::query_scalar!(r#"SELECT pg_try_advisory_lock($1) AS "locked!""#, lock_id)
        .fetch_one(&mut *conn)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(job = name, "job: skipped, another instance holds the lock");
            return false;
        }
        Err(error) => {
            tracing::error!(job = name, %error, "job: cannot take the advisory lock");
            return false;
        }
    }

    tracing::info!(job = name, "job: started");
    let started = Instant::now();
    let outcome = AssertUnwindSafe(job(&mut conn)).catch_unwind().await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let result = match outcome {
        Ok(result) => result,
        Err(panic) => {
            tracing::error!(
                job = name,
                duration_ms,
                panic = panic_message(panic.as_ref()),
                "job: panicked"
            );
            // The connection may be in any state; closing it ends the session and its lock.
            let _ = conn.detach().close().await;
            return false;
        }
    };
    match &result {
        Ok(()) => tracing::info!(job = name, duration_ms, "job: finished"),
        Err(error) => tracing::error!(
            job = name,
            duration_ms,
            error = format!("{error:#}"),
            "job: failed"
        ),
    }

    if let Err(error) =
        sqlx::query_scalar!(r#"SELECT pg_advisory_unlock($1) AS "unlocked!""#, lock_id)
            .fetch_one(&mut *conn)
            .await
    {
        // The lock dies with the session; close the connection instead of returning it to the pool.
        tracing::error!(job = name, %error, "job: cannot release the advisory lock");
        let _ = conn.detach().close().await;
    }
    result.is_ok()
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("(no message)")
}
