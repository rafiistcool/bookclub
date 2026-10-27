//! `books` → `works` + `editions` + `media` + `legacy_work_keys` (legacy-mapping.md §3).
//!
//! Rows are classified by key and processed OL first, then BC, then ISBN, then GB and OTHER, so a
//! later row whose match resolves to a work an earlier row created merges into it. Catalog fields
//! are written through `catalog::shared::ingest` only.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use super::report::{IsbnLine, IsbnOutcome};
use super::source::LegacyBook;
use super::{ImportError, Importer};
use crate::catalog::shared::ingest::{self, EditionRecord, Mode, Source, WorkGrouping, WorkRecord};
use crate::catalog::shared::isbn::{Isbn13, parse_isbn};
use crate::catalog::shared::lookup::{LookupError, Resolved, local_isbn};
use crate::catalog::shared::work_ref::{canonical_work_id, legacy_key};
use crate::text::{char_len, py_strip, truncate_chars};

static OL_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^OL[0-9]+W$").unwrap());
static BC_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^BC[a-f0-9]{10}$").unwrap());
static ISBN_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ISBN(?:[0-9]{9}[0-9X]|[0-9]{13})$").unwrap());
static GB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^GB[A-Za-z0-9_-]{1,40}$").unwrap());
static SPINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#[0-9a-f]{6}$").unwrap());

/// How long a throttled lookup waits before it retries, unless `--retry-wait-ms` says otherwise
/// (`LookupError::Throttled` carries no Retry-After).
pub(crate) const DEFAULT_RETRY_WAIT: Duration = Duration::from_secs(60);

/// The key classes of legacy-mapping.md §3.1, in processing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum BookClass {
    OpenLibrary,
    User,
    Isbn,
    /// GB and OTHER rows share the last step.
    Google,
    Other,
}

impl BookClass {
    fn rank(self) -> u8 {
        match self {
            BookClass::OpenLibrary => 0,
            BookClass::User => 1,
            BookClass::Isbn => 2,
            BookClass::Google | BookClass::Other => 3,
        }
    }
}

/// The class and canonical id of a legacy `ol_work_key`: its last path segment, with an ISBN
/// key's `x` uppercased (`works.py` `canonical_work_id(work_id_from_key(key))`).
pub(crate) fn classify(ol_work_key: &str) -> (BookClass, String) {
    let last = ol_work_key.rsplit('/').next().unwrap_or("");
    let id = canonical_work_id(last);
    let class = if OL_ID.is_match(&id) {
        BookClass::OpenLibrary
    } else if BC_ID.is_match(&id) {
        BookClass::User
    } else if ISBN_ID.is_match(&id) {
        BookClass::Isbn
    } else if GB_ID.is_match(&id) {
        BookClass::Google
    } else {
        BookClass::Other
    };
    (class, id)
}

/// `legacy_work_keys.match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchKind {
    OpenLibrary,
    User,
    IsbnMatched,
    IsbnUnmatched,
    IsbnError,
    IsbnInvalid,
    GoogleVolume,
    Other,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::OpenLibrary => "openlibrary",
            MatchKind::User => "user",
            MatchKind::IsbnMatched => "isbn_matched",
            MatchKind::IsbnUnmatched => "isbn_unmatched",
            MatchKind::IsbnError => "isbn_error",
            MatchKind::IsbnInvalid => "isbn_invalid",
            MatchKind::GoogleVolume => "google_volume",
            MatchKind::Other => "other",
        }
    }
}

/// Where a legacy book went: its work, the edition its ISBN key identified, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookTarget {
    pub work_id: i64,
    pub edition_id: Option<i64>,
    pub kind: MatchKind,
}

/// A legacy book's values, repaired to fit the target (legacy-mapping.md §0 "Normalising").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CleanBook {
    pub title: String,
    pub authors: String,
    pub description: String,
    pub subjects: Vec<String>,
    /// A cover id > 0.
    pub cover_id: Option<i64>,
    pub pages: Option<i32>,
    pub year: Option<i32>,
    /// The cover row's dominant colour: NULL when the colour came from a Google image or does
    /// not fit `media.dominant_color`.
    pub spine_color: Option<String>,
}

/// Repairs the values the target refuses, calling `count("books.<column>")` per repair.
pub(crate) fn clean_book(b: &LegacyBook, mut count: impl FnMut(&str)) -> CleanBook {
    let trimmed = py_strip(&b.title);
    let title = if trimmed.is_empty() {
        count("books.title");
        "Untitled".to_owned()
    } else if char_len(trimmed) > 1000 {
        count("books.title");
        py_strip(truncate_chars(trimmed, 1000)).to_owned()
    } else {
        trimmed.to_owned()
    };
    let authors = if char_len(&b.authors) > 2000 {
        count("books.authors");
        truncate_chars(&b.authors, 2000).to_owned()
    } else {
        b.authors.clone()
    };
    let description = if char_len(&b.description) > 50_000 {
        count("books.description");
        truncate_chars(&b.description, 50_000).to_owned()
    } else {
        b.description.clone()
    };
    let mut subjects: Vec<String> = match serde_json::from_str::<serde_json::Value>(&b.subjects) {
        Ok(serde_json::Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
        _ => {
            count("books.subjects");
            Vec::new()
        }
    };
    if subjects.len() > 50 {
        count("books.subjects");
        subjects.truncate(50);
    }
    let cover_id = match b.cover_id {
        Some(id) if id > 0 => Some(id),
        Some(_) => {
            count("books.cover_id");
            None
        }
        None => None,
    };
    let pages = match b.pages {
        Some(p) if (1..=100_000).contains(&p) => Some(p as i32),
        Some(_) => {
            count("books.pages");
            None
        }
        None => None,
    };
    let year = match b.year {
        Some(y) if (-5000..=3000).contains(&y) => Some(y as i32),
        Some(_) => {
            count("books.year");
            None
        }
        None => None,
    };
    let spine_color = match (&b.spine_color, cover_id, &b.cover_image_url) {
        (Some(color), Some(_), None) => {
            let color = color.trim().to_ascii_lowercase();
            if color.is_empty() || SPINE.is_match(&color) {
                Some(color)
            } else {
                count("books.spine_color");
                None
            }
        }
        _ => None,
    };
    CleanBook {
        title,
        authors,
        description,
        subjects,
        cover_id,
        pages,
        year,
        spine_color,
    }
}

/// The result of resolving one ISBN.
enum Lookup {
    Hit(Resolved),
    Miss,
    Error,
}

impl Importer {
    /// Maps every legacy book onto a work (and, for ISBN keys, an edition).
    pub(crate) async fn import_books(&mut self) -> Result<(), ImportError> {
        let mut books: Vec<(BookClass, String, LegacyBook)> = self
            .legacy
            .books
            .iter()
            .map(|b| {
                let (class, id) = classify(&b.ol_work_key);
                (class, id, b.clone())
            })
            .collect();
        books.sort_by_key(|(class, _, b)| (class.rank(), b.id));

        let mut by_key: HashMap<String, i64> = HashMap::new();
        let mut by_isbn: HashMap<String, i64> = HashMap::new();
        let mut owner_of_work: HashMap<i64, i64> = HashMap::new();
        let mut kinds: HashMap<MatchKind, u64> = HashMap::new();
        let mut cover_ids: Vec<i64> = Vec::new();
        let mut editions = 0u64;
        let mut merged = 0u64;

        for (class, id, book) in &books {
            let key = legacy_key(id);
            let clean = clean_book(book, |column| self.report.normalised(column));
            cover_ids.extend(clean.cover_id);

            // A key that canonicalises to one already imported (an x/X pair) merges.
            if let Some(first) = by_key.get(&key).copied() {
                let target = self.books[&first];
                self.books.insert(book.id, target);
                merged += 1;
                self.report.merges.push(format!(
                    "book {} ({}) → book {first}: the same key {key}",
                    book.id, book.ol_work_key
                ));
                if *class == BookClass::Isbn {
                    self.report.isbns.push(IsbnLine {
                        legacy_book_id: book.id,
                        key: book.ol_work_key.clone(),
                        isbn13: parse_isbn(&id[4..]).map(|i| i.as_str().to_owned()),
                        outcome: IsbnOutcome::SameAs {
                            legacy_book_id: first,
                        },
                    });
                }
                continue;
            }

            let target = match class {
                BookClass::OpenLibrary => self.open_library_work(id, book, &clean).await?,
                BookClass::User => self.user_work(&clean).await?,
                BookClass::Isbn => match parse_isbn(&id[4..]) {
                    None => {
                        self.report.isbns.push(IsbnLine {
                            legacy_book_id: book.id,
                            key: book.ol_work_key.clone(),
                            isbn13: None,
                            outcome: IsbnOutcome::Invalid,
                        });
                        self.bare_work(&clean, false, MatchKind::IsbnInvalid)
                            .await?
                    }
                    Some(isbn) => match by_isbn.get(isbn.as_str()).copied() {
                        // The ISBN-10 and ISBN-13 keys of one book.
                        Some(first) => {
                            self.report.isbns.push(IsbnLine {
                                legacy_book_id: book.id,
                                key: book.ol_work_key.clone(),
                                isbn13: Some(isbn.as_str().to_owned()),
                                outcome: IsbnOutcome::SameAs {
                                    legacy_book_id: first,
                                },
                            });
                            self.books[&first]
                        }
                        None => {
                            by_isbn.insert(isbn.as_str().to_owned(), book.id);
                            self.isbn_work(book, &isbn, &clean).await?
                        }
                    },
                },
                BookClass::Google => {
                    self.bare_work(&clean, false, MatchKind::GoogleVolume)
                        .await?
                }
                BookClass::Other => self.bare_work(&clean, true, MatchKind::Other).await?,
            };

            // The spine colour belongs to the cover row ingest attached (if it did).
            if let Some(color) = &clean.spine_color
                && let Some(cover_id) = clean.cover_id
            {
                sqlx::query!(
                    "UPDATE media SET dominant_color = $2
                     WHERE ol_cover_id = $1 AND removed_at IS NULL AND dominant_color IS NULL",
                    cover_id,
                    color
                )
                .execute(&mut *self.tx)
                .await?;
            }

            if let Some(first) = owner_of_work.get(&target.work_id).copied() {
                merged += 1;
                // No work id: sequences advance in a dry run, so it would differ from the real
                // run's.
                self.report.merges.push(format!(
                    "book {} ({}) → book {first}: both resolve to one work",
                    book.id, book.ol_work_key
                ));
            } else {
                owner_of_work.insert(target.work_id, book.id);
                *kinds.entry(target.kind).or_default() += 1;
            }
            if target.edition_id.is_some() {
                editions += 1;
            }
            if !id.is_empty() {
                // ON CONFLICT: a key a previous import of the catalog already recorded.
                sqlx::query!(
                    "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, edition_id,
                                                   match)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT DO NOTHING",
                    key,
                    book.id,
                    target.work_id,
                    target.edition_id,
                    target.kind.as_str()
                )
                .execute(&mut *self.tx)
                .await?;
            } else {
                self.report.notes.push(format!(
                    "book {} has the key {:?} with an empty id: no legacy_work_keys row",
                    book.id, book.ol_work_key
                ));
            }
            by_key.insert(key, book.id);
            self.books.insert(book.id, target);
        }

        let count = |kind| kinds.get(&kind).copied().unwrap_or(0);
        let works = owner_of_work.len() as u64;
        self.report.entity(
            "books",
            Some(books.len() as u64),
            Some(works),
            format!(
                "works (openlibrary {}, user {}, isbn matched {}, isbn unmatched {}, isbn error {}, \
                 isbn invalid {}, google volume {}, other {}; {merged} merged)",
                count(MatchKind::OpenLibrary),
                count(MatchKind::User),
                count(MatchKind::IsbnMatched),
                count(MatchKind::IsbnUnmatched),
                count(MatchKind::IsbnError),
                count(MatchKind::IsbnInvalid),
                count(MatchKind::GoogleVolume),
                count(MatchKind::Other),
            ),
        );
        self.report.entity(
            "editions",
            None,
            Some(editions),
            "(the editions ISBN keys identified)",
        );
        let attached = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM media
               WHERE ol_cover_id = ANY($1)
                 AND (EXISTS (SELECT 1 FROM works w WHERE w.cover_media_id = media.id)
                      OR EXISTS (SELECT 1 FROM editions e WHERE e.cover_media_id = media.id))"#,
            &cover_ids
        )
        .fetch_one(&mut *self.tx)
        .await?;
        self.report.entity(
            "covers",
            Some(cover_ids.len() as u64),
            Some(attached as u64),
            "media (Open Library cover ids, fetched on first view)",
        );
        Ok(())
    }

    /// An `OL…W` row: the Open Library work, found or created by its id.
    async fn open_library_work(
        &mut self,
        ol_id: &str,
        book: &LegacyBook,
        clean: &CleanBook,
    ) -> Result<BookTarget, ImportError> {
        let record = WorkRecord {
            ol_work_id: Some(ol_id.to_owned()),
            author_names: clean.authors.clone(),
            first_publish_year: clean.year,
            description: Some(clean.description.clone()),
            subjects: Some(clean.subjects.clone()),
            pages: clean.pages,
            ol_rating: book.ol_rating,
            ol_cover_id: clean.cover_id,
            ..WorkRecord::new(Source::OpenLibrary, clean.title.clone())
        };
        let work_id = ingest::upsert_work(&mut self.tx, &record, Mode::Refresh).await?;
        if let Some(fetched) = book.details_fetched_at {
            sqlx::query!(
                "UPDATE works SET details_fetched_at = GREATEST(details_fetched_at, $2)
                 WHERE id = $1",
                work_id,
                fetched
            )
            .execute(&mut *self.tx)
            .await?;
        }
        Ok(BookTarget {
            work_id,
            edition_id: None,
            kind: MatchKind::OpenLibrary,
        })
    }

    /// A `BC…` row: a user-added work (no creator is known).
    async fn user_work(&mut self, clean: &CleanBook) -> Result<BookTarget, ImportError> {
        let record = WorkRecord {
            author_names: clean.authors.clone(),
            first_publish_year: clean.year,
            description: Some(clean.description.clone()),
            subjects: Some(clean.subjects.clone()),
            pages: clean.pages,
            ol_cover_id: clean.cover_id,
            ..WorkRecord::new(Source::User, clean.title.clone())
        };
        let work_id = ingest::upsert_work(&mut self.tx, &record, Mode::Refresh).await?;
        Ok(BookTarget {
            work_id,
            edition_id: None,
            kind: MatchKind::User,
        })
    }

    /// GB, OTHER and invalid ISBN rows: a bare-facts work without an edition. Only OTHER rows
    /// keep their description; Google's text and categories are never imported.
    async fn bare_work(
        &mut self,
        clean: &CleanBook,
        keep_description: bool,
        kind: MatchKind,
    ) -> Result<BookTarget, ImportError> {
        let record = WorkRecord {
            author_names: clean.authors.clone(),
            first_publish_year: clean.year,
            description: keep_description.then(|| clean.description.clone()),
            pages: clean.pages,
            ol_cover_id: clean.cover_id,
            ..WorkRecord::new(Source::Legacy, clean.title.clone())
        };
        let work_id = ingest::upsert_work(&mut self.tx, &record, Mode::Refresh).await?;
        Ok(BookTarget {
            work_id,
            edition_id: None,
            kind,
        })
    }

    /// A valid `ISBN…` row: re-matched through the lookup, else bare facts with one edition.
    async fn isbn_work(
        &mut self,
        book: &LegacyBook,
        isbn: &Isbn13,
        clean: &CleanBook,
    ) -> Result<BookTarget, ImportError> {
        let lookup = self.resolve_isbn(book, isbn).await?;
        let (target, outcome) = match lookup {
            Lookup::Hit(resolved) => {
                let target = self.matched_isbn(isbn, clean, resolved).await?;
                let title =
                    sqlx::query_scalar!("SELECT title FROM works WHERE id = $1", target.work_id)
                        .fetch_one(&mut *self.tx)
                        .await?;
                self.note_differences(book, clean, &target).await?;
                (
                    target,
                    IsbnOutcome::Matched {
                        work_id: target.work_id,
                        title,
                    },
                )
            }
            Lookup::Miss => (
                self.unmatched_isbn(isbn, clean, MatchKind::IsbnUnmatched)
                    .await?,
                IsbnOutcome::Unmatched,
            ),
            Lookup::Error => (
                self.unmatched_isbn(isbn, clean, MatchKind::IsbnError)
                    .await?,
                IsbnOutcome::Error,
            ),
        };
        self.report.isbns.push(IsbnLine {
            legacy_book_id: book.id,
            key: book.ol_work_key.clone(),
            isbn13: Some(isbn.as_str().to_owned()),
            outcome,
        });
        Ok(target)
    }

    /// Resolves an ISBN: only the local catalog with `--offline`; otherwise the lookup, waiting
    /// out throttling (forever) and aborting on an unavailable source unless
    /// `--allow-unmatched`.
    async fn resolve_isbn(
        &mut self,
        book: &LegacyBook,
        isbn: &Isbn13,
    ) -> Result<Lookup, ImportError> {
        if self.args.offline {
            return Ok(match local_isbn(&mut self.tx, isbn).await? {
                Some(resolved) => Lookup::Hit(resolved),
                None => Lookup::Miss,
            });
        }
        let wait = self
            .args
            .retry_wait_ms
            .map_or(DEFAULT_RETRY_WAIT, Duration::from_millis);
        loop {
            match self.lookup.resolve_isbn(&mut self.tx, isbn).await {
                Ok(Some(resolved)) => return Ok(Lookup::Hit(resolved)),
                Ok(None) => return Ok(Lookup::Miss),
                Err(LookupError::Throttled) => {
                    tracing::info!(isbn = %isbn.as_str(), wait_ms = wait.as_millis() as u64,
                        "catalog lookup throttled; waiting");
                    tokio::time::sleep(wait).await;
                    self.report.lookup_wait += wait;
                }
                Err(LookupError::Unavailable) if self.args.allow_unmatched => {
                    return Ok(Lookup::Error);
                }
                Err(LookupError::Unavailable) => {
                    return Err(ImportError::Upstream(format!(
                        "the catalog could not resolve ISBN {} (legacy book {}, {}): a source is \
                         unavailable. Nothing was written; retry later, or pass --allow-unmatched \
                         to keep such books as bare facts",
                        isbn.as_str(),
                        book.id,
                        book.ol_work_key
                    )));
                }
                Err(LookupError::Db(e)) => return Err(ImportError::Db(e)),
            }
        }
    }

    /// A hit: the legacy year, pages and cover fill only the empty fields of the edition with
    /// that ISBN under the matched work (created when the lookup returned none) and, through it,
    /// of the work, with provenance `legacy`.
    async fn matched_isbn(
        &mut self,
        isbn: &Isbn13,
        clean: &CleanBook,
        resolved: Resolved,
    ) -> Result<BookTarget, ImportError> {
        let work_has_cover = sqlx::query_scalar!(
            r#"SELECT cover_media_id IS NOT NULL AS "has!" FROM works WHERE id = $1"#,
            resolved.work_id
        )
        .fetch_one(&mut *self.tx)
        .await?;
        let record = EditionRecord {
            isbn13: Some(isbn.clone()),
            pages: clean.pages,
            publication_year: clean.year,
            ol_cover_id: clean.cover_id.filter(|_| !work_has_cover),
            ..EditionRecord::new(Source::Legacy)
        };
        let (work_id, edition_id) =
            ingest::upsert_edition(&mut self.tx, resolved.work_id, &record, Mode::Refresh).await?;
        // The edition with this ISBN belongs to another work than the lookup named: keep the
        // lookup's work and leave the edition out.
        let edition_id = (work_id == resolved.work_id).then_some(edition_id);
        Ok(BookTarget {
            work_id: resolved.work_id,
            edition_id,
            kind: MatchKind::IsbnMatched,
        })
    }

    /// A miss (or an unavailable source under `--allow-unmatched`): a new bare-facts work built
    /// from the edition facts (title, authors, year, pages) and its one edition.
    async fn unmatched_isbn(
        &mut self,
        isbn: &Isbn13,
        clean: &CleanBook,
        kind: MatchKind,
    ) -> Result<BookTarget, ImportError> {
        let work = WorkRecord {
            author_names: clean.authors.clone(),
            first_publish_year: clean.year,
            pages: clean.pages,
            ..WorkRecord::new(Source::Legacy, clean.title.clone())
        };
        let edition = EditionRecord {
            isbn13: Some(isbn.clone()),
            pages: clean.pages,
            publication_year: clean.year,
            ol_cover_id: clean.cover_id,
            ..EditionRecord::new(Source::Legacy)
        };
        let (work_id, edition_id) = ingest::upsert_edition_with_work(
            &mut self.tx,
            &work,
            &edition,
            WorkGrouping::NewWork,
            Mode::Refresh,
        )
        .await?;
        Ok(BookTarget {
            work_id,
            edition_id: Some(edition_id),
            kind,
        })
    }

    /// Lists a matched row whose `BookOut` year or pages differ from what the legacy API showed
    /// (the matched record had its own value).
    async fn note_differences(
        &mut self,
        book: &LegacyBook,
        clean: &CleanBook,
        target: &BookTarget,
    ) -> Result<(), ImportError> {
        let row = sqlx::query!(
            r#"SELECT w.first_publish_year, COALESCE(e.pages, w.pages) AS pages
               FROM works w LEFT JOIN editions e ON e.id = $2 AND e.work_id = w.id
               WHERE w.id = $1"#,
            target.work_id,
            target.edition_id
        )
        .fetch_one(&mut *self.tx)
        .await?;
        let show = |v: Option<i32>| v.map_or("-".to_owned(), |v| v.to_string());
        let mut parts = Vec::new();
        if row.first_publish_year != clean.year {
            parts.push(format!(
                "year {} → {}",
                show(clean.year),
                show(row.first_publish_year)
            ));
        }
        if row.pages != clean.pages {
            parts.push(format!("pages {} → {}", show(clean.pages), show(row.pages)));
        }
        if !parts.is_empty() {
            self.report.differences.push(format!(
                "book {} ({}): {}",
                book.id,
                book.ol_work_key,
                parts.join(", ")
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(key: &str) -> LegacyBook {
        LegacyBook {
            id: 1,
            ol_work_key: key.to_owned(),
            title: "T".into(),
            authors: "A".into(),
            cover_id: None,
            cover_image_url: None,
            spine_color: None,
            year: None,
            description: String::new(),
            pages: None,
            subjects: "[]".into(),
            ol_rating: None,
            details_fetched_at: None,
        }
    }

    #[test]
    fn keys_are_classified_by_their_canonical_id() {
        assert_eq!(
            classify("/works/OL45883W"),
            (BookClass::OpenLibrary, "OL45883W".into())
        );
        assert_eq!(
            classify("/works/BC0000000001"),
            (BookClass::User, "BC0000000001".into())
        );
        assert_eq!(
            classify("/works/ISBN316100003x"),
            (BookClass::Isbn, "ISBN316100003X".into())
        );
        assert_eq!(
            classify("/works/isbn9780000000019"),
            (BookClass::Isbn, "ISBN9780000000019".into())
        );
        assert_eq!(
            classify("/works/GBsynth_x-Y"),
            (BookClass::Google, "GBsynth_x-Y".into())
        );
        assert_eq!(
            classify("/works/MISC-1"),
            (BookClass::Other, "MISC-1".into())
        );
        assert_eq!(
            classify("/works/BCDEADBEEF01"),
            (BookClass::Other, "BCDEADBEEF01".into())
        );
        assert_eq!(classify("/works/"), (BookClass::Other, String::new()));
    }

    #[test]
    fn values_the_target_refuses_are_repaired_and_counted() {
        let mut counted = Vec::new();
        let b = LegacyBook {
            title: "x".repeat(1200),
            authors: "a".repeat(2100),
            cover_id: Some(-1),
            spine_color: Some("".into()),
            pages: Some(0),
            year: Some(99999),
            subjects: serde_json::to_string(&(0..60).collect::<Vec<_>>()).unwrap(),
            ..book("/works/BCabcdef0123")
        };
        let clean = clean_book(&b, |c| counted.push(c.to_owned()));
        assert_eq!(char_len(&clean.title), 1000);
        assert_eq!(char_len(&clean.authors), 2000);
        assert_eq!(clean.cover_id, None);
        assert_eq!(clean.spine_color, None);
        assert_eq!((clean.pages, clean.year), (None, None));
        assert_eq!(clean.subjects.len(), 50);
        assert_eq!(clean.subjects[0], "0");
        counted.sort();
        assert_eq!(
            counted,
            vec![
                "books.authors",
                "books.cover_id",
                "books.pages",
                "books.subjects",
                "books.title",
                "books.year"
            ]
        );
    }

    #[test]
    fn blank_titles_become_untitled_and_bad_subjects_empty() {
        let mut counted = Vec::new();
        let b = LegacyBook {
            title: "   ".into(),
            subjects: "not json".into(),
            ..book("/works/BCabcdef0124")
        };
        let clean = clean_book(&b, |c| counted.push(c.to_owned()));
        assert_eq!(clean.title, "Untitled");
        assert!(clean.subjects.is_empty());
        assert_eq!(counted, vec!["books.title", "books.subjects"]);
    }

    #[test]
    fn spine_colours_survive_only_on_open_library_covers() {
        let with = |spine: &str, url: Option<&str>| {
            let b = LegacyBook {
                cover_id: Some(12),
                spine_color: Some(spine.into()),
                cover_image_url: url.map(str::to_owned),
                ..book("/works/OL1W")
            };
            clean_book(&b, |_| {}).spine_color
        };
        assert_eq!(with("#A1B2C3", None).as_deref(), Some("#a1b2c3"));
        assert_eq!(with("", None).as_deref(), Some(""));
        assert_eq!(with("#a1b2c3", Some("http://img.example/x")), None);
        assert_eq!(with("red", None), None);
    }
}
