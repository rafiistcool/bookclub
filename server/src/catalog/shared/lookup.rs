//! Remote resolution of ISBNs and titles (docs/phase1/conventions.md §7.4). Frozen: CAT
//! implements it in `CatalogService`; tests use `FakeCatalog` (tests/common).

use sqlx::PgConnection;

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
