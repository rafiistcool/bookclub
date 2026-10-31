//! One search of one work (docs/phase2/notes/cover.md §1, §4.1), in three steps so each caller
//! decides where its transactions end:
//!
//! 1. **facts** (read): the work's titles, first author, language, the Open Library works
//!    decided for it, an open suggestion (the takedown flag is read under the lock in step 3). A work that is no candidate is skipped.
//!    A work whose edition has a usable cover takes it (inherit, no request); a DNB work whose
//!    one ISBN cover check never ran gets that check first.
//! 2. **remote:** `CatalogLookup::cover_candidates`, outside any transaction of ours (the
//!    import's own transaction aside): every step before it has committed (or, with
//!    [`Context::rollback`], rolled back), so no lock is held across the request or the pace.
//! 3. **apply** (one transaction, a savepoint inside the caller's): the picture's media row
//!    first, then the work `FOR UPDATE`, re-checked; then the cover, or a suggestion; then the
//!    `cover_search` row.
//!
//! Lock order: media → works → cover_suggestions → cover_search. Every outcome but "throttled"
//! and "skipped" writes `cover_search`, so no book stays at the head of the queue.

use std::collections::HashSet;
use std::time::Duration;

use sqlx::{Connection, PgConnection, Postgres, Transaction};
use time::OffsetDateTime;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::matching::{self, Book, Pick, Reason, Verdict, language_hint};
use crate::catalog::service::attach_checked_cover;
use crate::catalog::shared::ingest::{self, CoverTarget, Source};
use crate::catalog::shared::isbn::Isbn13;
use crate::catalog::shared::lookup::{CatalogLookup, CoverAnswer, LookupError};
use crate::text::truncate_chars;

/// What one search of one work came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A clear match: the picture (`media_id`) is the work's cover now, provenance
    /// `openlibrary`.
    Attached {
        media_id: i64,
    },
    /// The work took its edition's cover.
    Inherited,
    /// The exact ISBN cover check gave the edition (and the work) its cover.
    Isbn {
        media_id: i64,
    },
    /// A suggestion waits for an admin (new, or the one that was open already).
    Suggested {
        suggestion_id: i64,
    },
    /// An uncertain match for a book nobody uses: recorded, no suggestion.
    Uncertain,
    NoMatch,
    /// The write changed nothing (a cover or a decided suggestion came first).
    Blocked,
    /// Open Library did not answer; the failure is recorded on the work.
    Unavailable,
    /// Throttled or cooling down; nothing recorded.
    Throttled,
    /// No candidate (any more): it has a cover, was merged away, is a reader's book, an admin
    /// left it empty, a reader's photo waits for it, or its title or author changed meanwhile.
    Skipped,
    /// The shutdown token was cancelled while waiting for the pace.
    Stopped,
}

impl Outcome {
    /// Open Library answered this search (for the job's breaker).
    pub fn answered(self) -> bool {
        matches!(
            self,
            Outcome::Attached { .. }
                | Outcome::Isbn { .. }
                | Outcome::Suggested { .. }
                | Outcome::Uncertain
                | Outcome::NoMatch
                | Outcome::Blocked
        )
    }
}

/// The wait between two Open Library requests of one caller (the job and `rematch-legacy`: 2
/// seconds), cancellable.
#[derive(Debug)]
pub struct Pacer {
    pace: Duration,
    last: Option<Instant>,
    shutdown: Option<CancellationToken>,
}

impl Pacer {
    pub fn new(pace: Duration, shutdown: Option<CancellationToken>) -> Pacer {
        Pacer {
            pace,
            last: None,
            shutdown,
        }
    }

    /// No pace (an admin's single search, the import).
    pub fn none() -> Pacer {
        Pacer::new(Duration::ZERO, None)
    }

    /// Waits until the pace since the previous request has passed. False when the token was
    /// cancelled (before or during the wait).
    pub async fn wait(&mut self) -> bool {
        if self.cancelled() {
            return false;
        }
        if let Some(last) = self.last {
            let until = last + self.pace;
            if until > Instant::now() {
                match &self.shutdown {
                    Some(token) => {
                        tokio::select! {
                            () = token.cancelled() => return false,
                            () = tokio::time::sleep_until(until) => {}
                        }
                    }
                    None => tokio::time::sleep_until(until).await,
                }
            }
        }
        self.last = Some(Instant::now());
        true
    }

    pub fn cancelled(&self) -> bool {
        self.shutdown
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }
}

/// How a caller searches.
pub struct Context<'a> {
    pub lookup: &'a dyn CatalogLookup,
    pub pacer: &'a mut Pacer,
    /// Every book may get a suggestion (an admin's search, the import, `rematch-legacy`).
    /// False for the job: a book nobody uses (tier 2) only records `uncertain`.
    pub always_suggest: bool,
    /// An admin's "Search now": also books an admin left empty or a reader's photo waits for.
    pub forced: bool,
    /// Every write is rolled back instead of committed, and a failure is not recorded
    /// (`rematch-legacy --dry-run`): the outcome is what a real search would come to, and
    /// nothing stays. Each step still ends its own transaction before the next request, so no
    /// lock is held across Open Library's answer.
    pub rollback: bool,
}

/// Ends one step's transaction: commits it, or rolls it back with [`Context::rollback`].
async fn finish(tx: Transaction<'_, Postgres>, rollback: bool) -> sqlx::Result<()> {
    if rollback {
        tx.rollback().await
    } else {
        tx.commit().await
    }
}

/// An edition cover the work can take.
#[derive(Debug, Clone, Copy)]
struct EditionCover {
    media_id: i64,
    by: Source,
}

/// What the search knows about a work (step 1).
#[derive(Debug)]
pub struct Facts {
    pub work_id: i64,
    pub source: String,
    pub match_key: Option<String>,
    pub book: Book,
    pub in_use: bool,
    state: CandidateState,
    details_open: bool,
    first_isbn: Option<(i64, Isbn13)>,
    edition_cover: Option<EditionCover>,
    blocked: HashSet<String>,
    open_suggestion: Option<i64>,
}

/// The candidate rules of notes §1 for one work.
#[derive(Debug, Clone)]
struct CandidateState {
    current: bool,
    readers_book: bool,
    missing: bool,
    admin_cleared: bool,
    photo_waiting: bool,
    match_key: Option<String>,
}

impl CandidateState {
    fn candidate(&self, forced: bool) -> bool {
        self.current
            && !self.readers_book
            && self.missing
            && (forced || (!self.admin_cleared && !self.photo_waiting))
    }
}

impl Facts {
    /// The work is a candidate (notes §1); `forced` ignores rules 4 and 5.
    pub fn candidate(&self, forced: bool) -> bool {
        self.state.candidate(forced)
    }

    pub fn has_cover(&self) -> bool {
        !self.state.missing
    }

    pub fn readers_book(&self) -> bool {
        self.state.readers_book
    }
}

async fn candidate_state(
    conn: &mut PgConnection,
    work_id: i64,
    lock: bool,
) -> sqlx::Result<Option<CandidateState>> {
    if lock {
        sqlx::query_scalar!("SELECT id FROM works WHERE id = $1 FOR UPDATE", work_id)
            .fetch_optional(&mut *conn)
            .await?;
    }
    let row = sqlx::query!(
        r#"SELECT w.merged_into_id IS NULL AS "current!", w.source = 'user' AS "readers_book!",
                  bc_cover_missing(w.cover_media_id) AS "missing!",
                  COALESCE(w.cover_media_id IS NULL
                           AND w.field_sources ->> 'cover_media_id' = 'admin', false)
                      AS "admin_cleared!",
                  EXISTS (SELECT 1 FROM catalog_edit_requests r
                          WHERE r.work_id = w.id AND r.edition_id IS NULL
                            AND r.kind = 'cover_replace' AND r.status = 'open')
                      AS "photo_waiting!",
                  w.match_key
           FROM works w WHERE w.id = $1"#,
        work_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| CandidateState {
        current: row.current,
        readers_book: row.readers_book,
        missing: row.missing,
        admin_cleared: row.admin_cleared,
        photo_waiting: row.photo_waiting,
        match_key: row.match_key,
    }))
}

/// The lowest-id edition of the work whose cover is usable (not removed, gone or failing, and
/// not the bytes of a cover picture a takedown judged: another reader's copy of it), with that
/// cover's provenance.
async fn edition_cover(
    conn: &mut PgConnection,
    work_id: i64,
) -> sqlx::Result<Option<EditionCover>> {
    let row = sqlx::query!(
        r#"SELECT e.cover_media_id AS "media_id!",
                  COALESCE(e.field_sources ->> 'cover_media_id', e.source) AS "by!"
           FROM editions e JOIN media m ON m.id = e.cover_media_id
           WHERE e.work_id = $1 AND m.removed_at IS NULL AND m.ol_gone_at IS NULL
             AND NOT (m.storage_key IS NULL AND m.fetch_failed_at IS NOT NULL)
             AND NOT EXISTS (SELECT 1 FROM media_removed_hashes h
                             WHERE h.sha256 = m.sha256 AND h.blocks_covers)
           ORDER BY e.id LIMIT 1"#,
        work_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| EditionCover {
        media_id: row.media_id,
        by: Source::parse(&row.by).unwrap_or(Source::Legacy),
    }))
}

/// Whether a takedown cleared the cover of this work or of a work merged into it.
async fn under_takedown(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM cover_search cs JOIN works w ON w.id = cs.work_id
               WHERE cs.takedown_at IS NOT NULL AND (w.id = $1 OR w.merged_into_id = $1)
           ) AS "takedown!""#,
        work_id
    )
    .fetch_one(conn)
    .await
}

async fn open_suggestion(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "SELECT id FROM cover_suggestions WHERE work_id = $1 AND status = 'open'",
        work_id
    )
    .fetch_optional(conn)
    .await
}

/// Step 1: what the search knows about `work_id`; `None` when the work does not exist.
pub async fn facts(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<Facts>> {
    let Some(state) = candidate_state(conn, work_id, false).await? else {
        return Ok(None);
    };
    let work = sqlx::query!(
        r#"SELECT w.title, w.subtitle, w.author_names, w.ol_work_id, w.source,
                  w.details_fetched_at IS NULL AS "details_open!",
                  bc_cover_in_use(w.id) AS "in_use!",
                  fa.name AS "first_name?", fa.sort_name AS "first_sort?"
           FROM works w
           LEFT JOIN LATERAL (
               SELECT a.name, a.sort_name FROM work_authors wa JOIN authors a ON a.id = wa.author_id
               WHERE wa.work_id = w.id ORDER BY wa.position, a.id LIMIT 1
           ) fa ON true
           WHERE w.id = $1"#,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let editions = sqlx::query!(
        "SELECT id, title, subtitle, language, isbn13 FROM editions WHERE work_id = $1 ORDER BY id",
        work_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let languages: Vec<String> = editions.iter().filter_map(|e| e.language.clone()).collect();
    let isbns: Vec<String> = editions.iter().filter_map(|e| e.isbn13.clone()).collect();
    let first_isbn = editions
        .iter()
        .find_map(|e| e.isbn13.clone().map(|isbn| (e.id, Isbn13(isbn))));
    let author = match work.first_name {
        Some(name) => Some((name, work.first_sort)),
        None => work
            .author_names
            .split(", ")
            .map(str::trim)
            .find(|name| !name.is_empty())
            .map(|name| (name.to_owned(), None)),
    };
    let book = Book {
        title: work.title,
        subtitle: work.subtitle,
        edition_titles: editions
            .iter()
            .filter_map(|e| {
                e.title
                    .clone()
                    .map(|title| (title, e.subtitle.clone().unwrap_or_default()))
            })
            .collect(),
        author,
        ol_work_id: work.ol_work_id,
        language: language_hint(&languages, &isbns),
    };
    let blocked = sqlx::query_scalar!(
        r#"SELECT DISTINCT s.ol_work_id AS "ol_work_id!"
           FROM cover_suggestions s JOIN works w ON w.id = s.work_id
           WHERE s.status IN ('approved', 'rejected') AND (w.id = $1 OR w.merged_into_id = $1)"#,
        work_id
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    Ok(Some(Facts {
        work_id,
        source: work.source,
        match_key: state.match_key.clone(),
        book,
        in_use: work.in_use,
        details_open: work.details_open,
        first_isbn,
        edition_cover: edition_cover(conn, work_id).await?,
        blocked,
        open_suggestion: open_suggestion(conn, work_id).await?,
        state,
    }))
}

/// Searches one work (notes §4.1). Each write is a transaction of its own on `conn` (a
/// savepoint when `conn` is inside one).
pub async fn search_one(
    conn: &mut PgConnection,
    ctx: &mut Context<'_>,
    work_id: i64,
) -> sqlx::Result<Outcome> {
    let started: OffsetDateTime = sqlx::query_scalar!(r#"SELECT clock_timestamp() AS "now!""#)
        .fetch_one(&mut *conn)
        .await?;
    let Some(facts) = facts(conn, work_id).await? else {
        return Ok(Outcome::Skipped);
    };
    if !facts.candidate(ctx.forced) {
        return Ok(Outcome::Skipped);
    }
    if let Some(suggestion_id) = facts.open_suggestion {
        return Ok(Outcome::Suggested { suggestion_id });
    }
    if let Some(cover) = facts.edition_cover {
        let mut tx = conn.begin().await?;
        let outcome = inherit(&mut tx, work_id, cover).await?;
        record(&mut tx, &facts, outcome, started, Written::default()).await?;
        finish(tx, ctx.rollback).await?;
        return Ok(outcome);
    }
    if facts.source == "dnb"
        && facts.details_open
        && let Some((edition_id, isbn)) = &facts.first_isbn
    {
        if !ctx.pacer.wait().await {
            return Ok(Outcome::Stopped);
        }
        match ctx.lookup.cover_by_isbn(isbn).await {
            Ok(cover) => {
                let mut tx = conn.begin().await?;
                let media = match cover {
                    Some(cover_id) => ingest::ol_cover_media(&mut tx, cover_id).await?,
                    None => None,
                };
                attach_checked_cover(&mut tx, work_id, *edition_id, media).await?;
                let covered = sqlx::query_scalar!(
                    r#"SELECT NOT bc_cover_missing(cover_media_id) AS "covered!"
                       FROM works WHERE id = $1"#,
                    work_id
                )
                .fetch_one(&mut *tx)
                .await?;
                if covered && let Some(media_id) = media {
                    let outcome = Outcome::Isbn { media_id };
                    record(&mut tx, &facts, outcome, started, Written::default()).await?;
                    finish(tx, ctx.rollback).await?;
                    return Ok(outcome);
                }
                // Open Library has no cover for the ISBN: the check is done (committed before
                // the title search's request, so the work's row is not locked meanwhile), and
                // the title search follows in the same attempt.
                finish(tx, ctx.rollback).await?;
            }
            Err(LookupError::Throttled) => return Ok(Outcome::Throttled),
            Err(LookupError::Unavailable) => {
                if !ctx.rollback {
                    record_failure(conn, work_id, "the ISBN cover check had no answer").await?;
                }
                return Ok(Outcome::Unavailable);
            }
            Err(LookupError::Db(error)) => return Err(error),
        }
    }
    let Some(query) = facts.book.query() else {
        let mut tx = conn.begin().await?;
        record(
            &mut tx,
            &facts,
            Outcome::NoMatch,
            started,
            Written::default(),
        )
        .await?;
        finish(tx, ctx.rollback).await?;
        return Ok(Outcome::NoMatch);
    };
    if !ctx.pacer.wait().await {
        return Ok(Outcome::Stopped);
    }
    let answer = match ctx.lookup.cover_candidates(&query).await {
        Ok(answer) => answer,
        Err(LookupError::Throttled) => return Ok(Outcome::Throttled),
        Err(LookupError::Unavailable) => {
            if !ctx.rollback {
                record_failure(conn, work_id, "Open Library's search had no answer").await?;
            }
            return Ok(Outcome::Unavailable);
        }
        Err(LookupError::Db(error)) => return Err(error),
    };
    let unusable = unusable_covers(conn, &answer).await?;
    let verdict = matching::verdict(&facts.book, &answer, &facts.blocked, &unusable);
    let refused = answer
        .refused
        .map(|status| format!("Open Library refused the query ({status})"));
    apply(conn, ctx, &facts, verdict, refused, started).await
}

/// The cover ids of the answer whose media row is removed or gone, or holds the bytes of a cover
/// picture a takedown judged.
async fn unusable_covers(
    conn: &mut PgConnection,
    answer: &CoverAnswer,
) -> sqlx::Result<HashSet<i64>> {
    let ids: Vec<i64> = answer
        .docs
        .iter()
        .flat_map(|doc| {
            [
                doc.cover_id,
                doc.edition.as_ref().and_then(|edition| edition.cover_id),
            ]
        })
        .flatten()
        .collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(sqlx::query_scalar!(
        r#"SELECT ol_cover_id AS "ol_cover_id!" FROM media m
           WHERE ol_cover_id = ANY($1)
             AND (removed_at IS NOT NULL OR ol_gone_at IS NOT NULL
                  OR EXISTS (SELECT 1 FROM media_removed_hashes h
                             WHERE h.sha256 = m.sha256 AND h.blocks_covers))"#,
        &ids
    )
    .fetch_all(conn)
    .await?
    .into_iter()
    .collect())
}

/// The work takes its edition's cover, only into an empty field.
async fn inherit(
    conn: &mut PgConnection,
    work_id: i64,
    cover: EditionCover,
) -> sqlx::Result<Outcome> {
    let set = ingest::set_cover(
        conn,
        CoverTarget::Work(work_id),
        cover.media_id,
        cover.by,
        true,
    )
    .await?;
    Ok(if set {
        Outcome::Inherited
    } else {
        Outcome::Blocked
    })
}

/// Step 3: the verdict applied under the work's lock, re-checked.
async fn apply(
    conn: &mut PgConnection,
    ctx: &Context<'_>,
    facts: &Facts,
    verdict: Verdict,
    error: Option<String>,
    started: OffsetDateTime,
) -> sqlx::Result<Outcome> {
    let mut tx = conn.begin().await?;
    let (pick, reason) = match verdict {
        Verdict::NoMatch => {
            let written = Written {
                error,
                ..Written::default()
            };
            record(&mut tx, facts, Outcome::NoMatch, started, written).await?;
            finish(tx, ctx.rollback).await?;
            return Ok(Outcome::NoMatch);
        }
        Verdict::Clear(pick) => (pick, None),
        Verdict::Uncertain(pick, reason) => (pick, Some(reason)),
    };
    // Media first (conventions §7.2): a takedown of the picture waits for us, or ran first and
    // this finds the row unusable.
    let Some(media_id) = ingest::ol_cover_media(&mut tx, pick.cover_id).await? else {
        record(
            &mut tx,
            facts,
            Outcome::NoMatch,
            started,
            Written::default(),
        )
        .await?;
        finish(tx, ctx.rollback).await?;
        return Ok(Outcome::NoMatch);
    };
    let Some(now) = candidate_state(&mut tx, facts.work_id, true).await? else {
        finish(tx, ctx.rollback).await?;
        return Ok(Outcome::Skipped);
    };
    if !now.candidate(ctx.forced) || now.match_key != facts.match_key {
        // Nothing written: the work has a cover now, or it is due again with its new key.
        finish(tx, ctx.rollback).await?;
        return Ok(Outcome::Skipped);
    }
    if let Some(suggestion_id) = open_suggestion(&mut tx, facts.work_id).await? {
        let outcome = Outcome::Suggested { suggestion_id };
        record(&mut tx, facts, outcome, started, Written::default()).await?;
        finish(tx, ctx.rollback).await?;
        return Ok(outcome);
    }
    // A reader's photo on an edition that landed during the remote step: inherit instead.
    if let Some(cover) = edition_cover(&mut tx, facts.work_id).await? {
        let outcome = inherit(&mut tx, facts.work_id, cover).await?;
        record(&mut tx, facts, outcome, started, Written::default()).await?;
        finish(tx, ctx.rollback).await?;
        return Ok(outcome);
    }
    // A takedown that landed during the remote step turns an attach into a suggestion.
    let takedown = under_takedown(&mut tx, facts.work_id).await?;
    let outcome = match (reason, takedown) {
        (None, false) => {
            let set = ingest::set_cover(
                &mut tx,
                CoverTarget::Work(facts.work_id),
                media_id,
                Source::OpenLibrary,
                true,
            )
            .await?;
            if set {
                Outcome::Attached { media_id }
            } else {
                Outcome::Blocked
            }
        }
        (reason, _) => {
            let reason = reason.unwrap_or(Reason::AfterTakedown);
            if !ctx.always_suggest && !facts.in_use {
                Outcome::Uncertain
            } else {
                match write_suggestion(&mut tx, facts.work_id, &pick, reason).await? {
                    Some(suggestion_id) => Outcome::Suggested { suggestion_id },
                    None => Outcome::Blocked,
                }
            }
        }
    };
    let written = Written {
        attached: matches!(outcome, Outcome::Attached { .. })
            .then(|| (pick.ol_work_id.clone(), pick.cover_id)),
        error: None,
    };
    record(&mut tx, facts, outcome, started, written).await?;
    finish(tx, ctx.rollback).await?;
    Ok(outcome)
}

/// Writes a suggestion under the work's lock (notes §3 decision 3): a closed one for the same
/// Open Library work is reopened, else a new row is inserted. `None` when nothing was written
/// (that Open Library work was decided for the book already).
async fn write_suggestion(
    conn: &mut PgConnection,
    work_id: i64,
    pick: &Pick,
    reason: Reason,
) -> sqlx::Result<Option<i64>> {
    let title = truncate_chars(&pick.title, 1000).to_owned();
    let authors = truncate_chars(&pick.authors, 2000).to_owned();
    let year = pick.year.filter(|year| (-5000..=3000).contains(year));
    let language = pick
        .language
        .clone()
        .filter(|l| l.len() == 3 && l.bytes().all(|b| b.is_ascii_lowercase()));
    let reopened = sqlx::query_scalar!(
        "UPDATE cover_suggestions
         SET status = 'open', decided_at = NULL, created_at = now(), ol_edition_id = $3,
             ol_cover_id = $4, title = $5, authors = $6, year = $7, language = $8, reason = $9
         WHERE work_id = $1 AND ol_work_id = $2 AND status = 'closed'
         RETURNING id",
        work_id,
        pick.ol_work_id,
        pick.ol_edition_id,
        pick.cover_id,
        title,
        authors,
        year,
        language,
        reason.as_str()
    )
    .fetch_optional(&mut *conn)
    .await?;
    if reopened.is_some() {
        return Ok(reopened);
    }
    // No conflict target: either unique key (one open per book, one row per Open Library
    // work) writes nothing.
    sqlx::query_scalar!(
        "INSERT INTO cover_suggestions (work_id, ol_work_id, ol_edition_id, ol_cover_id, title,
                                        authors, year, language, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT DO NOTHING
         RETURNING id",
        work_id,
        pick.ol_work_id,
        pick.ol_edition_id,
        pick.cover_id,
        title,
        authors,
        year,
        language,
        reason.as_str()
    )
    .fetch_optional(conn)
    .await
}

/// What a `cover_search` row records beyond the outcome.
#[derive(Debug, Default)]
struct Written {
    /// The Open Library work and cover id of an attached cover.
    attached: Option<(String, i64)>,
    error: Option<String>,
}

/// The `cover_search` row after an answered attempt (notes §4.1 table). Applies only when no
/// other attempt recorded a result since this one started (`searched_at` is when a result was
/// recorded), so two searches of one book at once record one result and count one miss.
async fn record(
    conn: &mut PgConnection,
    facts: &Facts,
    outcome: Outcome,
    started: OffsetDateTime,
    written: Written,
) -> sqlx::Result<()> {
    let name = match outcome {
        Outcome::Attached { .. } => "attached",
        Outcome::Inherited => "inherited",
        Outcome::Isbn { .. } => "isbn",
        Outcome::Suggested { .. } => "suggested",
        Outcome::Uncertain => "uncertain",
        Outcome::NoMatch => "no_match",
        Outcome::Blocked => "blocked",
        Outcome::Unavailable | Outcome::Throttled | Outcome::Skipped | Outcome::Stopped => {
            return Ok(());
        }
    };
    let error = written
        .error
        .map(|error| truncate_chars(&error, 300).to_owned());
    let (ol_work, ol_cover) = written.attached.unzip();
    if matches!(
        outcome,
        Outcome::Uncertain | Outcome::NoMatch | Outcome::Blocked
    ) {
        sqlx::query!(
            "INSERT INTO cover_search AS cs (work_id, searched_at, searched_key, outcome, misses,
                                             failures, next_search_at, last_error)
             VALUES ($1, clock_timestamp(), $2, $3, 1, 0, now() + interval '30 days', $4)
             ON CONFLICT (work_id) DO UPDATE SET
                 searched_at = clock_timestamp(), searched_key = $2, outcome = $3,
                 misses = (CASE WHEN cs.searched_key IS NOT DISTINCT FROM $2 THEN cs.misses
                                ELSE 0 END) + 1,
                 failures = 0,
                 next_search_at = now() + LEAST(
                     interval '30 days'
                         * power(2, LEAST(CASE WHEN cs.searched_key IS NOT DISTINCT FROM $2
                                               THEN cs.misses ELSE 0 END, 8)),
                     interval '180 days'),
                 last_error = $4
             WHERE cs.searched_at IS NULL OR cs.searched_at < $5",
            facts.work_id,
            facts.match_key,
            name,
            error,
            started
        )
        .execute(conn)
        .await?;
    } else {
        sqlx::query!(
            "INSERT INTO cover_search AS cs (work_id, searched_at, searched_key, outcome, misses,
                                             failures, next_search_at, last_error,
                                             attached_ol_work_id, attached_ol_cover_id)
             VALUES ($1, clock_timestamp(), $2, $3, 0, 0, NULL, NULL, $4, $5)
             ON CONFLICT (work_id) DO UPDATE SET
                 searched_at = clock_timestamp(), searched_key = $2, outcome = $3, misses = 0,
                 failures = 0, next_search_at = NULL, last_error = NULL,
                 attached_ol_work_id = COALESCE($4, cs.attached_ol_work_id),
                 attached_ol_cover_id = COALESCE($5, cs.attached_ol_cover_id)
             WHERE cs.searched_at IS NULL OR cs.searched_at < $6",
            facts.work_id,
            facts.match_key,
            name,
            ol_work,
            ol_cover,
            started
        )
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// Failures in a row after which a book Open Library fails on while it answers others counts a
/// miss (about 15 hours of retries, 1 + 2 + 4 + 8).
pub const FAILURES_BEFORE_MISS: i32 = 5;

/// Open Library gave no answer for this work: `failures + 1`, the error, and the next try after
/// 1 hour × 2^(failures − 1), at most 24 hours. The outcome of the last answer stays.
///
/// From the [`FAILURES_BEFORE_MISS`]th failure in a row on, a failure while Open Library
/// answered another book's search within the last 24 hours is a failure of this book, not an
/// outage: it also counts a miss (outcome `no_match`, `misses + 1`, the miss backoff), so the
/// tier-2 stop applies and the book is not asked every day for good (security review of the
/// branch). `searched_at` stays the last answer's (no answer came), and `failures` stays, so
/// the admin's list still says Open Library failed on the book.
pub async fn record_failure(
    conn: &mut PgConnection,
    work_id: i64,
    error: &str,
) -> sqlx::Result<()> {
    let error = truncate_chars(error, 300);
    let failures = sqlx::query_scalar!(
        "INSERT INTO cover_search AS cs (work_id, failures, next_search_at, last_error)
         VALUES ($1, 1, now() + interval '1 hour', $2)
         ON CONFLICT (work_id) DO UPDATE SET
             failures = cs.failures + 1,
             next_search_at = now() + LEAST(interval '1 hour' * power(2, LEAST(cs.failures, 8)),
                                            interval '24 hours'),
             last_error = $2
         RETURNING failures",
        work_id,
        error
    )
    .fetch_one(&mut *conn)
    .await?;
    if failures < FAILURES_BEFORE_MISS {
        return Ok(());
    }
    let missed = format!("{error} ({failures} times, while it answered other books)");
    let missed = truncate_chars(&missed, 300);
    sqlx::query!(
        "UPDATE cover_search cs SET
             searched_key = w.match_key, outcome = 'no_match',
             misses = (CASE WHEN cs.searched_key IS NOT DISTINCT FROM w.match_key THEN cs.misses
                            ELSE 0 END) + 1,
             next_search_at = now() + LEAST(
                 interval '30 days'
                     * power(2, LEAST(CASE WHEN cs.searched_key IS NOT DISTINCT FROM w.match_key
                                           THEN cs.misses ELSE 0 END, 8)),
                 interval '180 days'),
             last_error = $2
         FROM works w
         WHERE cs.work_id = $1 AND w.id = cs.work_id
           AND EXISTS (SELECT 1 FROM cover_search o
                       WHERE o.work_id <> $1 AND o.searched_at > now() - interval '24 hours')",
        work_id,
        missed
    )
    .execute(conn)
    .await?;
    Ok(())
}
