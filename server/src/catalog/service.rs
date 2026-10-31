//! CAT-owned catalog state (docs/phase1/conventions.md §3): the upstream response caches, and
//! the `CatalogLookup` implementation with the ISBN routing of conventions §7.4.

use std::sync::Arc;

use sqlx::{Connection, PgConnection};

use super::cache::{CACHE_TTL, Fetch, JsonCache, ResponseCache};
use super::openlibrary::{OlDoc, isbn_edition};
use super::shared::dnb_marc::DnbRecord;
use super::shared::ingest::{self, CoverTarget, Mode, Source, WorkGrouping};
use super::shared::isbn::{Isbn13, is_german_group};
use super::shared::lookup::{
    CatalogLookup, CoverAnswer, CoverQuery, LookupError, Resolved, local_isbn,
};
use crate::config::Config;
use crate::error::is_deadlock;
use crate::outbound::Outbound;

/// Entries of the Open Library response cache (search pages, trending, subjects, works,
/// authors, ISBN searches).
pub const JSON_CACHE_ENTRIES: u64 = 1024;
/// Entries of the DNB SRU cache (one per ISBN with a record).
pub const SRU_CACHE_ENTRIES: u64 = 256;

/// CAT's private state, kept in `AppState::catalog`.
pub struct CatalogService {
    pub(crate) outbound: Arc<Outbound>,
    /// Raw Open Library JSON, keyed like the legacy `_cache` (spec/catalog.md §2.17).
    pub(crate) json: JsonCache,
    /// DNB SRU `<record>` elements per ISBN.
    pub(crate) sru: ResponseCache<String, Arc<Vec<String>>>,
}

impl CatalogService {
    /// Frozen constructor.
    pub fn new(_config: &Config, outbound: Arc<Outbound>) -> Self {
        Self {
            outbound,
            json: ResponseCache::new(JSON_CACHE_ENTRIES, CACHE_TTL),
            sru: ResponseCache::new(SRU_CACHE_ENTRIES, CACHE_TTL),
        }
    }

    /// Drops every cached upstream answer. Reads of known books stay local without them.
    pub fn clear_caches(&self) {
        self.json.clear();
        self.sru.clear();
    }

    /// The remote route by registration group, without persisting anything and without a
    /// database connection: what a source knows about `isbn`. `Ok(None)` only when every source
    /// asked answered "unknown". [`persist`] stores a hit.
    pub(crate) async fn remote_isbn(&self, isbn: &Isbn13) -> Result<Option<RemoteHit>, NoAnswer> {
        let mut no_answer: Option<NoAnswer> = None;
        if is_german_group(isbn) {
            match self.dnb_record(isbn).await {
                Ok(Some(record)) => {
                    // The one Open Library cover check after a DNB hit. Without an answer the
                    // check stays open (`details_fetched_at` NULL) for the first detail view.
                    let cover = match self.ol_isbn_cover(isbn).await {
                        Ok(cover) => CoverCheck::Done(cover),
                        Err(_) => CoverCheck::Open,
                    };
                    return Ok(Some(RemoteHit::Dnb(record, cover)));
                }
                Ok(None) => {}
                // DNB throttled, cooling down or failing: fall through to Open Library.
                Err(error) => no_answer = Some(NoAnswer::from_fetch(&error, Busy::DnbSru)),
            }
        }
        match self.ol_isbn_search(isbn).await {
            Ok(Some(doc)) => Ok(Some(RemoteHit::OpenLibrary(doc))),
            // Open Library does not know it: the DNB's failure (if any) decided.
            Ok(None) => match no_answer {
                Some(error) => Err(error),
                None => Ok(None),
            },
            Err(error) => Err(
                match (NoAnswer::from_fetch(&error, Busy::OpenLibrary), no_answer) {
                    (throttled @ NoAnswer::Throttled(_), None | Some(NoAnswer::Throttled(_))) => {
                        throttled
                    }
                    _ => NoAnswer::Unavailable,
                },
            ),
        }
    }
}

/// The upstream whose throttle refused, for the 429 hint (`catalog::busy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Busy {
    OpenLibrary,
    DnbSru,
}

/// Why the remote route of an ISBN gave no definite answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoAnswer {
    /// The step that decided was throttled or cooling down: Open Library's, or the DNB's when
    /// Open Library then answered with a miss. 429, with that throttle's cooldown as the hint.
    Throttled(Busy),
    /// An upstream failed (502).
    Unavailable,
}

impl NoAnswer {
    /// Throttled stays throttled (429); anything else is "unavailable" (502).
    fn from_fetch(error: &Fetch, upstream: Busy) -> NoAnswer {
        match error {
            Fetch::Throttled => NoAnswer::Throttled(upstream),
            _ => NoAnswer::Unavailable,
        }
    }
}

impl From<NoAnswer> for LookupError {
    fn from(error: NoAnswer) -> LookupError {
        match error {
            NoAnswer::Throttled(_) => LookupError::Throttled,
            NoAnswer::Unavailable => LookupError::Unavailable,
        }
    }
}

/// Throttled stays throttled (429); anything else is "unavailable" (502).
fn lookup_error(error: &Fetch) -> LookupError {
    match error {
        Fetch::Throttled => LookupError::Throttled,
        _ => LookupError::Unavailable,
    }
}

/// The outcome of the Open Library cover check after a DNB hit.
#[derive(Debug, Clone, Copy)]
pub(crate) enum CoverCheck {
    /// Open Library answered: its cover id, or none.
    Done(Option<i64>),
    /// No answer; the first detail view checks again.
    Open,
}

/// What one source returned for an ISBN.
pub(crate) enum RemoteHit {
    Dnb(DnbRecord, CoverCheck),
    OpenLibrary(OlDoc),
}

/// Persists a remote hit in one transaction (a savepoint inside the caller's), retried once
/// when Postgres aborts it to break a deadlock (conventions §7.2 "Deadlocks").
pub(crate) async fn persist(
    conn: &mut PgConnection,
    isbn: &Isbn13,
    hit: &RemoteHit,
) -> Result<Option<Resolved>, LookupError> {
    let mut attempt = 0;
    loop {
        let mut tx = conn.begin().await?;
        match persist_once(&mut tx, isbn, hit).await {
            Ok(resolved) => {
                tx.commit().await?;
                return Ok(resolved);
            }
            Err(error) if is_deadlock(&error) && attempt == 0 => {
                tracing::warn!(%isbn, "ISBN lookup deadlocked; retrying once");
                tx.rollback().await?;
                attempt += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn persist_once(
    conn: &mut PgConnection,
    isbn: &Isbn13,
    hit: &RemoteHit,
) -> sqlx::Result<Option<Resolved>> {
    match hit {
        RemoteHit::OpenLibrary(doc) => {
            let (work_id, edition_id) = ingest::upsert_edition_with_work(
                conn,
                &doc.work_record(),
                &isbn_edition(isbn),
                WorkGrouping::NewWork,
                Mode::Refresh,
            )
            .await?;
            Ok(Some(Resolved {
                work_id,
                edition_id: Some(edition_id),
            }))
        }
        RemoteHit::Dnb(record, cover) => {
            // The cover's media row is locked (FOR SHARE) before any catalog row, like ingest
            // does, so a concurrent takedown cannot deadlock with this lookup.
            let media = match cover {
                CoverCheck::Done(Some(cover_id)) => ingest::ol_cover_media(conn, *cover_id).await?,
                _ => None,
            };
            let ids = ingest::upsert_dnb_record(conn, record, Mode::Replace).await?;
            let Some(&(work_id, edition_id)) = record
                .editions
                .iter()
                .zip(&ids)
                .find(|(edition, _)| edition.isbn13.as_ref() == Some(isbn))
                .map(|(_, ids)| ids)
            else {
                return Ok(None);
            };
            if let CoverCheck::Done(_) = cover {
                attach_checked_cover(conn, work_id, edition_id, media).await?;
            }
            Ok(Some(Resolved {
                work_id,
                edition_id: Some(edition_id),
            }))
        }
    }
}

/// Records the answer of the one Open Library cover check by ISBN for a DNB work
/// (conventions §7.4): `media` (the cover's row from `ingest::ol_cover_media`, resolved and
/// locked before any catalog row) becomes the edition's cover under the provenance rules, and
/// the check is marked done (`works.details_fetched_at`), so neither a later lookup nor the
/// first detail view repeats it. Only called when Open Library answered (a cover, none, or 404).
pub(crate) async fn attach_checked_cover(
    conn: &mut PgConnection,
    work_id: i64,
    edition_id: i64,
    media: Option<i64>,
) -> sqlx::Result<()> {
    if let Some(media_id) = media {
        ingest::set_cover(
            conn,
            CoverTarget::Edition(edition_id),
            media_id,
            Source::OpenLibrary,
            false,
        )
        .await?;
    }
    // Bookkeeping, not a provenance field: no ingest entry point sets it for a work without an
    // Open Library id.
    sqlx::query!(
        "UPDATE works SET details_fetched_at = now()
         WHERE id = $1 AND source = 'dnb' AND details_fetched_at IS NULL",
        work_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[async_trait::async_trait]
impl CatalogLookup for CatalogService {
    /// The local edition first, then [`CatalogLookup::resolve_isbn_remote`].
    async fn resolve_isbn(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        if let Some(found) = local_isbn(conn, isbn).await? {
            return Ok(Some(found));
        }
        self.resolve_isbn_remote(conn, isbn).await
    }

    /// 978-3: DNB SRU, then Open Library (also while DNB is throttled or failing); every other
    /// group: Open Library only, DNB is never asked. A DNB record is ingested with
    /// `Mode::Replace`, an Open Library hit with `Mode::Refresh`, so an existing bare-facts or
    /// user edition of the ISBN is upgraded under the provenance rules.
    async fn resolve_isbn_remote(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        match self.remote_isbn(isbn).await.map_err(LookupError::from)? {
            Some(hit) => persist(conn, isbn, &hit).await,
            None => Ok(None),
        }
    }

    /// A local work whose title matches (case and accents aside) and whose search text holds
    /// every word of title and authors; else the first Open Library hit of `"{title}
    /// {authors}"` (limit 5), persisted as a stub.
    async fn resolve_text(
        &self,
        conn: &mut PgConnection,
        title: &str,
        authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        let title = crate::text::collapse_ws(title);
        let authors = crate::text::collapse_ws(authors);
        if title.is_empty() {
            return Ok(None);
        }
        let words = [title.as_str(), authors.as_str()]
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let local = sqlx::query_scalar!(
            r#"SELECT w.id
               FROM works w
               WHERE w.merged_into_id IS NULL
                 AND lower(bc_unaccent(w.title)) = lower(bc_unaccent($1))
                 AND w.search_vector @@ plainto_tsquery('simple'::regconfig, bc_unaccent($2))
               ORDER BY ts_rank(w.search_vector,
                                plainto_tsquery('simple'::regconfig, bc_unaccent($2))) DESC,
                        w.id
               LIMIT 1"#,
            title,
            words
        )
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(work_id) = local {
            return Ok(Some(Resolved {
                work_id,
                edition_id: None,
            }));
        }
        let doc = match self.ol_text_search(&words).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return Ok(None),
            Err(error) => return Err(lookup_error(&error)),
        };
        let mut attempt = 0;
        loop {
            let mut tx = conn.begin().await?;
            match ingest::upsert_work_stubs(&mut tx, &[doc.work_record()]).await {
                Ok(ids) => {
                    tx.commit().await?;
                    return Ok(ids.first().map(|&work_id| Resolved {
                        work_id,
                        edition_id: None,
                    }));
                }
                Err(error) if is_deadlock(&error) && attempt == 0 => {
                    tx.rollback().await?;
                    attempt += 1;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Open Library's `search.json` by title and author (`ol_cover_search`); persists nothing.
    async fn cover_candidates(&self, query: &CoverQuery) -> Result<CoverAnswer, LookupError> {
        self.ol_cover_search(query)
            .await
            .map_err(|error| lookup_error(&error))
    }

    /// `/isbn/{isbn}.json` (`ol_isbn_cover`): the edition's first cover id, or none.
    async fn cover_by_isbn(&self, isbn: &Isbn13) -> Result<Option<i64>, LookupError> {
        self.ol_isbn_cover(isbn)
            .await
            .map_err(|error| lookup_error(&error))
    }
}
