//! CAT-owned catalog state: search caches, the Open Library and DNB clients
//! (docs/phase1/conventions.md §3). Wave 1 stub; CAT fills it in.

use std::sync::Arc;

use sqlx::PgConnection;

use crate::catalog::shared::isbn::Isbn13;
use crate::catalog::shared::lookup::{CatalogLookup, LookupError, Resolved};
use crate::config::Config;
use crate::outbound::Outbound;

/// CAT's private state, kept in `AppState::catalog`. CAT adds fields freely.
pub struct CatalogService {}

impl CatalogService {
    /// Frozen constructor.
    pub fn new(_config: &Config, _outbound: Arc<Outbound>) -> Self {
        Self {}
    }
}

/// W1 stub: no source is reachable yet, so every lookup is `Unavailable` (never a false miss).
#[async_trait::async_trait]
impl CatalogLookup for CatalogService {
    async fn resolve_isbn(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Err(LookupError::Unavailable)
    }

    async fn resolve_isbn_remote(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Err(LookupError::Unavailable)
    }

    async fn resolve_text(
        &self,
        _conn: &mut PgConnection,
        _title: &str,
        _authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        Err(LookupError::Unavailable)
    }
}
