//! [`FakeCatalog`]: a programmable `CatalogLookup` that creates works through ingest.

use std::collections::HashMap;
use std::sync::Mutex;

use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::Isbn13;
use bookclub_server::catalog::shared::lookup::{
    CatalogLookup, CoverAnswer, CoverQuery, LookupError, Resolved, local_isbn,
};
use sqlx::{Connection, PgConnection};

/// What a programmed lookup resolves to.
#[derive(Debug, Clone)]
pub enum FakeHit {
    /// Resolve to an existing work (and optionally edition).
    Existing {
        work_id: i64,
        edition_id: Option<i64>,
    },
    /// Create (once) a work through ingest and resolve to it; with an ISBN query, an edition
    /// with that ISBN too. `Source::OpenLibrary` needs `ol_work_id`.
    New {
        title: String,
        authors: String,
        ol_work_id: Option<String>,
        source: Source,
    },
}

/// A lookup that gives no definite answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeError {
    Throttled,
    Unavailable,
}

impl From<FakeError> for LookupError {
    fn from(error: FakeError) -> Self {
        match error {
            FakeError::Throttled => LookupError::Throttled,
            FakeError::Unavailable => LookupError::Unavailable,
        }
    }
}

/// A programmable `CatalogLookup`. Unprogrammed ISBNs and texts are `Ok(None)`.
///
/// - `resolve_isbn` tries the local step (`lookup::local_isbn`) first, like CAT; the failures
///   and the programmed hits stand for the remote step.
/// - `resolve_isbn_remote` skips the local step.
/// - `resolve_text` has no local step.
///
/// Every call is recorded for [`FakeCatalog::calls`].
#[derive(Debug, Default)]
pub struct FakeCatalog {
    state: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    isbn: HashMap<String, Programmed>,
    text: HashMap<(String, String), Programmed>,
    fail_all: Option<FakeError>,
    /// `None` = always; `Some(n)` = the next n calls.
    fail_isbn: HashMap<String, (FakeError, Option<usize>)>,
    calls: Vec<String>,
    /// Cover searches by (title, author) as `CoverQuery` sends them.
    covers: HashMap<(String, Option<String>), CoverAnswer>,
    /// ISBN cover checks by ISBN-13.
    isbn_covers: HashMap<String, Option<i64>>,
    /// The next n cover searches and ISBN cover checks fail.
    fail_covers: Option<(FakeError, usize)>,
    cover_calls: Vec<String>,
}

#[derive(Debug, Clone)]
struct Programmed {
    hit: FakeHit,
    /// What a `FakeHit::New` created on its first use.
    created: Option<Resolved>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Isbn(String),
    Text(String, String),
}

impl FakeCatalog {
    /// `resolve_isbn` and `resolve_isbn_remote` of `isbn13` resolve to `hit`.
    pub fn on_isbn(&self, isbn13: &str, hit: FakeHit) {
        let programmed = Programmed { hit, created: None };
        self.lock().isbn.insert(isbn13.to_owned(), programmed);
    }

    /// `resolve_text(title, authors)` resolves to `hit`.
    pub fn on_text(&self, title: &str, authors: &str, hit: FakeHit) {
        let programmed = Programmed { hit, created: None };
        self.lock()
            .text
            .insert((title.to_owned(), authors.to_owned()), programmed);
    }

    /// Every remote step fails with `err` from now on.
    pub fn fail_all(&self, err: FakeError) {
        self.lock().fail_all = Some(err);
    }

    /// The remote step for `isbn13` fails with `err`: always (`None`) or for the next `n` calls.
    pub fn fail_isbn(&self, isbn13: &str, err: FakeError, times: Option<usize>) {
        self.lock()
            .fail_isbn
            .insert(isbn13.to_owned(), (err, times));
    }

    /// Every call so far: `isbn:978…`, `isbn_remote:978…`, `text:<title>|<authors>`.
    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    /// `cover_candidates` for this title and author (`CoverQuery.title` and `.author`) answers
    /// `answer` (#74). Unprogrammed searches answer no match.
    pub fn on_covers(&self, title: &str, author: Option<&str>, answer: CoverAnswer) {
        self.lock()
            .covers
            .insert((title.to_owned(), author.map(str::to_owned)), answer);
    }

    /// `cover_by_isbn(isbn13)` answers `cover` (#74). Unprogrammed ISBNs have no cover.
    pub fn on_isbn_cover(&self, isbn13: &str, cover: Option<i64>) {
        self.lock().isbn_covers.insert(isbn13.to_owned(), cover);
    }

    /// The next `times` cover searches and ISBN cover checks fail with `err`.
    pub fn fail_covers(&self, err: FakeError, times: usize) {
        self.lock().fail_covers = Some((err, times));
    }

    /// Every cover search and ISBN cover check so far, apart from [`FakeCatalog::calls`]:
    /// `cover:<title>|<author>|<lang>` and `isbn_cover:978…`.
    pub fn cover_calls(&self) -> Vec<String> {
        self.lock().cover_calls.clone()
    }

    fn cover_failure(&self) -> Option<FakeError> {
        let mut state = self.lock();
        if let Some(error) = state.fail_all {
            return Some(error);
        }
        match state.fail_covers {
            Some((error, n)) if n > 0 => {
                state.fail_covers = Some((error, n - 1));
                Some(error)
            }
            _ => None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    fn record(&self, call: String) {
        self.lock().calls.push(call);
    }

    fn failure(&self, isbn: Option<&str>) -> Option<FakeError> {
        let mut state = self.lock();
        if let Some(error) = state.fail_all {
            return Some(error);
        }
        let isbn = isbn?;
        let (error, times) = state.fail_isbn.get(isbn).copied()?;
        match times {
            None => Some(error),
            Some(0) => {
                state.fail_isbn.remove(isbn);
                None
            }
            Some(n) => {
                state
                    .fail_isbn
                    .insert(isbn.to_owned(), (error, Some(n - 1)));
                Some(error)
            }
        }
    }

    fn programmed(&self, key: &Key) -> Option<Programmed> {
        let state = self.lock();
        match key {
            Key::Isbn(isbn) => state.isbn.get(isbn).cloned(),
            Key::Text(title, authors) => state.text.get(&(title.clone(), authors.clone())).cloned(),
        }
    }

    fn remember(&self, key: &Key, resolved: Resolved) {
        let mut state = self.lock();
        let programmed = match key {
            Key::Isbn(isbn) => state.isbn.get_mut(isbn),
            Key::Text(title, authors) => state.text.get_mut(&(title.clone(), authors.clone())),
        };
        if let Some(programmed) = programmed {
            programmed.created = Some(resolved);
        }
    }

    async fn remote(
        &self,
        conn: &mut PgConnection,
        key: Key,
        isbn: Option<&Isbn13>,
    ) -> Result<Option<Resolved>, LookupError> {
        if let Some(error) = self.failure(isbn.map(Isbn13::as_str)) {
            return Err(error.into());
        }
        let Some(programmed) = self.programmed(&key) else {
            return Ok(None);
        };
        if let Some(created) = programmed.created {
            return Ok(Some(created));
        }
        let resolved = match programmed.hit {
            FakeHit::Existing {
                work_id,
                edition_id,
            } => Resolved {
                work_id,
                edition_id,
            },
            FakeHit::New {
                title,
                authors,
                ol_work_id,
                source,
            } => {
                let work = WorkRecord {
                    ol_work_id,
                    author_names: authors.clone(),
                    authors: authors
                        .split(", ")
                        .filter(|name| !name.is_empty())
                        .map(AuthorRecord::named)
                        .collect(),
                    ..WorkRecord::new(source, title)
                };
                // One transaction (a savepoint inside the caller's), like CAT's
                // `catalog::service::persist`: ingest's advisory and row locks only serialise
                // concurrent lookups of one book inside a transaction. On a bare connection two
                // tasks resolving the same hit at once (two runs of one import job) raced in
                // `replace_work_authors` and one failed on `work_authors_pkey`.
                let mut tx = conn.begin().await?;
                let resolved = match isbn {
                    Some(isbn) => {
                        let edition = EditionRecord {
                            isbn13: Some(isbn.clone()),
                            ..EditionRecord::new(source)
                        };
                        let (work_id, edition_id) = ingest::upsert_edition_with_work(
                            &mut tx,
                            &work,
                            &edition,
                            WorkGrouping::NewWork,
                            Mode::Refresh,
                        )
                        .await?;
                        Resolved {
                            work_id,
                            edition_id: Some(edition_id),
                        }
                    }
                    None => Resolved {
                        work_id: ingest::upsert_work(&mut tx, &work, Mode::Refresh).await?,
                        edition_id: None,
                    },
                };
                tx.commit().await?;
                self.remember(&key, resolved);
                resolved
            }
        };
        Ok(Some(resolved))
    }
}

#[async_trait::async_trait]
impl CatalogLookup for FakeCatalog {
    async fn resolve_isbn(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        self.record(format!("isbn:{}", isbn.as_str()));
        if let Some(found) = local_isbn(conn, isbn).await? {
            return Ok(Some(found));
        }
        self.remote(conn, Key::Isbn(isbn.as_str().to_owned()), Some(isbn))
            .await
    }

    async fn resolve_isbn_remote(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        self.record(format!("isbn_remote:{}", isbn.as_str()));
        self.remote(conn, Key::Isbn(isbn.as_str().to_owned()), Some(isbn))
            .await
    }

    async fn resolve_text(
        &self,
        conn: &mut PgConnection,
        title: &str,
        authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        self.record(format!("text:{title}|{authors}"));
        self.remote(conn, Key::Text(title.to_owned(), authors.to_owned()), None)
            .await
    }

    async fn cover_candidates(&self, query: &CoverQuery) -> Result<CoverAnswer, LookupError> {
        self.lock().cover_calls.push(format!(
            "cover:{}|{}|{}",
            query.title,
            query.author.as_deref().unwrap_or_default(),
            query.language.as_deref().unwrap_or_default()
        ));
        if let Some(error) = self.cover_failure() {
            return Err(error.into());
        }
        let key = (query.title.clone(), query.author.clone());
        Ok(self.lock().covers.get(&key).cloned().unwrap_or_default())
    }

    async fn cover_by_isbn(&self, isbn: &Isbn13) -> Result<Option<i64>, LookupError> {
        self.lock()
            .cover_calls
            .push(format!("isbn_cover:{}", isbn.as_str()));
        if let Some(error) = self.cover_failure() {
            return Err(error.into());
        }
        Ok(self
            .lock()
            .isbn_covers
            .get(isbn.as_str())
            .copied()
            .flatten())
    }
}
