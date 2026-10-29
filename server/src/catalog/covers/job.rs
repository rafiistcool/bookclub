//! The background cover search (docs/phase2/notes/cover.md §4.4): every 10 minutes, under its
//! own advisory lock, one run:
//!
//! 1. closes moot suggestions;
//! 2. queues never-fetched Open Library pictures of books readers use for the prefetch;
//! 3. runs the dead check (§1.1);
//! 4. searches up to 30 due candidates: books readers use first (tier 1), never searched first,
//!    then the oldest due date, then the work id.
//!
//! One Open Library request every 2 seconds. Throttled ends the run with nothing recorded (the
//! next run resumes); unavailable records the failure on that work, ends the run and opens the
//! [`Breaker`], which the loop in `jobs.rs` keeps between runs. The shutdown token is checked
//! between steps, between works and during every pace sleep.

use std::time::Duration;

use sqlx::PgConnection;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::dead::{self, DeadReport};
use super::search::{Context, Outcome, Pacer, search_one};
use crate::AppState;
use crate::jobs::{lock_ids, run_locked};
use crate::outbound::RetryPolicy;

/// How often the job runs.
pub const COVER_SEARCH_EVERY: Duration = Duration::from_secs(600);
/// Works searched per run at most.
pub const COVER_SEARCH_BATCH: i64 = 30;
/// The wait between two Open Library requests of the job (a sixth of the shared 3 a second).
pub const COVER_SEARCH_PACE: Duration = Duration::from_secs(2);
/// Failing pictures fetched again per run at most.
pub const COVER_RECHECK_BATCH: i64 = 20;
/// Never-fetched pictures of books in use queued for the prefetch per run at most.
pub const COVER_PREFETCH_BATCH: i64 = 100;
/// The breaker's first pause and its longest.
pub const BREAKER_FIRST: Duration = Duration::from_secs(600);
pub const BREAKER_MAX: Duration = Duration::from_secs(6 * 3600);

/// The job's knobs; tests shrink the pace.
#[derive(Debug, Clone)]
pub struct JobSettings {
    pub batch: i64,
    pub pace: Duration,
    pub recheck_batch: i64,
    pub prefetch_batch: i64,
    /// The policy of the dead check's picture fetches.
    pub media_policy: RetryPolicy,
}

impl Default for JobSettings {
    fn default() -> Self {
        JobSettings {
            batch: COVER_SEARCH_BATCH,
            pace: COVER_SEARCH_PACE,
            recheck_batch: COVER_RECHECK_BATCH,
            prefetch_batch: COVER_PREFETCH_BATCH,
            media_policy: RetryPolicy::MEDIA,
        }
    }
}

/// What one run did (counts and flags; the log never names a title).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunReport {
    pub closed: u64,
    pub prefetched: u64,
    pub dead: DeadReport,
    pub searched: u64,
    pub attached: u64,
    pub inherited: u64,
    pub isbn: u64,
    pub suggested: u64,
    pub uncertain: u64,
    pub no_match: u64,
    pub blocked: u64,
    /// The work ids searched, in order.
    pub works: Vec<i64>,
    /// Open Library answered at least one search.
    pub answered: bool,
    /// The run ended on an unavailable answer.
    pub unavailable: bool,
    /// The run ended on throttling.
    pub throttled: bool,
    /// The shutdown token ended the run.
    pub stopped: bool,
}

/// Pauses the search after unavailable runs (§4.4): after the k-th in a row, 10 minutes ×
/// 2^(k − 1), at most 6 hours. Any answer closes it. It lives in the loop's memory.
#[derive(Debug, Clone, Default)]
pub struct Breaker {
    outages: u32,
    until: Option<Instant>,
}

impl Breaker {
    /// Whether a run may start now.
    pub fn allows(&self, now: Instant) -> bool {
        self.until.is_none_or(|until| now >= until)
    }

    /// Learns from a finished run.
    pub fn record(&mut self, run: &RunReport, now: Instant) {
        if run.unavailable {
            self.outages += 1;
            let factor = 2u32.saturating_pow(self.outages.saturating_sub(1).min(16));
            let pause = BREAKER_FIRST.saturating_mul(factor).min(BREAKER_MAX);
            self.until = Some(now + pause);
        } else if run.answered {
            self.outages = 0;
            self.until = None;
        }
    }

    /// Unavailable runs in a row.
    pub fn outages(&self) -> u32 {
        self.outages
    }
}

/// Closes the open suggestions nobody needs to decide any more (`bc_cover_suggestion_moot`).
pub async fn close_moot(conn: &mut PgConnection) -> sqlx::Result<u64> {
    Ok(sqlx::query!(
        "UPDATE cover_suggestions SET status = 'closed', decided_at = now()
         WHERE status = 'open' AND bc_cover_suggestion_moot(id)"
    )
    .execute(conn)
    .await?
    .rows_affected())
}

/// Never-fetched Open Library pictures of the works and editions readers use.
async fn unfetched_in_use(conn: &mut PgConnection, limit: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT m.id FROM media m
           WHERE m.source = 'openlibrary' AND m.storage_key IS NULL AND m.fetch_failed_at IS NULL
             AND m.removed_at IS NULL AND m.ol_gone_at IS NULL
             AND (EXISTS (SELECT 1 FROM works w
                          WHERE w.cover_media_id = m.id AND bc_cover_in_use(w.id))
                  OR EXISTS (SELECT 1 FROM editions e
                             WHERE e.cover_media_id = m.id AND bc_cover_in_use(e.work_id)))
           ORDER BY m.id
           LIMIT $1"#,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The due candidates (notes §4.4 step 4): tier 1 (in use) before tier 2 (DNB and legacy works
/// nobody uses, not DNB works whose every edition is still to come), never searched first, then
/// the due date, then the id. Due: never searched, `next_search_at` passed, the title or first
/// author changed, or an `uncertain` book that is in use now; tier 2 only below 2 misses.
pub async fn due_works(conn: &mut PgConnection, limit: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"WITH candidates AS (
               SELECT w.id, w.source, w.match_key, bc_cover_in_use(w.id) AS in_use,
                      cs.work_id IS NULL AS never, cs.next_search_at, cs.misses, cs.searched_key,
                      cs.outcome
               FROM works w
               LEFT JOIN cover_search cs ON cs.work_id = w.id
               WHERE w.merged_into_id IS NULL AND w.source <> 'user'
                 AND bc_cover_missing(w.cover_media_id)
                 AND NOT COALESCE(w.cover_media_id IS NULL
                                  AND w.field_sources ->> 'cover_media_id' = 'admin', false)
                 AND NOT EXISTS (SELECT 1 FROM catalog_edit_requests r
                                 WHERE r.work_id = w.id AND r.edition_id IS NULL
                                   AND r.kind = 'cover_replace' AND r.status = 'open')
                 AND NOT EXISTS (SELECT 1 FROM cover_suggestions s
                                 WHERE s.work_id = w.id AND s.status = 'open')
           ),
           keyed AS (
               SELECT c.*, (c.searched_key IS NOT NULL
                            AND c.searched_key IS DISTINCT FROM c.match_key) AS key_changed
               FROM candidates c
           )
           SELECT id AS "id!" FROM keyed k
           WHERE (k.in_use
                  OR (k.source IN ('dnb', 'legacy')
                      AND NOT (k.source = 'dnb'
                               AND EXISTS (SELECT 1 FROM editions e WHERE e.work_id = k.id)
                               AND NOT EXISTS (
                                   SELECT 1 FROM editions e
                                   WHERE e.work_id = k.id
                                     AND (e.planned_publication_date IS NULL
                                          OR e.planned_publication_date <= current_date)))
                      AND (k.never OR k.key_changed OR k.misses < 2)))
             AND (k.never OR k.key_changed OR k.next_search_at IS NULL
                  OR k.next_search_at <= now() OR (k.outcome = 'uncertain' AND k.in_use))
           ORDER BY k.in_use DESC, k.never DESC, k.next_search_at NULLS FIRST, k.id
           LIMIT $1"#,
        limit
    )
    .fetch_all(conn)
    .await
}

/// One run of the job on `conn` (which holds the job's advisory lock).
pub async fn run(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &CancellationToken,
    settings: &JobSettings,
) -> anyhow::Result<RunReport> {
    let mut report = RunReport {
        closed: close_moot(conn).await?,
        ..RunReport::default()
    };
    let unfetched = unfetched_in_use(conn, settings.prefetch_batch).await?;
    report.prefetched = unfetched.len() as u64;
    state.media_service.prefetch(&unfetched);

    let mut pacer = Pacer::new(settings.pace, Some(shutdown.clone()));
    let rows = dead::due_rows(conn, settings.recheck_batch).await?;
    let mut probe = None;
    report.dead =
        dead::recheck(state, &rows, &mut pacer, &mut probe, settings.media_policy).await?;
    if pacer.cancelled() {
        report.stopped = true;
        return Ok(report);
    }

    let due = due_works(conn, settings.batch).await?;
    let mut attached_media = Vec::new();
    for work_id in due {
        if pacer.cancelled() {
            report.stopped = true;
            break;
        }
        let mut ctx = Context {
            lookup: state.lookup.as_ref(),
            pacer: &mut pacer,
            always_suggest: false,
            forced: false,
        };
        let outcome = search_one(conn, &mut ctx, work_id).await?;
        if outcome.answered() {
            report.answered = true;
        }
        match outcome {
            Outcome::Attached { media_id } => {
                report.attached += 1;
                attached_media.push(media_id);
            }
            Outcome::Isbn { media_id } => {
                report.isbn += 1;
                attached_media.push(media_id);
            }
            Outcome::Inherited => report.inherited += 1,
            Outcome::Suggested { .. } => report.suggested += 1,
            Outcome::Uncertain => report.uncertain += 1,
            Outcome::NoMatch => report.no_match += 1,
            Outcome::Blocked => report.blocked += 1,
            Outcome::Skipped => continue,
            Outcome::Stopped => {
                report.stopped = true;
                break;
            }
            Outcome::Throttled => {
                report.throttled = true;
                break;
            }
            Outcome::Unavailable => {
                report.searched += 1;
                report.works.push(work_id);
                report.unavailable = true;
                break;
            }
        }
        report.searched += 1;
        report.works.push(work_id);
    }
    state.media_service.prefetch(&attached_media);
    tracing::info!(
        closed = report.closed,
        prefetched = report.prefetched,
        rechecked = report.dead.rechecked,
        gone = report.dead.gone,
        searched = report.searched,
        attached = report.attached,
        isbn = report.isbn,
        suggested = report.suggested,
        no_match = report.no_match,
        failed = report.unavailable,
        stopped = report.stopped || report.throttled,
        "cover search"
    );
    Ok(report)
}

/// One run under the job's advisory lock: `None` when another session holds it (a second
/// instance) or the run failed.
pub async fn run_once(
    state: &AppState,
    shutdown: &CancellationToken,
    settings: &JobSettings,
) -> Option<RunReport> {
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (job_state, token, job_settings, out) = (
        state.clone(),
        shutdown.clone(),
        settings.clone(),
        result.clone(),
    );
    let ran = run_locked(
        &state.db,
        lock_ids::COVER_SEARCH,
        "cover search",
        move |conn| {
            Box::pin(async move {
                let report = run(&job_state, conn, &token, &job_settings).await?;
                *out.lock().unwrap() = Some(report);
                Ok(())
            })
        },
    )
    .await;
    let report = result.lock().unwrap().take();
    if ran { report } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breaker_backs_off_and_any_answer_closes_it() {
        let start = Instant::now();
        let mut breaker = Breaker::default();
        assert!(breaker.allows(start));
        let down = RunReport {
            unavailable: true,
            ..RunReport::default()
        };
        breaker.record(&down, start);
        assert!(!breaker.allows(start + Duration::from_secs(599)));
        assert!(breaker.allows(start + Duration::from_secs(600)));
        breaker.record(&down, start);
        assert!(!breaker.allows(start + Duration::from_secs(1199)));
        assert!(breaker.allows(start + Duration::from_secs(1200)));
        for _ in 0..10 {
            breaker.record(&down, start);
        }
        assert!(breaker.allows(start + BREAKER_MAX), "at most 6 hours");
        assert!(!breaker.allows(start + BREAKER_MAX - Duration::from_secs(1)));
        // A run without any request leaves it as it is; an answer closes it.
        breaker.record(&RunReport::default(), start);
        assert_eq!(breaker.outages(), 12);
        let answered = RunReport {
            answered: true,
            ..RunReport::default()
        };
        breaker.record(&answered, start);
        assert_eq!(breaker.outages(), 0);
        assert!(breaker.allows(start));
    }
}
