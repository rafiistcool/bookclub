//! `quotes` (legacy-mapping.md §10) and `search_history` (§13): personal rows kept with their
//! ids.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use super::{ImportError, Importer, detail};
use crate::text::{casefold, char_len, py_strip, truncate_chars};

/// An imported quote, as `quote_added` events are matched against it (§12).
#[derive(Debug, Clone)]
pub(crate) struct QuoteFacts {
    pub id: i64,
    pub user_id: i64,
    /// The legacy book id (the event names the same one).
    pub book_id: i64,
    pub created_at: OffsetDateTime,
    /// The legacy body (its first 80 characters break ties between events).
    pub body: String,
}

impl Importer {
    /// Imports every quote with its id, on the edition its ISBN key identified (else none), with
    /// visibility `audience`; a page outside 0..20000 becomes NULL (counted).
    pub(crate) async fn import_quotes(&mut self) -> Result<(), ImportError> {
        let quotes = self.legacy.quotes.clone();
        let (mut written, mut dropped) = (0u64, 0u64);
        for q in &quotes {
            if !self.has_user(q.user_id) {
                dropped += 1;
                self.dangling(
                    "quotes",
                    q.id,
                    &format!("user {} does not exist", q.user_id),
                );
                continue;
            }
            let Some(book) = self.books.get(&q.book_id).copied() else {
                dropped += 1;
                self.dangling(
                    "quotes",
                    q.id,
                    &format!("book {} does not exist", q.book_id),
                );
                continue;
            };
            let body = self.fit("quotes.body", &q.body, 1000);
            if body.is_empty() {
                dropped += 1;
                self.dangling("quotes", q.id, "the body is empty");
                continue;
            }
            let page = self.in_range("quotes.page", q.page, 0..=20_000);
            sqlx::query!(
                "INSERT INTO quotes (id, user_id, work_id, edition_id, body, page, visibility,
                                     created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'audience', $7, NULL)",
                q.id,
                q.user_id,
                book.work_id,
                book.edition_id,
                body,
                page,
                q.created_at
            )
            .execute(&mut *self.tx)
            .await?;
            written += 1;
            self.quotes.push(QuoteFacts {
                id: q.id,
                user_id: q.user_id,
                book_id: q.book_id,
                created_at: q.created_at,
                body: q.body.clone(),
            });
        }
        self.report.entity(
            "quotes",
            Some(quotes.len() as u64),
            Some(written),
            detail(&[(dropped, "dropped")]),
        );
        Ok(())
    }

    /// Imports the search history with its ids. `normalized_query` is copied (a Python casefold
    /// already); a query over 300 characters is cut and its normalised form recomputed; queries
    /// of one user that then normalise alike keep the most recently used row.
    pub(crate) async fn import_search_history(&mut self) -> Result<(), ImportError> {
        let rows = self.legacy.search_history.clone();
        let mut best: BTreeMap<(i64, String), (OffsetDateTime, i64, String)> = BTreeMap::new();
        let (mut dropped, mut merged) = (0u64, 0u64);
        for h in &rows {
            if !self.has_user(h.user_id) {
                dropped += 1;
                self.dangling(
                    "search_history",
                    h.id,
                    &format!("user {} does not exist", h.user_id),
                );
                continue;
            }
            let trimmed = py_strip(&h.query);
            if trimmed.is_empty() {
                dropped += 1;
                self.dangling("search_history", h.id, "the query is empty");
                continue;
            }
            let (query, normalized) = if char_len(&h.query) > 300 {
                self.report.normalised("search_history.query");
                let query = truncate_chars(&h.query, 300).to_owned();
                let normalized = casefold(&query);
                (query, normalized)
            } else if h.normalized_query.is_empty() || char_len(&h.normalized_query) > 900 {
                self.report.normalised("search_history.normalized_query");
                (h.query.clone(), casefold(&h.query))
            } else {
                (h.query.clone(), h.normalized_query.clone())
            };
            let candidate = (h.last_used_at, h.id, query);
            match best.get_mut(&(h.user_id, normalized.clone())) {
                Some(current) => {
                    merged += 1;
                    if (candidate.0, std::cmp::Reverse(candidate.1))
                        > (current.0, std::cmp::Reverse(current.1))
                    {
                        *current = candidate;
                    }
                }
                None => {
                    best.insert((h.user_id, normalized), candidate);
                }
            }
        }
        let mut written = 0u64;
        for ((user_id, normalized), (last_used_at, id, query)) in &best {
            sqlx::query!(
                "INSERT INTO search_history (id, user_id, query, normalized_query, last_used_at)
                 VALUES ($1, $2, $3, $4, $5)",
                id,
                user_id,
                query,
                normalized,
                last_used_at
            )
            .execute(&mut *self.tx)
            .await?;
            written += 1;
        }
        self.report.entity(
            "search_history",
            Some(rows.len() as u64),
            Some(written),
            detail(&[(merged, "merged"), (dropped, "dropped")]),
        );
        Ok(())
    }
}
