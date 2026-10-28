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
//! Records: a deleted header withdraws its edition (`ingest::withdraw_dnb_edition`) and is never
//! mapped; every other record goes through `dnb_marc::map_record` and
//! `ingest::upsert_dnb_record(…, Mode::Replace)`. A record the mapping refuses, or one whose
//! ingest fails with anything but a deadlock, is skipped with a log line, never the page.

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
    /// Deleted headers (each withdraws its edition, if there is one).
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

    let cursor = match row {
        Some(row) if row.run_from.is_some() => {
            let run_from = row.run_from.expect("checked");
            match row.resumption_token {
                Some(token) if row.token_fresh == Some(true) => Cursor {
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
            }
        }
        _ => {
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
    // fail every later run at the same place). A deadlock still fails the page, which is
    // retried as a whole.
    let mut savepoint = tx.begin().await?;
    match ingest::upsert_dnb_record(&mut savepoint, &mapped, Mode::Replace).await {
        Ok(_) => {
            savepoint.commit().await?;
            counts.ingested += 1;
        }
        Err(error) if is_deadlock(&error) => return Err(error),
        Err(error) => {
            savepoint.rollback().await?;
            tracing::error!(identifier, %error, "dnb harvest: ingest refused a record");
            counts.skipped += 1;
        }
    }
    Ok(())
}

/// Whether the set is due: no failure within the interval, and either a run in progress
/// (interrupted, so it resumes) or no completed run within the interval.
pub async fn is_due(conn: &mut PgConnection, set: &str, every: Duration) -> sqlx::Result<bool> {
    let due = sqlx::query_scalar!(
        r#"SELECT (last_error_at IS NULL OR last_error_at <= now() - make_interval(secs => $2))
              AND (run_started_at IS NOT NULL OR last_success_at IS NULL
                   OR last_success_at <= now() - make_interval(secs => $2)) AS "due!"
           FROM harvest_state WHERE source = $1"#,
        state_key(set),
        every.as_secs_f64()
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
                      COALESCE(EXTRACT(EPOCH FROM last_error_at + make_interval(secs => $2) - now()), 0),
                      CASE WHEN run_started_at IS NOT NULL OR last_success_at IS NULL THEN 0
                           ELSE EXTRACT(EPOCH FROM last_success_at + make_interval(secs => $2) - now())
                      END
                  )::float8 AS "wait!"
           FROM harvest_state WHERE source = ANY($1)"#,
        &keys,
        every.as_secs_f64()
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
    fn state_rows_are_named_after_the_set() {
        assert_eq!(state_key("dnb-all:reiheN"), "dnb:dnb-all:reiheN");
        assert_eq!(state_key("dnb:reiheA"), "dnb:dnb:reiheA");
    }
}
