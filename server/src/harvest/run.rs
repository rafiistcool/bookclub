//! One harvest of one OAI set, and its cursor in `harvest_state` (work-plan HAR).
//!
//! - **Start.** A set with no run in progress starts a run: `run_started_at` = now, `run_from` =
//!   `harvested_until`, or now − `DNB_HARVEST_START_DAYS` the first time.
//! - **Pages.** Each page's ingests, the next resumption token (with `token_issued_at`) and
//!   `records_seen` are committed in one transaction, so a crash resumes after the last
//!   committed page and never skips one. A page transaction Postgres aborts with a deadlock is
//!   rolled back and tried again ([`PAGE_ATTEMPTS`] in all), then the run fails.
//! - **Restart.** `badResumptionToken`, or a stored token older than [`TOKEN_MAX_AGE`], clears the
//!   token and restarts the run from `run_from`.
//! - **End.** The last page (no token, or `noRecordsMatch`) also writes `harvested_until` = the
//!   run's start − [`OVERLAP`] and ends the run.
//! - **Failure.** The cursor stays where the last committed page left it, and `last_error` says
//!   why; the next run resumes or restarts the same run.
//!
//! Records: a deleted header withdraws its record's editions, every ISBN
//! (`ingest::withdraw_dnb_edition`), and is never mapped; every other record goes through
//! `dnb_marc::map_record` and `ingest::upsert_dnb_record(…, Mode::Replace)`, then locks the works
//! it landed on against a merge in flight ([`MERGED_RETRIES`]). A record the mapping refuses, or
//! one whose ingest fails with a data error ([`refuses_record`]), is skipped with a log line. Any
//! other error (a deadlock, a cancelled statement, a lock timeout, a lost connection) fails the
//! page, so the record is tried again.

use std::time::Duration;

use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;

use super::oai::{self, Answer, OaiRecord, Page};
use crate::AppState;
use crate::catalog::shared::dnb_marc::map_record;
use crate::catalog::shared::ingest::{self, Mode};
use crate::config::Config;
use crate::error::is_deadlock;
use crate::outbound::{RetryPolicy, UpstreamError, get_bytes};
use crate::text::truncate_chars;

/// A stored resumption token older than this is not used: the run restarts from `run_from`.
pub const TOKEN_MAX_AGE: Duration = Duration::from_secs(6 * 3600);
/// `harvested_until` is the run's start minus this, so the next run also asks again for records
/// the DNB stamped while this one ran.
pub const OVERLAP: Duration = Duration::from_secs(3600);
/// A page transaction aborted by a deadlock is tried this often in all, then the run fails.
pub const PAGE_ATTEMPTS: u32 = 3;
/// How often one run restarts from `run_from` after `badResumptionToken` before it fails.
pub const MAX_RESTARTS: u32 = 3;
/// Harvest runs are never closer together than this, whatever `DNB_HARVEST_INTERVAL_MINUTES`
/// says (the configuration refuses less, too).
pub const MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// After a failure, a run in progress whose resumption token is younger than [`TOKEN_MAX_AGE`]
/// tries again after this (or after the interval, when that is shorter), so it resumes from its
/// token before the token goes stale. Every other failure waits a whole interval.
pub const RESUME_RETRY: Duration = MIN_INTERVAL;
/// How often a record starts over when a work it landed on was merged away meanwhile.
pub const MERGED_RETRIES: u32 = 2;
/// Longest `last_error` stored.
const MAX_ERROR_CHARS: usize = 1000;

/// The `harvest_state.source` of an OAI set: `"dnb:"` + the setSpec.
pub fn state_key(set: &str) -> String {
    format!("dnb:{set}")
}

/// `DNB_HARVEST_INTERVAL_MINUTES`, never below [`MIN_INTERVAL`].
pub fn interval(config: &Config) -> Duration {
    config.dnb_harvest_interval.max(MIN_INTERVAL)
}

/// What one call of [`harvest_set`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SetReport {
    /// Pages committed.
    pub pages: u32,
    /// Records on those pages, deleted ones included.
    pub records: u64,
    /// Records ingested through `upsert_dnb_record`.
    pub ingested: u64,
    /// Deleted headers (each withdraws its record's editions, if there are any).
    pub withdrawn: u64,
    /// Records skipped: refused by the mapping, refused by ingest, or without metadata or IDN.
    pub skipped: u64,
    /// Restarts from `run_from` after `badResumptionToken` or a stale stored token.
    pub restarts: u32,
    /// The run reached the end of the list. False when `shutdown` stopped it between pages.
    pub completed: bool,
}

impl SetReport {
    fn add(&mut self, page: &SetReport) {
        self.records += page.records;
        self.ingested += page.ingested;
        self.withdrawn += page.withdrawn;
        self.skipped += page.skipped;
    }
}

/// Why a harvest of a set failed. The cursor keeps the last committed page.
#[derive(Debug, thiserror::Error)]
pub enum HarvestError {
    #[error("DNB OAI-PMH request failed: {0}")]
    Upstream(#[from] UpstreamError),
    #[error("DNB OAI-PMH error {code}: {message}")]
    Oai { code: String, message: String },
    #[error("{0}")]
    Envelope(String),
    #[error("the DNB rejected the resumption token {0} times in one run")]
    TokenRejected(u32),
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

/// Harvests one OAI set: resumes the run in progress, or starts one, and pages through the list
/// until its end or until `shutdown` (checked between pages). On failure `last_error` and
/// `last_error_at` are recorded. Takes no advisory lock: [`super::run_forever`] runs it under
/// `jobs::lock_ids::DNB_HARVEST`; tests call it directly.
pub async fn harvest_set(
    state: &AppState,
    conn: &mut PgConnection,
    set: &str,
    shutdown: &CancellationToken,
) -> Result<SetReport, HarvestError> {
    let source = state_key(set);
    let result = run_set(state, conn, set, &source, shutdown).await;
    match &result {
        Ok(report) => tracing::info!(
            set,
            pages = report.pages,
            records = report.records,
            ingested = report.ingested,
            withdrawn = report.withdrawn,
            skipped = report.skipped,
            restarts = report.restarts,
            completed = report.completed,
            "dnb harvest: set done"
        ),
        Err(error) => {
            tracing::warn!(set, %error, "dnb harvest: set failed");
            let message = error.to_string();
            if let Err(db_error) = sqlx::query!(
                "UPDATE harvest_state SET last_error = $2, last_error_at = now() WHERE source = $1",
                source,
                truncate_chars(&message, MAX_ERROR_CHARS)
            )
            .execute(&mut *conn)
            .await
            {
                tracing::error!(set, error = %db_error, "dnb harvest: cannot record the failure");
            }
        }
    }
    result
}

/// Where a run continues.
struct Cursor {
    run_from: OffsetDateTime,
    token: Option<String>,
}

async fn run_set(
    state: &AppState,
    conn: &mut PgConnection,
    set: &str,
    source: &str,
    shutdown: &CancellationToken,
) -> Result<SetReport, HarvestError> {
    let mut report = SetReport::default();
    let cursor = start_or_resume(conn, source, &state.config, &mut report).await?;
    let base = &state.outbound.urls.dnb_oai;
    let mut token = cursor.token;
    let mut rejected = 0;
    loop {
        if shutdown.is_cancelled() {
            return Ok(report);
        }
        let url = match &token {
            Some(token) => oai::resume_url(base, token),
            None => oai::list_records_url(base, set, cursor.run_from),
        };
        // The DNB OAI throttle spaces requests at least a second apart (outbound.rs). A
        // shutdown during the request stops at once: the cursor is at the last committed page.
        let answer = tokio::select! {
            () = shutdown.cancelled() => return Ok(report),
            answer = get_bytes(&state.outbound, &state.outbound.dnb_oai, url, &RetryPolicy::JOB) => answer?,
        };
        let xml = std::str::from_utf8(&answer.body).map_err(|error| {
            HarvestError::Envelope(format!("OAI-PMH page is not UTF-8: {error}"))
        })?;
        let page = match oai::parse_list_records(xml).map_err(HarvestError::Envelope)? {
            Answer::Page(page) => page,
            Answer::NoRecordsMatch => Page {
                records: Vec::new(),
                resumption_token: None,
            },
            Answer::BadResumptionToken(message) if token.is_some() => {
                rejected += 1;
                if rejected > MAX_RESTARTS {
                    return Err(HarvestError::TokenRejected(rejected));
                }
                report.restarts += 1;
                tracing::warn!(
                    set,
                    message,
                    "dnb harvest: token rejected, restarting the run"
                );
                restart(conn, source).await?;
                token = None;
                continue;
            }
            Answer::BadResumptionToken(message) => {
                return Err(HarvestError::Oai {
                    code: "badResumptionToken".to_owned(),
                    message,
                });
            }
            Answer::Error { code, message } => return Err(HarvestError::Oai { code, message }),
        };
        if page.resumption_token.is_some() && page.resumption_token == token {
            // Paging would never end; the next run tries again from the same cursor.
            return Err(HarvestError::Envelope(
                "the DNB sent the same resumption token again".to_owned(),
            ));
        }
        let counts = commit_page(conn, source, &page).await?;
        report.add(&counts);
        report.pages += 1;
        match page.resumption_token {
            Some(next) => token = Some(next),
            None => {
                report.completed = true;
                return Ok(report);
            }
        }
    }
}

/// The run in progress (its token when still fresh, else a restart from its `run_from`), or a
/// new run from `harvested_until` (the first: now − `DNB_HARVEST_START_DAYS`).
async fn start_or_resume(
    conn: &mut PgConnection,
    source: &str,
    config: &Config,
    report: &mut SetReport,
) -> sqlx::Result<Cursor> {
    let mut tx = conn.begin().await?;
    sqlx::query!(
        "INSERT INTO harvest_state (source) VALUES ($1) ON CONFLICT (source) DO NOTHING",
        source
    )
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query!(
        r#"SELECT run_from, resumption_token,
                  token_issued_at > now() - make_interval(secs => $2) AS "token_fresh"
           FROM harvest_state
           WHERE source = $1 AND run_started_at IS NOT NULL
           FOR UPDATE"#,
        source,
        TOKEN_MAX_AGE.as_secs_f64()
    )
    .fetch_optional(&mut *tx)
    .await?;

    let in_progress =
        row.and_then(|row| Some((row.run_from?, row.resumption_token, row.token_fresh)));
    let cursor = match in_progress {
        Some((run_from, token, token_fresh)) => match token {
            Some(token) if token_fresh == Some(true) => Cursor {
                run_from,
                token: Some(token),
            },
            stale => {
                if stale.is_some() {
                    tracing::warn!(
                        source,
                        "dnb harvest: stored token too old, restarting the run"
                    );
                    report.restarts += 1;
                }
                restart(&mut tx, source).await?;
                Cursor {
                    run_from,
                    token: None,
                }
            }
        },
        None => {
            let days = i32::try_from(config.dnb_harvest_start_days).unwrap_or(i32::MAX);
            let run_from = sqlx::query_scalar!(
                r#"UPDATE harvest_state
                   SET run_started_at = now(),
                       run_from = COALESCE(harvested_until, now() - make_interval(days => $2)),
                       resumption_token = NULL, token_issued_at = NULL, records_seen = 0
                   WHERE source = $1
                   RETURNING run_from AS "run_from!""#,
                source,
                days
            )
            .fetch_one(&mut *tx)
            .await?;
            Cursor {
                run_from,
                token: None,
            }
        }
    };
    tx.commit().await?;
    Ok(cursor)
}

/// Clears the token: the run starts again from its `run_from`.
async fn restart(conn: &mut PgConnection, source: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE harvest_state
         SET resumption_token = NULL, token_issued_at = NULL, records_seen = 0
         WHERE source = $1",
        source
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// One page in one transaction, tried again after a deadlock.
async fn commit_page(
    conn: &mut PgConnection,
    source: &str,
    page: &Page,
) -> sqlx::Result<SetReport> {
    let mut attempt = 1;
    loop {
        match try_commit_page(conn, source, page).await {
            Err(error) if is_deadlock(&error) && attempt < PAGE_ATTEMPTS => {
                tracing::warn!(
                    source,
                    attempt,
                    "dnb harvest: page deadlocked, trying again"
                );
                attempt += 1;
            }
            result => return result,
        }
    }
}

async fn try_commit_page(
    conn: &mut PgConnection,
    source: &str,
    page: &Page,
) -> sqlx::Result<SetReport> {
    let mut tx = conn.begin().await?;
    let mut counts = SetReport::default();
    for record in &page.records {
        apply_record(&mut tx, record, &mut counts).await?;
    }
    let seen = i64::try_from(counts.records).unwrap_or(i64::MAX);
    match &page.resumption_token {
        Some(token) => {
            sqlx::query!(
                "UPDATE harvest_state
                 SET resumption_token = $2, token_issued_at = now(),
                     records_seen = records_seen + $3
                 WHERE source = $1",
                source,
                token,
                seen
            )
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query!(
                "UPDATE harvest_state
                 SET resumption_token = NULL, token_issued_at = NULL,
                     records_seen = records_seen + $2,
                     harvested_until = COALESCE(run_started_at, now()) - make_interval(secs => $3),
                     last_success_at = now(), last_error = NULL, last_error_at = NULL,
                     run_started_at = NULL, run_from = NULL
                 WHERE source = $1",
                source,
                seen,
                OVERLAP.as_secs_f64()
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(counts)
}

async fn apply_record(
    tx: &mut PgConnection,
    record: &OaiRecord,
    counts: &mut SetReport,
) -> sqlx::Result<()> {
    counts.records += 1;
    let identifier = record.identifier.as_str();
    if record.deleted {
        match record.idn() {
            Some(idn) => {
                ingest::withdraw_dnb_edition(tx, idn).await?;
                counts.withdrawn += 1;
            }
            None => {
                tracing::warn!(identifier, "dnb harvest: deleted header without an IDN");
                counts.skipped += 1;
            }
        }
        return Ok(());
    }
    let Some(marc) = record.metadata.as_deref() else {
        tracing::warn!(identifier, "dnb harvest: record without metadata");
        counts.skipped += 1;
        return Ok(());
    };
    let mapped = match map_record(marc) {
        Ok(mapped) => mapped,
        Err(error) => {
            tracing::warn!(identifier, %error, "dnb harvest: skipping a record");
            counts.skipped += 1;
            return Ok(());
        }
    };
    // A savepoint per record: a record ingest refuses is skipped, not the page (which would
    // fail every later run at the same place). Any other error fails the page; a deadlock
    // retries it as a whole.
    let mut attempt = 0;
    loop {
        let mut savepoint = tx.begin().await?;
        let ids = match ingest::upsert_dnb_record(&mut savepoint, &mapped, Mode::Replace).await {
            Ok(ids) => ids,
            Err(error) if refuses_record(&error) => {
                savepoint.rollback().await?;
                tracing::error!(identifier, %error, "dnb harvest: ingest refused a record");
                counts.skipped += 1;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !lock_current_works(&mut savepoint, &ids).await? {
            if attempt < MERGED_RETRIES {
                // The record follows the merge on its next try (ingest skips merged-away works).
                savepoint.rollback().await?;
                attempt += 1;
                continue;
            }
            tracing::warn!(
                identifier,
                "dnb harvest: a work of the record was merged away during every try"
            );
        }
        savepoint.commit().await?;
        counts.ingested += 1;
        return Ok(());
    }
}

/// Locks the works a record's editions landed on `FOR NO KEY UPDATE`, in id order, and says
/// whether none of them was merged away (CAT's rule for writes that store a `work_id`,
/// `catalog::custom::insert_user_edition`). A merge in flight cannot see an edition inserted
/// meanwhile when it moves the loser's editions: the lock waits for it, and a merged-away work
/// sends the record round again. A merge that starts later waits for the page and moves the
/// edition itself. The lock comes after ingest's advisory and edition locks, in ingest's order.
async fn lock_current_works(conn: &mut PgConnection, ids: &[(i64, i64)]) -> sqlx::Result<bool> {
    let mut works: Vec<i64> = ids.iter().map(|&(work_id, _)| work_id).collect();
    works.sort_unstable();
    works.dedup();
    let current = sqlx::query_scalar!(
        r#"SELECT merged_into_id IS NULL AS "current!" FROM works WHERE id = ANY($1)
           ORDER BY id FOR NO KEY UPDATE"#,
        &works
    )
    .fetch_all(conn)
    .await?;
    Ok(current.len() == works.len() && current.into_iter().all(|current| current))
}

/// Whether an ingest error is a property of the record, which every later try would meet again,
/// so the record is skipped: a data exception (SQLSTATE class 22), an integrity constraint
/// violation (class 23), a value sqlx cannot decode, ingest's own refusal (`Protocol`), or a row
/// that ingest's statements should have found and did not (`RowNotFound`; works and editions are
/// never deleted, and the record's advisory locks keep concurrent ingests out). Everything else
/// fails the page and keeps the cursor: a deadlock (40P01, the page is retried), a cancelled
/// statement (57014), a lock timeout (55P03), a lost connection or a full pool.
pub fn refuses_record(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|code| code.starts_with("22") || code.starts_with("23")),
        sqlx::Error::Protocol(_)
        | sqlx::Error::RowNotFound
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnDecode { .. } => true,
        _ => false,
    }
}

/// How long a set waits after a failure: [`RESUME_RETRY`] (at most `every`) while its run in
/// progress holds a resumption token younger than [`TOKEN_MAX_AGE`], else `every`. The SQL of
/// [`is_due`] and [`next_due_in`] picks between the two.
fn retry_after(every: Duration) -> Duration {
    every.min(RESUME_RETRY)
}

/// Whether the set is due: no failure within its back-off (a whole interval, or
/// [`RESUME_RETRY`] for a run that can still resume from its token), and either a run in
/// progress (interrupted, so it resumes) or no completed run within the interval.
pub async fn is_due(conn: &mut PgConnection, set: &str, every: Duration) -> sqlx::Result<bool> {
    let due = sqlx::query_scalar!(
        r#"SELECT (last_error_at IS NULL
                   OR last_error_at <= now() - make_interval(secs =>
                          CASE WHEN run_started_at IS NOT NULL AND resumption_token IS NOT NULL
                                    AND token_issued_at > now() - make_interval(secs => $4)
                               THEN $3::float8 ELSE $2::float8 END))
              AND (run_started_at IS NOT NULL OR last_success_at IS NULL
                   OR last_success_at <= now() - make_interval(secs => $2)) AS "due!"
           FROM harvest_state WHERE source = $1"#,
        state_key(set),
        every.as_secs_f64(),
        retry_after(every).as_secs_f64(),
        TOKEN_MAX_AGE.as_secs_f64()
    )
    .fetch_optional(conn)
    .await?;
    Ok(due.unwrap_or(true))
}

/// How long until the first of `sets` is due ([`is_due`]); zero when one is due now.
pub async fn next_due_in(
    conn: &mut PgConnection,
    sets: &[String],
    every: Duration,
) -> sqlx::Result<Duration> {
    let mut keys: Vec<String> = sets.iter().map(|set| state_key(set)).collect();
    keys.sort();
    keys.dedup();
    let waits = sqlx::query_scalar!(
        r#"SELECT GREATEST(
                      COALESCE(EXTRACT(EPOCH FROM last_error_at - now() + make_interval(secs =>
                          CASE WHEN run_started_at IS NOT NULL AND resumption_token IS NOT NULL
                                    AND token_issued_at > now() - make_interval(secs => $4)
                               THEN $3::float8 ELSE $2::float8 END)), 0),
                      CASE WHEN run_started_at IS NOT NULL OR last_success_at IS NULL THEN 0
                           ELSE EXTRACT(EPOCH FROM last_success_at + make_interval(secs => $2) - now())
                      END
                  )::float8 AS "wait!"
           FROM harvest_state WHERE source = ANY($1)"#,
        &keys,
        every.as_secs_f64(),
        retry_after(every).as_secs_f64(),
        TOKEN_MAX_AGE.as_secs_f64()
    )
    .fetch_all(conn)
    .await?;
    if waits.len() < keys.len() {
        // A set without a row has never run: due now.
        return Ok(Duration::ZERO);
    }
    let soonest = waits.into_iter().fold(f64::INFINITY, f64::min);
    Ok(Duration::from_secs_f64(if soonest.is_finite() {
        soonest.max(0.0)
    } else {
        0.0
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(minutes: &str) -> Config {
        Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some("postgres://unused".to_owned()),
            "DEV_MODE" => Some("true".to_owned()),
            "DNB_HARVEST_INTERVAL_MINUTES" => Some(minutes.to_owned()),
            _ => None,
        })
        .unwrap()
    }

    #[test]
    fn runs_are_at_least_thirty_minutes_apart() {
        assert_eq!(interval(&config("240")), Duration::from_secs(240 * 60));
        assert_eq!(interval(&config("30")), MIN_INTERVAL);
        // The configuration refuses less; a value set in code is still raised to the floor.
        let mut short = config("30");
        short.dnb_harvest_interval = Duration::from_secs(60);
        assert_eq!(interval(&short), MIN_INTERVAL);
    }

    #[test]
    fn a_resumable_run_retries_before_its_token_goes_stale() {
        assert_eq!(retry_after(Duration::from_secs(240 * 60)), RESUME_RETRY);
        assert_eq!(retry_after(Duration::from_secs(1440 * 60)), RESUME_RETRY);
        assert_eq!(retry_after(MIN_INTERVAL), MIN_INTERVAL);
        assert!(RESUME_RETRY < TOKEN_MAX_AGE);
    }

    #[test]
    fn only_data_errors_skip_a_record() {
        assert!(refuses_record(&sqlx::Error::RowNotFound));
        assert!(refuses_record(&sqlx::Error::Protocol("refused".into())));
        assert!(!refuses_record(&sqlx::Error::PoolTimedOut));
        assert!(!refuses_record(&sqlx::Error::Io(std::io::Error::other(
            "connection reset"
        ))));
    }

    #[test]
    fn state_rows_are_named_after_the_set() {
        assert_eq!(state_key("dnb-all:reiheN"), "dnb:dnb-all:reiheN");
        assert_eq!(state_key("dnb:reiheA"), "dnb:dnb:reiheA");
    }
}
