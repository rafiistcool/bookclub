//! Ports of Python tests that no Wave 2 package ported in full (#49): the handoffs of
//! docs/phase1/notes/cat.md "Left for other packages, or PAR", the non-catalog half of
//! `test_local_reads.py`, and the halves that a package's own port stood in for with SQL
//! because the route belonged to another package (shelf → pick readers, every emitter → the
//! activity feeds, favourites → the book page). Each test runs end to end through the routes
//! of several packages, against the real `CatalogService` (`TestApp::with_real_lookup`) where
//! a lookup is involved, and wiremock, so nothing leaves the machine.

mod common;

use std::time::{Duration, Instant};

use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

/// `backend/tests/fixtures/goodreads.csv` (SHELF's copy, byte for byte).
const GOODREADS: &[u8] = include_bytes!("fixtures/goodreads/goodreads.csv");
/// Open Library's answer to `search.json?q=isbn:9780316556347`, recorded 2026-10-27 (CAT).
const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");
/// The DNB SRU record of 9783694050797, recorded 2026-10-27 (CAT).
const SRU_HIT: &str = include_str!("fixtures/dnb_sru/hit_9783694050797.xml");
/// The ISBN of Circe in the Goodreads fixture.
const CIRCE_ISBN: &str = "9780316556347";
/// A German ISBN with a DNB record (Patrice McDonough, "Der Mörder von Whitechapel").
const GERMAN_ISBN: &str = "9783694050797";

/// An Open Library work whose full record was fetched before (a known book), so no read of it
/// asks Open Library again.
async fn known_ol_work(app: &TestApp, ol_work_id: &str, title: &str, cover: Option<i64>) -> i64 {
    let record = WorkRecord {
        ol_work_id: Some(ol_work_id.to_owned()),
        author_names: "Madeline Miller".to_owned(),
        authors: vec![AuthorRecord::named("Madeline Miller")],
        first_publish_year: Some(2018),
        ol_cover_id: cover,
        full_details: true,
        ..WorkRecord::new(Source::OpenLibrary, title)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap()
}

/// `search.json?q=isbn:{isbn}` answers `body`.
async fn mount_isbn_search(app: &TestApp, isbn: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{isbn}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&app.upstream)
        .await;
}

/// `search.json?q={q}` answers `body`.
async fn mount_text_search(app: &TestApp, q: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", q))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&app.upstream)
        .await;
}

/// Every other `search.json` query finds nothing (a definite miss, not an outage). Mounted
/// below the default priority, so the specific mocks win.
async fn mount_empty_search(app: &TestApp) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .with_priority(10)
        .mount(&app.upstream)
        .await;
}

/// Upstream requests whose path contains `needle` and whose query contains `query`.
async fn requests(app: &TestApp, needle: &str, query: &str) -> usize {
    app.upstream_requests()
        .await
        .iter()
        .filter(|request| request.url.path().contains(needle))
        .filter(|request| {
            request
                .url
                .query_pairs()
                .any(|(_, value)| value.contains(query))
        })
        .count()
}

/// Uploads `raw` as a Goodreads export, waits for the job and returns its final state.
async fn import(app: &TestApp, who: &TestClient, raw: &[u8]) -> Value {
    let res = who
        .post_multipart(
            "/api/shelf/import",
            vec![Part::file("goodreads.csv", "text/csv", raw.to_vec())],
        )
        .await;
    res.assert_status(202);
    app.settle().await;
    let job = who
        .get(&format!("/api/shelf/import/{}", res.json()["id"]))
        .await;
    job.assert_status(200);
    job.json()
}

async fn shelf(who: &TestClient) -> Vec<Value> {
    let res = who.get("/api/shelf").await;
    res.assert_status(200);
    res.json()["items"].as_array().unwrap().clone()
}

fn by_title<'a>(items: &'a [Value], title: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["book"]["title"] == title)
        .unwrap_or_else(|| panic!("{title} is not on the shelf: {items:?}"))
}

fn skip(title: &str, code: &str, reason: &str) -> Value {
    json!({"title": title, "code": code, "reason": reason})
}

/// The Open Library cover id behind a `cover_public_id`.
async fn ol_cover(app: &TestApp, book: &Value) -> Option<i64> {
    let public_id = book["cover_public_id"].as_str()?;
    sqlx::query_scalar("SELECT ol_cover_id FROM media WHERE public_id = $1")
        .bind(public_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// parity: test_local_reads.py::test_known_book_reads_never_touch_open_library
/// End to end: the shelf entry, the pick, the diary entry, a pick post, the quote and a
/// nomination are written through their own routes (SHELF, CLUB, POSTS), then every read of the
/// legacy test, under its club-scoped path, and the club and home variants answer 200 without
/// a single upstream request. `/api/books/work/OL1W` is merged into `/works/` (CAT). The
/// nomination is a second book: the current pick cannot be nominated (the legacy test accepted
/// either answer).
#[sqlx::test]
async fn known_book_reads_never_touch_open_library(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let circe = known_ol_work(&app, "OL1W", "Circe", Some(123)).await;
    let achilles = known_ol_work(&app, "OL2W", "The Song of Achilles", None).await;
    let slug = &club.slug;

    ada.post(
        "/api/shelf",
        json!({"work_id": circe, "status": "currently_reading", "progress": 20}),
    )
    .await
    .assert_status(201);
    let pick = ada
        .put(
            &format!("/api/clubs/{slug}/pick"),
            json!({"work_id": circe}),
        )
        .await;
    pick.assert_status(200);
    let pick_id = pick.json()["id"].as_i64().unwrap();
    ada.post(
        &format!("/api/books/works/{circe}/posts"),
        json!({"body": "Halfway through.", "spoiler_upto": 40}),
    )
    .await
    .assert_status(201);
    ada.post(
        &format!("/api/clubs/{slug}/pick/posts"),
        json!({"body": "See you on Saturday."}),
    )
    .await
    .assert_status(201);
    ada.post(
        "/api/quotes",
        json!({"work_id": circe, "body": "But in a story"}),
    )
    .await
    .assert_status(201);
    grace
        .post(
            &format!("/api/clubs/{slug}/vote/nominations"),
            json!({"work_id": achilles}),
        )
        .await
        .assert_status(201);

    for path in [
        "/api/shelf".to_owned(),
        "/api/shelf?username=grace".to_owned(),
        "/api/shelf/physical-layout".to_owned(),
        "/api/auth/me/favorites".to_owned(),
        format!("/api/clubs/{slug}/pick"),
        format!("/api/clubs/{slug}/pick/history"),
        format!("/api/clubs/{slug}/picks/{pick_id}/milestones"),
        format!("/api/clubs/{slug}/overlap"),
        format!("/api/clubs/{slug}/members"),
        format!("/api/clubs/{slug}/vote"),
        format!("/api/clubs/{slug}/vote/suggestions"),
        "/api/diary".to_owned(),
        format!("/api/diary?club={slug}"),
        "/api/activity".to_owned(),
        format!("/api/clubs/{slug}/activity"),
        "/api/stats".to_owned(),
        format!("/api/clubs/{slug}/stats"),
        "/api/quotes".to_owned(),
        format!("/api/quotes?work={circe}"),
        format!("/api/books/works/{circe}"),
        "/api/books/works/OL1W".to_owned(),
        format!("/api/books/works/{circe}/posts"),
        "/api/books/works/OL1W/posts".to_owned(),
        format!("/api/clubs/{slug}/pick/posts"),
        format!("/api/clubs/{slug}/picks/{pick_id}/posts"),
    ] {
        let res = ada.get(&path).await;
        assert_eq!(res.status.as_u16(), 200, "{path}: {}", res.text());
    }
    // The reads see what the writes stored.
    let detail = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(detail["on_shelf"], "currently_reading");
    assert_eq!(detail["quote_count"], 1);
    assert_eq!(ol_cover(&app, &detail).await, Some(123));
    let vote = ada.get(&format!("/api/clubs/{slug}/vote")).await.json();
    assert_eq!(vote["nominations"][0]["book"]["id"], achilles);
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_local_reads.py::test_shelving_persists_search_metadata_for_later_local_reads
/// End to end: the search stores its hit (CAT), and the shelf add (SHELF) answers the stored
/// metadata in `BookOut` without client metadata; the cover is a same-origin `/media/` URL of
/// Open Library cover 123. With Open Library down, the book page and the shelf still serve
/// it. Reasoned change (CAT): the first detail view of a search stub tries its one-time
/// enrichment, which fails here, so the detail is served from the stored hit.
#[sqlx::test]
async fn shelving_persists_search_metadata_for_later_local_reads(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    mount_text_search(
        &app,
        "circe",
        json!({"numFound": 1, "docs": [{
            "key": "/works/OL1W",
            "title": "Circe",
            "author_name": ["Madeline Miller"],
            "cover_i": 123,
            "first_publish_year": 2018
        }]}),
    )
    .await;
    let hits = ada.get("/api/books/search?q=circe").await;
    hits.assert_status(200);
    let work_id = hits.json()["items"][0]["id"].as_i64().unwrap();

    let created = ada
        .post(
            "/api/shelf",
            json!({"work_id": work_id, "status": "want_to_read"}),
        )
        .await;
    created.assert_status(201);
    let book = created.json()["book"].clone();
    assert_eq!(book["id"], work_id);
    assert_eq!(book["ol_work_key"], "/works/OL1W");
    assert_eq!(book["title"], "Circe");
    assert_eq!(book["authors"], "Madeline Miller");
    assert_eq!(book["year"], 2018);
    assert!(
        book["cover_url"].as_str().unwrap().starts_with("/media/"),
        "{book}"
    );
    assert_eq!(ol_cover(&app, &book).await, Some(123));

    app.upstream.reset().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&app.upstream)
        .await;
    let detail = ada.get("/api/books/works/OL1W").await;
    detail.assert_status(200);
    let detail = detail.json();
    assert_eq!(detail["id"], work_id);
    assert_eq!(detail["title"], "Circe");
    assert_eq!(detail["authors"], "Madeline Miller");
    assert_eq!(detail["year"], 2018);
    assert_eq!(detail["on_shelf"], "want_to_read");
    assert_eq!(detail["cover_public_id"], book["cover_public_id"]);
    let items = shelf(&ada).await;
    assert_eq!(items[0]["book"], book);
}

/// parity: test_google_books.py::test_known_google_book_skips_upstream
/// KEEP: a known book is served locally, can be shelved, picked and written about, whatever its
/// source. The legacy test used a Google `ISBN…` key; here it is a book the importer brought
/// over under its legacy key `/works/ISBN9780316769488` (`legacy_work_keys`), opened by that
/// key, shelved (SHELF), picked (CLUB) and given a diary (POSTS), with no upstream request.
#[sqlx::test]
async fn a_known_legacy_isbn_book_skips_upstream(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let mut conn = app.db.acquire().await.unwrap();
    let work = ingest::upsert_work(
        &mut conn,
        &WorkRecord {
            author_names: "Madeline Miller".to_owned(),
            authors: vec![AuthorRecord::named("Madeline Miller")],
            first_publish_year: Some(2018),
            ..WorkRecord::new(Source::Legacy, "Circe")
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    let (_, edition) = ingest::upsert_edition(
        &mut conn,
        work,
        &EditionRecord {
            isbn13: parse_isbn("9780316769488"),
            ..EditionRecord::new(Source::Legacy)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    drop(conn);
    app.exec(&format!(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, edition_id, match)
         VALUES ('/works/ISBN9780316769488', 1, {work}, {edition}, 'isbn_unmatched')"
    ))
    .await;

    let opened = ada.get("/api/books/works/ISBN9780316769488").await;
    opened.assert_status(200);
    assert_eq!(opened.json()["id"], work);
    assert_eq!(opened.json()["title"], "Circe");
    ada.post(
        "/api/shelf",
        json!({"work_id": work, "edition_id": edition, "status": "currently_reading"}),
    )
    .await
    .assert_status(201);
    let detail = ada.get("/api/books/works/ISBN9780316769488").await.json();
    assert_eq!(detail["title"], "Circe");
    assert_eq!(detail["on_shelf"], "currently_reading");

    let pick = ada
        .put(
            &format!("/api/clubs/{}/pick", club.slug),
            json!({"work_id": work}),
        )
        .await;
    pick.assert_status(200);
    assert_eq!(pick.json()["book"]["id"], work);

    ada.post(
        "/api/books/works/ISBN9780316769488/posts",
        json!({"body": "Halfway through."}),
    )
    .await
    .assert_status(201);
    let diary = ada.get("/api/books/works/ISBN9780316769488/posts").await;
    diary.assert_status(200);
    assert_eq!(diary.json()["work_id"], work);
    assert_eq!(diary.json()["items"][0]["body"], "Halfway through.");
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_google_books.py::test_goodreads_import_prefers_google
/// ADAPT: the same fixture CSV imports through the real lookup chain instead of Google: Circe
/// by its ISBN at Open Library (the recorded answer, so the shelf shows Open Library's work and
/// the reader's edition), Achilles by title and author, and Unmatched Novel is a definite miss
/// (`no_match`). A second export shows the rest of the chain: a 978-3 ISBN goes to the DNB and
/// never to Open Library's search, and an ISBN the catalog already holds is resolved locally.
#[sqlx::test]
async fn the_goodreads_fixture_imports_through_the_real_lookup(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ol_circe: Value = serde_json::from_str(OL_SEARCH_CIRCE).unwrap();
    mount_isbn_search(&app, CIRCE_ISBN, ol_circe).await;
    mount_text_search(
        &app,
        "The Song of Achilles Madeline Miller",
        json!({"numFound": 1, "docs": [{
            "key": "/works/OL2W",
            "title": "The Song of Achilles",
            "author_name": ["Madeline Miller"],
            "first_publish_year": 2011
        }]}),
    )
    .await;
    mount_empty_search(&app).await;
    Mock::given(method("GET"))
        .and(path("/sru/dnb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(SRU_HIT.as_bytes().to_vec(), "text/xml;charset=UTF-8"),
        )
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;

    let job = import(&app, &ada, GOODREADS).await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["total"], 3);
    assert_eq!(job["imported"], 2);
    assert_eq!(
        job["skips"],
        json!([
            skip("Custom Shelf Book", "unknown_shelf", "Unknown shelf"),
            skip("Untitled", "no_title", "No title"),
            skip("Unmatched Novel", "no_match", "No match"),
        ])
    );
    let items = shelf(&ada).await;
    assert_eq!(items.len(), 2);
    let circe = by_title(&items, "Circe");
    assert_eq!(circe["status"], "want_to_read");
    assert_eq!(circe["book"]["ol_work_key"], "/works/OL18012166W");
    assert_eq!(circe["edition"]["isbn13"], CIRCE_ISBN);
    assert_eq!(ol_cover(&app, &circe["book"]).await, Some(8739376));
    let achilles = by_title(&items, "The Song of Achilles");
    assert_eq!(achilles["status"], "currently_reading");
    assert_eq!(achilles["book"]["ol_work_key"], "/works/OL2W");
    assert_eq!(achilles["edition"], Value::Null);
    assert_eq!(
        requests(&app, "/search.json", "isbn:9780000000002").await,
        1
    );
    assert_eq!(
        requests(&app, "/search.json", "Unmatched Novel Nobody").await,
        1
    );
    assert_eq!(requests(&app, "/sru/", "").await, 0);

    let grace = app.user("grace").await;
    let job = import(
        &app,
        &grace,
        format!(
            "Title,Author,ISBN13,My Rating,Date Read,Exclusive Shelf\n\
             Der Mörder von Whitechapel,Patrice McDonough,=\"{GERMAN_ISBN}\",,,to-read\n\
             Circe,Madeline Miller,=\"{CIRCE_ISBN}\",4,2024/05/01,read\n"
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["imported"], 2, "{job}");
    assert_eq!(job["skips"], json!([]));
    let items = shelf(&grace).await;
    let german = items
        .iter()
        .find(|item| item["edition"]["isbn13"] == GERMAN_ISBN)
        .unwrap_or_else(|| panic!("the DNB book is not on the shelf: {items:?}"));
    assert_eq!(german["book"]["source"], "dnb");
    let circe = by_title(&items, "Circe");
    assert_eq!(circe["status"], "finished");
    assert_eq!(circe["rating"], 4);
    assert_eq!(circe["book"]["ol_work_key"], "/works/OL18012166W");
    // The DNB answered the German ISBN once; Open Library was never searched for it, and the
    // local Circe edition needed no second request.
    assert_eq!(requests(&app, "/sru/", GERMAN_ISBN).await, 1);
    assert_eq!(
        requests(&app, "/search.json", &format!("isbn:{GERMAN_ISBN}")).await,
        0
    );
    assert_eq!(
        requests(&app, "/search.json", &format!("isbn:{CIRCE_ISBN}")).await,
        1
    );
}

/// parity: test_google_books.py::test_goodreads_import_while_cooling_down
/// ADAPT (work-plan google decision 9): while Open Library cools down after a 429, the import
/// is still accepted (202, not the legacy 429). Each row whose lookup is throttled is skipped
/// with `catalog_unavailable` at once (the cooldown is longer than a row's 60 s budget), the
/// job finishes, and Open Library receives no request.
#[sqlx::test]
async fn a_goodreads_import_during_a_cooldown_skips_without_asking(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let busy = ada.get("/api/books/search?q=circe").await;
    busy.assert_status(429);
    assert!(busy.detail().contains("busy"), "{}", busy.detail());
    let before = app.upstream_requests().await.len();

    let started = Instant::now();
    let job = import(&app, &ada, GOODREADS).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["imported"], 0);
    assert_eq!(
        job["skips"],
        json!([
            skip("Custom Shelf Book", "unknown_shelf", "Unknown shelf"),
            skip("Untitled", "no_title", "No title"),
            skip("Circe", "catalog_unavailable", "Catalog unavailable"),
            skip(
                "The Song of Achilles",
                "catalog_unavailable",
                "Catalog unavailable"
            ),
            skip(
                "Unmatched Novel",
                "catalog_unavailable",
                "Catalog unavailable"
            ),
        ])
    );
    assert_eq!(app.upstream_requests().await.len(), before);
    assert!(shelf(&ada).await.is_empty());
}

/// parity: test_google_books.py::test_pick_does_not_persist_ol_cdn_cover_url
/// KEEP (generalised): the pick body names a catalog work; a `cover_id`, `cover_url` or title
/// in it is ignored. The pick shows the work's own cover (Open Library cover 123, served from
/// `/media/`) or none, no media row is created, and nothing is fetched.
#[sqlx::test]
async fn a_pick_never_stores_a_client_cover(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = known_ol_work(&app, "OL1W", "Circe", Some(123)).await;
    let zine = app.work("Local Zine").await;
    let media = app.scalar_i64("SELECT count(*) FROM media").await;
    let ol_cdn = "https://covers.openlibrary.org/b/id/999-L.jpg?default=false";

    let res = ada
        .put(
            &format!("/api/clubs/{}/pick", club.slug),
            json!({"work_id": circe, "title": "Renamed", "cover_id": 999, "cover_url": ol_cdn}),
        )
        .await;
    res.assert_status(200);
    let book = res.json()["book"].clone();
    assert_eq!(book["title"], "Circe");
    assert!(
        book["cover_url"].as_str().unwrap().starts_with("/media/"),
        "{book}"
    );
    assert_eq!(ol_cover(&app, &book).await, Some(123));

    let res = ada
        .put(
            &format!("/api/clubs/{}/pick", club.slug),
            json!({"work_id": zine.id, "cover_url": ol_cdn}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["book"]["cover_url"], Value::Null);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, media);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM media WHERE ol_cover_id = 999")
            .await,
        0
    );
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_google_books.py::test_shelf_rejects_non_google_cover_url
/// KEEP (generalised, security): a third-party URL in a shelf add, such as a tracking pixel, is
/// ignored (201, as in the legacy test). It is never stored, served or fetched: the entry shows
/// the work's own cover or none, and no media row is created.
#[sqlx::test]
async fn a_shelf_add_never_stores_a_client_cover(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let circe = known_ol_work(&app, "OL1W", "Circe", Some(123)).await;
    let zine = app.work("Local Zine").await;
    let media = app.scalar_i64("SELECT count(*) FROM media").await;
    let pixel = "https://attacker.example/pixel.gif?u=1";

    let res = ada
        .post(
            "/api/shelf",
            json!({"work_id": circe, "cover_id": 999, "cover_url": pixel,
                   "status": "want_to_read"}),
        )
        .await;
    res.assert_status(201);
    let book = res.json()["book"].clone();
    assert!(
        book["cover_url"].as_str().unwrap().starts_with("/media/"),
        "{book}"
    );
    assert_eq!(ol_cover(&app, &book).await, Some(123));
    let res = ada
        .post(
            "/api/shelf",
            json!({"work_id": zine.id, "cover_url": pixel, "status": "want_to_read"}),
        )
        .await;
    res.assert_status(201);
    assert_eq!(res.json()["book"]["cover_url"], Value::Null);

    for item in shelf(&ada).await {
        assert!(!item.to_string().contains("attacker"), "{item}");
    }
    let detail = ada.get(&format!("/api/books/works/{}", zine.id)).await;
    assert_eq!(detail.json()["cover_url"], Value::Null);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, media);
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_google_books.py::test_shelf_isbn_x_and_X_are_the_same_row
/// KEEP: an ISBN-10 with `X` and with `x` names one edition, so shelving it under the second
/// spelling is 409 `Already on your shelf`. Shelving takes a work and an edition id now: both
/// spellings reach the same ids through the ISBN lookup (the second one locally) and through
/// the legacy key map, and the shelf holds one entry.
#[sqlx::test]
async fn isbn_x_and_x_are_one_shelf_entry(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_isbn_search(
        &app,
        "9780804429573",
        json!({"numFound": 1, "docs": [{
            "key": "/works/OL3W",
            "title": "Circe",
            "author_name": ["Madeline Miller"]
        }]}),
    )
    .await;
    let ada = app.user("ada").await;

    let upper = ada.get("/api/books/isbn/080442957X").await;
    upper.assert_status(200);
    let upper = upper.json();
    let (work, edition) = (upper["work_id"].clone(), upper["edition_id"].clone());
    ada.post(
        "/api/shelf",
        json!({"work_id": work, "edition_id": edition, "status": "want_to_read"}),
    )
    .await
    .assert_status(201);
    let calls = app.upstream_requests().await.len();

    let lower = ada.get("/api/books/isbn/080442957x").await.json();
    assert_eq!(lower["work_id"], work);
    assert_eq!(lower["edition_id"], edition);
    assert_eq!(lower["on_shelf"], true);
    let again = ada
        .post(
            "/api/shelf",
            json!({"work_id": lower["work_id"], "edition_id": lower["edition_id"],
                   "status": "want_to_read"}),
        )
        .await;
    again.assert_status(409);
    assert_eq!(again.detail(), "Already on your shelf");
    assert_eq!(again.json()["item"]["edition"]["id"], edition);
    // The second spelling was resolved locally.
    assert_eq!(app.upstream_requests().await.len(), calls);

    // The legacy key map treats both spellings as one key too. (Opening the stored search
    // stub runs its one-time enrichment, so upstream requests are not counted from here.)
    app.exec(&format!(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, edition_id, match)
         VALUES ('/works/ISBN080442957X', 1, {work}, {edition}, 'isbn_matched')"
    ))
    .await;
    let legacy = ada.get("/api/books/works/ISBN080442957x").await.json();
    assert_eq!(legacy["id"], work);
    let again = ada
        .post(
            "/api/shelf",
            json!({"work_id": legacy["id"], "status": "want_to_read"}),
        )
        .await;
    again.assert_status(409);
    assert_eq!(again.detail(), "Already on your shelf");

    let items = shelf(&ada).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["edition"]["isbn13"], "9780804429573");
}

/// Makes `who` a member of `club` through a single-use invite of `admin`'s (CLUB's routes),
/// which records `member_joined`.
async fn join_by_invite(club: &TestClub, admin: &TestClient, who: &TestClient) {
    let invite = admin
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "single_use"}),
        )
        .await;
    invite.assert_status(201);
    let code = invite.json()["code"].as_str().unwrap().to_owned();
    who.post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
        .assert_status(200);
}

/// The reader row of `username` in a pick.
fn reader<'a>(pick: &'a Value, username: &str) -> &'a Value {
    pick["readers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["username"] == username)
        .unwrap_or_else(|| panic!("{username} is not a reader: {pick}"))
}

/// parity: test_shelf.py::test_finish_notes_appear_on_club_pick
/// End to end: the finish notes written with SHELF's `PATCH /api/shelf/{entry_id}` show on
/// CLUB's `GET /api/clubs/{slug}/pick` (both packages' own tests stand the other one in with
/// SQL or a policy read).
#[sqlx::test]
async fn finish_notes_appear_on_the_club_pick(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let pick_path = format!("/api/clubs/{}/pick", club.slug);
    ada.put(&pick_path, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let item = ada
        .post("/api/shelf", json!({"work_id": circe.id}))
        .await
        .json();
    ada.patch(
        &format!("/api/shelf/{}", item["id"]),
        json!({"status": "finished", "rating": 5, "take": "What a book"}),
    )
    .await
    .assert_status(200);

    let pick = ada.get(&pick_path).await.json()["pick"].clone();
    assert_eq!(pick["finished"], json!(["ada"]));
    let row = reader(&pick, "ada");
    assert_eq!(row["rating"], 5);
    assert_eq!(row["take"], "What a book");
}

/// parity: test_shelf.py::test_reading_progress_shows_on_friend_shelf_and_pick
/// End to end: progress set with SHELF's PATCH shows on the pick (CLUB) for the reader and for
/// a co-member who joined with an invite, and on the reader's shelf as the co-member sees it.
#[sqlx::test]
async fn reading_progress_shows_on_a_friends_shelf_and_the_pick(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let pick_path = format!("/api/clubs/{}/pick", club.slug);
    ada.put(&pick_path, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let item = ada
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "currently_reading"}),
        )
        .await
        .json();
    let updated = ada
        .patch(
            &format!("/api/shelf/{}", item["id"]),
            json!({"progress": 35}),
        )
        .await;
    updated.assert_status(200);
    assert_eq!(updated.json()["status"], "currently_reading");
    assert_eq!(updated.json()["progress"], 35);

    let pick = ada.get(&pick_path).await.json()["pick"].clone();
    assert_eq!(pick["reading"], json!(["ada"]));
    assert_eq!(reader(&pick, "ada")["progress"], 35);

    join_by_invite(&club, &ada, &grace).await;
    let friend = grace.get("/api/shelf?username=ada").await.json();
    assert_eq!(friend["items"][0]["progress"], 35);
    let pick = grace.get(&pick_path).await.json()["pick"].clone();
    assert_eq!(pick["readers"][0]["username"], "ada");
    assert_eq!(pick["readers"][0]["progress"], 35);
}

/// parity: test_activity.py::test_feed_records_the_club_story_newest_first
/// End to end: every event comes from the route that records it, across SHELF (add, move,
/// progress, finish, remove), CLUB (join, pick, nomination, ballot) and POSTS (pick post,
/// reaction), and the club feed and the home feed tell the legacy story in the legacy order.
/// An unchanged progress and a repeated ballot record nothing. Reasoned change: joining a club,
/// not registering, records `member_joined`; the legacy single-club feed is the club feed.
#[sqlx::test]
async fn the_club_story_is_recorded_by_every_route(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user("grace").await;
    let ada = app.user("ada").await;
    let club = app.club(&grace, "Readers").await;
    let path = |rest: &str| format!("/api/clubs/{}{rest}", club.slug);
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let achilles = app
        .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
        .await;
    join_by_invite(&club, &grace, &ada).await;
    app.exec(&format!(
        "UPDATE club_memberships SET role = 'admin' WHERE user_id = {}",
        ada.id()
    ))
    .await;

    let entry = ada
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "want_to_read"}),
        )
        .await
        .json()["id"]
        .clone();
    let entry_path = format!("/api/shelf/{entry}");
    for body in [
        json!({"status": "currently_reading", "position": 0}),
        json!({"progress": 50}),
        json!({"progress": 50}),
        json!({"status": "finished", "position": 0, "rating": 4, "take": "Great"}),
    ] {
        ada.patch(&entry_path, body).await.assert_status(200);
    }
    let pick = ada
        .put(&path("/pick"), json!({"work_id": achilles.id}))
        .await
        .json();
    let post = ada
        .post(&path("/pick/posts"), json!({"body": "Chapter one!"}))
        .await;
    post.assert_status(201);
    let post_id = post.json()["id"].clone();
    ada.post(
        &format!("/api/posts/{post_id}/reactions"),
        json!({"emoji": "❤️"}),
    )
    .await
    .assert_status(200);
    let vote = ada
        .post(&path("/vote/nominations"), json!({"work_id": circe.id}))
        .await
        .json();
    let nomination = vote["nominations"][0]["id"].clone();
    for _ in 0..2 {
        ada.post(&path("/vote/cast"), json!({"nomination_id": nomination}))
            .await
            .assert_status(200);
    }
    ada.delete(&entry_path).await.assert_status(204);

    let expected = [
        "shelf_removed",
        "voted",
        "nominated",
        "reaction",
        "note_posted",
        "pick_set",
        "shelf_finished",
        "progress",
        "shelf_moved",
        "shelf_added",
        "member_joined",
    ];
    for feed_path in [path("/activity"), "/api/activity".to_owned()] {
        let feed = ada.get(&feed_path).await;
        feed.assert_status(200);
        let items = feed.json()["items"].as_array().unwrap().clone();
        let kinds: Vec<&str> = items
            .iter()
            .map(|item| item["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, expected, "{feed_path}");
        assert!(items.iter().all(|item| item["actor"] == "ada"));
        let find = |kind: &str| items.iter().find(|item| item["kind"] == kind).unwrap();
        let finished = find("shelf_finished");
        assert_eq!(finished["book"]["title"], "Circe");
        assert_eq!(finished["payload"], json!({"rating": 4, "take": "Great"}));
        assert_eq!(finished["mine"], true);
        assert!(!finished["created_label"].as_str().unwrap().is_empty());
        assert_eq!(find("progress")["payload"], json!({"progress": 50}));
        let reaction = find("reaction");
        assert_eq!(reaction["payload"]["emoji"], "❤️");
        assert_eq!(reaction["payload"]["author"], "ada");
        assert_eq!(reaction["payload"]["post_id"], post_id);
        let pick_set = find("pick_set");
        assert_eq!(pick_set["pick_id"], pick["id"]);
        assert_eq!(pick_set["book"]["title"], "The Song of Achilles");
    }
}

/// parity: test_favorites.py::test_replace_order_clear_and_eligibility
/// End to end, the book-page half SHELF's port leaves to CAT: the favourites written with
/// SHELF's `PUT /api/auth/me/favorites` show as `favorite_position` on CAT's book page, by
/// work id and by `OL…W` key, and clearing the list clears it.
#[sqlx::test]
async fn favourite_positions_show_on_the_book_page(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = known_ol_work(&app, "OL1W", "Circe", None).await;
    let song = known_ol_work(&app, "OL2W", "Song of Achilles", None).await;
    let galatea = known_ol_work(&app, "OL3W", "Galatea", None).await;
    for work in [circe, song, galatea] {
        ada.post("/api/shelf", json!({"work_id": work}))
            .await
            .assert_status(201);
    }
    let reordered = ada
        .put(
            "/api/auth/me/favorites",
            json!({"work_ids": [circe, galatea, song]}),
        )
        .await;
    reordered.assert_status(200);
    let position = |body: Value| body["favorite_position"].clone();
    let detail = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(detail["id"], circe);
    assert_eq!(position(detail), 1);
    assert_eq!(
        position(ada.get(&format!("/api/books/works/{song}")).await.json()),
        3
    );
    assert_eq!(position(ada.get("/api/books/works/OL3W").await.json()), 2);
    let shelf = ada.get("/api/shelf").await.json();
    let titles: Vec<&str> = shelf["favorites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["book"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Circe", "Galatea", "Song of Achilles"]);

    ada.put("/api/auth/me/favorites", json!({"work_ids": []}))
        .await
        .assert_status(200);
    assert_eq!(
        position(ada.get("/api/books/works/OL1W").await.json()),
        Value::Null
    );
}
