//! Legacy books → works, editions, covers and `legacy_work_keys` (docs/phase1/legacy-mapping.md
//! §3, §17 AC 6 (the catalog half), 9, 10, 11 and 15 (`rematch-legacy`)), on
//! tests/fixtures/legacy/fresh.sql.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::catalog::shared::isbn::{Isbn13, parse_isbn};
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError, Resolved};
use bookclub_server::catalog::shared::work_ref::{WorkRef, resolve_local};
use bookclub_server::importer::{self, IsbnOutcome, MatchKind, RematchArgs, Report};
use common::*;
use legacy::*;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;
use time::macros::datetime;

/// A work's catalog columns.
#[derive(Debug, sqlx::FromRow)]
struct Work {
    title: String,
    author_names: String,
    first_publish_year: Option<i32>,
    description: String,
    subjects: Vec<String>,
    pages: Option<i32>,
    ol_work_id: Option<String>,
    ol_rating: Option<f64>,
    source: String,
    created_by: Option<i64>,
    details_fetched_at: Option<OffsetDateTime>,
    field_sources: Value,
    cover: Option<i64>,
    cover_color: Option<String>,
}

async fn work(app: &TestApp, id: i64) -> Work {
    sqlx::query_as(
        "SELECT w.title, w.author_names, w.first_publish_year, w.description, w.subjects,
                w.pages, w.ol_work_id, w.ol_rating, w.source, w.created_by, w.details_fetched_at,
                w.field_sources, m.ol_cover_id AS cover, m.dominant_color AS cover_color
         FROM works w LEFT JOIN media m ON m.id = w.cover_media_id
         WHERE w.id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// The work a legacy key (as `legacy_work_keys` stores it) maps to, and its match.
async fn keyed(app: &TestApp, legacy_key: &str) -> (i64, Option<i64>, String) {
    sqlx::query_as("SELECT work_id, edition_id, match FROM legacy_work_keys WHERE legacy_key = $1")
        .bind(legacy_key)
        .fetch_one(&app.db)
        .await
        .unwrap_or_else(|e| panic!("no legacy key {legacy_key}: {e}"))
}

async fn fresh_import(app: &TestApp) -> (Legacy, Report) {
    let legacy = load("fresh.sql");
    let report = import_ok(app, legacy.args("demo")).await;
    (legacy, report)
}

/// OL rows become Open Library works with their legacy facts, cover (with the spine colour as
/// its dominant colour) and `details_fetched_at`.
#[sqlx::test]
async fn open_library_rows_become_open_library_works(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, report) = fresh_import(&app).await;
    let (work_id, edition, kind) = keyed(&app, "/works/OL45883W").await;
    assert_eq!((edition, kind.as_str()), (None, "openlibrary"));
    let w = work(&app, work_id).await;
    assert_eq!(
        (
            w.title.as_str(),
            w.author_names.as_str(),
            w.ol_work_id.as_deref()
        ),
        ("Middlemarch", "George Eliot", Some("OL45883W"))
    );
    assert_eq!(
        (w.first_publish_year, w.pages, w.ol_rating),
        (Some(1871), Some(880), Some(4.1))
    );
    assert_eq!(w.source, "openlibrary");
    assert_eq!(w.description, "A study of provincial life.");
    assert!(
        w.subjects.contains(&"Fiction".to_owned()),
        "{:?}",
        w.subjects
    );
    assert_eq!(
        (w.cover, w.cover_color.as_deref()),
        (Some(12345), Some("#a1b2c3"))
    );
    assert_eq!(
        w.details_fetched_at,
        Some(datetime!(2026-07-23 12:00:00.123456 UTC))
    );
    assert_eq!(
        w.field_sources["title"],
        Value::String("openlibrary".into())
    );
    let covers = report.entity("covers").unwrap();
    // cover ids 12345, 777 (the unmatched ISBN row); -1 is no cover.
    assert_eq!((covers.source, covers.target), (Some(2), Some(2)));
}

/// BC rows become user-added works without a creator, with their description and subjects and
/// `user` provenance.
#[sqlx::test]
async fn club_rows_become_user_works(db: PgPool) {
    let app = TestApp::new(db).await;
    fresh_import(&app).await;
    let (work_id, edition, kind) = keyed(&app, "/works/BC0000000001").await;
    assert_eq!((edition, kind.as_str()), (None, "user"));
    let w = work(&app, work_id).await;
    assert_eq!(
        (w.title.as_str(), w.author_names.as_str(), w.source.as_str()),
        ("Die Bibliothek am Meer", "Mara Winter", "user")
    );
    assert_eq!(
        (w.first_publish_year, w.pages, w.created_by, w.ol_rating),
        (Some(2025), Some(320), None, None)
    );
    assert!(w.description.starts_with("Als Nora"), "{}", w.description);
    assert_eq!(w.subjects, vec!["Roman".to_owned()]);
    assert_eq!(w.details_fetched_at, None);
    assert_eq!(w.field_sources["description"], Value::String("user".into()));
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM legacy_work_keys WHERE match = 'user'")
            .await,
        10
    );
}

/// #38 AC 9: Google rows lose `cover_image_url` (and the spine colour sampled from it), the
/// description, subjects and rating; no GB volume id is stored (`legacy_work_keys` holds a
/// `sha256:` key, which `WorkRef` still resolves); an ISBN-10 key becomes an ISBN-13 edition.
#[sqlx::test]
async fn google_rows_keep_only_bare_facts(db: PgPool) {
    let app = TestApp::new(db).await;
    let (legacy, report) = fresh_import(&app).await;

    // The GB row.
    let r = WorkRef::parse("GBsynthVol42_x-Y").unwrap();
    let WorkRef::Legacy(key) = &r else {
        panic!("{r:?}")
    };
    assert!(key.starts_with("sha256:") && key.len() == 71, "{key}");
    let (gb_work, edition, kind) = keyed(&app, key).await;
    assert_eq!((edition, kind.as_str()), (None, "google_volume"));
    let mut conn = app.db.acquire().await.unwrap();
    assert_eq!(resolve_local(&mut conn, &r).await.unwrap(), Some(gb_work));
    let w = work(&app, gb_work).await;
    assert_eq!(
        (w.title.as_str(), w.author_names.as_str(), w.source.as_str()),
        ("Synthetic Volume", "A. Nonymous", "legacy")
    );
    assert_eq!((w.description.as_str(), w.subjects.len()), ("", 0));
    assert_eq!(
        (w.ol_rating, w.cover, w.first_publish_year),
        (None, None, None)
    );
    assert_eq!(w.pages, Some(150));

    // The unmatched ISBN row: bare facts, one edition, the OL cover kept but not the colour of
    // the Google image.
    let (isbn_work, edition, kind) = keyed(&app, &format!("/works/ISBN{ISBN_UNMATCHED}")).await;
    assert_eq!(kind, "isbn_unmatched");
    let w = work(&app, isbn_work).await;
    assert_eq!(
        (w.title.as_str(), w.source.as_str()),
        ("Ein erfundenes Buch", "legacy")
    );
    assert_eq!(
        (w.description.as_str(), w.subjects.len(), w.ol_rating),
        ("", 0, None)
    );
    assert_eq!((w.cover, w.cover_color), (Some(777), None));
    let e: (Option<String>, Option<i32>, Option<i32>, String) = sqlx::query_as(
        "SELECT isbn13, pages, publication_year, source FROM editions WHERE id = $1",
    )
    .bind(edition.unwrap())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        e,
        (
            Some(ISBN_UNMATCHED.to_owned()),
            Some(250),
            Some(2019),
            "legacy".to_owned()
        )
    );

    // The ISBN-10 key.
    let isbn13 = parse_isbn(ISBN10_KEY).unwrap();
    let (_, edition, kind) = keyed(&app, &format!("/works/ISBN{ISBN10_KEY}")).await;
    assert_eq!(kind, "isbn_unmatched");
    let stored: Option<String> = sqlx::query_scalar("SELECT isbn13 FROM editions WHERE id = $1")
        .bind(edition.unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some(isbn13.as_str()));
    assert!(isbn13.as_str().starts_with("978316100003"));
    let line = report
        .isbns
        .iter()
        .find(|l| l.legacy_book_id == book_id(&legacy, &format!("/works/ISBN{ISBN10_KEY}")))
        .unwrap();
    assert_eq!(line.isbn13.as_deref(), Some(isbn13.as_str()));

    // Nothing Google-shaped anywhere in the catalog.
    let leaks = app
        .scalar_i64(
            "SELECT (SELECT count(*) FROM works
                     WHERE title || author_names || description || array_to_string(subjects, ' ')
                           ~* '(google|synthVol|Synthetic Google|Fiction / )')
                  + (SELECT count(*) FROM legacy_work_keys WHERE legacy_key ~ 'synthVol')
                  + (SELECT count(*) FROM works WHERE ol_rating IS NOT NULL AND source <> 'openlibrary')",
        )
        .await;
    assert_eq!(leaks, 0);
}

/// OTHER keys keep their description; invalid ISBN keys become bare facts without an edition and
/// are listed with their raw value.
#[sqlx::test]
async fn other_and_invalid_keys_become_bare_facts(db: PgPool) {
    let app = TestApp::new(db).await;
    let (legacy, report) = fresh_import(&app).await;

    let (other, edition, kind) = keyed(&app, "/works/MISC-legacy-1").await;
    assert_eq!((edition, kind.as_str()), (None, "other"));
    let w = work(&app, other).await;
    assert_eq!(w.title, "Loose Key Book");
    assert_eq!(w.description, "Entered through an old code path.");
    assert_eq!((w.source.as_str(), w.pages), ("legacy", None));

    let (invalid, edition, kind) = keyed(&app, &format!("/works/ISBN{ISBN_INVALID}")).await;
    assert_eq!((edition, kind.as_str()), (None, "isbn_invalid"));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {invalid}"
        ))
        .await,
        0
    );
    let id = book_id(&legacy, &format!("/works/ISBN{ISBN_INVALID}"));
    let line = report
        .isbns
        .iter()
        .find(|l| l.legacy_book_id == id)
        .unwrap();
    assert_eq!(line.outcome, IsbnOutcome::Invalid);
    assert!(
        report
            .lines
            .iter()
            .any(|l| l.contains(&format!("ISBN{ISBN_INVALID}")) && l.contains("invalid ISBN")),
        "{:?}",
        report.lines
    );
}

/// #38 AC 10: `cover_id = -1` gives no cover, `pages = 0` and 200000 give NULL, a 99999 year
/// gives NULL, an over-long title is truncated and a blank one is `Untitled`; the report counts
/// each and nothing aborts.
#[sqlx::test]
async fn out_of_range_values_are_repaired_and_counted(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, report) = fresh_import(&app).await;

    let (long, _, _) = keyed(&app, "/works/BCabcdef0123").await;
    let w = work(&app, long).await;
    // Cut at 1000 characters, then the trailing space trimmed.
    assert_eq!(w.title.chars().count(), 999);
    assert!(w.title.starts_with("Sehr lang Sehr lang"));
    assert_eq!(w.cover, None);
    assert_eq!(w.subjects.len(), 50);
    let (blank, _, _) = keyed(&app, "/works/BCabcdef0124").await;
    let w = work(&app, blank).await;
    assert_eq!((w.title.as_str(), w.subjects.len()), ("Untitled", 0));
    let (matched, edition, _) = keyed(&app, &format!("/works/ISBN{ISBN_MATCH}")).await;
    let pages: Option<i32> = sqlx::query_scalar("SELECT pages FROM editions WHERE id = $1")
        .bind(edition.unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(pages, None);
    assert_eq!(work(&app, matched).await.pages, None);

    for (column, n) in [
        ("books.cover_id", 1),
        ("books.pages", 2),
        ("books.year", 1),
        ("books.title", 2),
        ("books.subjects", 2),
    ] {
        assert_eq!(
            report.normalised(column),
            n,
            "{column}: {:?}",
            report.normalised
        );
    }
    let line = report
        .lines
        .iter()
        .find(|l| l.starts_with("normalised"))
        .unwrap();
    assert!(line.contains("books.pages 2"), "{line}");
}

/// #38 AC 6 (books): an OL row and an ISBN row that resolves to the same Open Library work
/// become one work; the ISBN key keeps its edition; the report lists the merge, and the
/// matched record's own year as a difference.
#[sqlx::test]
async fn an_isbn_row_resolving_to_an_imported_work_merges_into_it(db: PgPool) {
    let app = TestApp::new(db).await;
    app.catalog.on_isbn(
        ISBN_MATCH,
        FakeHit::New {
            title: "Middlemarch".into(),
            authors: "George Eliot".into(),
            ol_work_id: Some("OL45883W".into()),
            source: Source::OpenLibrary,
        },
    );
    let (legacy, report) = fresh_import(&app).await;

    let (ol_work, _, _) = keyed(&app, "/works/OL45883W").await;
    let (isbn_work, edition, kind) = keyed(&app, &format!("/works/ISBN{ISBN_MATCH}")).await;
    assert_eq!((isbn_work, kind.as_str()), (ol_work, "isbn_matched"));
    let e: (i64, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT work_id, isbn13, publication_year FROM editions WHERE id = $1")
            .bind(edition.unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    // The legacy year fills the edition's empty year, with provenance legacy.
    assert_eq!(e, (ol_work, Some(ISBN_MATCH.to_owned()), Some(2003)));
    let book = book_id(&legacy, &format!("/works/ISBN{ISBN_MATCH}"));
    assert_eq!(report.books[&book].work_id, ol_work);
    assert_eq!(report.books[&book].kind, MatchKind::IsbnMatched);
    let book_merges: Vec<&String> = report
        .merges
        .iter()
        .filter(|m| m.starts_with("book "))
        .collect();
    assert_eq!(book_merges.len(), 1, "{:?}", report.merges);
    // The merged row still counts as matched: every row counts in its class.
    let books = report.entity("books").unwrap();
    assert!(
        books.detail.contains("isbn matched 1,") && books.detail.ends_with("; 1 merged)"),
        "{}",
        books.detail
    );
    // The work keeps the Open Library year: BookOut shows 1871 where the legacy API showed 2003.
    assert_eq!(
        report.differences,
        vec![format!(
            "book {book} (/works/ISBN{ISBN_MATCH}): year 2003 → 1871, pages - → 880"
        )]
    );
    let w = work(&app, ol_work).await;
    assert_eq!(
        (w.first_publish_year, w.title.as_str()),
        (Some(1871), "Middlemarch")
    );
}

/// An x/X pair of ISBN keys canonicalises to one key: the second merges into the first, which
/// owns the one `legacy_work_keys` row.
#[sqlx::test]
async fn keys_that_canonicalise_to_one_merge(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec(
        "INSERT INTO books (id, ol_work_key, title, authors, description, subjects)
         VALUES (40, '/works/ISBN316100003x', 'Zehnstellig (klein)', 'Max Mustermann', '', '[]');",
    );
    let report = import_ok(&app, legacy.args("demo")).await;
    let first = book_id(&legacy, &format!("/works/ISBN{ISBN10_KEY}"));
    assert_eq!(report.books[&40], report.books[&first]);
    assert!(
        report
            .merges
            .iter()
            .any(|m| m.starts_with(&format!("book 40 (/works/ISBN316100003x) → book {first}"))),
        "{:?}",
        report.merges
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM legacy_work_keys WHERE legacy_key = '/works/ISBN{ISBN10_KEY}'"
        ))
        .await,
        1
    );
    // The twin counts in its first's class, so the classes add up to the 18 source rows.
    let books = report.entity("books").unwrap();
    assert_eq!(
        (books.source, books.target, books.detail.as_str()),
        (
            Some(18),
            Some(17),
            "works (openlibrary 1, user 10, isbn matched 0, isbn unmatched 4, isbn error 0, \
             isbn invalid 1, google volume 1, other 1; 1 merged)"
        )
    );
}

/// The ISBN-10 and ISBN-13 keys of one book map onto one work and one edition, with one lookup:
/// the report counts both rows in their class and the edition once.
#[sqlx::test]
async fn the_isbn_10_and_13_keys_of_one_book_share_one_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    // 316148410X is the ISBN-10 of ISBN_UNMATCHED (978-3-16-148410-0).
    legacy.exec(
        "INSERT INTO books (id, ol_work_key, title, authors, description, subjects)
         VALUES (41, '/works/ISBN316148410X', 'Ein erfundenes Buch', 'Erika Mustermann', '',
                 '[]');",
    );
    let report = import_ok(&app, legacy.args("demo")).await;
    let first = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    assert_eq!(report.books[&41], report.books[&first]);
    assert!(report.books[&41].edition_id.is_some());
    let remote = format!("isbn:{ISBN_UNMATCHED}");
    assert_eq!(
        app.catalog.calls().iter().filter(|c| **c == remote).count(),
        1
    );
    assert!(
        report.isbns.iter().any(|line| line.legacy_book_id == 41
            && line.outcome
                == IsbnOutcome::SameAs {
                    legacy_book_id: first
                }),
        "{:?}",
        report.isbns
    );
    let books = report.entity("books").unwrap();
    assert_eq!(
        (books.source, books.target, books.detail.as_str()),
        (
            Some(18),
            Some(17),
            "works (openlibrary 1, user 10, isbn matched 0, isbn unmatched 4, isbn error 0, \
             isbn invalid 1, google volume 1, other 1; 1 merged)"
        )
    );
    // Books 10, 11 (with 41) and 12 each identified one edition.
    let editions = report.entity("editions").unwrap();
    assert_eq!(editions.target, Some(3));
    assert_eq!(
        app.scalar_i64("SELECT count(DISTINCT edition_id) FROM legacy_work_keys")
            .await,
        3
    );
}

/// #38 AC 11 (catalog half): for every key class, a legacy book's year and pages as the legacy
/// API showed them equal the imported `BookOut` (with the edition its ISBN key identified),
/// unless the matched record had its own value; those rows are in `differences`.
#[sqlx::test]
async fn book_out_keeps_the_legacy_year_and_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    let (dnb, _) = app
        .dnb_work("Ein erfundenes Buch", "Erika Mustermann", "9783000000027")
        .await;
    app.catalog.on_isbn(
        ISBN_UNMATCHED,
        FakeHit::Existing {
            work_id: dnb.id,
            edition_id: None,
        },
    );
    let (legacy, report) = fresh_import(&app).await;
    let conn = rusqlite::Connection::open(&legacy.path).unwrap();
    let mut stmt = conn
        .prepare("SELECT id, ol_work_key, year, pages FROM books ORDER BY id")
        .unwrap();
    let books: Vec<(i64, String, Option<i64>, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut pg = app.db.acquire().await.unwrap();
    let mut seen = std::collections::HashSet::new();
    for (id, key, year, pages) in books {
        let target = report.books[&id];
        seen.insert(target.kind);
        let out = book_out(
            &mut pg,
            BookRef {
                work_id: target.work_id,
                edition_id: target.edition_id,
            },
        )
        .await
        .unwrap()
        .unwrap();
        // What the legacy API showed, after the normalisation of out-of-range values.
        let year = year
            .filter(|y| (-5000..=3000).contains(y))
            .map(|y| y as i32);
        let pages = pages
            .filter(|p| (1..=100_000).contains(p))
            .map(|p| p as i32);
        if (out.year, out.pages) != (year, pages) {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|d| d.starts_with(&format!("book {id} ({key})"))),
                "book {id} {key}: {:?} became {:?}, unlisted",
                (year, pages),
                (out.year, out.pages)
            );
            assert_eq!(target.kind, MatchKind::IsbnMatched, "{key}");
        }
    }
    for kind in [
        MatchKind::OpenLibrary,
        MatchKind::User,
        MatchKind::IsbnMatched,
        MatchKind::IsbnUnmatched,
        MatchKind::IsbnInvalid,
        MatchKind::GoogleVolume,
        MatchKind::Other,
    ] {
        assert!(seen.contains(&kind), "{kind:?} not covered");
    }
    // The matched DNB work had no year or pages: the legacy values filled them.
    let id = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    assert!(
        !report
            .differences
            .iter()
            .any(|d| d.starts_with(&format!("book {id} "))),
        "{:?}",
        report.differences
    );
    let w = work(&app, dnb.id).await;
    assert_eq!((w.first_publish_year, w.pages), (Some(2019), Some(250)));
    assert_eq!(w.field_sources["pages"], Value::String("legacy".into()));
}

async fn run_rematch(app: &TestApp, dry_run: bool) -> Report {
    importer::rematch(
        app.db.clone(),
        app.state.lookup.clone(),
        RematchArgs {
            dry_run,
            retry_wait_ms: Some(0),
            cover_pace_ms: Some(0),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("rematch failed: {e}"))
}

/// The ISBN that the fixture's ISBN-10 key stands for.
fn isbn10_as_13() -> String {
    parse_isbn(ISBN10_KEY).unwrap().as_str().to_owned()
}

/// #38 AC 15: after an `--offline` import, programming the catalog with an ISBN and running
/// `rematch-legacy` upgrades the bare-facts edition and its work (the source's provenance, the
/// legacy-ranked title replaced); the shelf entry still points at the same work and edition,
/// and the key's match becomes `isbn_matched`. A dry run changes nothing; a throttled lookup is
/// retried; an unavailable source leaves its edition as it is.
#[sqlx::test]
async fn rematch_legacy_upgrades_bare_facts_editions(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.offline = true;
    let report = import_ok(&app, args).await;
    let book = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    let target = report.books[&book];
    assert_eq!(target.kind, MatchKind::IsbnUnmatched);
    let edition = target.edition_id.unwrap();
    let entry: (i64, Option<i64>) =
        sqlx::query_as("SELECT work_id, edition_id FROM shelf_entries WHERE id = 21")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(entry, (target.work_id, Some(edition)));
    let state = |app: &TestApp| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, String, String, String)>(
                "SELECT e.source, w.source, w.title, k.match
                 FROM editions e JOIN works w ON w.id = e.work_id
                 JOIN legacy_work_keys k ON k.edition_id = e.id
                 WHERE e.id = $1",
            )
            .bind(edition)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let program = || {
        app.catalog.on_isbn(
            ISBN_UNMATCHED,
            FakeHit::New {
                title: "Ein erfundenes Buch: Roman".into(),
                authors: "Erika Mustermann".into(),
                ol_work_id: None,
                source: Source::Dnb,
            },
        );
    };
    app.catalog
        .fail_isbn(&isbn10_as_13(), FakeError::Unavailable, None);

    program();
    let dry = run_rematch(&app, true).await;
    assert!(dry.dry_run);
    assert_eq!(
        state(&app).await,
        (
            "legacy".to_owned(),
            "legacy".to_owned(),
            "Ein erfundenes Buch".to_owned(),
            "isbn_unmatched".to_owned()
        )
    );

    // Throttled twice, then answered: waited out, not counted as a miss.
    program();
    app.catalog
        .fail_isbn(ISBN_UNMATCHED, FakeError::Throttled, Some(2));
    let real = run_rematch(&app, false).await;
    assert_eq!(dry.lines, real.lines);
    let remote = format!("isbn_remote:{ISBN_UNMATCHED}");
    assert_eq!(
        app.catalog.calls().iter().filter(|c| **c == remote).count(),
        4,
        "one in the dry run, three in the real one"
    );
    assert_eq!(
        state(&app).await,
        (
            "dnb".to_owned(),
            "dnb".to_owned(),
            "Ein erfundenes Buch: Roman".to_owned(),
            "isbn_matched".to_owned()
        )
    );
    let after: (i64, Option<i64>) =
        sqlx::query_as("SELECT work_id, edition_id FROM shelf_entries WHERE id = 21")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(after, entry);
    // Three legacy editions with an ISBN: the matched one, the unmatched one (both unmatched
    // offline) and the ISBN-10 key's.
    let line = real.entity("legacy editions").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(3),
            Some(1),
            "upgraded (1 unmatched, 1 with the source unavailable, 0 under another work, \
             0 with a database error)"
        )
    );
    // The ISBN-10 key's edition is unchanged.
    let isbn10 = book_id(&legacy, &format!("/works/ISBN{ISBN10_KEY}"));
    let source: String = sqlx::query_scalar("SELECT source FROM editions WHERE id = $1")
        .bind(report.books[&isbn10].edition_id.unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(source, "legacy");
}

/// A database error a [`Scripted`] lookup raises inside the transaction it is handed.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// SQLSTATE 40P01, raised before the lookup runs (the transaction aborts).
    Deadlock,
    /// Another database error, raised after the lookup persisted its hit.
    Fail,
}

/// One `resolve_isbn_remote` call a [`Scripted`] lookup saw.
#[derive(Debug, Clone)]
struct Seen {
    isbn: String,
    /// `now()`: when the transaction the call ran in started.
    tx_started: OffsetDateTime,
    /// `clock_timestamp()` when the call returned.
    returned: OffsetDateTime,
    /// The ISBNs whose edition another connection already saw upgraded (committed).
    committed: HashSet<String>,
}

/// The FakeCatalog with database faults raised per ISBN, recording what each call saw.
struct Scripted {
    inner: Arc<FakeCatalog>,
    db: PgPool,
    faults: Mutex<HashMap<String, Vec<Fault>>>,
    seen: Mutex<Vec<Seen>>,
}

impl Scripted {
    fn new(app: &TestApp) -> Self {
        Self {
            inner: app.catalog.clone(),
            db: app.db.clone(),
            faults: Mutex::default(),
            seen: Mutex::default(),
        }
    }

    /// The next calls for `isbn13` raise `faults`, one each.
    fn fail(&self, isbn13: &str, faults: &[Fault]) {
        self.faults
            .lock()
            .unwrap()
            .insert(isbn13.to_owned(), faults.to_vec());
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn next_fault(&self, isbn13: &str) -> Option<Fault> {
        let mut faults = self.faults.lock().unwrap();
        let list = faults.get_mut(isbn13)?;
        (!list.is_empty()).then(|| list.remove(0))
    }

    async fn raise(conn: &mut PgConnection, code: &str) -> LookupError {
        let sql = format!(
            "DO $$ BEGIN RAISE EXCEPTION 'scripted fault' USING ERRCODE = '{code}'; END $$"
        );
        LookupError::Db(
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(conn)
                .await
                .unwrap_err(),
        )
    }
}

#[async_trait::async_trait]
impl CatalogLookup for Scripted {
    async fn resolve_isbn(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        self.inner.resolve_isbn(conn, isbn).await
    }

    async fn resolve_isbn_remote(
        &self,
        conn: &mut PgConnection,
        isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        let tx_started: OffsetDateTime = sqlx::query_scalar("SELECT now()")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        let committed: Vec<String> = sqlx::query_scalar(
            "SELECT isbn13 FROM editions WHERE isbn13 IS NOT NULL AND source <> 'legacy'",
        )
        .fetch_all(&self.db)
        .await
        .unwrap();
        let fault = self.next_fault(isbn.as_str());
        let result = match fault {
            Some(Fault::Deadlock) => Err(Self::raise(conn, "40P01").await),
            _ => self.inner.resolve_isbn_remote(conn, isbn).await,
        };
        let result = match (fault, result) {
            (Some(Fault::Fail), Ok(_)) => Err(Self::raise(conn, "P0001").await),
            (_, result) => result,
        };
        // An aborted transaction refuses every statement: read the clock on the pool.
        let returned: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.db)
            .await
            .unwrap();
        self.seen.lock().unwrap().push(Seen {
            isbn: isbn.as_str().to_owned(),
            tx_started,
            returned,
            committed: committed.into_iter().collect(),
        });
        result
    }

    async fn resolve_text(
        &self,
        conn: &mut PgConnection,
        title: &str,
        authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        self.inner.resolve_text(conn, title, authors).await
    }
}

/// `rematch-legacy` runs on the live server, so each edition gets its own transaction: an
/// upgrade is committed before the next lookup starts, a throttled lookup is waited out with
/// its transaction closed, a deadlock is retried in a new transaction, and another database
/// error rolls back only its own edition, which is reported while the others are upgraded.
#[sqlx::test]
async fn rematch_legacy_commits_each_edition_on_its_own(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.offline = true;
    let report = import_ok(&app, args).await;
    let isbn10 = isbn10_as_13();
    for (isbn, title) in [
        (ISBN_MATCH, "Middlemarch: Roman"),
        (ISBN_UNMATCHED, "Ein erfundenes Buch: Roman"),
        (isbn10.as_str(), "Zehnstellig: Roman"),
    ] {
        app.catalog.on_isbn(
            isbn,
            FakeHit::New {
                title: title.into(),
                authors: "Erika Mustermann".into(),
                ol_work_id: None,
                source: Source::Dnb,
            },
        );
    }
    app.catalog
        .fail_isbn(ISBN_MATCH, FakeError::Throttled, Some(1));
    let lookup = Arc::new(Scripted::new(&app));
    lookup.fail(ISBN_UNMATCHED, &[Fault::Deadlock]);
    lookup.fail(&isbn10, &[Fault::Fail]);

    let real = importer::rematch(
        app.db.clone(),
        lookup.clone(),
        RematchArgs {
            dry_run: false,
            retry_wait_ms: Some(0),
            cover_pace_ms: Some(0),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("rematch failed: {e}"));

    let line = real.entity("legacy editions").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(3),
            Some(2),
            "upgraded (0 unmatched, 0 with the source unavailable, 0 under another work, \
             1 with a database error)"
        )
    );
    let source_of = |book: i64| {
        let db = app.db.clone();
        let edition = report.books[&book].edition_id.unwrap();
        async move {
            sqlx::query_scalar::<_, String>("SELECT source FROM editions WHERE id = $1")
                .bind(edition)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let book = |key: &str| book_id(&legacy, &format!("/works/ISBN{key}"));
    assert_eq!(source_of(book(ISBN_MATCH)).await, "dnb");
    assert_eq!(source_of(book(ISBN_UNMATCHED)).await, "dnb");
    // The failed edition's own upgrade was rolled back; it is listed for a retry.
    assert_eq!(source_of(book(ISBN10_KEY)).await, "legacy");
    assert!(
        real.notes
            .iter()
            .any(|n| n.starts_with(&format!("isbn {isbn10} ")) && n.contains("error: database")),
        "{:?}",
        real.notes
    );

    let seen = lookup.seen();
    let calls: Vec<&str> = seen.iter().map(|s| s.isbn.as_str()).collect();
    assert_eq!(
        calls,
        [
            ISBN_MATCH,
            ISBN_MATCH,
            ISBN_UNMATCHED,
            ISBN_UNMATCHED,
            isbn10.as_str()
        ]
    );
    // The retries after the throttle and after the deadlock ran in transactions that started
    // after the failed call returned.
    for retry in [1, 3] {
        assert!(
            seen[retry].tx_started > seen[retry - 1].returned,
            "{:?}",
            seen
        );
    }
    // Each upgrade was committed before the next edition's lookup.
    assert!(seen[2].committed.contains(ISBN_MATCH), "{:?}", seen[2]);
    assert!(
        seen[4].committed.contains(ISBN_MATCH) && seen[4].committed.contains(ISBN_UNMATCHED),
        "{:?}",
        seen[4]
    );
}
