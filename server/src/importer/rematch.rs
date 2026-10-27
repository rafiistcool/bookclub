//! `bookclub-server rematch-legacy` (legacy-mapping.md §0, §3.3 step 5): re-resolves every
//! `source='legacy'` edition that has an ISBN through `CatalogLookup::resolve_isbn_remote`.
//!
//! The lookup persists a hit through ingest, which finds the bare-facts edition by its ISBN and
//! overwrites its `legacy`-ranked fields (and its work's title and authors) with the source
//! record; the edition's and work's `source` become the source's. Shelf entries and quotes keep
//! pointing at the same work and edition. `legacy_work_keys.match` of the upgraded edition becomes
//! `isbn_matched`.
//!
//! Unlike `import-legacy`, this runs on the live server, so every edition gets its own short
//! transaction: ingest's advisory and row locks are held for one edition's lookup only, never
//! across another lookup or a throttle wait, and one edition's failure does not undo the others.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use super::books::DEFAULT_RETRY_WAIT;
use super::report::ReportBuilder;
use super::{ImportError, RematchArgs, Report};
use crate::catalog::shared::isbn::{Isbn13, parse_isbn};
use crate::catalog::shared::lookup::{CatalogLookup, LookupError};
use crate::error::is_deadlock;

/// How often one edition's transaction is retried after Postgres broke a deadlock (40P01) with a
/// concurrent ingest (conventions.md §7.2).
const DEADLOCK_RETRIES: u32 = 2;

/// What one legacy edition's lookup did.
enum Outcome {
    /// The source knew the ISBN and upgraded the edition.
    Upgraded {
        title: String,
        source: String,
    },
    /// The source knew the ISBN under another work; the edition stays as it is (the two works
    /// can be merged through the queue).
    Elsewhere,
    Unmatched,
    Error,
    /// A database error (or deadlocks beyond the retries) rolled this edition back.
    Failed(String),
}

/// One try at one edition.
enum Attempt {
    Done(Outcome),
    /// Rolled back; wait out the cooldown, then try again in a new transaction.
    Throttled,
}

/// Re-resolves the legacy bare-facts editions by ISBN, each in its own transaction (rolled back
/// with `--dry-run`). A throttled lookup is waited out outside any transaction and retried; an
/// unavailable source or a database error leaves that edition as it is and is reported; a
/// deadlock is retried. Only a failure to reach the database at all aborts the run (the editions
/// already upgraded stay upgraded; a rerun continues with the rest).
pub async fn rematch(
    db: PgPool,
    lookup: Arc<dyn CatalogLookup>,
    args: RematchArgs,
) -> Result<Report, ImportError> {
    let wait = args
        .retry_wait_ms
        .map_or(DEFAULT_RETRY_WAIT, Duration::from_millis);
    let editions = sqlx::query!(
        r#"SELECT id, isbn13 AS "isbn13!" FROM editions
           WHERE source = 'legacy' AND isbn13 IS NOT NULL
           ORDER BY id"#
    )
    .fetch_all(&db)
    .await?;

    let mut report = ReportBuilder::default();
    let (mut upgraded, mut elsewhere, mut unmatched, mut errors, mut failed) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for edition in &editions {
        let Some(isbn) = parse_isbn(&edition.isbn13) else {
            report.notes.push(format!(
                "edition {}: {:?} is not a valid ISBN; left as it is",
                edition.id, edition.isbn13
            ));
            continue;
        };
        let mut deadlocks = 0;
        let outcome = loop {
            match attempt(&db, lookup.as_ref(), &isbn, edition.id, args.dry_run).await {
                Ok(Attempt::Done(outcome)) => break outcome,
                Ok(Attempt::Throttled) => {
                    tracing::info!(isbn = %isbn.as_str(), wait_ms = wait.as_millis() as u64,
                        "catalog lookup throttled; waiting");
                    tokio::time::sleep(wait).await;
                    report.lookup_wait += wait;
                }
                Err(error) if is_deadlock(&error) && deadlocks < DEADLOCK_RETRIES => {
                    deadlocks += 1;
                    tracing::warn!(isbn = %isbn.as_str(), %error, "deadlock; retrying the edition");
                }
                Err(sqlx::Error::Database(error)) => {
                    tracing::warn!(isbn = %isbn.as_str(), %error, "the edition was rolled back");
                    break Outcome::Failed(error.to_string());
                }
                Err(error) => return Err(ImportError::Db(error)),
            }
        };
        let line = match outcome {
            Outcome::Upgraded { title, source } => {
                upgraded += 1;
                format!("upgraded from {source}: \"{title}\"")
            }
            Outcome::Elsewhere => {
                elsewhere += 1;
                "known under another work; unchanged (merge the two works through the queue)"
                    .to_owned()
            }
            Outcome::Unmatched => {
                unmatched += 1;
                "unmatched (bare facts)".to_owned()
            }
            Outcome::Error => {
                errors += 1;
                "error: source unavailable (bare facts; retry later)".to_owned()
            }
            Outcome::Failed(error) => {
                failed += 1;
                format!("error: database: {error} (unchanged; retry later)")
            }
        };
        report.notes.push(format!(
            "isbn {} (edition {}): {line}",
            isbn.as_str(),
            edition.id
        ));
    }
    report.entity(
        "legacy editions",
        Some(editions.len() as u64),
        Some(upgraded),
        format!(
            "upgraded ({unmatched} unmatched, {errors} with the source unavailable, \
             {elsewhere} under another work, {failed} with a database error)"
        ),
    );
    Ok(Report::from_rematch(report, args.dry_run))
}

/// Resolves one edition in a transaction of its own and commits it (rolls it back with
/// `dry_run`). A throttled or unavailable lookup rolls back at once, so no lock outlives the
/// call. A database error is returned with the transaction rolled back (dropped).
async fn attempt(
    db: &PgPool,
    lookup: &dyn CatalogLookup,
    isbn: &Isbn13,
    edition_id: i64,
    dry_run: bool,
) -> Result<Attempt, sqlx::Error> {
    let mut tx = db.begin().await?;
    let resolved = match lookup.resolve_isbn_remote(&mut tx, isbn).await {
        Ok(resolved) => resolved,
        Err(LookupError::Throttled) => {
            tx.rollback().await?;
            return Ok(Attempt::Throttled);
        }
        Err(LookupError::Unavailable) => {
            tx.rollback().await?;
            return Ok(Attempt::Done(Outcome::Error));
        }
        Err(LookupError::Db(error)) => return Err(error),
    };
    let outcome = match resolved {
        None => Outcome::Unmatched,
        Some(_) => upgraded(&mut tx, edition_id).await?,
    };
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(Attempt::Done(outcome))
}

/// After a hit: whether ingest upgraded the edition, and if so its key's match.
async fn upgraded(conn: &mut PgConnection, edition_id: i64) -> Result<Outcome, sqlx::Error> {
    let now = sqlx::query!(
        "SELECT e.source, w.title FROM editions e JOIN works w ON w.id = e.work_id
         WHERE e.id = $1",
        edition_id
    )
    .fetch_one(&mut *conn)
    .await?;
    if now.source == "legacy" {
        return Ok(Outcome::Elsewhere);
    }
    sqlx::query!(
        "UPDATE legacy_work_keys SET match = 'isbn_matched'
         WHERE edition_id = $1 AND match IN ('isbn_unmatched', 'isbn_error')",
        edition_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(Outcome::Upgraded {
        title: now.title,
        source: now.source,
    })
}
