//! The cover search of `import-legacy` and `rematch-legacy` (#74, docs/phase2/notes/cover.md
//! §4.2, §4.3, §6): every imported catalog book without a cover is searched on Open Library,
//! in legacy book order; books readers added are counted, never searched.
//!
//! - **import-legacy**, right after the books, inside the import's transaction, each book in a
//!   savepoint: a database error in one book's step rolls back only that step. The first
//!   unavailable answer, the first book whose throttle waits give up (3 waits) and the time
//!   budget (5 minutes) stop the search; the books after it are "not searched" and have no
//!   `cover_search` row, so the job takes them first once the server runs. A cover is not part
//!   of the data: nothing here makes the import fail.
//! - **rematch-legacy**, after its ISBN pass, each book in a transaction of its own (rolled back
//!   with `--dry-run`), 2 seconds between two Open Library requests (it runs next to the
//!   server's job); throttling is waited out, an unavailable answer stops the pass.
//!
//! The report gets the `cover search` line and the `without a cover` section (with titles: the
//! report file is private).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use sqlx::{Connection, PgConnection, PgPool};

use super::books::DEFAULT_RETRY_WAIT;
use super::report::{CoverSearchLine, ReportBuilder};
use super::{ImportError, Importer, RematchArgs};
use crate::catalog::covers::search::{self, Context, Outcome, Pacer, search_one};
use crate::catalog::shared::lookup::CatalogLookup;
use crate::error::is_deadlock;

/// How long `import-legacy` searches covers at most (it is in the cutover's downtime).
pub(crate) const IMPORT_COVER_BUDGET: Duration = Duration::from_secs(300);
/// Throttle waits per book before the import's search stops.
const MAX_WAITS: u32 = 3;
/// `rematch-legacy`'s wait between two Open Library requests (the job's pace).
pub(crate) const REMATCH_COVER_PACE: Duration = Duration::from_secs(2);
/// Deadlock retries of one book in `rematch-legacy`.
const DEADLOCK_RETRIES: u32 = 2;

/// What became of one imported book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Searched(Outcome),
    /// Open Library did not answer for this book (the search stopped here).
    Unavailable,
    /// The throttle waits gave up (the search stopped here).
    Busy,
    /// Not searched: the search stopped at an earlier book.
    Outage,
    Budget,
    Database,
    Offline,
}

impl Fate {
    /// The `without a cover` text and the bucket it counts in.
    fn describe(self) -> (&'static str, Bucket) {
        match self {
            Fate::Searched(Outcome::Suggested { .. }) => {
                ("suggested, review it on the admin page", Bucket::Suggested)
            }
            Fate::Searched(_) => ("no match on Open Library", Bucket::NoMatch),
            Fate::Unavailable => (
                "not searched (Open Library unavailable)",
                Bucket::NotSearched,
            ),
            Fate::Busy => ("not searched (Open Library busy)", Bucket::NotSearched),
            Fate::Outage => (
                "not searched (stopped after an outage)",
                Bucket::NotSearched,
            ),
            Fate::Budget => (
                "not searched (the time budget ran out)",
                Bucket::NotSearched,
            ),
            Fate::Database => ("not searched (database error)", Bucket::NotSearched),
            Fate::Offline => ("not searched (--offline)", Bucket::NotSearched),
        }
    }

    fn covered(self) -> bool {
        matches!(
            self,
            Fate::Searched(Outcome::Attached { .. } | Outcome::Isbn { .. } | Outcome::Inherited)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Suggested,
    NoMatch,
    NotSearched,
}

/// The first legacy book of each work, in legacy book order: (legacy book id, work id).
fn first_per_work(mut order: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    order.sort_unstable();
    let mut seen = HashSet::new();
    order
        .into_iter()
        .filter(|(_, work)| seen.insert(*work))
        .collect()
}

/// The works among `works` the search would look at (notes §1), in the given order.
async fn candidates(
    conn: &mut PgConnection,
    works: &[(i64, i64)],
) -> sqlx::Result<Vec<(i64, i64)>> {
    let mut out = Vec::new();
    for &(legacy, work) in works {
        if let Some(facts) = search::facts(conn, work).await?
            && facts.candidate(false)
        {
            out.push((legacy, work));
        }
    }
    Ok(out)
}

/// The report's line and section. `missing`: the candidates still without a cover.
async fn summarize(
    conn: &mut PgConnection,
    candidates: &[(i64, i64)],
    fates: &HashMap<i64, Fate>,
    missing: &HashSet<i64>,
    imported: &[i64],
    offline: bool,
) -> sqlx::Result<(CoverSearchLine, Vec<String>)> {
    let work_ids: Vec<i64> = candidates.iter().map(|(_, work)| *work).collect();
    let titles: HashMap<i64, (String, String)> = sqlx::query!(
        "SELECT id, title, author_names FROM works WHERE id = ANY($1)",
        &work_ids
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.id, (row.title, row.author_names)))
    .collect();
    let mut line = CoverSearchLine {
        offline,
        candidates: candidates.len() as u64,
        ..CoverSearchLine::default()
    };
    let mut section = Vec::new();
    for (legacy, work) in candidates {
        if !missing.contains(work) {
            continue;
        }
        line.without += 1;
        let fate = fates.get(work).copied().unwrap_or(Fate::Offline);
        let (text, bucket) = fate.describe();
        match bucket {
            Bucket::Suggested => line.suggested += 1,
            Bucket::NoMatch => line.no_match += 1,
            Bucket::NotSearched => line.not_searched += 1,
        }
        let (title, authors) = titles.get(work).cloned().unwrap_or_default();
        let by = if authors.is_empty() {
            String::new()
        } else {
            format!(" by {authors}")
        };
        section.push(format!("book {legacy} \"{title}\"{by}: {text}"));
    }
    // The pictures the search attached are Open Library pictures not fetched yet too. They are
    // counted from the outcomes, so a rolled-back rematch reports what the real one does.
    let counts = sqlx::query!(
        r#"SELECT
               (SELECT count(*) FROM works
                WHERE id = ANY($1) AND source = 'user' AND bc_cover_missing(cover_media_id))
                   AS "readers!",
               (SELECT count(*) FROM works w JOIN media m ON m.id = w.cover_media_id
                WHERE w.id = ANY($1) AND NOT w.id = ANY($2) AND m.source = 'openlibrary'
                  AND m.storage_key IS NULL AND m.removed_at IS NULL AND m.ol_gone_at IS NULL)
                   AS "unfetched!""#,
        imported,
        &work_ids
    )
    .fetch_one(conn)
    .await?;
    let attached = candidates
        .iter()
        .filter(|(_, work)| {
            matches!(
                fates.get(work),
                Some(Fate::Searched(
                    Outcome::Attached { .. } | Outcome::Isbn { .. }
                ))
            )
        })
        .count() as u64;
    line.readers = counts.readers as u64;
    line.unfetched = counts.unfetched as u64 + attached;
    Ok((line, section))
}

impl Importer {
    /// The cover search of `import-legacy` (see the module docs).
    pub(crate) async fn search_covers(&mut self) -> Result<(), ImportError> {
        let order: Vec<(i64, i64)> = self
            .books
            .iter()
            .map(|(legacy, target)| (*legacy, target.work_id))
            .collect();
        let works = first_per_work(order);
        let imported: Vec<i64> = works.iter().map(|(_, work)| *work).collect();
        let candidates = candidates(&mut self.tx, &works).await?;
        let lookup = self.lookup.clone();
        let budget = self
            .args
            .cover_budget_ms
            .map_or(IMPORT_COVER_BUDGET, Duration::from_millis);
        let wait = self
            .args
            .retry_wait_ms
            .map_or(DEFAULT_RETRY_WAIT, Duration::from_millis);
        let started = Instant::now();
        let mut stopped: Option<Fate> = None;
        let mut fates = HashMap::new();
        for &(_, work) in &candidates {
            let fate = if self.args.offline {
                Fate::Offline
            } else if let Some(fate) = stopped {
                fate
            } else if started.elapsed() >= budget {
                stopped = Some(Fate::Budget);
                Fate::Budget
            } else {
                self.search_in_savepoint(lookup.as_ref(), work, wait)
                    .await?
            };
            if matches!(fate, Fate::Unavailable | Fate::Busy) {
                stopped = Some(Fate::Outage);
            }
            fates.insert(work, fate);
        }
        let work_ids: Vec<i64> = candidates.iter().map(|(_, work)| *work).collect();
        let missing: HashSet<i64> = sqlx::query_scalar!(
            "SELECT id FROM works WHERE id = ANY($1) AND bc_cover_missing(cover_media_id)",
            &work_ids
        )
        .fetch_all(&mut *self.tx)
        .await?
        .into_iter()
        .collect();
        let offline = self.args.offline;
        let (line, section) = summarize(
            &mut self.tx,
            &candidates,
            &fates,
            &missing,
            &imported,
            offline,
        )
        .await?;
        self.report.covers = Some(line);
        self.report.without_cover = section;
        Ok(())
    }

    /// One book's search in a savepoint of the import's transaction.
    async fn search_in_savepoint(
        &mut self,
        lookup: &dyn CatalogLookup,
        work_id: i64,
        wait: Duration,
    ) -> Result<Fate, ImportError> {
        let mut waits = 0;
        loop {
            let mut savepoint = Connection::begin(&mut *self.tx).await?;
            let mut pacer = Pacer::none();
            let mut ctx = Context {
                lookup,
                pacer: &mut pacer,
                always_suggest: true,
                forced: false,
            };
            match search_one(&mut savepoint, &mut ctx, work_id).await {
                Ok(Outcome::Throttled) => {
                    savepoint.rollback().await?;
                    waits += 1;
                    if waits > MAX_WAITS {
                        return Ok(Fate::Busy);
                    }
                    tracing::info!(
                        work_id,
                        wait_ms = wait.as_millis() as u64,
                        "cover search throttled; waiting"
                    );
                    tokio::time::sleep(wait).await;
                    self.report.lookup_wait += wait;
                }
                // No row for a book that was not searched: the job takes it first.
                Ok(Outcome::Unavailable) => {
                    savepoint.rollback().await?;
                    return Ok(Fate::Unavailable);
                }
                Ok(outcome) => {
                    savepoint.commit().await?;
                    return Ok(Fate::Searched(outcome));
                }
                Err(error) => {
                    tracing::warn!(work_id, %error, "the cover search of an imported book was rolled back");
                    savepoint.rollback().await?;
                    return Ok(Fate::Database);
                }
            }
        }
    }
}

/// The cover search of `rematch-legacy` (see the module docs).
pub(crate) async fn rematch_covers(
    db: &PgPool,
    lookup: &dyn CatalogLookup,
    args: &RematchArgs,
    report: &mut ReportBuilder,
) -> Result<(), ImportError> {
    let wait = args
        .retry_wait_ms
        .map_or(DEFAULT_RETRY_WAIT, Duration::from_millis);
    let pace = args
        .cover_pace_ms
        .map_or(REMATCH_COVER_PACE, Duration::from_millis);
    let mut conn = db.acquire().await?;
    let order: Vec<(i64, i64)> = sqlx::query!(
        "SELECT legacy_book_id, work_id FROM legacy_work_keys ORDER BY legacy_book_id"
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.legacy_book_id, row.work_id))
    .collect();
    let works = first_per_work(order);
    let imported: Vec<i64> = works.iter().map(|(_, work)| *work).collect();
    let candidates = candidates(&mut conn, &works).await?;
    let mut pacer = Pacer::new(pace, None);
    let mut stopped: Option<Fate> = None;
    let mut fates = HashMap::new();
    for &(_, work) in &candidates {
        let fate = match stopped {
            Some(fate) => fate,
            None => {
                rematch_one(
                    &mut conn,
                    lookup,
                    &mut pacer,
                    work,
                    args.dry_run,
                    wait,
                    report,
                )
                .await?
            }
        };
        if matches!(fate, Fate::Unavailable) {
            stopped = Some(Fate::Outage);
        }
        fates.insert(work, fate);
    }
    // Counted from the outcomes, so a dry run reports what the real run does.
    let missing: HashSet<i64> = candidates
        .iter()
        .map(|(_, work)| *work)
        .filter(|work| !fates.get(work).is_some_and(|fate| fate.covered()))
        .collect();
    let (line, section) =
        summarize(&mut conn, &candidates, &fates, &missing, &imported, false).await?;
    report.covers = Some(line);
    report.without_cover = section;
    Ok(())
}

/// One book of `rematch-legacy` in a transaction of its own; throttling is waited out.
async fn rematch_one(
    conn: &mut PgConnection,
    lookup: &dyn CatalogLookup,
    pacer: &mut Pacer,
    work_id: i64,
    dry_run: bool,
    wait: Duration,
    report: &mut ReportBuilder,
) -> Result<Fate, ImportError> {
    let mut deadlocks = 0;
    loop {
        let mut tx = conn.begin().await?;
        let mut ctx = Context {
            lookup,
            pacer: &mut *pacer,
            always_suggest: true,
            forced: false,
        };
        match search_one(&mut tx, &mut ctx, work_id).await {
            Ok(Outcome::Throttled) => {
                tx.rollback().await?;
                tracing::info!(
                    work_id,
                    wait_ms = wait.as_millis() as u64,
                    "cover search throttled; waiting"
                );
                tokio::time::sleep(wait).await;
                report.lookup_wait += wait;
            }
            Ok(Outcome::Unavailable) => {
                if dry_run {
                    tx.rollback().await?;
                } else {
                    tx.commit().await?;
                }
                return Ok(Fate::Unavailable);
            }
            Ok(outcome) => {
                if dry_run {
                    tx.rollback().await?;
                } else {
                    tx.commit().await?;
                }
                return Ok(Fate::Searched(outcome));
            }
            Err(error) if is_deadlock(&error) && deadlocks < DEADLOCK_RETRIES => {
                deadlocks += 1;
                tx.rollback().await?;
            }
            Err(sqlx::Error::Database(error)) => {
                tracing::warn!(work_id, %error, "the cover search of an imported book was rolled back");
                tx.rollback().await?;
                return Ok(Fate::Database);
            }
            Err(error) => return Err(ImportError::Db(error)),
        }
    }
}
