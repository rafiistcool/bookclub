//! The legacy importer against the real lookup (#49, work-plan PAR): IMP's own tests run it with
//! `FakeCatalog`. Here `import-legacy --offline` and `rematch-legacy` get `CatalogService` with
//! every upstream on wiremock. `--offline` checks only the local catalog
//! (`lookup::local_isbn`), so the run makes no upstream request. The imported data then reads
//! through the routes of every package without one. `rematch-legacy` resolves each legacy
//! edition with an ISBN exactly once, along the real route: DNB SRU first for 978-3, then
//! Open Library.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::importer::{self, IsbnOutcome, MatchKind, RematchArgs, Report};
use common::*;
use legacy::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, Request, ResponseTemplate};

/// The DNB SRU record of 9783694050797, recorded 2026-10-27 (CAT).
const SRU_HIT: &str = include_str!("fixtures/dnb_sru/hit_9783694050797.xml");
/// A DNB SRU answer with no record (CAT).
const SRU_MISS: &str = include_str!("fixtures/dnb_sru/miss.xml");
/// Open Library's answer to `search.json?q=isbn:9780316556347`, recorded 2026-10-27 (CAT).
const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");
/// A German ISBN with a DNB record (Patrice McDonough, "Der Mörder von Whitechapel").
const GERMAN: &str = "9783694050797";
/// The English ISBN of Circe (Little, Brown), with a recorded Open Library answer.
const ENGLISH: &str = "9780316556347";

/// Two more legacy ISBN books (ids 18 and 19) on anna's shelf (legacy user 2), whose ISBNs have
/// recorded upstream answers, so `rematch-legacy` can upgrade them.
const EXTRA_BOOKS: &str = "
    INSERT INTO books VALUES(18,'/works/ISBN9783694050797','Der Mörder','Patrice McDonough',
        NULL,NULL,NULL,2024,'',320,'[]',NULL,NULL);
    INSERT INTO books VALUES(19,'/works/ISBN9780316556347','Circe','Madeline Miller',
        NULL,NULL,NULL,2018,'',400,'[]',NULL,NULL);
    INSERT INTO shelf VALUES(28,2,18,'want_to_read',5,NULL,NULL,'','',NULL,
        '2026-08-30 12:00:00.123456',NULL,NULL);
    INSERT INTO shelf VALUES(29,2,19,'finished',6,NULL,5,'Wunderbar.','',NULL,
        '2026-08-30 12:00:00.123456','2026-08-01 12:00:00.123456','2026-08-20 12:00:00.123456');
";

/// The Open Library work the fixture's book 9 names (`OL45883W`), already in the target's
/// catalog with its full record and the edition of the fixture's matched ISBN key, as if the
/// harvester or a search had brought it in before the import.
async fn known_middlemarch(app: &TestApp) -> (i64, i64) {
    let mut conn = app.db.acquire().await.unwrap();
    let work = ingest::upsert_work(
        &mut conn,
        &WorkRecord {
            ol_work_id: Some("OL45883W".to_owned()),
            author_names: "George Eliot".to_owned(),
            authors: vec![AuthorRecord::named("George Eliot")],
            first_publish_year: Some(1871),
            full_details: true,
            ..WorkRecord::new(Source::OpenLibrary, "Middlemarch")
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    let (_, edition) = ingest::upsert_edition(
        &mut conn,
        work,
        &EditionRecord {
            isbn13: parse_isbn(ISBN_MATCH),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    (work, edition)
}

/// fresh.sql plus [`EXTRA_BOOKS`], imported `--offline` through the real lookup.
async fn offline_import(app: &TestApp) -> (Legacy, Report) {
    let legacy = load("fresh.sql");
    legacy.exec(EXTRA_BOOKS);
    let mut args = legacy.args("demo");
    args.offline = true;
    let report = importer::run(
        app.db.clone(),
        app.state.lookup.clone(),
        app.state.media.clone(),
        args,
    )
    .await
    .unwrap_or_else(|e| panic!("the import failed: {e}"));
    (legacy, report)
}

async fn rematch(app: &TestApp) -> Report {
    importer::rematch(
        app.db.clone(),
        app.state.lookup.clone(),
        RematchArgs {
            dry_run: false,
            retry_wait_ms: Some(0),
            cover_pace_ms: Some(0),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("rematch failed: {e}"))
}

/// A legacy user signed in with the old password.
async fn sign_in(app: &TestApp, username: &str) -> TestClient {
    let client = app.anon();
    client
        .post(
            "/api/auth/login",
            json!({"username": username, "password": KNOWN_PASSWORD}),
        )
        .await
        .assert_status(204);
    client
}

/// The SRU requests for `isbn`, the Open Library ISBN searches for it and Open Library's
/// `/isbn/{isbn}.json` (the one cover check after a DNB hit).
fn lookups(requests: &[Request], isbn: &str) -> (usize, usize, usize) {
    let sru = requests
        .iter()
        .filter(|r| r.url.path() == "/sru/dnb")
        .filter(|r| r.url.query_pairs().any(|(_, v)| v.contains(isbn)))
        .count();
    let ol = requests
        .iter()
        .filter(|r| r.url.path() == "/search.json")
        .filter(|r| {
            r.url
                .query_pairs()
                .any(|(k, v)| k == "q" && v == format!("isbn:{isbn}"))
        })
        .count();
    let cover = requests
        .iter()
        .filter(|r| r.url.path() == format!("/isbn/{isbn}.json"))
        .count();
    (sru, ol, cover)
}

/// (edition source, work source, work title, legacy key match) of a legacy book's edition.
async fn edition_state(app: &TestApp, edition: i64) -> (String, String, String, String) {
    sqlx::query_as(
        "SELECT e.source, w.source, w.title, k.match
         FROM editions e JOIN works w ON w.id = e.work_id
         JOIN legacy_work_keys k ON k.edition_id = e.id
         WHERE e.id = $1
         ORDER BY k.legacy_key LIMIT 1",
    )
    .bind(edition)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// (work, edition) of a target shelf entry (ids are kept from the legacy rows).
async fn shelf_target(app: &TestApp, entry: i64) -> (i64, Option<i64>) {
    sqlx::query_as("SELECT work_id, edition_id FROM shelf_entries WHERE id = $1")
        .bind(entry)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// Work-plan PAR: the importer run with the real lookup, offline. The ISBN key the target's
/// catalog already holds resolves locally (and lands on the Open Library work of book 9); every
/// other ISBN key becomes bare facts. Not one upstream request is made, by the import or by the
/// reads of the imported data through the routes of every package afterwards (shelf, book pages
/// by legacy key, the club's pick, history, members, activity and stats, the diary, quotes).
#[sqlx::test]
async fn an_offline_import_with_the_real_lookup_asks_no_upstream(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let (middlemarch, edition) = known_middlemarch(&app).await;
    let (legacy, report) = offline_import(&app).await;
    assert!(
        app.upstream_requests().await.is_empty(),
        "the offline import asked upstream: {:?}",
        app.upstream_requests().await
    );
    assert_eq!(report.lookup_wait, std::time::Duration::ZERO);

    let target = |key: &str| report.books[&book_id(&legacy, key)];
    let matched = target(&format!("/works/ISBN{ISBN_MATCH}"));
    assert_eq!(
        (matched.work_id, matched.edition_id, matched.kind),
        (middlemarch, Some(edition), MatchKind::IsbnMatched)
    );
    assert_eq!(target("/works/OL45883W").work_id, middlemarch);
    for key in [
        format!("/works/ISBN{ISBN_UNMATCHED}"),
        format!("/works/ISBN{ISBN10_KEY}"),
        format!("/works/ISBN{GERMAN}"),
        format!("/works/ISBN{ENGLISH}"),
    ] {
        let unmatched = target(&key);
        assert_eq!(unmatched.kind, MatchKind::IsbnUnmatched, "{key}");
        assert!(unmatched.edition_id.is_some(), "{key}");
    }
    let outcomes: Vec<(String, bool)> = report
        .isbns
        .iter()
        .map(|line| {
            (
                line.key.clone(),
                matches!(line.outcome, IsbnOutcome::Matched { .. }),
            )
        })
        .collect();
    assert!(
        outcomes.contains(&(format!("/works/ISBN{ISBN_MATCH}"), true)),
        "{outcomes:?}"
    );
    assert_eq!(
        outcomes.iter().filter(|(_, matched)| *matched).count(),
        1,
        "{outcomes:?}"
    );

    // The imported data reads through the routes, still without an upstream request.
    let demo = sign_in(&app, "demo").await;
    let lena = sign_in(&app, "lena").await;
    let shelf = lena.get("/api/shelf").await;
    shelf.assert_status(200);
    let titles: Vec<Value> = shelf.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["title"].clone())
        .collect();
    assert!(titles.contains(&json!("Ein erfundenes Buch")), "{titles:?}");
    let page = lena
        .get(&format!("/api/books/works/ISBN{ISBN_UNMATCHED}"))
        .await;
    page.assert_status(200);
    assert_eq!(page.json()["title"], "Ein erfundenes Buch");
    assert_eq!(page.json()["on_shelf"], "want_to_read");
    let page = demo.get("/api/books/works/OL45883W").await;
    page.assert_status(200);
    assert_eq!(page.json()["id"], middlemarch);
    assert_eq!(page.json()["on_shelf"], "finished");
    for path in [
        "/api/shelf?username=lena".to_owned(),
        "/api/shelf/physical-layout".to_owned(),
        "/api/auth/me/favorites".to_owned(),
        "/api/books/works/BC0000000001".to_owned(),
        format!("/api/books/works/ISBN{ISBN_MATCH}"),
        format!("/api/books/works/ISBN{ISBN10_KEY}"),
        "/api/books/works/BC0000000001/posts".to_owned(),
        format!("/api/clubs/{SLUG}/pick"),
        format!("/api/clubs/{SLUG}/pick/history"),
        format!("/api/clubs/{SLUG}/pick/posts"),
        format!("/api/clubs/{SLUG}/members"),
        format!("/api/clubs/{SLUG}/overlap"),
        format!("/api/clubs/{SLUG}/vote"),
        format!("/api/clubs/{SLUG}/vote/suggestions"),
        format!("/api/clubs/{SLUG}/activity"),
        format!("/api/clubs/{SLUG}/stats"),
        "/api/activity".to_owned(),
        "/api/diary".to_owned(),
        "/api/stats".to_owned(),
        "/api/quotes".to_owned(),
        "/api/search-history".to_owned(),
    ] {
        let res = demo.get(&path).await;
        assert_eq!(res.status.as_u16(), 200, "{path}: {}", res.text());
    }
    assert!(
        app.upstream_requests().await.is_empty(),
        "a read of imported data asked upstream: {:?}",
        app.upstream_requests().await
    );
}

/// The upstream requests apart from the cover searches by title and author (#74: `q`
/// starting with `title:(`), and those.
fn split_cover_searches(
    requests: Vec<wiremock::Request>,
) -> (Vec<wiremock::Request>, Vec<wiremock::Request>) {
    requests.into_iter().partition(|request| {
        !request
            .url
            .query_pairs()
            .any(|(key, value)| key == "q" && value.starts_with("title:("))
    })
}

/// Work-plan PAR, IMP's note: `rematch-legacy` with the real lookup resolves each legacy edition
/// with an ISBN once, remotely (the local step would find the bare-facts edition itself): a
/// 978-3 ISBN asks the DNB first and Open Library's search only after a DNB miss (a DNB hit
/// makes the one Open Library cover check instead); any other group asks Open Library only. The DNB and Open Library hits upgrade their editions in place, so the
/// shelf entries keep their work and edition, the legacy key's match becomes `isbn_matched`, and
/// the book page by the legacy key shows the source's record. A second run asks again only for
/// the editions that are still bare facts.
#[sqlx::test]
async fn rematch_legacy_asks_each_legacy_isbn_once_along_the_real_route(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    known_middlemarch(&app).await;
    let (legacy, report) = offline_import(&app).await;
    let german = report.books[&book_id(&legacy, &format!("/works/ISBN{GERMAN}"))];
    let english = report.books[&book_id(&legacy, &format!("/works/ISBN{ENGLISH}"))];
    let unmatched = report.books[&book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"))];
    let german_entry = shelf_target(&app, 28).await;
    let english_entry = shelf_target(&app, 29).await;
    assert_eq!(german_entry, (german.work_id, german.edition_id));
    assert_eq!(english_entry, (english.work_id, english.edition_id));
    assert_eq!(
        shelf_target(&app, 21).await,
        (unmatched.work_id, unmatched.edition_id)
    );

    Mock::given(method("GET"))
        .and(path("/sru/dnb"))
        .and(query_param("query", format!("num={GERMAN}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(SRU_HIT.as_bytes().to_vec(), "text/xml;charset=UTF-8"),
        )
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/sru/dnb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(SRU_MISS.as_bytes().to_vec(), "text/xml;charset=UTF-8"),
        )
        .with_priority(10)
        .mount(&app.upstream)
        .await;
    // Open Library has no cover for the German edition.
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{GERMAN}.json")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{ENGLISH}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(OL_SEARCH_CIRCE.as_bytes().to_vec(), "application/json"),
        )
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .with_priority(10)
        .mount(&app.upstream)
        .await;

    let first = rematch(&app).await;
    let (requests, covers) = split_cover_searches(app.upstream_requests().await);
    // COVER (#74): after the ISBN pass, every imported book without a cover is searched once
    // by title and author; those requests are counted apart.
    assert!(!covers.is_empty());
    assert!(covers.iter().all(|r| r.url.path() == "/search.json"));
    let isbn10 = parse_isbn(ISBN10_KEY).unwrap().as_str().to_owned();
    // The legacy editions with an ISBN: the four bare-facts ones. The matched key's edition is
    // the catalog's own (source openlibrary), so it is not asked about.
    assert_eq!(lookups(&requests, GERMAN), (1, 0, 1), "{requests:?}");
    assert_eq!(lookups(&requests, ISBN_UNMATCHED), (1, 1, 0));
    assert_eq!(lookups(&requests, &isbn10), (1, 1, 0));
    assert_eq!(lookups(&requests, ENGLISH), (0, 1, 0));
    assert_eq!(lookups(&requests, ISBN_MATCH), (0, 0, 0));
    assert_eq!(requests.len(), 7, "{requests:?}");
    let line = first.entity("legacy editions").unwrap();
    assert_eq!((line.source, line.target), (Some(4), Some(2)), "{line:?}");

    // The DNB record replaced the bare facts of the German edition; Open Library's those of
    // the English one. The shelf entries did not move.
    let (edition_source, work_source, title, matched) =
        edition_state(&app, german.edition_id.unwrap()).await;
    assert_eq!(
        (
            edition_source.as_str(),
            work_source.as_str(),
            matched.as_str()
        ),
        ("dnb", "dnb", "isbn_matched")
    );
    assert_ne!(title, "Der Mörder", "the DNB title replaces the legacy one");
    let (edition_source, work_source, _, matched) =
        edition_state(&app, english.edition_id.unwrap()).await;
    assert_eq!(
        (
            edition_source.as_str(),
            work_source.as_str(),
            matched.as_str()
        ),
        ("openlibrary", "openlibrary", "isbn_matched")
    );
    assert_eq!(shelf_target(&app, 28).await, german_entry);
    assert_eq!(shelf_target(&app, 29).await, english_entry);
    let (edition_source, _, title, matched) =
        edition_state(&app, unmatched.edition_id.unwrap()).await;
    assert_eq!(
        (edition_source.as_str(), title.as_str(), matched.as_str()),
        ("legacy", "Ein erfundenes Buch", "isbn_unmatched")
    );

    // The book pages by legacy key and anna's shelf show the upgraded records, without asking
    // upstream again.
    let anna = sign_in(&app, "anna").await;
    let before = app.upstream_requests().await.len();
    let page = anna.get(&format!("/api/books/works/ISBN{GERMAN}")).await;
    page.assert_status(200);
    assert_eq!(page.json()["id"], german.work_id);
    assert_eq!(
        page.json()["title"],
        json!(title_of(&app, german.work_id).await)
    );
    assert_eq!(page.json()["source"], "dnb");
    assert_eq!(page.json()["on_shelf"], "want_to_read");
    let shelf = anna.get("/api/shelf").await.json();
    let entry = shelf["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == 29)
        .unwrap()
        .clone();
    assert_eq!(entry["book"]["source"], "openlibrary");
    assert_eq!(entry["edition"]["isbn13"], ENGLISH);
    assert_eq!(entry["rating"], 5);
    assert_eq!(app.upstream_requests().await.len(), before);

    // A second run asks only about the two editions that are still bare facts.
    let second = rematch(&app).await;
    let (requests, all_covers) = split_cover_searches(app.upstream_requests().await);
    assert_eq!(
        all_covers.len(),
        2 * covers.len(),
        "each run searches them again"
    );
    assert_eq!(lookups(&requests, GERMAN), (1, 0, 1));
    assert_eq!(lookups(&requests, ENGLISH), (0, 1, 0));
    assert_eq!(lookups(&requests, ISBN_UNMATCHED), (2, 2, 0));
    assert_eq!(lookups(&requests, &isbn10), (2, 2, 0));
    assert_eq!(requests.len(), 11, "{requests:?}");
    let line = second.entity("legacy editions").unwrap();
    assert_eq!((line.source, line.target), (Some(2), Some(0)), "{line:?}");
}

async fn title_of(app: &TestApp, work: i64) -> String {
    sqlx::query_scalar("SELECT title FROM works WHERE id = $1")
        .bind(work)
        .fetch_one(&app.db)
        .await
        .unwrap()
}
