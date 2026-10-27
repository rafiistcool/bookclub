//! `bookclub-server rematch-legacy` (legacy-mapping.md §0, §3.3 step 5): re-resolves every
//! `source='legacy'` edition that has an ISBN through `CatalogLookup::resolve_isbn_remote`.
//!
//! The lookup persists a hit through ingest, which finds the bare-facts edition by its ISBN and
//! overwrites its `legacy`-ranked fields (and its work's title and authors) with the source
//! record; the edition's and work's `source` become the source's. Shelf entries and quotes keep
//! pointing at the same work and edition. `legacy_work_keys.match` of the upgraded edition becomes
//! `isbn_matched`.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use super::books::DEFAULT_RETRY_WAIT;
use super::report::ReportBuilder;
use super::{ImportError, RematchArgs, Report};
use crate::catalog::shared::isbn::parse_isbn;
use crate::catalog::shared::lookup::{CatalogLookup, LookupError};

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
}

/// Re-resolves the legacy bare-facts editions by ISBN, in one transaction (rolled back with
/// `--dry-run`). A throttled lookup is waited out and retried; an unavailable source leaves the
/// edition as it is and is reported.
pub async fn rematch(
    db: PgPool,
    lookup: Arc<dyn CatalogLookup>,
    args: RematchArgs,
) -> Result<Report, ImportError> {
    let wait = args
        .retry_wait_ms
        .map_or(DEFAULT_RETRY_WAIT, Duration::from_millis);
    let mut tx = db.begin().await?;
    let editions = sqlx::query!(
        r#"SELECT id, isbn13 AS "isbn13!" FROM editions
           WHERE source = 'legacy' AND isbn13 IS NOT NULL
           ORDER BY id"#
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut report = ReportBuilder::default();
    let (mut upgraded, mut elsewhere, mut unmatched, mut errors) = (0u64, 0u64, 0u64, 0u64);
    for edition in &editions {
        let Some(isbn) = parse_isbn(&edition.isbn13) else {
            report.notes.push(format!(
                "edition {}: {:?} is not a valid ISBN; left as it is",
                edition.id, edition.isbn13
            ));
            continue;
        };
        let resolved = loop {
            match lookup.resolve_isbn_remote(&mut tx, &isbn).await {
                Ok(resolved) => break Ok(resolved),
                Err(LookupError::Throttled) => {
                    tracing::info!(isbn = %isbn.as_str(), wait_ms = wait.as_millis() as u64,
                        "catalog lookup throttled; waiting");
                    tokio::time::sleep(wait).await;
                    report.lookup_wait += wait;
                }
                Err(LookupError::Unavailable) => break Err(()),
                Err(LookupError::Db(e)) => return Err(ImportError::Db(e)),
            }
        };
        let outcome = match resolved {
            Err(()) => Outcome::Error,
            Ok(None) => Outcome::Unmatched,
            Ok(Some(_)) => {
                let now = sqlx::query!(
                    "SELECT e.source, w.title FROM editions e JOIN works w ON w.id = e.work_id
                     WHERE e.id = $1",
                    edition.id
                )
                .fetch_one(&mut *tx)
                .await?;
                if now.source == "legacy" {
                    Outcome::Elsewhere
                } else {
                    sqlx::query!(
                        "UPDATE legacy_work_keys SET match = 'isbn_matched'
                         WHERE edition_id = $1 AND match IN ('isbn_unmatched', 'isbn_error')",
                        edition.id
                    )
                    .execute(&mut *tx)
                    .await?;
                    Outcome::Upgraded {
                        title: now.title,
                        source: now.source,
                    }
                }
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
             {elsewhere} under another work)"
        ),
    );

    if args.dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(Report::from_rematch(report, args.dry_run))
}
