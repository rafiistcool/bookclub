//! The DNB OAI-PMH harvester (#42, owner HAR) and `GET /api/books/new-releases`.
//!
//! - [`oai`]: the OAI-PMH envelope (request URLs, headers, resumption tokens, OAI errors). Each
//!   MARC21-xml record goes to the one W1 mapping, `catalog::shared::dnb_marc::map_record`.
//! - [`run`]: one harvest of one set with its cursor in `harvest_state`, and the schedule
//!   (`is_due`, `next_due_in`).
//! - [`new_releases`]: the Discover row of upcoming German titles, local only.
//! - [`run_forever`]: the background job (`DNB_HARVEST=true`): a [`run_pass`] under the
//!   advisory lock `jobs::lock_ids::DNB_HARVEST`, then a sleep of [`next_wait`].

pub mod new_releases;
pub mod oai;
pub mod run;

use std::time::Duration;

use sqlx::PgConnection;
use tokio_util::sync::CancellationToken;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use run::{HarvestError, SetReport, harvest_set};

use crate::AppState;
use crate::jobs::{self, lock_ids};

/// How soon `run_forever` looks again when a set is due but could not run (another instance
/// holds the lock, or the database failed).
pub const RECHECK: Duration = Duration::from_secs(60);

/// The harvest routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(new_releases::new_releases))
}

/// One pass of the job: harvests every set of `DNB_HARVEST_SETS` that is due ([`run::is_due`]),
/// one after the other, until `shutdown`. A failing set does not stop the others; the pass then
/// fails with every set's error.
pub async fn harvest_due(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &CancellationToken,
) -> anyhow::Result<()> {
    let every = run::interval(&state.config);
    let mut failures = Vec::new();
    for set in &state.config.dnb_harvest_sets {
        if shutdown.is_cancelled() {
            break;
        }
        if !run::is_due(conn, set, every).await? {
            tracing::info!(set, "dnb harvest: not due yet");
            continue;
        }
        if let Err(error) = harvest_set(state, conn, set, shutdown).await {
            failures.push(format!("{set}: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(failures.join("; ")))
    }
}

/// Harvests every `DNB_HARVEST_SETS` set every `DNB_HARVEST_INTERVAL_MINUTES` (never more often
/// than every 30 minutes) until `shutdown`. Started by `jobs::spawn_all` only when
/// `DNB_HARVEST` is set. Each pass runs under the advisory lock, so a second instance skips it.
/// When a set is due is read from `harvest_state`, so a restart neither harvests early nor waits
/// a whole interval, and an interrupted run resumes at once.
pub async fn run_forever(state: AppState, shutdown: CancellationToken) {
    loop {
        run_pass(&state, &shutdown).await;
        let wait = next_wait(&state).await;
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// One pass of [`run_forever`]: [`harvest_due`] under the advisory lock
/// `jobs::lock_ids::DNB_HARVEST`. False when another instance holds the lock (nothing is sent),
/// or when the pass failed (`jobs::run_locked` logs why).
pub async fn run_pass(state: &AppState, shutdown: &CancellationToken) -> bool {
    let job_state = state.clone();
    let job_shutdown = shutdown.clone();
    jobs::run_locked(
        &state.db,
        lock_ids::DNB_HARVEST,
        "dnb harvest",
        move |conn| Box::pin(async move { harvest_due(&job_state, conn, &job_shutdown).await }),
    )
    .await
}

/// How long [`run_forever`] sleeps after a pass: until the next set is due ([`run::next_due_in`]),
/// at least [`RECHECK`] and at most the interval ([`wait_after`]); [`RECHECK`] when the schedule
/// cannot be read.
pub async fn next_wait(state: &AppState) -> Duration {
    let every = run::interval(&state.config);
    let due_in = match state.db.acquire().await {
        Ok(mut conn) => run::next_due_in(&mut conn, &state.config.dnb_harvest_sets, every).await,
        Err(error) => Err(error),
    };
    match due_in {
        Ok(due_in) => wait_after(due_in, every),
        Err(error) => {
            tracing::error!(%error, "dnb harvest: cannot read the schedule");
            RECHECK
        }
    }
}

/// The sleep before the next pass when the next set is due in `due_in`: never shorter than
/// [`RECHECK`], so a set that is due but cannot run (another instance holds the lock, or it
/// failed before recording why) never makes the job spin, and never longer than `every`.
pub fn wait_after(due_in: Duration, every: Duration) -> Duration {
    due_in.clamp(RECHECK, every.max(RECHECK))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_waits_at_least_a_minute_and_at_most_the_interval() {
        let every = Duration::from_secs(240 * 60);
        assert_eq!(wait_after(Duration::ZERO, every), RECHECK);
        assert_eq!(wait_after(Duration::from_secs(1), every), RECHECK);
        let soon = Duration::from_secs(10 * 60);
        assert_eq!(wait_after(soon, every), soon);
        assert_eq!(wait_after(Duration::from_secs(300 * 60), every), every);
        // An interval below RECHECK (only set in code) still waits RECHECK.
        assert_eq!(wait_after(Duration::ZERO, Duration::from_secs(1)), RECHECK);
    }
}
