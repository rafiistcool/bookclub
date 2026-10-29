//! Remote resolution of ISBNs and titles (docs/phase1/conventions.md §7.4). Frozen: CAT
//! implements it in `CatalogService`; tests use `FakeCatalog` (tests/common).

use sqlx::PgConnection;

use super::books::follow_merges;
use super::isbn::Isbn13;

/// Resolves books to local works, fetching and persisting remote records when needed.
#[async_trait::async_trait]
pub trait CatalogLookup: Send + Sync {
    /// Local editions first; then the remote route by registration group; persists hits through
    /// ingest. `Ok(None)` = a definite miss (every source answered). `Err(Throttled/Unavailable)`
    /// = no definite answer.
    async fn resolve_isbn(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError>;

    /// The remote route only, skipping the local step; persists the hit, which upgrades an
    /// existing bare-facts or user edition with that ISBN under the provenance rules. Used by
    /// "add a book or edition with an ISBN" (a hit means 409) and by `rematch-legacy`.
    async fn resolve_isbn_remote(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError>;

    /// Local full-text search, then Open Library search (limit 5, first hit); persists the hit.
    async fn resolve_text(
        &self,
        conn: &mut PgConnection,
        title: &str,
        authors: &str,
    ) -> Result<Option<Resolved>, LookupError>;

    /// The cover search (#74, docs/phase2/notes/cover.md §2.1): Open Library's `search.json` by
    /// title and author, at most 10 works with their best edition for the query. Persists
    /// nothing. `Err(Throttled/Unavailable)` = no answer. Default: no match (implementors that
    /// never search covers).
    async fn cover_candidates(&self, _query: &CoverQuery) -> Result<CoverAnswer, LookupError> {
        Ok(CoverAnswer::default())
    }

    /// The one Open Library cover check by ISBN (`/isbn/{isbn}.json`): a cover id, or `None`
    /// when Open Library has no such edition or no cover. Persists nothing. Default: `None`.
    async fn cover_by_isbn(&self, _isbn: &Isbn13) -> Result<Option<i64>, LookupError> {
        Ok(None)
    }
}

/// What the cover search asks Open Library (#74).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverQuery {
    /// The work's main title as stored (brackets dropped, cut at the first separator), not
    /// folded and not escaped.
    pub title: String,
    /// The first author's surname part as written.
    pub author: Option<String>,
    /// ISO 639-1, sent as `lang`.
    pub language: Option<String>,
}

/// Open Library's answer: how many works matched, and the first 10.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverAnswer {
    pub num_found: i64,
    pub docs: Vec<CoverDoc>,
    /// Open Library refused the query with this 4xx status (not 429): no match, and the
    /// status is recorded as the search's last error.
    pub refused: Option<u16>,
}

/// One Open Library work of the answer, with its best edition for the query. Values are
/// cleaned (NFC, no control characters, collapsed whitespace, cut to the column limits).
#[derive(Debug, Clone, PartialEq)]
pub struct CoverDoc {
    /// `OL…W`.
    pub ol_work_id: String,
    pub title: String,
    pub subtitle: String,
    pub authors: Vec<String>,
    /// A positive cover id.
    pub cover_id: Option<i64>,
    /// Unknown when Open Library did not say (never 0 for "unknown").
    pub edition_count: Option<i32>,
    pub first_publish_year: Option<i32>,
    /// MARC codes.
    pub languages: Vec<String>,
    pub edition: Option<CoverEdition>,
}

/// The best edition of a [`CoverDoc`] for the query and `lang`.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverEdition {
    /// `OL…M`.
    pub ol_edition_id: String,
    pub title: String,
    pub subtitle: String,
    pub cover_id: Option<i64>,
    /// MARC codes (often missing in the answer).
    pub languages: Vec<String>,
}

/// A local work (and edition) a lookup resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub work_id: i64,
    pub edition_id: Option<i64>,
}

/// Why a lookup has no definite answer.
#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("throttled")]
    Throttled,
    #[error("unavailable")]
    Unavailable,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// The local step every ISBN resolution starts with: the edition with this ISBN (a withdrawn one
/// too, it still resolves) and its current work. `resolve_isbn` implementations, `FakeCatalog`
/// and the importer's `--offline` mode share it.
pub async fn local_isbn(conn: &mut PgConnection, isbn: &Isbn13) -> sqlx::Result<Option<Resolved>> {
    let Some(edition) = sqlx::query!(
        "SELECT id, work_id FROM editions WHERE isbn13 = $1",
        isbn.as_str()
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    Ok(follow_merges(conn, edition.work_id)
        .await?
        .map(|work_id| Resolved {
            work_id,
            edition_id: Some(edition.id),
        }))
}
